// Dictation lifecycle commands: cancel, recovery, results, the transcript
// log, and the global hotkey.
//
// This file used to open with the delivered-transcript pass itself -- the
// resident streaming worker, its spawn-a-fresh-one fallback, and a
// `transcribe-cpp` canary behind a feature flag, some 360 lines of choosing
// between engines and retrying across processes. All of it existed because
// there were two engines and the streaming one was the fallback when Granite
// did not deliver. There is one engine now, `granite_engine.rs` owns running
// it, and a Granite pass that fails is the end of the dictation rather than
// the start of a second attempt on a weaker engine.

/// Abandons the dictation in progress without transcribing or delivering it.
///
/// This was `runtime.cancel()` and nothing else, which only cancelled an
/// inference that was already in flight. Pressed during the recording — where
/// the button spends most of its life — it returned `runtime_not_active`, the
/// transcriber swallowed the error as designed, and the recording carried on
/// with the timer running. Cancel has to reach whichever stage is actually live.
///
/// Both stages are cancelled rather than one or the other: a press that lands in
/// the gap between capture ending and inference starting must not leave either
/// of them to finish on its own. Neither call failing is an error here — between
/// them they cover every stage, so at most one can succeed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn capture_transcribe_cancel(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), &'static str> {
    require_main_or_hud_window(&window)?;
    let capture_stopped = app.state::<CaptureWizardCoordinator>().cancel()?;
    let inference_stopped = app.state::<RuntimeWizardCoordinator>().cancel().is_ok();
    // The activation session and the exclusive-operation slot are held until a
    // stop releases them, and no stop is coming. Without this the next press is
    // read as a Stop that finds nothing to stop, which is the same stuck state
    // the ceiling watcher exists to prevent.
    app.state::<HotkeyCoordinator>().abandon_active_session();
    app.state::<OperationCoordinator>().finish_dictation();
    // In-progress hypotheses must not outlive the dictation as if they were a
    // result the user could act on.
    app.state::<CaptureHudCoordinator>().abandon();
    log_event(
        &app,
        "dictation_cancel",
        &[(
            "stage",
            if capture_stopped {
                "capture"
            } else if inference_stopped {
                "inference"
            } else {
                "already_over"
            },
        )],
    );
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn runtime_recover(
    window: tauri::WebviewWindow,
    runtime: tauri::State<'_, RuntimeWizardCoordinator>,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    runtime.recover_manually()
}

const fn domain_error_code(error: &DomainError) -> &'static str {
    match error.code {
        ErrorCode::Cancelled => "runtime_cancelled",
        ErrorCode::DeadlineExceeded => "runtime_deadline_exceeded",
        ErrorCode::StaleEvent => "runtime_stale_response",
        ErrorCode::InvalidData => "runtime_invalid_data",
        ErrorCode::InvalidTransition => "runtime_invalid_transition",
        ErrorCode::QueueFull => "runtime_queue_full",
        ErrorCode::Unauthorized => "runtime_unauthorized",
        ErrorCode::TooNew => "runtime_too_new",
        ErrorCode::AppNotReady => "runtime_not_ready",
        ErrorCode::SessionAlreadyActive => "runtime_busy",
        ErrorCode::AdapterFailed => "runtime_adapter_failed",
        ErrorCode::NoSpeechDetected => "runtime_no_speech_detected",
        ErrorCode::EngineQuarantined => "runtime_engine_quarantined",
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn result_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ResultCoordinator>,
    capture: tauri::State<'_, CaptureWizardCoordinator>,
) -> Result<RecoverableResultView, &'static str> {
    require_main_window(&window)?;
    let mut view = state.view()?;
    view.retry_available = capture.has_retained_audio();
    Ok(view)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn result_copy(
    window: tauri::WebviewWindow,
    results: tauri::State<'_, ResultCoordinator>,
    clipboard: tauri::State<'_, ClipboardWriter>,
) -> Result<u32, &'static str> {
    require_main_window(&window)?;
    let (session_id, text) = results.copy_payload()?;
    clipboard
        .write_result(session_id, text)
        .map_err(|_| "clipboard_write_failed")?
        .clipboard_sequence
        .ok_or("clipboard_sequence_unavailable")
}

/// The listed transcripts, newest first.
///
/// Window-guarded, like every command that can see transcript text: `main` and
/// the pinned `log`, and nothing else. This is the **only** way a window obtains
/// transcript text — `transcript-log-changed` says the list moved and carries
/// nothing, so an event listener still has to come through here.
///
/// Not "this session's": the list is seeded at launch from the optional on-disk
/// history. See `SessionTranscriptCoordinator`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn session_transcript_log(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, SessionTranscriptCoordinator>,
) -> Result<Vec<SessionTranscriptEntryView>, &'static str> {
    require_main_or_log_window(&window)?;
    state.log()
}

