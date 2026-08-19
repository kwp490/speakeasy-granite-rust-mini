//! Setup's engine smoke test: transcribe a bundled clip and check the words.
//!
//! # Why this step exists at all
//!
//! Granite Speech is an instruction model with an audio projector bolted on. If
//! that projector fails to attach, or the samples cannot be decoded, it does
//! **not** error -- it answers the prompt from the instruction alone and
//! produces fluent, confident, entirely invented text. "The engine returned a
//! transcript" is therefore evidence of nothing. Only *content* separates a run
//! that read the waveform from one that did not, so this compares against a
//! sentence whose words are known.
//!
//! That is also why setup runs it rather than trusting a successful download:
//! every file can verify against its digest and the engine can still be unable
//! to hear.
//!
//! # What it compares
//!
//! Words, case-folded and stripped of punctuation -- not the transcript
//! verbatim. This is not a hedge; it is measured. The clip says
//!
//! > The quick brown fox jumps over the lazy dog, and Monday begins at dawn.
//!
//! and Granite `Q4_K_M` returned, on 2026-08-19,
//!
//! > The quick brown fox jumps over the lazy dog. And Monday begins at dawn.
//!
//! A period where the sentence has a comma, and a capital where it has
//! lowercase. Every word is right and the engine plainly heard the clip, so an
//! exact-transcript comparison would have refused a working install over a
//! punctuation choice. The exact pin belongs in
//! `workers/granite-worker/tests/granite_worker_smoke.rs`, where a change in it
//! is a finding for a developer rather than a blocked user.
//!
//! Never a substring or a prefix. A `contains` assertion went green in this
//! repository on a transcript missing a third of its utterance.

// Nothing calls `verify_engine` yet: the wizard step that will is held back
// until its copy is reviewed, because setup's wording is reviewable by rule.
// The module is proven against the real engine by its own hardware test in the
// meantime. **Remove this the moment the wizard calls it** -- a standing
// `dead_code` allow is how an unused module stops being noticed.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use speakeasy_domain::{
    CancelToken, Deadline, SystemClock, WorkerClient, WorkerCommand, WorkerEvent, WorkerSessionId,
};
use speakeasy_windows::{CrashThrottle, ProcessDeadlines, ProcessSupervisor, ProcessWorkerClient};

/// The sentence `fixtures/smoke.wav` speaks.
///
/// Kept here **and** in `scripts/New-SmokeFixture.ps1`, checked against each
/// other by [`tests::the_spoken_sentence_matches_the_fixture_generator`]. A
/// fixture whose ground truth is written down in one place drifts the first
/// time either is edited, and the drift is silent: the clip still plays, the
/// engine still transcribes, and the comparison starts failing for a reason
/// that has nothing to do with the engine.
///
/// Changing this line means regenerating the clip *and* re-verifying by
/// transcription. Do not assume a synthesiser says what it reads: an earlier
/// version ended "and Granite writes it down", and the voice's pronunciation of
/// the product name came back as "Granit", which would have pinned a
/// mis-transcription and broken the day a model update got it right.
pub const SPOKEN: &str = "The quick brown fox jumps over the lazy dog, and Monday begins at dawn.";

/// The artifact the worker is asked to load. Must match the worker's own
/// `GRANITE_ARTIFACT_ID`, which is a literal there because that crate
/// deliberately links no manifest reader.
const ARTIFACT_ID: &str = "granite-speech-4.1-2b-q4_k_m";

/// Long enough for a cold model load on a slow disk, and bounded so a wedged
/// worker fails setup rather than hanging it.
const LOAD_DEADLINE: Duration = Duration::from_mins(3);

/// One short clip, well under the desktop's 90 s per-utterance budget.
const TRANSCRIBE_DEADLINE: Duration = Duration::from_mins(2);

/// 100 ms of 16 kHz mono, the frame size the retained-audio path already proves.
const PUSH_FRAME_SAMPLES: usize = 1_600;

/// What setup learned by running the engine once.
///
/// Three outcomes rather than a `bool`. "It did not work" is not actionable,
/// and the two failures need opposite advice: a mismatch means the engine ran
/// and cannot hear, which is a broken install of files that verified; an
/// unavailable engine means it never ran, which is usually a missing file or a
/// machine that cannot host it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The words came back. The engine can hear.
    Verified,
    /// The engine produced text, and it is not what the clip says.
    ///
    /// Carries the transcript because the difference is the diagnosis: fluent
    /// prose unrelated to the sentence means the projector is detached, while a
    /// close-but-wrong transcript is more likely a damaged model file.
    Mismatch { transcript: String },
    /// The engine never produced a transcript to compare.
    Unavailable { reason: &'static str },
}

