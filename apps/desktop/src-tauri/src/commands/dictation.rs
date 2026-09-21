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
    app: tauri::AppHandle,
    runtime: tauri::State<'_, RuntimeWizardCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    // Exclusive for the same reason a model install is: this discards the
    // resident worker, and doing that under a running pass would fail that
    // dictation rather than recover the engine.
    operations.begin(ExclusiveOperation::EngineRestart)?;
    let outcome = restart_granite_engine(&app, &runtime);
    operations.finish(ExclusiveOperation::EngineRestart);
    outcome
}

/// Restarts the engine the production dictation path actually uses.
///
/// [`RuntimeWizardCoordinator::recover_manually`] clears the *wizard's* crash
/// state, and that is not the state a dictation consults. The resident worker
/// and the quarantine that refuses a pass both live on
/// [`GraniteEngineCoordinator`], so recovery that never touched it reported
/// success while every later dictation still failed its quarantine check.
///
/// The warm is deliberately not awaited. It hashes the pack and loads roughly
/// 2 GB, which is far longer than an IPC command may hold, so this returns once
/// a replacement warm is under way; the caller watches `capture_hud_status` for
/// `ready` and reports success from that rather than from this returning.
fn restart_granite_engine(
    app: &tauri::AppHandle,
    runtime: &RuntimeWizardCoordinator,
) -> Result<(), &'static str> {
    // The wizard's crash state as well, and first: it refuses while a runtime
    // operation is active, which is the one condition under which tearing the
    // worker down would be wrong.
    runtime.recover_manually()?;
    {
        let granite = app
            .try_state::<GraniteEngineCoordinator>()
            .ok_or("granite_state_unavailable")?;
        granite.clear_quarantine();
        granite.invalidate();
    }
    warm_granite_engine(app);
    Ok(())
}

/// Switches the resident worker to the other provider, when setup staged a
/// binary for it.
///
/// The dock's right-click menu and the Settings equivalent both reach this
/// through their own exclusivity wrapper -- see `runtime_switch_engine_provider`
/// and `dispatch_menu_action` -- since a switch discards the resident worker
/// exactly as `restart_granite_engine` does, under the same
/// `ExclusiveOperation::EngineRestart` guard.
///
/// # Errors
///
/// `engine_provider_not_installed` when `provider` names a binary
/// [`RuntimeWizardCoordinator::paths`] has not resolved -- this project will
/// not claim a worker exists that setup never staged, so the check runs, and
/// the preference is left unpersisted, before anything else here happens.
fn switch_engine_provider(
    app: &tauri::AppHandle,
    runtime: &RuntimeWizardCoordinator,
    profile: &ProfileCoordinator,
    provider: EngineProvider,
) -> Result<(), &'static str> {
    let paths = runtime
        .paths()
        .map_err(|_| "engine_provider_not_installed")?;
    let installed = installed_configuration(&profile.root);
    // Switching "to" what is already installed clears the override rather than
    // storing a redundant one that would just resolve to a no-op every warm.
    let next_override = if provider.code() == installed {
        None
    } else {
        if paths.granite_worker_alternate.is_none() {
            return Err("engine_provider_not_installed");
        }
        Some(provider)
    };
    // The wizard's crash state, same as reload and for the same reason: it
    // refuses while a dictation holds the runtime lease, which is the one
    // condition under which tearing the worker down would be wrong.
    runtime.recover_manually()?;
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.engine_provider_override = next_override;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    {
        let granite = app
            .try_state::<GraniteEngineCoordinator>()
            .ok_or("granite_state_unavailable")?;
        granite.clear_quarantine();
        granite.invalidate();
    }
    warm_granite_engine(app);
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn runtime_switch_engine_provider(
    provider: EngineProvider,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    runtime: tauri::State<'_, RuntimeWizardCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    profile: tauri::State<'_, ProfileCoordinator>,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    // Exclusive for the same reason reload is: this discards the resident
    // worker, and doing that under a running pass would fail that dictation
    // rather than switch the engine.
    operations.begin(ExclusiveOperation::EngineRestart)?;
    let outcome = switch_engine_provider(&app, &runtime, &profile, provider);
    operations.finish(ExclusiveOperation::EngineRestart);
    outcome
}

/// The provider a toggle click should move to, from the one currently running.
///
/// Two providers, so a toggle rather than a picker: the dock's native menu can
/// only ever act on the one binary setup did not currently pick, and the
/// Settings control offers the same single choice for the same reason. `None`
/// for anything other than the two live codes, which keeps a caller from
/// having to special-case `unrecorded`.
fn opposite_engine_provider(current: &str) -> Option<EngineProvider> {
    match current {
        "cpu" => Some(EngineProvider::Cuda),
        "cuda" => Some(EngineProvider::Cpu),
        _ => None,
    }
}

