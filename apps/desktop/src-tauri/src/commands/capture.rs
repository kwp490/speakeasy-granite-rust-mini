/// The compact transcriber's single poll. Non-mutating, by contract: it reads
/// state from the other coordinators and never advances any of them.
///
/// # Every lookup here is `try_state`, and that is load-bearing
///
/// Windows declared in `tauri.conf.json` are created and load their document
/// before `setup` has finished managing the coordinators, so this command is
/// genuinely reachable while half of them are still absent. `app.state::<T>()`
/// **panics** when `T` is not managed yet, and the panic surfaces inside a
/// `WebView` callback — a function that cannot unwind — so the process aborts
/// rather than failing the one call.
///
/// That is not hypothetical. The first installed build to contain the side
/// dock crashed on every launch with `0xc0000409` and
/// `state() called before manage() for CaptureWizardCoordinator`, from this
/// function. Two windows polled it at 10 Hz then — the large transcriber's
/// `hud` and the hidden `hud-dock` — which was twice the pressure on a race
/// that a release build already loses far more often than a dev build; dev
/// serves the frontend over Vite, which is slow enough that `setup` usually
/// wins. Only `hud-dock` is left, and it polls whether or not it is shown: a
/// `visible: false` window still runs its React tree.
///
/// The composition root notes the same race where it cost less: a status
/// command lost it on the first launch after an install, and the settings page
/// it fed stayed blank until the window was reloaded. Anything reachable from
/// a window's first paint has to tolerate an unmanaged coordinator; the honest answer is
/// "still starting", which the poll retries 100ms later, and which
/// `capture_status_unavailable` already says.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_hud_status(app: tauri::AppHandle) -> Result<CaptureHudView, &'static str> {
    let (
        Some(capture),
        Some(hud),
        Some(hotkey_state),
        Some(queue),
        Some(profile),
    ) = (
        app.try_state::<CaptureWizardCoordinator>(),
        app.try_state::<CaptureHudCoordinator>(),
        app.try_state::<HotkeyCoordinator>(),
        app.try_state::<OrderedFinalizationQueue>(),
        app.try_state::<ProfileCoordinator>(),
    )
    else {
        return Err("capture_status_unavailable");
    };
    let capture_view = capture.view()?;
    let hotkey = hotkey_state.view()?;
    let setup = setup_requirement(&app, &capture)?;
    let delivery_pending = hud
        .live
        .lock()
        .map_err(|_| "capture_status_unavailable")?
        .delivery_outcome
        .is_none();

    // Through the shared helper, not a local copy of the promotion. The rule
    // that `complete` with delivery unresolved is still `finalizing` is now also
    // what stops the shortcut opening a second dictation, and two statements of
    // it would be two answers to one question -- which is how `can_start` came
    // to refuse a press the shortcut accepted.
    let session = hud_session_with_delivery(capture_view.state.as_str(), delivery_pending);
    let running = matches!(
        session,
        "starting" | "streaming" | "stopping" | "finalizing"
    );

    hud.view(HudComposition {
        session,
        level: capture.level(),
        device_diagnostic: capture_view
            .device_name
            .as_ref()
            .map_or_else(|| "not_opened".to_owned(), |_| "opened".to_owned()),
        device_name: capture_view.device_name.clone().unwrap_or_default(),
        hotkey_binding: hotkey.binding,
        hotkey_registration: hotkey.registration,
        can_start: setup.is_none() && !running,
        can_stop: capture_view.can_stop,
        setup_complete: setup.is_none(),
        setup_reason: setup.map(str::to_owned),
        elapsed_ms: capture.elapsed_ms(),
        ceiling_ms: u64::from(DICTATION_CEILING_SECONDS) * 1_000,
        queue_depth: queue.depth(),
        // Deliberately the stored preference rather than the fully resolved
        // device: resolving means enumerating Windows' capture devices, which is
        // far too expensive to do at 10 Hz. The picker already holds a device
        // list on its own slower timer and completes the same fallback there.
        preferred_device_id: profile
            .settings
            .lock()
            .ok()
            .and_then(|settings| settings.preferred_capture_device_id.clone())
            .unwrap_or_default(),
        // The HUD polls this at 10 Hz, so both of these are cached field reads
        // behind a mutex rather than anything that resolves a pack or touches
        // the filesystem.
        //
        // `warm_state`, not `engine_reason`. They answer different questions and
        // this field is documented — in `CaptureHudView` and in
        // `transcriberState.ts` — as `cold | warming | ready | <error code>`,
        // which is the vocabulary `warm_state` speaks. `engine_reason` speaks
        // pack codes (`cpu_gpu_runtime_missing`, `memory_below_granite_floor`)
        // and can never say `warming` or `ready`, so filling this from it made
        // the frontend's `ENGINE_LOADING` set unmatchable after the first poll
        // and the dock reported a loaded engine throughout the ~2 GB launch
        // warm. The pack reason is still disclosed, by `granite_warm` in the
        // log and by the Advanced page; it is simply not what this means.
        engine: app
            .try_state::<GraniteEngineCoordinator>()
            .map_or("engine_unavailable", |granite| granite.warm_state()),
        engine_device: app
            .try_state::<GraniteEngineCoordinator>()
            .map_or("granite_state_unavailable", |granite| granite.device()),
        error_code: capture_view.error_code.clone(),
    })
}