impl Verdict {
    /// Whether setup may present this as a working engine.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Runs the engine once and reports what it heard.
///
/// Spawns through [`ProcessWorkerClient`] rather than a `Command` of its own,
/// deliberately: that is the one function that sets `CREATE_NO_WINDOW`, and a
/// console window belonging to setup takes the foreground exactly like one
/// belonging to the app.
///
/// Never returns `Err`. Every failure is a [`Verdict`] the wizard can put in
/// front of the user with an instruction, because "setup crashed" is a worse
/// outcome here than "setup can say what is wrong".
#[must_use]
pub fn verify_engine(worker_exe: &Path, model_root: &Path, clip: &Path) -> Verdict {
    let samples = match read_fixture_samples(clip) {
        Ok(samples) => samples,
        Err(reason) => return Verdict::Unavailable { reason },
    };
    match transcribe(worker_exe, model_root, &samples) {
        Ok(transcript) => {
            if words(&transcript) == words(SPOKEN) {
                Verdict::Verified
            } else {
                Verdict::Mismatch { transcript }
            }
        }
        Err(reason) => Verdict::Unavailable { reason },
    }
}

/// Drives one utterance through a freshly spawned worker.
fn transcribe(
    worker_exe: &Path,
    model_root: &Path,
    samples: &[f32],
) -> Result<String, &'static str> {
    let clock = Arc::new(SystemClock::default());
    // Setup runs the engine exactly once, so the throttle's restart-loop job is
    // moot here -- it is constructed because the supervisor requires one, with
    // the smallest legal values rather than a copy of the app's tuning.
    let deadlines = ProcessDeadlines::new(LOAD_DEADLINE, Duration::from_secs(5))
        .map_err(|_| "worker_did_not_start")?;
    let throttle =
        CrashThrottle::new(1, Duration::from_mins(1)).map_err(|_| "worker_did_not_start")?;
    let supervisor = ProcessSupervisor::new(deadlines, throttle);
    let mut command = Command::new(worker_exe);
    let mut client = ProcessWorkerClient::spawn(
        &mut command,
        supervisor,
        Arc::clone(&clock),
        Deadline::after(clock.as_ref(), LOAD_DEADLINE),
        None,
    )
    .map_err(|_| "worker_did_not_start")?;

    let cancel = CancelToken::default();
    let session = WorkerSessionId(1);

    client
        .request(
            WorkerCommand::LoadModel {
                artifact_id: ARTIFACT_ID.to_owned(),
                model_root: model_root.to_string_lossy().into_owned(),
            },
            &cancel,
            Deadline::after(clock.as_ref(), LOAD_DEADLINE),
        )
        .map_err(|_| "model_did_not_load")?;
    client
        .request(
            WorkerCommand::StartStream {
                session_id: session,
                sample_rate_hz: 16_000,
            },
            &cancel,
            Deadline::after(clock.as_ref(), TRANSCRIBE_DEADLINE),
        )
        .map_err(|_| "engine_refused_the_clip")?;
    for (sequence, frame) in samples.chunks(PUSH_FRAME_SAMPLES).enumerate() {
        client
            .request(
                WorkerCommand::PushAudio {
                    session_id: session,
                    sequence: sequence as u64,
                    samples: frame.to_vec(),
                },
                &cancel,
                Deadline::after(clock.as_ref(), TRANSCRIBE_DEADLINE),
            )
            .map_err(|_| "engine_refused_the_clip")?;
    }
    let events = client
        .request(
            WorkerCommand::FinishStream {
                session_id: session,
            },
            &cancel,
            Deadline::after(clock.as_ref(), TRANSCRIBE_DEADLINE),
        )
        .map_err(|_| "engine_did_not_finish")?;

