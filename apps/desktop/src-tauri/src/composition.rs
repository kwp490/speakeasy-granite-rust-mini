macro_rules! desktop_handler {
    ($($command:ident),+ $(,)?) => {{ tauri::generate_handler![$($command),+] }};
}

/// Starts the desktop composition root and blocks until it exits.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or run the desktop shell.
#[allow(clippy::too_many_lines)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Launching SpeakEasy again means "give me SpeakEasy", and after the
            // startup flip that is the transcriber, not the workspace. This used
            // to show `main`: relaunching while the transcriber was minimized
            // left it minimized and popped settings the user had not asked for.
            show_dock(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(HotkeyCoordinator::default());
    let builder = builder.setup(|app| {
        let app_root = app.path().app_data_dir()?;
        migrate_legacy_startup(&std::env::current_exe()?)
            .map_err(|_| "startup_migration_failed")?;
        let root = app_root.join("model-lifecycle");
        let resource_root = app.path().resource_dir()?;
        // One binary, and it is Granite's. This list used to name the streaming
        // worker and its two ONNX Runtime DLLs, which do not exist here.
        //
        // The CUDA redistributables are deliberately absent: a CPU install is a
        // complete install, so requiring them would fail the health check on
        // every machine that chose CPU. Whether the *installed configuration*
        // can actually run is a different question, asked against what setup
        // recorded rather than against this list.
        let required_resources = [resource_root.join("proof/granite-worker.exe")];
        clear_pending_update_after_health_checks(
            &app_root,
            env!("CARGO_PKG_VERSION"),
            &std::env::current_exe()?,
            &required_resources,
        )
        .map_err(|_| "pending_update_health_check_failed")?;
        let profile = ProfileCoordinator::new(app_root.clone());
        let mut profile_settings = profile
            .settings
            .lock()
            .map_err(|_| "profile_state_unavailable")?
            .clone();
        let hotkey_seeded = consume_installer_hotkey_seed(&app_root, &mut profile_settings);
        let logging_seeded = consume_installer_logging_seed(&app_root, &mut profile_settings);
        let retention_seeded = consume_installer_retention_seed(&app_root, &mut profile_settings);
        // Read here with the other seeds so every one of them is consumed in
        // one pass, but applied further down: dictionary entries are not
        // `Settings`, and the coordinator that owns them does not exist yet.
        let installer_vocabulary = consume_installer_vocabulary_seed(&app_root);
        if hotkey_seeded || logging_seeded || retention_seeded {
            let _ = profile.save(&profile_settings);
            *profile
                .settings
                .lock()
                .map_err(|_| "profile_state_unavailable")? = profile_settings.clone();
        }
        apply_hotkey_preferences(&app.state::<HotkeyCoordinator>(), &profile_settings)?;
        app.manage(OperationCoordinator::default());
        app.manage(CaptureHudCoordinator::default());
        app.manage(CaptureWizardCoordinator::default());
        let finalization_app = app.handle().clone();
        let finalization_queue = OrderedFinalizationQueue::new(
            speakeasy_worker::DEFAULT_FINALIZATION_QUEUE_CAPACITY,
            move |job| process_finalization_job(&finalization_app, job),
        )
        .map_err(|_| "finalization_queue_unavailable")?;
        app.manage(finalization_queue);
        let runtime = RuntimeWizardCoordinator::new(resource_root);
        // Startup readiness resolves the pack a dictation would load, and on a
        // CUDA-capable machine that depends on whether a CUDA-capable Granite
        // worker exists. No worker has been asked yet, so this takes the same
        // conservative assumption `warm_granite_if_configured`'s first selection
        // takes; `ModelCoordinator::settle_after_warm` re-resolves with the
        // worker's own answer once the warm has spoken.
        let granite = GraniteEngineCoordinator::default();
        let models = ModelCoordinator::new(root, granite.cuda_worker_available());
        app.manage(models);
        app.manage(GpuQualificationCoordinator::default());
        // Managed here rather than further down, ahead of the coordinators
        // below that open files: the settings page fires its startup reads
        // concurrently, and every statement between `setup` beginning and this
        // being managed is a window in which a read of it fails. That is not
        // hypothetical — a status command lost exactly this race on the first
        // launch after an install, and the page it fed stayed blank until the
        // window was reloaded.
        app.manage(runtime);
        let history = HistoryCoordinator::new(&app_root, &profile_settings);
        // Seeded before either is managed, so the first `session_transcript_log`
        // a window can fire already sees the retained entries. The settings page
        // reads on mount and would otherwise render an empty log once and fill
        // it a poll later, which reads as "my retained transcripts are gone".
        let session_transcripts = SessionTranscriptCoordinator::default();
        if profile_settings.privacy.persisted_history_enabled {
            session_transcripts.seed_from_history(&history.stored(SESSION_TRANSCRIPT_LIMIT));
        }
        app.manage(session_transcripts);
        app.manage(history);
        let personalization = PersonalizationCoordinator::new(&app_root)?;
        // Applied before the coordinator is managed, so the settings page's
        // first read already sees them. Failure is deliberately not fatal: the
        // words are a convenience the user can retype, and refusing to start
        // the app over a rejected term would be the wrong trade by a distance.
        //
        // Not fatal is not the same as not reported, and this used to be a bare
        // `let _ =`. A rejected batch is all-or-nothing, so the user's symptom
        // was an empty word list in Settings with nothing in the log, nothing on
        // screen, and no way to tell it from having typed nothing — which is how
        // it survived until 2026-08-20. The count goes in the line too: an
        // installer that collected five words and applied five is the claim
        // being made, and a count is what makes it checkable.
        if !installer_vocabulary.is_empty() {
            let outcome = personalization.add_protected_terms(&installer_vocabulary);
            log_startup_event(
                &app_root,
                profile_settings.privacy.disk_logging_enabled,
                "installer_vocabulary",
                &[
                    ("count", &installer_vocabulary.len().to_string()),
                    ("result", outcome.err().unwrap_or("applied")),
                ],
            );
        }
        app.manage(personalization);
        app.manage(profile);
        app.manage(WindowsCredentialManager::default());
        app.manage(granite);
        app.manage(DiagnosticsRuntimeCoordinator::default());
        app.manage(ResultCoordinator::default());
        app.manage(TargetObserver::spawn().map_err(|_| "target_observer_unavailable")?);
        app.manage(ClipboardWriter::spawn().map_err(|_| "clipboard_writer_unavailable")?);
        app.manage(CommitWriter::spawn().map_err(|_| "commit_writer_unavailable")?);
        configure_hud(app)?;
        // Shared by the tray's own menu-event hook below and the side dock's
        // popup menu (`hud_dock_context_menu`), which dispatches through the
        // app-wide handler rather than a tray-specific one — the two are
        // different attachment points in tauri's menu API, but the same two
        // ids should mean the same thing regardless of where they were
        // clicked.
        app.on_menu_event(|app, event| dispatch_menu_action(app, event.id().as_ref()));
        let settings = MenuItem::with_id(
            app,
            "settings",
            native_catalog::TRAY_SETTINGS,
            true,
            None::<&str>,
        )?;
        let quit = MenuItem::with_id(app, "quit", native_catalog::TRAY_QUIT, true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&settings, &quit])?;
        // Tauri does not default the tray to the app icon, and `tray_icon`
        // registers the shell entry *without* `NIF_ICON` when none is supplied
        // rather than failing — so the notification area drew an empty cell that
        // still occupied a slot. Measured on this host: with SpeakEasy running
        // the overflow flyout gained a blank first cell and shifted every other
        // icon along one place; stopping the app put them back. Nothing errored
        // and the menu worked, which is why this survived so long.
        //
        // The icon comes from the windows' own default rather than a second read
        // of `icons/icon.ico`, so the tray can never disagree with the taskbar.
        let mut tray = TrayIconBuilder::new()
            .tooltip(native_catalog::TRAY_TOOLTIP)
            .menu(&menu)
            // The tray must never become the only way back to the app, but it
            // is a reasonable second route to settings.
            .on_menu_event(|app, event| dispatch_menu_action(app, event.id().as_ref()));
        if let Some(icon) = app.default_window_icon().cloned() {
            tray = tray.icon(icon);
        }
        tray.build(app)?;
        // A conflicting or unavailable binding is reported through
        // `hotkey_status` and must not stop the desktop from starting.
        let _ = register_activation_hotkey(&app.handle().clone());
        warm_granite_engine(&app.handle().clone());
        Ok(())
    });
    let builder = builder.invoke_handler(desktop_handler![
        profile_status,
        personalization_status,
        correction_record,
        snippet_save,
        personalization_delete,
        personalization_import_preview,
        personalization_import_commit,
        personalization_export,
        personalization_reset,
        history_configure,
        disk_logging_configure,
        delivery_configure,
        recording_feedback_configure,
        history_export,
        history_delete_all,
        startup_configure,
        credential_status,
        reset_preview,
        reset_commit,
        diagnostics_status,
        diagnostics_export,
        model_catalog,
        model_hardware,
        gpu_status,
        gpu_retest,
        model_install_start,
        model_install_cancel,
        model_install_status,
        model_remove,
        capture_hud_status,
        capture_devices,
        capture_wizard_status,
        capture_device_configure,
        capture_level,
        capture_audio_snapshot,
        app_quit,
        dictation_start,
        dictation_stop,
        dictation_retry,
        open_settings_window,
        hud_dock_placement_configure,
        hud_dock_context_menu,
        transcript_log_pin,
        transcript_log_unpin,
        capture_notice_dismiss,
        capture_transcribe_cancel,
        runtime_recover,
        result_status,
        result_copy,
        session_transcript_log,
        session_transcript_copy,
        hud_transcript_copy,
        hotkey_status,
        hotkey_configure,
    ]);
    builder
        .on_window_event(on_window_event)
        .build(tauri::generate_context!())
        .expect("failed to build the SpeakEasy desktop shell")
        .run(|_app, _event| {});
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PhysicalBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn configure_hud(app: &mut tauri::App) -> tauri::Result<()> {
    // Declared in `tauri.conf.json`, never built on demand: a command handler
    // runs off the main thread, and building a webview window from there
    // deadlocked the whole app's IPC in testing — every other command stopped
    // responding along with the one that tried it, because
    // `WebviewWindowBuilder::build()` blocks its caller on the main thread
    // servicing the request, and something about that path never returns when
    // called this way. So every window this app will ever show exists from
    // launch, and the only thing a command does is show or hide one.
    let Some(dock) = app.get_webview_window("hud-dock") else {
        return Ok(());
    };
    // Every one of the app's on-top windows is non-focusable, and this is not
    // cosmetic. `deliver_final_text` inspects the foreground window to decide
    // where a transcript goes, so any window of SpeakEasy's own that lands
    // there hijacks the dictation — it does not error, it refuses with
    // `target_inspect_refused` and falls back to the clipboard, which reads as
    // a delivery bug somewhere else entirely.
    //
    // `notice` joined them on 2026-08-25 and is the one that would have found
    // this out the hard way: it is shown *during* a dictation's delivery, which
    // is the exact moment the foreground window is being read.
    // `configure_hud_reaches_every_window_that_can_show_during_a_dictation`
    // pins the list against the config so a fourth window cannot be added
    // without arriving here.
    dock.set_focusable(false)?;
    enforce_declared_size(app, &dock, "hud-dock");
    for label in ["log", "notice"] {
        if let Some(window) = app.get_webview_window(label) {
            window.set_focusable(false)?;
            enforce_declared_size(app, &window, label);
        }
    }
    let saved = app
        .state::<ProfileCoordinator>()
        .settings
        .lock()
        .ok()
        .map(|settings| settings.hud_dock.clone());
    place_hud_dock(&dock, saved);
    dock.show()?;
    Ok(())
}

