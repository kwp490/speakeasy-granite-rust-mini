//! Windows lifecycle and identity adapter boundary for `SpeakEasy`.

#![allow(clippy::must_use_candidate)]

mod activity;
mod clipboard;
mod commit;
mod confirm;
mod credentials;
mod diagnostic_wav;
mod diagnostics;
mod input;
mod startup;
mod target;
mod worker_process;

pub use activity::{
    ActivityHookEvidence, ActivityMonitor, InputActivityEpoch, capture_input_activity,
};
pub use clipboard::ClipboardWriter;
pub use commit::CommitWriter;
pub use confirm::{Confirmation, confirm_destructive_action};
#[cfg(windows)]
pub use credentials::WindowsCredentialManager;
pub use credentials::{
    CredentialKeyRef, LEGACY_OPENAI_FALLBACK, LEGACY_OPENAI_PRIMARY, LEGACY_REMOTE_TOKEN,
    LegacyCredentialReport, LegacyCredentialSource,
};
pub use diagnostic_wav::{
    DiagnosticWavConsent, DiagnosticWavFile, DiagnosticWavPolicy, save_diagnostic_wav,
};
pub use diagnostics::{DIAGNOSTICS_LOG_MAX_BYTES, append_diagnostics_line, redact_diagnostic_text};
pub use input::{activation_modifiers_released, wait_for_activation_modifiers};
pub use startup::{STARTUP_VALUE_NAME, StartupStatus};
#[cfg(windows)]
pub use startup::{migrate_legacy_startup, set_startup_with_windows, startup_status};
pub use target::TargetObserver;
pub use worker_process::ProcessWorkerClient;

use std::collections::VecDeque;
use std::io;
use std::process::{Child, Command};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use speakeasy_domain::{ProducerId, SessionId};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AudioDeviceIdentity {
    pub stable_id: Option<String>,
    pub display_name: String,
    pub is_default: bool,
}

