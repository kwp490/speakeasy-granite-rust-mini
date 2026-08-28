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
//!
//! # What it also decides
//!
//! **Which provider this installation is recorded as running on.** That used to
//! come from a radio button on an earlier page, and on 2026-08-20 a support log
//! read `engine=cpu_gpu_runtime_missing device=cpu installed=cuda` — setup had
//! written down a graphics-card installation it had never had. Nothing lied;
//! nothing checked.
//!
//! So the marker is derived here, from what actually happened, and it takes
//! **three** independent facts to say "graphics card":
//!
//! 1. the payload is published and complete
//!    (`speakeasy_models::inspect_gpu_payload`),
//! 2. the live worker reports a CUDA backend compiled in, at `Hello` — the
//!    startup handshake, and the only thing that can answer it, because
//!    llama.cpp's CUDA backend is linked into the binary rather than sitting
//!    beside it as a file to stat, and
//!
//! 3. NVML lists that worker's **process id** as holding a compute context.
//!
//! The third is not redundant with the second. A CUDA-built worker on a machine
//! whose driver refuses, whose card is claimed, or whose VRAM is exhausted runs
//! the same model on the CPU and reports it in llama.cpp's stderr rather than as
//! an error. Anything short of all three records the processor, which is the
//! truthful answer for a run that happened on the processor.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use speakeasy_domain::{
    CancelToken, Deadline, SystemClock, WorkerClient, WorkerCommand, WorkerEvent, WorkerSessionId,
};
use speakeasy_models::{
    CudaContextProbe, CudaContextProof, GpuPayloadRejection, NvmlCudaContextProbe,
    bundled_manifest, inspect_gpu_payload, prove_cuda_context,
};
use speakeasy_windows::{CrashThrottle, ProcessDeadlines, ProcessSupervisor, ProcessWorkerClient};

/// The sentence `fixtures/smoke.wav` speaks.
///
/// Kept here **and** in `scripts/New-SmokeFixture.ps1`, checked against each
/// other by `the_spoken_sentence_matches_the_fixture_generator`. A
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

/// The clip itself, compiled into the installer.
///
/// `include_bytes!` rather than a staged file: the bootstrapper is one
/// executable by design, and a clip that travels beside it is a file that can go
/// missing between being written and being read. 205 KB against a 7 MB
/// installer buys the removal of a whole failure mode -- there is no
/// "clip missing" verdict because there is no clip to miss.
const BUNDLED_CLIP: &[u8] = include_bytes!("../fixtures/smoke.wav");

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
/// Which provider setup **proved** this installation runs on.
///
/// Proved, not chosen and not preferred. This is what goes into
/// `install-provider.txt`, which the app reads for the life of the installation
/// to tell "running on the processor because that is what was installed" — which
/// is normal — from "running on the processor on a graphics-card installation" —
/// which is a fault. A value that came from anywhere but a run that happened
/// makes both of those unanswerable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvenProvider {
    Processor,
    GraphicsCard,
}

/// The three facts behind [`ProvenProvider`], kept separately.
///
/// Reported rather than reduced to the answer, because when the answer is
/// `Processor` on a machine with a good card the useful question is *which* of
/// the three failed — and a boolean cannot say. Every field is a stable code, so
/// this travels into the wizard's copy and the log unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEvidence {
    /// Why the graphics-card payload was refused, if it was.
    pub payload: Option<GpuPayloadRejection>,
    /// Whether the worker said it has a CUDA backend compiled in, at `Hello`.
    ///
    /// `None` when the handshake did not answer at all — a pre-v2 worker, or a
    /// worker that failed before it spoke. Deliberately not folded into `false`:
    /// "it said no" and "it did not say" are different facts, and treating the
    /// second as the first is the overreach that produced the wrong log line in
    /// the first place.
    pub handshake_cuda: Option<bool>,
    /// Whether NVML places the worker's own process on a device.
    pub context: Option<CudaContextProof>,
}

impl ProviderEvidence {
    /// The provider this evidence proves.
    ///
    /// Every gate has to pass. Written as one expression on purpose: three
    /// separate early returns is how one of them ends up ordered after the
    /// decision it was supposed to gate.
    #[must_use]
    pub fn proven(&self) -> ProvenProvider {
        if self.payload.is_none()
            && self.handshake_cuda == Some(true)
            && self.context.is_some_and(CudaContextProof::is_proven)
        {
            ProvenProvider::GraphicsCard
        } else {
            ProvenProvider::Processor
        }
    }

