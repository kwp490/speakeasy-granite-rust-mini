//! IBM Granite Speech 4.1 on llama.cpp — the engine behind the *delivered*
//! transcript.
//!
//! # Why this is a crate of its own
//!
//! llama.cpp is compiled from source by `llama-cpp-sys-2` through `CMake`, so
//! anything that links this crate needs a C++ toolchain to build. Keeping it in
//! a crate of its own confines that prerequisite to the crates that actually
//! want Granite: `speakeasy-worker`, the boundary the desktop app drives, links
//! no native libraries at all and checks in seconds.
//!
//! The split was originally against `speakeasy-asr`, which owned a streaming
//! sherpa-onnx recognizer beside this one. That crate and its engine are gone;
//! the split still earns its keep for the toolchain reason above.
//!
//! # What it is for
//!
//! **The delivered transcript, with nothing behind it.** Granite runs once over
//! the retained recording after the user stops, and that single pass produces
//! the transcript, its punctuation and its casing together. There is no
//! streaming pass to fall back to and no second engine: a pass that fails, or
//! returns nothing, ends the dictation with a named reason.
//!
//! # Residency
//!
//! [`GraniteModel::load`] loads the ~2 GB of weights once; [`GraniteModel`]'s
//! own `transcribe_wav_file`/`transcribe_samples` methods reuse that loaded
//! model and projector across as many utterances as the caller likes, each
//! getting its own context (KV cache starts empty per call — nothing about an
//! utterance's generation state is shared with the next one). This is the
//! shape `workers/granite-worker` runs in production: load once at
//! `LoadModel`, transcribe many times at `FinishStream`, never reloading
//! between dictations.
//!
//! The free functions [`transcribe_wav_file`] and [`transcribe_samples`] are
//! the one-shot equivalent — load, transcribe once, tear down — kept for
//! tests and ad hoc tooling that only need a single utterance and would
//! rather not construct a [`GraniteModel`] first.

#![allow(clippy::must_use_candidate)]

use std::error::Error;
use std::ffi::CString;
use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::Path;
use std::sync::OnceLock;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::mtmd::{
    MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText, mtmd_default_marker,
};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

#[cfg(test)]
mod granite_smoke;

/// The instruction Granite Speech is prompted with to transcribe rather than
/// summarise or answer.
///
/// The shipped path makes one audio-grounded generation turn. It does not add
/// a second text-only polish turn, because that turn has no audio grounding and
/// would require a separate prompt contract.
///
/// Taken verbatim from the model card's own `llama-cli` invocation, because the
/// prompt is part of the model's interface: this checkpoint is an instruction
/// model whose audio behaviour is selected by what you ask it for, and asking
/// differently gets different behaviour rather than a different phrasing of the
/// same behaviour.
pub const TRANSCRIBE_PROMPT: &str =
    "transcribe the speech with proper punctuation and capitalization.";

/// Whether llama.cpp's CUDA backend is compiled into this build.
///
/// The single source of truth for a fact that only the binary knows: the
/// backend is linked in by the `cuda` feature rather than loaded beside the
/// executable, so nothing about the filesystem can tell, and a host that
/// guesses will report the wrong device. `GraniteOptions::default` reads it for
/// `use_gpu`, and `workers/granite-worker` reports it over the wire at `Hello`
/// so the desktop app can log which device Granite actually ran on.
///
/// Deliberately one constant rather than two `cfg!`s: the day those two
/// disagree, the app claims a device the worker is not using.
pub const CUDA_ENABLED: bool = cfg!(feature = "cuda");

/// Tokens per `llama_decode` while evaluating the prompt and audio embedding.
///
/// Kept at or below llama.cpp's own default `n_batch` so the context does not
/// have to be reconfigured to accept it.
const EVAL_BATCH_TOKENS: i32 = 512;

/// Initial buffer handed to `token_to_piece_bytes`, which reports the size it
/// actually needed when this is too small.
const TOKEN_PIECE_BUFFER: usize = 8;

