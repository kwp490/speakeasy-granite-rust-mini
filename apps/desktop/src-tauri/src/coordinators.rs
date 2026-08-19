/// This session's finals, for recall and copying.
///
/// Memory only. Nothing here is written to disk and the whole log dies with the
/// process — that is the entire privacy claim, so there is deliberately no
/// persistence seam for one to be added to later by accident. The separate,
/// off-by-default on-disk history feature is `HistoryCoordinator` and is
/// untouched by this.
#[derive(Debug, Default)]
pub struct SessionTranscriptCoordinator {
    entries: Mutex<Vec<SessionTranscriptEntry>>,
    next_id: AtomicU64,
}

impl SessionTranscriptCoordinator {
    /// Records one final. Best-effort: the log is a convenience, so a poisoned
    /// lock must never propagate into the dictation that produced the text.
    fn record(&self, session_id: SessionId, text: &str, provenance: &'static str) {
        if text.trim().is_empty() {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        entries.push(SessionTranscriptEntry {
            id: format!("transcript-{id}"),
            session_id,
            text: text.to_owned(),
            provenance,
            recorded_unix_ms: i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(i64::MAX),
        });
        if entries.len() > SESSION_TRANSCRIPT_LIMIT {
            entries.remove(0);
        }
    }

    /// Adopts persisted transcripts at launch, newest last, so a retained log
    /// is a log rather than a list of what happened since the app started.
    ///
    /// This is the whole of the retention feature on the read side, and it is
    /// deliberately the *only* place the two stores meet. `persisted_history_enabled`
    /// (off by default) already decides whether a delivered transcript is
    /// written to disk at all, so with retention off nothing was ever stored,
    /// this seeds nothing, and the log is empty at every launch.
    ///
    /// That is worth stating plainly, because it is a stronger guarantee than
    /// the setting's own wording suggests: "cleared when the app closes" is
    /// achieved by never writing, not by deleting on the way out. A
    /// delete-on-exit implementation would be a promise the process cannot
    /// keep -- a crash, a kill, or a power cut all skip it, and the transcripts
    /// the user was told were discarded would still be on disk. Nothing to
    /// delete is the only version of this that survives being killed.
    ///
    /// Entries seeded here carry no `SessionId` of their own: the id in a
    /// stored record is hex text from a previous process, and the live one is
    /// used only for delivery correlation within *this* run. Copy does not
    /// need it (see `copy_payload`'s caller), so a seeded entry gets a fresh
    /// one rather than a parsed-back impostor that could collide with a
    /// running dictation.
    fn seed_from_history(&self, stored: &[TranscriptResult]) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        for record in stored.iter().rev().take(SESSION_TRANSCRIPT_LIMIT) {
            let text = record
                .polished_text
                .clone()
                .unwrap_or_else(|| record.raw_text.clone());
            if text.trim().is_empty() {
                continue;
            }
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            entries.push(SessionTranscriptEntry {
                id: format!("transcript-{id}"),
                session_id: SessionId::from_bytes([0; 16]),
                text,
                // `Raw` and `Polished` are the storage layer's older shape and
                // nothing writes them today; they are mapped rather than
                // ignored so a database carried forward from an earlier
                // version still lists its transcripts instead of dropping
                // them at the match.
                provenance: match record.provenance {
                    ResultProvenance::LastValidDraft => "last_valid_draft",
                    ResultProvenance::FinalizedStream
                    | ResultProvenance::Raw
                    | ResultProvenance::Polished => "finalized_stream",
                },
                recorded_unix_ms: record.created_unix_ms,
            });
        }
    }

    /// This session's finals, newest first.
    fn log(&self) -> Result<Vec<SessionTranscriptEntryView>, &'static str> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "session_transcript_unavailable")?;
        Ok(entries
            .iter()
            .rev()
            .map(|entry| SessionTranscriptEntryView {
                id: entry.id.clone(),
                text: entry.text.clone(),
                provenance: entry.provenance.to_owned(),
                recorded_unix_ms: entry.recorded_unix_ms,
            })
            .collect())
    }

    fn copy_payload(&self, id: &str) -> Result<(SessionId, String), &'static str> {
        self.entries
            .lock()
            .map_err(|_| "session_transcript_unavailable")?
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.session_id, entry.text.clone()))
            .ok_or("session_transcript_entry_unavailable")
    }

    /// The newest recorded final, for the transcriber's own copy control.
    ///
    /// Deliberately not addressable by id, unlike `copy_payload`: "newest" is the
    /// whole of the transcriber's reach into this log, so there is no entry it can
    /// name and nothing to forge. Browsing the log stays main-only.
    fn copy_latest_payload(&self) -> Result<(SessionId, String), &'static str> {
        self.entries
            .lock()
            .map_err(|_| "session_transcript_unavailable")?
            .last()
            .map(|entry| (entry.session_id, entry.text.clone()))
            .ok_or("session_transcript_entry_unavailable")
    }
}

