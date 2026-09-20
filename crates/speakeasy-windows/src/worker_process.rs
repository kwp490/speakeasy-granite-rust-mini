//! A supervised worker child process, and the framed-protocol client for it.
//!
//! Lives here rather than in `speakeasy-worker` because the two things it owns
//! are Windows concerns that this crate already holds: `ProcessSupervisor` and
//! `OwnedProcessTree` for the job-object ownership, and `CREATE_NO_WINDOW` for
//! the console that would otherwise steal the foreground from delivery.
//! `speakeasy-worker` deliberately links nothing native and checks in seconds;
//! putting this there would have pulled `keyring`, `uiautomation` and
//! `win32job` in behind it.
//!
//! It moved out of `apps/desktop` on 2026-08-19 so `apps/bootstrapper` can run
//! the setup smoke test through the identical spawn. A second spawn written
//! beside this one would be a second place for `CREATE_NO_WINDOW` to go
//! missing, and the symptom of that is a dictation delivered into a console
//! window rather than an error.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use crate::{OwnedProcessTree, ProcessSupervisor, StopOutcome};
use speakeasy_domain::{CancelToken, Clock, Deadline, DomainError, ErrorCode};
use speakeasy_worker::{
    ProtocolError, RequestId, WORKER_PROTOCOL_VERSION, WorkerClient, WorkerCommand,
    WorkerErrorCode, WorkerEvent, WorkerRequest, WorkerResponse, frame_bytes, read_frame,
    worker_response_is_terminal,
};

/// One frame handed to the writer thread, with somewhere to report the result.
struct FrameWrite {
    frame: Vec<u8>,
    response: SyncSender<Result<(), &'static str>>,
}

pub struct ProcessWorkerClient<K> {
    process: OwnedProcessTree,
    /// Frames go to the thread that owns `ChildStdin`, never straight down it.
    ///
    /// `write_all` on a full pipe blocks until the child drains it. A worker
    /// that completes the handshake and then stops reading fills the buffer on
    /// an audio request, and the caller used to block there: before it could
    /// poll cancellation, before `receive_until`'s deadline applied, and while
    /// still owning the job object whose drop would have torn the tree down.
    writes: Option<SyncSender<FrameWrite>>,
    responses: Receiver<Result<WorkerResponse, ProtocolError>>,
    supervisor: ProcessSupervisor,
    clock: Arc<K>,
    started_at_ns: u64,
    next_request_id: u64,
    diagnostic_log: Option<PathBuf>,
}

