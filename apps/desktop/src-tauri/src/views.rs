use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
#[cfg(test)]
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use capture_wizard::{CaptureDeviceView, CaptureWizardCoordinator, CaptureWizardView};
use granite_engine::{
    GraniteEngineCoordinator, GraniteEnvironment, run_granite_final_pass,
    warm_granite_if_configured, GraniteSelection, granite_selection,
};
use process_worker::ProcessWorkerClient;
use runtime_wizard::RuntimeWizardCoordinator;
#[cfg(test)]
use serde::Deserialize;
use serde::Serialize;
use speakeasy_worker::{
    FinalSourceReason, OrderedFinalizationQueue, WorkerFinalAdapter, judge_granite_pass,
};
// The start/stop cues are synthesised tones now rather than two Windows system
// sounds, so they live with the audio stack that already owns `cpal` — see
// `speakeasy-audio/src/cue.rs` for why they are not a `PlaySound` call.
use speakeasy_audio::{RecordingFeedback, play_recording_feedback};
use speakeasy_domain::{
    ActivationEffect, ActivationInput, ActivationMode, ActivationReducer, AsrLanguage, AsrRequest,
    AsrTask, CancelToken, CorrelationId, DOMAIN_SCHEMA_VERSION, Deadline, DeliveryRefusal,
    DomainError, ErrorCode, EngineSnapshot, ExclusiveOperation, FinalAsr, FinalAudioJob,
    FinalTranscript, OperationArbiter, OperationDisposition, SessionId, SystemClock,
    TranscriptProvenance, UtteranceAudio,
};
#[cfg(test)]
use speakeasy_domain::{
    AppReadiness, IngressEvent, ProducerId, Reducer, ReducerDisposition, SessionPhase,
};
use speakeasy_models::{
    Archive, DownloadPolicy, DownloadRequest, GpuProbe, GpuQualification,
    HardwareProbe, InstallManager, InstallSpec, LooseInstallFile, NvmlGpuProbe, Pack, RequiredFile,
    RuntimeEvidence, RuntimeState, SafeStandardHardwareProbe, bundled_manifest, download_to_file,
};
use speakeasy_storage::{
    ActivationHotkeyMode, DEFAULT_ACTIVATION_HOTKEY, HistoryPolicy, HistoryRepository,
    HudDockEdge, HudDockPlacement, ImportChoices, ImportPreview,
    ImportReport, PersonalizationRepository, ProductionImportPlan,
    ProductionImportRoot, ResultProvenance, SafeDeliveryPreference, SessionResultList, Settings,
    SettingsStore, TranscriptResult, WritingRulePreferences,
    clear_pending_update_after_health_checks, extract_v1_protected_terms,
};
use speakeasy_transforms::{
    DictionaryEntry, DictionarySet, ImportPolicy as PersonalizationImportPolicy,
    PersonalizationBundle, PipelineMode, PipelineRequest, RuleCleanupConfig, RuleCleanupMode,
    Snippet, SnippetSet, TransformPipeline,
};
use speakeasy_windows::{
    ClipboardWriter, CommitWriter, Confirmation, LegacyCredentialSource, TargetObserver,
    WindowsCredentialManager, confirm_destructive_action, migrate_legacy_startup,
    set_startup_with_windows, startup_status,
};
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(test)]
const FAKE_TRANSCRIPT: &str = "This is a private fake transcript.";
#[cfg(test)]
const AUDIT_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg(test)]
pub enum FakeFailure {
    AudioStart,
    Finalize,
    Delivery,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg(test)]
pub struct FakeFlowRequest {
    pub schema_version: u16,
    pub failure: Option<FakeFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg(test)]
pub struct IpcState {
    pub schema_version: u16,
    pub sequence: u64,
    pub readiness: &'static str,
    pub session: &'static str,
    pub engine: &'static str,
    pub delivery: &'static str,
    pub transcript: Option<&'static str>,
    pub error_code: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg(test)]
pub struct FakeFlowResponse {
    pub schema_version: u16,
    pub states: Vec<IpcState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(test)]
struct RedactedAuditEvent {
    code: &'static str,
    transcript_characters: usize,
}

#[derive(Debug)]
#[cfg(test)]
struct FakeActorRequest {
    failure: Option<FakeFailure>,
    reply: SyncSender<Result<FakeFlowResponse, &'static str>>,
}

#[derive(Debug)]
#[cfg(test)]
pub struct Phase1Coordinator {
    audit: Arc<Mutex<VecDeque<RedactedAuditEvent>>>,
    requests: SyncSender<FakeActorRequest>,
}

#[cfg(test)]
impl Default for Phase1Coordinator {
    fn default() -> Self {
        let audit = Arc::new(Mutex::new(VecDeque::new()));
        let (requests, receiver) = sync_channel::<FakeActorRequest>(8);
        let worker_audit = Arc::clone(&audit);
        thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let response = catch_unwind(AssertUnwindSafe(|| {
                    Self::run_fake_inner(&worker_audit, request.failure)
                }))
                .map_err(|_| "internal_panic")
                .and_then(|result| result);
                let _ = request.reply.try_send(response);
            }
        });
        Self { audit, requests }
    }
}