#[derive(Debug, Default)]
pub struct ResultCoordinator {
    result: Mutex<Option<FinalTranscript>>,
    error_code: Mutex<Option<String>>,
}

impl ResultCoordinator {
    /// Retains one non-empty final result in memory for recovery and copying.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when the result is empty or coordinator state
    /// is unavailable.
    pub fn accept(&self, transcript: FinalTranscript) -> Result<(), &'static str> {
        if transcript.text.trim().is_empty() {
            return Err("empty_result_rejected");
        }
        *self.result.lock().map_err(|_| "result_state_unavailable")? = Some(transcript);
        *self
            .error_code
            .lock()
            .map_err(|_| "result_state_unavailable")? = None;
        Ok(())
    }

    /// Records a sanitized recoverable failure without discarding a prior result.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when coordinator state is unavailable.
    pub fn fail(&self, code: &'static str) -> Result<(), &'static str> {
        *self
            .error_code
            .lock()
            .map_err(|_| "result_state_unavailable")? = Some(code.to_owned());
        Ok(())
    }

    /// Clears any prior result and error, for a dictation that completed
    /// without malfunctioning but produced no speech to show.
    ///
    /// Unlike `fail`, which deliberately keeps a prior result recoverable,
    /// this is not a failure: leaving an unrelated older transcript standing
    /// would misrepresent what this dictation actually produced, so the view
    /// falls back to its default `empty` state instead.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when coordinator state is unavailable.
    pub fn clear(&self) -> Result<(), &'static str> {
        *self.result.lock().map_err(|_| "result_state_unavailable")? = None;
        *self
            .error_code
            .lock()
            .map_err(|_| "result_state_unavailable")? = None;
        Ok(())
    }

    fn copy_payload(&self) -> Result<(SessionId, String), &'static str> {
        self.result
            .lock()
            .map_err(|_| "result_state_unavailable")?
            .as_ref()
            .map(|result| (result.session_id, result.text.clone()))
            .ok_or("result_unavailable")
    }

    fn view(&self) -> Result<RecoverableResultView, &'static str> {
        let result = self
            .result
            .lock()
            .map_err(|_| "result_state_unavailable")?
            .clone();
        let error_code = self
            .error_code
            .lock()
            .map_err(|_| "result_state_unavailable")?
            .clone();
        Ok(match result {
            Some(result) => RecoverableResultView {
                state: if error_code.is_some() {
                    "failed"
                } else {
                    "ready"
                }
                .to_owned(),
                text: Some(result.text),
                provenance: Some(
                    match result.provenance {
                        TranscriptProvenance::FinalizedStream => "finalized_stream",
                        TranscriptProvenance::LastValidDraft => "last_valid_draft",
                    }
                    .to_owned(),
                ),
                input_samples: Some(result.metrics.input_samples),
                final_segments: Some(result.metrics.final_segments),
                draft_revisions: Some(result.metrics.draft_revisions),
                error_code,
                retry_available: false,
            },
            None => RecoverableResultView {
                state: if error_code.is_some() {
                    "failed"
                } else {
                    "empty"
                }
                .to_owned(),
                text: None,
                provenance: None,
                input_samples: None,
                final_segments: None,
                draft_revisions: None,
                error_code,
                retry_available: false,
            },
        })
    }
}

/// Re-verifies a trusted installed pack and performs non-empty inference
/// through the concrete supervised process adapter before admitting Ready.
///
/// # Errors
///
/// Returns a recoverable domain error when reverification, runtime smoke,
/// inference, or the runtime transition fails.
pub async fn admit_streaming_runtime(
    manager: &InstallManager,
    spec: &InstallSpec,
    adapter: &WorkerFinalAdapter<ProcessWorkerClient<SystemClock>, Arc<SystemClock>>,
    audio: UtteranceAudio,
    request: AsrRequest,
    cancel: CancelToken,
    deadline: Deadline,
) -> Result<(RuntimeState, FinalTranscript), DomainError> {
    manager.reverify(spec).map_err(|_| DomainError {
        code: ErrorCode::InvalidData,
        recoverable: true,
    })?;
    let smoke = RuntimeState::VerifiedOnDisk
        .transition(RuntimeState::RuntimeSmokeTesting)
        .map_err(|_| DomainError {
            code: ErrorCode::InvalidTransition,
            recoverable: true,
        })?;
    let capabilities = adapter.capabilities();
    let transcript = adapter.transcribe(audio, request, cancel, deadline).await?;
    admissible_delivered_transcript(&transcript)?;
    let ready = smoke
        .transition(RuntimeState::Ready(RuntimeEvidence {
            artifact_id: spec.id.clone(),
            runtime_abi: capabilities.runtime_abi.to_owned(),
            provider: capabilities.provider.to_owned(),
            inference_sample_count: transcript.metrics.input_samples,
        }))
        .map_err(|_| DomainError {
            code: ErrorCode::InvalidTransition,
            recoverable: true,
        })?;
    Ok((ready, transcript))
}