    let text = events
        .iter()
        .filter_map(|event| match event {
            // `is_final` only. Granite buffers and transcribes once at
            // `FinishStream`, but the protocol is shared with an engine that
            // streams, and folding a partial into the comparison would compare
            // against a prefix -- the exact shape this module refuses.
            WorkerEvent::Transcript {
                text,
                is_final: true,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    if text.trim().is_empty() {
        return Err("engine_returned_nothing");
    }
    Ok(text)
}

/// Reads the bundled clip as 16 kHz mono `f32`.
///
/// Strict on purpose. The fixture is generated at exactly 16 kHz, 16-bit, mono
/// by `scripts/New-SmokeFixture.ps1`, and anything else reaching here means the
/// committed file is not the file this expects. Guessing at another layout would
/// feed the engine noise and then report the engine as broken.
fn read_fixture_samples(path: &Path) -> Result<Vec<f32>, &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "clip_missing")?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("clip_unreadable");
    }
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if channels != 1 || sample_rate != 16_000 || bits != 16 {
        return Err("clip_unreadable");
    }
    // Walk the chunk table rather than assuming `data` starts at byte 44: the
    // synthesiser writes a `fact` chunk on some voices, and a fixed offset would
    // read its bytes as audio and transcribe static.
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if id == b"data" {
            let end = body.saturating_add(size).min(bytes.len());
            return Ok(bytes[body..end]
                .chunks_exact(2)
                .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0)
                .collect());
        }
        offset = body + size + (size & 1);
    }
    Err("clip_unreadable")
}

/// Case-folded, punctuation-stripped words: the unit of comparison.
///
/// See this module's header for why it is words rather than the transcript
/// verbatim.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// [`SPOKEN`] and the generator agree.
    ///
    /// The generator's own comment promises this check exists. It did not until
    /// 2026-08-19, so the promise was the only thing keeping the two in step.
    #[test]
    fn the_spoken_sentence_matches_the_fixture_generator() {
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/New-SmokeFixture.ps1"),
        )
        .expect("the fixture generator is in the repository");

        let quoted = format!("'{SPOKEN}'");
        assert!(
            script.contains(&quoted),
            "New-SmokeFixture.ps1 speaks a different sentence"
        );
    }

    /// The committed clip is the shape [`read_fixture_samples`] insists on.
    #[test]
    fn the_bundled_clip_reads_as_sixteen_kilohertz_mono() {
        let clip = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/smoke.wav");
        let samples = read_fixture_samples(&clip).expect("the bundled clip must read");

        assert!(samples.len() > 16_000, "clip must exceed one second");
        assert!(
            samples.iter().any(|sample| sample.abs() > 0.01),
            "clip must carry audio, not silence"
        );
    }

    /// Punctuation and casing do not decide the verdict; wording does.
    #[test]
    fn comparison_ignores_casing_and_punctuation_but_not_wording() {
        assert_eq!(words("The quick brown fox."), words("the QUICK brown fox"));
        assert_ne!(words("the quick brown fox"), words("the quick brown ox"));
        assert_ne!(
            words(SPOKEN),
            words("The quick brown fox jumps over the lazy dog."),
            "a truncated transcript must not compare equal"
        );
    }

    /// The real thing: the staged worker, the staged model, the bundled clip.
    ///
    /// Ignored because it needs both GGUFs on disk and takes a cold model load.
    /// Everything above this proves the plumbing around the engine; only this
    /// proves the engine is actually reached and actually heard the clip, which
    /// is the entire claim setup makes by running it.
    ///
    /// `SPEAKEASY_GRANITE_MODEL_ROOT` overrides the model directory so this can
    /// run against an existing install rather than a second copy of ~2 GB.
    #[test]
    #[ignore = "hardware: needs granite-worker.exe and the staged GGUF files"]
    fn the_real_engine_transcribes_the_bundled_clip() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let worker = std::env::var_os("SPEAKEASY_GRANITE_WORKER").map_or_else(
            || repository.join("target/release/speakeasy-granite-worker.exe"),
            PathBuf::from,
        );
        let model_root = std::env::var_os("SPEAKEASY_GRANITE_MODEL_ROOT").map_or_else(
            || repository.join(".tools/granite-speech-4.1-2b"),
            PathBuf::from,
        );
        let clip = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/smoke.wav");

        let verdict = verify_engine(&worker, &model_root, &clip);

        assert_eq!(
            verdict,
            Verdict::Verified,
            "the engine must transcribe the bundled clip"
        );
    }

    /// A missing or malformed clip is `Unavailable`, never a mismatch.
    ///
    /// The distinction is the point of the enum: a mismatch tells the user their
    /// engine cannot hear, which is the wrong thing to say when the clip is what
    /// is broken.
    #[test]
    fn an_unreadable_clip_is_unavailable_rather_than_a_mismatch() {
        let verdict = verify_engine(
            Path::new("does-not-exist.exe"),
            Path::new("does-not-exist"),
            Path::new("does-not-exist.wav"),
        );

        assert_eq!(
            verdict,
            Verdict::Unavailable {
                reason: "clip_missing"
            }
        );
        assert!(!verdict.is_verified());
    }
}