// Four independent yes/no facts about one pack, each answering a different
// question the row has to render: is it a candidate build, may it be fetched,
// is it here, must installation be confirmed. Collapsing them into an enum
// would invent states that cannot occur (`downloadable` and `installed` are
// orthogonal — the GPU pack is installed-and-never-downloadable).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize)]
pub struct ModelCatalogItem {
    id: String,
    revision: String,
    display_name: String,
    archive_bytes: u64,
    installed_bytes: u64,
    confirmation_required: bool,
    source_repository: String,
    source_revision: String,
    license_name: String,
    license_spdx: Option<String>,
    license_url: String,
    runtime: String,
    provider: String,
    capabilities: Vec<String>,
    hardware_evidence: String,
    /// Whether this pack can be fetched by the app at all. A pack with no
    /// archive URL installs only from an archive supplied on disk, so offering
    /// the user an Install button for it is offering something that cannot
    /// happen — which is exactly what the GPU pack's row did.
    downloadable: bool,
    /// Whether *this* pack is on disk. Per row, because install state is per
    /// pack: the UI previously rendered one global coordinator state against
    /// every row, so with two packs admitted both claimed the same
    /// installedness and Remove was live on a pack that was never installed.
    installed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelHardwareView {
    operating_system: String,
    operating_system_build: Option<String>,
    architecture: String,
    physical_cores: Option<usize>,
    logical_processors: usize,
    has_avx2: bool,
    total_memory_bytes: Option<u64>,
    available_disk_bytes: Option<u64>,
    adapters: Vec<String>,
    /// Whether a model has actually run on this machine's GPU.
    ///
    /// This was a hardcoded `false` with no branch that could set it, which was
    /// honest but useless: it could not distinguish a machine with no Nvidia
    /// card from one that has never been asked to try. It is still never set
    /// true by inventory alone — see [`GpuStatusView`] for the distinction.
    qualified: bool,
}

/// The GPU decision, as the setup UI needs to render it.
///
/// Separate from [`ModelHardwareView`] because the two answer different
/// questions and are read at different times. `model_hardware` describes the
/// host; this says whether the app can run here and, when it cannot, why — the
/// difference between "install a driver" and "this card is too old" is the
/// whole content of the message a blocked user gets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GpuStatusView {
    /// A stable code, never a device name: this reaches the diagnostic log,
    /// which is a privacy surface.
    status: String,
    /// True only once an execution test has passed. Inventory never sets it.
    qualified: bool,
    /// True when the card cleared the capability floor but nothing has run on
    /// it yet. Distinct from `qualified` on purpose.
    admissible: bool,
    adapter_name: Option<String>,
    compute_capability: Option<String>,
    total_vram_bytes: Option<u64>,
    free_vram_bytes: Option<u64>,
    driver_version: Option<String>,
    minimum_compute_capability: String,
    /// The provider the streaming pack actually resolved to, or `None` when no
    /// pack is installed for any provider.
    ///
    /// Deliberately distinct from what the probe prefers. An admissible card
    /// whose pack was never installed runs on CPU, and a user told only
    /// "GPU detected" would have no way to find that out.
    active_provider: Option<String>,
    /// A stable code for why that engine and not another. See
    /// [`granite_engine::EngineChoiceReason::code`].
    engine_reason: String,
}