/// The next concrete thing standing between this profile and a dictation, or
/// `None` when nothing is.
///
/// Deliberately capability, not bookkeeping. This used to consult an
/// `onboarding.completed` flag as well, which was a wizard's bookkeeping rather
/// than a fact about the machine: a profile could have skipped the seven steps
/// and still have a verified model and a working microphone, and telling that
/// user "Setup needed" while their shortcut dictated perfectly well would have
/// been false. The wizard and the flag are both gone -- setup is the installer's
/// job -- and what is left is the question actually worth asking, which is
/// whether this profile can dictate right now.
///
/// On a genuine first run the model is absent, so first launch still lands on
/// Setup needed with a concrete requirement.
///
/// # Errors
///
/// Returns `capture_status_unavailable` when the model coordinator is not
/// managed yet — see `capture_hud_status`, its only caller, for why that is a
/// reachable state and why it must not be a panic.
fn setup_requirement(
    app: &tauri::AppHandle,
    capture: &CaptureWizardCoordinator,
) -> Result<Option<&'static str>, &'static str> {
    let Some(models) = app.try_state::<ModelCoordinator>() else {
        return Err("capture_status_unavailable");
    };
    if models.status_snapshot().state != "verified_on_disk" {
        return Ok(Some("model_missing"));
    }
    // Through the coordinator's cache rather than `CaptureWizardCoordinator::
    // devices` directly. This is on the 10 Hz path and that call is a full
    // WASAPI enumeration; see `has_supported_microphone`.
    if !capture.has_supported_microphone() {
        return Ok(Some("microphone_missing"));
    }
    Ok(None)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_devices(window: tauri::WebviewWindow) -> Result<Vec<CaptureDeviceView>, &'static str> {
    // Main-only since 2026-08-28. The dock held this so `MicPicker` could list
    // devices; that component was deleted, nothing in the dock's tree enumerates
    // devices any more, and authority a window no longer exercises is authority
    // it should not keep. Choosing a microphone is a Settings → Audio job, which
    // is also the only place it has a keyboard path.
    require_main_window(&window)?;
    CaptureWizardCoordinator::devices()
}

// `capture_start` and `capture_stop` are deliberately absent. They were the
// guided-test path in settings, and settings no longer starts, stops or cancels
// a dictation: there is one controller, and it is the transcriber plus the
// global shortcut. Keeping a second start path is exactly the
// two-inconsistent-controllers failure the single-controller rule exists to
// prevent — `capture_stop` stopped without delivering, so a dictation begun in
// settings silently skipped the paste that the identical action from the
// shortcut performed.
//
// What did *not* go with them is retrying a transcription that failed while the
// audio is still retained. That is recovery, not a guided test, and it lives on
// as `dictation_retry` below.

/// Input level for the Audio page's meter.
///
/// Non-mutating, and deliberately not a second way to open a microphone: `level`
/// is written by the capture loop, so it moves only while a dictation is actually
/// running. `active` is what lets the page say so instead of showing a dead bar
/// and letting the user conclude their microphone is deaf.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_level(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, CaptureWizardCoordinator>,
) -> Result<CaptureLevelView, &'static str> {
    require_main_window(&window)?;
    let view = state.view()?;
    Ok(CaptureLevelView {
        level: state.level(),
        active: view.can_stop,
        device_diagnostic: view
            .device_name
            .as_ref()
            .map_or_else(|| "not_opened".to_owned(), |_| "opened".to_owned()),
    })
}