/// Which stage of loading or running Granite failed.
///
/// One variant per stage rather than one opaque error, because these fail for
/// genuinely different reasons and the `FinalAsr` adapter that wraps this will
/// have to tell "the model file is not on disk" from "the projector refused the
/// audio". The `code` strings follow the convention `EngineChoiceReason::code`
/// established for engine disclosure: stable, `snake_case`, and safe to put in a
/// diagnostic log because they name no path and carry no transcript.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraniteStage {
    /// llama.cpp's global backend could not be initialised.
    Backend,
    /// The model GGUF could not be loaded.
    ModelLoad,
    /// The audio projector GGUF could not be loaded.
    ProjectorLoad,
    /// The projector loaded but does not do audio — a vision-only `mmproj`, or
    /// the wrong file for this model.
    AudioUnsupported,
    /// The audio could not be decoded into samples.
    AudioDecode,
    /// The prompt and audio could not be turned into chunks.
    Tokenize,
    /// Evaluating those chunks failed.
    Evaluate,
    /// Generation failed part-way through.
    Generate,
    /// The generated tokens were not valid UTF-8.
    Detokenize,
}

impl GraniteStage {
    /// A stable code for this stage, safe for logs.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Backend => "granite_backend_unavailable",
            Self::ModelLoad => "granite_model_load_failed",
            Self::ProjectorLoad => "granite_projector_load_failed",
            Self::AudioUnsupported => "granite_projector_has_no_audio",
            Self::AudioDecode => "granite_audio_decode_failed",
            Self::Tokenize => "granite_tokenize_failed",
            Self::Evaluate => "granite_evaluate_failed",
            Self::Generate => "granite_generate_failed",
            Self::Detokenize => "granite_detokenize_failed",
        }
    }
}

/// A Granite failure, with the stage it happened at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraniteError {
    stage: GraniteStage,
    detail: String,
}

impl GraniteError {
    fn at(stage: GraniteStage, detail: impl Display) -> Self {
        Self {
            stage,
            detail: detail.to_string(),
        }
    }

    /// Which stage failed.
    pub const fn stage(&self) -> GraniteStage {
        self.stage
    }

    /// The underlying failure, for a developer rather than a user.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl Display for GraniteError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage.code(), self.detail)
    }
}

impl Error for GraniteError {}

/// How to run one transcription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraniteOptions {
    /// What to ask the model to do with the audio.
    pub prompt: String,
    /// Threads for both the projector and the language model.
    pub n_threads: i32,
    /// Context window. Has to hold the audio embedding plus the prompt plus
    /// whatever is generated.
    pub n_ctx: u32,
    /// Upper bound on generated tokens, so a model that fails to emit
    /// end-of-generation cannot run forever.
    pub max_new_tokens: usize,
    /// Whether to offload to the GPU. Only meaningful when this crate is built
    /// with the `cuda` feature; without it there is no GPU backend to offload
    /// to and this is ignored by llama.cpp.
    pub use_gpu: bool,
}