/// The two checks a delivered transcript has to clear, on its way through
/// [`admit_streaming_runtime`] above.
///
/// It had a second caller, `resident_retained_pass`, until the streaming engine
/// left and the two paths became one. Kept as its own function because the
/// split the two checks draw — a plumbing bug versus a silent utterance — is
/// deliberate and easy to collapse by accident. `input_samples == 0` is a real
/// plumbing bug — the pass was handed no audio at all — and stays
/// [`ErrorCode::AdapterFailed`], which does count toward worker quarantine.
/// Empty *text* is a silent utterance and must be
/// [`ErrorCode::NoSpeechDetected`], which does not: three short silences inside
/// one minute used to quarantine all delivery.
fn admissible_delivered_transcript(transcript: &FinalTranscript) -> Result<(), DomainError> {
    if transcript.metrics.input_samples == 0 {
        return Err(DomainError {
            code: ErrorCode::AdapterFailed,
            recoverable: true,
        });
    }
    if transcript.text.is_empty() {
        return Err(DomainError {
            code: ErrorCode::NoSpeechDetected,
            recoverable: true,
        });
    }
    Ok(())
}

impl Default for CaptureHudCoordinator {
    fn default() -> Self {
        Self {
            live: Mutex::new(HudLiveState::default()),
            published: Mutex::new(CaptureHudView {
                schema_version: DOMAIN_SCHEMA_VERSION,
                sequence: 0,
                session_id: String::new(),
                session: "idle".to_owned(),
                vad: "manual_stop_only".to_owned(),
                level: 0.0,
                device_diagnostic: "not_opened".to_owned(),
                streaming_mode: "final_only".to_owned(),
                mutable_text: String::new(),
                stable_display_text: String::new(),
                final_text: String::new(),
                device_name: String::new(),
                hotkey_binding: String::new(),
                hotkey_registration: "pending".to_owned(),
                can_start: false,
                can_stop: false,
                setup_complete: false,
                setup_reason: None,
                elapsed_ms: 0,
                ceiling_ms: 0,
                preferred_device_id: String::new(),
                delivery_outcome: "held".to_owned(),
                // The streaming coordinator's own default before it, kept. This
                // baseline exists only to be compared against the first real
                // composition, so claiming `ready` here would publish a warm
                // that has not happened.
                engine: "cold".to_owned(),
                queue_depth: 0,
                error_code: None,
                final_source_reason: None,
            }),
        }
    }
}