/// Quits the app from settings, through the same graceful path the transcriber's
/// close and the tray's Quit take — including the mid-dictation confirmation.
///
/// The other compensating keyboard path (UI-GUIDE "Accessibility and input").
/// Without it the only ways out are a mouse click on the transcriber's close
/// button or the tray menu, neither of which a keyboard user can reach.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn app_quit(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), &'static str> {
    require_main_window(&window)?;
    if !request_quit(&app) {
        return Ok(());
    }
    shutdown_gracefully(&app);
    app.exit(0);
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_wizard_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, CaptureWizardCoordinator>,
) -> Result<CaptureWizardView, &'static str> {
    require_main_or_hud_window(&window)?;
    state.view()
}

/// Persists the microphone the user picked, so the shortcut path — which has no
/// UI to ask — records from the same device.
///
/// Not in the session-controls allowlist, but the behavior is required and
/// `capture_start` is named as the persistence path, which is main-only and is removed
/// once capture leaves settings. This writes exactly one preference field and
/// grants no authority the transcriber does not already have: it opens no
/// device, reads no audio, and starts nothing.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_device_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    device_id: String,
) -> Result<(), &'static str> {
    // Main-only, for the same reason as `capture_devices` above.
    require_main_window(&window)?;
    // Only a device Windows is currently offering may be stored, so a stale or
    // fabricated id cannot be persisted into the shortcut's device resolution.
    let known = CaptureWizardCoordinator::devices()?
        .into_iter()
        .any(|device| device.id == device_id && device.supported);
    if !known {
        return Err("capture_device_unavailable");
    }
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    if settings.preferred_capture_device_id.as_deref() == Some(device_id.as_str()) {
        return Ok(());
    }
    settings.preferred_capture_device_id = Some(device_id);
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    Ok(())
}

/// Starts a dictation from a button. Identical in every respect to starting it
/// from the global shortcut — same session, same debounce, same capture tap.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn dictation_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    let coordinator = app.state::<HotkeyCoordinator>();
    // No target snapshot here either — see the note in the shortcut handler. The
    // button and the shortcut share one path by decision, so they also share the
    // latency, and this command was the slower of the two to notice it.
    let Some(HotkeyAction::Start(session_id)) = coordinator.request_start() else {
        // Debounced, or a session is already running. Both are successful
        // no-ops: a second press must never open a second session.
        return Ok(());
    };
    if let Err(code) = start_dictation(&app, session_id) {
        app.state::<HotkeyCoordinator>().abandon_active_session();
        return Err(code);
    }
    Ok(())
}

/// Stops a dictation from a button, transcribes it and delivers the
/// authoritative final — the same path the shortcut takes.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn dictation_stop(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    let Some(HotkeyAction::Stop) = app.state::<HotkeyCoordinator>().request_stop() else {
        // Nothing active, or debounced. A second stop must not queue a second
        // transcription.
        return Ok(());
    };
    stop_dictation(&app)
}

/// Shows the settings workspace, creating it if it was closed.
///
/// Must never disturb an active dictation: it only touches window state.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn open_settings_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    show_settings_window(&app);
    Ok(())
}

/// Routes window close requests.
///
/// There was no window-event handler at all before this; the only exit path was
/// the tray's Quit item. Both windows now have a close button, and they mean
/// different things: closing settings puts it away, closing the transcriber
/// ends the app.
fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let tauri::WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    let app = window.app_handle().clone();
    match window.label() {
        // Settings hides, never destroys. It must not quit the app and must not
        // touch a dictation that is running while the user reads settings.
        "main" => {
            api.prevent_close();
            let _ = window.hide();
        }
        // The side dock is the primary surface, so closing it ends the app
        // rather than just that window. It shared this arm with the large
        // transcriber's `hud` window, which was a second presentation of the
        // same surface; that window left with the large HUD, and the arm named
        // it until 2026-08-28. An arm for a label no window carries reads as
        // proof the window exists.
        "hud-dock" => {
            api.prevent_close();
            if !request_quit(&app) {
                return;
            }
            shutdown_gracefully(&app);
            app.exit(0);
        }
        _ => {}
    }
}

