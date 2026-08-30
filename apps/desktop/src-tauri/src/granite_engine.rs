//! Granite Speech as the desktop's only final-pass engine.
//!
//! Resident across dictations: `GraniteEngineCoordinator` spawns
//! `workers/granite-worker` once and keeps the process (and its loaded ~2 GB
//! model) alive rather than spawning and tearing it down per dictation. The
//! shape was copied from the streaming engine's own resident-worker
//! coordinator, which left with that engine.
//!
//! `verify_pack_files` runs once, at warm time, not on every dictation —
//! `WorkerFinalAdapter::run_locked` still sends `LoadModel` before every
//! single dictation regardless (it has no way to know a model is already
//! resident), but `workers/granite-worker`'s `load_model` recognises a repeat
//! request for the same artifact and skips reloading, so that per-dictation
//! `LoadModel` is a no-op fast path rather than a second ~2 GB load.
//!
//! Granite's `model_root` is resolved directly from the same app-owned
//! `<models>/<pack>/<revision>` layout `InstallManager` activates.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use speakeasy_domain::{
    AsrExecution, AsrFeature, AsrLanguage, AsrRequest, AsrStreaming, AsrTask, CancelToken,
    Deadline, DomainError, EngineCapabilities, ErrorCode, FinalAsr, FinalTranscript, SystemClock,
    UtteranceAudio,
};
use speakeasy_models::{
    CudaContextProbe, CudaContextProof, ExecutionProvider, GpuProbe, InstallSpec, NvmlGpuProbe,
    Pack, PackRole, TrustedManifest, admit, bundled_manifest, prove_cuda_context,
    verify_pack_files,
};
use speakeasy_windows::{CrashThrottle, ProcessDeadlines, ProcessSupervisor};
use speakeasy_worker::{WorkerClient, WorkerCommand, WorkerEvent, WorkerFinalAdapter};

use speakeasy_windows::ProcessWorkerClient;

/// Why a dictation is running on the provider it is running on.
///
/// The reason travels with the choice rather than being re-derived by whoever
/// displays it, because after a fallback the honest answer is not deducible
/// from the provider alone: "running on CPU" is the same value whether this
/// machine has no GPU or has a perfectly good one whose worker was never
/// built, and those two owe the user different sentences.
///
/// This lived with the streaming engine and was shared by both. There is one
/// engine now, so it lives with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineChoiceReason {
    /// The GPU probe's preferred provider, and its pack is installed.
    ProbePreferred,
    /// The probe preferred CUDA, but the CUDA pack is not installed, so this
    /// runs on the installed CPU pack instead.
    CpuGpuPackNotInstalled,
    /// The probe preferred CUDA and this installation carries no CUDA-capable
    /// Granite worker, so there is no GPU path to take however many CUDA packs
    /// are installed. Today this is the branch every CUDA-capable machine
    /// takes -- Granite's GPU support is a build feature, not a download.
    CpuGpuRuntimeMissing,
}

impl EngineChoiceReason {
    /// A stable code for the UI and the diagnostic log. Never a device name:
    /// this travels into the log, which is a privacy surface.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ProbePreferred => "probe_preferred",
            Self::CpuGpuPackNotInstalled => "cpu_gpu_pack_not_installed",
            Self::CpuGpuRuntimeMissing => "cpu_gpu_runtime_missing",
        }
    }
}

/// What the resident worker turned out to be, as two separate facts.
///
/// Separate because they answer different questions and only one of them is
/// about this run. `compiled_cuda` is a property of the binary, from the startup
/// handshake, and is what pack selection needs: a CUDA-capable binary can take
/// a CUDA pack. `context` is a property of the *process*, from NVML, and is the
/// only thing that says the card is actually being used.
///
/// Collapsing the two is how `device=cuda` came to mean "this binary was built
/// with CUDA" — which is not a device, and is true of a worker that failed to
/// initialize CUDA and ran the whole dictation on the processor. llama.cpp
/// reports that fallback in its own stderr and nowhere a caller can see.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorkerProvider {
    /// Whether a CUDA backend is compiled into this worker, per `Hello`.
    ///
    /// `None` means the handshake did not answer — a pre-v2 binary, or one that
    /// failed before it spoke. Never folded into `false`: "it said no" and "it
    /// did not say" are different, and only the first is a statement about the
    /// binary.
    compiled_cuda: Option<bool>,
    /// Whether NVML places this worker's own process on a device.
    ///
    /// `None` when it was not asked, which is every CPU build: a worker with no
    /// CUDA backend has no context to hold, and asking would turn "there is
    /// nothing to prove" into a driver query on every processor install.
    context: Option<CudaContextProof>,
}

impl WorkerProvider {
    /// The device this worker is **running on**, as a stable code.
    ///
    /// Four values, and the fourth is the one that keeps this honest.
    /// `cuda_unverified` is a CUDA-capable worker whose context could not be
    /// checked, and it is deliberately neither `cuda` nor `cpu`: calling it
    /// `cuda` is the unverified claim this whole path exists to remove, and
    /// calling it `cpu` would report a fault on a machine that is probably
    /// using its card perfectly well behind a driver that would not answer.
    const fn device(self) -> &'static str {
        match (self.compiled_cuda, self.context) {
            (None, _) => "unknown",
            (Some(true), Some(CudaContextProof::Holding)) => "cuda",
            // A binary with no CUDA backend, and a CUDA binary NVML says is not
            // on a device: one arm, because the answer is the same fact -- this
            // run is on the processor -- and the two reasons for it are carried
            // by `WorkerProvider`'s own fields rather than by this string.
            (Some(false), _) | (Some(true), Some(CudaContextProof::NotHolding)) => "cpu",
            (Some(true), Some(CudaContextProof::ProbeUnavailable(_)) | None) => "cuda_unverified",
        }
    }

    /// Whether this run is provably on the graphics card.
    const fn proved_graphics_card(self) -> bool {
        matches!(
            (self.compiled_cuda, self.context),
            (Some(true), Some(CudaContextProof::Holding))
        )
    }

    /// Whether this run is provably **not** on the graphics card.
    ///
    /// Deliberately not `!proved_graphics_card()`. Three states exist and only
    /// two of them are answers: a binary with no CUDA backend cannot be on the
    /// card, and NVML listing no context for this pid is the definitive
    /// negative, but a probe that could not be asked -- and a worker that never
    /// answered its handshake at all -- prove nothing in either direction.
    /// Collapsing those into the negative is what made
    /// [`ProviderIntegrity::GpuInstallNotOperational`] tell a user their
    /// dictation had moved to the processor on the strength of a failed driver
    /// query, which is the one inference `speakeasy_models::granite_gpu`'s own
    /// header forbids.
    const fn disproved_graphics_card(self) -> bool {
        matches!(
            (self.compiled_cuda, self.context),
            (Some(false), _) | (Some(true), Some(CudaContextProof::NotHolding))
        )
    }
}

/// Whether what setup recorded still describes what is running.
///
/// The field that made the original defect visible and did nothing about it. A
/// support log read `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`:
/// every value correct, the combination impossible, and no code anywhere looked
/// at the three together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderIntegrity {
    /// Setup recorded no configuration, so there is nothing to check. An
    /// installation placed by hand, or one whose engine check never ran.
    Unrecorded,
    /// What is running is what was recorded. The quiet, normal case.
    Matches,
    /// **The actionable failure.** Setup recorded a graphics-card installation
    /// and this run is not on the graphics card.
    ///
    /// Reported rather than absorbed. Dictation still works — the same GGUF runs
    /// on the processor and produces the same transcript — so refusing to
    /// transcribe would cost the user their dictation to make a point about
    /// provisioning. What must never happen is the *label* being wrong, and
    /// before this it was: the app reported the installation it was told about
    /// and ran something else.
    GpuInstallNotOperational,
    /// Setup recorded a processor installation and this run is on the graphics
    /// card.
    ///
    /// Not a fault. It is what `scripts/Enable-GraniteCuda.ps1` produces on
    /// purpose — a CUDA worker staged over a CPU install — and the honest thing
    /// is to say so rather than report the record as though it were the truth.
    RunningBeyondRecord,
    /// Setup recorded a graphics-card installation and **this run could not be
    /// checked** — the driver query failed, or the worker never answered its
    /// handshake.
    ///
    /// Added 2026-08-21. Until then this fell into
    /// [`Self::GpuInstallNotOperational`], whose copy tells the user dictation
    /// "is running on the processor instead" — a claim about a device on
    /// evidence that says nothing about any device. The worker is most likely on
    /// the card and the only thing that happened is that NVML would not answer.
    ///
    /// Not a fault, and deliberately not folded into [`Self::Matches`] either:
    /// that would claim an agreement nothing verified, which is the same mistake
    /// pointing the other way. `device=` had this right all along and reported
    /// `cuda_unverified`; it was the comparison one layer up that collapsed it.
    GpuRecordUnconfirmed,
}

impl ProviderIntegrity {
    /// A stable code for the log and the UI catalog.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unrecorded => "unrecorded",
            Self::Matches => "ok",
            Self::GpuInstallNotOperational => "gpu_install_not_operational",
            Self::RunningBeyondRecord => "running_beyond_record",
            Self::GpuRecordUnconfirmed => "gpu_record_unconfirmed",
        }
    }

    /// Whether this is a condition someone has to do something about.
    pub const fn is_fault(self) -> bool {
        matches!(self, Self::GpuInstallNotOperational)
    }
}

/// Compare what setup recorded against what the worker turned out to be.
///
/// `recorded` is the token `install-provider.txt` carries, as
/// `installed_configuration` reads it: `"cpu"`, `"cuda"`, or `"unrecorded"`.
///
/// A free function taking both facts, rather than a method reaching for either,
/// so the combination that produced the original defect is reachable in a test
/// on a machine that cannot produce it.
fn assess_provider_integrity(recorded: &str, worker: WorkerProvider) -> ProviderIntegrity {
    let on_card = worker.proved_graphics_card();
    match recorded {
        // An empty marker file and an absent one are the same statement. The app
        // substitutes `unrecorded` for absent; the empty string is what a
        // half-written file leaves, and guessing either way about it would be a
        // claim about a configuration nobody verified.
        "" | "unrecorded" => ProviderIntegrity::Unrecorded,
        "cuda" if on_card => ProviderIntegrity::Matches,
        // Only the definitive negative is the fault. `disproved_graphics_card`
        // carries why the two are not each other's complement; the effect here
        // is that this arm and `WorkerProvider::device` agree by construction --
        // `cpu` is the fault, `cuda_unverified` and `unknown` are the
        // unconfirmed state, and nothing reports a device it did not establish.
        "cuda" if worker.disproved_graphics_card() => ProviderIntegrity::GpuInstallNotOperational,
        "cuda" => ProviderIntegrity::GpuRecordUnconfirmed,
        _ if on_card => ProviderIntegrity::RunningBeyondRecord,
        _ => ProviderIntegrity::Matches,
    }
}

/// The resident adapter type: the generic client/clock pair the streaming
/// coordinator also used for its own resident worker.
type ResidentGraniteAdapter =
    WorkerFinalAdapter<ProcessWorkerClient<SystemClock>, Arc<SystemClock>>;

/// Bound on getting the resident worker up and the model loaded. Inherited from
/// the streaming engine's `WARM_TIMEOUT` — Granite's own measured load is
/// far under this (~3.5 s on this machine), so the margin is generous on
/// purpose rather than tuned to the measurement.
const GRANITE_WARM_TIMEOUT: Duration = Duration::from_mins(1);

/// The literal `workers/granite-worker` checks its `LoadModel` command
/// against. Distinct on purpose from the manifest pack's own id — they answer
/// different questions (which model the worker loaded, versus which
/// install/quantization/provider variant the manifest pinned), and the pack
/// id carries a provider suffix this does not. The two are held in step by
/// `the_worker_artifact_id_and_the_install_eligible_pack_agree` below, since
/// the worker crate deliberately links no manifest reader of its own.
const GRANITE_WORKER_ARTIFACT_ID: &str = "granite-speech-4.1-2b-q4_k_m";

/// Whether this installation could execute Granite on the GPU at all.
///
/// The streaming engine asks `RuntimeWizardCoordinator::cuda_runtime_available`,
/// which re-stats the ONNX Runtime CUDA execution provider's fifteen files on
/// every call. Granite's equivalent question is not answerable the same way,
/// and not answerable at all today: llama.cpp's CUDA backend is compiled
/// *into* `speakeasy-granite-worker.exe` by the crate's `cuda` feature, not
/// loaded beside it, so there is no file to stat. Packaging never requests the
/// feature, so no *installed* build has ever carried a CUDA worker.
///
/// So this is a constant `false` rather than a probe, and it is a parameter of
/// [`choose_granite_pack`] rather than read there, so the CUDA-preferring
/// branches are exercised by tests on hardware that cannot produce them.
///
/// # Superseded 2026-08-10 — the worker answers this now
///
/// This was a hardcoded `false`, and it became wrong the day
/// `scripts/Enable-GraniteCuda.ps1` staged a `--features cuda` worker over an
/// installed CPU one. Nothing behaved wrongly — the worker decides for itself
/// via `speakeasy_granite::CUDA_ENABLED`, and there is only one Granite pack to
/// choose between — but `granite_warm` logged `engine=cpu_gpu_runtime_missing`
/// while `nvidia-smi` showed the worker holding a CUDA context. A diagnostic
/// that confident and that wrong is worse than none.
///
/// The fix this comment prescribed for two phases is now built: the worker
/// reports `compiled_accelerators` at `Hello` (worker protocol v2), because
/// nothing about the filesystem can tell. `GraniteEngineCoordinator` caches
/// the answer and pack selection uses it.
///
/// What remains, and why this constant still exists as documentation rather
/// than code: the *first* selection of a process happens before any worker
/// exists, so it still has to assume. It assumes `false`, the conservative
/// answer, and `warm_granite_engine` re-selects once the worker has spoken so
/// the logged reason is the corrected one. `device=` in `granite_warm` is the
/// authority on where Granite ran; `engine=` names the pack and why.
const GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS: bool = false;