impl<K: Clock + 'static> ProcessWorkerClient<K> {
    /// Starts an owned worker process and proves the framed protocol handshake.
    ///
    /// # Errors
    ///
    /// Returns a recoverable domain error when process ownership, private
    /// pipes, startup, or the versioned handshake cannot be established.
    pub fn spawn(
        command: &mut Command,
        supervisor: ProcessSupervisor,
        clock: Arc<K>,
        startup_deadline: Deadline,
        diagnostic_log: Option<PathBuf>,
    ) -> Result<Self, DomainError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The workers are console binaries, and a windowed parent has no console
        // for them to inherit — so Windows gives each one its own, and a terminal
        // window appears and *takes the foreground* mid-warm-up. Measured on the
        // installed 1.2.3 build: `inference-worker.exe` at ~5s after launch and
        // `granite-worker.exe` at ~9s, each a CASCADIA_HOSTING_WINDOW_CLASS window
        // titled with the worker's full path.
        //
        // This did not show up until `main.rs` declared `windows_subsystem =
        // "windows"`. Before that the app owned a console of its own and the
        // children quietly attached to it, which is why one stray window existed
        // rather than three.
        //
        // It matters beyond tidiness: delivery inspects the foreground window to
        // decide where the transcript goes, so a console stealing the foreground
        // aims a dictation at a terminal. Every worker is spawned through this one
        // function, so this is the only place the flag is needed.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut process = supervisor
            .spawn(command)
            .map_err(|_| domain_error(ErrorCode::AdapterFailed))?;
        let input = process
            .child_mut()
            .stdin
            .take()
            .ok_or_else(|| domain_error(ErrorCode::AdapterFailed))?;
        let output = process
            .child_mut()
            .stdout
            .take()
            .ok_or_else(|| domain_error(ErrorCode::AdapterFailed))?;
        let stderr = process
            .child_mut()
            .stderr
            .take()
            .ok_or_else(|| domain_error(ErrorCode::AdapterFailed))?;
        spawn_stderr_forwarder(stderr, diagnostic_log.clone());
        let writes = Some(spawn_frame_writer(input, diagnostic_log.clone()));
        let responses = spawn_protocol_reader(output);
        let started_at_ns = clock.now().0;
        let mut client = Self {
            process,
            writes,
            responses,
            supervisor,
            clock,
            started_at_ns,
            next_request_id: 1,
            diagnostic_log,
        };
        client.request(
            WorkerCommand::Hello,
            &CancelToken::default(),
            startup_deadline,
        )?;
        Ok(client)
    }

    /// The child's operating-system process id.
    ///
    /// Exposed for the CUDA proof, which is a question only NVML can answer and
    /// only about a *process*: NVML lists the pids holding a compute context on
    /// each device, and matching on the executable's name instead would be
    /// satisfied by a second copy of the same worker started by something else.
    ///
    /// Not an identity for anything else. A pid is reused by Windows after the
    /// process exits, so this is only meaningful while this client is alive —
    /// which it is, by construction, for as long as the caller holds `self`.
    pub fn process_id(&self) -> u32 {
        self.process.child().id()
    }

    /// Requests protocol shutdown and enforces the supervisor stop deadline.
    ///
    /// # Errors
    ///
    /// Returns a recoverable domain error on framing, deadline, worker, or
    /// process-tree shutdown failure.
    pub fn shutdown(mut self, deadline: Deadline) -> Result<StopOutcome, DomainError> {
        let request_id = self.take_request_id();
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id,
            command: WorkerCommand::Shutdown,
        };
        let frame = frame_bytes(&request).map_err(|error| {
            append_diagnostic_line(
                self.diagnostic_log.as_deref(),
                &format!("worker_write_failed kind={}", protocol_error_kind(&error)),
            );
            domain_error(ErrorCode::InvalidData)
        })?;
        let cancel = CancelToken::default();
        // Bounded by the same deadline the stop is: a worker that stops reading
        // must not be able to hold shutdown open either.
        self.write_within_deadline(frame, &cancel, deadline)?;
        let _ = self.receive_until(request_id, &WorkerCommand::Shutdown, &cancel, deadline)?;
        self.supervisor
            .stop(&mut self.process, || Ok(()))
            .map_err(|_| domain_error(ErrorCode::AdapterFailed))
    }

    fn take_request_id(&mut self) -> RequestId {
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    /// Hands one frame to the writer thread and waits, bounded like every other
    /// step of a request.
    ///
    /// Terminating the process tree on expiry is not tidying up: it is the only
    /// thing that can unblock a `write_all` sitting on a full pipe. Without it
    /// this would trade a stuck caller for a stuck thread, and the thread owns
    /// the pipe, so the next request would block too.
    fn write_within_deadline(
        &mut self,
        frame: Vec<u8>,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<(), DomainError> {
        let Some(writes) = self.writes.as_ref() else {
            return Err(domain_error(ErrorCode::AdapterFailed));
        };
        let (response, result) = mpsc::sync_channel(1);
        if writes.send(FrameWrite { frame, response }).is_err() {
            return Err(domain_error(ErrorCode::AdapterFailed));
        }
        loop {
            match result.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(kind)) => {
                    append_diagnostic_line(
                        self.diagnostic_log.as_deref(),
                        &format!("worker_write_failed kind={kind}"),
                    );
                    return Err(domain_error(ErrorCode::AdapterFailed));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(domain_error(ErrorCode::AdapterFailed));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if cancel.is_cancelled() {
                        let _ = self.process.terminate();
                        return Err(domain_error(ErrorCode::Cancelled));
                    }
                    if deadline.expired(self.clock.now()) {
                        append_diagnostic_line(
                            self.diagnostic_log.as_deref(),
                            "worker_write_stalled",
                        );
                        let _ = self.process.terminate();
                        return Err(domain_error(ErrorCode::DeadlineExceeded));
                    }
                }
            }
        }
    }

    fn receive_until(
        &mut self,
        request_id: RequestId,
        command: &WorkerCommand,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<Vec<WorkerEvent>, DomainError> {
        let mut events = Vec::new();
        loop {
            if cancel.is_cancelled() {
                let _ = self.process.terminate();
                return Err(domain_error(ErrorCode::Cancelled));
            }
            if deadline.expired(self.clock.now()) {
                let _ = self.process.terminate();
                return Err(domain_error(ErrorCode::DeadlineExceeded));
            }
            match self.responses.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(response)) => {
                    if response.protocol_version != WORKER_PROTOCOL_VERSION
                        || response.request_id != request_id
                    {
                        let _ = self.process.terminate();
                        return Err(domain_error(ErrorCode::StaleEvent));
                    }
                    let terminal = worker_response_is_terminal(command, &response.event);
                    let failed = matches!(response.event, WorkerEvent::Error { .. });
                    if let WorkerEvent::Error { code, .. } = &response.event {
                        record_worker_error(self.diagnostic_log.as_deref(), *code);
                    }
                    events.push(response.event);
                    if terminal {
                        return if failed {
                            Err(domain_error(ErrorCode::AdapterFailed))
                        } else {
                            Ok(events)
                        };
                    }
                }
                Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                    let elapsed = self.clock.now().0.saturating_sub(self.started_at_ns);
                    self.supervisor
                        .record_unexpected_exit(Duration::from_nanos(elapsed));
                    let exit_status = self.process.child_mut().try_wait().ok().flatten();
                    append_diagnostic_line(
                        self.diagnostic_log.as_deref(),
                        &format!("worker_unexpected_exit exit_status={exit_status:?}"),
                    );
                    let _ = self.process.terminate();
                    return Err(domain_error(ErrorCode::AdapterFailed));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl<K: Clock + 'static> WorkerClient for ProcessWorkerClient<K> {
    fn request(
        &mut self,
        command: WorkerCommand,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<Vec<WorkerEvent>, DomainError> {
        if cancel.is_cancelled() {
            return Err(domain_error(ErrorCode::Cancelled));
        }
        if deadline.expired(self.clock.now()) {
            return Err(domain_error(ErrorCode::DeadlineExceeded));
        }
        let request_id = self.take_request_id();
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id,
            command,
        };
        request
            .validate()
            .map_err(|_| domain_error(ErrorCode::InvalidData))?;
        let frame = frame_bytes(&request).map_err(|error| {
            append_diagnostic_line(
                self.diagnostic_log.as_deref(),
                &format!("worker_write_failed kind={}", protocol_error_kind(&error)),
            );
            domain_error(ErrorCode::InvalidData)
        })?;
        self.write_within_deadline(frame, cancel, deadline)?;
        self.receive_until(request_id, &request.command, cancel, deadline)
    }
}

impl<K> Drop for ProcessWorkerClient<K> {
    fn drop(&mut self) {
        // The sender first, so a writer parked on `recv` ends on disconnect;
        // then the tree, which is what releases one parked inside `write_all`.
        self.writes.take();
        let _ = self.process.terminate();
    }
}

/// Owns `ChildStdin` and writes whatever frames it is handed.
///
/// A thread rather than the calling thread, because `write_all` on a full pipe
/// is unbounded and a request has a deadline. The thread ends when its sender
/// is dropped, or when a write fails — which is what terminating the process
/// tree causes.
fn spawn_frame_writer(
    mut input: ChildStdin,
    diagnostic_log: Option<PathBuf>,
) -> SyncSender<FrameWrite> {
    let (sender, receiver) = mpsc::sync_channel::<FrameWrite>(1);
    std::thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let outcome = input
                .write_all(&request.frame)
                .and_then(|()| input.flush())
                .map_err(|_| "io");
            if outcome.is_err() {
                append_diagnostic_line(diagnostic_log.as_deref(), "worker_write_failed kind=io");
            }
            let _ = request.response.send(outcome);
        }
    });
    sender
}