/// Routes the tray menu's and the side dock's popup menu's clicks to the same
/// three actions, since both attach to tauri's menu API at a different point
/// (tray-specific vs. app-wide) but the ids mean the same thing either way.
fn dispatch_menu_action(app: &tauri::AppHandle, id: &str) {
    match id {
        // Neither the tray nor the dock must become the only way back to the
        // app, but both are a reasonable second route to settings.
        "settings" => show_settings_window(app),
        // The same graceful path the transcriber's own close button takes,
        // including the mid-dictation confirmation — "Close" on the dock's
        // menu and "Quit" on the tray's are genuinely the same action.
        "quit" if request_quit(app) => {
            shutdown_gracefully(app);
            app.exit(0);
        }
        _ => {}
    }
}

/// Asks before ending the app mid-dictation. Returns whether to proceed.
///
/// Never discards speech silently. The prompt is a native modal because the
/// transcriber is no-activate and a `WebView` modal cannot reliably hold
/// focus, and it defaults to "keep recording" so a stray keypress cannot
/// throw a dictation away.
fn request_quit(app: &tauri::AppHandle) -> bool {
    let dictating = app
        .state::<CaptureWizardCoordinator>()
        .view()
        .is_ok_and(|view| view.can_stop);
    if !dictating {
        return true;
    }
    let choice = confirm_destructive_action(
        native_catalog::QUIT_DURING_DICTATION_TITLE,
        native_catalog::QUIT_DURING_DICTATION_MESSAGE,
    );
    log_event(
        app,
        "quit_during_dictation",
        &[(
            "result",
            if choice == Confirmation::Proceed {
                "discarded"
            } else {
                "kept_recording"
            },
        )],
    );
    choice == Confirmation::Proceed
}

/// Releases what the app owns outside its own process before exiting.
///
/// The resident inference worker is the one that matters: it holds ~564 MB and
/// its own child bridge. The Job object kills it on close, but relying on that
/// alone leaves the teardown untested, so it is stopped explicitly first.
fn shutdown_gracefully(app: &tauri::AppHandle) {
    let capture = app.state::<CaptureWizardCoordinator>();
    if capture.view().is_ok_and(|view| view.can_stop) {
        let _ = capture.stop();
    }
    app.state::<GraniteEngineCoordinator>().shutdown();
    log_event(app, "shutdown", &[("result", "ok")]);
}

/// Brings the dock back into view.
///
/// Deliberately does not focus it: the dock is no-activate, and making it the
/// foreground window would change the delivery target the next dictation
/// pastes into.
fn show_dock(app: &tauri::AppHandle) {
    if let Some(dock) = app.get_webview_window("hud-dock") {
        let _ = dock.unminimize();
        let _ = dock.show();
    }
}

/// Shows the settings workspace, recreating it if it was destroyed.
///
/// Shared by the transcriber's gear, the tray and the single-instance handler.
/// Touches window state only: opening settings during a dictation must not
/// disturb the session.
fn show_settings_window(app: &tauri::AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
        return;
    }
    let _ = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title(native_catalog::SETTINGS_WINDOW_TITLE)
        .inner_size(880.0, 720.0)
        .min_inner_size(720.0, 560.0)
        .build();
}

/// Persists where the user dragged the side dock, snapping it flush against
/// whichever edge it landed nearest.
///
/// Takes raw physical coordinates in, which is what the deleted default HUD's
/// `hud_placement_configure` took — the frontend drag handler was written
/// against that signature and still is. What this one does that the deleted
/// command did not is decide the edge: the dock is never left floating
/// mid-screen the way that window could be.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hud_dock_placement_configure(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    x: i32,
    y: i32,
) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    let Some(dock) = app.get_webview_window("hud-dock") else {
        return Err("hud_dock_window_unavailable");
    };
    let Ok(size) = dock.outer_size() else {
        return Err("hud_dock_window_unavailable");
    };
    let monitor = dock
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| dock.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return Err("hud_dock_window_unavailable");
    };
    // The usable rectangle, not the whole display: a dock dragged low must land
    // above the taskbar rather than behind it (see `work_bounds_of`).
    let work = work_bounds_of(&monitor);

    // Nearest edge, not "which half of the monitor": compares how far the
    // window's left edge sits from the work area's left edge against how far
    // its right edge sits from the work area's right edge.
    let left_distance = i64::from(x) - i64::from(work.x);
    let right_distance =
        (i64::from(work.x) + i64::from(work.width)) - (i64::from(x) + i64::from(size.width));
    let edge = if left_distance <= right_distance {
        HudDockEdge::Left
    } else {
        HudDockEdge::Right
    };
    let snapped_x = edge_x(work, edge, size.width, edge_margin(monitor.scale_factor(), work, size.width));
    let clamped_y = clamp_y_to_bounds(work, y, size.height);
    let _ = dock.set_position(tauri::PhysicalPosition::new(snapped_x, clamped_y));

    let monitor_id = monitor.name().cloned();
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    let next = HudDockPlacement {
        edge,
        position_y: Some(clamped_y),
        monitor_id,
    };
    if settings.hud_dock == next {
        return Ok(());
    }
    settings.hud_dock = next;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    Ok(())
}

