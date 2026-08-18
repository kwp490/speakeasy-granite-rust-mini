//! Proof that Granite Speech genuinely transcribes on this machine.
//!
//! # The trap this exists to avoid
//!
//! Granite Speech is an instruction model with an audio projector bolted on,
//! which means it will happily answer the prompt *without* the audio. Hand it a
//! projector that failed to attach, or samples it could not decode, and it does
//! not error — it generates fluent text from the instruction alone. "It
//! returned a non-empty string" therefore proves nothing at all about whether
//! any audio was read, which is the same shape of failure as ONNX Runtime
//! silently falling back to CPU (see `speakeasy-asr`'s `cuda_smoke`).
//!
//! What distinguishes the two is *content*: only a run that actually consumed
//! the waveform can produce the words that are in it. So the assertion is the
//! whole transcript against the fixture's known ground truth, never a substring
//! and never a length check. A `contains` assertion here would pass on a
//! transcript missing a third of the utterance — which is precisely the bug that
//! went green in this repository once before, when sherpa's endpointing was
//! dropping audio.
//!
//! # Running it
//!
//! ```text
//! cargo test -p speakeasy-granite granite -- --ignored --nocapture
//! ```
//!
//! Needs, and fails loudly without, the two GGUF files under gitignored
//! `.tools/`. Failing rather than skipping is deliberate: a proof that skips
//! when its subject is absent reports the same thing whether the engine works or
//! was never tried.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::{GraniteModel, GraniteOptions, GraniteStage, transcribe_samples, transcribe_wav_file};

/// Workspace root, derived from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Where the published GGUF conversions live locally.
///
/// These are IBM's own published conversions from
/// `ibm-granite/granite-speech-4.1-2b-GGUF`, not ours — pinned by length and
/// SHA-256 in `models/trusted-manifest.json`.
fn granite_dir() -> PathBuf {
    workspace_root()
        .join(".tools")
        .join("granite-speech-4.1-2b")
}

/// The shipped quantization's model GGUF — `Q4_K_M` since 2026-08-04, when
/// Phase 8's measurement replaced `Q8_0` with it (~21% faster on a 120 s
/// utterance, identical transcript).
///
/// Every proof that is not specifically *about* the quantization loads this,
/// so the next swap is one line here rather than a literal in each test. The
/// `granite_q4_k_m_versus_q8_0_*` comparisons deliberately name both files
/// instead: pointing either side of a comparison at "whatever is shipped"
/// would make it stop comparing the moment the shipped one changed.
fn shipped_model() -> PathBuf {
    granite_dir().join("granite-speech-4.1-2b-Q4_K_M.gguf")
}

/// What is literally written in `beckett.wav`.
///
/// The same fixture the sherpa streaming and CUDA proofs use, so the engines are
/// directly comparable on one utterance. For the record, on this fixture sherpa
/// on CPU returned `Evered ever failed no matter, try again, fail again fail
/// better` — losing "Ever tried" outright and carrying no sentence casing.
const GROUND_TRUTH: &str =
    "Ever tried. Ever failed. No matter. Try again. Fail again. Fail better.";

/// What Granite actually returns, pinned exactly.
///
/// It differs from [`GROUND_TRUTH`] in one respect: `?` rather than `.` after
/// the two opening clauses. That is a defensible reading of a rhetorical line
/// rather than a recognition error — every word, every capital and every
/// sentence boundary is right — so it is pinned as-is rather than softened into
/// a substring match. Decoding is greedy, so this is reproducible; if this
/// string changes, something in the pipeline changed.
const EXPECTED: &str = "Ever tried? Ever failed? No matter. Try again. Fail again. Fail better.";

/// Collapses runs of whitespace so the comparison is about words and
/// punctuation rather than whether a decoder emitted a leading space.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The words alone, lowercased and stripped of punctuation.
///
/// Lets the test assert the *words* against the ground truth while allowing
/// Granite its own punctuation, so a genuine recognition error — a dropped or
/// wrong word — fails even if it somehow arrived with plausible punctuation.
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