impl AudioDeviceIdentity {
    pub fn same_physical_device(&self, other: &Self) -> bool {
        match (&self.stable_id, &other.stable_id) {
            (Some(left), Some(right)) => left == right,
            _ => self.display_name == other.display_name && self.is_default == other.is_default,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicrophoneDiagnostic {
    Available,
    PermissionDenied,
    NoDefaultDevice,
    SelectedDeviceMissing,
    DeviceDisconnected,
    Suspended,
    SessionLocked,
    RdpTransition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureGeneration {
    pub session_id: SessionId,
    pub producer_id: ProducerId,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioLifecycleEvent {
    PermissionDenied,
    NoDefaultDevice,
    Start {
        device: AudioDeviceIdentity,
        token: CaptureGeneration,
    },
    Stop {
        session_id: SessionId,
    },
    DeviceDisconnected,
    DefaultDeviceChanged(AudioDeviceIdentity),
    Suspend,
    Resume,
    SessionLocked,
    SessionUnlocked,
    RdpTransition,
    Callback(CaptureGeneration),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioLifecycleAction {
    Started,
    Stopped,
    AcceptCallback,
    RejectStaleCallback,
    StopAndPreserveBuffered,
    RetrySelectedDeviceExplicitly,
    NoAction,
}

#[derive(Debug)]
pub struct AudioDeviceLifecycle {
    selected: Option<AudioDeviceIdentity>,
    active: Option<CaptureGeneration>,
    diagnostic: MicrophoneDiagnostic,
    next_generation: u64,
}

impl Default for AudioDeviceLifecycle {
    fn default() -> Self {
        Self {
            selected: None,
            active: None,
            diagnostic: MicrophoneDiagnostic::NoDefaultDevice,
            next_generation: 1,
        }
    }
}

impl AudioDeviceLifecycle {
    pub const fn diagnostic(&self) -> MicrophoneDiagnostic {
        self.diagnostic
    }

    pub fn selected_device(&self) -> Option<&AudioDeviceIdentity> {
        self.selected.as_ref()
    }

    pub fn issue_generation(
        &mut self,
        session_id: SessionId,
        producer_id: ProducerId,
    ) -> CaptureGeneration {
        let token = CaptureGeneration {
            session_id,
            producer_id,
            generation: self.next_generation,
        };
        self.next_generation = self.next_generation.saturating_add(1);
        token
    }

    pub fn apply(&mut self, event: AudioLifecycleEvent) -> AudioLifecycleAction {
        match event {
            AudioLifecycleEvent::PermissionDenied => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::PermissionDenied)
            }
            AudioLifecycleEvent::NoDefaultDevice => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::NoDefaultDevice)
            }
            AudioLifecycleEvent::Start { device, token } => {
                if self.active.is_some() {
                    return AudioLifecycleAction::NoAction;
                }
                self.selected = Some(device);
                self.active = Some(token);
                self.diagnostic = MicrophoneDiagnostic::Available;
                AudioLifecycleAction::Started
            }
            AudioLifecycleEvent::Stop { session_id } => {
                if self
                    .active
                    .is_some_and(|active| active.session_id == session_id)
                {
                    self.active = None;
                    AudioLifecycleAction::Stopped
                } else {
                    AudioLifecycleAction::RejectStaleCallback
                }
            }
            AudioLifecycleEvent::DeviceDisconnected => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::DeviceDisconnected)
            }
            AudioLifecycleEvent::DefaultDeviceChanged(device) => {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.same_physical_device(&device))
                {
                    self.selected = Some(device);
                    return AudioLifecycleAction::NoAction;
                }
                self.stop_with_diagnostic(MicrophoneDiagnostic::SelectedDeviceMissing)
            }
            AudioLifecycleEvent::Suspend => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::Suspended)
            }
            AudioLifecycleEvent::Resume | AudioLifecycleEvent::SessionUnlocked => {
                AudioLifecycleAction::RetrySelectedDeviceExplicitly
            }
            AudioLifecycleEvent::SessionLocked => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::SessionLocked)
            }
            AudioLifecycleEvent::RdpTransition => {
                self.stop_with_diagnostic(MicrophoneDiagnostic::RdpTransition)
            }
            AudioLifecycleEvent::Callback(token) => {
                if self.active == Some(token) {
                    AudioLifecycleAction::AcceptCallback
                } else {
                    AudioLifecycleAction::RejectStaleCallback
                }
            }
        }
    }

    fn stop_with_diagnostic(&mut self, diagnostic: MicrophoneDiagnostic) -> AudioLifecycleAction {
        self.diagnostic = diagnostic;
        if self.active.take().is_some() {
            AudioLifecycleAction::StopAndPreserveBuffered
        } else {
            AudioLifecycleAction::NoAction
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CredentialKey {
    pub service: String,
    pub username: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialStatus {
    Present,
    Missing,
    AccessDenied,
    Unavailable,
}

pub trait CredentialManager: Send + Sync {
    fn status(&self, key: &CredentialKey) -> CredentialStatus;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutRegistration {
    RegisteredNoOp,
    Conflict,
    Unavailable,
}

pub trait ShortcutOwner: Send + Sync {
    fn register_activation(&self) -> ShortcutRegistration;
    fn unregister_activation(&self);
}

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use win32job::{ExtendedLimitInfo, Job};

/// Startup and graceful-stop limits owned by a process supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessDeadlines {
    pub startup: Duration,
    pub graceful_stop: Duration,
}

impl ProcessDeadlines {
    /// Constructs non-zero process deadlines.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] when either duration is zero.
    pub fn new(startup: Duration, graceful_stop: Duration) -> io::Result<Self> {
        if startup.is_zero() || graceful_stop.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "process deadlines must be non-zero",
            ));
        }
        Ok(Self {
            startup,
            graceful_stop,
        })
    }
}