// ── The side dock ────────────────────────────────────────────────────────
// The app's only HUD: a narrow card that clings to a screen edge rather than
// floating mid-screen. It was a second presentation of the compact transcriber
// until the fork deleted that window. Declared in `tauri.conf.json` (96x360
// logical, `visible: false`), which is why its own size lives there rather than
// as a Rust constant.

/// Re-applies the dock's declared size, because creating the window at that
/// size does not produce it.
///
/// Windows clamps a window to the default minimum *tracking* size while it is
/// being created — the dock has `WS_CAPTION` even with `decorations: false`,
/// which is what brings that clamp in — so the declared width is silently
/// widened and nothing anywhere reports it. Measured on this machine: 60
/// declared came back 129.6 logical, and so did 96, and so did 96 with a
/// matching `minWidth`. The stylesheet's card arithmetic then describes a
/// window that does not exist, which is exactly the class of failure that is
/// invisible until someone measures the running window.
///
/// A `set_size` *after* creation is not subject to that clamp and holds — 96
/// logical, with the webview reflowing to match. So the size is declared once
/// in `tauri.conf.json` and asserted here rather than being duplicated as a
/// Rust constant; the config is read back so the two cannot drift.
///
/// Runs in `configure_hud`, before anything shows the dock, so the resize is
/// never on screen. It also has to precede `place_hud_dock`, which reads
/// `outer_size()` to work out the edge offset — seated against the old width
/// the dock would sit 34 logical px further out than it asked for.
fn enforce_declared_size(app: &tauri::App, window: &tauri::WebviewWindow, label: &str) {
    let Some(declared) = app
        .config()
        .app
        .windows
        .iter()
        .find(|declared| declared.label == label)
    else {
        return;
    };
    let _ = window.set_size(tauri::LogicalSize::new(declared.width, declared.height));
}