#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn granite_transcribes_the_fixture_on_cpu() {
    let model = shipped_model();
    let projector = granite_dir().join("mmproj-model-f16.gguf");
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");

    for path in [&model, &projector, &audio] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let options = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };

    let started = Instant::now();
    let transcript = transcribe_wav_file(&model, &projector, &audio, &options)
        .expect("Granite must transcribe the fixture");
    let elapsed = started.elapsed();

    println!("granite cpu elapsed={elapsed:?} transcript={transcript:?}");

    // The words are checked against what is actually said, so a dropped or
    // misheard word fails here even if the punctuation looks plausible. A model
    // that never received the audio still generates fluent text from the
    // prompt, so this is what distinguishes a real transcription from one.
    assert_eq!(
        words(&transcript),
        words(GROUND_TRUTH),
        "the transcribed words must match what is said in the fixture"
    );
    // And the whole string is pinned, so any change in the pipeline — a prompt
    // tweak, a quantization change, an upstream bump — shows up as a diff
    // rather than passing quietly.
    assert_eq!(
        normalize(&transcript),
        normalize(EXPECTED),
        "the transcript no longer matches the pinned output"
    );
}

/// A minimal RIFF/WAVE reader for 16 kHz mono 16-bit PCM, the one shape the
/// fixtures in this repo come in. Mirrors
/// `apps/desktop/src-tauri/src/transcript_quality.rs`'s `read_wave` — kept as a
/// second, deliberately tiny copy rather than a shared dependency, since
/// pulling `speakeasy-desktop` into this crate (or vice versa) to share nine
/// lines of chunk-walking would be a far worse trade.
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

/// The same fixture and pinned transcript as
/// [`granite_transcribes_the_fixture_on_cpu`], but through [`transcribe_samples`]
/// instead of [`transcribe_wav_file`] — the path a supervised worker process
/// actually uses, since it receives buffered `PushAudio` samples, not a file on
/// disk. Proves the `MtmdBitmap::from_audio_data` path is behaviour-preserving
/// relative to `MtmdBitmap::from_file`, not just that it runs.
#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn granite_transcribes_the_fixture_on_cpu_from_buffered_samples() {
    let model = shipped_model();
    let projector = granite_dir().join("mmproj-model-f16.gguf");
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");

    for path in [&model, &projector, &audio] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let samples = read_wave_samples(&audio);
    let options = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };

    let started = Instant::now();
    let transcript = transcribe_samples(&model, &projector, &samples, &options)
        .expect("Granite must transcribe the buffered samples");
    let elapsed = started.elapsed();

    println!("granite cpu (samples) elapsed={elapsed:?} transcript={transcript:?}");

    assert_eq!(
        words(&transcript),
        words(GROUND_TRUTH),
        "the transcribed words must match what is said in the fixture"
    );
    assert_eq!(
        normalize(&transcript),
        normalize(EXPECTED),
        "the transcript no longer matches the pinned output"
    );
}

/// Proves a loaded [`GraniteModel`] can transcribe more than one independent
/// utterance without reloading — the whole premise of making
/// `workers/granite-worker` resident. Runs the fixture twice against the
/// *same* loaded model and projector: if the second call's KV cache carried
/// any state from the first, or the model somehow needed to be reloaded, this
/// would either fail outright or return a transcript that has drifted from
/// the first call's, since decoding is greedy and therefore deterministic
/// given identical, independent starting state each time.
#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn a_loaded_granite_model_transcribes_two_independent_utterances_identically() {
    let model_path = shipped_model();
    let projector_path = granite_dir().join("mmproj-model-f16.gguf");
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");

    for path in [&model_path, &projector_path, &audio] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let options = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };

    let load_started = Instant::now();
    let model =
        GraniteModel::load(&model_path, &projector_path, &options).expect("Granite must load once");
    let load_elapsed = load_started.elapsed();

    let first_started = Instant::now();
    let first = model
        .transcribe_wav_file(&audio, &options)
        .expect("the first utterance must transcribe against the loaded model");
    let first_elapsed = first_started.elapsed();

    let second_started = Instant::now();
    let second = model
        .transcribe_wav_file(&audio, &options)
        .expect("the second utterance must transcribe against the same loaded model");
    let second_elapsed = second_started.elapsed();

    println!(
        "granite resident: load={load_elapsed:?} first={first_elapsed:?} second={second_elapsed:?}"
    );

    for (label, transcript) in [("first", &first), ("second", &second)] {
        assert_eq!(
            words(transcript),
            words(GROUND_TRUTH),
            "the {label} call's words must match what is said in the fixture"
        );
        assert_eq!(
            normalize(transcript),
            normalize(EXPECTED),
            "the {label} call's transcript no longer matches the pinned output"
        );
    }
    assert_eq!(
        first, second,
        "two independent calls against one loaded model must produce the identical transcript"
    );
}