#[cfg(test)]
mod switch_engine_provider_tests {
    use super::*;

    #[test]
    fn opposite_provider_toggles_between_the_two_live_codes() {
        assert_eq!(opposite_engine_provider("cpu"), Some(EngineProvider::Cuda));
        assert_eq!(opposite_engine_provider("cuda"), Some(EngineProvider::Cpu));
        assert_eq!(opposite_engine_provider("unrecorded"), None);
    }
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
    let profile = app.state::<ProfileCoordinator>();
    apply_hotkey_candidate(
        &coordinator,
        HotkeyCandidate {
            binding,
            mode,
            enabled,
        },
        |binding| {
            let _ = app.global_shortcut().unregister(binding);
        },
        || register_activation_hotkey(&app),
        |candidate| {
            let mut settings = profile
                .settings
                .lock()
                .map_err(|_| "profile_state_unavailable")?
                .clone();
            settings.hotkey.enabled = candidate.enabled;
            candidate
                .binding
                .clone_into(&mut settings.hotkey.activation_binding);
            settings.hotkey.activation_mode = stored_mode;
            profile.save(&settings)?;
            *profile
                .settings
                .lock()
                .map_err(|_| "profile_state_unavailable")? = settings;
            Ok(())
        },
    )
}

/// One candidate activation-hotkey change: what the coordinator holds, and what
/// `hotkey_configure` was asked to put there.
struct HotkeyCandidate {
    binding: String,
    mode: ActivationMode,
    enabled: bool,
}

/// Reads the coordinator's live activation state as a candidate.
fn read_hotkey_candidate(coordinator: &HotkeyCoordinator) -> Result<HotkeyCandidate, &'static str> {
    Ok(HotkeyCandidate {
        binding: coordinator
            .binding
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?
            .clone(),
        mode: *coordinator
            .mode
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?,
        enabled: *coordinator
            .enabled
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?,
    })
}

/// Writes a candidate into the live coordinator.
fn write_hotkey_candidate(
    coordinator: &HotkeyCoordinator,
    candidate: &HotkeyCandidate,
) -> Result<(), &'static str> {
    {
        let mut current = coordinator
            .binding
            .lock()
            .map_err(|_| "hotkey_state_unavailable")?;
        candidate.binding.clone_into(&mut current);
    }
    *coordinator
        .mode
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = candidate.mode;
    *coordinator
        .enabled
        .lock()
        .map_err(|_| "hotkey_state_unavailable")? = candidate.enabled;
    Ok(())
}