/// The memory this machine must have before Granite is attempted at all.
///
/// Measured, not guessed at, though the margin above the measurement is a
/// judgment call. On this machine, sampled through a full transcription run:
/// `granite-worker.exe` holds a **3,164 MiB** working set. The floor was set
/// when the streaming worker was resident beside it at another **1,263 MiB** —
/// ~4.3 GiB of workers before the Tauri host, the `WebView2` processes, or
/// Windows. That second worker is gone and the floor is deliberately unchanged:
/// 8 GiB is the smallest round floor that leaves the rest room;
/// below it Granite would be paging, and a model that swaps is far slower than
/// no second pass at all — which is the opposite of what this engine is for.
///
/// Falling short is **not** an error and not a per-dictation disclosure. It is
/// a static property of the machine, the same category as "the GGUFs were
/// never fetched", and that class of condition behaves one way:
/// `Ok(None)`, the ordinary single-engine path, no HUD noise on every
/// dictation forever. It is still not silent — the coordinator records
/// `memory_below_granite_floor` and `lib.rs`'s `granite_warm` logs it once per
/// launch, so a support log distinguishes "too small for Granite" from "never
/// installed Granite" from "running Granite on CPU".
///
/// Distinct from `runtime_wizard`'s `MINIMUM_TOTAL_MEMORY_BYTES`, which gates
/// the whole dictation and is deliberately much lower; see its own comment for
/// why the two must not be one number.
const GRANITE_MINIMUM_TOTAL_MEMORY_BYTES: u64 = 8 * 1_024 * 1_024 * 1_024;

/// Whether this machine can host Granite beside everything a dictation
/// already needs.
///
/// A probe that could not answer (`None`) reads as "no", the same fail-closed
/// reading `RuntimeWizardCoordinator::begin_role` gives it — an unknown memory
/// budget is not evidence of a large one.
const fn granite_memory_is_sufficient(total_memory_bytes: Option<u64>) -> bool {
    match total_memory_bytes {
        Some(bytes) => bytes >= GRANITE_MINIMUM_TOTAL_MEMORY_BYTES,
        None => false,
    }
}

/// How a warm decides whether the bytes under a pack's `model_root` are the
/// bytes the manifest pins.
///
/// A parameter rather than a direct call to [`verify_pack_files`], for the same
/// reason `cuda_context_probe` is one: the claim is *exactly one pass per warm*,
/// and only a count can observe that. Counting the production function from
/// outside counts the caller instead.
///
/// **Not an environment variable and not a feature flag.** Both composition
/// roots name [`TrustedDigestVerifier`] and the binary can name nothing else, so
/// nothing in production can weaken the check.
pub trait PackVerifier: Sync {
    /// `false` when a required file is missing, is the wrong length, or is not
    /// the pinned digest. Which of the three is [`verify_pack_files`]'s to
    /// report; a warm turns all of them into the same install-defect outcome,
    /// so nothing here needs to tell them apart.
    fn bytes_match(&self, pack: &Pack, model_root: &Path) -> bool;
}

/// The only verifier the shipped binary knows about: the manifest's own pinned
/// lengths and SHA-256 digests, read from disk.
pub struct TrustedDigestVerifier;

impl PackVerifier for TrustedDigestVerifier {
    fn bytes_match(&self, pack: &Pack, model_root: &Path) -> bool {
        verify_pack_files(pack, model_root).is_ok()
    }
}

/// Everything about *this machine and this install* that decides whether
/// Granite runs at all, gathered once at the call site.
///
/// A struct rather than four parameters because the two entry points below
/// must answer the identical question — a machine that warms Granite at launch
/// and then declines it per dictation, or the reverse, is a bug that would
/// only show up as "the second pass silently stopped happening". Passing one
/// value makes them impossible to give different inputs by accident.
pub struct GraniteEnvironment<'a> {
    /// The staged worker binary, when this install carries one. `None` on any
    /// install built before the worker was packaged, and on any that dropped it.
    pub granite_worker_exe: Option<&'a Path>,
    /// `ModelCoordinator.root.join("models")` — the same root
    /// `InstallManager::new` takes.
    pub install_root: &'a Path,
    /// What the hardware probe reported, or `None` when it could not answer.
    /// Checked against [`GRANITE_MINIMUM_TOTAL_MEMORY_BYTES`].
    pub total_memory_bytes: Option<u64>,
    /// Where the worker's protocol frames go, when disk logging is on.
    pub diagnostic_log: Option<PathBuf>,
    /// Which configuration setup recorded having *proved* it installed, as the
    /// token `install-provider.txt` carries: `"cpu"`, `"cuda"`, or
    /// `"unrecorded"`.
    ///
    /// Here because it is the only thing that makes a processor run legible: on
    /// a processor installation it is the expected outcome, and on a
    /// graphics-card installation it is a fault, and nothing else in this
    /// environment can tell those apart. Read by `assess_provider_integrity`
    /// once the worker has come up.
    pub recorded_provider: &'a str,
    /// How to ask whether the worker's own process is holding a CUDA context.
    ///
    /// Beside `recorded_provider` deliberately: these are the two halves of the
    /// same comparison, and the probe was the half that was not passed in.
    /// `warm` named `NvmlCudaContextProbe` inline, so the app's `cuda_unverified`
    /// — a CUDA worker whose context could not be checked — was the one device
    /// value no test on any machine could produce. Setup's side of it had been
    /// reachable since `smoke::verify_engine_with` took its probe; the app's had
    /// not, which is how a value shipped with copy, a code and a UI branch and no
    /// proof that anything ever emitted it.
    ///
    /// **Not an environment variable.** A production switch whose only purpose is
    /// to make the app misreport its own provider is the shape of the defect this
    /// module exists to have removed.
    pub cuda_context_probe: &'a dyn CudaContextProbe,
    /// How the warm checks the pack's bytes. `&TrustedDigestVerifier` in the
    /// shipped binary; see [`PackVerifier`] for why it is a parameter rather
    /// than a direct call.
    pub verifier: &'a dyn PackVerifier,
}

/// Bounds the `FinishStream` call, where Granite's ~2 GB load and inference
/// both happen (not `LoadModel`, which only checks file presence). Sized
/// generously against a realistic dictation rather than against the 6.42 s
/// fixture the hardware tests use, whose resident pass measured 2.93 s on the
/// processor and 361 ms on an RTX 4070 Laptop GPU (2026-08-21).
const GRANITE_FINISH_STREAM_DEADLINE: Duration = Duration::from_secs(90);

/// Static identity for the CPU Granite pack. The packs are a fixed,
/// compile-time-known set, which is why `EngineCapabilities`'s fields are
/// `&'static str` at all.
const CPU_PACK_CAPABILITIES: EngineCapabilities = EngineCapabilities {
    execution: AsrExecution::Local,
    streaming: AsrStreaming::Offline,
    sample_rate_hz: 16_000,
    channels: 1,
    language: AsrLanguage::English,
    task: AsrTask::Transcribe,
    features: &[AsrFeature::Punctuation, AsrFeature::KnownLocale],
    runtime: "llama.cpp",
    runtime_abi: "llama-cpp-2-0.1.153-granite",
    provider: "cpu",
    artifact_revision: "q4_k_m-2026-05-11",
    license: "Apache-2.0",
};

/// The capability identity of the Granite pack running on `provider`, or
/// `None` when this project ships no Granite pack for it.
///
/// The streaming engine's equivalent is the shape this mirrors, with one
/// deliberate difference: that one returned a value for every provider because
/// it shipped a real pack for each. Here `None` for CUDA is the honest answer
/// rather than a gap. There is no CUDA Granite GGUF in the manifest and no
/// CUDA-enabled worker binary has ever been compiled
/// ([`GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS`]), so there is no
/// revision to name and no runtime ABI to claim -- and inventing placeholders
/// for a pack that does not exist is exactly how a wrong `artifact_revision`
/// reaches a transcript's provenance. The day both exist, this becomes a second
/// arm holding that pack's real values; [`choose_granite_pack`] already routes
/// around it, so nothing else changes.
const fn capabilities_for(provider: ExecutionProvider) -> Option<EngineCapabilities> {
    match provider {
        ExecutionProvider::Cpu => Some(CPU_PACK_CAPABILITIES),
        ExecutionProvider::Cuda | ExecutionProvider::DirectMl => None,
    }
}

const fn domain_error(code: ErrorCode) -> DomainError {
    DomainError {
        code,
        recoverable: true,
    }
}

/// A crash throttle independent of `RuntimeWizardCoordinator`'s, so a run of
/// Granite crashes never quarantines ordinary dictation and vice versa. It was
/// the second of three when the streaming engine had one too. Persists across
/// dictations, rather than being per-call, because quarantine only means
/// anything if it survives past the one call that tripped it.
///
/// Also owns the resident worker adapter, as the streaming coordinator owned
/// its own: warming is idempotent (a second `ensure_ready` while one is already
/// loaded just clones the `Arc`) and retryable (a failed warm is not cached, so
/// the next dictation tries again rather than latching a permanent failure).
/// Which artifact a warm's digest pass hashed, and what it concluded.
///
/// The identity is carried rather than derived. `settle_after_warm` promotes a
/// *pack* to `verified_on_disk` and it re-resolves which pack that is, using the
/// post-warm CUDA answer -- which is precisely what the warm can change. So "the
/// warm said ready" and "this pack's bytes were checked" are two claims, and a
/// promotion needs the second. Matching id *and* revision is required: the same
/// pack at a new revision is different bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WarmVerification {
    /// No digest pass ran this warm -- no worker, memory below the floor,
    /// nothing configured, or the engine is quarantined. The model on disk is
    /// neither proven nor suspect; nobody looked.
    NotAttempted,
    /// These exact bytes matched the manifest, immediately before the worker
    /// was handed the model root, **during this invocation**.
    Verified { pack_id: String, revision: String },
    /// This invocation hashed nothing, because the adapter was already loaded;
    /// the identity is what an *earlier* pass in this process verified.
    ///
    /// Its own variant rather than folded into `Verified`, because they are
    /// different claims. Both still promote, and both are still compared against
    /// the resolved pack.
    AlreadyLoaded { pack_id: String, revision: String },
    /// A different pack is loaded than the one this invocation resolved.
    ///
    /// [`Self::identity`] is `None`, so neither pack can be promoted or
    /// condemned: the resident pack's digests describe a pack the resolver no
    /// longer points at, and the requested pack has no digests at all.
    /// `ensure_ready` refuses on this, hands back no adapter, and clears the
    /// slot so the next call warms the requested pack.
    ///
    /// Latent while one Granite pack is admitted, and reachable in principle
    /// because `granite_selection` re-resolves on every warm with
    /// `cuda_worker_available()`.
    ResidentMismatch {
        resident_pack_id: String,
        resident_revision: String,
        requested_pack_id: String,
        requested_revision: String,
    },
    /// This pack's bytes did not match. A named, actionable install fault.
    Failed { pack_id: String, revision: String },
}

impl WarmVerification {
    /// The artifact this outcome is about, when it is about one.
    #[must_use]
    pub fn identity(&self) -> Option<(&str, &str)> {
        match self {
            // A mismatch is about two artifacts, which is the same as being
            // about none: a promotion needs one pack whose bytes were read, and
            // there is no such pack here.
            Self::NotAttempted | Self::ResidentMismatch { .. } => None,
            Self::Verified { pack_id, revision }
            | Self::AlreadyLoaded { pack_id, revision }
            | Self::Failed { pack_id, revision } => Some((pack_id, revision)),
        }
    }

    /// A stable code for the support log, so a mismatch is findable rather than
    /// merely harmless. `granite_warm` carries it as `bytes=`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Verified { .. } => "verified",
            Self::AlreadyLoaded { .. } => "already_loaded",
            Self::ResidentMismatch { .. } => "resident_pack_mismatch",
            Self::Failed { .. } => "unverified",
        }
    }

    /// Whether this outcome says the bytes on disk match the manifest.
    ///
    /// True for `AlreadyLoaded` as well: a pass ran earlier in this process, on
    /// that exact artifact, and its result is still what the loaded adapter was
    /// built from. What it is *not* is evidence about a different artifact,
    /// which is why the caller still compares identities.
    #[must_use]
    pub const fn bytes_match(&self) -> bool {
        matches!(self, Self::Verified { .. } | Self::AlreadyLoaded { .. })
    }
}

/// The resident worker, and the artifact it was loaded for.
///
/// The identity lives *with* the adapter so
/// [`GraniteEngineCoordinator::ensure_ready`] can answer about the pack that is
/// loaded rather than the one this invocation resolved. See
/// [`resident_answer`].
struct ResidentPack {
    adapter: Arc<ResidentGraniteAdapter>,
    pack_id: String,
    revision: String,
}

/// What [`GraniteEngineCoordinator::ensure_ready`] did, as two facts rather than
/// one.
///
/// The adapter is a `Result` inside the struct rather than the struct inside a
/// `Result`, because the verification matters on the failure path too: a warm
/// that ends on `granite_model_files_unverified` is the one outcome entitled to
/// condemn a pack, and an `Err` alone carries no identity.
pub struct EnsureReadyOutcome {
    /// What this invocation did about the bytes.
    pub verification: WarmVerification,
    /// The resident adapter, or why there is none.
    pub adapter: Result<Arc<ResidentGraniteAdapter>, DomainError>,
}

/// What [`warm_granite_if_configured`] did, per invocation.
///
/// Returned rather than recorded. See that function's own doc comment.
pub struct WarmOutcome {
    /// What this invocation did about the bytes.
    pub verification: WarmVerification,
    /// The error this warm ended on, when it ended on one. `None` covers both a
    /// successful warm and every "Granite is not part of this install" exit,
    /// which are the same non-fault to every caller.
    pub error: Option<DomainError>,
}

pub struct GraniteEngineCoordinator {
    crashes: Mutex<CrashThrottle>,
    started_at: std::time::Instant,
    /// The resident worker and the pack it holds. **No "last warm" verdict lives
    /// beside it**: a warm's outcome is returned to the caller that asked for
    /// it. See [`warm_granite_if_configured`].
    adapter: Mutex<Option<ResidentPack>>,
    reason: Mutex<&'static str>,
    /// Which device the resident worker reported it can actually use, as
    /// opposed to which pack `reason` says was selected. Separate fields
    /// because they are separate facts, and conflating them is what made the
    /// support log claim CPU while Granite ran on the GPU.
    device: Mutex<&'static str>,
    /// The worker's answer, remembered for the life of the process so pack
    /// selection stops having to guess.
    ///
    /// `None` before any worker has spoken, which is genuinely all the host
    /// knows at that point: the CUDA backend is linked into the binary rather
    /// than sitting beside it, so the first selection of a process cannot do
    /// better than assume the conservative answer.
    cuda_worker: Mutex<Option<bool>>,
    /// Whether what setup recorded still describes what is running.
    ///
    /// Recorded at warm and read by the log and the settings page. `Unrecorded`
    /// until a warm happens, which is the same thing an installation with no
    /// marker reports — both mean "nothing has been checked", and inventing a
    /// distinction between them would be a claim about a check that has not run.
    integrity: Mutex<ProviderIntegrity>,
    /// Where the resident worker is in its lifecycle: `cold`, `warming`,
    /// `ready`, or an error code. This is what the HUD's `engine` field means.
    ///
    /// **A field rather than a derivation, and that is load-bearing.** The
    /// obvious implementation is "`ready` when `adapter` is `Some`" — and
    /// [`Self::ensure_ready`] holds `adapter` locked across the whole ~2 GB
    /// load, so an accessor that touched it would block the HUD's 10 Hz poll
    /// for the entire warm. That poll runs in a `WebView` callback. The dock
    /// would freeze for seconds at launch, which is a worse bug than the one
    /// this field fixes and would look like the app hanging rather than like a
    /// lock being held. Nothing here may lock `adapter`.
    warm: Mutex<&'static str>,
}