impl CaptureHudCoordinator {
    /// Returns the bounded session identity and final-source disclosure used
    /// by diagnostics. The transcript itself is intentionally not exposed.
    fn diagnostics(&self) -> Result<(Option<SessionId>, Option<String>), &'static str> {
        let live = self.live.lock().map_err(|_| "capture_status_unavailable")?;
        Ok((live.session_id, live.final_source_reason.map(str::to_owned)))
    }

    /// Marks a dictation as beginning, clearing anything the previous one left.
    ///
    /// From here until `finish`, only this session may write live text: a tap
    /// belonging to a superseded session is ignored rather than allowed to
    /// paint stale words over the current one.
    fn begin(&self, session_id: SessionId) {
        if let Ok(mut live) = self.live.lock() {
            *live = HudLiveState {
                session_id: Some(session_id),
                ..HudLiveState::default()
            };
        }
    }

    /// Drops the live session whole, for a dictation the user abandoned.
    ///
    /// Distinct from `end_live`, which keeps the session and waits for an
    /// authoritative final to replace the hypotheses. A cancelled dictation has
    /// no final coming, so leaving the last hypothesis on screen would leave
    /// display-only live text standing as the result.
    fn abandon(&self) {
        if let Ok(mut live) = self.live.lock() {
            *live = HudLiveState::default();
        }
    }

    /// Shows the authoritative final — the same text delivery uses — together
    /// with what actually happened to it.
    fn finish(&self, text: &str, outcome: &'static str, source_reason: Option<&'static str>) {
        if let Ok(mut live) = self.live.lock() {
            live.stable_display_text.clear();
            live.mutable_text.clear();
            text.clone_into(&mut live.final_text);
            live.delivery_outcome = Some(outcome);
            live.final_source_reason = source_reason;
        }
    }

    /// Composes the full view and advances `sequence` on any observable change.
    ///
    /// Deriving the composed half here rather than pushing it keeps the HUD to
    /// one poll while still honouring the stale-response guard: `sequence`
    /// moves whenever anything the frontend can see moves, including when only
    /// the composed fields changed.
    fn view(&self, composed: HudComposition) -> Result<CaptureHudView, &'static str> {
        let live = self
            .live
            .lock()
            .map_err(|_| "capture_status_unavailable")?
            .clone();
        let mut published = self
            .published
            .lock()
            .map_err(|_| "capture_status_unavailable")?;
        let next = CaptureHudView {
            schema_version: DOMAIN_SCHEMA_VERSION,
            sequence: published.sequence,
            session_id: live
                .session_id
                .map(|id| format!("{id:?}"))
                .unwrap_or_default(),
            session: composed.session.to_owned(),
            vad: "manual_stop_only".to_owned(),
            level: composed.level,
            device_diagnostic: composed.device_diagnostic,
            streaming_mode: live.streaming_mode.unwrap_or("final_only").to_owned(),
            mutable_text: live.mutable_text,
            stable_display_text: live.stable_display_text,
            final_text: live.final_text,
            device_name: composed.device_name,
            hotkey_binding: composed.hotkey_binding,
            hotkey_registration: composed.hotkey_registration,
            can_start: composed.can_start,
            can_stop: composed.can_stop,
            setup_complete: composed.setup_complete,
            setup_reason: composed.setup_reason,
            elapsed_ms: composed.elapsed_ms,
            ceiling_ms: composed.ceiling_ms,
            preferred_device_id: composed.preferred_device_id,
            delivery_outcome: live.delivery_outcome.unwrap_or("held").to_owned(),
            engine: composed.engine.to_owned(),
            queue_depth: composed.queue_depth,
            error_code: composed.error_code,
            final_source_reason: live.final_source_reason.map(str::to_owned),
        };
        if next != *published {
            published.clone_from(&next);
            published.sequence = published.sequence.saturating_add(1);
        }
        Ok(published.clone())
    }
}

/// The half of the HUD view derived from the other coordinators at read time.
struct HudComposition {
    session: &'static str,
    level: f32,
    device_diagnostic: String,
    device_name: String,
    hotkey_binding: String,
    hotkey_registration: String,
    can_start: bool,
    can_stop: bool,
    setup_complete: bool,
    setup_reason: Option<String>,
    elapsed_ms: u64,
    ceiling_ms: u64,
    preferred_device_id: String,
    engine: &'static str,
    queue_depth: usize,
    error_code: Option<String>,
}

/// Translates the capture wizard's vocabulary into the session states
/// UI-GUIDE's truthful-disclosure rule distinguishes for the user.
///
/// Doing this here rather than in the tap is what makes `Listening` and
/// `Transcribing…` correct when streaming is unavailable: the capture pipeline
/// runs either way, and only the live text depends on the engine.
fn hud_session_of(capture_state: &str) -> &'static str {
    match capture_state {
        "arming" => "starting",
        "capturing" => "streaming",
        "draining" | "captured" => "stopping",
        "finalizing" => "finalizing",
        "complete" => "complete",
        "failed" | "unavailable" => "failed",
        _ => "idle",
    }
}