fn record_worker_error(diagnostic_log: Option<&Path>, code: WorkerErrorCode) {
    append_diagnostic_line(diagnostic_log, &format!("worker_error_code={code:?}"));
}

/// A stable, codes-only label for a framing failure -- never the OS's own
/// error text, which can carry a path or other detail this log must not
/// carry. `write_frame`'s two failure sites (`request`, `shutdown`) used to
/// map any error straight to `AdapterFailed` with nothing recorded at all;
/// found the hard way, chasing an intermittent installed-build worker
/// crash that left no other trace (see the stale-clock deadline bug in
/// `speakeasy_worker`'s `WorkerFinalAdapter::clock`).
fn protocol_error_kind(error: &ProtocolError) -> &'static str {
    match error {
        ProtocolError::Io(io_error) => match io_error.kind() {
            std::io::ErrorKind::BrokenPipe => "broken_pipe",
            std::io::ErrorKind::UnexpectedEof => "unexpected_eof",
            std::io::ErrorKind::TimedOut => "timed_out",
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            std::io::ErrorKind::ConnectionReset => "connection_reset",
            std::io::ErrorKind::ConnectionAborted => "connection_aborted",
            std::io::ErrorKind::WouldBlock => "would_block",
            std::io::ErrorKind::Interrupted => "interrupted",
            _ => "io_other",
        },
        ProtocolError::FrameTooLarge { .. } => "frame_too_large",
        ProtocolError::Json(_) => "json",
        ProtocolError::Invalid(_) => "invalid",
    }
}