impl Default for GraniteEngineCoordinator {
    fn default() -> Self {
        Self {
            crashes: Mutex::new(
                CrashThrottle::new(3, Duration::from_mins(1))
                    .expect("static crash policy must be valid"),
            ),
            started_at: std::time::Instant::now(),
            adapter: Mutex::new(None),
            reason: Mutex::new("not_configured"),
            device: Mutex::new("not_configured"),
            cuda_worker: Mutex::new(None),
            integrity: Mutex::new(ProviderIntegrity::Unrecorded),
            warm: Mutex::new("cold"),
        }
    }
}

impl GraniteEngineCoordinator {
    pub fn is_quarantined(&self) -> bool {
        self.crashes
            .lock()
            .is_ok_and(|crashes| crashes.is_quarantined())
    }

    /// A stable code for which Granite pack this machine resolved and why —
    /// `not_configured` until a resolution happens, then one of
    /// [`EngineChoiceReason`]'s codes.
    ///
    /// This is the disclosure surface for provider selection, and the reason
    /// it exists at all: after a fallback the honest answer is not deducible
    /// from the provider alone. `lib.rs`'s `warm_granite_engine` logs it
    /// beside the warm result, the way the deleted `streaming_warm` event
    /// logged the streaming engine's own status. A code and nothing else —
    /// never a device name, never a path.
    pub fn engine_reason(&self) -> &'static str {
        self.reason
            .lock()
            .map_or("granite_state_unavailable", |reason| *reason)
    }

    fn record_engine_reason(&self, reason: &'static str) {
        if let Ok(mut slot) = self.reason.lock() {
            *slot = reason;
        }
    }

    /// Which device the resident Granite worker actually runs on, as the
    /// worker itself reported at `Hello`.
    ///
    /// Distinct from [`Self::engine_reason`], which names the *pack* that was
    /// selected and why. The two disagree on any machine running a
    /// CUDA-compiled worker against the CPU-named pack — there is only one
    /// Granite pack, and the same GGUF runs on either device — and reading the
    /// pack reason as a device claim is exactly the mistake this field exists
    /// to stop. `unknown` means the worker answered `Hello` without the field,
    /// which is a pre-v2 binary.
    pub fn device(&self) -> &'static str {
        self.device
            .lock()
            .map_or("granite_state_unavailable", |device| *device)
    }

    /// Where the resident worker is: `cold`, `warming`, `ready`, or an error
    /// code. The vocabulary the HUD's `engine` field is documented in.
    ///
    /// Distinct from [`Self::engine_reason`], which answers a different
    /// question — which *pack* was selected and why. The HUD used to be filled
    /// from that one, and the result was a dock that said "ready" from the
    /// first poll while Granite was still loading two gigabytes: `engine_reason`
    /// can never return `warming` or `ready`, so the frontend's loading state
    /// was unreachable for the life of the process. Both facts are worth
    /// having; only this one answers "has it loaded".
    ///
    /// Quarantine is checked first because it outranks whatever the last warm
    /// left behind: an engine that has crashed its way out of service is not
    /// `ready` even though an adapter may still be cached.
    ///
    /// Cheap by contract — this is read at 10 Hz. Two uncontended locks and a
    /// copy, no filesystem, and deliberately **not** `adapter`; see the field's
    /// own comment for what touching it would cost.
    pub fn warm_state(&self) -> &'static str {
        if self.is_quarantined() {
            return "granite_quarantined";
        }
        self.warm
            .lock()
            .map_or("granite_state_unavailable", |state| *state)
    }

    fn record_warm_state(&self, state: &'static str) {
        if let Ok(mut slot) = self.warm.lock() {
            *slot = state;
        }
    }

    /// What setup recorded versus what is running, as a stable code.
    ///
    /// `ok` and `unrecorded` are the quiet answers; the other two say something
    /// happened that the user or an operator is entitled to know.
    pub fn provider_integrity(&self) -> ProviderIntegrity {
        self.integrity
            .lock()
            .map_or(ProviderIntegrity::Unrecorded, |integrity| *integrity)
    }

    fn record_worker_provider(&self, worker: WorkerProvider, recorded_provider: &str) {
        if let Ok(mut slot) = self.device.lock() {
            *slot = worker.device();
        }
        // Only a definite answer is remembered, and it is the *compiled*
        // capability rather than the device: pack selection asks "can this
        // binary take a CUDA pack", which a worker running on the processor
        // behind an unanswerable driver can still do. `None` means the handshake
        // did not answer, and recording `false` for that would turn "we did not
        // ask successfully" into "there is no GPU worker".
        if let (Some(compiled), Ok(mut slot)) = (worker.compiled_cuda, self.cuda_worker.lock()) {
            *slot = Some(compiled);
        }
        if let Ok(mut slot) = self.integrity.lock() {
            *slot = assess_provider_integrity(recorded_provider, worker);
        }
    }

    /// What pack selection should assume about a CUDA-capable worker.
    ///
    /// Conservative until a worker has actually said otherwise, because the
    /// alternative is claiming a GPU path that may not exist. Once one has
    /// spoken, every later selection in this process knows.
    pub fn cuda_worker_available(&self) -> bool {
        self.cuda_worker.lock().is_ok_and(|cached| {
            cached.unwrap_or(GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS)
        })
    }

    pub fn record_worker_failure(&self) {
        if let Ok(mut crashes) = self.crashes.lock() {
            let _ = crashes.record_crash(self.started_at.elapsed());
        }
    }

    /// Releases the resident worker so the app can exit cleanly, or so the
    /// next dictation spawns a fresh one after a fault the worker cannot
    /// recover from.
    ///
    /// Idempotent, and safe to call when the engine never warmed. Dropping
    /// the adapter here only releases this reference — a call to
    /// `run_granite_final_pass` still holding it via [`Self::ensure_ready`]
    /// keeps the process alive until that call finishes, and the process is
    /// torn down (`ProcessWorkerClient`'s `Drop`) when the last reference
    /// goes.
    pub fn shutdown(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.take();
        }
        // Back to `cold`, because that is now true: the next dictation warms
        // again. Leaving `ready` behind would have the dock claim a loaded
        // engine after `invalidate` threw the worker away, which is the same
        // class of lie this whole change removes — just pointed the other way.
        self.record_warm_state("cold");
    }

    /// Discards the resident worker so the next [`Self::ensure_ready`] builds
    /// a new one.
    ///
    /// The streaming coordinator had the same method for the same reason:
    /// without it, one fault that leaves the worker process itself
    /// unusable (rather than just this dictation) would mean every dictation
    /// after it fails against the same corpse forever.
    pub fn invalidate(&self) {
        self.shutdown();
    }

    /// Spawns the worker and loads the model if that has not already
    /// happened. Returns the shared adapter, and what this invocation did about
    /// the bytes.
    ///
    /// Both, and in one value, because they are two different facts and the
    /// second one used to be left in a coordinator field for the caller to read
    /// back. See [`EnsureReadyOutcome`].
    ///
    /// `EnsureReadyOutcome::adapter` is a recoverable [`DomainError`] when the
    /// pack's files fail verification or the worker process fails to spawn or
    /// load. A spawn or load failure also records a crash via
    /// [`Self::record_worker_failure`]; a verification failure does not — see
    /// `a_hash_mismatch_fails_verification_without_quarantining` in this
    /// module's tests for why a static install defect must not count as a
    /// crash.
    /// `recorded_provider` is the token `install-provider.txt` carries, so the
    /// warm can compare what setup proved against what actually came up. Passed
    /// in rather than read here: the coordinator has no profile root, and the
    /// marker is a fact about the *installation* that the composition root
    /// already holds.
    pub fn ensure_ready(
        &self,
        granite_worker_exe: &Path,
        choice: &GranitePackChoice<'_>,
        diagnostic_log: Option<PathBuf>,
        recorded_provider: &str,
        cuda_context_probe: &dyn CudaContextProbe,
        verifier: &dyn PackVerifier,
    ) -> EnsureReadyOutcome {
        let requested = (choice.pack.id(), choice.pack.revision());
        let Ok(mut slot) = self.adapter.lock() else {
            // A poisoned lock says nothing about any pack's bytes, so the
            // verification is `NotAttempted` rather than anything that could be
            // settled from.
            return EnsureReadyOutcome {
                verification: WarmVerification::NotAttempted,
                adapter: Err(domain_error(ErrorCode::AdapterFailed)),
            };
        };
        match resident_answer(
            slot.as_ref()
                .map(|held| (held.pack_id.as_str(), held.revision.as_str(), &held.adapter)),
            requested,
        ) {
            // Nothing loaded. Fall through and warm.
            ResidentAnswer::Cold => {}
            // The loaded adapter is this pack's. Nothing was hashed here, and the
            // outcome says so.
            ResidentAnswer::Reuse {
                adapter,
                verification,
            } => {
                return EnsureReadyOutcome {
                    verification,
                    adapter: Ok(adapter),
                };
            }
            // A different pack is loaded, so this invocation refuses. It does
            // **not** hand back the loaded adapter: returning it ran a dictation
            // on pack A that had resolved pack B, and certified B on A's
            // digests. The slot is cleared so the next call warms the pack that
            // was actually requested -- a refusal now and the right pack next
            // time, rather than a silent substitution either time.
            ResidentAnswer::Refuse(verification) => {
                *slot = None;
                self.record_warm_state("granite_resident_pack_mismatch");
                return EnsureReadyOutcome {
                    verification,
                    adapter: Err(domain_error(ErrorCode::AdapterFailed)),
                };
            }
        }
        self.record_engine_reason(choice.reason.code());
        // Every exit from here sets the warm state, including both failures.
        // A `warming` that is only cleared on success latches forever after one
        // failed warm, which is a worse lie than the "always ready" this
        // replaces: the dock would sit amber for the life of the process while
        // the engine was in fact idle and retryable.
        self.record_warm_state("warming");
        // The one digest pass a launch takes, immediately before the worker is
        // handed `model_root`. The identity is recorded either way, because the
        // caller has to promote *this* pack rather than whichever one it
        // re-resolves afterwards.
        let identity = (requested.0.to_owned(), requested.1.to_owned());
        if !verifier.bytes_match(choice.pack, &choice.model_root) {
            // Deliberately its own code rather than the generic warm failure.
            // A hash mismatch or a missing file is a static install defect the
            // user can act on, and it takes no quarantine strike — see
            // `a_hash_mismatch_fails_verification_without_quarantining`.
            self.record_warm_state("granite_model_files_unverified");
            return EnsureReadyOutcome {
                verification: WarmVerification::Failed {
                    pack_id: identity.0,
                    revision: identity.1,
                },
                adapter: Err(domain_error(ErrorCode::AdapterFailed)),
            };
        }
        let verification = WarmVerification::Verified {
            pack_id: identity.0.clone(),
            revision: identity.1.clone(),
        };
        let (adapter, worker) = match warm(
            granite_worker_exe,
            choice,
            diagnostic_log,
            cuda_context_probe,
        ) {
            Ok((adapter, worker)) => (Arc::new(adapter), worker),
            Err(error) => {
                self.record_worker_failure();
                // The specific code, not a generic one: this is the same string
                // `granite_warm` logs as `result=`, so the chip and the support
                // log name the same failure. Reached through `crate::` because
                // this module is a real `mod` while the command files are
                // `include!`d into the crate root — one mapping of `ErrorCode`
                // to a wire code, not two that can drift.
                self.record_warm_state(crate::domain_error_code(&error));
                // The digest pass still ran, and it still passed. A worker that
                // would not spawn is not evidence against the bytes, and
                // reporting `NotAttempted` here would throw away the one hash a
                // launch takes.
                return EnsureReadyOutcome {
                    verification,
                    adapter: Err(error),
                };
            }
        };
        self.record_worker_provider(worker, recorded_provider);
        // The identity is stored with the adapter, so the next invocation that
        // finds it resident can say which pack it is holding.
        *slot = Some(ResidentPack {
            adapter: Arc::clone(&adapter),
            pack_id: identity.0,
            revision: identity.1,
        });
        self.record_warm_state("ready");
        EnsureReadyOutcome {
            verification,
            adapter: Ok(adapter),
        }
    }
}

/// What `ensure_ready` does with a slot that may already hold an adapter.
///
/// `Refuse` carries **no adapter**, and that is the point: a mismatch cannot hand
/// back the loaded pack's adapter because the type has nowhere to put one. An
/// answer that both refused and carried an adapter is what let a resident pack A
/// run a dictation that had resolved pack B.
pub enum ResidentAnswer<T> {
    /// Nothing is loaded. The caller warms the requested pack.
    Cold,
    /// What is loaded is the pack this invocation resolved.
    ///
    /// The verification is built from the **resident** identity, never from the
    /// requested one. They are equal on this arm, and building it from the
    /// request is what shipped as a defect: the same construction on the arm
    /// below would have certified a pack nobody hashed.
    Reuse {
        adapter: T,
        verification: WarmVerification,
    },
    /// What is loaded is a different pack, so the requested pack is not loaded
    /// and this invocation may not run.
    Refuse(WarmVerification),
}