/// Applies one activation-hotkey change as a single transaction.
///
/// Registration runs before persistence, and a failure at either step puts the
/// previous binding, mode and enabled flag back and re-registers them. The
/// order is the whole point: this used to unregister the working shortcut
/// first and save before registering, so a binding Windows refused left the
/// user with no activation shortcut at all *and* the refused value on disk for
/// the next launch.
///
/// `register` reads the coordinator rather than taking an argument, because
/// that is how `register_activation_hotkey` works — so the candidate has to be
/// written before it runs, and a rollback has to rewrite the coordinator rather
/// than simply decline to write it.
fn apply_hotkey_candidate(
    coordinator: &HotkeyCoordinator,
    candidate: HotkeyCandidate,
    unregister: impl Fn(&str),
    register: impl Fn() -> Result<(), &'static str>,
    persist: impl Fn(&HotkeyCandidate) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let previous = read_hotkey_candidate(coordinator)?;
    unregister(&previous.binding);
    write_hotkey_candidate(coordinator, &candidate)?;

    let outcome = register().and_then(|()| persist(&candidate));
    if let Err(error) = outcome {
        unregister(&candidate.binding);
        write_hotkey_candidate(coordinator, &previous)?;
        // Best effort, and deliberately not reported in place of `error`: the
        // user asked about their change, and that is what failed. A restore
        // that cannot re-register leaves the registration status saying so.
        let _ = register();
        return Err(error);
    }
    Ok(())
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

/// What a warm should launch, and what provider it should be judged against.
///
/// Returned by [`resolve_active_worker`] rather than left to each call site,
/// because there are two of them and they must never disagree about which
/// binary a "switch" preference actually resolves to.
struct ActiveWorker {
    /// The worker binary to hand to `GraniteEnvironment.granite_worker_exe`.
    exe: PathBuf,
    /// What to hand `GraniteEnvironment.recorded_provider`.
    ///
    /// **This is the one detail the whole switch depends on, and it is easy to
    /// get wrong**: the exe alone can be swapped correctly while this still
    /// carries the raw `install-provider.txt` value, in which case
    /// `assess_provider_integrity` compares a deliberate CPU run against a
    /// `cuda` record and reports `gpu_install_not_operational` -- a real fault
    /// banner for a user who asked for exactly what is running. This field
    /// exists so that mistake is a wrong return value in one function, not a
    /// wrong comparison spread across two warm call sites.
    effective_provider: &'static str,
    /// A persisted override could not be honored this warm because its binary
    /// is no longer on disk. Distinct from "no override was requested" so a
    /// caller can disclose the degradation rather than silently reverting to
    /// whatever setup installed.
    override_unmet: bool,
}

/// Resolves the worker binary and effective provider a warm should use,
/// honoring a persisted switch only when the binary it names is actually
/// staged.
///
/// Never conjures a path: an override naming a provider with no resolved
/// [`RuntimePaths::granite_worker_alternate`] falls back to the installed
/// binary exactly as if no override had been set, because the one thing this
/// project will not do is claim a worker exists that setup never staged.
fn resolve_active_worker(
    paths: &RuntimePaths,
    installed: &'static str,
    override_: Option<EngineProvider>,
) -> ActiveWorker {
    let fall_back_to_installed = || ActiveWorker {
        exe: paths.granite_worker.clone(),
        effective_provider: installed,
        override_unmet: false,
    };
    let Some(requested) = override_ else {
        return fall_back_to_installed();
    };
    if requested.code() == installed {
        // Switching "back" to what is already canonical is a no-op, not a
        // request for the alternate -- there may not even be one.
        return fall_back_to_installed();
    }
    let Some(alternate) = &paths.granite_worker_alternate else {
        return ActiveWorker {
            override_unmet: true,
            ..fall_back_to_installed()
        };
    };
    ActiveWorker {
        exe: alternate.clone(),
        effective_provider: requested.code(),
        override_unmet: false,
    }
}

#[cfg(test)]
mod resolve_active_worker_tests {
    use super::*;

    fn paths_with(alternate: Option<&str>) -> RuntimePaths {
        RuntimePaths {
            root: PathBuf::from("root"),
            proof: PathBuf::from("root/proof"),
            granite_worker: PathBuf::from("root/proof/granite-worker.exe"),
            granite_worker_alternate: alternate
                .map(|name| PathBuf::from("root/proof").join(name)),
        }
    }

    #[test]
    fn no_override_runs_whatever_was_installed() {
        let paths = paths_with(Some("granite-worker.cpu.exe"));
        let active = resolve_active_worker(&paths, "cuda", None);
        assert_eq!(active.exe, paths.granite_worker);
        assert_eq!(active.effective_provider, "cuda");
        assert!(!active.override_unmet);
    }

    #[test]
    fn switching_back_to_the_installed_provider_is_a_no_op() {
        let paths = paths_with(Some("granite-worker.cpu.exe"));
        let active = resolve_active_worker(&paths, "cuda", Some(EngineProvider::Cuda));
        assert_eq!(active.exe, paths.granite_worker);
        assert_eq!(active.effective_provider, "cuda");
        assert!(!active.override_unmet);
    }

    /// The regression this type exists to prevent: a switch that changes the
    /// binary without also changing what it is judged against would still
    /// launch the right worker while reporting a fault, because
    /// `assess_provider_integrity` would be told the installation is `cuda`
    /// while a plain processor worker just answered its handshake. Both fields
    /// must move together.
    #[test]
    fn switching_to_the_alternate_moves_both_the_binary_and_the_provider() {
        let paths = paths_with(Some("granite-worker.cpu.exe"));
        let active = resolve_active_worker(&paths, "cuda", Some(EngineProvider::Cpu));
        assert_eq!(
            active.exe,
            paths.granite_worker_alternate.expect("alternate staged")
        );
        assert_eq!(
            active.effective_provider, "cpu",
            "the effective provider must follow the binary, not the install record"
        );
        assert!(!active.override_unmet);
    }

    /// A preference that can never be honored -- the alternate vanished after
    /// it was persisted -- must revert to the installed binary rather than
    /// naming a path that does not exist, and must say so rather than pretend
    /// the switch happened.
    #[test]
    fn an_override_with_no_staged_alternate_falls_back_and_reports_itself() {
        let paths = paths_with(None);
        let active = resolve_active_worker(&paths, "cuda", Some(EngineProvider::Cpu));
        assert_eq!(active.exe, paths.granite_worker);
        assert_eq!(active.effective_provider, "cuda");
        assert!(active.override_unmet);
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