impl GpuStatusView {
    fn from_snapshot(
        snapshot: &speakeasy_models::GpuSnapshot,
        selection: Option<&GraniteSelection>,
        qualification: &GpuQualification,
    ) -> Self {
        let decision = qualification;
        let device = decision.device();
        Self {
            active_provider: selection
                .map(|selection| selection.capabilities.provider.to_owned()),
            engine_reason: selection
                .map_or("no_pack_installed", |selection| selection.reason.code())
                .to_owned(),
            status: decision.code(),
            qualified: decision.is_qualified(),
            admissible: device.is_some(),
            adapter_name: device.map(|device| device.name.clone()),
            compute_capability: device.map(|device| device.compute_capability.to_string()),
            total_vram_bytes: device.map(|device| device.total_vram_bytes),
            free_vram_bytes: device.map(|device| device.free_vram_bytes),
            driver_version: snapshot.driver_version.clone(),
            minimum_compute_capability: speakeasy_models::MINIMUM_COMPUTE_CAPABILITY.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelInstallView {
    state: String,
    error: Option<String>,
    bytes_downloaded: Option<u64>,
    bytes_total: Option<u64>,
}

/// The CUDA runtime as an offer: what it costs, whether it is here, and what an
/// install is currently doing.
///
/// Separate from [`ModelInstallView`] rather than folded into it, and that is
/// load-bearing rather than tidiness: `setup_requirement` reads the model
/// coordinator's state and treats anything other than `verified_on_disk` as
/// `model_missing`. A runtime download writing `downloading` there would make a
/// perfectly ready app announce "Setup needed" for the twenty minutes it takes
/// to fetch 2.97 GB.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CudaRuntimeView {
    /// `absent`, `partial`, `downloading`, `installing`, `installed`,
    /// `cancelled`, or `failed`.
    ///
    /// `partial` is a real state, not a rounding of `absent`: 2 GB of a 2.45 GB
    /// runtime is on disk and cannot run, and a user who paid for that transfer
    /// is owed something better than being told nothing happened.
    state: String,
    error: Option<String>,
    /// Whether to offer this at all. False when the probe found no admissible
    /// card, because fetching 2.97 GB of CUDA for a machine that cannot use it
    /// is pure cost.
    offered: bool,
    /// What the offer must show. Never presented without these.
    download_bytes: u64,
    installed_bytes: u64,
    file_count: u32,
    /// Component codes already fully present, for a resumed install.
    installed_components: Vec<String>,
    bytes_downloaded: Option<u64>,
    bytes_total: Option<u64>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize)]
pub struct ProfileView {
    schema_version: u16,
    startup_with_windows: bool,
    history_enabled: bool,
    history_retention_days: u16,
    history_plaintext_disclosure_accepted: bool,
    delivery_preference: SafeDeliveryPreference,
    recording_feedback_enabled: bool,
    disk_logging_enabled: bool,
    /// The microphone dictation will actually record from, so the Audio page can
    /// show that rather than the OS-reported default. The two are frequently
    /// different, and a page claiming a device that will not be used is wrong.
    preferred_capture_device_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticsView {
    schema_version: u16,
    engine: String,
    worker: String,
    runtime: String,
    provider: String,
    rtf_median: Option<f64>,
    rtf_p95: Option<f64>,
    latency_p50_ms: Option<u64>,
    latency_p95_ms: Option<u64>,
    audio_overflow_count: u64,
    device: String,
    vad: String,
    delivery_capability: String,
    delivery_reason: String,
    model_id: String,
    model_revision: String,
    model_source: String,
    final_source_reason: Option<String>,
    recent_reason_codes: Vec<String>,
    logs_sanitized: bool,
}

/// Runtime measurements that cannot be reconstructed from the model manifest
/// alone. The coordinator deliberately keeps only bounded counters and codes;
/// it never stores audio, hypotheses, or transcript text.
#[derive(Clone, Debug, Default)]
struct DiagnosticsRuntimeSnapshot {
    rtf_median: Option<f64>,
    rtf_p95: Option<f64>,
    latency_p50_ms: Option<u64>,
    latency_p95_ms: Option<u64>,
    final_source_reason: Option<String>,
}

#[derive(Debug, Default)]
struct DiagnosticsRuntimeCoordinator {
    snapshot: Mutex<DiagnosticsRuntimeSnapshot>,
    recent_reason_codes: Mutex<VecDeque<String>>,
}

impl DiagnosticsRuntimeCoordinator {
    /// Records why the last dictation produced no text.
    ///
    /// This field is named `final_source_reason` and used to mean something
    /// else: *which* engine supplied a transcript that still arrived, after
    /// Granite had been rejected. There is no second engine, so the field now
    /// carries the reason nothing arrived at all — the same
    /// `FinalSourceReason::code()` values, reporting an outcome rather than a
    /// substitution.
    ///
    /// Cleared on success, deliberately. A stale reason from three dictations
    /// ago sitting under a working engine is worse than no reason: it invites
    /// someone to fix a problem that is already gone.
    fn record_final_source(&self, reason: Option<&str>) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.final_source_reason = reason.map(str::to_owned);
        }
    }

    fn snapshot(&self) -> DiagnosticsRuntimeSnapshot {
        self.snapshot.lock().map_or_else(
            |_| DiagnosticsRuntimeSnapshot::default(),
            |snapshot| snapshot.clone(),
        )
    }

    fn record_event(&self, line: &str) {
        // The in-memory path receives the same redaction as the disk boundary.
        // This is deliberate: the buffer is always on, so it must not become a
        // back door for native paths or future unstructured error strings.
        let event = bounded_diagnostic_text(&redact_diagnostic_text(line.trim_end()));
        if let Ok(mut events) = self.recent_reason_codes.lock() {
            if events.len() == DIAGNOSTICS_EVENT_CAPACITY {
                events.pop_front();
            }
            events.push_back(event);
        }
    }

