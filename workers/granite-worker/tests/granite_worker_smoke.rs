//! Proof that the compiled worker *process* — not just the library function —
//! genuinely transcribes over the framed protocol.
//!
//! `speakeasy-granite`'s own hardware proofs (`granite_smoke.rs`) already
//! cover `transcribe_wav_file`/`transcribe_samples` in-process. What those
//! cannot prove is that this crate's `main.rs` wires the protocol to those
//! functions correctly end to end — that `LoadModel` accepts the real
//! artifact, that buffered `PushAudio` samples survive to `FinishStream`, and
//! that the response frames the desktop app's `ProcessWorkerClient` expects
//! actually arrive. So this test spawns the real compiled binary and drives it
//! exactly as that client would.
//!
//! This has to be an integration test (`tests/`, not a `#[cfg(test)]` module
//! inside `src/main.rs`): Cargo only sets `CARGO_BIN_EXE_<name>` for targets
//! other than the one being tested, and a package with only a `[[bin]]`
//! target has no library for an integration test to import from either — so
//! the handful of wire-protocol literals below (the artifact id, the GGUF
//! filenames, the sample rate) are deliberately duplicated from `main.rs`
//! rather than shared, the same trade already made for the WAV reader in
//! `speakeasy-granite`'s own `granite_smoke` module.
//!
//! ```text
//! cargo test -p speakeasy-granite-worker --test granite_worker_smoke -- --ignored --nocapture
//! ```
//!
//! Needs, and fails loudly without, the two GGUF files under gitignored
//! `.tools/` and the shared `beckett.wav` fixture.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use speakeasy_domain::{
    RequestId, WORKER_PROTOCOL_VERSION, WorkerCommand, WorkerErrorCode, WorkerEvent, WorkerRequest,
    WorkerResponse, WorkerSessionId, read_frame, worker_response_is_terminal, write_frame,
};

const GRANITE_ARTIFACT_ID: &str = "granite-speech-4.1-2b-q4_k_m";
const GRANITE_MODEL_FILENAME: &str = "granite-speech-4.1-2b-Q4_K_M.gguf";
const GRANITE_PROJECTOR_FILENAME: &str = "mmproj-model-f16.gguf";
const SAMPLE_RATE_HZ: u32 = 16_000;

/// Same push-frame size the retained-audio final adapter uses
/// (`speakeasy-asr`'s `RETAINED_PUSH_FRAME_SAMPLES`) — not load-bearing here,
/// just a realistic chunking rather than one giant `PushAudio` frame.
const PUSH_FRAME_SAMPLES: usize = 1_600;

const GROUND_TRUTH: &str =
    "Ever tried. Ever failed. No matter. Try again. Fail again. Fail better.";
const EXPECTED: &str = "Ever tried? Ever failed? No matter. Try again. Fail again. Fail better.";

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

/// A minimal RIFF/WAVE reader for 16 kHz mono 16-bit PCM — the same small,
/// deliberately duplicated reader as `speakeasy-granite`'s `granite_smoke`
/// module and `speakeasy-desktop`'s `transcript_quality`; see either for why a
/// shared dependency is not worth it for nine lines of chunk-walking.
fn read_wave_samples(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
        "not a RIFF/WAVE file: {}",
        path.display()
    );
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let half = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let mut offset = 12;
    let mut data: Option<(usize, usize)> = None;
    while offset + 8 <= bytes.len() {
        let size = usize::try_from(word(offset + 4)).unwrap_or(0);
        let body = offset + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                assert!(size >= 16, "short fmt chunk in {}", path.display());
                assert_eq!(half(body), 1, "{} is not PCM", path.display());
                assert_eq!(half(body + 2), 1, "{} is not mono", path.display());
                assert_eq!(word(body + 4), 16_000, "{} is not 16 kHz", path.display());
                assert_eq!(half(body + 14), 16, "{} is not 16-bit", path.display());
            }
            b"data" => data = Some((body, end)),
            _ => {}
        }
        offset = offset.saturating_add(size.saturating_add(size & 1).saturating_add(8));
    }
    let (body, end) = data.unwrap_or_else(|| panic!("no data chunk in {}", path.display()));
    bytes[body..end]
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0)
        .collect()
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        })
        .filter(|word: &String| !word.is_empty())
        .collect()
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