fn append_diagnostic_line(diagnostic_log: Option<&Path>, line: &str) {
    let Some(path) = diagnostic_log else {
        return;
    };
    let _ = crate::append_diagnostics_line(path, &format!("{line}\n"));
}

/// Forwards the worker subprocess's stderr into the diagnostic log line by
/// line. The shared diagnostic writer redacts path-shaped native error text
/// before persistence; stderr is not trusted to be privacy-safe merely because
/// the normal protocol keeps transcript output on stdout.
fn spawn_stderr_forwarder(stderr: ChildStderr, diagnostic_log: Option<PathBuf>) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            append_diagnostic_line(diagnostic_log.as_deref(), &format!("worker_stderr={line}"));
        }
    });
}

fn spawn_protocol_reader(
    mut output: ChildStdout,
) -> Receiver<Result<WorkerResponse, ProtocolError>> {
    let (sender, receiver) = mpsc::sync_channel(32);
    thread::spawn(move || {
        loop {
            let response = read_frame(&mut output);
            let failed = response.is_err();
            if sender.send(response).is_err() || failed {
                break;
            }
        }
    });
    receiver
}

const fn domain_error(code: ErrorCode) -> DomainError {
    DomainError {
        code,
        recoverable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CrashThrottle, ProcessDeadlines};
    use speakeasy_domain::SystemClock;
    use speakeasy_worker::{MAX_AUDIO_SAMPLES_PER_REQUEST, WorkerSessionId};

    fn supervisor() -> ProcessSupervisor {
        ProcessSupervisor::new(
            ProcessDeadlines::new(Duration::from_millis(50), Duration::from_millis(50))
                .expect("deadlines"),
            CrashThrottle::new(2, Duration::from_mins(1)).expect("crash throttle"),
        )
    }

    fn spawn_error(result: Result<ProcessWorkerClient<SystemClock>, DomainError>) -> DomainError {
        match result {
            Ok(_) => panic!("fixture worker unexpectedly started"),
            Err(error) => error,
        }
    }

    #[test]
    fn startup_hang_hits_deadline_and_terminates_owned_process() {
        let clock = Arc::new(SystemClock::default());
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Threading.Thread]::Sleep(30000)",
        ]);
        let error = spawn_error(ProcessWorkerClient::spawn(
            &mut command,
            supervisor(),
            Arc::clone(&clock),
            Deadline::after(clock.as_ref(), Duration::from_millis(100)),
            None,
        ));
        assert_eq!(error.code, ErrorCode::DeadlineExceeded);
    }

    #[test]
    fn immediate_worker_crash_is_recoverable_disconnect() {
        let clock = Arc::new(SystemClock::default());
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "exit 23"]);
        let error = spawn_error(ProcessWorkerClient::spawn(
            &mut command,
            supervisor(),
            Arc::clone(&clock),
            Deadline::after(clock.as_ref(), Duration::from_secs(2)),
            None,
        ));
        assert_eq!(error.code, ErrorCode::AdapterFailed);
        assert!(error.recoverable);
    }

    #[test]
    fn stale_spoofed_response_is_rejected_and_process_is_terminated() {
        let clock = Arc::new(SystemClock::default());
        let script = concat!(
            "$json='{\"protocol_version\":1,\"request_id\":99,",
            "\"event\":{\"type\":\"ready\",\"worker_version\":\"spoof\"}}';",
            "$payload=[Text.Encoding]::UTF8.GetBytes($json);",
            "$output=[Console]::OpenStandardOutput();",
            "$length=[BitConverter]::GetBytes([uint32]$payload.Length);",
            "$output.Write($length,0,$length.Length);",
            "$output.Write($payload,0,$payload.Length);",
            "$output.Flush();",
            "[Threading.Thread]::Sleep(30000)"
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        let error = spawn_error(ProcessWorkerClient::spawn(
            &mut command,
            supervisor(),
            Arc::clone(&clock),
            Deadline::after(clock.as_ref(), Duration::from_secs(2)),
            None,
        ));
        assert_eq!(error.code, ErrorCode::StaleEvent);
        assert!(error.recoverable);
    }

    #[test]
    fn worker_stderr_writer_redacts_native_paths() {
        let root = tempfile::tempdir().expect("diagnostic root");
        let path = root.path().join("logs/speakeasy.log");
        append_diagnostic_line(
            Some(&path),
            r"thread 'main' panicked at C:\Users\Alice\worker\main.rs:9:2",
        );
        let contents = std::fs::read_to_string(path).expect("diagnostic log");
        assert!(!contents.contains("Alice"));
        assert!(!contents.contains("main.rs"));
        assert!(contents.contains("<redacted-path>"));
    }
    /// A worker that answers the handshake and then never reads stdin again.
    ///
    /// The pipe buffer fills on the first audio request, which is where the
    /// caller used to block: `write_all` is unbounded, so the deadline in
    /// `receive_until` was never reached and cancellation was never polled.
    fn stalled_reader() -> Command {
        // Built from the constant rather than a literal: a protocol bump would
        // otherwise turn this fixture into a `StaleEvent` that reads like a
        // broken client. `request_id` is 1 because `spawn`'s `Hello` is the
        // first request this client makes.
        let script = format!(
            concat!(
                "$json='{{\"protocol_version\":{version},\"request_id\":1,",
                "\"event\":{{\"type\":\"ready\",\"worker_version\":\"stalled\"}}}}';",
                "$payload=[Text.Encoding]::UTF8.GetBytes($json);",
                "$output=[Console]::OpenStandardOutput();",
                "$length=[BitConverter]::GetBytes([uint32]$payload.Length);",
                "$output.Write($length,0,$length.Length);",
                "$output.Write($payload,0,$payload.Length);",
                "$output.Flush();",
                // Never reads stdin. The sleep outlasts every deadline below, so
                // a test that finishes did so because the write was bounded.
                "[Threading.Thread]::Sleep(120000)"
            ),
            version = WORKER_PROTOCOL_VERSION
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        command
    }

    /// Enough audio to overflow the pipe buffer on any Windows default.
    fn audio_request() -> WorkerCommand {
        WorkerCommand::PushAudio {
            session_id: WorkerSessionId(1),
            sequence: 1,
            samples: vec![0.123_456_79_f32; MAX_AUDIO_SAMPLES_PER_REQUEST],
        }
    }

    fn stalled_client() -> ProcessWorkerClient<SystemClock> {
        let clock = Arc::new(SystemClock::default());
        ProcessWorkerClient::spawn(
            &mut stalled_reader(),
            supervisor(),
            Arc::clone(&clock),
            Deadline::after(clock.as_ref(), Duration::from_secs(10)),
            None,
        )
        .unwrap_or_else(|error| panic!("fixture handshake failed: {error:?}"))
    }

    #[test]
    fn a_worker_that_stops_reading_expires_the_request_instead_of_blocking() {
        let clock = Arc::new(SystemClock::default());
        let mut client = stalled_client();

        let started = std::time::Instant::now();
        let error = client
            .request(
                audio_request(),
                &CancelToken::default(),
                Deadline::after(clock.as_ref(), Duration::from_millis(300)),
            )
            .expect_err("a worker that never reads cannot answer");

        assert_eq!(error.code, ErrorCode::DeadlineExceeded);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the request must expire on its own deadline rather than on the child exiting: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_blocked_write_is_still_cancellable() {
        let clock = Arc::new(SystemClock::default());
        let mut client = stalled_client();
        let cancel = CancelToken::default();
        let trigger = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            trigger.cancel();
        });

        let started = std::time::Instant::now();
        let error = client
            .request(
                audio_request(),
                &cancel,
                // Far beyond the cancellation, so reaching `Cancelled` proves
                // the cancel was polled rather than the deadline arriving.
                Deadline::after(clock.as_ref(), Duration::from_secs(60)),
            )
            .expect_err("a cancelled request cannot succeed");

        assert_eq!(error.code, ErrorCode::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "cancellation must be reached while the write is blocked: {:?}",
            started.elapsed()
        );
    }
}