/// Pops the side dock's right-click menu at the cursor.
///
/// Both ids are the tray's — `dispatch_menu_action` already handles them, and
/// "Close" here is genuinely the same action as the tray's "Quit," just
/// relabeled for where it's clicked from.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hud_dock_context_menu(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    let settings = MenuItem::with_id(
        &app,
        "settings",
        native_catalog::HUD_DOCK_MENU_SETTINGS,
        true,
        None::<&str>,
    )
    .map_err(|_| "menu_unavailable")?;
    let close = MenuItem::with_id(
        &app,
        "quit",
        native_catalog::HUD_DOCK_MENU_CLOSE,
        true,
        None::<&str>,
    )
    .map_err(|_| "menu_unavailable")?;
    let menu = Menu::with_items(&app, &[&settings, &close]).map_err(|_| "menu_unavailable")?;
    window.popup_menu(&menu).map_err(|_| "menu_unavailable")
}

/// Transcribes the audio still retained in memory again, after a transcription
/// that failed.
///
/// The recovery half of the deleted `capture_transcribe`: a failed final pass
/// keeps the utterance retained, and throwing that away because the guided-test
/// path went with it would silently drop a capability the user has today.
///
/// Deliberately does **not** deliver. The user is looking at settings when they
/// press this, so the focused application is `SpeakEasy` itself — pasting into it
/// is not what they asked for. The final lands in the session transcript log and
/// the recoverable result, where they can copy it.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn dictation_retry(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    let audio = app.state::<CaptureWizardCoordinator>().retained_audio()?;
    let request = request_for_audio(&audio);
    let outcome = run_retained_transcription(&app, audio, request).await;
    log_event(
        &app,
        "dictation_retry",
        &[("result", outcome.as_ref().err().copied().unwrap_or("ok"))],
    );
    // No speech is not a retry failure: the recoverable result already shows
    // it as `empty` rather than `failed`, and the settings UI should not
    // report an error the backend itself does not consider one.
    match outcome {
        Ok(finalized) => {
            // `NotAttempted`, because this path deliberately does not deliver
            // -- see the doc comment above. No application received the text,
            // so there is no target classification for history to honour.
            persist_delivered_history(
                &app,
                &finalized.pending_history,
                DeliveryTarget::NotAttempted,
            );
            Ok(())
        }
        Err(code) if is_no_speech(code) => Ok(()),
        Err(code) => Err(code),
    }
}

/// Builds the ASR request for a retained utterance.
///
/// The correlation id is derived from the session id rather than generated, so
/// a line in the diagnostic log can be tied back to the dictation that wrote
/// it without the two ever being stored together.
/// Whether a failure code means "the recording held no speech".
///
/// Two spellings, and both are live. `runtime_no_speech_detected` is what the
/// runtime path has always returned; `no_speech` is
/// `FinalSourceReason::NoSpeech`'s code, which is what the engine's own verdict
/// produces. They arrive at the same match arms and mean the same thing.
///
/// This exists because silence is **not a malfunction** and the difference is
/// load-bearing three times over: no quarantine strike, no stale prior
/// transcript left standing as this dictation's result, and the ordinary
/// `complete` state rather than `failed`. Matching one spelling and not the
/// other would have reported ordinary silence as an engine fault — which is
/// exactly what happened when the verdict started returning its own codes.
fn is_no_speech(code: &str) -> bool {
    matches!(code, "runtime_no_speech_detected" | "no_speech")
}

