/// What the model status becomes once a warm has finished, given what is on
/// disk and what the warm hashed.
///
/// Split out of [`ModelCoordinator::settle_after_warm`] because the two rules it
/// enforces are pure, and both were broken by the version that inlined them --
/// where they could only be tested with 2.30 GB of real weights laid out on
/// disk, which is to say not at all.
///
/// **Never `verifying`.** The warm thread's work is done by the time this runs.
/// A presence answer of `installed_unverified` with nothing hashed stays
/// `installed_unverified`: the files are here and nobody read them. The first
/// version returned the presence answer unchanged from a catch-all *and* had
/// `readiness` return `verifying`, so any warm that stopped short of the digest
/// pass left the model line reading "Verifying installed model" for the life of
/// the process, with the settings page polling every 750 ms behind it.
///
/// **Only the pack that was hashed.** `resolved` is re-resolved with the
/// post-warm CUDA answer, which is exactly what the warm can change, so it is
/// not necessarily the pack the digest pass read. Both id and revision must
/// match or nothing is promoted. A `ResidentMismatch` -- a warm that found an
/// adapter loaded for a pack the resolver no longer points at -- has no identity
/// at all, so it falls out at the same guard: neither the loaded pack's digests
/// nor the resolved pack's absence of them is a fact about the other one.
fn settled_model_state(
    presence: (&'static str, Option<String>),
    resolved: Option<&(String, String)>,
    verification: &WarmVerification,
) -> (&'static str, Option<String>) {
    // The invariant is enforced here rather than assumed of the caller: whatever
    // `readiness` reports, a *settled* status may not say a pass is running,
    // because the warm thread's work is over. Stated as a mapping so that a
    // future `readiness` returning `verifying` -- which is what the first
    // version of this state machine did -- cannot leak through the
    // fall-throughs below.
    let presence = match presence {
        ("verifying", _) => ("installed_unverified", None),
        settled => settled,
    };
    let Some(identity) = verification.identity() else {
        return presence;
    };
    let about_this_pack =
        resolved.is_some_and(|(id, revision)| id == identity.0 && revision == identity.1);
    if !about_this_pack {
        return presence;
    }
    if verification.bytes_match() {
        ("verified_on_disk", None)
    } else {
        (
            "failed",
            Some("granite_model_files_unverified".to_owned()),
        )
    }
}

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
/// # This is a presence check, and it never claims a verification
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
/// only one with any claim to describing the bytes that get loaded, and it
/// already runs on its own thread.
///
/// So this answers presence and stops. `installed_unverified` is the honest
/// name for what it finds: the required files are here at their pinned lengths
/// and **nobody has read their bytes**. It is deliberately not `verifying` —
/// that word says a pass is running, and a state that claims an action is in
/// progress when no thread is doing it is the manufactured claim this
/// repository keeps finding. `verifying` is set by
/// [`ModelCoordinator::mark_verifying`] for exactly as long as a warm is
/// actually hashing.
fn readiness(root: &Path, cuda_worker_available: bool) -> (&'static str, Option<String>) {
    if bundled_manifest().is_err() {
        ("failed", Some("catalog_unavailable".to_owned()))
    } else if granite_engine::granite_selection(&root.join("models"), cuda_worker_available)
        .is_some_and(|selection| {
            InstallManager::new(root.join("models")).is_present(&selection.install_spec)
        })
    {
        ("installed_unverified", None)
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
    /// warm's digest pass is the one hash a launch takes, so its verdict is the
    /// only thing entitled to say `verified_on_disk`, and — when it says the
    /// bytes are wrong — the only thing entitled to say that either.
    ///
    /// # Two rules, both of which were broken by the first version of this
    ///
    /// **It must promote the pack that was actually hashed.** The warm can
    /// change which pack resolves: the CUDA answer arrives with the worker, and
    /// this function re-resolves with it. So a `&'static str` "the warm said
    /// ready" was being used as "these bytes were checked", and on a machine
    /// where the capability flipped the resolution it would have stamped pack B
    /// verified on the strength of pack A's digests. [`WarmVerification`]
    /// carries the identity, and a mismatch promotes nothing.
    ///
    /// **And the verdict is the caller's, not the coordinator's.** `verification`
    /// is what the warm this settle belongs to returned. It used to be read back
    /// off a shared field on `GraniteEngineCoordinator`, which any other pass in
    /// the process could overwrite between the warm ending and this running --
    /// including a dictation's own warm, which calls the same `ensure_ready`.
    ///
    /// **It must never leave `verifying` behind.** The warm thread has ended by
    /// the time this runs, so nothing is verifying, whatever happened. A warm
    /// that never reached the digest pass — no worker, memory below the floor,
    /// nothing configured, quarantine — lands on `installed_unverified`: the
    /// files are here and nobody read them. Reporting `verifying` there left the
    /// dock's model line saying "Verifying installed model" for the life of the
    /// process and the Transcription page polling `model_install_status` every
    /// 750 ms forever.
    fn settle_after_warm(
        &self,
        cuda_worker_available: bool,
        verification: &WarmVerification,
    ) {
        let presence = readiness(&self.root, cuda_worker_available);
        // Only a pack that is still present can be promoted or condemned; if
        // the resolver now points somewhere else, presence is the whole answer.
        let resolved = (presence.0 == "installed_unverified")
            .then(|| {
                granite_engine::granite_selection(&self.root.join("models"), cuda_worker_available)
            })
            .flatten()
            .map(|selection| (selection.pack_id, selection.pack_revision));
        let (state, error) = settled_model_state(presence, resolved.as_ref(), verification);
        Self::set_status(&self.status, state, error);
    }

    /// Says a digest pass is running, for exactly as long as one is -- and only
    /// when there is something to hash.
    ///
    /// A model that is `absent` stays `absent`. Announcing a verification over a
    /// machine with no pack installed is a flash of a state that cannot be true:
    /// the warm will not reach a digest pass, and the user watching the dock sees
    /// the app claim to be checking a model they have not installed.
    ///
    /// Called **before** the warm thread is spawned, not inside it, so there is
    /// no window in which the dock and the shortcut are exposed to a model that
    /// is about to start being hashed but does not say so yet. Paired with
    /// [`Self::settle_after_warm`], which always runs once the warm's work is
    /// done and therefore always replaces this.
    fn mark_verifying(&self) {
        if self.status_snapshot().state == "installed_unverified" {
            Self::set_status(&self.status, "verifying", None);
        }
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