/// Threads to transcribe with, derived from the machine rather than fixed.
///
/// # Why this is not simply "all of them"
///
/// Measured on this project's development machine (i9-14900KF, 8 P-cores +
/// 16 E-cores = 24 cores / 32 logical) against a 120 s utterance.
/// `granite_thread_count_sweep_on_a_two_minute_utterance` in this crate's
/// `granite_smoke` module is the rig, and re-running it is how these numbers
/// get revised rather than argued about:
///
/// | threads | transcribe | versus 4 |
/// | --- | --- | --- |
/// | 4 (the old default) | 30.8–32.0 s | — |
/// | 8 | 20.3–21.3 s | ~1.5x faster |
/// | 12 | 20.9–21.8 s | ~1.5x faster |
/// | 16 | 19.7–20.3 s | ~1.6x faster |
/// | 20 | 20.1 s | ~1.6x faster |
/// | 24 | 25.5 s | slower again |
/// | 32 | **128.0 s** | **4x slower than 4** |
///
/// Two things that heuristic has to respect. **8 through 20 is one flat
/// plateau**, not a ramp — the spread across it is inside this rig's own
/// ~5% run-to-run variance, so the extra threads past 8 buy nothing
/// measurable. And **oversubscribing is catastrophic, not merely
/// suboptimal**: asking for all 32 logical processors was four times slower
/// than the under-provisioned default it replaced, because this is a hybrid
/// part and llama.cpp's thread pool runs at the speed of its slowest worker
/// — the E-cores and the hyperthread siblings stall the P-cores waiting on
/// every barrier. Even 24, the true physical core count, is already past the
/// peak.
///
/// So the sensible default is the *bottom* of the plateau, not the top: it
/// is the same speed, and it leaves the rest of the machine to the streaming
/// worker, the Tauri host, and whatever the user is actually typing into.
///
/// Halving the logical count approximates physical cores on the ordinary
/// hyperthreaded parts this ships to, and the cap keeps a 64- or 128-thread
/// machine from walking off the same cliff this one has. An unanswerable
/// probe reads as a small machine, which is the fail-safe direction: too few
/// threads is 1.5x slow, too many is 4x slow.
fn recommended_thread_count() -> i32 {
    let logical = std::thread::available_parallelism().map_or(2, NonZeroUsize::get);
    i32::try_from((logical / 2).clamp(1, 8)).unwrap_or(4)
}

impl Default for GraniteOptions {
    fn default() -> Self {
        Self {
            prompt: TRANSCRIBE_PROMPT.to_owned(),
            n_threads: recommended_thread_count(),
            n_ctx: 4096,
            // Deliberately far above what any utterance that *fits* `n_ctx`
            // could legitimately need, so that `n_ctx` is always the binding
            // constraint rather than this one. That ordering is the whole
            // point, because the two run out in opposite ways: overrunning
            // `n_ctx` fails the decode loudly (`GraniteStage::Generate`),
            // while overrunning this budget just stops generating and
            // returns the truncated text as though the model had finished.
            //
            // Measurement found the old 512 close enough to matter: a
            // 120 s utterance needs ~400 tokens for its 312 words, ~25%
            // short of the ceiling, and a 4-minute one would have been cut
            // by roughly a third with no error and no disclosure. The
            // reference metrics expose the problem in measurement, but never
            // gate delivery: a truncation has perfect *precision*, so only
            // retention moves against it.
            //
            // Costs nothing when generation ends normally: the loop breaks
            // on the end-of-generation token, so this bound is only ever
            // reached by a model that has stopped producing one. As a
            // runaway guard it is still tight enough to matter -- 2,048
            // tokens is ~1,500 words, which `final_divergence::is_plausible`
            // rejects outright for any dictation-length clip.
            max_new_tokens: 2048,
            // Asking for the GPU without the feature that provides one would be
            // a silent no-op, so the default states what this build can do.
            use_gpu: CUDA_ENABLED,
        }
    }
}

/// llama.cpp's process-global backend.
///
/// `LlamaBackend::init` refuses a second call — it guards on a process-wide
/// `AtomicBool` — so this is initialised once and never dropped. Never dropping
/// it is correct rather than a leak: the alternative is freeing the backend
/// while a model loaded from it may still be alive.
fn backend() -> Result<&'static LlamaBackend, GraniteError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| GraniteError::at(GraniteStage::Backend, error))
}

/// A Granite Speech model and its audio projector, loaded once and ready to
/// transcribe many independent utterances without reloading the ~2 GB of
/// weights each time.
///
/// Only [`GraniteOptions::use_gpu`] and [`GraniteOptions::n_threads`] take
/// effect at [`GraniteModel::load`] — they configure the model and the
/// projector, which are fixed for this value's lifetime.
/// [`GraniteOptions::n_ctx`], [`GraniteOptions::max_new_tokens`] and
/// [`GraniteOptions::prompt`] are read again on every call to
/// `transcribe_wav_file`/`transcribe_samples`, since each utterance gets its
/// own context and generation.
///
/// `LlamaModel` and `MtmdContext` are both `Send + Sync` (neither borrows the
/// other at the Rust type level — `MtmdContext` holds its own FFI handle), so
/// this type is too, with no unsafe impl of its own needed.
pub struct GraniteModel {
    model: LlamaModel,
    projector: MtmdContext,
}