/// The audio-support guard has to fire, because the failure it catches is
/// otherwise indistinguishable from a bad transcription.
///
/// Passing the *model* where the projector belongs is the cheapest way to get a
/// GGUF that loads but carries no audio projector.
#[test]
#[ignore = "hardware: needs the Granite GGUF files under .tools/granite-speech-4.1-2b/"]
fn a_projector_without_audio_is_refused_rather_than_answered_from_the_prompt() {
    let model = shipped_model();
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");
    assert!(model.is_file(), "missing {}", model.display());

    let options = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };
    let outcome = transcribe_wav_file(&model, &model, &audio, &options);

    let Some(stage) = outcome.as_ref().err().map(super::GraniteError::stage) else {
        panic!("a non-audio projector must fail, got {outcome:?}")
    };
    assert!(
        matches!(
            stage,
            GraniteStage::ProjectorLoad | GraniteStage::AudioUnsupported
        ),
        "expected the projector to be refused, got {}",
        stage.code()
    );
}

/// Where the longer, real-speech corpus recording lives -- the same file
/// `transcript_quality.rs`'s corpus builder draws from. Not checked in,
/// gitignored under `.tools/` like every other hardware fixture here.
fn obama_wav() -> PathBuf {
    workspace_root()
        .join(".tools")
        .join("downloads")
        .join("Obama.wav")
}

/// `samples` at 16 kHz, as seconds.
fn seconds(samples: usize) -> f64 {
    f64::from(u32::try_from(samples).unwrap_or(u32::MAX)) / 16_000.0
}

/// A realistic multi-minute utterance, not `beckett.wav`'s 10 s.
///
/// Phase 8 of the Granite handoff needs this because every existing CPU
/// latency number (`docs/handoff/granite-final-pass.md`'s "Measured"
/// section) is on that one 10 s clip, which cannot answer whether Granite
/// keeps up with a real dictation: encode/decode cost that is noise against a
/// ~2 s model-load-and-teardown budget is not noise against two minutes of
/// speech.
///
/// Sliced from `.tools/downloads/Obama.wav`, 120 s starting 90 s in, past the
/// applause `transcript_quality.rs`'s own module doc says dominates the
/// opening of this recording.
///
/// There is no human transcript of this slice to grade correctness against,
/// so the assertion is plausibility -- real, on-topic speech was read, not
/// the prompt answered from nothing and not a repetition runaway -- mirroring
/// `final_divergence::is_plausible`'s reference-free gate, not a pinned
/// string.
#[test]
#[ignore = "hardware: needs .tools/granite-speech-4.1-2b/ and .tools/downloads/Obama.wav"]
fn granite_transcribes_a_two_minute_utterance_on_cpu_resident() {
    let model_path = shipped_model();
    let projector_path = granite_dir().join("mmproj-model-f16.gguf");
    let source_path = obama_wav();
    for path in [&model_path, &projector_path, &source_path] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let source = read_wave_samples(&source_path);
    let start = 90 * 16_000;
    let clip_len = 120 * 16_000;
    let duration_seconds = 120.0_f64;
    assert!(
        source.len() >= start + clip_len,
        "Obama.wav is only {:.1}s; this test needs at least {:.1}s of it",
        seconds(source.len()),
        seconds(start + clip_len)
    );
    let clip = &source[start..start + clip_len];

    let options = GraniteOptions {
        use_gpu: false,
        n_ctx: 16_384,
        max_new_tokens: 2_048,
        ..GraniteOptions::default()
    };

    let load_started = Instant::now();
    let model =
        GraniteModel::load(&model_path, &projector_path, &options).expect("Granite must load");
    let load_elapsed = load_started.elapsed();

    let started = Instant::now();
    let transcript = model
        .transcribe_samples(clip, &options)
        .expect("Granite must transcribe the 2-minute clip");
    let elapsed = started.elapsed();

    let word_count = transcript.split_whitespace().count();
    let real_time_factor = elapsed.as_secs_f64() / duration_seconds;
    println!(
        "granite cpu resident, {duration_seconds}s utterance: load={load_elapsed:?} \
         transcribe={elapsed:?} rtf={real_time_factor:.3} words={word_count}"
    );
    println!("transcript={transcript:?}");

    let allowed_max = 3.0 + duration_seconds * 4.0;
    assert!(
        f64::from(u32::try_from(word_count).unwrap_or(u32::MAX)) <= allowed_max,
        "produced {word_count} words from {duration_seconds}s of audio, over the plausibility \
         ceiling of {allowed_max:.0} words -- looks like a repetition runaway, not a transcription"
    );
    assert!(
        word_count > 20,
        "produced only {word_count} words from {duration_seconds}s of real speech -- looks like \
         the model answered the prompt rather than reading the audio"
    );
}