/// Starts the resident Granite worker in the background at app launch.
///
/// Granite runs on every dictation, so the ~2 GB model load is paid once here
/// rather than on the first hotkey press. This used to be one of two warms
/// and its doc pointed at the other; it is the only one now.
///
/// Off the UI thread and entirely best-effort: a machine without Granite
/// installed, or with a quarantined engine, pays nothing beyond this
/// function returning quickly — see
/// [`granite_engine::warm_granite_if_configured`]'s own doc for exactly what
/// "not configured" means here.
fn warm_granite_engine(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let coordinator = app.state::<GraniteEngineCoordinator>();
        let granite_worker_exe = app
            .state::<RuntimeWizardCoordinator>()
            .paths()
            .ok()
            .map(|paths| paths.granite_worker);
        let models = app.state::<ModelCoordinator>();
        let outcome = warm_granite_if_configured(
            GraniteEnvironment {
                granite_worker_exe: granite_worker_exe.as_deref(),
                install_root: &models.root.join("models"),
                total_memory_bytes: SafeStandardHardwareProbe
                    .probe(&models.root)
                    .total_memory_bytes,
                diagnostic_log: diagnostic_log_path(&app),
            },
            &coordinator,
        );
        let result = match &outcome {
            Ok(()) => "ok",
            Err(error) => domain_error_code(error),
        };
        // Readiness verifies the pack a dictation would actually load, and
        // which pack that is depends on whether this worker turned out to be
        // CUDA-capable — a fact only the worker can report, and only after it
        // has spoken. Startup had to guess conservatively; this is the first
        // moment the guess can be corrected.
        //
        // Its previous caller was the on-demand CUDA runtime install, which
        // left with ONNX Runtime. Dropping the call rather than re-pointing it
        // would have left readiness permanently describing the startup guess.
        models.refresh_readiness(coordinator.cuda_worker_available());
        // `engine` carries which Granite pack this machine resolved and why,
        // mirroring `streaming_warm`'s own field. A stable code and nothing
        // else -- a fallback to CPU must be findable in a support log, and
        // "running on CPU" alone cannot say whether that was the preference or
        // the consolation prize.
        //
        // `device` is a different fact and used to be missing, which made
        // `engine` the only thing here that looked like an answer to "what did
        // it run on" -- and it is not one. There is a single Granite pack, so
        // the pack reason reads `cpu_...` even when a CUDA-compiled worker is
        // running the same GGUF on the GPU, and a support log said exactly
        // that while `nvidia-smi` showed the worker holding a CUDA context.
        // Only the worker can answer this, so it is asked at `Hello`.
        log_event(
            &app,
            "granite_warm",
            &[
                ("result", result),
                ("engine", coordinator.engine_reason()),
                ("device", coordinator.device()),
            ],
        );
    });
}

/// The sanitized diagnostic log path, when the user has enabled disk logging.
fn diagnostic_log_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let profile = app.state::<ProfileCoordinator>();
    profile
        .settings
        .lock()
        .is_ok_and(|settings| settings.privacy.disk_logging_enabled)
        .then(|| profile.root.join("logs").join("speakeasy.log"))
}

/// The transfer `model_install_status` should report progress against.
///
/// Recorded when a download starts rather than re-derived when progress is
/// polled. Progress used to be computed by asking the GPU probe which pack this
/// machine prefers, which is a different question from which pack is being
/// downloaded — on a CUDA-capable machine installing the CPU pack it sized a
/// 475 MB transfer against the 2.2 GB CUDA pack and watched for a `.part` file
/// that would never appear, so the bar sat at zero for the whole install.
#[derive(Clone, Debug)]
struct ActiveDownload {
    part_path: PathBuf,
    completed_bytes: u64,
    total_bytes: u64,
}

#[derive(Debug)]
enum ModelInstallPayload {
    Archive(DownloadRequest),
    Loose(Vec<(PathBuf, DownloadRequest)>),
}

impl ModelInstallPayload {
    fn requests(&self) -> Vec<&DownloadRequest> {
        match self {
            Self::Archive(request) => vec![request],
            Self::Loose(files) => files.iter().map(|(_, request)| request).collect(),
        }
    }

    fn total_bytes(&self) -> Option<u64> {
        self.requests().iter().try_fold(0_u64, |total, request| {
            total.checked_add(request.expected_bytes)
        })
    }
}

#[derive(Debug)]
pub struct ModelCoordinator {
    root: PathBuf,
    status: Arc<Mutex<ModelInstallView>>,
    cancel: Arc<Mutex<Option<CancelToken>>>,
    active_download: Arc<Mutex<Option<ActiveDownload>>>,
}

/// Holds execution evidence for the GPU currently in use.
///
/// The probe is intentionally re-read for every status request, so the
/// evidence cannot be baked into a launch-time snapshot. A changed adapter
/// invalidates the old proof; a changed free-VRAM reading does not, because it
/// is an ambient resource rather than device identity.
#[derive(Debug, Default)]
pub struct GpuQualificationCoordinator {
    decision: Mutex<Option<GpuQualification>>,
}

impl GpuQualificationCoordinator {
    fn current(&self, snapshot: &speakeasy_models::GpuSnapshot) -> GpuQualification {
        let observed = speakeasy_models::admit(snapshot);
        let Some(device) = observed.device() else {
            return observed;
        };
        let Ok(decision) = self.decision.lock() else {
            return observed;
        };
        match decision.as_ref() {
            Some(GpuQualification::Qualified {
                device: proven,
                evidence,
            }) if same_gpu_identity(proven, device) => GpuQualification::Qualified {
                device: device.clone(),
                evidence: evidence.clone(),
            },
            _ => observed,
        }
    }

    // `record` lived here: it promoted the GPU from "admissible" to
    // "qualified" once a model had genuinely executed on it, and the streaming
    // engine's warm-time smoke test was what called it. Granite has no
    // equivalent smoke yet because it has no GPU path to smoke -- its CUDA
    // support is a build feature and no CUDA worker is published -- so
    // qualification would have had no way to ever become true. Rather than
    // keep a promotion nothing can trigger, the coordinator now only reports
    // what the probe sees, and `gpu_status` says "admissible, not qualified"
    // honestly. This comes back with the CUDA worker, not before.
}

