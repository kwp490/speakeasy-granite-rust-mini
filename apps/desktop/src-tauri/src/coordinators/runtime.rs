/// Whether a dictation started right now would find a model it can load.
///
/// Readiness is a claim about the pack a dictation will actually load, so it
/// asks the same resolver dictation asks. It used to ask whether *any* pack in
/// the manifest verified — `.any()`, unfiltered — which is a different question
/// and, once a second pack existed, a wrong one: installing the CPU pack on a
/// CUDA-capable machine flipped the app past `setup_requirement`'s
/// `model_missing` gate while the resolver went on resolving the *uninstalled*
/// CUDA pack. The app reported itself ready and could not transcribe. That trap
/// is not gone just because there is one engine now — Granite still publishes
/// CPU and CUDA packs — so this still asks the resolver rather than the catalog.
///
/// # This is a presence check, and it stops short of `verified_on_disk`
///
/// It used to call `reverify`, which hashes the resolved pack. That put a
/// 2.30 GB read here, and this function runs **twice** on a configured launch —
/// once inside `ModelCoordinator::new` on the `setup` path, and again from
/// [`ModelCoordinator::settle_after_warm`]. With the engine warm's own
/// `verify_pack_files` that was **three** full hashes, about 6.90 GB of reading,
/// before the app was usable.
///
/// One of the three is enough, and the right one is the warm's: it is the hash
/// taken immediately before the worker is handed the `model_root`, so it is the
/// only one with any claim to describing the bytes that get loaded. It also
/// already runs on its own thread. So presence decides *which* pack is here and
/// whether setup is needed, exactly as `InstallManager::is_present` is
/// documented for, and `verifying` is the honest name for a pack whose bytes
/// nobody has read yet this launch.
fn readiness(root: &Path, cuda_worker_available: bool) -> (&'static str, Option<String>) {
    if bundled_manifest().is_err() {
        ("failed", Some("catalog_unavailable".to_owned()))
    } else if granite_engine::granite_selection(&root.join("models"), cuda_worker_available)
        .is_some_and(|selection| {
            InstallManager::new(root.join("models")).is_present(&selection.install_spec)
        })
    {
        ("verifying", None)
    } else {
        ("absent", None)
    }
}

impl ModelCoordinator {
    fn new(root: PathBuf, cuda_worker_available: bool) -> Self {
        let (state, error) = readiness(&root, cuda_worker_available);
        Self {
            root,
            status: Arc::new(Mutex::new(ModelInstallView {
                state: state.to_owned(),
                error,
                bytes_downloaded: None,
                bytes_total: None,
            })),
            cancel: Arc::new(Mutex::new(None)),
            active_download: Arc::new(Mutex::new(None)),
        }
    }

    fn set_status(status: &Mutex<ModelInstallView>, state: &str, error: Option<String>) {
        if let Ok(mut status) = status.lock() {
            *status = ModelInstallView {
                state: state.to_owned(),
                error,
                bytes_downloaded: None,
                bytes_total: None,
            };
        }
    }

    fn status_snapshot(&self) -> ModelInstallView {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Recomputes readiness once the engine warm has spoken, and promotes the
    /// pack out of `verifying` using what the warm actually found.
    ///
    /// Two things happen here, and they used to be one. **Re-resolving** matters
    /// because which pack a dictation loads depends on whether this worker turned
    /// out to be CUDA-capable — a fact only the worker can report. Installing the
    /// CUDA runtime changes the same answer without touching a pack, and left
    /// alone the app would go on reporting "Setup needed" until relaunched.
    ///
    /// **Promotion** matters because `readiness` no longer hashes anything. The
    /// warm's `verify_pack_files` is the one hash a launch takes, so its verdict
    /// is the only thing entitled to say `verified_on_disk` — and, when it says
    /// the bytes are wrong, the only thing entitled to say so. A warm that failed
    /// for some other reason (no worker, memory below the floor, quarantine)
    /// leaves the pack at `verifying`: it says nothing about the bytes, and
    /// claiming a corrupt model on that evidence would be a manufactured fault.
    fn settle_after_warm(&self, cuda_worker_available: bool, warm_state: &str) {
        let (state, error) = readiness(&self.root, cuda_worker_available);
        let (state, error) = match (state, warm_state) {
            ("verifying", "ready") => ("verified_on_disk", None),
            ("verifying", "granite_model_files_unverified") => (
                "failed",
                Some("granite_model_files_unverified".to_owned()),
            ),
            _ => (state, error),
        };
        Self::set_status(&self.status, state, error);
    }
}


#[cfg(test)]
impl Phase1Coordinator {
    /// Returns the number of retained redacted events without exposing content.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when the audit lock is unavailable.
    pub fn audit_event_count(&self) -> Result<usize, &'static str> {
        self.audit
            .lock()
            .map(|events| events.len())
            .map_err(|_| "audit_unavailable")
    }

