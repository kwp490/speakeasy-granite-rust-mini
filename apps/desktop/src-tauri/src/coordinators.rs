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
                // Same reasoning as `engine` above: this baseline exists only
                // to be diffed against the first real composition, and naming a
                // device here would claim a worker had reported one.
                engine_device: "not_configured".to_owned(),
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
    /// Distinct from ending a dictation normally, which keeps the session and
    /// waits for the authoritative final. It had a sibling, `end_live`, that
    /// drew that distinction while the streaming engine put hypotheses on
    /// screen: a cancelled dictation has no final coming, so leaving the last
    /// hypothesis up would have left display-only live text standing as the
    /// result. There are no hypotheses now, and dropping the session whole is
    /// still the right answer for a dictation nobody wants a result from.
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
            engine_device: composed.engine_device.to_owned(),
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
    /// The compute device Granite runs on, never the microphone. See
    /// `CaptureHudView::engine_device`.
    engine_device: &'static str,
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

/// The session state the user is shown, with delivery folded in.
///
/// Split out of `capture_hud_status` so the dock and the shortcut cannot
/// disagree about when a dictation is over. They did: `can_start` has always
/// refused while a dictation was still finishing, and the global shortcut had no
/// such rule, so the same press the dock declined started a second recording.
///
/// The promotion is the load-bearing half. Transcription being finished is not
/// the text having arrived somewhere, so `complete` with delivery unresolved is
/// reported as `finalizing` -- a completion the user cannot act on is not a
/// completion.
fn hud_session_with_delivery(capture_state: &str, delivery_pending: bool) -> &'static str {
    let session = hud_session_of(capture_state);
    if session == "complete" && delivery_pending {
        "finalizing"
    } else {
        session
    }
}