fn same_gpu_identity(
    left: &speakeasy_models::CudaDevice,
    right: &speakeasy_models::CudaDevice,
) -> bool {
    left.name == right.name
        && left.compute_capability == right.compute_capability
        && left.total_vram_bytes == right.total_vram_bytes
}

#[derive(Debug)]
pub struct ProfileCoordinator {
    root: PathBuf,
    store: SettingsStore,
    settings: Mutex<Settings>,
    load_error: Mutex<Option<&'static str>>,
    reset_nonce: Mutex<Option<String>>,
}

#[derive(Debug)]
pub struct PersonalizationCoordinator {
    repository: Mutex<PersonalizationRepository>,
    export_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct PersonalizationView {
    schema_version: u16,
    transform_pipeline_version: u16,
    locale_status: String,
    hotword_path: String,
    contacts_import_enabled: bool,
    dictionary: Vec<DictionaryEntry>,
    snippets: Vec<Snippet>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PersonalizationImportPreviewView {
    fingerprint_sha256: String,
    dictionary_count: usize,
    snippet_count: usize,
    conflicts: usize,
    contacts_imported: bool,
}

impl PersonalizationCoordinator {
    fn new(root: &std::path::Path) -> Result<Self, &'static str> {
        let repository = PersonalizationRepository::open(root.join("config/personalization.json"))
            .map_err(|_| "personalization_recovery_required")?;
        Ok(Self {
            repository: Mutex::new(repository),
            export_root: root.join("exports"),
        })
    }

    fn view(&self) -> Result<PersonalizationView, &'static str> {
        let repository = self
            .repository
            .lock()
            .map_err(|_| "personalization_state_unavailable")?;
        Ok(PersonalizationView {
            schema_version: speakeasy_transforms::PERSONALIZATION_SCHEMA_VERSION,
            transform_pipeline_version: speakeasy_transforms::TRANSFORM_PIPELINE_VERSION,
            locale_status: "en_us_limited_other_locales_identity".to_owned(),
            hotword_path: "final_postprocess_only_manifest_no_hotwords".to_owned(),
            contacts_import_enabled: false,
            dictionary: repository.state().dictionary.clone(),
            snippets: repository.state().snippets.clone(),
        })
    }
}

impl ProfileCoordinator {
    fn new(root: PathBuf) -> Self {
        let store = SettingsStore::new(root.join("config/settings.json"));
        let (settings, load_error) = match store.load() {
            Ok((settings, _)) => (settings, None),
            Err(_) => (Settings::default(), Some("profile_recovery_required")),
        };
        Self {
            root,
            store,
            settings: Mutex::new(settings),
            load_error: Mutex::new(load_error),
            reset_nonce: Mutex::new(None),
        }
    }

    fn view(&self) -> Result<ProfileView, &'static str> {
        if let Some(error) = *self
            .load_error
            .lock()
            .map_err(|_| "profile_state_unavailable")?
        {
            return Err(error);
        }
        let settings = self
            .settings
            .lock()
            .map_err(|_| "profile_state_unavailable")?;
        Ok(ProfileView {
            schema_version: DOMAIN_SCHEMA_VERSION,
            onboarding: settings.onboarding.clone(),
            startup_with_windows: settings.startup_with_windows,
            history_enabled: settings.privacy.persisted_history_enabled,
            history_retention_days: settings.privacy.history_retention_days,
            history_plaintext_disclosure_accepted: settings
                .privacy
                .history_plaintext_disclosure_accepted,
            delivery_preference: settings.delivery.safe_preference,
            recording_feedback_enabled: settings.delivery.feedback_enabled,
            disk_logging_enabled: settings.privacy.disk_logging_enabled,
            preferred_capture_device_id: settings.preferred_capture_device_id.clone(),
        })
    }

    fn save(&self, settings: &Settings) -> Result<(), &'static str> {
        if let Some(error) = *self
            .load_error
            .lock()
            .map_err(|_| "profile_state_unavailable")?
        {
            return Err(error);
        }
        self.store.save(settings).map_err(|_| "profile_save_failed")
    }
}

const DIAGNOSTICS_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DIAGNOSTICS_EVENT_CAPACITY: usize = 256;
const DIAGNOSTIC_EVENT_MAX_BYTES: usize = 512;

fn bounded_diagnostic_text(text: &str) -> String {
    if text.len() <= DIAGNOSTIC_EVENT_MAX_BYTES {
        return text.to_owned();
    }
    let limit = DIAGNOSTIC_EVENT_MAX_BYTES.saturating_sub(3);
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    format!("{}...", &text[..end])
}