    fn recent_reason_codes(&self) -> Vec<String> {
        self.recent_reason_codes
            .lock()
            .map_or_else(|_| Vec::new(), |events| events.iter().cloned().collect())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticsExportView {
    file_name: String,
    categories: Vec<String>,
    contains_sensitive_content: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CredentialStatusView {
    openai_legacy: String,
    remote_legacy: String,
    values_exposed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResetPreviewView {
    nonce: String,
    categories: Vec<String>,
    excludes_v1: bool,
    excludes_custom_models: bool,
    excludes_credentials: bool,
}

/// Everything the compact transcriber needs, in one 100 ms poll.
///
/// The device name, shortcut binding and gating flags live here rather than in
/// three extra commands because the alternative triples IPC traffic at 10 Hz
/// for data that changes rarely.
///
/// Adding fields is backward compatible, so `schema_version` is unchanged: it
/// is bumped only when a field is removed or retyped.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CaptureHudView {
    schema_version: u16,
    sequence: u64,
    /// Identifies the dictation this view describes, so the frontend can drop a
    /// late response belonging to a session that has already been superseded.
    session_id: String,
    session: String,
    vad: String,
    level: f32,
    device_diagnostic: String,
    streaming_mode: String,
    mutable_text: String,
    stable_display_text: String,
    final_text: String,
    device_name: String,
    hotkey_binding: String,
    hotkey_registration: String,
    can_start: bool,
    can_stop: bool,
    setup_complete: bool,
    setup_reason: Option<String>,
    elapsed_ms: u64,
    ceiling_ms: u64,
    /// The microphone the next dictation will actually record from, so the
    /// picker can show it instead of claiming nothing is selected. Empty when no
    /// preference is stored, in which case the resolution falls back to the
    /// OS-reported default exactly as `hotkey_capture_device` does.
    preferred_device_id: String,
    /// What actually happened to the final: `inserted` only when
    /// `CommitWriter::write_focused` returned `Ok`. See UI-GUIDE's
    /// truthful-disclosure rule.
    delivery_outcome: String,
    /// The resident engine's warm state: `cold`, `warming`, `ready`, or an
    /// error code.
    ///
    /// Here because it is otherwise unobservable, and its absence was a lie by
    /// omission: a verified-on-disk model reports `setup_complete: true` and
    /// `can_start: true` the instant the app launches, while the launch warm is
    /// still loading the model. A start landing in that window blocks inside
    /// `dictation_start` on the same mutex for up to `WARM_TIMEOUT`, with
    /// nothing on screen to explain the wait.
    ///
    /// Not a `setup_reason`: loading is not a thing the user has to go and fix,
    /// and routing it through `setup_reason` would clear `setup_complete` and
    /// so also `can_start`.
    engine: String,
    /// Number of finalized utterances waiting for the single finalizer.
    queue_depth: usize,
    error_code: Option<String>,
    /// Disclosed only when Granite did not deliver and the retained fallback
    /// supplied the final. See [`HudLiveState::final_source_reason`].
    final_source_reason: Option<String>,
}

/// The part of the HUD view the dictation itself owns.
///
/// Session state is derived from the capture coordinator at read time rather
/// than mirrored here, so the HUD reports the correct state even when streaming
/// is unavailable and no live tap ever runs.
#[derive(Clone, Debug, Default)]
struct HudLiveState {
    session_id: Option<SessionId>,
    streaming_mode: Option<&'static str>,
    mutable_text: String,
    stable_display_text: String,
    final_text: String,
    delivery_outcome: Option<&'static str>,
    /// A [`speakeasy_worker::FinalSourceReason`] code, e.g. `granite_failed`,
    /// disclosing why Granite did not deliver. `None` when Granite delivered
    /// or was never configured for this dictation.
    final_source_reason: Option<&'static str>,
}

#[derive(Debug)]
pub struct CaptureHudCoordinator {
    live: Mutex<HudLiveState>,
    /// The last view served, so `sequence` advances on any observable change —
    /// including one where only the composed fields moved.
    published: Mutex<CaptureHudView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HotkeyView {
    binding: String,
    mode: String,
    registration: String,
    enabled: bool,
    active: bool,
}

/// Product policy: manual press-to-stop is the normal endpoint, with no VAD.
/// Every dictation has a two-minute safety ceiling; hitting it stops capture
/// and delivers the retained audio through the same path as a manual stop — see
/// `capture_wizard::MAX_CAPTURE_SECONDS`.
const DICTATION_CEILING_SECONDS: u32 = capture_wizard::MAX_CAPTURE_SECONDS;
/// Bound on waiting for the capture thread to drain after an explicit stop.
const CAPTURE_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Bound on waiting for the activation modifiers to be released before pasting.
const COMMIT_MODIFIER_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HotkeyAction {
    Start(SessionId),
    Stop,
}

#[derive(Debug)]
pub struct HotkeyCoordinator {
    binding: Mutex<String>,
    mode: Mutex<ActivationMode>,
    enabled: Mutex<bool>,
    registration: Mutex<&'static str>,
    activation: Mutex<ActivationReducer>,
    active_session: Mutex<Option<SessionId>>,
    last_press: Mutex<Option<Instant>>,
}

impl Default for HotkeyCoordinator {
    fn default() -> Self {
        Self {
            binding: Mutex::new(DEFAULT_ACTIVATION_HOTKEY.to_owned()),
            mode: Mutex::new(ActivationMode::Toggle),
            enabled: Mutex::new(true),
            registration: Mutex::new("pending"),
            activation: Mutex::new(ActivationReducer::default()),
            active_session: Mutex::new(None),
            last_press: Mutex::new(None),
        }
    }
}

impl HotkeyCoordinator {
    const DEBOUNCE: Duration = Duration::from_millis(150);

    fn view(&self) -> Result<HotkeyView, &'static str> {
        let active = self
            .active_session
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?
            .is_some();
        let registration = *self
            .registration
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?;
        let binding = self
            .binding
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?
            .clone();
        let mode = *self.mode.lock().map_err(|_| "hotkey_state_unavailable")?;
        let enabled = *self
            .enabled
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?;
        Ok(HotkeyView {
            binding,
            mode: match mode {
                ActivationMode::Toggle => "toggle",
                ActivationMode::PushToTalk => "push_to_talk",
                ActivationMode::HandsFree => "hands_free",
            }
            .to_owned(),
            registration: registration.to_owned(),
            enabled,
            active,
        })
    }

    fn on_event(&self, state: ShortcutState) -> Option<HotkeyAction> {
        let mut active_session = self.active_session.lock().ok()?;
        let mut activation = self.activation.lock().ok()?;
        let mode = self.mode.lock().map(|mode| *mode).ok()?;
        let session_id = active_session.unwrap_or_else(new_session_id);
        let input = match (mode, state) {
            (ActivationMode::Toggle, ShortcutState::Pressed) => {
                let mut last_press = self.last_press.lock().ok()?;
                let now = Instant::now();
                if last_press.is_some_and(|last| now.duration_since(last) < Self::DEBOUNCE) {
                    return None;
                }
                *last_press = Some(now);
                ActivationInput::TogglePressed { session_id }
            }
            (ActivationMode::PushToTalk, ShortcutState::Pressed) => {
                ActivationInput::PushToTalkPressed { session_id }
            }
            (ActivationMode::PushToTalk, ShortcutState::Released) => {
                ActivationInput::PushToTalkReleased { session_id }
            }
            (ActivationMode::HandsFree, ShortcutState::Pressed) => {
                ActivationInput::HandsFreePressed { session_id }
            }
            (ActivationMode::Toggle | ActivationMode::HandsFree, ShortcutState::Released) => {
                return None;
            }
        };
        match activation.apply(input) {
            ActivationEffect::Start { session_id, .. } => {
                *active_session = Some(session_id);
                Some(HotkeyAction::Start(session_id))
            }
            ActivationEffect::Stop { .. } => {
                *active_session = None;
                Some(HotkeyAction::Stop)
            }
            ActivationEffect::Ignored
            | ActivationEffect::StaleSession
            | ActivationEffect::SessionAlreadyActive => None,
        }
    }

    /// Resolves a Start pressed in the compact transcriber.
    ///
    /// Deliberately the same `active_session`, `activation` reducer and
    /// `last_press` debounce the shortcut uses. A button press and a shortcut
    /// press are therefore competing for one session rather than opening two —
    /// the single-controller rule exists to enforce.
    fn request_start(&self) -> Option<HotkeyAction> {
        let mut active_session = self.active_session.lock().ok()?;
        let mut activation = self.activation.lock().ok()?;
        if !self.accept_press()? {
            return None;
        }
        let session_id = new_session_id();
        match activation.apply(ActivationInput::TogglePressed { session_id }) {
            ActivationEffect::Start { session_id, .. } => {
                *active_session = Some(session_id);
                Some(HotkeyAction::Start(session_id))
            }
            _ => None,
        }
    }

    /// Resolves a Stop pressed in the compact transcriber. Stops whichever
    /// session is active, whatever activation mode started it.
    fn request_stop(&self) -> Option<HotkeyAction> {
        let mut active_session = self.active_session.lock().ok()?;
        let mut activation = self.activation.lock().ok()?;
        let session_id = (*active_session)?;
        if !self.accept_press()? {
            return None;
        }
        match activation.apply(ActivationInput::ManualStop { session_id }) {
            ActivationEffect::Stop { .. } => {
                *active_session = None;
                Some(HotkeyAction::Stop)
            }
            _ => None,
        }
    }

    /// Records a press and reports whether it cleared the debounce window.
    fn accept_press(&self) -> Option<bool> {
        let mut last_press = self.last_press.lock().ok()?;
        let now = Instant::now();
        if last_press.is_some_and(|last| now.duration_since(last) < Self::DEBOUNCE) {
            return Some(false);
        }
        *last_press = Some(now);
        Some(true)
    }

    fn abandon_active_session(&self) {
        if let Ok(mut active_session) = self.active_session.lock() {
            active_session.take();
        }
        if let Ok(mut activation) = self.activation.lock() {
            *activation = ActivationReducer::default();
        }
    }
}

fn new_session_id() -> SessionId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[8..].copy_from_slice(&Instant::now().elapsed().as_nanos().to_le_bytes()[..8]);
    SessionId::from_bytes(bytes)
}

fn register_activation_hotkey(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let coordinator = app.state::<HotkeyCoordinator>();
    if !*coordinator
        .enabled
        .lock()
        .map_err(|_| "hotkey_state_unavailable")?
    {
        *coordinator
            .registration
            .lock()
            .map_err(|_| "hotkey_state_unavailable")? = "disabled";
        return Ok(());
    }
    let binding = coordinator
        .binding
        .lock()
        .map_err(|_| "hotkey_state_unavailable")?
        .clone();
    let registration =
        app.global_shortcut()
            .on_shortcut(binding.as_str(), |app, _shortcut, event| {
                let coordinator = app.state::<HotkeyCoordinator>();
                // **Nothing expensive may run here.** This is the whole distance
                // between the user's key press and the microphone opening.
                //
                // A full UI Automation snapshot of the foreground window used to
                // run first, as a pre-flight check that a delivery target was
                // observable. Measured cost of that check: 68 ms into an empty
                // Notepad, 1.7 s into VS Code, 12.8 s into a WebView2 window —
                // and the snapshot it produced was stored, never read, and
                // discarded at stop. Delivery inspects the target afresh when it
                // actually needs it (see `deliver_final`), so the pre-flight
                // bought nothing and cost the user the start of every sentence.
                let action = coordinator.on_event(event.state);
                match action {
                    Some(HotkeyAction::Start(session_id)) => {
                        if start_dictation(app, session_id).is_err() {
                            app.state::<HotkeyCoordinator>().abandon_active_session();
                        }
                    }
                    Some(HotkeyAction::Stop) => {
                        let _ = stop_dictation(app);
                    }
                    None => {}
                }
            });
    let coordinator = app.state::<HotkeyCoordinator>();
    let mut status = coordinator
        .registration
        .lock()
        .map_err(|_| "hotkey_state_unavailable")?;
    *status = if registration.is_ok() {
        "registered"
    } else {
        "conflict"
    };
    registration.map_err(|_| "hotkey_conflict")
}

/// Picks the microphone for hotkey dictation: prefers whatever device the
/// user last explicitly selected in the manual capture flow (persisted in
/// settings), since that is not necessarily the OS-reported default device.
/// Falls back to guessing a default when no preference is saved, or when the
/// saved device has disappeared or is no longer supported.
fn hotkey_capture_device(app: &tauri::AppHandle) -> Result<String, &'static str> {
    let devices = CaptureWizardCoordinator::devices()?;
    let preferred = app
        .state::<ProfileCoordinator>()
        .settings
        .lock()
        .ok()
        .and_then(|settings| settings.preferred_capture_device_id.clone());
    if let Some(preferred_id) = preferred.as_deref()
        && let Some(device) = devices
            .iter()
            .find(|device| device.id == preferred_id && device.supported)
    {
        log_event(
            app,
            "hotkey_capture_device_selected",
            &[("source", "preferred_setting")],
        );
        return Ok(device.id.clone());
    }
    let fallback = devices
        .iter()
        .find(|device| device.is_default && device.supported)
        .or_else(|| devices.iter().find(|device| device.supported))
        .map(|device| device.id.clone())
        .ok_or("capture_device_unavailable");
    log_event(
        app,
        "hotkey_capture_device_selected",
        &[(
            "source",
            if preferred.is_some() {
                "fallback_preferred_unavailable"
            } else {
                "fallback_no_preference_saved"
            },
        )],
    );
    fallback
}

/// Starts one dictation. The single implementation behind both the global
/// shortcut and the transcriber's Start button.
///
/// There is deliberately no second start path: `capture_start` stops and does
/// not deliver, so a dictation begun through it would silently skip the paste
/// that the identical action from the shortcut performs.
fn start_dictation(app: &tauri::AppHandle, session_id: SessionId) -> Result<(), &'static str> {
    let device_id = hotkey_capture_device(app)?;
    let capture = app.state::<CaptureWizardCoordinator>();
    let operations = app.state::<OperationCoordinator>();
    app.state::<CaptureHudCoordinator>().begin(session_id);
    let result = capture.start_for_session(
        &device_id,
        DICTATION_CEILING_SECONDS,
        session_id,
        // No tap. This took a live streaming tap that fed words to the HUD as
        // the user spoke; capture now just records, and the transcript exists
        // only after the recording stops.
        None,
        || operations.replace_completed_dictation(session_id),
    );
    log_event_for_session(
        app,
        session_id,
        "dictation_start",
        &[("result", result.as_ref().err().copied().unwrap_or("ok"))],
    );
    if result.is_err() {
        operations.finish_dictation();
        return result;
    }
    if app
        .state::<ProfileCoordinator>()
        .settings
        .lock()
        .is_ok_and(|settings| settings.delivery.feedback_enabled)
    {
        play_recording_feedback(RecordingFeedback::Started);
    }
    watch_for_unattended_capture_end(app, session_id);
    Ok(())
}