/// Restores the whole-number coercion the deleted transcriber placement used
/// to own.
///
/// Window coordinates are computed in `i64` because a multi-monitor virtual
/// desktop can put a monitor's origin far from zero and the intermediate sums
/// overflow `i32` long before any real coordinate does. Saturating rather than
/// wrapping matters: a wrapped coordinate places a window on the opposite side
/// of the desktop, which looks like a placement bug rather than an arithmetic
/// one.
fn saturating_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value.is_negative() {
        i32::MIN
    } else {
        i32::MAX
    })
}

/// How far the dock sits from the edge it clings to, in logical pixels.
///
/// Not zero. Flush against the edge, the dock's rounded corners and drop
/// shadow are clipped by the screen boundary and it reads as a panel welded to
/// the display rather than a card floating over it. It also sits exactly where
/// Windows' own edge gestures live — a maximised window's snap target, and the
/// auto-hide taskbar's reveal strip — so a control there is one the OS
/// intercepts first.
const DOCK_EDGE_MARGIN: f64 = 24.0;

/// The monitor's usable rectangle: the full display minus whatever the shell
/// has reserved, which on Windows is the taskbar.
///
/// The dock is placed against this rather than against the monitor's full
/// bounds, so a dock dragged to the bottom of the screen cannot end up
/// underneath the taskbar — a window that is `alwaysOnTop` and `skipTaskbar`
/// has no entry to click and no way back if it lands there.
fn work_bounds_of(monitor: &tauri::window::Monitor) -> PhysicalBounds {
    let area = monitor.work_area();
    PhysicalBounds {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    }
}