impl GraniteModel {
    /// Loads the model and its audio projector.
    ///
    /// # Errors
    ///
    /// Returns a [`GraniteError`] naming the stage that failed — see
    /// [`GraniteStage::Backend`], [`GraniteStage::ModelLoad`],
    /// [`GraniteStage::ProjectorLoad`] and [`GraniteStage::AudioUnsupported`].
    pub fn load(
        model_gguf: &Path,
        projector_gguf: &Path,
        options: &GraniteOptions,
    ) -> Result<Self, GraniteError> {
        let backend = backend()?;

        let mut model_params = LlamaModelParams::default();
        if options.use_gpu {
            model_params = model_params.with_n_gpu_layers(u32::MAX);
        }
        let model = LlamaModel::load_from_file(backend, model_gguf, &model_params)
            .map_err(|error| GraniteError::at(GraniteStage::ModelLoad, error))?;

        let marker = CString::new(mtmd_default_marker())
            .map_err(|error| GraniteError::at(GraniteStage::ProjectorLoad, error))?;
        let projector_path = utf8_path(projector_gguf, GraniteStage::ProjectorLoad)?;
        let projector = MtmdContext::init_from_file(
            projector_path,
            &model,
            &MtmdContextParams {
                use_gpu: options.use_gpu,
                print_timings: false,
                n_threads: options.n_threads,
                media_marker: marker,
                image_min_tokens: -1,
                image_max_tokens: -1,
            },
        )
        .map_err(|error| GraniteError::at(GraniteStage::ProjectorLoad, error))?;

        // A vision-only projector would tokenize and evaluate without complaint
        // and return text generated from the prompt alone, which reads exactly
        // like a bad transcription. Refusing here makes that failure legible.
        if !projector.support_audio() {
            return Err(GraniteError::at(
                GraniteStage::AudioUnsupported,
                "the projector reports no audio support",
            ));
        }

        Ok(Self { model, projector })
    }

    /// Transcribes an audio file against this already-loaded model.
    ///
    /// # Errors
    ///
    /// See [`transcribe_wav_file`].
    pub fn transcribe_wav_file(
        &self,
        audio: &Path,
        options: &GraniteOptions,
    ) -> Result<String, GraniteError> {
        self.transcribe(options, |projector| {
            let audio_path = utf8_path(audio, GraniteStage::AudioDecode)?;
            MtmdBitmap::from_file(projector, audio_path, false)
                .map_err(|error| GraniteError::at(GraniteStage::AudioDecode, error))
        })
    }

    /// Transcribes already-decoded 16 kHz mono PCM against this already-loaded
    /// model.
    ///
    /// # Errors
    ///
    /// See [`transcribe_samples`].
    pub fn transcribe_samples(
        &self,
        samples: &[f32],
        options: &GraniteOptions,
    ) -> Result<String, GraniteError> {
        self.transcribe(options, |_projector| {
            MtmdBitmap::from_audio_data(samples)
                .map_err(|error| GraniteError::at(GraniteStage::AudioDecode, error))
        })
    }

    /// Shared per-utterance path behind `transcribe_wav_file` and
    /// `transcribe_samples`. The two differ only in how the bitmap is built —
    /// from a file path via llama.cpp's own decoder, or from samples the
    /// caller already has in memory — so that is the one thing the closure
    /// supplies. Everything here is single-use per utterance: a fresh
    /// context (KV cache starts empty), sampler and batch, same as the
    /// one-shot free function used to build per call before this type
    /// existed. Only `self.model` and `self.projector` are reused.
    fn transcribe(
        &self,
        options: &GraniteOptions,
        build_bitmap: impl FnOnce(&MtmdContext) -> Result<MtmdBitmap, GraniteError>,
    ) -> Result<String, GraniteError> {
        let backend = backend()?;

        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(options.n_ctx))
            .with_n_threads(options.n_threads)
            .with_n_threads_batch(options.n_threads);
        let mut context = self
            .model
            .new_context(backend, context_params)
            .map_err(|error| GraniteError::at(GraniteStage::ModelLoad, error))?;