/// Stops one dictation, transcribes it and delivers the authoritative final.
/// The single implementation behind both the global shortcut and the
/// transcriber's Stop & transcribe button, so the two cannot diverge.
fn stop_dictation(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let capture = app.state::<CaptureWizardCoordinator>();
    if let Err(code) = capture.stop() {
        log_event(app, "dictation_stop", &[("result", code)]);
        return Err(code);
    }
    log_event(app, "dictation_stop", &[("result", "ok")]);
    announce_capture_stopped(app);
    transcribe_and_deliver(app);
    Ok(())
}

fn announce_capture_stopped(app: &tauri::AppHandle) {
    if app
        .state::<ProfileCoordinator>()
        .settings
        .lock()
        .is_ok_and(|settings| settings.delivery.feedback_enabled)
    {
        play_recording_feedback(RecordingFeedback::Stopped);
    }
}

/// Waits for the capture to drain, transcribes it and delivers the final.
///
/// Shared by the user-initiated stop and by the safety ceiling, so both reach
/// delivery through exactly the same code.
fn transcribe_and_deliver(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(code) = wait_for_captured_audio(&app) {
            log_event(&app, "dictation_drain", &[("result", code)]);
            app.state::<OperationCoordinator>().finish_dictation();
            return;
        }
        let capture = app.state::<CaptureWizardCoordinator>();
        let audio = match capture.retained_audio() {
            Ok(audio) => audio,
            Err(code) => {
                log_event(&app, "dictation_queue", &[("result", code)]);
                app.state::<OperationCoordinator>().finish_dictation();
                return;
            }
        };
        let request = request_for_audio(&audio);
        let engine = final_engine_snapshot(&app);
        if let Ok(utterance_id) = app
            .state::<OrderedFinalizationQueue>()
            .submit(audio, request, engine)
        {
            let id = utterance_id.value().to_string();
            log_event(
                &app,
                "dictation_queue",
                &[("result", "accepted"), ("utterance_id", id.as_str())],
            );
        } else {
            log_event(&app, "dictation_queue", &[("result", "consumer_stopped")]);
            app.state::<OperationCoordinator>().finish_dictation();
        }
    });
}