/// The measurement that reversed the quantization decision, kept as a
/// regression guard rather than retired with it.
///
/// `Q8_0` used to be install-eligible by *decision*, not measurement; this
/// test supplied the measurement (Phase 8), and Phase 9 made `Q4_K_M` the
/// shipped pack on the strength of it. It stays because the claim it proves
/// -- that the two quantizations agree on this fixture -- is what lets every
/// other proof in this module keep the same pinned string across the swap.
/// Same fixture, same options, nothing but the GGUF differing.
///
/// Correctness is checked by word match against the same ground truth every
/// other `beckett.wav` proof in this module pins -- not by exact-string pin,
/// because quantization is expected to change punctuation/casing choices,
/// which is exactly one of the two things this measurement exists to compare.
#[test]
#[ignore = "hardware: needs both Granite quantizations under .tools/granite-speech-4.1-2b/"]
fn granite_q4_k_m_versus_q8_0_on_beckett() {
    let projector = granite_dir().join("mmproj-model-f16.gguf");
    let q8 = granite_dir().join("granite-speech-4.1-2b-Q8_0.gguf");
    let q4 = granite_dir().join("granite-speech-4.1-2b-Q4_K_M.gguf");
    let audio = workspace_root()
        .join(".tools")
        .join("fixtures")
        .join("beckett.wav");
    for path in [&projector, &q8, &q4, &audio] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let options = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };

    for (label, model_path) in [("q8_0", &q8), ("q4_k_m", &q4)] {
        let load_started = Instant::now();
        let model = GraniteModel::load(model_path, &projector, &options)
            .unwrap_or_else(|error| panic!("{label} must load: {error}"));
        let load_elapsed = load_started.elapsed();

        let started = Instant::now();
        let transcript = model
            .transcribe_wav_file(&audio, &options)
            .unwrap_or_else(|error| panic!("{label} must transcribe beckett.wav: {error}"));
        let elapsed = started.elapsed();

        println!(
            "granite {label} beckett.wav: load={load_elapsed:?} transcribe={elapsed:?} \
             transcript={transcript:?}"
        );
        assert_eq!(
            words(&transcript),
            words(GROUND_TRUTH),
            "{label}'s transcribed words must match what is said in the fixture"
        );
    }
}