    /// The single code that best explains a processor result, for the log.
    ///
    /// Ordered by what a reader can act on: a payload that was never published
    /// outranks a handshake that therefore could not say yes.
    #[must_use]
    pub fn code(&self) -> &'static str {
        if let Some(rejection) = &self.payload {
            return rejection.code();
        }
        match self.handshake_cuda {
            None => "worker_handshake_silent",
            Some(false) => "worker_not_cuda_capable",
            Some(true) => self
                .context
                .map_or("cuda_context_unprovable", CudaContextProof::code),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The words came back. The engine can hear.
    ///
    /// Carries the provider it heard them on, and the evidence for it. The
    /// evidence rather than only the answer, because "processor" on a machine
    /// with a capable card needs to say which of the three gates closed.
    Verified {
        provider: ProvenProvider,
        evidence: ProviderEvidence,
    },
    /// The engine produced text, and it is not what the clip says.
    ///
    /// Carries the transcript because the difference is the diagnosis: fluent
    /// prose unrelated to the sentence means the projector is detached, while a
    /// close-but-wrong transcript is more likely a damaged model file.
    Mismatch { transcript: String },
    /// The engine never produced a transcript to compare.
    Unavailable { reason: &'static str },
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
pub fn verify_engine(worker_exe: &Path, model_root: &Path) -> Verdict {
    verify_engine_with(worker_exe, model_root, &NvmlCudaContextProbe)
}

/// [`verify_engine`], with the CUDA context probe supplied.
///
/// The probe is a parameter for the same reason `choose_granite_pack` takes its
/// CUDA flag as one: the case this exists to catch — a complete CUDA payload
/// that runs on the CPU anyway — cannot be produced on demand by any machine
/// this is developed on, so the decision has to be reachable without a card.
#[must_use]
pub fn verify_engine_with(
    worker_exe: &Path,
    model_root: &Path,
    context_probe: &dyn CudaContextProbe,
) -> Verdict {
    let samples = match read_fixture_samples(BUNDLED_CLIP) {
        Ok(samples) => samples,
        Err(reason) => return Verdict::Unavailable { reason },
    };
    // Asked before the run, because it is a fact about the files rather than
    // about this process, and because a refused payload means the handshake
    // below cannot promote the result however it answers.
    let payload = gpu_payload_rejection(worker_exe);
    match transcribe(worker_exe, model_root, &samples, context_probe) {
        Ok(run) => {
            let evidence = ProviderEvidence {
                payload,
                handshake_cuda: run.handshake_cuda,
                context: run.context,
            };
            if words(&run.transcript) == words(SPOKEN) {
                Verdict::Verified {
                    provider: evidence.proven(),
                    evidence,
                }
            } else {
                Verdict::Mismatch {
                    transcript: run.transcript,
                }
            }
        }
        Err(reason) => Verdict::Unavailable { reason },
    }
}

/// Why the graphics-card payload is not usable, if it is not.
///
/// `None` means published and complete. A manifest that will not parse reads as
/// "not published", which is the conservative answer and the same one every
/// other reader of a broken catalog gives.
///
/// Public because [`Verdict`] deliberately drops it. A run that never started
/// returns [`Verdict::Unavailable`] with a reason and no evidence, and one cause
/// of not starting is a CUDA worker whose libraries are not beside it — Windows
/// cannot resolve the imports and names no file. So a caller holding an
/// `Unavailable` asks this to find out whether that is the explanation. Only
/// [`GpuPayloadRejection::RuntimeFilesMissing`] ever is: a worker that was never
/// published or never installed does not stop the *installed* one from running.
#[must_use]
pub fn gpu_payload_rejection(worker_exe: &Path) -> Option<GpuPayloadRejection> {
    let manifest = bundled_manifest().ok()?;
    let directory = worker_exe.parent()?;
    let file_name = worker_exe.file_name()?.to_string_lossy().into_owned();
    inspect_gpu_payload(&manifest, directory, &file_name).err()
}

/// One completed run of the engine, with what it said about itself.
struct EngineRun {
    transcript: String,
    handshake_cuda: Option<bool>,
    context: Option<CudaContextProof>,
}

/// Drives one utterance through a freshly spawned worker.
fn transcribe(
    worker_exe: &Path,
    model_root: &Path,
    samples: &[f32],
    context_probe: &dyn CudaContextProbe,
) -> Result<EngineRun, &'static str> {
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

    // The startup handshake. `ProcessWorkerClient::spawn` already sent a `Hello`
    // and discarded the `Ready`, so this asks again rather than changing that
    // signature for one caller: `Hello` is stateless and costs one round-trip.
    //
    // A handshake that does not answer is `None`, never `false`. "It said it has
    // no CUDA backend" and "it never said" are different facts, and only the
    // first is a statement about the binary.
    let handshake_cuda = client
        .request(
            WorkerCommand::Hello,
            &cancel,
            Deadline::after(clock.as_ref(), LOAD_DEADLINE),
        )
        .ok()
        .and_then(|events| {
            events.into_iter().find_map(|event| match event {
                WorkerEvent::Ready {
                    compiled_accelerators,
                    ..
                } => Some(compiled_accelerators.iter().any(|name| name == "cuda")),
                _ => None,
            })
        });

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
    // After the model is loaded and before the clip is pushed: llama.cpp creates
    // its CUDA context and allocates its buffers during `LoadModel`, so this is
    // the earliest moment a context exists to find -- and asking before it would
    // report `NotHolding` about a worker that goes on to use the card.
    //
    // Only asked when the handshake said the binary could. A CPU worker has no
    // context to hold, and querying NVML about it would turn "there is nothing
    // to prove" into a driver question on every processor install.
    let context = (handshake_cuda == Some(true))
        .then(|| prove_cuda_context(context_probe, client.process_id()));
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
    Ok(EngineRun {
        transcript: text,
        handshake_cuda,
        context,
    })
}

/// Reads the bundled clip as 16 kHz mono `f32`.
///
/// Strict on purpose. The fixture is generated at exactly 16 kHz, 16-bit, mono
/// by `scripts/New-SmokeFixture.ps1`, and anything else reaching here means the
/// committed file is not the file this expects. Guessing at another layout would
/// feed the engine noise and then report the engine as broken.
fn read_fixture_samples(bytes: &[u8]) -> Result<Vec<f32>, &'static str> {
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32_768.0)
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

/// A verification running on its own thread, polled by the wizard.
///
/// Mirrors `download::Run` deliberately: the wizard already has one 250 ms
/// timer and one polling shape, and a second shape would be a second thing to
/// get right. The engine check has to be off the UI thread for the same reason
/// the transfer is -- a cold model load is seconds at best, and the wizard must
/// keep painting while it happens.
pub struct Run {
    progress: Arc<Progress>,
}

#[derive(Default)]
pub struct Progress {
    finished: AtomicBool,
    verdict: Mutex<Option<Verdict>>,
}

impl Progress {
    /// Whether the run has settled.
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    /// The verdict, once [`Self::finished`] is true.
    pub fn verdict(&self) -> Option<Verdict> {
        self.verdict.lock().ok().and_then(|slot| slot.clone())
    }
}

impl Run {
    /// The shared state the wizard polls.
    #[must_use]
    pub fn progress(&self) -> &Arc<Progress> {
        &self.progress
    }
}

/// Starts a verification on a background thread.
#[must_use]
pub fn start(worker_exe: PathBuf, model_root: PathBuf) -> Run {
    let progress = Arc::new(Progress::default());
    let worker_progress = Arc::clone(&progress);
    std::thread::spawn(move || {
        let verdict = verify_engine(&worker_exe, &model_root);
        if let Ok(mut slot) = worker_progress.verdict.lock() {
            *slot = Some(verdict);
        }
        // Set last, and after the verdict, so a poll that sees `finished` can
        // trust `verdict` to have been written. The opposite order reports a
        // finished check with no result for one poll interval.
        worker_progress.finished.store(true, Ordering::Relaxed);
    });
    Run { progress }
}

/// Where the payload stages the worker and the clip, relative to an install root.
#[must_use]
pub fn staged_worker(install_root: &Path) -> PathBuf {
    install_root.join("proof").join("granite-worker.exe")
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
        let samples = read_fixture_samples(BUNDLED_CLIP).expect("the bundled clip must read");

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
        let verdict = verify_engine(&worker, &model_root);

        let Verdict::Verified { provider, evidence } = &verdict else {
            panic!("the engine must transcribe the bundled clip, got {verdict:?}");
        };
        // Printed rather than asserted. Which provider a developer's own machine
        // proves is a property of that machine and of whether the worker under
        // test was built with `--features cuda`; asserting either would make this
        // fail on a correct install. The evidence code is the useful output.
        println!(
            "proven provider={provider:?} evidence={} ({evidence:?})",
            evidence.code()
        );
    }