    fn run_fake(&self, request: &FakeFlowRequest) -> Result<FakeFlowResponse, &'static str> {
        if request.schema_version != DOMAIN_SCHEMA_VERSION {
            return Err("ipc_schema_unsupported");
        }
        let (reply, response) = sync_channel(1);
        self.requests
            .try_send(FakeActorRequest {
                failure: request.failure,
                reply,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => "coordinator_busy",
                TrySendError::Disconnected(_) => "coordinator_unavailable",
            })?;
        response
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "coordinator_deadline")?
    }

    fn run_fake_inner(
        audit: &Mutex<VecDeque<RedactedAuditEvent>>,
        failure: Option<FakeFailure>,
    ) -> Result<FakeFlowResponse, &'static str> {
        let correlation_id = CorrelationId::from_bytes([1; 16]);
        let producer_id = ProducerId::from_bytes([2; 16]);
        let session_id = SessionId::from_bytes([3; 16]);
        let mut reducer = Reducer::default();
        let mut states = vec![state(0, SessionPhase::Idle, None, None)];
        if reducer.begin_session(session_id) != ReducerDisposition::Applied {
            return Err("invalid_transition");
        }
        states.push(state(1, SessionPhase::Arming, None, None));

        if matches!(failure, Some(FakeFailure::AudioStart)) {
            apply_phase(
                &mut reducer,
                correlation_id,
                producer_id,
                session_id,
                1,
                SessionPhase::Failed,
            )?;
            states.push(state(
                2,
                SessionPhase::Failed,
                None,
                Some("audio_start_failed"),
            ));
            return Ok(FakeFlowResponse {
                schema_version: DOMAIN_SCHEMA_VERSION,
                states,
            });
        }

        for (source_sequence, phase) in [
            (1, SessionPhase::Capturing),
            (2, SessionPhase::Draining),
            (3, SessionPhase::Finalizing),
        ] {
            apply_phase(
                &mut reducer,
                correlation_id,
                producer_id,
                session_id,
                source_sequence,
                phase,
            )?;
            states.push(state(source_sequence + 1, phase, None, None));
        }

        if matches!(failure, Some(FakeFailure::Finalize)) {
            apply_phase(
                &mut reducer,
                correlation_id,
                producer_id,
                session_id,
                4,
                SessionPhase::Failed,
            )?;
            states.push(state(
                5,
                SessionPhase::Failed,
                None,
                Some("finalize_failed"),
            ));
        } else {
            apply_phase(
                &mut reducer,
                correlation_id,
                producer_id,
                session_id,
                4,
                SessionPhase::Delivering,
            )?;
            states.push(state(
                5,
                SessionPhase::Delivering,
                Some(FAKE_TRANSCRIPT),
                None,
            ));
            let final_phase = if matches!(failure, Some(FakeFailure::Delivery)) {
                SessionPhase::Failed
            } else {
                SessionPhase::Delivered
            };
            apply_phase(
                &mut reducer,
                correlation_id,
                producer_id,
                session_id,
                5,
                final_phase,
            )?;
            states.push(state(
                6,
                final_phase,
                Some(FAKE_TRANSCRIPT),
                (final_phase == SessionPhase::Failed).then_some("delivery_failed"),
            ));
        }

        Self::record_audit(audit, &states)?;
        Ok(FakeFlowResponse {
            schema_version: DOMAIN_SCHEMA_VERSION,
            states,
        })
    }

    fn record_audit(
        audit: &Mutex<VecDeque<RedactedAuditEvent>>,
        states: &[IpcState],
    ) -> Result<(), &'static str> {
        let event = RedactedAuditEvent {
            code: states
                .last()
                .and_then(|item| item.error_code)
                .unwrap_or("completed"),
            transcript_characters: states
                .last()
                .and_then(|item| item.transcript)
                .map_or(0, str::len),
        };
        let mut audit = audit.lock().map_err(|_| "audit_unavailable")?;
        if audit.len() == AUDIT_CAPACITY {
            audit.pop_front();
        }
        audit.push_back(event);
        Ok(())
    }
}

#[cfg(test)]
fn apply_phase(
    reducer: &mut Reducer,
    correlation_id: CorrelationId,
    producer_id: ProducerId,
    session_id: SessionId,
    source_sequence: u64,
    phase: SessionPhase,
) -> Result<(), &'static str> {
    let ingress = IngressEvent {
        schema_version: DOMAIN_SCHEMA_VERSION,
        correlation_id,
        session_id: Some(session_id),
        producer_id,
        source_sequence,
        producer_monotonic_ns: source_sequence,
        payload: (),
    };
    let (disposition, _) = reducer.apply(ingress, phase, source_sequence);
    (disposition == ReducerDisposition::Applied)
        .then_some(())
        .ok_or("invalid_transition")
}

#[cfg(test)]
const fn state(
    sequence: u64,
    session: SessionPhase,
    transcript: Option<&'static str>,
    error_code: Option<&'static str>,
) -> IpcState {
    IpcState {
        schema_version: DOMAIN_SCHEMA_VERSION,
        sequence,
        readiness: match AppReadiness::Ready {
            AppReadiness::Ready => "ready",
            _ => "unreachable",
        },
        session: match session {
            SessionPhase::Idle => "idle",
            SessionPhase::Arming => "arming",
            SessionPhase::Capturing => "capturing",
            SessionPhase::Draining => "draining",
            SessionPhase::Finalizing => "finalizing",
            SessionPhase::Delivering => "delivering",
            SessionPhase::Delivered => "delivered",
            SessionPhase::Cancelled => "cancelled",
            SessionPhase::Failed => "failed",
        },
        engine: if matches!(session, SessionPhase::Finalizing) {
            "running"
        } else {
            "ready"
        },
        delivery: if matches!(session, SessionPhase::Delivering | SessionPhase::Delivered) {
            "result_view_only"
        } else {
            "ready"
        },
        transcript,
        error_code,
    }
}

