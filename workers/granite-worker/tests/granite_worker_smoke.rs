//! Proof that the compiled worker *process* refuses an artifact it does not
//! serve, over the framed protocol, from a real spawned binary.
//!
//! It proved more than that until 2026-08-28. Two tests here drove a full
//! dictation end to end -- `LoadModel` accepting the real artifact, buffered
//! `PushAudio` samples surviving to `FinishStream`, the response frames the
//! desktop app's `ProcessWorkerClient` expects actually arriving -- and both
//! read `.tools/fixtures/beckett.wav`, which has not existed in any checkout
//! for months. They reported nothing while reading as merely `#[ignore]`d,
//! which is the third time a fixture under gitignored `.tools/` has done that
//! here; the first two are recorded in `.gitignore` beside the
//! `!apps/bootstrapper/fixtures/smoke.wav` exception added for them, and in
//! the deletion of `speakeasy-granite`'s `granite_smoke` module.
//!
//! **What went with them is real coverage, not just dead code.** Nothing now
//! proves that this crate's `main.rs` wires the protocol to a transcription
//! correctly end to end. `granite_final_pass_transcribes_the_fixture_through_the_real_worker_process`
//! in `apps/desktop` drives the compiled worker against the committed
//! `smoke.wav` and asserts a whole transcript, which covers the same path from
//! one layer up; that is the reason this was judged an acceptable loss rather
//! than an argument that the loss is nil.
//!
//! This has to be an integration test (`tests/`, not a `#[cfg(test)]` module
//! inside `src/main.rs`): Cargo only sets `CARGO_BIN_EXE_<name>` for targets
//! other than the one being tested, and a package with only a `[[bin]]` target
//! has no library for an integration test to import from either.
//!
//! **No fixtures, and no longer `#[ignore]`d.** `LoadModel` checks the artifact
//! id before it touches the disk, so the surviving test needs neither the GGUF
//! files nor a WAV -- it only needs the worker binary Cargo builds for it. Its
//! `#[ignore]` said it needed "the Granite GGUF files", which was never true and
//! kept a runnable test out of the gate.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use speakeasy_domain::{
    RequestId, WORKER_PROTOCOL_VERSION, WorkerCommand, WorkerErrorCode, WorkerEvent, WorkerRequest,
    WorkerResponse, read_frame, worker_response_is_terminal, write_frame,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn granite_dir() -> PathBuf {
    workspace_root()
        .join(".tools")
        .join("granite-speech-4.1-2b")
}

/// Drives one request to completion, returning every event up to and
/// including the terminal one (an `Error`, or the command's own success
/// event) — `FinishStream` is the one command that emits two frames.
fn drive(
    input: &mut impl Write,
    output: &mut impl Read,
    request_id: u64,
    command: WorkerCommand,
) -> Vec<WorkerEvent> {
    let request = WorkerRequest {
        protocol_version: WORKER_PROTOCOL_VERSION,
        request_id: RequestId(request_id),
        command,
    };
    write_frame(input, &request).expect("request must encode");
    let mut events = Vec::new();
    loop {
        let response: WorkerResponse = read_frame(output).expect("response must decode");
        assert_eq!(response.protocol_version, WORKER_PROTOCOL_VERSION);
        assert_eq!(response.request_id, request.request_id);
        let terminal = worker_response_is_terminal(&request.command, &response.event);
        events.push(response.event);
        if terminal {
            return events;
        }
    }
}

#[test]
fn granite_worker_process_refuses_an_untrusted_artifact() {
    let binary = env!("CARGO_BIN_EXE_speakeasy-granite-worker");
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("worker process must spawn");
    let mut input = child.stdin.take().expect("piped stdin");
    let mut output = child.stdout.take().expect("piped stdout");

    let refused = drive(
        &mut input,
        &mut output,
        1,
        WorkerCommand::LoadModel {
            artifact_id: "not-the-real-artifact".to_owned(),
            model_root: granite_dir().display().to_string(),
        },
    );
    assert!(matches!(
        refused.as_slice(),
        [WorkerEvent::Error {
            code: WorkerErrorCode::ArtifactNotTrusted,
            ..
        }]
    ));

    let shutdown = drive(&mut input, &mut output, 2, WorkerCommand::Shutdown);
    assert!(matches!(shutdown.as_slice(), [WorkerEvent::ShuttingDown]));
    let status = child.wait().expect("worker process must exit");
    assert!(status.success(), "worker exited with {status:?}");
}