/// Captures the engine identity at the immutable audio handoff boundary.
///
/// Recorded when the audio is handed off rather than when the pass runs, so a
/// dictation's log line names the engine that was resolved for *it* even if
/// the engine is invalidated and re-warmed while it is queued.
fn final_engine_snapshot(app: &tauri::AppHandle) -> EngineSnapshot {
    let models = app.state::<ModelCoordinator>();
    let granite = app.state::<GraniteEngineCoordinator>();
    granite_selection(&models.root.join("models"), granite.cuda_worker_available()).map_or_else(
        || EngineSnapshot::new("unresolved", "unresolved", "unresolved"),
        |selection| {
            EngineSnapshot::new(
                selection.capabilities.runtime,
                &selection.pack_id,
                selection.capabilities.provider,
            )
        },
    )
}

/// Runs one queued job and keeps all final transcription/delivery behavior in
/// the existing authoritative path. The queue worker is deliberately the only
/// finalization consumer, so utterances cannot race one another.
fn process_finalization_job(app: &tauri::AppHandle, job: FinalAudioJob) {
    let id = job.utterance_id.value().to_string();
    log_event(
        app,
        "dictation_finalize",
        &[
            ("result", "started"),
            ("utterance_id", id.as_str()),
            ("engine", job.engine.runtime.as_str()),
        ],
    );
    let outcome = tauri::async_runtime::block_on(run_retained_transcription(
        app,
        job.audio,
        job.request,
    ));
    match outcome {
        Ok(text) => deliver_final_text(app, &text),
        Err(code) if is_no_speech(code) => deliver_final_text(app, ""),
        Err(code) => log_event(app, "dictation_transcription", &[("result", code)]),
    }
    log_event(
        app,
        "dictation_finalize",
        &[("result", "finished"), ("utterance_id", id.as_str())],
    );
}