        let bitmap = build_bitmap(&self.projector)?;

        let chunks = self
            .projector
            .tokenize(
                MtmdInputText {
                    text: build_prompt(&options.prompt),
                    add_special: true,
                    parse_special: true,
                },
                &[&bitmap],
            )
            .map_err(|error| GraniteError::at(GraniteStage::Tokenize, error))?;

        // A KV-cache coordinate, not a loop index: generation continues from
        // wherever evaluating the prompt and the audio chunks left off, which is
        // why this is a range starting at an offset rather than a counter from
        // zero.
        let first_position = chunks
            .eval_chunks(&self.projector, &context, 0, 0, EVAL_BATCH_TOKENS, true)
            .map_err(|error| GraniteError::at(GraniteStage::Evaluate, error))?;

        // Saturating rather than fallible: `max_new_tokens` is a budget, so
        // clamping an absurd value to the end of the position space reads it
        // correctly. There is no configuration worth a panic here.
        let position_limit = first_position
            .saturating_add(i32::try_from(options.max_new_tokens).unwrap_or(i32::MAX));

        // Greedy, not sampled: a transcript is a reading of what was said, and
        // the test that pins this asserts a whole transcript, which needs the
        // decode to be reproducible.
        let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
        let mut generated: Vec<u8> = Vec::new();
        let mut batch = LlamaBatch::new(1, 1);

        for position in first_position..position_limit {
            // -1 is llama.cpp's "the logits from the last evaluated position".
            let token = sampler.sample(&context, -1);
            sampler.accept(token);
            if self.model.is_eog_token(token) {
                break;
            }
            generated.extend_from_slice(&piece_bytes(&self.model, token)?);

            batch.clear();
            batch
                .add(token, position, &[0], true)
                .map_err(|error| GraniteError::at(GraniteStage::Generate, error))?;
            context
                .decode(&mut batch)
                .map_err(|error| GraniteError::at(GraniteStage::Generate, error))?;
        }

        // Decoded once, at the end, rather than per token: a single character
        // can span two tokens, so decoding each token alone would corrupt
        // exactly the non-ASCII text this product has to keep intact.
        String::from_utf8(generated)
            .map_err(|error| GraniteError::at(GraniteStage::Detokenize, error))
    }
}

/// Transcribes an audio file with Granite Speech, returning the text.
///
/// `audio` is decoded by llama.cpp's own helper, which reads wav, mp3 and flac
/// and detects the format from magic bytes rather than the extension.
///
/// This is the one-shot equivalent of [`GraniteModel::load`] followed by
/// [`GraniteModel::transcribe_wav_file`] — kept for tests and ad hoc tooling
/// that only need a single utterance. Production code that will run more
/// than one utterance should load a [`GraniteModel`] once instead; see
/// `workers/granite-worker`.
///
/// # Errors
///
/// Returns a [`GraniteError`] naming the stage that failed — see
/// [`GraniteStage`]. Notably, a projector that does not support audio is
/// reported as [`GraniteStage::AudioUnsupported`] rather than producing empty
/// text, because the two are indistinguishable to a caller otherwise.
pub fn transcribe_wav_file(
    model_gguf: &Path,
    projector_gguf: &Path,
    audio: &Path,
    options: &GraniteOptions,
) -> Result<String, GraniteError> {
    GraniteModel::load(model_gguf, projector_gguf, options)?.transcribe_wav_file(audio, options)
}