    /// Every path to a processor result, and the one path to a graphics-card one.
    ///
    /// Pure logic, no worker and no card. The condition that produced the
    /// original defect -- a machine with a perfectly good graphics card and a
    /// payload that has no CUDA worker in it -- is the first case here, and it is
    /// unreachable on hardware: this machine cannot be made to have a payload
    /// missing its worker and a held CUDA context at the same time. It read
    /// "an unpublished worker" until 2026-08-28, which stopped being the way to
    /// describe that payload when the worker was pinned on 2026-08-26.
    #[test]
    fn only_all_three_gates_together_prove_a_graphics_card_installation() {
        let complete = |handshake, context| ProviderEvidence {
            payload: None,
            handshake_cuda: handshake,
            context,
        };

        // GPU hardware, CPU payload. The reported failure's own shape.
        let cpu_payload = ProviderEvidence {
            payload: Some(GpuPayloadRejection::WorkerNotPublished),
            // Deliberately the *most* favourable answers for the other two
            // gates: even a worker claiming CUDA and holding a context must not
            // promote an installation whose payload was never published, because
            // the marker describes the installation rather than the run.
            handshake_cuda: Some(true),
            context: Some(CudaContextProof::Holding),
        };
        assert_eq!(cpu_payload.proven(), ProvenProvider::Processor);
        assert_eq!(cpu_payload.code(), "gpu_worker_not_published");

        // A CUDA worker with no CUDA libraries beside it.
        let missing_dlls = ProviderEvidence {
            payload: Some(GpuPayloadRejection::RuntimeFilesMissing(vec![
                "cublas64_13.dll".to_owned(),
            ])),
            handshake_cuda: Some(true),
            context: Some(CudaContextProof::Holding),
        };
        assert_eq!(missing_dlls.proven(), ProvenProvider::Processor);
        assert_eq!(missing_dlls.code(), "gpu_runtime_files_missing");

        // A complete payload whose worker is a CPU build.
        let cpu_worker = complete(Some(false), None);
        assert_eq!(cpu_worker.proven(), ProvenProvider::Processor);
        assert_eq!(cpu_worker.code(), "worker_not_cuda_capable");

        // A handshake that never answered. Not the same as answering "no", and
        // must not be promoted on the strength of the other two gates.
        let silent = complete(None, None);
        assert_eq!(silent.proven(), ProvenProvider::Processor);
        assert_eq!(silent.code(), "worker_handshake_silent");

        // A CUDA build that ran on the CPU anyway -- the case no static check
        // can see, and the reason the NVML gate exists.
        let fell_back = complete(Some(true), Some(CudaContextProof::NotHolding));
        assert_eq!(fell_back.proven(), ProvenProvider::Processor);
        assert_eq!(fell_back.code(), "cuda_context_absent");

        // A driver that would not answer. Records the processor, because a
        // marker written on an unanswerable question is the manufactured claim
        // this whole path exists to remove -- and says which question it was.
        let unprovable = complete(
            Some(true),
            Some(CudaContextProof::ProbeUnavailable(
                speakeasy_models::GpuProbeFailure::LibraryMissing,
            )),
        );
        assert_eq!(unprovable.proven(), ProvenProvider::Processor);
        assert_eq!(unprovable.code(), "cuda_context_unprovable");

        // All three, and only then.
        let proven = complete(Some(true), Some(CudaContextProof::Holding));
        assert_eq!(proven.proven(), ProvenProvider::GraphicsCard);
        assert_eq!(proven.code(), "cuda_context_held");
    }