/// Spawns the worker binary with piped stdio, proves `Hello`/`LoadModel`/
/// `StartStream` in sequence, and hands back the pipes for the caller to
/// drive further. Factored out so the two tests below, and any future one,
/// do not have to repeat the handshake.
fn spawn_and_warm(model_root: &Path) -> (Child, ChildStdin, ChildStdout, WorkerSessionId) {
    let binary = env!("CARGO_BIN_EXE_speakeasy-granite-worker");
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("worker process must spawn");
    let mut input = child.stdin.take().expect("piped stdin");
    let mut output = child.stdout.take().expect("piped stdout");

    let hello = drive(&mut input, &mut output, 1, WorkerCommand::Hello);
    assert!(matches!(hello.as_slice(), [WorkerEvent::Ready { .. }]));

    let loaded = drive(
        &mut input,
        &mut output,
        2,
        WorkerCommand::LoadModel {
            artifact_id: GRANITE_ARTIFACT_ID.to_owned(),
            model_root: model_root.display().to_string(),
        },
    );
    assert!(matches!(
        loaded.as_slice(),
        [WorkerEvent::ModelLoaded { .. }]
    ));

    let session_id = WorkerSessionId(1);
    let started = drive(
        &mut input,
        &mut output,
        3,
        WorkerCommand::StartStream {
            session_id,
            sample_rate_hz: SAMPLE_RATE_HZ,
        },
    );
    assert!(matches!(
        started.as_slice(),
        [WorkerEvent::StreamStarted { .. }]
    ));
    (child, input, output, session_id)
}

/// Pushes every chunk of `samples`, asserting each is accepted in order.
fn push_all_audio(
    input: &mut impl Write,
    output: &mut impl Read,
    session_id: WorkerSessionId,
    samples: &[f32],
) {
    for (index, chunk) in samples.chunks(PUSH_FRAME_SAMPLES).enumerate() {
        let accepted = drive(
            input,
            output,
            4 + index as u64,
            WorkerCommand::PushAudio {
                session_id,
                sequence: index as u64,
                samples: chunk.to_vec(),
            },
        );
        assert!(
            matches!(accepted.as_slice(), [WorkerEvent::AudioAccepted { .. }]),
            "push {index} must be accepted, got {accepted:?}"
        );
    }
}

#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn granite_worker_process_transcribes_the_fixture_on_cpu() {
    let model_root = granite_dir();
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");
    for path in [
        &model_root.join(GRANITE_MODEL_FILENAME),
        &model_root.join(GRANITE_PROJECTOR_FILENAME),
        &audio,
    ] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }
    let samples = read_wave_samples(&audio);

    let (mut child, mut input, mut output, session_id) = spawn_and_warm(&model_root);
    push_all_audio(&mut input, &mut output, session_id, &samples);

    let started_at = Instant::now();
    let finished = drive(
        &mut input,
        &mut output,
        1_000,
        WorkerCommand::FinishStream { session_id },
    );
    let elapsed = started_at.elapsed();
    let [
        WorkerEvent::Transcript { text, is_final, .. },
        WorkerEvent::StreamFinished { .. },
    ] = finished.as_slice()
    else {
        panic!("expected a final transcript followed by StreamFinished, got {finished:?}");
    };
    assert!(is_final);
    println!("granite worker process elapsed={elapsed:?} transcript={text:?}");
    assert_eq!(
        words(text),
        words(GROUND_TRUTH),
        "the transcribed words must match what is said in the fixture"
    );
    assert_eq!(
        normalize(text),
        normalize(EXPECTED),
        "the transcript no longer matches the pinned output"
    );

    let shutdown = drive(&mut input, &mut output, 1_001, WorkerCommand::Shutdown);
    assert!(matches!(shutdown.as_slice(), [WorkerEvent::ShuttingDown]));
    let status = child.wait().expect("worker process must exit");
    assert!(status.success(), "worker exited with {status:?}");
}