/// Copies one session-log entry to the clipboard.
///
/// Main-only and deliberately so: clipboard authority stays out of the
/// transcriber. The text is fetched in Rust from the id, so the window never
/// hands text back to be written — it can only name an entry that the backend
/// already holds.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn session_transcript_copy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, SessionTranscriptCoordinator>,
    clipboard: tauri::State<'_, ClipboardWriter>,
    id: String,
) -> Result<u32, &'static str> {
    require_main_or_log_window(&window)?;
    let (session_id, text) = state.copy_payload(&id)?;
    clipboard
        .write_result(session_id, text)
        .map_err(|_| "clipboard_write_failed")?
        .clipboard_sequence
        .ok_or("clipboard_sequence_unavailable")
}

/// Copies the transcriber's own last final to the clipboard.
///
/// This is the amendment to the rule that keeps clipboard authority out of the
/// transcriber (see `require_main_or_hud_window`). Three properties keep the
/// grant narrow enough to be worth making:
///
/// 1. It takes no argument. There is no id to forge and no way to name another
///    session's entry — `copy_latest_payload` resolves the newest final here, in
///    Rust.
/// 2. The window never hands text back to be written, so it cannot use the
///    clipboard as an arbitrary write primitive. It can only ask for the text the
///    backend already holds.
/// 3. It is the same `ClipboardWriter` the main window uses, so the write is
///    sequenced and observable exactly as `result_copy` is.
///
/// What it buys: `refused` and `held` deliveries stop being dead ends. Before
/// this, a transcript the target app rejected could only be recovered by opening
/// settings and finding it in the session log.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hud_transcript_copy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, SessionTranscriptCoordinator>,
    clipboard: tauri::State<'_, ClipboardWriter>,
) -> Result<u32, &'static str> {
    require_main_or_hud_window(&window)?;
    let (session_id, text) = state.copy_latest_payload()?;
    clipboard
        .write_result(session_id, text)
        .map_err(|_| "clipboard_write_failed")?
        .clipboard_sequence
        .ok_or("clipboard_sequence_unavailable")
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hotkey_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, HotkeyCoordinator>,
) -> Result<HotkeyView, &'static str> {
    require_main_or_hud_window(&window)?;
    state.view()
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hotkey_configure(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    binding: String,
    mode: String,
    enabled: bool,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    if binding.trim().is_empty() {
        return Err("hotkey_binding_invalid");
    }
    let (mode, stored_mode) = match mode.as_str() {
        "toggle" => (ActivationMode::Toggle, ActivationHotkeyMode::Toggle),
        "push_to_talk" => (ActivationMode::PushToTalk, ActivationHotkeyMode::PushToTalk),
        "hands_free" => (ActivationMode::HandsFree, ActivationHotkeyMode::HandsFree),
        _ => return Err("hotkey_mode_invalid"),
    };
    let coordinator = app.state::<HotkeyCoordinator>();
    let previous = coordinator
        .binding
        .lock()
        .map_err(|_| "hotkey_state_unavailable")?
        .clone();
    let _ = app.global_shortcut().unregister(previous.as_str());
    {
        let mut current = coordinator
            .binding
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?;
        binding.clone_into(&mut current);
    }
    *coordinator
        .mode
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = mode;
    *coordinator
        .enabled
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = enabled;

    let profile = app.state::<ProfileCoordinator>();
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.hotkey.enabled = enabled;
    settings.hotkey.activation_binding = binding;
    settings.hotkey.activation_mode = stored_mode;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;

    register_activation_hotkey(&app)
}

/// Applies the persisted activation preference to the live coordinator.
fn apply_hotkey_preferences(
    coordinator: &HotkeyCoordinator,
    settings: &Settings,
) -> Result<(), &'static str> {
    {
        let mut current = coordinator
            .binding
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?;
        settings.hotkey.activation_binding.clone_into(&mut current);
    }
    *coordinator
        .mode
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = match settings.hotkey.activation_mode {
        ActivationHotkeyMode::Toggle => ActivationMode::Toggle,
        ActivationHotkeyMode::PushToTalk => ActivationMode::PushToTalk,
        ActivationHotkeyMode::HandsFree => ActivationMode::HandsFree,
    };
    *coordinator
        .enabled
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = settings.hotkey.enabled;
    Ok(())
}

/// Consumes the one-shot binding the installer recorded for this profile.
///
/// The seed file carries no transcript content and is removed after it is read
/// so that later user changes always win. Consuming the seed also arms the
/// installed dictation defaults (hotkey enabled, automatic paste on commit).
fn consume_installer_hotkey_seed(app_root: &Path, settings: &mut Settings) -> bool {
    let seed = app_root.join("config/install-hotkey.txt");
    let Ok(contents) = std::fs::read_to_string(&seed) else {
        return false;
    };
    let _ = std::fs::remove_file(&seed);
    let binding = contents.trim();
    if binding.is_empty() || binding.len() > 64 {
        return false;
    }
    settings.hotkey.enabled = true;
    binding.clone_into(&mut settings.hotkey.activation_binding);
    settings.delivery.auto_paste = true;
    true
}