/// `DOCK_EDGE_MARGIN` in physical pixels at `scale_factor`, narrowed on a
/// display too small to seat the dock and both margins.
///
/// Takes the scale factor rather than the `Monitor` it comes from so the
/// arithmetic is reachable from a unit test — `Monitor` cannot be constructed
/// without a running event loop.
fn edge_margin(scale_factor: f64, work: PhysicalBounds, dock_width: u32) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let requested = (DOCK_EDGE_MARGIN * scale_factor).round() as i64;
    let available = i64::from(work.width.saturating_sub(dock_width)) / 2;
    requested.clamp(0, available.max(0))
}

/// The physical x that seats a `dock_width`-wide window `margin` in from
/// `edge`.
fn edge_x(work: PhysicalBounds, edge: HudDockEdge, dock_width: u32, margin: i64) -> i32 {
    match edge {
        HudDockEdge::Left => saturating_i32(i64::from(work.x) + margin),
        HudDockEdge::Right => saturating_i32(
            i64::from(work.x) + i64::from(work.width.saturating_sub(dock_width)) - margin,
        ),
    }
}

/// Keeps a `dock_height`-tall window's y inside `work`, the same clamp the
/// deleted default HUD's `clamp_to_bounds` did for its y — factored out
/// because the dock's x is never clamped this way, it is snapped to an edge
/// instead.
fn clamp_y_to_bounds(work: PhysicalBounds, y: i32, dock_height: u32) -> i32 {
    let maximum_y = i64::from(work.y) + i64::from(work.height.saturating_sub(dock_height));
    saturating_i32(i64::from(y).clamp(i64::from(work.y).min(maximum_y), maximum_y))
}

/// Places the side dock, restoring `saved`'s edge and y when its monitor is
/// still present and falling back to the right edge, vertically centered,
/// when it is not.
///
/// The x axis is never restored verbatim, the way the deleted large HUD's
/// placement restored its own: the dock is always snapped flush against
/// `saved.edge`, so a monitor resize cannot leave it floating mid-screen the
/// way a raw stored x could.
fn place_hud_dock(dock: &tauri::WebviewWindow, saved: Option<HudDockPlacement>) {
    let Ok(size) = dock.outer_size() else { return };
    let monitors = dock.available_monitors().unwrap_or_default();

    if let Some(saved) = saved
        && let Some(y) = saved.position_y
        && let Some(monitor) = saved
            .monitor_id
            .as_ref()
            .and_then(|name| monitors.iter().find(|monitor| monitor.name() == Some(name)))
    {
        let work = work_bounds_of(monitor);
        let x = edge_x(work, saved.edge, size.width, edge_margin(monitor.scale_factor(), work, size.width));
        let clamped_y = clamp_y_to_bounds(work, y, size.height);
        let _ = dock.set_position(tauri::PhysicalPosition::new(x, clamped_y));
        return;
    }

    let default_monitor = dock
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next());
    if let Some(monitor) = default_monitor {
        let work = work_bounds_of(&monitor);
        let x = edge_x(
            work,
            HudDockEdge::Right,
            size.width,
            edge_margin(monitor.scale_factor(), work, size.width),
        );
        let centered_y =
            saturating_i32(i64::from(work.y) + i64::from(work.height.saturating_sub(size.height)) / 2);
        let _ = dock.set_position(tauri::PhysicalPosition::new(x, centered_y));
    }
}