    /// A payload that is not on this disk cannot produce a graphics-card marker.
    ///
    /// Against the real bundled manifest and a worker path that does not exist,
    /// which is what the *installer's own working directory* is: setup has no
    /// `proof/` of its own, so this is the answer any machine gives before the
    /// payload is staged.
    ///
    /// **The expected rejection changed on 2026-08-26** from `WorkerNotPublished`
    /// to `WorkerNotInstalled`, when the CUDA worker was pinned. That is the
    /// whole reason this test exists against the real manifest rather than a
    /// fixture -- its own comment promised it would "fail on the day a CUDA
    /// worker is pinned without the rest of this path being finished", and it
    /// did. The property it holds is unchanged and is the one that matters: a
    /// published worker is never enough to record `cuda`, only a present and
    /// operational one is.
    #[test]
    fn a_payload_that_is_not_staged_here_refuses_the_graphics_card_configuration() {
        let rejection = gpu_payload_rejection(Path::new("proof/granite-worker.exe"))
            .expect("no worker is staged beside setup, so the payload must be refused");
        assert_eq!(rejection, GpuPayloadRejection::WorkerNotInstalled);
    }

    /// An engine that cannot start is `Unavailable`, never a mismatch.
    ///
    /// The distinction is the point of the enum: a mismatch tells the user their
    /// engine cannot hear, which is the wrong thing to say when the engine never
    /// ran at all.
    #[test]
    fn an_engine_that_cannot_start_is_unavailable_rather_than_a_mismatch() {
        let verdict = verify_engine(Path::new("does-not-exist.exe"), Path::new("does-not-exist"));

        assert_eq!(
            verdict,
            Verdict::Unavailable {
                reason: "worker_did_not_start"
            }
        );
    }
}