/// Consumes the one-shot transcript-retention choice the installer recorded.
///
/// Follows [`consume_installer_logging_seed`] exactly, including the deletion:
/// a seed is a starting value and never a policy, so a user who turns retention
/// on afterwards must not find it off again on the next launch.
///
/// Settings already default this off, so strictly only `"1"` needs to act. Both
/// are handled anyway — an explicit `"0"` from setup and an absent seed are
/// different facts, and treating the first as the second is how a channel
/// starts drifting from what it says it carries.
fn consume_installer_retention_seed(app_root: &Path, settings: &mut Settings) -> bool {
    let seed = app_root.join("config/install-retention.txt");
    let Ok(contents) = std::fs::read_to_string(&seed) else {
        return false;
    };
    let _ = std::fs::remove_file(&seed);
    match contents.trim() {
        "0" => {
            settings.privacy.persisted_history_enabled = false;
            true
        }
        "1" => {
            settings.privacy.persisted_history_enabled = true;
            true
        }
        _ => false,
    }
}

/// Consumes the words the installer's vocabulary page collected.
///
/// Returns them rather than applying them, because they do not live in
/// `Settings` — they are dictionary entries, and the coordinator that owns
/// those is built later in `composition.rs`. The seed is still deleted here, so
/// the read and the delete stay in one place with the others.
///
/// Bounded at 128 terms and 64 characters each, matching what
/// `extract_v1_protected_terms` accepts from an imported profile. The bound is
/// not about this text box: it is about the file, which anything with write
/// access to the profile directory could replace before first launch.
///
/// **Commas, and newlines too.** Setup's box became a comma-separated list on
/// 2026-08-20 and writes the comma form, so commas are what this has to read.
/// Newlines still separate because this file is untrusted input — an
/// installation predating the change, or a file written by hand, means the same
/// thing by a line break, and a word lost to punctuation is the least
/// defensible failure this path could have.
///
/// De-duplicated case-insensitively, and that is load-bearing rather than tidy:
/// two entries whose sources differ only in case are a conflicting rule to the
/// dictionary validator, which rejects the **whole** batch. A file containing
/// "Ken, ken" would otherwise cost the user every word in it.
fn consume_installer_vocabulary_seed(app_root: &Path) -> Vec<String> {
    let seed = app_root.join("config/install-vocabulary.txt");
    let Ok(contents) = std::fs::read_to_string(&seed) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&seed);
    let mut terms: Vec<String> = Vec::new();
    for candidate in contents.split([',', '\n', '\r']) {
        let term = candidate.trim();
        if term.is_empty() || term.chars().count() > 64 {
            continue;
        }
        // `to_lowercase`, matching the validator's own match key, rather than
        // the ASCII fold: a pair this misses is a pair it still refuses.
        let folded = term.to_lowercase();
        if terms.iter().any(|kept| kept.to_lowercase() == folded) {
            continue;
        }
        terms.push(term.to_owned());
        if terms.len() == 128 {
            break;
        }
    }
    terms
}

/// Which configuration setup installed, as a stable code for the log.
///
/// **Read, never consumed.** The other seeds are one-shot starting values a
/// user can then change; this one is a record of what is on disk, and it stays
/// true for the life of the installation. Deleting it would make the second
/// launch unable to answer the question the first one could.
///
/// The question it answers is the one `docs/ARCHITECTURE.md` calls "which
/// provider runs, and how you find out": running on the processor is the
/// expected outcome of a processor install and a *fault* in a graphics-card
/// install, and those two owe the user opposite messages. Without this they are
/// the same silent state.
///
/// `"unrecorded"` for an installation that predates the seed or was placed by
/// hand — deliberately its own answer rather than being folded into `"cpu"`,
/// which would be a claim about a choice nobody made.
fn installed_configuration(app_root: &Path) -> &'static str {
    match std::fs::read_to_string(app_root.join("config/install-provider.txt"))
        .as_deref()
        .map(str::trim)
    {
        Ok("cpu") => "cpu",
        Ok("cuda") => "cuda",
        _ => "unrecorded",
    }
}

/// Consumes the one-shot diagnostic-logging choice the installer recorded.
///
/// The seed file carries only "0" or "1" and is removed after it is read so
/// that later user changes always win. Settings already default logging on;
/// this only needs to act when the installer's page recorded an opt-out.
fn consume_installer_logging_seed(app_root: &Path, settings: &mut Settings) -> bool {
    let seed = app_root.join("config/install-logging.txt");
    let Ok(contents) = std::fs::read_to_string(&seed) else {
        return false;
    };
    let _ = std::fs::remove_file(&seed);
    match contents.trim() {
        "0" => {
            settings.privacy.disk_logging_enabled = false;
            true
        }
        "1" => {
            settings.privacy.disk_logging_enabled = true;
            true
        }
        _ => false,
    }
}