/// Transcribes already-decoded 16 kHz mono PCM with Granite Speech.
///
/// This is what a supervised worker process wants: the framed protocol hands
/// over raw `f32` samples in `[-1, 1]` accumulated from `PushAudio`, and
/// `MtmdBitmap::from_audio_data` accepts them directly, so no temporary WAV
/// file has to be written to disk first. Resampling is the caller's
/// responsibility — this crate assumes 16 kHz because that is what Granite
/// Speech's projector was trained on, and validates nothing about the rate
/// because it has no way to know what rate the samples actually are.
///
/// This is the one-shot equivalent of [`GraniteModel::load`] followed by
/// [`GraniteModel::transcribe_samples`] — see that type's docs for the
/// resident, load-once-transcribe-many shape `workers/granite-worker` runs in
/// production.
///
/// # Errors
///
/// Returns a [`GraniteError`] naming the stage that failed — see
/// [`GraniteStage`] and [`transcribe_wav_file`].
pub fn transcribe_samples(
    model_gguf: &Path,
    projector_gguf: &Path,
    samples: &[f32],
    options: &GraniteOptions,
) -> Result<String, GraniteError> {
    GraniteModel::load(model_gguf, projector_gguf, options)?.transcribe_samples(samples, options)
}

/// The prompt as Granite Speech expects to receive it.
///
/// # Why the turn structure is not optional
///
/// Fed a bare instruction, this model emits end-of-generation as its *first*
/// sampled token and the transcript comes back empty — with the audio having
/// been read, projected and decoded perfectly (102 audio tokens, encoded in
/// 1.5 s, measured 2026-08-03). Nothing errors. The open `ASSISTANT:` tag is
/// what tells the model it is its turn to speak; without it the likeliest
/// continuation of the document is the end of it.
///
/// # Why this is written out rather than rendered
///
/// This is the chat template embedded in the GGUF itself, read out of its
/// metadata:
///
/// ```jinja
/// {% for message in messages %}{% if message['role'] == 'user' %}USER: {{ message['content'] }}
///  ASSISTANT:{% elif message['role'] == 'assistant' %}{{ message['content'] }}{% endif %}{% endfor %}
/// ```
///
/// llama.cpp cannot render it: `llama_chat_apply_template` recognises a fixed
/// set of template families and returns -1 for this one, so
/// `LlamaModel::apply_chat_template` fails rather than falling back. The
/// alternative to these two literals is taking a Jinja interpreter as a
/// dependency, which is a poor trade for `USER:` and `ASSISTANT:`.
///
/// Hardcoding a model's prompt format is normally how you get a silent
/// mismatch after an upstream re-publish. It is safe *here* because the bytes
/// are pinned by length and SHA-256 in `models/trusted-manifest.json` — the
/// same pin that makes the weights trustworthy also freezes the template, so
/// the two cannot drift apart. Re-read the template if that pin is ever moved.
///
/// Note the leading space before `ASSISTANT:` is the template's own, not a typo.
fn build_prompt(instruction: &str) -> String {
    format!(
        "USER: {}\n{}\n ASSISTANT:",
        mtmd_default_marker(),
        instruction
    )
}

/// One token's bytes, retrying with the size llama.cpp asks for when the first
/// buffer is too small.
fn piece_bytes(model: &LlamaModel, token: LlamaToken) -> Result<Vec<u8>, GraniteError> {
    match model.token_to_piece_bytes(token, TOKEN_PIECE_BUFFER, false, None) {
        Ok(bytes) => Ok(bytes),
        Err(llama_cpp_2::TokenToStringError::InsufficientBufferSpace(needed)) => {
            let needed = usize::try_from(needed.abs())
                .map_err(|error| GraniteError::at(GraniteStage::Detokenize, error))?;
            model
                .token_to_piece_bytes(token, needed, false, None)
                .map_err(|error| GraniteError::at(GraniteStage::Detokenize, error))
        }
        Err(error) => Err(GraniteError::at(GraniteStage::Detokenize, error)),
    }
}

/// llama.cpp's C API takes paths as C strings, so a non-UTF-8 path cannot be
/// passed through at all — it fails here rather than somewhere less obvious.
fn utf8_path(path: &Path, stage: GraniteStage) -> Result<&str, GraniteError> {
    path.to_str()
        .ok_or_else(|| GraniteError::at(stage, "path is not valid UTF-8"))
}