/// Watches a running dictation so that a capture ending on its own — the safety
/// ceiling, or a device fault — still reaches transcription and delivery.
///
/// Without this a capture that runs to the ceiling is simply lost: the audio is
/// retained and nothing ever transcribes it, the activation reducer still
/// believes a session is live, and the user's next press is read as a Stop that
/// finds nothing to stop. Hitting a duration limit must cost the user nothing
/// but the recording's tail.
fn watch_for_unattended_capture_end(app: &tauri::AppHandle, session_id: SessionId) {
    let app = app.clone();
    std::thread::spawn(move || {
        let capture = app.state::<CaptureWizardCoordinator>();
        loop {
            std::thread::sleep(Duration::from_millis(200));
            // A stop the user asked for is already being finished by
            // `stop_dictation`; this watcher exists only for the other ways a
            // capture can end.
            if capture.stop_was_requested() {
                return;
            }
            let Ok(view) = capture.view() else { return };
            if view.can_stop || view.state == "arming" {
                continue;
            }
            // Capture is over and nobody asked for it. Release the activation
            // session so the next press starts cleanly, then finish exactly as
            // a user stop would.
            let hotkey = app.state::<HotkeyCoordinator>();
            if hotkey.request_stop().is_none() {
                hotkey.abandon_active_session();
            }
            let delivered = view.can_transcribe;
            log_event_for_session(
                &app,
                session_id,
                "dictation_ceiling_stop",
                &[
                    ("result", if delivered { "delivering" } else { "no_audio" }),
                    ("state", view.state.as_str()),
                ],
            );
            if delivered {
                announce_capture_stopped(&app);
                transcribe_and_deliver(&app);
            } else {
                app.state::<OperationCoordinator>().finish_dictation();
            }
            return;
        }
    });
}