/// Whether an already-loaded adapter answers the pack this invocation resolved.
///
/// Generic over the adapter so the rule is exercisable without a worker process.
/// One Granite pack is admitted today, so the mismatched branch is unreachable
/// through the real resolver; it is not unreachable in principle, because
/// `granite_selection` re-resolves on every warm with `cuda_worker_available()`
/// and that answer changes the first time a worker reports a CUDA build.
///
/// Never `Verified`: nothing is hashed in the invocation that calls this.
pub fn resident_answer<T: Clone>(
    resident: Option<(&str, &str, &T)>,
    requested: (&str, &str),
) -> ResidentAnswer<T> {
    let Some((pack_id, revision, adapter)) = resident else {
        return ResidentAnswer::Cold;
    };
    if (pack_id, revision) == requested {
        return ResidentAnswer::Reuse {
            adapter: adapter.clone(),
            verification: WarmVerification::AlreadyLoaded {
                pack_id: pack_id.to_owned(),
                revision: revision.to_owned(),
            },
        };
    }
    ResidentAnswer::Refuse(WarmVerification::ResidentMismatch {
        resident_pack_id: pack_id.to_owned(),
        resident_revision: revision.to_owned(),
        requested_pack_id: requested.0.to_owned(),
        requested_revision: requested.1.to_owned(),
    })
}

/// Spawns an owned worker process, loads the model into it once, and wraps it
/// in the final-pass adapter. The load here is what makes the per-dictation
/// `LoadModel` `WorkerFinalAdapter::run_locked` sends a no-op fast path in
/// the worker.
///
/// Also returns what the worker turned out to be, which is the only point in
/// the process where that is knowable: the pack is chosen before any worker
/// exists, CUDA is compiled into the binary rather than sitting beside it as a
/// file to stat, and whether a context was actually created is a property of the
/// running process that only NVML can see.
fn warm(
    granite_worker_exe: &Path,
    choice: &GranitePackChoice<'_>,
    diagnostic_log: Option<PathBuf>,
    cuda_context_probe: &dyn CudaContextProbe,
) -> Result<(ResidentGraniteAdapter, WorkerProvider), DomainError> {
    let process_deadlines = ProcessDeadlines::new(Duration::from_secs(10), Duration::from_secs(5))
        .map_err(|_| domain_error(ErrorCode::InvalidData))?;
    let crashes = CrashThrottle::new(3, Duration::from_mins(1))
        .map_err(|_| domain_error(ErrorCode::InvalidData))?;
    let supervisor = ProcessSupervisor::new(process_deadlines, crashes);
    let clock = Arc::new(SystemClock::default());
    let startup_deadline = Deadline::after(clock.as_ref(), process_deadlines.startup);
    let mut command = Command::new(granite_worker_exe);
    if let Some(parent) = granite_worker_exe.parent() {
        command.current_dir(parent);
    }
    let mut client = ProcessWorkerClient::spawn(
        &mut command,
        supervisor,
        Arc::clone(&clock),
        startup_deadline,
        diagnostic_log,
    )?;
    // A second `Hello`. `ProcessWorkerClient::spawn` already sent one and threw
    // the `Ready` away, and this asks again rather than changing that signature
    // for one caller — `Hello` is stateless, the worker answers it any number
    // of times, and this costs one round-trip once per warm.
    //
    // A failure here must not fail the warm: the device is a diagnostic, and
    // refusing to run Granite because it could not be named would trade a
    // working final pass for a log field.
    let compiled_cuda = client
        .request(
            WorkerCommand::Hello,
            &CancelToken::default(),
            Deadline::after(clock.as_ref(), GRANITE_WARM_TIMEOUT),
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
    let model_root = choice.model_root.to_string_lossy().into_owned();
    client.request(
        WorkerCommand::LoadModel {
            artifact_id: GRANITE_WORKER_ARTIFACT_ID.to_owned(),
            model_root: model_root.clone(),
        },
        &CancelToken::default(),
        Deadline::after(clock.as_ref(), GRANITE_WARM_TIMEOUT),
    )?;
    // After `LoadModel`, because llama.cpp creates its CUDA context and
    // allocates its buffers there — asking earlier reports `NotHolding` about a
    // worker that goes on to use the card for the whole session.
    //
    // Only for a binary that said it could. A CPU worker has no context to hold,
    // and querying NVML about one would make a driver question part of every
    // processor install's warm path.
    let context = (compiled_cuda == Some(true))
        .then(|| prove_cuda_context(cuda_context_probe, client.process_id()));
    Ok((
        WorkerFinalAdapter::new(
            client,
            clock,
            model_root,
            GRANITE_WORKER_ARTIFACT_ID.to_owned(),
            choice.capabilities,
        ),
        WorkerProvider {
            compiled_cuda,
            context,
        },
    ))
}

/// Whether a fault from the resident Granite worker means the worker itself
/// is unusable, rather than just this dictation having gone wrong. Mirrors
/// `StreamingFault::worker_is_unusable`'s exact set
/// (`streaming_capture.rs`) for the streaming engine's own resident worker —
/// a dead or unresponsive worker outlives the dictation that discovered it,
/// so the coordinator has to be told to rebuild rather than keep handing the
/// same broken client to every later dictation.
const fn granite_worker_is_unusable(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::AdapterFailed | ErrorCode::DeadlineExceeded)
}

/// Warms the resident worker if Granite is part of this install; called once
/// at app launch (`lib.rs`'s `warm_granite_engine`), as the streaming engine
/// was warmed at launch before it.
///
/// `Ok(())` when there is no worker binary or no resolvable manifest pack —
/// identical to [`run_granite_final_pass`]'s own "not configured" handling,
/// so a machine without Granite pays nothing extra at startup.
///
/// Deliberately does not eagerly re-warm after an
/// [`GraniteEngineCoordinator::invalidate`] the way `AppHudSink::ended` does
/// for the streaming engine — that would need an `AppHandle` threaded into
/// this module purely to spawn a background thread, coupling it to Tauri
/// state the way [`run_granite_final_pass`] deliberately is not. The next
/// dictation's own [`GraniteEngineCoordinator::ensure_ready`] call re-warms
/// lazily instead, at the cost of that one dictation paying the load time
/// inline — a disclosed simplification, not an oversight.
///
/// Warms the engine, and reports what **this invocation** did about the bytes.
///
/// [`WarmOutcome::error`] carries a recoverable [`DomainError`] under the same
/// conditions [`GraniteEngineCoordinator::ensure_ready`] does.
///
/// **Returned, never recorded.** No "last warm" field exists to read it back
/// from, and that is load-bearing: `ensure_ready` can return a resident adapter
/// without hashing, and `run_granite_final_pass` calls the same function, so a
/// field would be answered by whichever invocation wrote last rather than by the
/// one whose settle is about to act on it. See [`WarmVerification`].
pub fn warm_granite_if_configured(
    environment: GraniteEnvironment<'_>,
    coordinator: &GraniteEngineCoordinator,
) -> WarmOutcome {
    /// Every exit that reached no digest pass, spelled once. The model on disk
    /// is neither proven nor suspect after one of these: nobody looked.
    fn not_attempted(error: Option<DomainError>) -> WarmOutcome {
        WarmOutcome {
            verification: WarmVerification::NotAttempted,
            error,
        }
    }

    // Every path that returns without warming says so, and `cold` is not that
    // answer. `cold` means "not loaded yet", the frontend maps it to
    // `loading_model` along with `warming`, and a machine that will never warm
    // would sit on "Loading model" for the life of the process — which is the
    // same lie this change removes, pointed the other way. Caught by running
    // it: a build whose worker path does not resolve logged
    // `granite_warm result=ok engine=not_configured` and the dock reported
    // loading indefinitely.
    // Split from `not_configured` deliberately, because they are different
    // facts and the indicator paints them differently. A missing *pack* is a
    // machine that has not finished setting up — nothing has failed, and the
    // dock already says "Setup needed". A missing *worker binary* is an
    // installation that is broken, and calling that "not configured" would
    // report a fault as a to-do.
    let Some(granite_worker_exe) = environment.granite_worker_exe else {
        coordinator.record_warm_state("granite_worker_missing");
        return not_attempted(None);
    };
    if !granite_memory_is_sufficient(environment.total_memory_bytes) {
        coordinator.record_engine_reason("memory_below_granite_floor");
        coordinator.record_warm_state("memory_below_granite_floor");
        return not_attempted(None);
    }
    // Same ordering as `run_granite_final_pass`, and for the same reason —
    // see the comment there.
    if coordinator.is_quarantined() {
        return not_attempted(Some(domain_error(ErrorCode::EngineQuarantined)));
    }
    let Ok(manifest) = bundled_manifest() else {
        coordinator.record_warm_state("not_configured");
        return not_attempted(None);
    };
    let Some(choice) = admitted_granite_pack(
        &manifest,
        environment.install_root,
        coordinator.cuda_worker_available(),
    ) else {
        coordinator.record_warm_state("not_configured");
        return not_attempted(None);
    };
    let outcome = coordinator.ensure_ready(
        granite_worker_exe,
        &choice,
        environment.diagnostic_log.clone(),
        environment.recorded_provider,
        environment.cuda_context_probe,
        environment.verifier,
    );
    if let Err(error) = outcome.adapter {
        // The verification travels with the failure. A warm that hashed the
        // pack and *then* failed to spawn a worker has still read those bytes,
        // and it is the only thing entitled to condemn them.
        return WarmOutcome {
            verification: outcome.verification,
            error: Some(error),
        };
    }

    // Select once more, now that the worker has said what it is. The first
    // selection of a process necessarily guessed — a CUDA backend compiled
    // into the binary is invisible until the binary answers — and on a machine
    // running a CUDA worker that guess produced `cpu_gpu_runtime_missing`,
    // which asserts there is no GPU worker while one is running the dictation.
    // Cheap: the adapter is cached, so this re-reads the manifest and the
    // filesystem and touches no process.
    if coordinator.cuda_worker_available()
        && let Some(corrected) = admitted_granite_pack(&manifest, environment.install_root, true)
    {
        coordinator.record_engine_reason(corrected.reason.code());
    }
    WarmOutcome {
        verification: outcome.verification,
        error: None,
    }
}

/// Which Granite pack a dictation will run on, where it lives, and why that
/// engine rather than another.
///
/// It was modelled on the streaming engine's `AsrPackChoice`, which the fork
/// removed, and differs from it in one deliberate way. It carries the [`Pack`]
/// because Granite admission uses its complete manifest metadata when verifying
/// the selected CPU/GPU variant. That type carried a typed `provider`
/// alongside its spec; this does not, because [`capabilities_for`] has already
/// resolved the provider into the identity that actually travels anywhere
/// (`capabilities.provider`), and a second copy of the same fact is a second
/// thing that can disagree.
pub struct GranitePackChoice<'a> {
    /// The pack to load.
    pub pack: &'a Pack,
    /// Its activation directory. Every required file is present; the bytes are
    /// still unverified — [`GraniteEngineCoordinator::ensure_ready`] hashes
    /// them on the cold path, which is where the trust boundary is.
    pub model_root: PathBuf,
    /// The capability identity the chosen provider's pack publishes, resolved
    /// once here so nothing downstream can pair a pack with another's
    /// provenance.
    pub capabilities: EngineCapabilities,
    /// Why this engine and not another one.
    pub reason: EngineChoiceReason,
}

/// The Granite engine decision itself, with the GPU probe and the disk
/// factored out.
///
/// Split from [`admitted_granite_pack`] for exactly the reason the streaming
/// engine split its own choice function from its admission function, and
/// it was not incidental there either: a decision function that reaches for
/// `NvmlGpuProbe` and the real filesystem itself can only ever be exercised as
/// whatever the developer's own machine happens to be, and the case this one
/// exists for — preferring CUDA and having to fall back — is a case no machine
/// this has ever run on can produce.
///
/// `None` means no Granite pack is installed for any provider. That is an
/// ordinary "Granite is not part of this install", not a fault, and the
/// callers below turn it into their own `Ok(None)`.
///
/// Presence, not verification, is what `present` tests — the stance the
/// streaming engine's own pack selection took before the fork removed it: a
/// pack that is present but corrupt is a **fault**, and quietly downgrading the
/// user because of it would hide a broken install rather than report one. So
/// this proceeds, and `verify_pack_files` fails loudly on the cold path.
fn choose_granite_pack<'a>(
    manifest: &'a TrustedManifest,
    preferred: ExecutionProvider,
    cuda_worker_available: bool,
    present: &impl Fn(&Pack) -> bool,
) -> Option<(&'a Pack, ExecutionProvider, EngineChoiceReason)> {
    let installed = |provider: ExecutionProvider| -> Option<&'a Pack> {
        // No capabilities for a provider means this project publishes no
        // Granite pack for it, so there is nothing to select even if a
        // manifest somewhere grew one — the two have to land together.
        capabilities_for(provider)?;
        let pack = manifest
            .select_sole_install_eligible(PackRole::FinalAsr, provider)
            .ok()?;
        present(pack).then_some(pack)
    };
    let cpu_because = |reason| {
        installed(ExecutionProvider::Cpu).map(|pack| (pack, ExecutionProvider::Cpu, reason))
    };
    // Checked before the pack, because it outranks it: without a CUDA-capable
    // worker binary there is no GPU path to take however many CUDA packs are
    // installed. Today this is the branch every CUDA-capable machine takes —
    // see `GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS`.
    if preferred == ExecutionProvider::Cuda && !cuda_worker_available {
        return cpu_because(EngineChoiceReason::CpuGpuRuntimeMissing);
    }
    if let Some(pack) = installed(preferred) {
        return Some((pack, preferred, EngineChoiceReason::ProbePreferred));
    }
    if preferred == ExecutionProvider::Cuda {
        return cpu_because(EngineChoiceReason::CpuGpuPackNotInstalled);
    }
    None
}

/// Resolves the Granite pack this machine should actually run, probing the GPU
/// and the disk for real.
///
/// GPU-preferred, CPU-fallback, with `GpuQualification::preferred_provider()`
/// as the single place the *preference* is decided. That was shared with the
/// streaming pack's own admission before the fork removed it, so the two
/// engines could not disagree about what this machine prefers; there is one
/// engine now, and the call stays the single place regardless.
///
/// The reason travels on the returned choice;
/// [`GraniteEngineCoordinator::ensure_ready`] is what records it, so the code
/// in the support log always describes an engine that was actually warmed.
fn admitted_granite_pack<'a>(
    manifest: &'a TrustedManifest,
    install_root: &Path,
    cuda_worker_available: bool,
) -> Option<GranitePackChoice<'a>> {
    let model_root = |pack: &Pack| install_root.join(pack.id()).join(pack.revision());
    let present = |pack: &Pack| {
        let root = model_root(pack);
        pack.required_files()
            .iter()
            .all(|file| root.join(file.path()).is_file())
    };
    let preferred = admit(&NvmlGpuProbe.probe()).preferred_provider();
    let (pack, provider, reason) =
        choose_granite_pack(manifest, preferred, cuda_worker_available, &present)?;
    Some(GranitePackChoice {
        model_root: model_root(pack),
        capabilities: capabilities_for(provider)
            .expect("choose_granite_pack only returns providers that publish capabilities"),
        pack,
        reason,
    })
}