/// Appends a sanitized diagnostic line describing an internal decision or
/// refusal, gated behind the user's disk-logging opt-in. Callers must only
/// pass structured field values (error codes, booleans, counts, session
/// ids) here, never raw transcript text or other captured content — this
/// respects the same "logs are sanitized" promise shown in diagnostics.
fn log_event(app: &tauri::AppHandle, event: &str, fields: &[(&str, &str)]) {
    let session_id = app
        .state::<CaptureHudCoordinator>()
        .diagnostics()
        .ok()
        .and_then(|(session_id, _)| session_id);
    log_event_with_session(app, session_id, event, fields);
}

fn log_event_for_session(
    app: &tauri::AppHandle,
    session_id: SessionId,
    event: &str,
    fields: &[(&str, &str)],
) {
    log_event_with_session(app, Some(session_id), event, fields);
}

fn log_event_with_session(
    app: &tauri::AppHandle,
    session_id: Option<SessionId>,
    event: &str,
    fields: &[(&str, &str)],
) {
    let profile = app.state::<ProfileCoordinator>();
    let enabled = profile
        .settings
        .lock()
        .is_ok_and(|settings| settings.privacy.disk_logging_enabled);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let mut line = format!("{millis}");
    if let Some(session_id) = session_id {
        let bytes = session_id.into_bytes();
        let _ = write!(
            line,
            " session={:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
    }
    let _ = write!(line, " event={event}");
    for (key, value) in fields {
        let _ = write!(line, " {key}={value}");
    }
    line.push('\n');
    if let Some(diagnostics) = app.try_state::<DiagnosticsRuntimeCoordinator>() {
        diagnostics.record_event(&line);
    }
    // Disk logging remains opt-in. The in-memory buffer above is deliberately
    // independent so the first occurrence is available for a later export.
    if enabled {
        let _ = append_diagnostics_log(&profile.root, &line);
    }
}

fn append_diagnostics_log(root: &Path, line: &str) -> std::io::Result<()> {
    let dir = root.join("logs");
    fs::create_dir_all(&dir)?;
    let path = dir.join("speakeasy.log");
    append_diagnostics_line(&path, line)
}

/// The single file-writing boundary for diagnostics, including worker stderr.
/// Redaction happens here as a defense in depth even when a caller has already
/// reduced its values to structured fields. This keeps future writers from
/// quietly bypassing the privacy promise.
pub(crate) fn append_diagnostics_line(path: &Path, line: &str) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "diagnostic path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() > DIAGNOSTICS_LOG_MAX_BYTES) {
        let rotated = path.with_extension("log.1");
        // Windows cannot rename over an existing destination. Only the prior
        // generation is removed; the active log is preserved until rename.
        let _ = fs::remove_file(&rotated);
        fs::rename(path, rotated)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let sanitized = redact_diagnostic_text(line);
    file.write_all(sanitized.as_bytes())
}

/// Redacts path-shaped substrings from native panic and loader messages before
/// they can reach the persistent diagnostic surface. Transcript text is not
/// sent through this path, but native error strings are not trusted to obey
/// that boundary themselves.
fn redact_diagnostic_text(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut token_start = 0;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            if token_start < index {
                output.push_str(&redact_diagnostic_token(&line[token_start..index]));
            }
            output.push(character);
            token_start = index + character.len_utf8();
        }
    }
    if token_start < line.len() {
        output.push_str(&redact_diagnostic_token(&line[token_start..]));
    }
    output
}

fn redact_diagnostic_token(token: &str) -> String {
    let bytes = token.as_bytes();
    let mut path_start = None;
    for index in 0..bytes.len() {
        let windows_drive = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/');
        let unc = index + 1 < bytes.len() && bytes[index] == b'\\' && bytes[index + 1] == b'\\';
        let unix = bytes[index] == b'/'
            && (index == 0 || matches!(bytes[index - 1], b'=' | b'(' | b'[' | b'"' | b'\''));
        if windows_drive || unc || unix {
            path_start = Some(index);
            break;
        }
    }
    let Some(path_start) = path_start else {
        return token.to_owned();
    };
    let prefix = &token[..path_start];
    format!("{prefix}<redacted-path>")
}

#[derive(Debug)]
pub struct ImportCoordinator {
    destination: PathBuf,
    plan: Mutex<Option<ProductionImportPlan>>,
}

pub struct HistoryCoordinator {
    database_path: PathBuf,
    export_root: PathBuf,
    repository: Mutex<Option<HistoryRepository>>,
    initialization_error: Mutex<Option<&'static str>>,
    session: Mutex<SessionResultList>,
}