/// The same comparison as [`granite_q4_k_m_versus_q8_0_on_beckett`], but on
/// the 2-minute clip: 10 s is too short for a quantization's throughput
/// difference to be more than model-load noise, and throughput at the length
/// that matters is what the Q4_K_M-versus-Q8_0 decision actually turned on.
/// This is the test that produced the ~21% figure Phase 9 acted on.
#[test]
#[ignore = "hardware: needs both Granite quantizations under .tools/granite-speech-4.1-2b/ and .tools/downloads/Obama.wav"]
fn granite_q4_k_m_versus_q8_0_on_a_two_minute_utterance() {
    let projector = granite_dir().join("mmproj-model-f16.gguf");
    let q8 = granite_dir().join("granite-speech-4.1-2b-Q8_0.gguf");
    let q4 = granite_dir().join("granite-speech-4.1-2b-Q4_K_M.gguf");
    let source_path = obama_wav();
    for path in [&projector, &q8, &q4, &source_path] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let source = read_wave_samples(&source_path);
    let start = 90 * 16_000;
    let clip_len = 120 * 16_000;
    let duration_seconds = 120.0_f64;
    assert!(
        source.len() >= start + clip_len,
        "Obama.wav is only {:.1}s; this test needs at least {:.1}s of it",
        seconds(source.len()),
        seconds(start + clip_len)
    );
    let clip = &source[start..start + clip_len];

    let options = GraniteOptions {
        use_gpu: false,
        n_ctx: 16_384,
        max_new_tokens: 2_048,
        ..GraniteOptions::default()
    };
    let allowed_max = 3.0 + duration_seconds * 4.0;

    for (label, model_path) in [("q8_0", &q8), ("q4_k_m", &q4)] {
        let load_started = Instant::now();
        let model = GraniteModel::load(model_path, &projector, &options)
            .unwrap_or_else(|error| panic!("{label} must load: {error}"));
        let load_elapsed = load_started.elapsed();

        let started = Instant::now();
        let transcript = model
            .transcribe_samples(clip, &options)
            .unwrap_or_else(|error| panic!("{label} must transcribe the 2-minute clip: {error}"));
        let elapsed = started.elapsed();

        let word_count = transcript.split_whitespace().count();
        let real_time_factor = elapsed.as_secs_f64() / duration_seconds;
        println!(
            "granite {label} {duration_seconds}s utterance: load={load_elapsed:?} \
             transcribe={elapsed:?} rtf={real_time_factor:.3} words={word_count}"
        );
        println!("{label} transcript={transcript:?}");

        assert!(
            f64::from(u32::try_from(word_count).unwrap_or(u32::MAX)) <= allowed_max,
            "{label} produced {word_count} words, over the plausibility ceiling of \
             {allowed_max:.0} -- looks like a repetition runaway, not a transcription"
        );
        assert!(
            word_count > 20,
            "{label} produced only {word_count} words from {duration_seconds}s of real speech"
        );
    }
}

/// The 120 s slice of `Obama.wav` every Phase 8/9/10 timing number is
/// measured on, so the sweeps below and the figures already in the handoff
/// are directly comparable rather than nearly so.
fn two_minute_clip(source: &[f32]) -> &[f32] {
    let start = 90 * 16_000;
    let clip_len = 120 * 16_000;
    assert!(
        source.len() >= start + clip_len,
        "Obama.wav is only {:.1}s; this needs at least {:.1}s of it",
        seconds(source.len()),
        seconds(start + clip_len)
    );
    &source[start..start + clip_len]
}

/// The last `count` words of a transcript, for eyeballing whether a
/// generation ended on a sentence or was cut off mid-clause.
fn tail_words(text: &str, count: usize) -> String {
    let all: Vec<&str> = text.split_whitespace().collect();
    all[all.len().saturating_sub(count)..].join(" ")
}