/// Runs Granite as the final pass over `audio`, when it is configured at all.
///
/// `install_root` is `ModelCoordinator.root.join("models")` -- the same root
/// `InstallManager::new` takes, so `model_root` below lands in the same
/// `<root>/<pack id>/<revision>` shape an installed pack would, even though
/// Granite's archive-less pack never actually goes through `InstallManager`.
///
/// `Ok(None)` means Granite is not part of this install — no worker binary,
/// no resolvable manifest pack, or its files are not staged under
/// `install_root` — and it is the caller's job to turn that into a named
/// refusal. It used to mean "the ordinary single-engine fallback runs exactly
/// as it did before this engine existed", which described a streaming path
/// that no longer exists; there is nothing to fall back to now.
///
/// The behaviour is unchanged and still correct, which is why this is a
/// comment fix rather than a bug fix: `judge_granite_pass` maps `Ok(None)` to
/// `FinalSourceReason::GraniteUnavailable`, so the dictation ends with a
/// reason the user can act on rather than silence. The distinction this
/// return value still carries is between "not installed" and "failed" — the
/// first is not a fault, takes no quarantine strike, and must never be
/// reported as a crash.
///
/// The third of those became true when [`admitted_granite_pack`] grew a
/// presence check. Before it, a machine that had never fetched the GGUFs
/// reached `verify_pack_files` and failed with `AdapterFailed`, which meant a
/// disclosure on every dictation forever — exactly the noise the `Ok(None)`
/// gates exist to prevent, arrived at from the one direction they did not
/// cover. A *corrupt* pack still fails loudly; see [`choose_granite_pack`] on
/// presence versus verification.
///
/// # Errors
///
/// Returns a recoverable [`DomainError`] when Granite is configured but
/// quarantined, its files fail verification, or the worker process itself
/// fails.
pub async fn run_granite_final_pass(
    environment: GraniteEnvironment<'_>,
    coordinator: &GraniteEngineCoordinator,
    audio: UtteranceAudio,
    request: AsrRequest,
    cancel: CancelToken,
) -> Result<Option<FinalTranscript>, DomainError> {
    let Some(granite_worker_exe) = environment.granite_worker_exe else {
        return Ok(None);
    };
    // A machine too small for Granite is in the same category as one that
    // never installed it: `Ok(None)`, no disclosure, the ordinary path. See
    // `GRANITE_MINIMUM_TOTAL_MEMORY_BYTES` for why this is not an error.
    if !granite_memory_is_sufficient(environment.total_memory_bytes) {
        coordinator.record_engine_reason("memory_below_granite_floor");
        return Ok(None);
    }
    // Ahead of pack resolution, not after it. Quarantine can only be reached
    // by a Granite that was configured and crashed, so resolving first would
    // change nothing on a healthy machine — but once resolution learned to
    // check the disk, a quarantined engine whose files went missing
    // would answer `Ok(None)` and the user would never be told it was
    // quarantined at all. `ErrorCode::EngineQuarantined` exists precisely so
    // that is not silent.
    if coordinator.is_quarantined() {
        return Err(domain_error(ErrorCode::EngineQuarantined));
    }
    let Ok(manifest) = bundled_manifest() else {
        return Ok(None);
    };
    let Some(choice) = admitted_granite_pack(
        &manifest,
        environment.install_root,
        coordinator.cuda_worker_available(),
    ) else {
        return Ok(None);
    };

    // The verification is deliberately dropped here and nowhere else. A
    // dictation's own warm can be the first of a process, but the model status
    // is settled by the launch warm's caller — and a second settler reading a
    // second invocation's verdict is how two callers came to disagree about one
    // pack in the first place.
    let adapter = coordinator
        .ensure_ready(
            granite_worker_exe,
            &choice,
            environment.diagnostic_log,
            environment.recorded_provider,
            environment.cuda_context_probe,
            environment.verifier,
        )
        .adapter?;

    // Built from the resident adapter's own clock, not a fresh one -- see
    // `WorkerFinalAdapter::clock`'s doc comment for the shipped bug this caused.
    // A fresh `SystemClock::default()` here read as already-expired the
    // moment the worker had been resident longer than 90s, deterministically,
    // regardless of how much real per-request time had actually elapsed.
    let deadline = Deadline::after(adapter.clock(), GRANITE_FINISH_STREAM_DEADLINE);
    match adapter.transcribe(audio, request, cancel, deadline).await {
        Ok(transcript) => Ok(Some(transcript)),
        Err(error) => {
            if error.code != ErrorCode::NoSpeechDetected {
                coordinator.record_worker_failure();
            }
            if granite_worker_is_unusable(error.code) {
                coordinator.invalidate();
            }
            Err(error)
        }
    }
}

/// A flattened, owned view of the pack a dictation would load right now.
///
/// [`GranitePackChoice`] borrows the manifest it was selected from, which is
/// fine inside this module where the manifest is a local, and useless to the
/// diagnostics and readiness callers outside it -- they would each have to
/// hold a manifest alive to hold a choice. Those callers wanted four strings
/// and a provider, so this hands them exactly that.
pub struct GraniteSelection {
    pub pack_id: String,
    pub pack_revision: String,
    /// `upstream_repository@upstream_revision`, for the diagnostics view's
    /// model-source line.
    pub source: String,
    pub install_spec: InstallSpec,
    pub capabilities: EngineCapabilities,
    pub reason: EngineChoiceReason,
}