/// Bounded crash history used to stop automatic restart loops.
#[derive(Debug)]
pub struct CrashThrottle {
    maximum_crashes: usize,
    window: Duration,
    crashes: VecDeque<Duration>,
}

impl CrashThrottle {
    /// Creates a crash throttle using monotonic elapsed durations supplied by
    /// the caller's clock.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for a zero limit or window.
    pub fn new(maximum_crashes: usize, window: Duration) -> io::Result<Self> {
        if maximum_crashes == 0 || window.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "crash throttle limit and window must be non-zero",
            ));
        }
        Ok(Self {
            maximum_crashes,
            window,
            crashes: VecDeque::with_capacity(maximum_crashes),
        })
    }

    /// Records a crash and returns whether another automatic restart is
    /// permitted. Equal-to-window events are expired.
    pub fn record_crash(&mut self, elapsed: Duration) -> bool {
        while self
            .crashes
            .front()
            .is_some_and(|oldest| elapsed.saturating_sub(*oldest) >= self.window)
        {
            self.crashes.pop_front();
        }
        self.crashes.push_back(elapsed);
        self.crashes.len() < self.maximum_crashes
    }

    pub fn is_quarantined(&self) -> bool {
        self.crashes.len() >= self.maximum_crashes
    }

    pub fn reset(&mut self) {
        self.crashes.clear();
    }
}

/// A child process whose entire descendant tree is terminated when this value
/// is dropped on Windows.
pub struct OwnedProcessTree {
    child: Child,
    #[cfg(windows)]
    _job: Job,
}

impl OwnedProcessTree {
    /// Spawns a child and assigns it to a kill-on-close Windows Job Object.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the job cannot be created/configured, the
    /// child cannot be spawned, or assignment fails.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let mut limits = ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            let job = Job::create_with_limit_info(&limits).map_err(job_error)?;
            let mut child = command.spawn()?;
            if let Err(error) = job.assign_process(child.as_raw_handle() as isize) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(job_error(error));
            }
            Ok(Self { child, _job: job })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }

    pub fn child(&self) -> &Child {
        &self.child
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Terminates the root child when still running and waits for it.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when status inspection, termination, or waiting
    /// fails.
    pub fn terminate(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        self.child.wait().map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopOutcome {
    Graceful,
    ForcedAfterDeadline,
}

/// Parent-owned worker process policy. Protocol reader threads report the
/// successful handshake through the receiver passed to [`Self::await_startup`].
pub struct ProcessSupervisor {
    deadlines: ProcessDeadlines,
    crashes: CrashThrottle,
}

impl ProcessSupervisor {
    pub const fn new(deadlines: ProcessDeadlines, crashes: CrashThrottle) -> Self {
        Self { deadlines, crashes }
    }

    /// Spawns an owned process unless crash quarantine is active.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::WouldBlock`] while quarantined, or propagates
    /// process-tree creation errors.
    pub fn spawn(&self, command: &mut Command) -> io::Result<OwnedProcessTree> {
        if self.crashes.is_quarantined() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "worker restart is quarantined after repeated crashes",
            ));
        }
        OwnedProcessTree::spawn(command)
    }

    /// Waits for a protocol reader to report a successful handshake. Failure,
    /// disconnect, or timeout terminates the entire owned process tree.
    ///
    /// # Errors
    ///
    /// Returns the reader's error, [`io::ErrorKind::BrokenPipe`] on disconnect,
    /// or [`io::ErrorKind::TimedOut`] on startup timeout.
    pub fn await_startup(
        &self,
        process: &mut OwnedProcessTree,
        ready: &Receiver<io::Result<()>>,
    ) -> io::Result<()> {
        match ready.recv_timeout(self.deadlines.startup) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => terminate_with_error(process, error),
            Err(RecvTimeoutError::Disconnected) => terminate_with_error(
                process,
                io::Error::new(io::ErrorKind::BrokenPipe, "worker handshake channel closed"),
            ),
            Err(RecvTimeoutError::Timeout) => terminate_with_error(
                process,
                io::Error::new(io::ErrorKind::TimedOut, "worker handshake timed out"),
            ),
        }
    }

    /// Requests graceful shutdown, waits to the configured deadline, then
    /// force-terminates a worker that has not exited.
    ///
    /// # Errors
    ///
    /// Propagates shutdown-request, status inspection, termination, or wait
    /// failures.
    pub fn stop(
        &self,
        process: &mut OwnedProcessTree,
        request_shutdown: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<StopOutcome> {
        request_shutdown()?;
        let deadline = Instant::now() + self.deadlines.graceful_stop;
        loop {
            if process.child_mut().try_wait()?.is_some() {
                return Ok(StopOutcome::Graceful);
            }
            if Instant::now() >= deadline {
                process.terminate()?;
                return Ok(StopOutcome::ForcedAfterDeadline);
            }
            std::thread::yield_now();
        }
    }

    pub fn record_unexpected_exit(&mut self, elapsed: Duration) -> bool {
        self.crashes.record_crash(elapsed)
    }

    pub fn is_quarantined(&self) -> bool {
        self.crashes.is_quarantined()
    }

    pub fn reset_quarantine(&mut self) {
        self.crashes.reset();
    }
}