/// Phase 10 item 14: `GraniteOptions` has never been tuned, and `n_threads`
/// is the one that was obviously wrong.
///
/// The shipped default is 4 (`GraniteOptions::default`), chosen when this
/// crate was a spike and never measured against an alternative. This machine
/// has 24 cores / 32 logical processors, so the shipped configuration has
/// been transcribing on a sixth of it. Every latency number in the handoff --
/// including the RTF 0.278 that justified the `Q4_K_M` swap -- was measured at
/// 4 threads.
///
/// `n_threads` binds at `GraniteModel::load` (it configures both the model
/// and the projector) *and* at each `transcribe` (`with_n_threads` /
/// `with_n_threads_batch`), so every point in this sweep reloads rather than
/// reusing one resident model. Load cost is reported but excluded from the
/// comparison: residency means production pays it once per launch, not once
/// per dictation.
///
/// `n_ctx` and `max_new_tokens` are held generous here so a truncated
/// generation cannot masquerade as a fast one -- a run that stopped early
/// because it ran out of budget would otherwise look like the winner.
/// Whether the *shipped* budget truncates is the separate question
/// `the_shipped_granite_defaults_hold_a_two_minute_utterance` asks.
///
/// Run this alone. Phase 9 measured the same clip at RTF 0.278 by itself and
/// 0.382 while six other hardware tests shared the CPU -- a bigger swing than
/// any tuning win this sweep is likely to find.
#[test]
#[ignore = "hardware: needs .tools/granite-speech-4.1-2b/ and .tools/downloads/Obama.wav; run alone"]
fn granite_thread_count_sweep_on_a_two_minute_utterance() {
    let model_path = shipped_model();
    let projector_path = granite_dir().join("mmproj-model-f16.gguf");
    let source_path = obama_wav();
    for path in [&model_path, &projector_path, &source_path] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let source = read_wave_samples(&source_path);
    let clip = two_minute_clip(&source);
    let duration_seconds = 120.0_f64;

    let mut results: Vec<(i32, f64, usize, String)> = Vec::new();
    for n_threads in [4, 8, 12, 16, 20, 24, 32] {
        let options = GraniteOptions {
            use_gpu: false,
            n_threads,
            n_ctx: 16_384,
            max_new_tokens: 2_048,
            ..GraniteOptions::default()
        };

        let load_started = Instant::now();
        let model = GraniteModel::load(&model_path, &projector_path, &options)
            .unwrap_or_else(|error| panic!("Granite must load at {n_threads} threads: {error}"));
        let load_elapsed = load_started.elapsed();

        let started = Instant::now();
        let transcript = model
            .transcribe_samples(clip, &options)
            .unwrap_or_else(|error| {
                panic!("Granite must transcribe at {n_threads} threads: {error}")
            });
        let elapsed = started.elapsed();

        let word_count = transcript.split_whitespace().count();
        let real_time_factor = elapsed.as_secs_f64() / duration_seconds;
        println!(
            "granite threads={n_threads}: load={load_elapsed:?} transcribe={elapsed:?} \
             rtf={real_time_factor:.3} words={word_count}"
        );
        results.push((n_threads, elapsed.as_secs_f64(), word_count, transcript));
    }

    // Every point has to have produced a real transcription, or a "fast"
    // thread count is just one that failed quietly.
    let allowed_max = 3.0 + duration_seconds * 4.0;
    for (n_threads, _, word_count, _) in &results {
        assert!(
            f64::from(u32::try_from(*word_count).unwrap_or(u32::MAX)) <= allowed_max,
            "{n_threads} threads produced {word_count} words, over the plausibility ceiling \
             of {allowed_max:.0} -- a repetition runaway, not a transcription"
        );
        assert!(
            *word_count > 20,
            "{n_threads} threads produced only {word_count} words from {duration_seconds}s \
             of real speech"
        );
    }

    // Thread count changes the order floating-point reductions happen in, so
    // it *can* change a greedy decode. Whether it does on this model matters
    // well beyond this sweep: every pinned-transcript proof in this module
    // was recorded at 4 threads, and if the decode is thread-dependent then
    // raising the default silently invalidates all of them. Reported rather
    // than asserted -- the pinned proofs are where that claim belongs, and
    // this sweep's job is to say whether they need re-running.
    let baseline = &results[0].3;
    for (n_threads, _, _, transcript) in &results {
        if normalize(transcript) != normalize(baseline) {
            println!("granite threads={n_threads}: TRANSCRIPT DIFFERS from the 4-thread baseline");
        }
    }
    println!("granite 4-thread baseline transcript={baseline:?}");

    let (best_threads, best_seconds, _, _) = results
        .iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .expect("the sweep ran at least one configuration");
    let baseline_seconds = results[0].1;
    println!(
        "granite sweep: fastest at {best_threads} threads, {best_seconds:.2}s versus the \
         shipped 4-thread {baseline_seconds:.2}s ({:.2}x)",
        baseline_seconds / best_seconds
    );
}