/// Whether a dictation has stopped recording but not yet finished with the
/// transcript.
///
/// The window this covers is long and the user cannot see into it: measured
/// 2026-08-25 on an installed release build, inference alone is 4.2 s on the
/// card and 44.5 s on the processor. Nothing is on screen saying "still
/// working" except the dock, which the user has usually looked away from.
///
/// So the press that arrives here is the ordinary second press of a toggle --
/// especially after a ceiling stop, where the recording ended without being
/// asked to and the user was by definition still talking. Observed 490 ms after
/// a ceiling fired at 120,183 ms: it opened a second dictation, which queued
/// behind the first, waited 36.6 s, and pasted its own transcript wherever the
/// user had got to. Nothing errored; both transcripts were delivered.
///
/// Owner decision 2026-08-26: one at a time. A press in this window is refused
/// rather than queued, because the user pressing it is ending a recording that
/// has already ended, not asking for another one.
///
/// `false` when either coordinator is absent or its lock is poisoned. That is
/// the fail-open direction on purpose: this guard exists to suppress an
/// *unwanted* dictation, and a broken read must never be able to suppress a
/// wanted one.
fn dictation_is_finishing(app: &tauri::AppHandle) -> bool {
    let (Some(capture), Some(hud)) = (
        app.try_state::<CaptureWizardCoordinator>(),
        app.try_state::<CaptureHudCoordinator>(),
    ) else {
        return false;
    };
    let Ok(view) = capture.view() else {
        return false;
    };
    let delivery_pending = hud
        .live
        .lock()
        .is_ok_and(|live| live.delivery_outcome.is_none());
    matches!(
        hud_session_with_delivery(view.state.as_str(), delivery_pending),
        "stopping" | "finalizing"
    )
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
        // The profile directory, which is also where setup left its record of
        // which configuration it installed. Cloned rather than held, so the
        // state borrow ends before the warm does.
        let profile_root = app.state::<ProfileCoordinator>().root.clone();
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
                // What setup *proved*, read from disk. The warm compares it
                // against what the worker turns out to be, which is the only
                // place the two are ever seen together -- and they disagreeing
                // silently is the defect this field exists to make impossible.
                recorded_provider: installed_configuration(&profile_root),
                // The real probe, named at the composition root rather than
                // inside the warm. This is the *only* place it is named, which
                // is what makes a staged one possible without a switch in the
                // shipped binary.
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
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
        // the way the deleted `streaming_warm` event carried the streaming
        // engine's. A stable code and nothing else -- a fallback to CPU must be
        // findable in a support log, and
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
                // What setup installed, which is the only thing here that is
                // not a fact about this run. Without it `device=cpu` is
                // ambiguous in the one way that matters: on a processor install
                // it is the expected outcome, and on a graphics-card install it
                // is a fault. The app cannot re-derive which was chosen, so
                // setup writes it down and this reads it back.
                ("installed", installed_configuration(&profile_root)),
                // The comparison of the two, made once rather than left for a
                // reader to make. Three correct fields whose combination is
                // impossible is what this log carried on 2026-08-20 --
                // `engine=cpu_gpu_runtime_missing device=cpu installed=cuda` --
                // and nothing anywhere looked at them together, so nothing
                // reported it. `ok` and `unrecorded` are the quiet answers.
                ("provider", coordinator.provider_integrity().code()),
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

    /// Adds words the user asked to have protected, as dictionary entries.
    ///
    /// The entry shape is `extract_v1_protected_terms`': source and replacement
    /// are the same word, matched case-insensitively on word boundaries, with
    /// `protected` set. That is what "protect this word" means here — it does
    /// not teach the recogniser anything, it stops the finishing pass from
    /// rewriting a word it got right.
    ///
    /// `DictionaryOrigin::UserEntry` rather than `ImportedProfile`, which is
    /// the origin the v1 import path uses: nobody imported these, someone typed
    /// them into setup. The distinction is visible in the settings list and in
    /// the precedence order, so getting it wrong would be a small lie in two
    /// places — and `replace_user_entry_terms` now uses it as the marker for
    /// which entries this page owns.
    ///
    /// Replaces rather than merges, since 2026-08-20. `add_imported_terms` keyed
    /// its de-duplication on the entry id, and these ids are positional
    /// (`installer-0`, `installer-1`, …), so a second install left stale entries
    /// behind — and a stale entry holding a word the new list also held was a
    /// `ConflictingRule` that rejected every word the user typed, silently. An
    /// ordinary uninstall keeps `personalization.json` on purpose, so a second
    /// install is the common case rather than an odd one.
    ///
    /// # The spaced companion, since 2026-08-27
    ///
    /// A compound term also gets a second entry keyed on its spaced form —
    /// `Logic Monitor` -> `LogicMonitor` — because that is how the recogniser
    /// actually writes a compound name and the identity rule cannot match it.
    /// See `speakeasy_transforms::spaced_variant` for the measurement.
    ///
    /// **They are ordinary visible entries on purpose.** Making the matcher
    /// quietly accept a spaced form would fix the same transcripts and leave no
    /// way to find out why "I need service now" had become "I need
    /// `ServiceNow`".
    /// An entry the user can read in the settings list and delete is the
    /// inspectable version of the same behaviour, and this app's conventions
    /// prefer that to magic even when it costs list length.
    fn add_protected_terms(&self, terms: &[String]) -> Result<(), &'static str> {
        self.repository
            .lock()
            .map_err(|_| "personalization_state_unavailable")?
            .replace_user_entry_terms(protected_term_entries(terms))
            .map_err(|_| "personalization_terms_rejected")
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


/// Builds the dictionary entries for a list of protected terms.
///
/// A free function rather than a method so the entry shape can be asserted
/// without a repository on disk — the collision guard below is the part that has
/// already caused a silent, total loss of a user's vocabulary once.
/// What the recogniser writes instead of a term, where somebody has measured it.
///
/// `(heard, meant)`. A term gains an entry from this table only when the *meant*
/// side is a word the user actually protects, so a profile without `HUIT` never
/// rewrites anybody's `Hewitt`.
///
/// # Why this is a hand-written table and has to stay small
///
/// These are the failures no rule predicts. A spacing rule reaches a compound
/// the recogniser split; nothing reaches a *phonetic* substitution, because the
/// wrong answer is not a transformation of the right one — `Hewitt` is what the
/// model hears when a Harvard speaker says `HUIT`, and no amount of edit
/// distance or fuzzy matching derives one from the other. So each row is a
/// measurement, and a row nobody measured is a guess that silently rewrites
/// correct transcripts.
///
/// Both rows below were measured on 2026-08-27 against a 55 s recording of real
/// speech: `Hewitt` came back in **every one of nine passes**, biased and
/// unbiased alike, and `Helen` in every unbiased pass.
///
/// # Each row costs something, and `Helen` costs the most
///
/// A correction is unconditional, so it fires on the ordinary word too: with
/// this row live, a user dictating about a person called Helen gets `Hellen`.
/// That is not a defect to be fixed later, it is the trade — the audio cannot
/// distinguish the two, so the only alternative is that `Hellen` is never right.
/// Shipped on the owner's decision of 2026-08-27, for an audience where the
/// name in the vocabulary is the one that gets said. The entries are visible in
/// Settings and deletable, which is the whole reason they are entries rather
/// than a rule buried in the transform.
/// # What is deliberately absent, and why the list stops here
///
/// Five dictations of one sentence on 2026-08-27 produced **six failures and
/// six distinct wrong forms**, not one of which this table had predicted: no
/// run produced `Hewitt` or `Helen` at all. What they produced instead was
/// `Project monitor` for `LogicMonitor`, and `Ellen` and `Haley` for `Hellen`.
///
/// None of those three is here, and each is refused for its own reason.
/// `Project monitor` is a semantic substitution — the recogniser heard a
/// different, ordinary phrase, and a rule rewriting it would fire on anyone
/// dictating about a project monitor. `Ellen` and `Haley` are common given
/// names; correcting them would corrupt every real Ellen and Haley in every
/// transcript, for every user, which is a worse trade than the `Helen` row
/// above and was declined on that basis. `Hellen` is accepted as unreliable
/// (2 of 5) rather than pursued.
///
/// **The ceiling this table has is the point.** Every rule in it held across
/// those five runs — the predicted forms simply never appeared — while the
/// model invented new ones. Prediction catches the slice already seen, so rows
/// are added from measurement and never from imagination, and a row is refused
/// when the word it rewrites is one somebody might legitimately say.
///
/// # The clearest evidence of that ceiling, measured the same day
///
/// The five runs above were dictated through a laptop microphone array. Five
/// more of the *same sentence* through a close-talk headset scored 21 of 25
/// against 19 — `LogicMonitor` and `ServiceNow` went to 5 of 5, and `Hellen`
/// from 2 to 4. But `JIRA` went **down**, 4 of 5 to 2, and the reason is the
/// whole argument: `Jura` — added to this table hours earlier from the first
/// set — did not appear once. The model had switched to `Gira`.
///
/// Nothing about the rule caused that; a correction cannot influence
/// recognition. Changing the microphone changed which wrong form the model
/// produced for a word it hears correctly and spells as a plausible name.
/// `Jura` stays anyway, because it was measured and costs nothing, but it is
/// now a row guarding a form nobody has seen since the day it was written.
///
/// So a row here buys a *specific* string, not a term. Adding one is worth
/// doing when the string is safe and the failure is common; expecting the set
/// to converge is not.
const MEASURED_MISHEARINGS: &[(&str, &str)] = &[
    ("Hewitt", "HUIT"),
    ("Helen", "Hellen"),
    // All three from the 2026-08-27 dictations, and safe in a way the refused
    // candidates are not: `servenow` is not a word in any language, and `Jura`
    // and `Gira` are a mountain range and an Italian verb, neither of which
    // anybody is dictating about in a ticketing system.
    //
    // `Jura` and `Gira` are the same mishearing twice. The speaker says JIRA,
    // the model hears the sound correctly and spells it as a plausible word,
    // and *which* word depends on the microphone. Both are kept; neither
    // should be read as having closed the case.
    ("Jura", "JIRA"),
    ("Gira", "JIRA"),
    ("servenow", "ServiceNow"),
];

fn protected_term_entries(terms: &[String]) -> Vec<DictionaryEntry> {
    // Every source that will exist, folded for comparison the same way the
    // matcher folds them. A derived variant that collides with a term the
    // user actually typed must be dropped rather than added: two entries
    // with one source are a `ConflictingRule`, and that rejects **every**
    // word in the batch rather than the duplicate -- the 2026-08-20 defect
    // that silently left a user with none of their vocabulary.
    let mut claimed: std::collections::BTreeSet<String> =
        terms.iter().map(|term| term.to_lowercase()).collect();
    let mut entries = Vec::with_capacity(terms.len());
    let mut derived = Vec::new();
    for (index, term) in terms.iter().enumerate() {
        entries.push(DictionaryEntry {
            id: format!("installer-{index}"),
            locale: "en-US".to_owned(),
            source: term.clone(),
            replacement: term.clone(),
            case_policy: CasePolicy::InsensitiveCanonical,
            boundary_policy: BoundaryPolicy::UnicodeWord,
            origin: DictionaryOrigin::UserEntry,
            precedence: 0,
            protected: true,
            enabled: true,
        });
        // The measured mishearings for this term, if any. **Before** the spaced
        // companion below, not after: that block ends in a `continue` for a
        // term with no lower-to-upper transition, and `HUIT` and `Hellen` are
        // exactly such terms -- so ordering these second silently skipped every
        // row in the table. Caught by the test that asserts a transcript rather
        // than the one that counts entries.
        //
        // Same guard as the companion: a `heard` form the user typed as a word
        // in its own right keeps its identity rule and gets no correction,
        // because two entries with one source reject the whole batch.
        for (row, (heard, meant)) in MEASURED_MISHEARINGS.iter().enumerate() {
            if !meant.eq_ignore_ascii_case(term) || !claimed.insert(heard.to_lowercase()) {
                continue;
            }
            derived.push(DictionaryEntry {
                // The row is in the id, not just the term. One term can have
                // several measured mishearings -- `JIRA` has two, `Jura` and
                // `Gira` -- and an id keyed on the term alone made them a
                // `DuplicateId` that rejected the batch. Caught by the
                // validator the moment the second row landed, which is what it
                // is for; a scheme that assumed one-per-term was fine right up
                // to the first term that had two.
                id: format!("installer-{index}-heard-{row}"),
                locale: "en-US".to_owned(),
                source: (*heard).to_owned(),
                replacement: term.clone(),
                case_policy: CasePolicy::InsensitiveCanonical,
                boundary_policy: BoundaryPolicy::UnicodeWord,
                origin: DictionaryOrigin::UserEntry,
                precedence: 0,
                protected: false,
                enabled: true,
            });
        }

        let Some(spaced) = speakeasy_transforms::spaced_variant(term) else {
            continue;
        };
        if !claimed.insert(spaced.to_lowercase()) {
            continue;
        }
        derived.push(DictionaryEntry {
            // A distinct id namespace, so `replace_user_entry_terms` on the
            // next install replaces these too rather than leaving orphans
            // keyed on a position the new list no longer has.
            id: format!("installer-{index}-spaced"),
            locale: "en-US".to_owned(),
            source: spaced,
            replacement: term.clone(),
            case_policy: CasePolicy::InsensitiveCanonical,
            boundary_policy: BoundaryPolicy::UnicodeWord,
            origin: DictionaryOrigin::UserEntry,
            precedence: 0,
            // Not `protected`: this is a correction rather than a word to
            // leave alone. The spaced form is what the user did *not* want.
            protected: false,
            enabled: true,
        });

    }
    entries.extend(derived);
    entries
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

/// Records a decision made before any coordinator is managed.
///
/// `log_event` cannot be used during `setup`: it reaches for
/// `app.state::<ProfileCoordinator>()`, which **panics** when the state is not
/// managed yet, inside a path that cannot unwind. Everything it needs is in hand
/// at that point anyway — the data root and the settings that were just loaded.
///
/// It exists because the first thing this logs had no log at all. Setup's
/// vocabulary was applied through a `let _ =`, so a rejected batch left the user
/// with none of their words and left no trace of why; the fault was found by
/// reading `personalization.json`, which is not a diagnostic route anyone has.
pub(crate) fn log_startup_event(
    root: &Path,
    disk_logging_enabled: bool,
    event: &str,
    fields: &[(&str, &str)],
) {
    if !disk_logging_enabled {
        return;
    }
    let _ = append_diagnostics_log(root, &diagnostic_line(None, event, fields));
}

/// One diagnostic line, in the format the log has always used.
///
/// Extracted so the startup path above cannot drift from the running one: two
/// formatters would eventually disagree about the field order, and the log is
/// read by eye.
fn diagnostic_line(session_id: Option<SessionId>, event: &str, fields: &[(&str, &str)]) -> String {
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
    line
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
    let line = diagnostic_line(session_id, event, fields);
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
        }
    }

    /// Writes a finished transcript to the plaintext history database, and says
    /// whether a row was actually written.
    ///
    /// Called after delivery has classified the target, never before: the
    /// `secure_target` flag the repository refuses on is a fact about where the
    /// transcript went, and is not known until then. The bool is returned rather
    /// than dropped because "refused by policy" and "stored" are different
    /// outcomes the diagnostic log has to tell apart.
    fn persist(&self, result: &TranscriptResult) -> Result<bool, &'static str> {
        let mut slot = self
            .repository
            .lock()
            .map_err(|_| "history_state_unavailable")?;
        let Some(repository) = slot.as_mut() else {
            return Ok(false);
        };
        repository
            .record(result)
            .map_err(|_| "history_write_failed")
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
