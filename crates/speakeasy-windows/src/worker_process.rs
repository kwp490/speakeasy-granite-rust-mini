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

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::{OwnedProcessTree, ProcessSupervisor, StopOutcome};
use speakeasy_domain::{CancelToken, Clock, Deadline, DomainError, ErrorCode};
use speakeasy_worker::{
    ProtocolError, RequestId, WORKER_PROTOCOL_VERSION, WorkerClient, WorkerCommand,
    WorkerErrorCode, WorkerEvent, WorkerRequest, WorkerResponse, read_frame,
    worker_response_is_terminal, write_frame,
};

pub struct ProcessWorkerClient<K> {
    process: OwnedProcessTree,
    input: ChildStdin,
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
        let responses = spawn_protocol_reader(output);
        let started_at_ns = clock.now().0;
        let mut client = Self {
            process,
            input,
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
        if let Err(error) = write_frame(&mut self.input, &request) {
            append_diagnostic_line(
                self.diagnostic_log.as_deref(),
                &format!("worker_write_failed kind={}", protocol_error_kind(&error)),
            );
            return Err(domain_error(ErrorCode::AdapterFailed));
        }
        let cancel = CancelToken::default();
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
        if let Err(error) = write_frame(&mut self.input, &request) {
            append_diagnostic_line(
                self.diagnostic_log.as_deref(),
                &format!("worker_write_failed kind={}", protocol_error_kind(&error)),
            );
            return Err(domain_error(ErrorCode::AdapterFailed));
        }
        self.receive_until(request_id, &request.command, cancel, deadline)
    }
}

impl<K> Drop for ProcessWorkerClient<K> {
    fn drop(&mut self) {
        let _ = self.process.terminate();
    }
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
}