/// Phase 10 item 14, the correctness half: `max_new_tokens` is a **silent**
/// ceiling, and this pins both that it is real and that the shipped value
/// clears it.
///
/// The generation loop stops on reaching `first_position + max_new_tokens`
/// (`GraniteModel::transcribe`) with no error, no end-of-generation token and
/// nothing that distinguishes "the model finished" from "the model was cut
/// off mid-clause". The old default of 512 was closer to biting than anyone
/// had checked: this 120 s clip needs ~400 tokens for its 312 words, about
/// 25% of headroom, and a 4-minute dictation would have lost roughly a third
/// of itself this way. Nothing downstream reliably catches that either -- a
/// truncation is perfectly *precise*, so of the three `FinalDivergence`
/// metrics only retention exposes it. Those metrics remain measurement-only;
/// the reference-free plausibility gate is the sole delivery guard.
///
/// Three configurations on one resident model, exactly as the worker drives
/// it:
///
/// - **shipped** -- `GraniteOptions::default()`, which must transcribe the
///   whole clip.
/// - **starved** -- a deliberately tiny budget, which must truncate *and
///   return `Ok`*, because that silent success is the whole hazard. A test
///   that only proved the shipped value works would go on passing if some
///   future change made truncation impossible to trigger, and would
///   therefore stop testing anything.
/// - **generous** -- four times the shipped budget in a much larger context,
///   which must agree with shipped word for word. Disagreement means shipped
///   is still clipping something.
#[test]
#[ignore = "hardware: needs .tools/granite-speech-4.1-2b/ and .tools/downloads/Obama.wav; run alone"]
fn the_shipped_granite_generation_budget_clears_a_two_minute_utterance() {
    let model_path = shipped_model();
    let projector_path = granite_dir().join("mmproj-model-f16.gguf");
    let source_path = obama_wav();
    for path in [&model_path, &projector_path, &source_path] {
        assert!(
            path.is_file(),
            "missing {}; see this module's documentation",
            path.display()
        );
    }

    let source = read_wave_samples(&source_path);
    let clip = two_minute_clip(&source);

    // Exactly what `workers/granite-worker` runs, save the GPU flag a CPU
    // build ignores anyway.
    let shipped = GraniteOptions {
        use_gpu: false,
        ..GraniteOptions::default()
    };
    let too_small = GraniteOptions {
        max_new_tokens: 128,
        ..shipped.clone()
    };
    let generous = GraniteOptions {
        n_ctx: 16_384,
        max_new_tokens: 8_192,
        ..shipped.clone()
    };

    let model =
        GraniteModel::load(&model_path, &projector_path, &shipped).expect("Granite must load");

    let mut transcripts = Vec::new();
    for (label, options) in [
        ("shipped", &shipped),
        ("too_small", &too_small),
        ("generous", &generous),
    ] {
        let started = Instant::now();
        let transcript = model
            .transcribe_samples(clip, options)
            .unwrap_or_else(|error| panic!("{label} must transcribe: {error}"));
        let elapsed = started.elapsed();
        println!(
            "granite {label} (n_ctx={} max_new_tokens={}): transcribe={elapsed:?} words={}",
            options.n_ctx,
            options.max_new_tokens,
            words(&transcript).len()
        );
        println!("{label} tail={:?}", tail_words(&transcript, 12));
        transcripts.push(transcript);
    }
    let (shipped_text, truncated_text, generous_text) =
        (&transcripts[0], &transcripts[1], &transcripts[2]);

    // The hazard itself: a budget too small to finish returns `Ok` with a
    // short transcript rather than reporting anything. If this ever starts
    // failing because `starved` came back complete, the ceiling stopped
    // being reachable and the shipped headroom below needs re-deriving, not
    // this assertion relaxing.
    assert!(
        words(truncated_text).len() < words(shipped_text).len(),
        "a 128-token budget returned {} words against shipped's {} -- the truncation this \
         test characterizes did not happen, so the headroom claim below is unproven",
        words(truncated_text).len(),
        words(shipped_text).len()
    );

    // And the shipped budget is genuinely clear of it: quadrupling the budget
    // in a 4x context changes nothing.
    assert_eq!(
        normalize(shipped_text),
        normalize(generous_text),
        "the shipped generation budget produced a DIFFERENT transcript than an 8192-token one \
         on the same 120s clip -- the shipped path is truncating real dictations with no error \
         and no disclosure"
    );
}