fn terminate_with_error(process: &mut OwnedProcessTree, error: io::Error) -> io::Result<()> {
    process.terminate().and(Err(error))
}

#[cfg(windows)]
#[allow(clippy::needless_pass_by_value)]
fn job_error(error: win32job::JobError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn session(byte: u8) -> SessionId {
        SessionId::from_bytes([byte; 16])
    }

    fn producer(byte: u8) -> ProducerId {
        ProducerId::from_bytes([byte; 16])
    }

    fn device(id: &str, is_default: bool) -> AudioDeviceIdentity {
        AudioDeviceIdentity {
            stable_id: Some(id.to_owned()),
            display_name: "Fixture microphone".to_owned(),
            is_default,
        }
    }

    #[test]
    fn device_loss_preserves_audio_and_rejects_stale_callbacks() {
        let mut lifecycle = AudioDeviceLifecycle::default();
        let token = lifecycle.issue_generation(session(1), producer(2));
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::Start {
                device: device("fixture-1", true),
                token
            }),
            AudioLifecycleAction::Started
        );
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::Callback(token)),
            AudioLifecycleAction::AcceptCallback
        );
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::DeviceDisconnected),
            AudioLifecycleAction::StopAndPreserveBuffered
        );
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::Callback(token)),
            AudioLifecycleAction::RejectStaleCallback
        );
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::Resume),
            AudioLifecycleAction::RetrySelectedDeviceExplicitly
        );
    }

    #[test]
    fn default_change_never_switches_an_active_session_to_an_unrelated_device() {
        let mut lifecycle = AudioDeviceLifecycle::default();
        let token = lifecycle.issue_generation(session(3), producer(4));
        lifecycle.apply(AudioLifecycleEvent::Start {
            device: device("fixture-1", true),
            token,
        });
        assert_eq!(
            lifecycle.apply(AudioLifecycleEvent::DefaultDeviceChanged(device(
                "fixture-2",
                true
            ))),
            AudioLifecycleAction::StopAndPreserveBuffered
        );
        assert_eq!(
            lifecycle.diagnostic(),
            MicrophoneDiagnostic::SelectedDeviceMissing
        );
    }

    #[test]
    fn suspend_lock_and_rdp_require_explicit_retry_with_new_generation() {
        for disruption in [
            AudioLifecycleEvent::Suspend,
            AudioLifecycleEvent::SessionLocked,
            AudioLifecycleEvent::RdpTransition,
        ] {
            let mut lifecycle = AudioDeviceLifecycle::default();
            let old = lifecycle.issue_generation(session(5), producer(6));
            lifecycle.apply(AudioLifecycleEvent::Start {
                device: device("fixture-1", true),
                token: old,
            });
            assert_eq!(
                lifecycle.apply(disruption),
                AudioLifecycleAction::StopAndPreserveBuffered
            );
            let new = lifecycle.issue_generation(session(5), producer(6));
            assert_ne!(old, new);
            assert_eq!(
                lifecycle.apply(AudioLifecycleEvent::Callback(old)),
                AudioLifecycleAction::RejectStaleCallback
            );
        }
    }

    #[test]
    fn repeated_start_stop_is_idempotent_and_session_scoped() {
        let mut lifecycle = AudioDeviceLifecycle::default();
        for index in 1u8..=200 {
            let token = lifecycle.issue_generation(session(index), producer(7));
            assert_eq!(
                lifecycle.apply(AudioLifecycleEvent::Start {
                    device: device("fixture-1", true),
                    token
                }),
                AudioLifecycleAction::Started
            );
            assert_eq!(
                lifecycle.apply(AudioLifecycleEvent::Stop {
                    session_id: token.session_id
                }),
                AudioLifecycleAction::Stopped
            );
            assert_eq!(
                lifecycle.apply(AudioLifecycleEvent::Callback(token)),
                AudioLifecycleAction::RejectStaleCallback
            );
        }
    }

    #[test]
    fn deadlines_reject_zero_values() {
        assert!(ProcessDeadlines::new(Duration::ZERO, Duration::from_secs(1)).is_err());
        assert!(ProcessDeadlines::new(Duration::from_secs(1), Duration::ZERO).is_err());
        assert!(ProcessDeadlines::new(Duration::from_secs(1), Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn crash_throttle_expires_old_events_and_quarantines_bursts() {
        let mut throttle = CrashThrottle::new(3, Duration::from_secs(10)).unwrap();
        assert!(throttle.record_crash(Duration::from_secs(1)));
        assert!(throttle.record_crash(Duration::from_secs(2)));
        assert!(!throttle.record_crash(Duration::from_secs(3)));
        assert!(throttle.is_quarantined());
        assert!(throttle.record_crash(Duration::from_secs(12)));
        assert!(!throttle.is_quarantined());
        throttle.reset();
        assert!(!throttle.is_quarantined());
    }

    #[cfg(windows)]
    #[test]
    fn owned_process_tree_assigns_a_real_child() {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "exit", "0"]);
        let mut owned = OwnedProcessTree::spawn(&mut command).unwrap();
        assert!(owned.child_mut().wait().unwrap().success());
    }

    #[cfg(windows)]
    #[test]
    fn supervisor_enforces_start_and_stop_deadlines_and_quarantine() {
        let deadlines =
            ProcessDeadlines::new(Duration::from_millis(10), Duration::from_millis(10)).unwrap();
        let crashes = CrashThrottle::new(2, Duration::from_mins(1)).unwrap();
        let mut supervisor = ProcessSupervisor::new(deadlines, crashes);

        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "ping", "-n", "6", "127.0.0.1", ">nul"]);
        let mut process = supervisor.spawn(&mut command).unwrap();
        let (_ready_sender, ready_receiver) = mpsc::channel();
        let error = supervisor
            .await_startup(&mut process, &ready_receiver)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(process.child_mut().try_wait().unwrap().is_some());

        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "ping", "-n", "6", "127.0.0.1", ">nul"]);
        let mut process = supervisor.spawn(&mut command).unwrap();
        assert_eq!(
            supervisor.stop(&mut process, || Ok(())).unwrap(),
            StopOutcome::ForcedAfterDeadline
        );

        assert!(supervisor.record_unexpected_exit(Duration::from_secs(1)));
        assert!(!supervisor.record_unexpected_exit(Duration::from_secs(2)));
        assert!(supervisor.is_quarantined());
        let mut command = Command::new("cmd.exe");
        match supervisor.spawn(&mut command) {
            Ok(_) => panic!("quarantined supervisor spawned a process"),
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::WouldBlock),
        }
    }
}