impl HistoryCoordinator {
    /// The newest `limit` persisted transcripts, oldest first.
    ///
    /// Best-effort by design: this feeds the session log's launch seed, and a
    /// history database that will not open must not stop the app from starting
    /// or from dictating. An unreadable history reads here as an empty one --
    /// the initialization error it already recorded is what reports the fault.
    fn stored(&self, limit: usize) -> Vec<TranscriptResult> {
        self.repository
            .lock()
            .ok()
            .and_then(|repository| {
                repository
                    .as_ref()
                    .and_then(|repository| repository.list(limit).ok())
            })
            .unwrap_or_default()
    }

    fn new(root: &std::path::Path, settings: &Settings) -> Self {
        let database_path = root.join("data/speakeasy.sqlite3");
        let policy = HistoryPolicy {
            enabled: settings.privacy.persisted_history_enabled,
            retention_days: settings.privacy.history_retention_days,
            plaintext_disclosure_accepted: settings.privacy.history_plaintext_disclosure_accepted,
        };
        let (repository, initialization_error) =
            match HistoryRepository::open(&database_path, policy) {
                Ok(mut repository) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .try_into()
                        .unwrap_or(i64::MAX);
                    if repository.apply_retention(now).is_err() {
                        (None, Some("history_recovery_required"))
                    } else {
                        (Some(repository), None)
                    }
                }
                Err(_) => (None, Some("history_recovery_required")),
            };
        Self {
            database_path,
            export_root: root.join("exports"),
            repository: Mutex::new(repository),
            initialization_error: Mutex::new(initialization_error),
            session: Mutex::new(SessionResultList::default()),
        }
    }

    fn record(&self, result: &TranscriptResult) -> Result<(), &'static str> {
        self.session
            .lock()
            .map_err(|_| "history_state_unavailable")?
            .push(result.clone())
            .map_err(|_| "history_result_invalid")?;
        if let Some(repository) = self
            .repository
            .lock()
            .map_err(|_| "history_state_unavailable")?
            .as_mut()
        {
            repository
                .record(result)
                .map_err(|_| "history_write_failed")?;
        }
        Ok(())
    }
}

impl ImportCoordinator {
    fn new(root: &std::path::Path) -> Self {
        Self {
            destination: root.join("migration"),
            plan: Mutex::new(None),
        }
    }
}

#[derive(Debug, Default)]
pub struct OperationCoordinator {
    arbiter: Arc<Mutex<OperationArbiter>>,
    dictation: Mutex<Option<ExclusiveOperation>>,
}

impl OperationCoordinator {
    #[cfg(test)]
    fn begin_dictation(&self, session_id: SessionId) -> Result<(), &'static str> {
        let operation = ExclusiveOperation::Dictation(session_id);
        let mut slot = self
            .dictation
            .lock()
            .map_err(|_| "operation_state_unavailable")?;
        let mut arbiter = self
            .arbiter
            .lock()
            .map_err(|_| "operation_state_unavailable")?;
        if !matches!(
            arbiter.begin(operation),
            OperationDisposition::Started | OperationDisposition::AlreadyOwned
        ) {
            return Err("dictation_operation_conflict");
        }
        *slot = Some(operation);
        Ok(())
    }

    fn replace_completed_dictation(&self, session_id: SessionId) -> Result<(), &'static str> {
        let operation = ExclusiveOperation::Dictation(session_id);
        let mut slot = self
            .dictation
            .lock()
            .map_err(|_| "operation_state_unavailable")?;
        let mut arbiter = self
            .arbiter
            .lock()
            .map_err(|_| "operation_state_unavailable")?;
        if let Some(previous) = slot.take() {
            let _ = arbiter.finish(previous);
        }
        if !matches!(
            arbiter.begin(operation),
            OperationDisposition::Started | OperationDisposition::AlreadyOwned
        ) {
            return Err("dictation_operation_conflict");
        }
        *slot = Some(operation);
        Ok(())
    }

    fn finish_dictation(&self) {
        if let Ok(mut slot) = self.dictation.lock()
            && let Some(operation) = slot.take()
            && let Ok(mut arbiter) = self.arbiter.lock()
        {
            let _ = arbiter.finish(operation);
        }
    }

    fn begin(&self, operation: ExclusiveOperation) -> Result<(), &'static str> {
        let mut arbiter = self
            .arbiter
            .lock()
            .map_err(|_| "operation_state_unavailable")?;
        matches!(
            arbiter.begin(operation),
            OperationDisposition::Started | OperationDisposition::AlreadyOwned
        )
        .then_some(())
        .ok_or("dictation_active_operation_deferred")
    }
}