/// A transcription that succeeded, and the history row it has not earned yet.
///
/// Carried rather than written because `secure_target` is a fact about the
/// application that received the text, and nothing here has looked at the
/// foreground window. See `DeliveryTarget`.
///
/// **This type is what keeps the ordering honest.** It is produced only by
/// `publish_successful_transcript`, and `persist_delivered_history` is the only
/// consumer of its `pending_history`, so a history row cannot exist until the
/// session log, the recoverable result and the capture state have all been
/// updated. The defect it replaces was a write that ran before any of them.
struct FinalizedDictation {
    /// The text to deliver.
    text: String,
    /// The history row, pending a target classification.
    pending_history: TranscriptResult,
}

/// The parts of a history row that come from personalization rather than from
/// the transcript, handed to `publish_successful_transcript` so that it -- and
/// nothing earlier -- can assemble one.
struct HistoryRow {
    session_id: String,
    polished_text: Option<String>,
    provenance: ResultProvenance,
}

fn request_for_audio(audio: &UtteranceAudio) -> AsrRequest {
    AsrRequest {
        correlation_id: CorrelationId::from_bytes(audio.session_id.into_bytes()),
        session_id: audio.session_id,
        language: AsrLanguage::English,
        task: AsrTask::Transcribe,
    }
}

#[allow(clippy::too_many_lines)]
async fn run_retained_transcription(
    app: &tauri::AppHandle,
    audio: UtteranceAudio,
    request: AsrRequest,
) -> Result<FinalizedDictation, &'static str> {
    let models = app.state::<ModelCoordinator>();
    let capture = app.state::<CaptureWizardCoordinator>();
    let runtime = app.state::<RuntimeWizardCoordinator>();
    let results = app.state::<ResultCoordinator>();
    let profile = app.state::<ProfileCoordinator>();
    let personalization = app.state::<PersonalizationCoordinator>();
    let operations = app.state::<OperationCoordinator>();
    let memory = SafeStandardHardwareProbe
        .probe(&models.root)
        .total_memory_bytes;
    let cancel = match runtime.begin(memory) {
        Ok(cancel) => cancel,
        Err(code) => {
            results.fail(code)?;
            capture.mark_transcription_finished(Some(code));
            operations.finish_dictation();
            return Err(code);
        }
    };
    capture.mark_finalizing();
    let diagnostic_log = profile
        .settings
        .lock()
        .is_ok_and(|settings| settings.privacy.disk_logging_enabled)
        .then(|| profile.root.join("logs").join("speakeasy.log"));

    // One engine, one pass, one verdict. This used to be a fork: Granite ran,
    // and anything short of a plausible transcript fell through to re-running
    // the retained audio on the streaming engine, with the Granite failure
    // disclosed alongside text that still arrived. There is no second engine
    // to fall through to, so a rejected pass ends the dictation and its reason
    // becomes the error the user is shown rather than a footnote on a delivery.
    let outcome: Result<FinalTranscript, &'static str> = {
        let granite_worker_exe = runtime.paths().ok().map(|paths| paths.granite_worker);
        let pass = run_granite_final_pass(
            GraniteEnvironment {
                granite_worker_exe: granite_worker_exe.as_deref(),
                install_root: &models.root.join("models"),
                // The same probe `runtime.begin` above was gated on, reused
                // rather than re-taken: two reads a few lines apart could
                // disagree, and then the dictation and the pass would be
                // answering different questions about the same machine.
                total_memory_bytes: memory,
                diagnostic_log: diagnostic_log.clone(),
                // What setup proved it installed. A dictation's own warm can be
                // the first one of a process -- the launch warm is best-effort --
                // so the comparison has to be available here too, or a machine
                // whose startup warm failed would never notice the mismatch.
                recorded_provider: installed_configuration(&profile.root),
                // As at the launch warm, and it has to be the same answer: a
                // dictation's own warm can be the first of a process, so this is
                // a second composition-root site rather than a second decision.
                cuda_context_probe: &speakeasy_models::NvmlCudaContextProbe,
            },
            &app.state::<GraniteEngineCoordinator>(),
            audio.clone(),
            request,
            cancel.clone(),
        )
        .await;
        let verdict = judge_granite_pass(pass);
        match verdict.delivered {
            Some(transcript) => Ok(transcript),
            // `reason` is `Some` exactly when `delivered` is `None`, so the
            // fallback arm is unreachable by construction. It is spelled out
            // rather than unwrapped because an unreachable arm that is wrong
            // about which invariant holds is how a panic reaches a user.
            None => Err(verdict
                .reason
                .map_or("granite_failed", FinalSourceReason::code)),
        }
    };

    runtime.finish();
    if matches!(
        &outcome,
        Err(code)
            if matches!(
                *code,
                "runtime_adapter_failed"
                    | "runtime_deadline_exceeded"
                    | "runtime_stale_response"
                    | "runtime_worker_out_of_memory"
            )
    ) {
        runtime.record_worker_failure();
    }
    operations.finish_dictation();
    // The reason travels to the diagnostics surface before anything else
    // happens to the outcome, so a failure that also fails to clean up still
    // leaves the user something to read. Cleared on success in the same call.
    app.state::<DiagnosticsRuntimeCoordinator>()
        .record_final_source(outcome.as_ref().err().copied());
    match outcome {
        Ok(mut transcript) => {
            let (locale, rules) = {
                let settings = profile
                    .settings
                    .lock()
                    .map_err(|_| "profile_state_unavailable")?;
                (settings.locale.clone(), settings.writing_rules.clone())
            };
            let personalization_state = personalization
                .repository
                .lock()
                .map_err(|_| "personalization_state_unavailable")?
                .state()
                .clone();
            apply_final_personalization(&mut transcript, personalization_state, &locale, &rules)?;
            let provenance = match transcript.provenance {
                TranscriptProvenance::FinalizedStream => ResultProvenance::FinalizedStream,
                TranscriptProvenance::LastValidDraft => ResultProvenance::LastValidDraft,
            };
            let polished_text =
                (transcript.text != transcript.raw_text).then(|| transcript.text.clone());
            let session_id = transcript.session_id.into_bytes().iter().fold(
                String::new(),
                |mut output, byte| {
                    let _ = write!(output, "{byte:02x}");
                    output
                },
            );
            // The final is deliberately not published to the HUD here. The
            // transcriber must not say what happened to the text before
            // delivery has resolved, so `deliver_final_text` publishes it
            // together with the outcome; until then the HUD keeps reporting
            // `finalizing`.
            publish_successful_transcript(
                &app.state::<SessionTranscriptCoordinator>(),
                &results,
                &capture,
                transcript,
                HistoryRow {
                    session_id,
                    polished_text,
                    provenance,
                },
            )
        }
        // Silence is not a malfunction, so it must not read as one: no
        // quarantine strike (the string is absent from the quarantine list
        // above), no stale prior transcript left standing as if it were this
        // dictation's result, and the capture panel reports the ordinary
        // `complete` state rather than `failed`.
        Err(code) if is_no_speech(code) => {
            results.clear()?;
            capture.mark_transcription_finished(None);
            Err(code)
        }
        Err(code) => {
            results.fail(code)?;
            capture.mark_transcription_finished(Some(code));
            Err(code)
        }
    }
}

