#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_diagnostic_paths_are_redacted_before_persistence() {
        let panic = r"thread 'main' panicked at C:\Users\Alice\SpeakEasy\src\worker.rs:42:7";
        let native = r"loader failed file=/home/alice/models/nemotron/model.onnx";
        let panic_redacted = redact_diagnostic_text(panic);
        let native_redacted = redact_diagnostic_text(native);
        assert!(!panic_redacted.contains("Alice"));
        assert!(!panic_redacted.contains("worker.rs"));
        assert!(!native_redacted.contains("/home/alice"));
        assert!(!native_redacted.contains("model.onnx"));
        assert!(panic_redacted.contains("<redacted-path>"));
        assert!(native_redacted.contains("file=<redacted-path>"));
    }

    #[test]
    fn diagnostic_rotation_preserves_one_previous_generation() {
        let root = tempfile::tempdir().expect("temporary diagnostic root");
        let path = root.path().join("logs/speakeasy.log");
        let old = "old-diagnostic\n";
        let mut oversized = old.to_owned();
        oversized.push_str(&"x".repeat(usize::try_from(DIAGNOSTICS_LOG_MAX_BYTES).unwrap() + 1));
        append_diagnostics_line(&path, &oversized).expect("seed diagnostic log");
        append_diagnostics_line(&path, "event=after_rotation\n").expect("rotate diagnostic log");
        let rotated = path.with_extension("log.1");
        assert!(rotated.is_file());
        assert!(
            fs::read_to_string(rotated)
                .expect("read rotated log")
                .contains("old-diagnostic")
        );
        assert!(
            fs::read_to_string(path)
                .expect("read active log")
                .contains("after_rotation")
        );
    }

    #[test]
    fn diagnostics_reason_codes_are_always_on_bounded_and_redacted() {
        let diagnostics = DiagnosticsRuntimeCoordinator::default();
        diagnostics
            .record_event("123 event=worker_failed path=C:\\Users\\Alice\\SpeakEasy\\worker.exe");
        for index in 0..=DIAGNOSTICS_EVENT_CAPACITY {
            diagnostics.record_event(&format!("{index} event=bounded_failure reason=code"));
        }

        let events = diagnostics.recent_reason_codes();
        assert_eq!(events.len(), DIAGNOSTICS_EVENT_CAPACITY);
        assert!(!events.iter().any(|event| event.contains("Alice")));
        assert!(
            events
                .iter()
                .all(|event| event.len() <= DIAGNOSTIC_EVENT_MAX_BYTES)
        );
        assert!(
            events
                .last()
                .is_some_and(|event| event.contains("event=bounded_failure"))
        );
    }

    #[test]
    fn install_payload_uses_every_granite_file_and_preserves_archive_packs() {
        let manifest = bundled_manifest().expect("bundled manifest");
        let temp = tempfile::tempdir().expect("download root");
        let granite = manifest
            .packs()
            .iter()
            .find(|pack| pack.id() == "granite-speech-4.1-2b-q4_k_m-cpu")
            .expect("Granite pack");
        let ModelInstallPayload::Loose(files) =
            model_install_payload(granite, temp.path()).expect("Granite payload")
        else {
            panic!("Granite must install as loose files");
        };
        assert_eq!(files.len(), 2);
        assert_eq!(
            files
                .iter()
                .map(|(path, request)| (path.as_path(), request.expected_bytes))
                .collect::<Vec<_>>(),
            vec![
                (
                    Path::new("granite-speech-4.1-2b-Q4_K_M.gguf"),
                    1_139_247_200
                ),
                (Path::new("mmproj-model-f16.gguf"), 1_159_354_752),
            ]
        );

        // The archive branch, on a pack derived from the shipped one.
        //
        // No pack in this catalog is an archive pack any more: both Granite
        // packs are the loose-GGUF shape, and the `.tar.gz`/`.tar.bz2` packs
        // were the streaming engine's. Asking the manifest for one, which is
        // what this test used to do, now finds nothing and panics.
        //
        // The branch is kept rather than deleted because the manifest schema
        // still admits archive packs and the GPU Granite pack is likely to be
        // one -- a CUDA worker plus its two redistributable DLLs is an archive,
        // not a loose file set. So the coverage is kept too, by adding an
        // `archive` block to the real pack and re-parsing. Deriving it from the
        // shipped pack rather than hand-writing a fixture means the rest of the
        // pack stays valid against whatever the schema requires, which a
        // hand-written one would silently drift from.
        let mut catalog: serde_json::Value =
            serde_json::from_slice(speakeasy_models::BUNDLED_TRUSTED_MANIFEST_BYTES)
                .expect("bundled manifest parses as JSON");
        let pack = catalog["packs"]
            .as_array_mut()
            .expect("packs array")
            .iter_mut()
            .find(|pack| pack["id"] == "granite-speech-4.1-2b-q4_k_m-cpu")
            .expect("Granite pack");
        pack["id"] = serde_json::json!("granite-speech-4.1-2b-q4_k_m-archived");
        pack["archive_prefix"] = serde_json::json!("granite-speech-4.1-2b-q4_k_m");
        pack["archive"] = serde_json::json!({
            "url": "https://example.invalid/granite-q4-k-m.tar.gz",
            "bytes": 2_298_601_952_u64,
            "sha256": "0".repeat(64),
        });
        let archived = speakeasy_models::TrustedManifest::parse(
            &serde_json::to_vec(&catalog).expect("re-serialize catalog"),
        )
        .expect("derived catalog is still valid");
        let archive_pack = archived
            .packs()
            .iter()
            .find(|pack| pack.id() == "granite-speech-4.1-2b-q4_k_m-archived")
            .expect("derived archive pack");

        let ModelInstallPayload::Archive(request) =
            model_install_payload(archive_pack, temp.path()).expect("archive payload")
        else {
            panic!("an archive pack must keep the archive payload shape");
        };
        assert_eq!(
            request.destination,
            temp.path().join(format!(
                "{}-{}.archive",
                archive_pack.id(),
                archive_pack.revision()
            ))
        );
    }

    /// Probe: reports what UI Automation actually offers in whatever window is
    /// focused when it runs. Live external typing needs to track the exact range
    /// `SpeakEasy` inserted, and that is only possible if the target exposes
    /// readable document offsets — which Electron apps frequently do not.
    ///
    /// Answering this empirically decides the adapter design, so run it against
    /// each target you care about before any of it is written (`--nocapture` is
    /// required, otherwise the report is swallowed):
    ///
    /// ```text
    /// cargo test -p speakeasy-desktop --lib probe_focused -- --ignored --nocapture
    /// ```
    ///
    /// You have ten seconds after starting it to click into the target's text
    /// box and type a couple of words.
    #[test]
    #[ignore = "interactive probe; focus a target window while it runs"]
    fn probe_focused_target_uia_capability() {
        let observer = TargetObserver::spawn().expect("uia observer");
        println!("\nFocus the target's text field and type a few words. Probing in 10s...");
        std::thread::sleep(Duration::from_secs(10));

        let snapshot = match observer.inspect(new_session_id()) {
            Ok(snapshot) => snapshot,
            Err(refusal) => {
                println!("REFUSED: {refusal:?}");
                return;
            }
        };
        println!("app            : {}", snapshot.executable.path);
        println!("integrity      : {:?}", snapshot.integrity);
        println!("capability     : {:?}", snapshot.capability);
        println!("read_only      : {}", snapshot.is_read_only);
        println!("password       : {}", snapshot.is_password);
        println!(
            "patterns       : text={} text2(caret)={} value={}",
            snapshot.patterns.text, snapshot.patterns.text2, snapshot.patterns.value
        );
        match &snapshot.selection {
            Some(selection) => println!(
                "selection      : start={:?} end={:?} caret={:?} empty={}",
                selection.start, selection.end, selection.caret, selection.is_empty
            ),
            None => println!("selection      : NONE"),
        }
        println!(
            "content f.print: {}",
            if snapshot.content_fingerprint.is_some() {
                "readable"
            } else {
                "NONE"
            }
        );

        // The verdict that actually decides the design.
        let offsets = snapshot
            .selection
            .as_ref()
            .is_some_and(|selection| selection.start.is_some() && selection.end.is_some());
        println!(
            "\nVERDICT: {}",
            if offsets {
                "document offsets readable - an insertion range can be tracked and \
                 verified, so a real select-and-replace is possible here."
            } else if snapshot.patterns.text {
                "TextPattern present but NO usable offsets - the inserted range \
                 cannot be verified. Append-only with a refuse-to-correct fallback."
            } else {
                "no TextPattern - blind typing only. Nothing can be verified or \
                 corrected in place; this target must stay commit-on-finish."
            }
        );
    }

    #[test]
    fn fake_flow_is_ordered_and_redacted_audit_omits_content() {
        let coordinator = Phase1Coordinator::default();
        let response = coordinator
            .run_fake(&FakeFlowRequest {
                schema_version: DOMAIN_SCHEMA_VERSION,
                failure: None,
            })
            .expect("fake flow");
        assert_eq!(response.states.last().expect("state").session, "delivered");
        assert!(
            response
                .states
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        let audit = coordinator.audit.lock().expect("audit");
        assert_eq!(audit[0].code, "completed");
        assert_eq!(audit[0].transcript_characters, FAKE_TRANSCRIPT.len());
    }

    #[test]
    fn every_failure_is_recoverable_as_sanitized_state() {
        for failure in [
            FakeFailure::AudioStart,
            FakeFailure::Finalize,
            FakeFailure::Delivery,
        ] {
            let response = Phase1Coordinator::default()
                .run_fake(&FakeFlowRequest {
                    schema_version: DOMAIN_SCHEMA_VERSION,
                    failure: Some(failure),
                })
                .expect("failure response");
            let last = response.states.last().expect("state");
            assert_eq!(last.session, "failed");
            assert!(last.error_code.is_some());
        }
    }

    #[test]
    fn result_view_retains_text_and_provenance_without_fabricating_empty_output() {
        let coordinator = ResultCoordinator::default();
        let session_id = SessionId::from_bytes([9; 16]);
        coordinator
            .accept(FinalTranscript {
                session_id,
                raw_text: "  exact engine output  ".to_owned(),
                text: "exact engine output".to_owned(),
                provenance: TranscriptProvenance::FinalizedStream,
                metrics: speakeasy_domain::FinalAsrMetrics {
                    input_samples: 16_000,
                    final_segments: 1,
                    draft_revisions: 2,
                },
            })
            .expect("accept result");
        let view = coordinator.view().expect("result view");
        assert_eq!(view.text.as_deref(), Some("exact engine output"));
        assert_eq!(view.provenance.as_deref(), Some("finalized_stream"));
        assert_eq!(view.input_samples, Some(16_000));

        assert_eq!(
            coordinator.accept(FinalTranscript {
                session_id,
                raw_text: String::new(),
                text: String::new(),
                provenance: TranscriptProvenance::LastValidDraft,
                metrics: speakeasy_domain::FinalAsrMetrics::default(),
            }),
            Err("empty_result_rejected")
        );
    }

    /// Composition stand-in for the halves of the HUD view that come from the
    /// other coordinators. Deliberately not "everything is fine": the default
    /// is a profile that cannot dictate, so a test has to opt in to readiness.
    fn idle_composition() -> HudComposition {
        HudComposition {
            session: "idle",
            level: 0.0,
            device_diagnostic: "not_opened".to_owned(),
            device_name: String::new(),
            hotkey_binding: "Ctrl+Alt+L".to_owned(),
            hotkey_registration: "registered".to_owned(),
            can_start: true,
            can_stop: false,
            setup_complete: true,
            setup_reason: None,
            elapsed_ms: 0,
            ceiling_ms: 30_000,
            preferred_device_id: String::new(),
            // Warmed, so these tests stay about composition rather than about
            // the load. The warm states themselves are covered by
            // `granite_engine`'s own tests, not here.
            engine: "ready",
            engine_device: "cpu",
            queue_depth: 0,
            error_code: None,
        }
    }

    /// The warm state has to reach the frontend, and has to move `sequence` when
    /// it changes.
    ///
    /// Without it, `Loading model` is unimplementable: a verified-on-disk model
    /// makes `setup_complete`, `can_start` and `session: "idle"` all report
    /// ready from the moment the window appears, while the launch warm is still
    /// loading — and a start landing in that window blocks inside
    /// `dictation_start` on the load's own mutex.
    #[test]
    fn the_engines_warm_state_is_published_and_advances_the_sequence() {
        let hud = CaptureHudCoordinator::default();
        let warming = hud
            .view(HudComposition {
                engine: "warming",
                ..idle_composition()
            })
            .expect("HUD status");
        assert_eq!(warming.engine, "warming");
        // Nothing else about the profile changed, so the load finishing is the
        // only thing the stale-response guard has to notice here.
        let ready = hud.view(idle_composition()).expect("HUD status");
        assert_eq!(ready.engine, "ready");
        assert!(ready.sequence > warming.sequence);
    }

    /// The coordinator the HUD actually reads answers in the HUD's vocabulary,
    /// and never in the pack's.
    ///
    /// The test above proves the *plumbing* carries a warm state, because it
    /// hands `HudComposition` the string by hand. It never touches the code that
    /// decides what the string is — and that code was filling the field from
    /// `engine_reason()`, which speaks pack codes and can say neither `warming`
    /// nor `ready`. So the dock's loading state was unreachable from the first
    /// poll onward while a green test asserted the contract it violated. That is
    /// the recurring near-miss in this repository: a passing test over a path
    /// nothing real executes.
    ///
    /// This one asks the real accessor. It cannot assert `warming` — that needs
    /// a worker mid-load — but it can pin the two properties that were actually
    /// wrong: a fresh coordinator is `cold` rather than a pack code, and no pack
    /// code is reachable through this accessor at all.
    #[test]
    fn the_hud_engine_field_speaks_warm_states_and_never_pack_reasons() {
        let coordinator = granite_engine::GraniteEngineCoordinator::default();

        // Before any warm the honest answer is `cold`. It used to be
        // `not_configured` here, which is a pack reason and which
        // `ENGINE_LOADING` in `transcriberState.ts` does not match — so the dock
        // went straight to idle and claimed a ready engine.
        assert_eq!(coordinator.warm_state(), "cold");

        // The two accessors answer different questions, and the HUD must read
        // this one. If they ever return the same string for a fresh
        // coordinator, the distinction has collapsed and this test is the only
        // thing that would notice.
        assert_eq!(coordinator.engine_reason(), "not_configured");
        assert_ne!(coordinator.warm_state(), coordinator.engine_reason());

        // Every code the pack selector can produce, asserted absent from the
        // warm vocabulary. Read from `EngineChoiceReason::code` rather than
        // retyped, so a new pack reason cannot quietly become a valid warm
        // state — a hand-written copy of a value cannot see that value change.
        for reason in [
            granite_engine::EngineChoiceReason::ProbePreferred,
            granite_engine::EngineChoiceReason::CpuGpuPackNotInstalled,
            granite_engine::EngineChoiceReason::CpuGpuRuntimeMissing,
        ] {
            assert_ne!(
                coordinator.warm_state(),
                reason.code(),
                "the HUD's engine field must never carry a pack reason; it is \
                 documented as cold/warming/ready/<error code> and the frontend \
                 keys its loading state on exactly that"
            );
        }

        // `shutdown` returns it to `cold`, because that is then true: the next
        // dictation warms again. Latching `ready` past an `invalidate` would be
        // the same lie pointed the other way.
        coordinator.shutdown();
        assert_eq!(coordinator.warm_state(), "cold");
    }

    /// A warm that concludes without warming does not leave the dock loading.
    ///
    /// `cold` means "not loaded **yet**" and the frontend maps it to
    /// `loading_model` alongside `warming`. A machine that will never warm —
    /// no worker binary, no manifest, no admissible pack — must therefore not
    /// be left on `cold`, or the dock reports "Loading model" for the life of
    /// the process while nothing is loading.
    ///
    /// Found by running it, not by reading it: a build whose worker path did
    /// not resolve logged `granite_warm result=ok engine=not_configured` and
    /// the dock sat on `loading_model` for a full minute of sampling. Before
    /// this change that same path reported idle, so the first cut of the fix
    /// made this case worse — which is exactly why the phase asks for the state
    /// to be *seen* working before anything renders it.
    #[test]
    fn a_warm_that_never_starts_is_not_reported_as_loading() {
        let coordinator = granite_engine::GraniteEngineCoordinator::default();
        // No worker binary: the first early return in
        // `warm_granite_if_configured`, and the one the dev-run build hits.
        let outcome = granite_engine::warm_granite_if_configured(
            granite_engine::GraniteEnvironment {
                granite_worker_exe: None,
                install_root: std::path::Path::new("."),
                total_memory_bytes: Some(32 * 1024 * 1024 * 1024),
                diagnostic_log: None,
                recorded_provider: "unrecorded",
                // Never reached: the worker-path check returns before anything
                // warms. The real probe rather than a double, because a test
                // double here would imply this path exercises it.
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
            },
            &coordinator,
        );
        assert!(outcome.is_ok(), "an unconfigured engine is not an error");
        assert_ne!(
            coordinator.warm_state(),
            "cold",
            "a warm that never started must not report as still loading"
        );
        assert_ne!(coordinator.warm_state(), "warming");
        assert_eq!(coordinator.warm_state(), "not_configured");
    }

    /// The HUD poll fills `engine` from the warm state, asserted against source.
    ///
    /// The two tests above cannot catch the bug that actually happened. One
    /// hands `HudComposition` a string by hand; the other asks the coordinator
    /// directly. **Neither touches the line that chooses between the two
    /// accessors**, and that line is the whole defect: `capture_hud_status`
    /// filled `engine` from `engine_reason()` for the life of the fork, so a
    /// revert of the fix would leave both of them green.
    ///
    /// It cannot be reached by calling the command — it is a
    /// `#[tauri::command]` taking an `AppHandle`, and standing up a Tauri app
    /// in a unit test to read one field would be a worse instrument than
    /// reading the source. So this reads the source, the way the window
    /// allowlist and the menu-id check already do.
    #[test]
    fn the_hud_poll_fills_engine_from_the_warm_state_not_the_pack_reason() {
        let source = include_str!("commands/capture.rs");
        let start = source
            .find("fn capture_hud_status")
            .expect("capture_hud_status must exist");
        let end = source[start..]
            .find("\nfn ")
            .map_or(source.len(), |offset| start + offset);
        let poll = &source[start..end];

        // Instrument self-check first: every assertion below is "the wrong
        // thing is absent", which is also what slicing the wrong function
        // reports. Prove the slice is the right one and is non-trivial.
        assert!(
            poll.contains("engine:") && poll.contains("engine_device:"),
            "the slice does not contain the fields under test, so it is the \
             wrong function or the extraction is broken"
        );

        assert!(
            poll.contains("granite.warm_state()"),
            "capture_hud_status must fill `engine` from `warm_state()`. The \
             frontend documents this field as cold/warming/ready/<error code> \
             and keys its loading state on it."
        );
        assert!(
            !poll.contains("granite.engine_reason()"),
            "capture_hud_status must not fill the HUD from `engine_reason()`. \
             That returns pack codes, never `warming` or `ready`, so the dock's \
             loading state becomes unreachable and it claims a loaded engine \
             throughout the ~2 GB launch warm. The pack reason is still \
             disclosed by `granite_warm` in the log and by the Advanced page."
        );
        assert!(
            poll.contains("granite.device()"),
            "`engine_device` must come from the worker's own reported device"
        );
    }

    /// The compute device is not the microphone, and the view keeps them apart.
    ///
    /// `device_name` and `device_diagnostic` are the capture device;
    /// `engine_device` is where Granite runs. The names are close enough that
    /// one being filled from the other would read as plausible in every log and
    /// on every page.
    #[test]
    fn the_engine_device_is_not_the_capture_device() {
        let hud = CaptureHudCoordinator::default();
        let view = hud
            .view(HudComposition {
                device_name: "Headset Microphone".to_owned(),
                engine_device: "cuda",
                ..idle_composition()
            })
            .expect("HUD status");
        assert_eq!(view.engine_device, "cuda");
        assert_eq!(view.device_name, "Headset Microphone");

        // And it participates in the stale-response guard like every other
        // field: `CaptureHudView` derives `PartialEq` and `view()` compares
        // before publishing, so a device change has to move the sequence or the
        // frontend would never see it.
        let moved = hud
            .view(HudComposition {
                device_name: "Headset Microphone".to_owned(),
                engine_device: "cpu",
                ..idle_composition()
            })
            .expect("HUD status");
        assert_eq!(moved.engine_device, "cpu");
        assert!(moved.sequence > view.sequence);
    }

    #[test]
    fn hud_defaults_to_explicit_final_only_claim_block() {
        let hud = CaptureHudCoordinator::default();
        let view = hud.view(idle_composition()).expect("HUD status");
        assert_eq!(view.streaming_mode, "final_only");
        assert!(view.mutable_text.is_empty());
        assert!(view.stable_display_text.is_empty());
        assert!(view.final_text.is_empty());
    }

    /// Silence must not report as a fault, in either spelling.
    ///
    /// The engine's verdict returns `no_speech`; the runtime path returns
    /// `runtime_no_speech_detected`. Both reach the same match arms, and when
    /// only one of them was matched, ordinary silence took the failure path —
    /// a quarantine strike, a `failed` capture state, and an error where the
    /// honest answer is "you did not say anything".
    #[test]
    fn both_spellings_of_silence_are_treated_as_silence_not_as_a_fault() {
        assert!(is_no_speech("no_speech"));
        assert!(is_no_speech("runtime_no_speech_detected"));

        // Everything else is a real failure and must keep taking the failure
        // path, including the reasons that sit closest to silence.
        for code in [
            "granite_empty",
            "granite_failed",
            "granite_implausible",
            "granite_unavailable",
            "granite_quarantined",
            "runtime_adapter_failed",
        ] {
            assert!(!is_no_speech(code), "{code} must not be treated as silence");
        }
    }

    /// The window a second press must not be able to start a dictation in.
    ///
    /// Pure, so it can be asserted without a Tauri app: `dictation_is_finishing`
    /// is this function plus two coordinator reads, and this is the half that
    /// decides. What it protects is a real observed sequence -- a ceiling stop at
    /// 120,183 ms, a press 490 ms later, a second dictation queued behind the
    /// first for 36.6 s, and its transcript pasted wherever the user had moved on
    /// to. Nothing errored, which is why only a rule can catch it.
    #[test]
    fn a_dictation_is_still_finishing_until_its_transcript_is_delivered() {
        // Recording over, transcript not delivered: every one of these is a
        // window where the next press is a *new* dictation rather than a stop.
        for state in ["draining", "captured", "finalizing"] {
            assert_eq!(
                hud_session_with_delivery(state, true),
                if state == "finalizing" { "finalizing" } else { "stopping" },
                "{state} is still finishing"
            );
        }

        // The promotion, which is the load-bearing half. Transcription being
        // finished is not the text having arrived: `complete` with delivery
        // unresolved has to read as `finalizing`, or the guard opens exactly at
        // the moment inference ends -- 4.2 s in on the card, 44.5 s on the
        // processor, and still before the paste.
        assert_eq!(hud_session_with_delivery("complete", true), "finalizing");
        assert_eq!(hud_session_with_delivery("complete", false), "complete");

        // Recording, and a press here is a stop. The guard must never reach
        // these, or the shortcut stops being able to end a dictation.
        assert_eq!(hud_session_with_delivery("arming", true), "starting");
        assert_eq!(hud_session_with_delivery("capturing", true), "streaming");

        // Nothing in flight. A refusal here would be a shortcut that does
        // nothing, for no reason the user could discover.
        for state in ["idle", "ready", "unknown"] {
            assert!(
                !matches!(
                    hud_session_with_delivery(state, true),
                    "stopping" | "finalizing"
                ),
                "{state} must not refuse a press"
            );
        }
        assert!(!matches!(
            hud_session_with_delivery("failed", true),
            "stopping" | "finalizing"
        ));
    }

    #[test]
    fn delivery_outcome_is_reported_as_it_happened() {
        let hud = CaptureHudCoordinator::default();
        let session_id = SessionId::from_bytes([9; 16]);

        hud.begin(session_id);
        // Before delivery resolves, the view must not claim any outcome.
        assert_eq!(
            hud.view(idle_composition())
                .expect("HUD status")
                .delivery_outcome,
            "held"
        );

        hud.finish("the text a password box refused", "refused", None);
        let view = hud.view(idle_composition()).expect("HUD status");
        assert_eq!(view.delivery_outcome, "refused");
        assert_eq!(
            view.final_text, "the text a password box refused",
            "a refused paste must leave the text recoverable, not discard it"
        );
    }

    /// A dictation shows no text until there is a real one.
    ///
    /// This was `authoritative_final_replaces_the_live_guess`, and it proved
    /// the delivered transcript overwrote the streaming hypotheses that had
    /// been on screen while the user spoke. There are no hypotheses to
    /// overwrite. What is worth pinning instead is that the window between
    /// starting and finishing is genuinely empty -- the failure this guards
    /// against is a stale transcript from the *previous* dictation still
    /// standing while the current one records.
    #[test]
    fn a_recording_shows_no_text_until_the_engine_returns_one() {
        let hud = CaptureHudCoordinator::default();
        let first = SessionId::from_bytes([9; 16]);
        hud.begin(first);
        hud.finish("Ever tried? Ever failed?", "inserted", None);
        assert_eq!(
            hud.view(idle_composition()).expect("HUD status").final_text,
            "Ever tried? Ever failed?"
        );

        hud.begin(SessionId::from_bytes([10; 16]));
        let recording = hud.view(idle_composition()).expect("HUD status");
        assert!(
            recording.final_text.is_empty(),
            "the previous dictation's text must not stand while a new one records"
        );
        assert!(recording.stable_display_text.is_empty());
        assert!(recording.mutable_text.is_empty());

        hud.finish("No matter. Try again.", "inserted", None);
        let delivered = hud.view(idle_composition()).expect("HUD status");
        assert_eq!(delivered.final_text, "No matter. Try again.");
    }

    #[test]
    fn capture_states_map_to_the_states_the_user_is_shown() {
        // Streaming may be unavailable — no model, no worker — and these still
        // have to be right, which is why they derive from capture, not the tap.
        assert_eq!(hud_session_of("arming"), "starting");
        assert_eq!(hud_session_of("capturing"), "streaming");
        assert_eq!(hud_session_of("draining"), "stopping");
        assert_eq!(hud_session_of("captured"), "stopping");
        assert_eq!(hud_session_of("finalizing"), "finalizing");
        assert_eq!(hud_session_of("complete"), "complete");
        assert_eq!(hud_session_of("failed"), "failed");
        assert_eq!(hud_session_of("unavailable"), "failed");
        assert_eq!(hud_session_of("idle"), "idle");
        assert_eq!(hud_session_of("something-new"), "idle");
    }

    #[test]
    fn desktop_operation_gate_blocks_model_changes_during_retained_dictation() {
        let operations = OperationCoordinator::default();
        let session_id = SessionId::from_bytes([7; 16]);
        operations.begin_dictation(session_id).expect("dictation");
        assert_eq!(
            operations.begin(ExclusiveOperation::ModelInstall),
            Err("dictation_active_operation_deferred")
        );
        assert_eq!(
            operations.begin(ExclusiveOperation::ModelDelete),
            Err("dictation_active_operation_deferred")
        );
        assert_eq!(
            operations.begin(ExclusiveOperation::ApplicationUpdate),
            Err("dictation_active_operation_deferred")
        );
        assert_eq!(
            operations.begin(ExclusiveOperation::StorageMigration),
            Err("dictation_active_operation_deferred")
        );
        operations.finish_dictation();
        assert_eq!(operations.begin(ExclusiveOperation::ModelInstall), Ok(()));
    }

    #[test]
    fn completed_dictation_can_be_replaced_for_an_explicit_recapture() {
        let operations = OperationCoordinator::default();
        let first = SessionId::from_bytes([7; 16]);
        let second = SessionId::from_bytes([8; 16]);
        operations.begin_dictation(first).expect("first dictation");
        operations
            .replace_completed_dictation(second)
            .expect("replacement dictation");
        assert_eq!(
            operations.begin(ExclusiveOperation::ModelInstall),
            Err("dictation_active_operation_deferred")
        );
        operations.finish_dictation();
        assert_eq!(operations.begin(ExclusiveOperation::ModelInstall), Ok(()));
    }

    #[test]
    fn safe_final_scenario_applies_explicit_correction_and_final_boundary_snippet_only() {
        let mut transcript = FinalTranscript {
            session_id: SessionId::from_bytes([7; 16]),
            raw_text: "open ai met an open air pilot".to_owned(),
            text: "open ai met an open air pilot".to_owned(),
            provenance: TranscriptProvenance::FinalizedStream,
            metrics: speakeasy_domain::FinalAsrMetrics::default(),
        };
        let state = PersonalizationBundle {
            dictionary: vec![DictionaryEntry {
                id: "proper".to_owned(),
                locale: "en-US".to_owned(),
                source: "open ai".to_owned(),
                replacement: "OpenAI".to_owned(),
                case_policy: speakeasy_transforms::CasePolicy::InsensitiveCanonical,
                boundary_policy: speakeasy_transforms::BoundaryPolicy::UnicodeWord,
                origin: speakeasy_transforms::DictionaryOrigin::ExplicitCorrection,
                precedence: 100,
                protected: true,
                enabled: true,
            }],
            snippets: vec![Snippet {
                id: "sig".to_owned(),
                name: "signature".to_owned(),
                body: "Regards,\nAda".to_owned(),
                enabled: true,
            }],
            ..PersonalizationBundle::default()
        };
        apply_final_personalization(
            &mut transcript,
            state.clone(),
            "en-US",
            &WritingRulePreferences::default(),
        )
        .unwrap();
        assert_eq!(transcript.raw_text, "open ai met an open air pilot");
        assert_eq!(transcript.text, "OpenAI met an open air pilot");

        transcript.raw_text = "snippet signature".to_owned();
        transcript.text.clone_from(&transcript.raw_text);
        apply_final_personalization(
            &mut transcript,
            state,
            "en-US",
            &WritingRulePreferences::default(),
        )
        .unwrap();
        assert_eq!(transcript.text, "Regards,\nAda");
        assert!(!transcript.text.ends_with('\n'));
    }

    /// `immediate_repetitions` and `self_corrections` never run, whatever the
    /// user's writing-rule settings say.
    ///
    /// This used to assert the bypass was *engine-conditional*: off for a
    /// Granite transcript, on for a streaming one, because
    /// `resolve_self_correction` discards everything before `" I mean "` --
    /// live data loss on any transcript, and it fires more often on Granite's
    /// fluent output specifically. Every delivered transcript is Granite's now,
    /// so the condition is gone and the bypass is absolute.
    ///
    /// The test is kept, and pinned harder, precisely because the rules are
    /// unreachable: the two settings toggles that used to reach them were
    /// removed, so nothing in the UI would notice if a future change wired
    /// them back up. This would.
    #[test]
    fn the_two_destructive_cleanup_rules_never_run_even_when_settings_ask_for_them() {
        let rules = WritingRulePreferences {
            enabled: true,
            filler_words: false,
            immediate_repetitions: true,
            self_corrections: true,
            spoken_lists: false,
        };
        let transcript_with = |text: &str| FinalTranscript {
            session_id: SessionId::from_bytes([8; 16]),
            raw_text: text.to_owned(),
            text: text.to_owned(),
            provenance: TranscriptProvenance::FinalizedStream,
            metrics: speakeasy_domain::FinalAsrMetrics::default(),
        };

        let mut self_correction = transcript_with("This is what I mean is important");
        apply_final_personalization(
            &mut self_correction,
            PersonalizationBundle::default(),
            "en-US",
            &rules,
        )
        .unwrap();
        assert_eq!(
            self_correction.text, "This is what I mean is important",
            "self-correction resolution would have discarded everything before \" I mean \""
        );

        let mut repetition = transcript_with("the the cat cat sat");
        apply_final_personalization(
            &mut repetition,
            PersonalizationBundle::default(),
            "en-US",
            &rules,
        )
        .unwrap();
        assert_eq!(repetition.text, "the the cat cat sat");
    }

    #[test]
    fn the_dock_is_seated_in_from_the_edge_it_clings_to() {
        // A 1920x1080 display with a 40px taskbar along the bottom: the work
        // area is what the dock is placed against, so it is 1040 tall here.
        let work = PhysicalBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let dock_width = 130;
        let margin = edge_margin(1.0, work, dock_width);
        assert_eq!(margin, 24);

        // Both edges keep the same gap, measured from the outside in.
        assert_eq!(edge_x(work, HudDockEdge::Left, dock_width, margin), 24);
        assert_eq!(edge_x(work, HudDockEdge::Right, dock_width, margin), 1766);

        // The margin is logical, so it grows with the display's scale factor
        // rather than shrinking to a hairline on a 150% panel.
        assert_eq!(edge_margin(1.5, work, dock_width), 36);
        assert_eq!(edge_margin(2.0, work, dock_width), 48);

        // A monitor left of the primary: virtual-screen x is legitimately
        // negative and the gap must still be measured from that origin.
        let left = PhysicalBounds {
            x: -2560,
            y: -200,
            width: 2560,
            height: 1440,
        };
        let left_margin = edge_margin(1.0, left, dock_width);
        assert_eq!(edge_x(left, HudDockEdge::Left, dock_width, left_margin), -2536);
        assert_eq!(edge_x(left, HudDockEdge::Right, dock_width, left_margin), -154);
    }

    #[test]
    fn a_display_too_narrow_for_both_margins_narrows_them_rather_than_overlapping() {
        // Narrower than the dock plus two 24px margins. The margin gives way;
        // the dock does not go off-screen and the two edges do not cross.
        let cramped = PhysicalBounds {
            x: 0,
            y: 0,
            width: 160,
            height: 600,
        };
        let margin = edge_margin(1.0, cramped, 130);
        assert_eq!(margin, 15);
        assert_eq!(edge_x(cramped, HudDockEdge::Left, 130, margin), 15);
        assert_eq!(edge_x(cramped, HudDockEdge::Right, 130, margin), 15);

        // Narrower than the dock itself: `saturating_sub` floors the available
        // room at zero rather than producing a negative margin that would push
        // the window off the left of the display.
        let tiny = PhysicalBounds {
            x: 100,
            y: 100,
            width: 90,
            height: 600,
        };
        let tiny_margin = edge_margin(1.0, tiny, 130);
        assert_eq!(tiny_margin, 0);
        assert_eq!(edge_x(tiny, HudDockEdge::Left, 130, tiny_margin), 100);
        assert_eq!(edge_x(tiny, HudDockEdge::Right, 130, tiny_margin), 100);
    }

    #[test]
    fn the_dock_is_clamped_into_the_work_area_not_the_whole_display() {
        // 1080 tall display, 40px taskbar: a dock dragged to the bottom must
        // land above the taskbar, not behind it. It is `alwaysOnTop` and
        // `skipTaskbar`, so there would be nothing to click to get it back.
        let work = PhysicalBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        assert_eq!(clamp_y_to_bounds(work, 5_000, 360), 680);
        assert_eq!(clamp_y_to_bounds(work, -500, 360), 0);
        assert_eq!(clamp_y_to_bounds(work, 300, 360), 300);

        // Taller than the work area it is being placed in: still the origin
        // rather than an inverted clamp range.
        let short = PhysicalBounds {
            x: 0,
            y: 60,
            width: 1920,
            height: 200,
        };
        assert_eq!(clamp_y_to_bounds(short, 5_000, 360), 60);

        // A monitor above and left of the primary, where virtual-screen
        // coordinates are legitimately negative. This case came from the
        // deleted large-HUD placement tests and is kept here because it is not
        // specific to that window: the dock can be dragged onto exactly such a
        // monitor, and the arithmetic must neither wrap nor clamp to zero.
        let above_primary = PhysicalBounds {
            x: -2560,
            y: -200,
            width: 2560,
            height: 1440,
        };
        assert_eq!(clamp_y_to_bounds(above_primary, 100, 360), 100);
        assert_eq!(clamp_y_to_bounds(above_primary, -9_999, 360), -200);
    }

    /// Every window is named in `configure_hud`, so it can be made
    /// non-focusable at runtime.
    ///
    /// `deliver_final_text` decides where a transcript goes by inspecting the
    /// foreground window, so any window of the app's own that can activate
    /// becomes a paste target. Three separate causes have done it, and the
    /// `notice` window added on 2026-08-25 is the one most exposed to it: it is
    /// shown *during* delivery, which is the exact moment the foreground window
    /// is being read.
    ///
    /// **The declaration half lives in `tests/scaffold.test.mjs`** ("every
    /// window is declared, and none of them can take the foreground"), which
    /// asserts `focus: false` on every entry in `tauri.conf.json`. This is the
    /// half it cannot see: a window may declare `focus: false` and still need
    /// `set_focusable(false)` afterwards, and only Rust knows whether
    /// `configure_hud` reached it.
    #[test]
    fn configure_hud_reaches_every_window_that_can_show_during_a_dictation() {
        let config = include_str!("../tauri.conf.json");
        let parsed: serde_json::Value =
            serde_json::from_str(config).expect("tauri.conf.json must parse");
        let windows = parsed["app"]["windows"]
            .as_array()
            .expect("the config must declare windows");
        assert!(!windows.is_empty());
        let composition = include_str!("composition.rs");
        for window in windows {
            let label = window["label"].as_str().expect("every window has a label");
            // `main` is shown by a user action, long after any dictation, and
            // is the one window a person deliberately types into.
            if label == "main" {
                continue;
            }
            assert!(
                composition.contains(&format!("\"{label}\"")),
                "configure_hud must name {label} so it can be made non-focusable"
            );
        }
        assert!(composition.contains("set_focusable(false)"));
    }

    #[test]
    fn the_hud_poll_never_reaches_for_state_that_can_panic() {
        // `app.state::<T>()` panics when `T` is not managed, and a panic raised
        // inside a WebView callback cannot unwind, so the whole process aborts.
        // Windows declared in `tauri.conf.json` load their document before
        // `setup` finishes managing the coordinators, and two of them poll
        // `capture_hud_status` at 10 Hz — so that window is reachable, not
        // theoretical. The first installed build carrying the side dock
        // aborted on every launch with `0xc0000409` and
        // `state() called before manage() for CaptureWizardCoordinator`.
        //
        // Scoped to the poll and its helper rather than the whole file: every
        // other command in here runs from a user action, long after `setup`.
        let source = include_str!("commands/capture.rs");
        let start = source
            .find("fn capture_hud_status")
            .expect("capture_hud_status must exist");
        let end = source[start..]
            .find("\nfn ")
            .map_or(source.len(), |offset| start + offset);
        let poll = &source[start..end];
        assert!(
            !poll.contains("app.state::<") && !poll.contains(".state::<"),
            "capture_hud_status must resolve coordinators with try_state; \
             app.state::<T>() aborts the process when T is not managed yet"
        );
        assert!(poll.contains("try_state::<"));

        let helper_start = source
            .find("fn setup_requirement")
            .expect("setup_requirement must exist");
        let helper_end = source[helper_start..]
            .find("\nfn ")
            .map_or(source.len(), |offset| helper_start + offset);
        assert!(
            !source[helper_start..helper_end].contains(".state::<"),
            "setup_requirement is called from the HUD poll and must not panic either"
        );
    }

    #[test]
    fn only_the_session_controls_are_reachable_from_the_transcriber() {
        // Asserted against the source rather than trusted to review. Any command
        // that gains `require_main_or_hud_window` has to be added here
        // deliberately — that is the point of the test.
        //
        // Decision 3's clipboard prohibition is amended rather than dropped, and
        // `hud_transcript_copy` is the whole of the amendment: the transcriber may
        // copy the final it just produced. It takes no argument and resolves the
        // newest entry in Rust, so it cannot name anything else, and the
        // addressable `session_transcript_copy` stays main-only — asserted in the
        // forbidden list below so the two cannot be confused for each other.
        let sources = [
            include_str!("commands/capture.rs"),
            include_str!("commands/dictation.rs"),
            include_str!("commands/profile.rs"),
        ];
        let allowed = [
            "dictation_start",
            "dictation_stop",
            "capture_transcribe_cancel",
            "capture_devices",
            "capture_device_configure",
            "capture_wizard_status",
            "hotkey_status",
            "open_settings_window",
            "hud_transcript_copy",
            "hud_dock_placement_configure",
            "hud_dock_context_menu",
        ];

        let mut hud_reachable = Vec::new();
        let mut current_command: Option<&str> = None;
        for source in sources {
            for line in source.lines() {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed
                    .strip_prefix("fn ")
                    .or_else(|| trimmed.strip_prefix("async fn "))
                    && let Some(name) = rest.split('(').next()
                {
                    current_command = Some(name);
                }
                if trimmed.starts_with("require_main_or_hud_window(&window)")
                    && let Some(name) = current_command
                {
                    hud_reachable.push(name);
                }
            }
        }

        hud_reachable.sort_unstable();
        hud_reachable.dedup();
        let mut expected: Vec<&str> = allowed.to_vec();
        expected.sort_unstable();
        assert_eq!(
            hud_reachable, expected,
            "the transcriber's allowlist changed; the IPC schema must change with it"
        );

        // The specific authorities that must never be reachable from a
        // no-activate window, named so a regression is unmissable.
        for forbidden in [
            "result_copy",
            // The addressable transcript copy. `hud_transcript_copy` is allowed
            // above and this is not, which is exactly the line the amendment to
            // decision 3 draws: copying the last final is permitted, naming any
            // entry in the session log is not.
            "session_transcript_copy",
            "session_transcript_log",
            "history_export",
            "history_delete_all",
            "model_install_start",
            "model_remove",
            "personalization_import_commit",
            "diagnostics_export",
            "reset_commit",
            "credential_status",
            "hud_placement_reset",
        ] {
            assert!(
                !hud_reachable.contains(&forbidden),
                "{forbidden} must stay refused from the hud window"
            );
        }
    }

    /// Every id a menu item is built with is an id `dispatch_menu_action`
    /// matches.
    ///
    /// The dock's menu carried "Return to default HUD" built with the id
    /// `hud_dock_return`, and nothing handled it: the dispatcher matched
    /// `"settings"` and `"quit"` and fell through to `_ => {}`. Clicking it did
    /// nothing at all, silently, from the fork until 2026-08-27 — a control
    /// reporting success by not erroring, which is the shape this repository
    /// exists to remove. Review did not catch it and no test could, because
    /// nothing anywhere read the menu.
    ///
    /// The `_ => {}` arm is correct and stays: an unrecognised id arriving at
    /// runtime must not panic. That is exactly why the check has to be here —
    /// the arm that makes the app robust is the same arm that makes a dead id
    /// invisible, so the only place the two can be told apart is against the
    /// source.
    ///
    /// One direction only. A *handled* id nobody builds is harmless dead code;
    /// a *built* id nobody handles is a control that lies.
    #[test]
    fn every_menu_id_that_is_built_has_a_handler() {
        // The id is the first string literal inside the call: the shape is
        // `MenuItem::with_id(app, "quit", ..)` on one line and `&app,` then
        // `"settings",` across several, and both put the handle first. Bounded
        // by the call's own matching paren rather than by a line count, so a
        // literal belonging to the next statement cannot be read as an id.
        fn built_ids(source: &str) -> Vec<String> {
            let mut ids = Vec::new();
            let mut rest = source;
            while let Some(at) = rest.find("MenuItem::with_id(") {
                let open = at + "MenuItem::with_id(".len();
                let mut depth = 1usize;
                let bytes = rest.as_bytes();
                let mut end = open;
                while end < bytes.len() && depth > 0 {
                    match bytes[end] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    end += 1;
                }
                let call = &rest[open..end.min(rest.len())];
                if let Some(first) = call.find('"')
                    && let Some(len) = call[first + 1..].find('"')
                {
                    ids.push(call[first + 1..first + 1 + len].to_owned());
                }
                rest = &rest[end.min(rest.len())..];
            }
            ids
        }

        // Match-arm patterns in the dispatcher's body. A guard is allowed
        // between the pattern and the arrow (`"quit" if request_quit(app) =>`),
        // so the literal is taken from everything left of `=>`.
        fn handled_ids(source: &str) -> Vec<String> {
            let start = source
                .find("fn dispatch_menu_action")
                .expect("dispatch_menu_action must exist");
            let end = source[start..]
                .find("\nfn ")
                .map_or(source.len(), |offset| start + offset);
            let mut ids = Vec::new();
            for line in source[start..end].lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                let Some(arrow) = trimmed.find("=>") else {
                    continue;
                };
                let pattern = &trimmed[..arrow];
                if let Some(first) = pattern.find('"')
                    && let Some(len) = pattern[first + 1..].find('"')
                {
                    ids.push(pattern[first + 1..first + 1 + len].to_owned());
                }
            }
            ids
        }

        let capture = include_str!("commands/capture.rs");
        let composition = include_str!("composition.rs");

        let mut built = built_ids(capture);
        built.extend(built_ids(composition));
        built.sort_unstable();
        built.dedup();
        let handled = handled_ids(capture);

        // Instrument self-checks. Every assertion below is of the form "nothing
        // was found", and a parser that reads nothing reports exactly that — so
        // the reading is worthless until both extractors are shown to work.
        assert!(
            built.len() >= 2,
            "the id extractor found {} ids; the dock and the tray build four \
             menu items between them, so it is broken rather than the code \
             being clean",
            built.len()
        );
        assert!(
            built.contains(&"settings".to_owned()) && built.contains(&"quit".to_owned()),
            "the id extractor missed ids that are plainly in the source: {built:?}"
        );
        assert!(
            handled.contains(&"settings".to_owned()) && handled.contains(&"quit".to_owned()),
            "the dispatcher extractor missed arms that are plainly there: {handled:?}"
        );
        // And proved able to fail: the extractor must see a dead id when one is
        // present. This is the deleted `hud_dock_return` call, restored as a
        // literal so the test that replaced it can demonstrate it would have
        // caught it.
        let regression = r#"
            let return_to_default = MenuItem::with_id(
                &app,
                "hud_dock_return",
                native_catalog::HUD_DOCK_MENU_RETURN,
                true,
                None::<&str>,
            )
        "#;
        assert_eq!(
            built_ids(regression),
            vec!["hud_dock_return".to_owned()],
            "the extractor cannot see the very id this test exists to catch"
        );

        for id in &built {
            assert!(
                handled.contains(id),
                "the menu item built with the id `{id}` has no arm in \
                 dispatch_menu_action, so clicking it does nothing and reports \
                 nothing. Add an arm, or delete the item."
            );
        }
    }

    /// Setup's words become entries, and a compound also gets its spaced
    /// companion so a recogniser that heard two words is corrected.
    #[test]
    fn setup_terms_gain_a_spaced_companion_for_every_compound() {
        let terms = ["LogicMonitor", "Splunk", "PagerDuty"]
            .iter()
            .map(|term| (*term).to_owned())
            .collect::<Vec<_>>();
        let entries = protected_term_entries(&terms);

        let sources: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| (entry.source.as_str(), entry.replacement.as_str()))
            .collect();
        assert_eq!(
            sources,
            vec![
                ("LogicMonitor", "LogicMonitor"),
                ("Splunk", "Splunk"),
                ("PagerDuty", "PagerDuty"),
                ("Logic Monitor", "LogicMonitor"),
                ("Pager Duty", "PagerDuty"),
            ],
            "every term keeps its identity rule; only compounds gain a companion"
        );

        // The identity rules stay protected -- they exist to stop the finishing
        // pass rewriting a word it got right. The companions are corrections of
        // a form the user did not want, so they are not.
        for entry in &entries {
            assert_eq!(
                entry.protected,
                entry.source == entry.replacement,
                "{} has the wrong protected flag",
                entry.id
            );
            assert!(entry.enabled, "{} must be live", entry.id);
        }

        // Whatever this produces has to survive the validator it is handed to,
        // or the batch is rejected whole and the user gets nothing.
        speakeasy_transforms::DictionarySet::new(entries)
            .expect("the seeded entry set must validate");
    }

    /// The guard that matters. A derived variant colliding with a word the user
    /// actually typed is a `ConflictingRule`, and that rejects **every** entry
    /// in the batch rather than the duplicate -- which is how a user once ended
    /// up with none of their vocabulary and no error. Two spellings of the same
    /// compound is an entirely ordinary thing to type.
    #[test]
    fn a_spaced_variant_that_collides_with_a_typed_term_is_dropped_not_conflicting() {
        let terms = ["ServiceNow", "Service Now", "OpenAI"]
            .iter()
            .map(|term| (*term).to_owned())
            .collect::<Vec<_>>();
        let entries = protected_term_entries(&terms);

        let sources: Vec<&str> = entries.iter().map(|entry| entry.source.as_str()).collect();
        assert_eq!(
            sources,
            // `servenow` is the measured mishearing of `ServiceNow` and rides
            // along here; it is not part of what this test is about. What is:
            // `Service Now` appears exactly once, as the term the user typed,
            // and no second entry was derived onto the same source.
            vec!["ServiceNow", "Service Now", "OpenAI", "servenow", "Open AI"],
            "the colliding companion is dropped; the unrelated one still arrives"
        );
        assert_eq!(
            sources.iter().filter(|source| **source == "Service Now").count(),
            1,
            "a source may appear once, or the validator rejects the whole batch"
        );
        assert!(
            entries.iter().all(|entry| entry.source != "Service Now"
                || entry.replacement == "Service Now"),
            "the term the user typed keeps its own identity rule"
        );

        speakeasy_transforms::DictionarySet::new(entries)
            .expect("a colliding pair must not reach the validator as a conflict");
    }

    /// Ids are the marker `replace_user_entry_terms` uses to decide what this
    /// page owns, so a companion has to be replaced on a second install rather
    /// than orphaned under a position the new list no longer has.
    #[test]
    fn every_seeded_entry_id_is_unique_and_namespaced() {
        let terms = ["LogicMonitor", "PagerDuty", "ChatGPT"]
            .iter()
            .map(|term| (*term).to_owned())
            .collect::<Vec<_>>();
        let entries = protected_term_entries(&terms);
        let ids: std::collections::BTreeSet<&str> =
            entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len(), "duplicate id in the seeded set");
        assert!(ids.contains("installer-0"));
        assert!(ids.contains("installer-0-spaced"));
    }

    /// One term can carry several measured mishearings -- `JIRA` has both
    /// `Jura` and `Gira` -- and an id keyed on the term alone made them a
    /// `DuplicateId` that rejected the entire batch. Asserted directly, because
    /// the shape that broke was invisible until a second row for one term
    /// existed.
    #[test]
    fn a_term_with_two_measured_mishearings_gets_two_distinct_entries() {
        let entries = protected_term_entries(&["JIRA".to_owned()]);
        let corrections: Vec<(&str, &str)> = entries
            .iter()
            .filter(|entry| entry.source != entry.replacement)
            .map(|entry| (entry.source.as_str(), entry.replacement.as_str()))
            .collect();
        assert_eq!(corrections, vec![("Jura", "JIRA"), ("Gira", "JIRA")]);

        let ids: std::collections::BTreeSet<&str> =
            entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len(), "duplicate id: {ids:?}");

        speakeasy_transforms::DictionarySet::new(entries)
            .expect("two mishearings for one term must validate");
    }

    /// The failures no rule predicts, corrected because somebody measured them.
    ///
    /// Scoped to the term: a profile that does not protect `HUIT` must not
    /// rewrite anybody's `Hewitt`, because the correction is unconditional and
    /// would otherwise fire on a surname nobody asked about.
    #[test]
    fn a_measured_mishearing_is_corrected_only_for_a_term_the_profile_protects() {
        let with = protected_term_entries(&["HUIT".to_owned(), "Hellen".to_owned()]);
        let pairs: Vec<(&str, &str)> = with
            .iter()
            .map(|entry| (entry.source.as_str(), entry.replacement.as_str()))
            .collect();
        assert!(pairs.contains(&("Hewitt", "HUIT")), "{pairs:?}");
        assert!(pairs.contains(&("Helen", "Hellen")), "{pairs:?}");

        // Neither term present: neither correction, and nothing else appears.
        let without = protected_term_entries(&["Splunk".to_owned()]);
        assert_eq!(without.len(), 1, "{without:?}");
        assert_eq!(without[0].source, "Splunk");

        speakeasy_transforms::DictionarySet::new(with).expect("must validate");
    }

    /// The correction has to actually reach a transcript, not merely exist as a
    /// row. Asserted on whole strings through the real transform, and on the
    /// **verbatim** output of dictations that actually happened rather than on
    /// invented examples -- a row exists because a recogniser produced that
    /// exact string, so that string is what it has to be tested against.
    #[test]
    fn the_measured_mishearings_rewrite_a_transcript() {
        let entries = protected_term_entries(&[
            "HUIT".to_owned(),
            "Hellen".to_owned(),
            "JIRA".to_owned(),
            "ServiceNow".to_owned(),
        ]);
        let set = speakeasy_transforms::DictionarySet::new(entries).expect("must validate");

        // From the 55 s recording of 2026-08-27.
        assert_eq!(
            set.apply("the rest of the Hewitt team could follow along.", "en-US")
                .0,
            "the rest of the HUIT team could follow along."
        );
        assert_eq!(
            set.apply("Helen took the handoff at noon.", "en-US").0,
            "Hellen took the handoff at noon."
        );

        // From runs 3 and 5 of the five acceptance dictations, quoted exactly.
        assert_eq!(
            set.apply("Ellen filed it in Jura for the HUIT team.", "en-US").0,
            // `Ellen` is deliberately left alone: it is a common given name and
            // correcting it would corrupt every real Ellen. Only `Jura` moves.
            "Ellen filed it in JIRA for the HUIT team."
        );
        assert_eq!(
            set.apply("paged me about servenow this morning.", "en-US").0,
            "paged me about ServiceNow this morning."
        );

        // And from the headset runs, where `Jura` never appeared and `Gira`
        // took its place three times out of five. Both forms are asserted here
        // deliberately: the pair is the record that one term can mis-transcribe
        // two different ways depending on the microphone.
        assert_eq!(
            set.apply("Hellen filed it in Gira for the HUIT team.", "en-US")
                .0,
            "Hellen filed it in JIRA for the HUIT team."
        );
    }

    /// The rows refused on 2026-08-27, pinned so nobody adds them later without
    /// re-reading why they were refused.
    ///
    /// Each would rewrite a word somebody might legitimately say: `Ellen` and
    /// `Haley` are common given names, and a project monitor is a thing that
    /// exists. A correction is unconditional, so the cost lands on every user
    /// who says the ordinary word -- which is a different trade from `servenow`,
    /// which is not a word at all.
    #[test]
    fn the_refused_mishearings_stay_refused() {
        let entries = protected_term_entries(&[
            "Hellen".to_owned(),
            "LogicMonitor".to_owned(),
            "HUIT".to_owned(),
            "JIRA".to_owned(),
            "ServiceNow".to_owned(),
        ]);
        let set = speakeasy_transforms::DictionarySet::new(entries).expect("must validate");
        for untouched in [
            "Ellen filed the ticket.",
            "Haley filed the ticket.",
            "We ran a project monitor over the release.",
        ] {
            assert_eq!(
                set.apply(untouched, "en-US").0,
                untouched,
                "an ordinary sentence must survive the correction table"
            );
        }
    }

    /// A `heard` form the user typed as a word of its own keeps it, and the
    /// correction is dropped rather than becoming a `ConflictingRule` that
    /// rejects every entry in the batch.
    #[test]
    fn a_mishearing_colliding_with_a_typed_term_is_dropped() {
        let entries = protected_term_entries(&["HUIT".to_owned(), "Hewitt".to_owned()]);
        let sources: Vec<&str> = entries.iter().map(|entry| entry.source.as_str()).collect();
        assert_eq!(sources, vec!["HUIT", "Hewitt"], "{sources:?}");
        assert!(
            entries.iter().all(|entry| entry.source != "Hewitt"
                || entry.replacement == "Hewitt"),
            "the typed word must keep its own identity rule"
        );
        speakeasy_transforms::DictionarySet::new(entries).expect("must validate");
    }
}