/// Waits for the capture thread to drain and retain the utterance.
fn wait_for_captured_audio(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let capture = app.state::<CaptureWizardCoordinator>();
    let deadline = Instant::now() + CAPTURE_DRAIN_TIMEOUT;
    loop {
        let view = capture.view()?;
        if view.can_transcribe {
            return Ok(());
        }
        if view.state == "failed" {
            return Err("capture_failed");
        }
        if Instant::now() >= deadline {
            return Err("capture_drain_timeout");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Copies and pastes the final transcript into the target focused right now.
///
/// Refusals no longer go unmentioned: the transcriber reports what actually
/// happened, because UI-GUIDE's truthful-disclosure rule forbids showing "Text
/// inserted" unless `CommitWriter::write_focused` returned `Ok`. The text stays
/// recoverable either way, and refusals are still logged (sanitized: refusal
/// reason only, never the text) when disk logging is enabled. Delivers an
/// accepted final, and reports what became of it.
///
/// This used to carry a `source_reason` alongside the text: why Granite had
/// not delivered, on a transcript the streaming fallback had produced
/// instead. Reaching this function at all now means Granite delivered, so
/// there is nothing left to disclose here. A rejected pass never gets this
/// far -- its reason is the dictation's error, reported on the failure path
/// in `run_retained_transcription`.
fn deliver_final_text(app: &tauri::AppHandle, text: &str) {
    let hud = app.state::<CaptureHudCoordinator>();
    if text.trim().is_empty() {
        log_event(app, "hotkey_delivery", &[("result", "empty_text")]);
        hud.finish(text, "held", None);
        return;
    }
    let auto_paste = app
        .state::<ProfileCoordinator>()
        .settings
        .lock()
        .is_ok_and(|settings| settings.delivery.auto_paste);
    if !auto_paste {
        log_event(app, "hotkey_delivery", &[("result", "auto_paste_disabled")]);
        hud.finish(text, "held", None);
        return;
    }
    let session_id = new_session_id();
    let observer = app.state::<TargetObserver>();
    let snapshot = match observer.inspect(session_id) {
        Ok(snapshot) => snapshot,
        Err(refusal) => {
            let reason = format!("{refusal:?}");
            let os_error = observer.last_os_error();
            log_target_inspect_refusal(app, &reason, os_error);
            let outcome = deliver_via_clipboard_fallback(app, session_id, text, refusal);
            hud.finish(text, outcome, None);
            return;
        }
    };
    // Sanitized: the focused executable's own path and integrity relationship
    // are not user content, so they are safe to log even though the transcript
    // text never is. This is what actually explains ElevatedTarget refusals.
    let integrity = format!("{:?}", snapshot.integrity);
    let executable = snapshot.executable.path.clone();
    let process_id = snapshot.process_id.to_string();
    log_event(
        app,
        "hotkey_delivery_target",
        &[
            ("integrity", integrity.as_str()),
            ("executable", executable.as_str()),
            ("process_id", process_id.as_str()),
        ],
    );
    let result = app.state::<CommitWriter>().write_focused(
        snapshot,
        text.to_owned(),
        Instant::now() + COMMIT_MODIFIER_TIMEOUT,
    );
    match result {
        Ok(_) => {
            log_event(app, "hotkey_delivery", &[("result", "committed")]);
            hud.finish(text, "inserted", None);
        }
        Err(refusal) => {
            let reason = format!("{refusal:?}");
            log_event(
                app,
                "hotkey_delivery",
                &[("result", "commit_refused"), ("reason", reason.as_str())],
            );
            let outcome = deliver_via_clipboard_fallback(app, session_id, text, refusal);
            hud.finish(text, outcome, None);
        }
    }
}

/// Logs why the target couldn't even be inspected, including the sanitized
/// numeric OS error behind a `TargetInaccessible` refusal when one was
/// captured. Only the code is logged, never the OS-provided message text —
/// the same sanitization the rest of hotkey delivery logging already applies.
fn log_target_inspect_refusal(app: &tauri::AppHandle, reason: &str, os_error: Option<u32>) {
    match os_error {
        Some(code) => {
            let code = code.to_string();
            log_event(
                app,
                "hotkey_delivery",
                &[
                    ("result", "target_inspect_refused"),
                    ("reason", reason),
                    ("os_error", code.as_str()),
                ],
            );
        }
        None => {
            log_event(
                app,
                "hotkey_delivery",
                &[("result", "target_inspect_refused"), ("reason", reason)],
            );
        }
    }
}

/// Falls back to a clipboard-only copy when paste was refused or the target
/// could not be inspected at all, so a dictation is never silently dropped
/// just because delivery into the focused app couldn't proceed.
///
/// Refused for the same handful of reasons `classify_guard` refuses
/// everything: a password field, secure desktop, genuinely higher-integrity
/// target, or another unknown-sensitive target must never receive a clipboard
/// write either. Every other refusal still gets a best-effort copy —
/// including `TargetInaccessible` (New Outlook's `AppContainer` denying even
/// `PROCESS_QUERY_LIMITED_INFORMATION`), where `SpeakEasy` never learned
/// whether the focused control was sensitive. That is a deliberate,
/// user-chosen trade-off: losing the transcript silently was judged worse
/// than the residual chance the focused field turns out to have been one.
fn deliver_via_clipboard_fallback(
    app: &tauri::AppHandle,
    session_id: SessionId,
    text: &str,
    original_refusal: DeliveryRefusal,
) -> &'static str {
    if matches!(
        original_refusal,
        DeliveryRefusal::Password
            | DeliveryRefusal::SecureDesktop
            | DeliveryRefusal::ElevatedTarget
            | DeliveryRefusal::UnknownSensitive
    ) {
        log_event(
            app,
            "hotkey_delivery",
            &[("result", "clipboard_fallback_skipped")],
        );
        return "refused";
    }
    match app
        .state::<ClipboardWriter>()
        .write_result(session_id, text.to_owned())
    {
        Ok(_) => {
            log_event(
                app,
                "hotkey_delivery",
                &[("result", "clipboard_fallback_committed")],
            );
            "copied"
        }
        Err(refusal) => {
            let reason = format!("{refusal:?}");
            log_event(
                app,
                "hotkey_delivery",
                &[
                    ("result", "clipboard_fallback_refused"),
                    ("reason", reason.as_str()),
                ],
            );
            "refused"
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecoverableResultView {
    state: String,
    text: Option<String>,
    provenance: Option<String>,
    input_samples: Option<usize>,
    final_segments: Option<usize>,
    draft_revisions: Option<usize>,
    error_code: Option<String>,
    retry_available: bool,
}

/// One authoritative final produced during this run of the app.
#[derive(Clone, Debug)]
struct SessionTranscriptEntry {
    /// Opaque, process-local, and unique per entry. Deliberately not the session
    /// id: retrying the retained audio produces a second final for the same
    /// session, and both belong in the log.
    id: String,
    session_id: SessionId,
    text: String,
    provenance: &'static str,
    recorded_unix_ms: i64,
}

/// Input level for the Audio page's meter. Display-only and non-mutating.
#[derive(Clone, Debug, Serialize)]
pub struct CaptureLevelView {
    level: f32,
    /// Whether a dictation is running. The level only moves when one is.
    active: bool,
    device_diagnostic: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionTranscriptEntryView {
    id: String,
    text: String,
    provenance: String,
    recorded_unix_ms: i64,
}

/// The most finals kept in the session log before the oldest is dropped.
///
/// A bound exists because the log lives for the whole app session and nothing
/// prunes it; transcripts are small, so this is generous rather than tight.
const SESSION_TRANSCRIPT_LIMIT: usize = 100;