/// Puts a finished transcript everywhere it has to be before delivery is tried.
///
/// Its own function so a test can drive the real coordinators without a
/// `tauri::AppHandle`, and so the ordering is one statement rather than a
/// stretch of a 200-line match arm. The order is the point: **nothing fallible
/// and optional may run before this**. Until 2026-08-28 a history-database write
/// did, with `?`, so a `SQLite` error discarded the transcript before the
/// session log ever saw it and skipped `mark_transcription_finished`, latching
/// the dock on `finalizing` for the life of the process.
///
/// The session log records the authoritative final before delivery deliberately:
/// a refused paste must still leave the text somewhere the user can reach it.
///
/// # Errors
///
/// Returns `empty_result_rejected` or a coordinator-state code from
/// `ResultCoordinator::accept`. Both are genuine failures to publish, unlike a
/// history write, which is why they are still allowed to propagate.
fn publish_successful_transcript(
    session_log: &SessionTranscriptCoordinator,
    results: &ResultCoordinator,
    capture: &CaptureWizardCoordinator,
    transcript: FinalTranscript,
    row: HistoryRow,
) -> Result<FinalizedDictation, &'static str> {
    let final_text = transcript.text.clone();
    let pending_history = TranscriptResult {
        session_id: row.session_id,
        created_unix_ms: i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(i64::MAX),
        raw_text: transcript.raw_text.clone(),
        polished_text: row.polished_text,
        provenance: row.provenance,
        // A placeholder. `persist_delivered_history` fills it from what
        // delivery observed, and it is the only thing that may.
        secure_target: false,
    };
    session_log.record(
        transcript.session_id,
        &final_text,
        match transcript.provenance {
            TranscriptProvenance::FinalizedStream => "finalized_stream",
            TranscriptProvenance::LastValidDraft => "last_valid_draft",
        },
    );
    results.accept(transcript)?;
    capture.mark_transcription_finished(None);
    Ok(FinalizedDictation {
        text: final_text,
        pending_history,
    })
}