/// The direct proof for residency, shaped exactly like the real desktop path:
/// `WorkerFinalAdapter::run_locked` sends a fresh `LoadModel` before *every*
/// dictation, unconditionally -- it has no way to know the process already has
/// one loaded -- so it is not enough for the worker *process* to stay alive;
/// `load_model` itself has to recognise a repeat request for the same
/// artifact and skip reloading. This test sends `LoadModel` twice, once per
/// dictation, and asserts the second one is dramatically faster than the
/// first (it does not reload) while both dictations still transcribe
/// correctly. It also re-proves per-stream cleanup: a second
/// `StartStream`/`FinishStream` cycle reusing stream state the first cycle
/// failed to clear (`self.active`) would fail regardless of the model.
/// Pushes `samples` then drives `FinishStream`, returning the transcript text
/// and how long `FinishStream` itself took. Assumes `StartStream` for
/// `session_id` already happened.
fn run_dictation(
    input: &mut impl Write,
    output: &mut impl Read,
    session_id: WorkerSessionId,
    request_id_base: u64,
    samples: &[f32],
) -> (String, std::time::Duration) {
    push_all_audio(input, output, session_id, samples);
    let started = Instant::now();
    let finished = drive(
        input,
        output,
        request_id_base,
        WorkerCommand::FinishStream { session_id },
    );
    let elapsed = started.elapsed();
    let [
        WorkerEvent::Transcript { text, .. },
        WorkerEvent::StreamFinished { .. },
    ] = finished.as_slice()
    else {
        panic!("expected a final transcript followed by StreamFinished, got {finished:?}");
    };
    (text.clone(), elapsed)
}

#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn granite_worker_process_transcribes_two_dictations_each_preceded_by_load_model() {
    let model_root = granite_dir();
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");
    for path in [
        &model_root.join(GRANITE_MODEL_FILENAME),
        &model_root.join(GRANITE_PROJECTOR_FILENAME),
        &audio,
    ] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }
    let samples = read_wave_samples(&audio);

    // `spawn_and_warm` already drives one `LoadModel` and one `StartStream`.
    let (mut child, mut input, mut output, first_session) = spawn_and_warm(&model_root);
    let (first, first_elapsed) =
        run_dictation(&mut input, &mut output, first_session, 1_000, &samples);

    // Exactly what `WorkerFinalAdapter::run_locked` sends before every
    // dictation -- same artifact, same model_root, unconditionally.
    let reload_started = Instant::now();
    let reloaded = drive(
        &mut input,
        &mut output,
        1_500,
        WorkerCommand::LoadModel {
            artifact_id: GRANITE_ARTIFACT_ID.to_owned(),
            model_root: model_root.display().to_string(),
        },
    );
    let reload_elapsed = reload_started.elapsed();
    assert!(matches!(
        reloaded.as_slice(),
        [WorkerEvent::ModelLoaded { .. }]
    ));

    let second_session = WorkerSessionId(2);
    let started = drive(
        &mut input,
        &mut output,
        2_000,
        WorkerCommand::StartStream {
            session_id: second_session,
            sample_rate_hz: SAMPLE_RATE_HZ,
        },
    );
    assert!(matches!(
        started.as_slice(),
        [WorkerEvent::StreamStarted { .. }]
    ));
    let (second, second_elapsed) =
        run_dictation(&mut input, &mut output, second_session, 2_001, &samples);

    println!(
        "granite worker resident: first={first_elapsed:?} reload={reload_elapsed:?} second={second_elapsed:?} transcript={first:?}"
    );

    for (label, text) in [("first", &first), ("second", &second)] {
        assert_eq!(
            words(text),
            words(GROUND_TRUTH),
            "the {label} dictation's words must match what is said in the fixture"
        );
        assert_eq!(
            normalize(text),
            normalize(EXPECTED),
            "the {label} dictation's transcript no longer matches the pinned output"
        );
    }
    assert_eq!(
        first, second,
        "two independent dictations against one loaded model must produce the identical transcript"
    );
    // The real proof of residency: a repeat `LoadModel` for the same artifact
    // must not pay the ~2 GB load cost again. An order of magnitude below the
    // first dictation's *transcribe* time (which itself excludes the load) is
    // a generous margin against a fast machine's load time while still
    // catching a regression that silently reintroduced a reload.
    assert!(
        reload_elapsed < first_elapsed / 10,
        "a repeat LoadModel for the same artifact took {reload_elapsed:?}, \
         suspiciously close to a real load -- did load_model stop \
         recognising an already-resident model?"
    );

    let shutdown = drive(&mut input, &mut output, 3_000, WorkerCommand::Shutdown);
    assert!(matches!(shutdown.as_slice(), [WorkerEvent::ShuttingDown]));
    let status = child.wait().expect("worker process must exit");
    assert!(status.success(), "worker exited with {status:?}");
}

#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
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