/// What Granite would run right now, or `None` when no pack is installed.
///
/// This is the replacement for the streaming engine's provider-preferring
/// admission call, which several callers outside the engine used to ask of
/// the streaming pack. It takes no provider override: the streaming engine had
/// one because both its packs were downloadable and the user could sensibly
/// prefer either, whereas Granite's provider follows whether a CUDA-capable
/// *worker binary* was built. There is no override that can conjure one, so
/// offering the choice would have been a control that cannot do what it says.
pub fn granite_selection(
    install_root: &Path,
    cuda_worker_available: bool,
) -> Option<GraniteSelection> {
    let manifest = bundled_manifest().ok()?;
    let choice = admitted_granite_pack(&manifest, install_root, cuda_worker_available)?;
    Some(GraniteSelection {
        pack_id: choice.pack.id().to_owned(),
        pack_revision: choice.pack.revision().to_owned(),
        source: format!(
            "trusted_manifest:{}@{}",
            choice.pack.source().upstream_repository(),
            choice.pack.source().upstream_revision()
        ),
        install_spec: InstallSpec::from(choice.pack),
        capabilities: choice.capabilities,
        reason: choice.reason,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A [`PackVerifier`] that counts what the **warm** asked it, and answers a
    /// staged verdict.
    ///
    /// The count is the whole instrument. The test this replaces wrapped the
    /// production function in a closure it called twice itself and then asserted
    /// the closure had been called twice -- a measurement of the test, which
    /// would have stayed green with a second digest pass inserted anywhere in
    /// this module. Nothing here calls `bytes_match`; only the warm does.
    ///
    /// The staged answer is what makes the pass observable at all: the shipped
    /// pack is 2.30 GB and no checkout can lay out bytes the real digests
    /// accept, so a warm over staged stubs would always refuse and the code
    /// after the check would never be reached.
    struct CountingVerifier {
        passes: AtomicU32,
        answer: bool,
    }

    impl CountingVerifier {
        const fn accepting() -> Self {
            Self {
                passes: AtomicU32::new(0),
                answer: true,
            }
        }

        const fn refusing() -> Self {
            Self {
                passes: AtomicU32::new(0),
                answer: false,
            }
        }

        fn passes(&self) -> u32 {
            self.passes.load(Ordering::Relaxed)
        }
    }

    impl PackVerifier for CountingVerifier {
        fn bytes_match(&self, _pack: &Pack, _model_root: &Path) -> bool {
            self.passes.fetch_add(1, Ordering::Relaxed);
            self.answer
        }
    }

    /// A `<root>/<pack id>/<revision>` directory holding a stub for every file
    /// the real pack requires, so `admitted_granite_pack`'s presence check
    /// passes. The bytes are wrong on purpose -- what a warm concludes about
    /// them is [`CountingVerifier`]'s to stage.
    fn stage_present_pack(pack: &Pack) -> tempfile::TempDir {
        let install_root = tempfile::tempdir().expect("tempdir");
        let model_root = install_root.path().join(pack.id()).join(pack.revision());
        std::fs::create_dir_all(&model_root).expect("create model root");
        for required in pack.required_files() {
            std::fs::write(model_root.join(required.path()), b"stub").expect("write stub file");
        }
        install_root
    }

    /// The reported failure, reproduced exactly.
    ///
    /// `engine=cpu_gpu_runtime_missing device=cpu installed=cuda` — three
    /// correct fields whose combination cannot happen, sitting in a support log
    /// on 2026-08-20 with nothing anywhere comparing them. The install marker
    /// said graphics card, the runtime said processor, and the disagreement had
    /// no name, no code and no surface.
    ///
    /// This is the test that could not be written before, because
    /// `assess_provider_integrity` did not exist: there was no function that saw
    /// both facts.
    #[test]
    fn a_graphics_card_record_running_on_the_processor_is_a_named_fault() {
        // A CPU worker: the handshake said it has no CUDA backend, so there was
        // nothing to ask NVML about. This is every machine running the shipped
        // payload.
        let cpu_worker = WorkerProvider {
            compiled_cuda: Some(false),
            context: None,
        };
        let integrity = assess_provider_integrity("cuda", cpu_worker);

        assert_eq!(integrity, ProviderIntegrity::GpuInstallNotOperational);
        assert!(integrity.is_fault());
        assert_eq!(integrity.code(), "gpu_install_not_operational");
        // And the device it reports is the truth about the run, not the record.
        assert_eq!(cpu_worker.device(), "cpu");
    }

    /// GPU hardware, CPU payload: quiet, because the marker is truthful.
    ///
    /// The same machine as the test above — capable card, worker with no CUDA in
    /// it — differing only in what setup wrote down. That is the whole point of
    /// deriving the marker from proof: the *machine* was never the problem.
    #[test]
    fn a_processor_record_running_on_the_processor_says_nothing() {
        let integrity = assess_provider_integrity(
            "cpu",
            WorkerProvider {
                compiled_cuda: Some(false),
                context: None,
            },
        );
        assert_eq!(integrity, ProviderIntegrity::Matches);
        assert!(!integrity.is_fault());
        assert_eq!(integrity.code(), "ok");
    }

    /// A CUDA worker staged over a processor install — what
    /// `scripts/Enable-GraniteCuda.ps1` produces on purpose.
    ///
    /// Not a fault, and not hidden either. Reporting the record as though it
    /// were the truth would mislabel the provider in the other direction, which
    /// the requirement forbids just as flatly.
    #[test]
    fn running_past_the_record_is_disclosed_rather_than_treated_as_a_fault() {
        let integrity = assess_provider_integrity(
            "cpu",
            WorkerProvider {
                compiled_cuda: Some(true),
                context: Some(CudaContextProof::Holding),
            },
        );
        assert_eq!(integrity, ProviderIntegrity::RunningBeyondRecord);
        assert!(!integrity.is_fault());
    }

    /// A CUDA build that ran on the processor anyway.
    ///
    /// The case no static check can see and the reason the NVML gate exists:
    /// `compiled_cuda` is `true`, the binary is exactly right, and the card is
    /// not being used. A machine with a refusing driver, a claimed card, or
    /// exhausted VRAM produces this, and llama.cpp reports it in its own stderr
    /// rather than as an error.
    #[test]
    fn a_cuda_build_that_never_got_a_context_is_reported_as_the_processor() {
        let fell_back = WorkerProvider {
            compiled_cuda: Some(true),
            context: Some(CudaContextProof::NotHolding),
        };
        assert_eq!(fell_back.device(), "cpu");
        assert!(!fell_back.proved_graphics_card());
        assert_eq!(
            assess_provider_integrity("cuda", fell_back),
            ProviderIntegrity::GpuInstallNotOperational
        );
    }

    /// A CUDA build whose context could not be checked is neither answer.
    ///
    /// `cuda_unverified`, deliberately its own device value. Calling it `cuda`
    /// is the unverified claim this whole path removes; calling it `cpu` reports
    /// a fault on a machine that is very likely using its card behind a driver
    /// that would not answer a query.
    #[test]
    fn an_unprovable_context_is_labelled_as_unverified_rather_than_guessed() {
        let unprovable = WorkerProvider {
            compiled_cuda: Some(true),
            context: Some(CudaContextProof::ProbeUnavailable(
                speakeasy_models::GpuProbeFailure::LibraryMissing,
            )),
        };
        assert_eq!(unprovable.device(), "cuda_unverified");
        // And the comparison says the same thing. It used to answer
        // `GpuInstallNotOperational` here, on the argument that setup wrote
        // `cuda` only where it had proof so an installation that can no longer
        // prove it is one whose card stopped being used. That argument is
        // wrong: what stopped is the *query*. The fault's copy tells the user
        // dictation moved to the processor, and nothing here establishes any
        // device at all.
        assert_eq!(
            assess_provider_integrity("cuda", unprovable),
            ProviderIntegrity::GpuRecordUnconfirmed
        );
        assert!(!ProviderIntegrity::GpuRecordUnconfirmed.is_fault());
    }

    /// A worker that never answered the handshake reports `unknown`, not `cpu`.
    ///
    /// "It said it has no CUDA backend" and "it did not say" are different
    /// facts. Folding the second into the first is the overreach that had the
    /// host asserting there was no GPU worker while one ran the dictation.
    #[test]
    fn a_silent_handshake_is_unknown_rather_than_the_processor() {
        let silent = WorkerProvider {
            compiled_cuda: None,
            context: None,
        };
        assert_eq!(silent.device(), "unknown");
        // Nothing proves the card is in use -- and nothing refutes it either,
        // which is the half the old assertion here dropped. A worker that did
        // not answer is the same evidential position as a driver that did not
        // answer, so it gets the same answer.
        assert_eq!(
            assess_provider_integrity("cuda", silent),
            ProviderIntegrity::GpuRecordUnconfirmed
        );
    }

    /// The device and the integrity verdict cannot disagree about the device.
    ///
    /// The whole class of defect this module keeps producing is a layer
    /// asserting something a neighbouring layer does not support, so the
    /// correspondence is pinned rather than left to two `match` arms that happen
    /// to line up today: only `cpu` -- the definitive negative -- may be the
    /// fault, and the fault is the only verdict whose copy names a device.
    #[test]
    fn only_a_definitive_processor_run_may_be_reported_as_the_fault() {
        let cases = [
            (Some(false), None, "cpu"),
            (Some(true), Some(CudaContextProof::NotHolding), "cpu"),
            (Some(true), Some(CudaContextProof::Holding), "cuda"),
            (
                Some(true),
                Some(CudaContextProof::ProbeUnavailable(
                    speakeasy_models::GpuProbeFailure::LibraryMissing,
                )),
                "cuda_unverified",
            ),
            (None, None, "unknown"),
        ];
        for (compiled_cuda, context, device) in cases {
            let worker = WorkerProvider {
                compiled_cuda,
                context,
            };
            assert_eq!(worker.device(), device);
            let verdict = assess_provider_integrity("cuda", worker);
            assert_eq!(
                verdict.is_fault(),
                device == "cpu",
                "{device} must not decide the fault this way"
            );
        }
    }

    /// An installation with no marker is checked against nothing.
    ///
    /// Reachable on purpose — setup writes the marker only once its engine check
    /// has proved something, so an install whose check never ran has none. Both
    /// the empty string and the token the app substitutes read the same way.
    #[test]
    fn an_installation_that_recorded_nothing_reports_nothing() {
        for recorded in ["unrecorded", ""] {
            assert_eq!(
                assess_provider_integrity(
                    recorded,
                    WorkerProvider {
                        compiled_cuda: Some(false),
                        context: None,
                    },
                ),
                ProviderIntegrity::Unrecorded,
                "{recorded:?} must not be compared against anything"
            );
        }
    }

    /// The one combination that is a match on the graphics card.
    #[test]
    fn a_graphics_card_record_holding_a_context_is_the_quiet_answer() {
        let on_card = WorkerProvider {
            compiled_cuda: Some(true),
            context: Some(CudaContextProof::Holding),
        };
        assert_eq!(on_card.device(), "cuda");
        assert!(on_card.proved_graphics_card());
        assert_eq!(
            assess_provider_integrity("cuda", on_card),
            ProviderIntegrity::Matches
        );
    }

    /// A machine comfortably over the Granite floor, so tests about
    /// everything *else* are not silently short-circuited by the memory gate.
    /// The one test that is about the gate names its own numbers.
    const AMPLE_MEMORY: Option<u64> = Some(GRANITE_MINIMUM_TOTAL_MEMORY_BYTES * 2);

    /// What Granite says about `apps/bootstrapper/fixtures/smoke.wav`.
    ///
    /// The same string `apps/bootstrapper`'s `smoke::SPOKEN` holds, and arrived at
    /// the same way -- discovered by running the model, never typed. Both guesses
    /// at the *previous* fixture's ground truth were wrong, one of them on a
    /// punctuation choice ("dog. And Monday" for a comma that was spoken), which
    /// is why this is pinned from output rather than from the script that
    /// generated the audio.
    ///
    /// Spelled here as well as there rather than shared: this crate deliberately
    /// links no part of the bootstrapper, and a constant reached across that
    /// boundary would be the only reason to. The two are held together by both
    /// tests failing loudly if the model's answer ever changes, which is the
    /// thing worth detecting.
    const SMOKE_CLIP_TRANSCRIPT: &str =
        "The quick brown fox jumps over the lazy dog. And Monday begins at dawn.";

    /// The pack the real resolver would land on, without the disk check —
    /// tests stage their own files and need the pack before they exist.
    fn granite_pack(manifest: &TrustedManifest) -> &Pack {
        manifest
            .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
            .expect("the install-eligible CPU Granite pack must still be in the manifest")
    }

    /// The choice `admitted_granite_pack` builds, for tests that stage files
    /// themselves and so cannot go through the probe-and-disk path.
    fn cpu_choice<'a>(pack: &'a Pack, install_root: &Path) -> GranitePackChoice<'a> {
        GranitePackChoice {
            model_root: install_root.join(pack.id()).join(pack.revision()),
            pack,
            capabilities: CPU_PACK_CAPABILITIES,
            reason: EngineChoiceReason::ProbePreferred,
        }
    }

    #[test]
    fn granite_is_not_configured_without_a_worker_binary() {
        let coordinator = GraniteEngineCoordinator::default();
        let audio = UtteranceAudio {
            session_id: speakeasy_domain::SessionId::from_bytes([1; 16]),
            sample_rate_hz: 16_000,
            samples: vec![0; 1_600],
        };
        let request = AsrRequest {
            correlation_id: speakeasy_domain::CorrelationId::from_bytes([2; 16]),
            session_id: audio.session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        };
        let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: None,
                install_root: Path::new("unused"),
                total_memory_bytes: AMPLE_MEMORY,
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
            audio,
            request,
            CancelToken::default(),
        ));
        assert_eq!(outcome, Ok(None));
        assert!(!coordinator.is_quarantined());
    }

    #[test]
    fn a_quarantined_coordinator_refuses_before_resolving_a_pack() {
        let coordinator = GraniteEngineCoordinator::default();
        for _ in 0..3 {
            coordinator.record_worker_failure();
        }
        assert!(coordinator.is_quarantined());

        let audio = UtteranceAudio {
            session_id: speakeasy_domain::SessionId::from_bytes([3; 16]),
            sample_rate_hz: 16_000,
            samples: vec![0; 1_600],
        };
        let request = AsrRequest {
            correlation_id: speakeasy_domain::CorrelationId::from_bytes([4; 16]),
            session_id: audio.session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        };
        // A worker binary path and install root that do not exist: reaching
        // `verify_pack_files` or a process spawn would fail with a different
        // error than `EngineQuarantined`, so seeing that exact code proves the
        // quarantine check ran before either was attempted.
        let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: Some(Path::new("definitely-does-not-exist.exe")),
                install_root: Path::new("definitely-does-not-exist-root"),
                total_memory_bytes: AMPLE_MEMORY,
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
            audio,
            request,
            CancelToken::default(),
        ));
        assert_eq!(outcome, Err(domain_error(ErrorCode::EngineQuarantined)));
    }

    #[test]
    fn the_manifest_pack_id_still_resolves_to_the_final_asr_role() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        assert_eq!(pack.role(), PackRole::FinalAsr);
    }

    /// Guards the one drift the Q8_0-to-Q4_K_M quantization swap made easy to get
    /// half-right. `workers/granite-worker` hardcodes its own artifact id and
    /// GGUF filename and never reads the manifest -- it deliberately links no
    /// manifest reader -- so nothing but this test connects the worker's idea
    /// of "the model" to the manifest's own `install_eligible` flag. Flipping
    /// either without the other produces a worker that refuses every
    /// `LoadModel` with `ArtifactNotTrusted`, or one that loads a quantization
    /// the manifest never admitted; both surface only on hardware, and the
    /// second one silently.
    #[test]
    fn the_worker_artifact_id_and_the_install_eligible_pack_agree() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        assert!(
            pack.is_install_eligible(),
            "the pack this engine resolves must be the one the manifest admits"
        );
        assert_eq!(
            pack.id(),
            format!("{GRANITE_WORKER_ARTIFACT_ID}-cpu"),
            "the manifest pack id is the worker's artifact id plus a provider suffix"
        );
        assert_eq!(
            pack.revision(),
            CPU_PACK_CAPABILITIES.artifact_revision,
            "the capability identity travels into the transcript's provenance"
        );
        // The worker opens the model GGUF by a filename it derives from the
        // same quantization, so the pack has to actually carry that file.
        let expected_gguf = format!("{GRANITE_WORKER_ARTIFACT_ID}.gguf");
        assert!(
            pack.required_files()
                .iter()
                .any(|file| file.path().to_ascii_lowercase() == expected_gguf),
            "no required file matches {expected_gguf}"
        );
    }

    /// On this machine, and on every machine this can land on today, the
    /// selection resolves to CPU. Both CUDA-preferring paths are exercised
    /// here because `choose_granite_pack` takes the preference and the CUDA
    /// worker's availability as arguments -- a test that could only ask what
    /// *this* machine prefers would never reach either branch, which is the
    /// mistake taking the preference as an argument was made to avoid.
    #[test]
    fn granite_falls_back_to_cpu_for_every_preference_a_machine_can_have() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let installed = |_: &Pack| true;

        // A machine with no admissible card. Nothing to fall back from.
        let (pack, provider, reason) = choose_granite_pack(
            &manifest,
            ExecutionProvider::Cpu,
            GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS,
            &installed,
        )
        .expect("the CPU pack is installed");
        assert_eq!(pack.id(), granite_pack(&manifest).id());
        assert_eq!(provider, ExecutionProvider::Cpu);
        assert_eq!(reason, EngineChoiceReason::ProbePreferred);

        // A machine whose card the probe prefers -- this one, among others.
        // No CUDA-enabled Granite worker has ever been compiled, so the
        // runtime check outranks the pack check and this is the branch taken.
        let (pack, provider, reason) = choose_granite_pack(
            &manifest,
            ExecutionProvider::Cuda,
            GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS,
            &installed,
        )
        .expect("the CPU pack is installed");
        assert_eq!(pack.id(), granite_pack(&manifest).id());
        assert_eq!(provider, ExecutionProvider::Cpu);
        assert_eq!(
            reason,
            EngineChoiceReason::CpuGpuRuntimeMissing,
            "and the fallback must be disclosable, not silent"
        );

        // The hypothetical the shape exists for: a CUDA worker that *is*
        // available, with no CUDA pack in the manifest to run on it. Still
        // CPU, and disclosed with the other reason -- the two are different
        // sentences to the user, which is why both variants exist.
        let (pack, provider, reason) =
            choose_granite_pack(&manifest, ExecutionProvider::Cuda, true, &installed)
                .expect("the CPU pack is installed");
        assert_eq!(pack.id(), granite_pack(&manifest).id());
        assert_eq!(provider, ExecutionProvider::Cpu);
        assert_eq!(reason, EngineChoiceReason::CpuGpuPackNotInstalled);
    }

    /// Selection is presence-aware, so a machine that never fetched the GGUFs
    /// answers "Granite is not part of this install" rather than reaching
    /// verification and failing. That distinction is the whole difference
    /// between a silent, correct fallback and a disclosure on every dictation
    /// forever -- see [`run_granite_final_pass`]'s own doc.
    #[test]
    fn an_unfetched_pack_is_not_selected_at_all() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        assert!(
            choose_granite_pack(
                &manifest,
                ExecutionProvider::Cpu,
                GRANITE_CUDA_WORKER_ASSUMED_BEFORE_ANY_WORKER_SPEAKS,
                &|_: &Pack| false,
            )
            .is_none(),
            "no pack on disk means no choice, not a corrupt-install fault"
        );
    }

    /// Too little memory declines Granite without faulting — and the outer gate
    /// now refuses the dictation before it is ever asked.
    ///
    /// The ordering assert used to run the other way: the dictation floor was
    /// held strictly *below* Granite's, so a mid-range machine still dictated
    /// through the streaming path and merely lost the second pass. With one
    /// engine that split only bought the user a delay — they passed the outer
    /// gate, spoke, waited out the pass, and got `GraniteUnavailable` anyway.
    /// `MINIMUM_TOTAL_MEMORY_BYTES` is now Granite's floor, and this asserts it
    /// never drops back below it.
    ///
    /// The check below is therefore defence in depth rather than the gate that
    /// fires in practice, and it is still worth pinning: it is what keeps a
    /// machine that somehow reaches here from reading as a *fault*. Too little
    /// memory is "Granite is not part of this install", never a crash to
    /// quarantine over.
    #[test]
    fn too_little_memory_declines_granite_without_faulting() {
        assert!(
            crate::runtime_wizard::minimum_total_memory_bytes()
                >= GRANITE_MINIMUM_TOTAL_MEMORY_BYTES,
            "the dictation floor must not sit below Granite's, or a dictation is              admitted that cannot possibly finish"
        );

        let coordinator = GraniteEngineCoordinator::default();
        let audio = UtteranceAudio {
            session_id: speakeasy_domain::SessionId::from_bytes([7; 16]),
            sample_rate_hz: 16_000,
            samples: vec![0; 1_600],
        };
        let request = AsrRequest {
            correlation_id: speakeasy_domain::CorrelationId::from_bytes([8; 16]),
            session_id: audio.session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        };
        // Paths that do not exist: if the memory gate did not short-circuit
        // first, resolution or the spawn would fail with an error rather than
        // returning `Ok(None)`, so this outcome is what proves the ordering.
        let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: Some(Path::new("definitely-does-not-exist.exe")),
                install_root: Path::new("definitely-does-not-exist-root"),
                total_memory_bytes: Some(GRANITE_MINIMUM_TOTAL_MEMORY_BYTES - 1),
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
            audio,
            request,
            CancelToken::default(),
        ));
        assert_eq!(
            outcome,
            Ok(None),
            "too little memory must read as 'Granite is not part of this install', not a fault"
        );
        assert!(!coordinator.is_quarantined());
        assert_eq!(
            coordinator.engine_reason(),
            "memory_below_granite_floor",
            "and it must still be findable in a support log"
        );
    }

    /// An unanswerable probe fails closed. An unknown memory budget is not
    /// evidence of a large one, and guessing wrong here means Granite paging.
    #[test]
    fn an_unknown_memory_budget_declines_granite() {
        assert!(!granite_memory_is_sufficient(None));
        assert!(!granite_memory_is_sufficient(Some(
            GRANITE_MINIMUM_TOTAL_MEMORY_BYTES - 1
        )));
        assert!(granite_memory_is_sufficient(Some(
            GRANITE_MINIMUM_TOTAL_MEMORY_BYTES
        )));
    }

    /// Nothing may claim a capability identity for a provider this project
    /// publishes no pack for: the fields travel into a transcript's
    /// provenance, and an invented `artifact_revision` there is a lie that
    /// outlives the dictation.
    #[test]
    fn only_the_cpu_provider_publishes_a_capability_identity() {
        assert!(capabilities_for(ExecutionProvider::Cpu).is_some());
        assert!(capabilities_for(ExecutionProvider::Cuda).is_none());
        assert!(capabilities_for(ExecutionProvider::DirectMl).is_none());
        assert_eq!(
            capabilities_for(ExecutionProvider::Cpu)
                .expect("cpu publishes one")
                .provider,
            "cpu"
        );
    }

    /// Corrupt files fail verification without ever touching the crash
    /// throttle: a static install defect is not a crash, and quarantining it
    /// would only make every future dictation fail the identical cheap check
    /// for no added trust, while `record_worker_failure` exists so genuine
    /// process crashes -- not corrupt bytes -- are what leads to quarantine.
    #[test]
    fn a_hash_mismatch_fails_verification_without_quarantining() {
        let coordinator = GraniteEngineCoordinator::default();
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        let install_root = tempfile::tempdir().expect("tempdir");
        let model_root = install_root.path().join(pack.id()).join(pack.revision());
        std::fs::create_dir_all(&model_root).expect("create model root");
        for required in pack.required_files() {
            std::fs::write(model_root.join(required.path()), b"not the real bytes")
                .expect("write stub file");
        }

        let audio = UtteranceAudio {
            session_id: speakeasy_domain::SessionId::from_bytes([5; 16]),
            sample_rate_hz: 16_000,
            samples: vec![0; 1_600],
        };
        let request = AsrRequest {
            correlation_id: speakeasy_domain::CorrelationId::from_bytes([6; 16]),
            session_id: audio.session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        };
        // The worker binary itself is never spawned: verification fails first.
        let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: Some(Path::new("unused-worker.exe")),
                install_root: install_root.path(),
                total_memory_bytes: AMPLE_MEMORY,
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
            audio,
            request,
            CancelToken::default(),
        ));
        assert_eq!(outcome, Err(domain_error(ErrorCode::AdapterFailed)));
        assert!(!coordinator.is_quarantined());
        assert_eq!(coordinator.warm_state(), "granite_model_files_unverified");

        // The same install through the launch warm, which is the caller that
        // has to act on the verdict. The failure names the artifact whose bytes
        // were read, and it is the pack that was actually resolved:
        // `settle_after_warm` compares this against the pack it re-resolves and
        // promotes nothing on a mismatch, so an identity that were merely
        // plausible here -- or absent -- would let a different pack be stamped
        // on this pack's evidence.
        let warm = warm_granite_if_configured(
            GraniteEnvironment {
                granite_worker_exe: Some(Path::new("unused-worker.exe")),
                install_root: install_root.path(),
                total_memory_bytes: AMPLE_MEMORY,
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
        );
        assert_eq!(
            warm.verification,
            WarmVerification::Failed {
                pack_id: pack.id().to_owned(),
                revision: pack.revision().to_owned(),
            },
            "a failed digest pass must report which pack failed"
        );
        assert_eq!(warm.error, Some(domain_error(ErrorCode::AdapterFailed)));
        assert!(!coordinator.is_quarantined());
    }

    /// A warm reads the pack exactly once, and the count is of the warm.
    ///
    /// `readiness` used to hash, and it runs twice on a configured launch, so
    /// with the warm's own pass a launch read 2.30 GB three times before the app
    /// was usable. One survives, and it
    /// is this one -- taken immediately before the worker is handed the
    /// `model_root`.
    ///
    /// **Counted at the warm boundary through an injected [`PackVerifier`].**
    /// The test this replaces counted its own calls to a closure it had wrapped
    /// around the production function: two deliberate calls, an assertion that
    /// there had been two, and no way for a third pass inside the warm to make
    /// any difference. Nothing in this test calls `bytes_match`. Insert a second
    /// `verifier.bytes_match(..)` anywhere in `ensure_ready` and this goes red.
    ///
    /// Both verdicts are exercised, because the refusing one takes a different
    /// exit and could hash again on its way out.
    #[test]
    fn one_warm_takes_exactly_one_digest_pass() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);

        let warm_with = |verifier: &CountingVerifier, install_root: &Path| {
            warm_granite_if_configured(
                GraniteEnvironment {
                    // Never spawned: the pack is staged as stubs, so there is no
                    // real worker to reach and none is needed. What is being
                    // counted happens before the spawn.
                    granite_worker_exe: Some(Path::new("unused-worker.exe")),
                    install_root,
                    total_memory_bytes: AMPLE_MEMORY,
                    diagnostic_log: None,
                    recorded_provider: "unrecorded",
                    cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                    verifier,
                },
                &GraniteEngineCoordinator::default(),
            )
        };

        let install_root = stage_present_pack(pack);
        let accepting = CountingVerifier::accepting();
        let outcome = warm_with(&accepting, install_root.path());
        assert_eq!(
            accepting.passes(),
            1,
            "a warm reads the pack once -- not zero times, and not twice"
        );
        assert_eq!(
            outcome.verification,
            WarmVerification::Verified {
                pack_id: pack.id().to_owned(),
                revision: pack.revision().to_owned(),
            },
            "and it reports the artifact it read"
        );

        let refusing = CountingVerifier::refusing();
        let outcome = warm_with(&refusing, install_root.path());
        assert_eq!(
            refusing.passes(),
            1,
            "a refusal is one pass too; nothing re-reads the pack to be sure"
        );
        assert_eq!(
            outcome.verification,
            WarmVerification::Failed {
                pack_id: pack.id().to_owned(),
                revision: pack.revision().to_owned(),
            }
        );
    }

    /// A resident adapter answers only for the pack it holds, and a mismatch
    /// hands back nothing.
    ///
    /// Requesting B while A is resident must not execute A and must not report
    /// success. Both halves are asserted here: `Refuse` carries no adapter (the
    /// type has nowhere to put one), and the verification it carries certifies
    /// neither pack. `ensure_ready` turns that arm into an `Err` and clears the
    /// slot, so the next call warms the pack that was requested.
    ///
    /// The rule is generic over the adapter so it is exercisable without a worker
    /// process: one Granite pack is admitted today, so the mismatched branch is
    /// unreachable through the real resolver, and a test that could only ask what
    /// this machine resolves would never reach the branch that had the defect.
    #[test]
    fn a_resident_adapter_answers_only_for_the_pack_it_holds() {
        let a = ("granite-speech-4.1-2b-q4_k_m-cpu", "1");
        let b = ("granite-speech-4.1-2b-q4_k_m-cuda", "1");
        // Stands in for the adapter. A `&str` is enough: what matters is whether
        // this value can escape, not what it is.
        let adapter_a = "adapter-for-pack-A";

        match resident_answer(Some((a.0, a.1, &adapter_a)), a) {
            ResidentAnswer::Reuse {
                adapter,
                verification,
            } => {
                assert_eq!(
                    adapter, adapter_a,
                    "the same pack reuses the loaded adapter"
                );
                assert_eq!(
                    verification,
                    WarmVerification::AlreadyLoaded {
                        pack_id: a.0.to_owned(),
                        revision: a.1.to_owned(),
                    },
                    "a pass ran earlier in this process, on these bytes"
                );
            }
            _ => panic!("the requested pack is the resident pack"),
        }

        // Pack B requested, pack A resident.
        match resident_answer(Some((a.0, a.1, &adapter_a)), b) {
            ResidentAnswer::Refuse(verification) => {
                assert_eq!(
                    verification,
                    WarmVerification::ResidentMismatch {
                        resident_pack_id: a.0.to_owned(),
                        resident_revision: a.1.to_owned(),
                        requested_pack_id: b.0.to_owned(),
                        requested_revision: b.1.to_owned(),
                    }
                );
                assert_eq!(
                    verification.identity(),
                    None,
                    "a mismatch is about two artifacts, so it may certify neither"
                );
                assert!(!verification.bytes_match());
            }
            ResidentAnswer::Reuse { adapter, .. } => {
                panic!("pack B was requested and adapter {adapter} escaped");
            }
            ResidentAnswer::Cold => panic!("an adapter was resident"),
        }

        // The revision alone is enough. Matching on the id is the obvious
        // half-fix and it passes a same-pack-different-revision upgrade.
        assert!(matches!(
            resident_answer(Some((a.0, a.1, &adapter_a)), (a.0, "2")),
            ResidentAnswer::Refuse(_)
        ));

        // And an empty slot is neither.
        assert!(matches!(
            resident_answer::<&str>(None, a),
            ResidentAnswer::Cold
        ));
    }

    /// A minimal RIFF/WAVE reader for 16 kHz mono 16-bit PCM -- the same small,
    /// deliberately duplicated reader `workers/granite-worker`'s own smoke test
    /// carries; see it for why a shared dependency is not worth it for nine
    /// lines of chunk-walking.
    /// `speakeasy-granite`'s `granite_smoke` carried a fourth copy until that
    /// module was deleted on 2026-08-27.
    fn read_wave(path: &std::path::Path) -> Vec<i16> {
        let bytes =
            std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
            "not a RIFF/WAVE file: {}",
            path.display()
        );
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
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
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect()
    }

    /// Write the cold and resident pass durations where someone can read them.
    ///
    /// Written down, not only printed. `--nocapture` did not deliver this line on
    /// this machine -- `--show-output` reported the test's stdout as empty -- so
    /// the two numbers the resident-worker test exists to produce were being
    /// discarded while it passed. A measurement that cannot be read is not a
    /// measurement.
    ///
    /// `cold` is spawn plus a ~2.2 GB load plus inference, and is not comparable
    /// across builds because this crate's own SHA-256 verification dominates it
    /// in a debug harness. `resident` is almost entirely the release worker's own
    /// inference, and is the number worth quoting.
    fn record_resident_timing(
        target_debug: &Path,
        worker_exe: &Path,
        cold: Duration,
        resident: Duration,
    ) {
        let report = format!(
            "granite final pass: worker={} first={cold:?} second={resident:?}
",
            if image_is_cuda_build(worker_exe) {
                "cuda"
            } else {
                "cpu"
            }
        );
        print!("{report}");
        std::fs::write(target_debug.join("granite-resident-timing.txt"), &report)
            .expect("the measurement must be written");
    }

    /// Whether a built worker carries llama.cpp's CUDA backend.
    ///
    /// The same `ggml-cuda` marker `scripts/GraniteWorkerProvider.ps1` reads, and
    /// for the reason its comments give: the CUDA build is an order of magnitude
    /// larger than the CPU one (54 MB against 4 MB measured 2026-08-21), so size
    /// is a proxy where the marker is the fact.
    ///
    /// It was also read by `Enable-GraniteCuda.ps1`, which staged a CUDA worker
    /// over an installed one and was retired on 2026-08-26 when setup learned to
    /// fetch a published worker itself. To stage one for the hardware tests now:
    /// `cargo build --release -p speakeasy-granite-worker --features cuda`, then
    /// copy that exe and the three CUDA libraries into `target/debug/proof/` —
    /// the libraries are in an installed build's own `proof/` directory, put
    /// there by setup.
    fn image_is_cuda_build(worker_exe: &Path) -> bool {
        let image = std::fs::read(worker_exe).expect("the staged worker must be readable");
        image
            .windows(b"ggml-cuda".len())
            .any(|window| window == b"ggml-cuda")
    }

    /// A [`CudaContextProbe`] whose answer is decided by the test.
    ///
    /// The reason [`GraniteEnvironment::cuda_context_probe`] exists. Two of the
    /// four values [`WorkerProvider::device`] can return are unreachable against
    /// a real driver on a machine with a working card: it will answer, and it
    /// will place a CUDA worker on a device. So the two that matter most --
    /// "answered no" and "could not be asked" -- have to be staged.
    struct StagedContextProbe(Result<Vec<u32>, speakeasy_models::GpuProbeFailure>);

    impl CudaContextProbe for StagedContextProbe {
        fn compute_process_ids(&self) -> Result<Vec<u32>, speakeasy_models::GpuProbeFailure> {
            self.0.clone()
        }
    }

    /// What a CUDA-built worker reports as its device, under three probes.
    ///
    /// The proof that closes the graphics-card path on real hardware, and it
    /// needs real hardware for exactly one of its three cases: `cuda` is a claim
    /// about a driver, a card and a live process, and nothing staged can make it
    /// true. The other two are staged *because* they cannot be produced on a
    /// working machine on demand, which is the whole argument for the probe being
    /// a parameter rather than a call.
    ///
    /// Each case is its own scope with its own coordinator, and that is not
    /// tidiness: each warm leaves a resident worker holding roughly 3 GiB of
    /// VRAM, and three at once does not fit on an 8 GiB card. A test that ran out
    /// of video memory would report `cpu` for the first case and read exactly
    /// like the regression it is here to detect.
    ///
    /// The staged worker is checked for `ggml-cuda` first. Without that check
    /// this test passes on a CPU worker -- `compiled_cuda` would be `Some(false)`,
    /// every case would answer `cpu`, and two of the three assertions would be
    /// vacuously satisfied by the wrong binary.
    ///
    /// ```text
    /// cargo test -p speakeasy-desktop --lib a_cuda_worker_reports -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "hardware: needs a CUDA-built target/debug/proof/granite-worker.exe, the staged GGUF files, and an NVIDIA card"]
    fn a_cuda_worker_reports_the_device_its_context_probe_can_prove() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repository root");
        let target_debug = repository.join("target").join("debug");
        let worker_exe = target_debug.join("proof").join("granite-worker.exe");
        let install_root = target_debug.join("model-lifecycle").join("models");
        assert!(
            worker_exe.is_file(),
            "missing {}; see this test's documentation",
            worker_exe.display()
        );
        assert!(
            image_is_cuda_build(&worker_exe),
            "the staged worker is not a CUDA build; see this test's documentation"
        );

        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        let choice = cpu_choice(pack, &install_root);

        // The card, proved. `installed=cpu` because that is what an ordinary
        // release records, and a CUDA worker staged over it is precisely
        // `RunningBeyondRecord` -- disclosed, and deliberately not a fault.
        {
            let coordinator = GraniteEngineCoordinator::default();
            coordinator
                .ensure_ready(
                    &worker_exe,
                    &choice,
                    None,
                    "cpu",
                    &speakeasy_models::NvmlCudaContextProbe,
                    &TrustedDigestVerifier,
                )
                .adapter
                .expect("a CUDA worker must warm on a machine with a card");
            assert_eq!(
                coordinator.device(),
                "cuda",
                "NVML must place this worker's own pid on a device"
            );
            assert_eq!(
                coordinator.provider_integrity(),
                ProviderIntegrity::RunningBeyondRecord
            );
        }

        // A driver that will not answer. Neither `cuda` -- which would be the
        // unverified claim -- nor `cpu`, which would report a fault on a machine
        // that is almost certainly using its card.
        {
            let coordinator = GraniteEngineCoordinator::default();
            coordinator
                .ensure_ready(
                    &worker_exe,
                    &choice,
                    None,
                    "cpu",
                    &StagedContextProbe(Err(speakeasy_models::GpuProbeFailure::LibraryMissing)),
                    &TrustedDigestVerifier,
                )
                .adapter
                .expect("an unanswerable driver must not fail the warm");
            assert_eq!(coordinator.device(), "cuda_unverified");
            // Nothing was proved, so nothing is promoted: against a processor
            // record this is agreement, not a discrepancy.
            assert_eq!(coordinator.provider_integrity(), ProviderIntegrity::Matches);
        }

        // The definitive negative: NVML answered, and this pid is not on any
        // device. Against a graphics-card record that is the actionable fault,
        // and it is the one combination a CPU-only release could never produce
        // honestly.
        {
            let coordinator = GraniteEngineCoordinator::default();
            coordinator
                .ensure_ready(
                    &worker_exe,
                    &choice,
                    None,
                    "cuda",
                    &StagedContextProbe(Ok(vec![])),
                    &TrustedDigestVerifier,
                )
                .adapter
                .expect("a worker NVML does not list must still warm");
            assert_eq!(coordinator.device(), "cpu");
            assert_eq!(
                coordinator.provider_integrity(),
                ProviderIntegrity::GpuInstallNotOperational
            );
        }
    }

    /// End-to-end proof against the **real** compiled worker process and the
    /// real GGUF files -- not just the library calls `speakeasy-granite`'s own
    /// proofs already cover. Exercises exactly the path `run_retained_
    /// transcription` drives: resolve the manifest pack, hash-verify its
    /// files, spawn `proof/granite-worker.exe`, and reconcile its transcript.
    ///
    /// Ignored by default because it needs the built worker binary staged at
    /// `target/debug/proof/granite-worker.exe` and the two GGUF files staged
    /// at `target/debug/model-lifecycle/models/<pack id>/<revision>/` -- the
    /// same `<install_root>/models/...` shape `ModelCoordinator.root` resolves
    /// to in the real app. Panics loudly rather than skipping when either is
    /// absent, per this repository's convention for hardware-gated tests.
    ///
    /// The clip is `apps/bootstrapper/fixtures/smoke.wav`, which travels with the
    /// code. It was `.tools/fixtures/beckett.wav` until 2026-08-21, and that file
    /// is gone -- `.tools/` is gitignored, so the fixture existed only on the
    /// machine that made it, and by the time this test was next run there was
    /// nothing to run it against. That is the second time this repository has
    /// lost a fixture that way; the first is recorded in `.gitignore` beside the
    /// exception that keeps this one. A hardware test whose input cannot be
    /// obtained is not a test that is hard to run, it is a test that is gone.
    ///
    /// ```text
    /// cargo test -p speakeasy-desktop --lib granite_final_pass_transcribes -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "hardware: needs target/debug/proof/granite-worker.exe and the staged GGUF files"]
    fn granite_final_pass_transcribes_the_fixture_through_the_real_worker_process() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repository root");
        let target_debug = repository.join("target").join("debug");
        let worker_exe = target_debug.join("proof").join("granite-worker.exe");
        let install_root = target_debug.join("model-lifecycle").join("models");
        let wav = repository
            .join("apps")
            .join("bootstrapper")
            .join("fixtures")
            .join("smoke.wav");
        for path in [&worker_exe, &wav] {
            assert!(
                path.is_file(),
                "missing {}; see this test's documentation",
                path.display()
            );
        }
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        let model_root = install_root.join(pack.id()).join(pack.revision());
        for required in pack.required_files() {
            assert!(
                model_root.join(required.path()).is_file(),
                "missing {}; see this test's documentation",
                model_root.join(required.path()).display()
            );
        }

        let coordinator = GraniteEngineCoordinator::default();
        let samples = read_wave(&wav);
        let session_id = speakeasy_domain::SessionId::from_bytes([0x5a; 16]);
        let audio = UtteranceAudio {
            session_id,
            sample_rate_hz: 16_000,
            samples,
        };
        let request = AsrRequest {
            correlation_id: speakeasy_domain::CorrelationId::from_bytes([0x5b; 16]),
            session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        };
        let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: Some(&worker_exe),
                install_root: &install_root,
                total_memory_bytes: AMPLE_MEMORY,
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                verifier: &TrustedDigestVerifier,
            },
            &coordinator,
            audio,
            request,
            CancelToken::default(),
        ))
        .expect("Granite must transcribe the fixture without error")
        .expect("Granite must be configured given the files staged above");
        assert_eq!(
            outcome.text, SMOKE_CLIP_TRANSCRIPT,
            "the transcript no longer matches the pinned output"
        );
        assert!(!coordinator.is_quarantined());
    }

    /// The direct proof that `run_granite_final_pass` reuses the resident
    /// worker rather than spawning a fresh one per call: drives it twice
    /// against the *same* coordinator and asserts `Arc::ptr_eq` on the
    /// adapter `ensure_ready` hands back each time -- if either call had
    /// spawned a new process, this would be a different `Arc` (and a second
    /// spawn would also fail outright, since the second dictation pushes audio
    /// into a stream the first `Worker` already believed it had finished, on a
    /// process that only ever answers `Hello` once).
    ///
    /// Same fixture requirements as
    /// [`granite_final_pass_transcribes_the_fixture_through_the_real_worker_process`].
    ///
    /// **This is also the resident-run measurement**, and the only one: `first`
    /// is a cold pass and `second` is the resident one, so the pair is what makes
    /// the resident worker's value legible. It writes them to
    /// `target/debug/granite-resident-timing.txt`. Note that `first` is not
    /// comparable across builds -- it includes this crate's own SHA-256
    /// verification of ~2.3 GB of weights, which a debug build dominates, the
    /// trap `CLAUDE.md` records under "Running the app". `second` is almost
    /// entirely the release worker's own inference and is the number worth
    /// quoting.
    #[test]
    #[ignore = "hardware: needs target/debug/proof/granite-worker.exe and the staged GGUF files"]
    fn run_granite_final_pass_reuses_the_resident_worker_across_dictations() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repository root");
        let target_debug = repository.join("target").join("debug");
        let worker_exe = target_debug.join("proof").join("granite-worker.exe");
        let install_root = target_debug.join("model-lifecycle").join("models");
        let wav = repository
            .join("apps")
            .join("bootstrapper")
            .join("fixtures")
            .join("smoke.wav");
        for path in [&worker_exe, &wav] {
            assert!(
                path.is_file(),
                "missing {}; see this test's documentation",
                path.display()
            );
        }

        let coordinator = GraniteEngineCoordinator::default();
        let samples = read_wave(&wav);
        let run = |seed: u8| {
            let session_id = speakeasy_domain::SessionId::from_bytes([seed; 16]);
            let audio = UtteranceAudio {
                session_id,
                sample_rate_hz: 16_000,
                samples: samples.clone(),
            };
            let request = AsrRequest {
                correlation_id: speakeasy_domain::CorrelationId::from_bytes(
                    [seed.wrapping_add(1); 16],
                ),
                session_id,
                language: AsrLanguage::English,
                task: AsrTask::Transcribe,
            };
            tauri::async_runtime::block_on(run_granite_final_pass(
                GraniteEnvironment {
                    granite_worker_exe: Some(&worker_exe),
                    install_root: &install_root,
                    total_memory_bytes: AMPLE_MEMORY,
                    diagnostic_log: None,
                    recorded_provider: "unrecorded",
                    cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                    verifier: &TrustedDigestVerifier,
                },
                &coordinator,
                audio,
                request,
                CancelToken::default(),
            ))
            .expect("Granite must transcribe the fixture without error")
            .expect("Granite must be configured given the files staged above")
        };

        let started = std::time::Instant::now();
        let first = run(0x5a);
        let first_elapsed = started.elapsed();

        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let pack = granite_pack(&manifest);
        let choice = cpu_choice(pack, &install_root);
        // The resident adapter, asked for the same way a dictation asks.
        let resident = |why: &str| {
            coordinator
                .ensure_ready(
                    &worker_exe,
                    &choice,
                    None,
                    "unrecorded",
                    &speakeasy_models::NvmlCudaContextProbe,
                    &TrustedDigestVerifier,
                )
                .adapter
                .unwrap_or_else(|error| panic!("{why}: {error:?}"))
        };
        let first_adapter = resident("the first dictation must have warmed the engine");

        let started = std::time::Instant::now();
        let second = run(0x6b);
        let second_elapsed = started.elapsed();

        let second_adapter = resident("the engine is still warm after the second dictation");
        record_resident_timing(&target_debug, &worker_exe, first_elapsed, second_elapsed);

        assert!(
            Arc::ptr_eq(&first_adapter, &second_adapter),
            "both dictations must run against the identical resident adapter"
        );
        assert_eq!(
            first.text, SMOKE_CLIP_TRANSCRIPT,
            "the first dictation's transcript no longer matches the pinned output"
        );
        assert_eq!(
            second.text, first.text,
            "two independent dictations against the resident worker must produce the identical transcript"
        );
        assert!(!coordinator.is_quarantined());
    }

    /// Reproduction harness for the stale-clock deadline bug: on the installed
    /// build, 2026-08-04, the resident worker failed on 2 of 4 real
    /// dictations, always fast (~9.6-9.8 s, nowhere near the 90 s deadline)
    /// and silent (no worker stderr, no Windows Application-log entry), and
    /// always following a real idle gap of several minutes on an
    /// already-warmed worker. Every debug-tree test that exercises residency
    /// (including the one directly above this) calls back to back with no
    /// idle gap, which is exactly the shape that never failed on the real
    /// machine either -- so it cannot have caught this. This test inserts
    /// the one missing ingredient: a real sleep between two calls against
    /// the same resident adapter.
    ///
    /// Deliberately **not** a pass/fail gate on which outcome occurs -- the
    /// bug is intermittent (roughly 1-in-2 on the one session that found
    /// it), a fixed sleep is not known to cross whatever the real threshold
    /// is, and asserting success would make this test as likely to be
    /// flaky as the bug itself. What it does assert: the first call must
    /// succeed (that half was reliable every time it was tried for real),
    /// and *if* the second call fails, the failure must be the same
    /// characterized shape (`AdapterFailed`, delivered in well under the
    /// 90 s deadline) rather than some unrelated problem. Either way it
    /// prints the full diagnostic log slice for the second call, including
    /// whatever `protocol_error_kind` (added this phase, alongside this
    /// test) recorded, since that is the detail this bug had none of before.
    ///
    /// Idle gap defaults to 300 s, matching the shorter of the two real gaps
    /// that produced a failure (`GRANITE_IDLE_GAP_SECS` overrides it — sweep
    /// shorter values to find the real threshold). Same fixture requirements as
    /// [`granite_final_pass_transcribes_the_fixture_through_the_real_worker_process`].
    #[test]
    #[ignore = "hardware: needs target/debug/proof/granite-worker.exe and the staged GGUF files; sleeps for the idle gap (default 300s)"]
    fn run_granite_final_pass_survives_an_idle_gap_before_a_second_dictation() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repository root");
        let target_debug = repository.join("target").join("debug");
        let worker_exe = target_debug.join("proof").join("granite-worker.exe");
        let install_root = target_debug.join("model-lifecycle").join("models");
        let wav = repository
            .join("apps")
            .join("bootstrapper")
            .join("fixtures")
            .join("smoke.wav");
        for path in [&worker_exe, &wav] {
            assert!(
                path.is_file(),
                "missing {}; see this test's documentation",
                path.display()
            );
        }

        let diagnostic_log =
            std::env::temp_dir().join(format!("granite-idle-gap-repro-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&diagnostic_log);

        let coordinator = GraniteEngineCoordinator::default();
        let samples = read_wave(&wav);
        let run = |seed: u8| {
            let session_id = speakeasy_domain::SessionId::from_bytes([seed; 16]);
            let audio = UtteranceAudio {
                session_id,
                sample_rate_hz: 16_000,
                samples: samples.clone(),
            };
            let request = AsrRequest {
                correlation_id: speakeasy_domain::CorrelationId::from_bytes(
                    [seed.wrapping_add(1); 16],
                ),
                session_id,
                language: AsrLanguage::English,
                task: AsrTask::Transcribe,
            };
            let started = std::time::Instant::now();
            let outcome = tauri::async_runtime::block_on(run_granite_final_pass(
                GraniteEnvironment {
                    granite_worker_exe: Some(&worker_exe),
                    install_root: &install_root,
                    total_memory_bytes: AMPLE_MEMORY,
                    diagnostic_log: Some(diagnostic_log.clone()),
                    recorded_provider: "unrecorded",
                    cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
                    verifier: &TrustedDigestVerifier,
                },
                &coordinator,
                audio,
                request,
                CancelToken::default(),
            ));
            (outcome, started.elapsed())
        };

        let (first, first_elapsed) = run(0x5a);
        first
            .expect("Granite must transcribe the fixture without error")
            .expect("Granite must be configured given the files staged above");
        println!("idle gap repro: first call ok in {first_elapsed:?}");

        // Cleared so the log slice printed after the second call is only the
        // second call's own -- otherwise it is the first call's (much
        // longer) model-load output with the second call's few lines
        // buried somewhere inside it.
        let _ = std::fs::remove_file(&diagnostic_log);

        let idle_gap = std::env::var("GRANITE_IDLE_GAP_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(Duration::from_mins(5), Duration::from_secs);
        println!("idle gap repro: sleeping {idle_gap:?} before the second call");
        std::thread::sleep(idle_gap);

        let (second, second_elapsed) = run(0x6b);
        let log_tail = std::fs::read_to_string(&diagnostic_log).unwrap_or_default();
        let _ = std::fs::remove_file(&diagnostic_log);
        println!("idle gap repro: second call finished in {second_elapsed:?}");
        println!("idle gap repro: diagnostic log for the second call:\n{log_tail}");

        match second {
            Ok(_) => println!(
                "idle gap repro: did NOT reproduce this run -- the bug is intermittent, this is not a failure of the test"
            ),
            Err(error) => {
                println!("idle gap repro: REPRODUCED -- second call failed with {error:?}");
                assert_eq!(
                    error,
                    domain_error(ErrorCode::AdapterFailed),
                    "a reproduction of the stale-clock deadline bug must fail the same \
                     characterized way (AdapterFailed) -- a different error means this is \
                     a different bug"
                );
                assert!(
                    second_elapsed < Duration::from_secs(90),
                    "a reproduction must be fast, well under GRANITE_FINISH_STREAM_DEADLINE \
                     (90s) -- {second_elapsed:?} is a timeout, not the silent fault this test is for"
                );
            }
        }
    }
}