/// Applies the dictionary, snippets and writing rules to an accepted final.
///
/// Two cleanup rules are forced off here and are not the user's to enable.
/// `immediate_repetitions` and `self_corrections` were always disabled for a
/// Granite transcript: `resolve_self_correction` discards everything before
/// `" I mean "`, which is live data loss on any transcript, and it fires more
/// often on Granite's fluent output specifically than on a transducer's. That
/// used to be a per-engine decision carried in an `is_granite` flag, because a
/// streaming transcript could still be delivered and did want them. Every
/// delivered transcript is Granite's now, so the flag was always true, the two
/// rules are unreachable, and their settings toggles were removed rather than
/// left on screen doing nothing.
fn apply_final_personalization(
    transcript: &mut FinalTranscript,
    state: PersonalizationBundle,
    locale: &str,
    rules: &WritingRulePreferences,
) -> Result<(), &'static str> {
    let pipeline = TransformPipeline::new(
        DictionarySet::new(state.dictionary).map_err(|_| "personalization_invalid")?,
        SnippetSet::new(state.snippets).map_err(|_| "personalization_invalid")?,
    );
    transcript.text = pipeline
        .apply_with_cleanup(
            PipelineRequest {
                text: &transcript.text,
                locale,
                mode: PipelineMode::PlainText,
                utterance_final: true,
            },
            RuleCleanupConfig {
                mode: if rules.enabled {
                    RuleCleanupMode::Conservative
                } else {
                    RuleCleanupMode::Off
                },
                filler_words: rules.filler_words,
                immediate_repetitions: false,
                self_corrections: false,
                spoken_lists: rules.spoken_lists,
            },
        )
        .text;
    Ok(())
}


/// Hides the capture-limit notice.
///
/// Callable only from the notice itself, which is the only window that has a
/// reason to: it dismisses on its own timer and on its own button, and nothing
/// else should be able to take a warning off the user's screen.
///
/// Hidden rather than closed, for the reason every window here is: a closed
/// window has to be rebuilt to be shown again, and building one from a command
/// handler deadlocks the whole app's IPC.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_notice_dismiss(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    (window.label() == "notice")
        .then_some(())
        .ok_or("window_not_authorized")?;
    let notice = app
        .get_webview_window("notice")
        .ok_or("capture_notice_window_unavailable")?;
    notice.hide().map_err(|_| "window_operation_refused")
}

/// Shows the pinned transcript-log window.
///
/// Declared in `tauri.conf.json` and shown here, never built on demand: a
/// command handler runs off the main thread and `WebviewWindowBuilder::build()`
/// from there deadlocks every command in the app, not just this one.
///
/// It is not focused, and that is not a detail. `deliver_final_text` inspects
/// the foreground window to decide where a transcript goes, so a log window
/// that took the foreground would become the paste target for the next
/// dictation — which does not error, it refuses with `target_inspect_refused`
/// and falls back to the clipboard, and reads as a delivery bug somewhere else
/// entirely. `configure_hud` already called `set_focusable(false)` on it at
/// startup; showing a non-focusable window does not activate it.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn transcript_log_pin(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    let log = app
        .get_webview_window("log")
        .ok_or("transcript_log_window_unavailable")?;
    log.show().map_err(|_| "window_operation_refused")
}

/// Hides the pinned transcript-log window.
///
/// Reachable from the log window itself, which is how its own close button and
/// right-click work, as well as from settings. Hidden rather than closed: a
/// closed window cannot be reopened without building one, which is the
/// deadlock above.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn transcript_log_unpin(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_or_log_window(&window)?;
    let log = app
        .get_webview_window("log")
        .ok_or("transcript_log_window_unavailable")?;
    log.hide().map_err(|_| "window_operation_refused")
}
