fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), &'static str> {
    (window.label() == "main")
        .then_some(())
        .ok_or("window_not_authorized")
}

/// Commands the pinned transcript-log window is allowed to call.
///
/// A third gate rather than a widening of either existing one, because the log
/// window's authority is genuinely a different shape. It needs
/// `session_transcript_copy`, which can name *any* entry in the log — exactly
/// the browsing authority `require_main_or_hud_window` refuses the dock. And
/// the dock must not gain that authority just because the log window has it,
/// which is what folding them into one gate would have done.
///
/// The log window earns it by being what it is: a window the user opened on
/// purpose, from settings, whose entire content is that list. The dock is
/// permanent furniture that is always on screen during a dictation.
///
/// It is still a no-activate window, so this is not a relaxation of the
/// foreground rule — only of the browsing one.
fn require_main_or_log_window(window: &tauri::WebviewWindow) -> Result<(), &'static str> {
    matches!(window.label(), "main" | "log")
        .then_some(())
        .ok_or("window_not_authorized")
}

/// Commands the compact transcriber is allowed to call.
///
/// The allowlist is the session controls plus `hud_transcript_copy`.
/// Every other command keeps `require_main_window`, so no OS-input, delivery,
/// history, model, personalization, diagnostics, reset or credential command is
/// reachable from the transcriber.
///
/// Decision 3 originally kept clipboard authority out of the transcriber
/// entirely. It is amended, not dropped: the transcriber may copy *the final it
/// just produced* and nothing else. `session_transcript_copy` — which can name
/// any entry in the log — stays main-only, so browsing history from here is
/// still forbidden. See `hud_transcript_copy` for why that narrower grant is not
/// forgeable.
///
/// `hud-dock` is the transcriber's other presentation, not a different
/// authority: the side dock is the same no-activate window family as `hud`,
/// just narrower content, so it shares this exact gate rather than getting a
/// gate of its own.
fn require_main_or_hud_window(window: &tauri::WebviewWindow) -> Result<(), &'static str> {
    matches!(window.label(), "main" | "hud-dock")
        .then_some(())
        .ok_or("window_not_authorized")
}

#[tauri::command]
fn profile_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ProfileCoordinator>,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    state.view()
}

#[tauri::command]
fn personalization_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    state.view()
}

#[tauri::command]
fn correction_record(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    id: String,
    locale: String,
    observed: String,
    corrected: String,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .record_explicit_correction(id, locale, observed, corrected)
        .map_err(|_| "correction_invalid")?;
    state.view()
}

#[tauri::command]
fn snippet_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    id: String,
    name: String,
    body: String,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .upsert_snippet(Snippet {
            id,
            name,
            body,
            enabled: true,
        })
        .map_err(|_| "snippet_invalid_or_action_rejected")?;
    state.view()
}

#[tauri::command]
fn personalization_delete(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    kind: String,
    id: String,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    let mut repository = state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?;
    match kind.as_str() {
        "dictionary" => repository
            .delete_dictionary(&id)
            .map_err(|_| "personalization_delete_failed")?,
        "snippet" => repository
            .delete_snippet(&id)
            .map_err(|_| "personalization_delete_failed")?,
        _ => return Err("personalization_kind_invalid"),
    };
    drop(repository);
    state.view()
}

#[tauri::command]
fn personalization_import_preview(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    json: String,
) -> Result<PersonalizationImportPreviewView, &'static str> {
    require_main_window(&window)?;
    let preview = state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .preview_import(json.as_bytes())
        .map_err(|_| "personalization_import_rejected")?;
    Ok(PersonalizationImportPreviewView {
        fingerprint_sha256: preview.fingerprint_sha256,
        dictionary_count: preview.dictionary_count,
        snippet_count: preview.snippet_count,
        conflicts: preview.conflicts.len(),
        contacts_imported: preview.contacts_imported,
    })
}

#[tauri::command]
fn personalization_import_commit(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    fingerprint: String,
    policy: PersonalizationImportPolicy,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .commit_import(&fingerprint, policy)
        .map_err(|_| "personalization_import_failed")?;
    state.view()
}

#[tauri::command]
fn personalization_export(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
) -> Result<String, &'static str> {
    require_main_window(&window)?;
    fs::create_dir_all(&state.export_root).map_err(|_| "personalization_export_failed")?;
    let file_name = format!(
        "speakeasy-personalization-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .export_json(&state.export_root.join(&file_name))
        .map_err(|_| "personalization_export_failed")?;
    Ok(file_name)
}

#[tauri::command]
fn personalization_reset(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PersonalizationCoordinator>,
    confirmed: bool,
) -> Result<PersonalizationView, &'static str> {
    require_main_window(&window)?;
    if !confirmed {
        return Err("personalization_reset_confirmation_required");
    }
    state
        .repository
        .lock()
        .map_err(|_| "personalization_state_unavailable")?
        .reset()
        .map_err(|_| "personalization_reset_failed")?;
    state.view()
}

#[tauri::command]
fn history_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    history: tauri::State<'_, HistoryCoordinator>,
    enabled: bool,
    retention_days: u16,
    plaintext_disclosure_accepted: bool,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    if let Some(error) = *history
        .initialization_error
        .lock()
        .map_err(|_| "history_state_unavailable")?
    {
        return Err(error);
    }
    if !(1..=365).contains(&retention_days) || (enabled && !plaintext_disclosure_accepted) {
        return Err("history_consent_required");
    }
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.privacy.persisted_history_enabled = enabled;
    settings.privacy.history_retention_days = retention_days;
    settings.privacy.history_plaintext_disclosure_accepted =
        enabled && plaintext_disclosure_accepted;
    profile.save(&settings)?;
    if let Some(repository) = history
        .repository
        .lock()
        .map_err(|_| "history_state_unavailable")?
        .as_mut()
    {
        repository
            .set_policy(HistoryPolicy {
                enabled,
                retention_days,
                plaintext_disclosure_accepted,
            })
            .map_err(|_| "history_policy_invalid")?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX);
        repository
            .apply_retention(now)
            .map_err(|_| "history_retention_failed")?;
    }
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    profile.view()
}

#[tauri::command]
fn disk_logging_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    enabled: bool,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.privacy.disk_logging_enabled = enabled;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    profile.view()
}

#[tauri::command]
fn delivery_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    preference: SafeDeliveryPreference,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.delivery.safe_preference = preference;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    profile.view()
}

#[tauri::command]
fn recording_feedback_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    enabled: bool,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.delivery.feedback_enabled = enabled;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    profile.view()
}

#[tauri::command]
fn history_list(
    window: tauri::WebviewWindow,
    history: tauri::State<'_, HistoryCoordinator>,
) -> Result<Vec<TranscriptResult>, &'static str> {
    require_main_window(&window)?;
    history
        .repository
        .lock()
        .map_err(|_| "history_state_unavailable")?
        .as_ref()
        .map_or(Ok(Vec::new()), |repository| {
            repository.list(100).map_err(|_| "history_read_failed")
        })
}

#[tauri::command]
fn history_export(
    window: tauri::WebviewWindow,
    history: tauri::State<'_, HistoryCoordinator>,
    disclosure_accepted: bool,
) -> Result<String, &'static str> {
    require_main_window(&window)?;
    if !disclosure_accepted {
        return Err("history_export_disclosure_required");
    }
    fs::create_dir_all(&history.export_root).map_err(|_| "history_export_failed")?;
    let file_name = format!(
        "speakeasy-history-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let path = history.export_root.join(&file_name);
    history
        .repository
        .lock()
        .map_err(|_| "history_state_unavailable")?
        .as_ref()
        .ok_or("history_unavailable")?
        .export_json(&path, true)
        .map_err(|_| "history_export_failed")?;
    Ok(file_name)
}

#[tauri::command]
fn history_delete_all(
    window: tauri::WebviewWindow,
    history: tauri::State<'_, HistoryCoordinator>,
    confirmed: bool,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    if !confirmed {
        return Err("history_delete_confirmation_required");
    }
    let mut slot = history
        .repository
        .lock()
        .map_err(|_| "history_state_unavailable")?;
    let repository = slot.take().ok_or("history_unavailable")?;
    let policy = repository.policy().clone();
    repository
        .delete_all()
        .map_err(|_| "history_delete_failed")?;
    *slot = HistoryRepository::open(&history.database_path, policy).ok();
    history
        .session
        .lock()
        .map_err(|_| "history_state_unavailable")?
        .clear();
    Ok(())
}

#[tauri::command]
fn startup_status_view(window: tauri::WebviewWindow) -> Result<bool, &'static str> {
    require_main_window(&window)?;
    startup_status()
        .map(|status| status.enabled)
        .map_err(|_| "startup_status_unavailable")
}

#[tauri::command]
fn startup_configure(
    window: tauri::WebviewWindow,
    profile: tauri::State<'_, ProfileCoordinator>,
    enabled: bool,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    let executable = std::env::current_exe().map_err(|_| "startup_executable_unavailable")?;
    set_startup_with_windows(enabled, &executable).map_err(|_| "startup_write_failed")?;
    let mut settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    settings.startup_with_windows = enabled;
    profile.save(&settings)?;
    *profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")? = settings;
    profile.view()
}

#[tauri::command]
fn credential_status(
    window: tauri::WebviewWindow,
    credentials: tauri::State<'_, WindowsCredentialManager>,
) -> Result<CredentialStatusView, &'static str> {
    require_main_window(&window)?;
    let report = credentials.legacy_report();
    Ok(CredentialStatusView {
        openai_legacy: credential_source_code(report.openai).to_owned(),
        remote_legacy: credential_source_code(report.remote).to_owned(),
        values_exposed: false,
    })
}

const fn credential_source_code(source: LegacyCredentialSource) -> &'static str {
    match source {
        LegacyCredentialSource::PrimaryService => "primary_service",
        LegacyCredentialSource::LegacyService => "legacy_service",
        LegacyCredentialSource::Missing => "missing",
        LegacyCredentialSource::AccessDenied => "access_denied",
        LegacyCredentialSource::Unavailable => "unavailable",
    }
}

#[tauri::command]
fn import_preview(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ImportCoordinator>,
) -> Result<Option<ImportPreview>, &'static str> {
    require_main_window(&window)?;
    let Some(root) = ProductionImportRoot::detect().map_err(|_| "v1_source_invalid")? else {
        return Ok(None);
    };
    // Source fingerprints protect preview and commit. Avoid searching PATH for
    // a process-list helper merely to add a best-effort warning.
    let running = false;
    let plan = ProductionImportPlan::inspect(root, running).map_err(|_| "v1_preview_failed")?;
    let preview = plan.preview().clone();
    *state
        .plan
        .lock()
        .map_err(|_| "v1_import_state_unavailable")? = Some(plan);
    Ok(Some(preview))
}

#[tauri::command]
fn import_commit(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ImportCoordinator>,
    personalization: tauri::State<'_, PersonalizationCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    nonce: String,
    choices: ImportChoices,
) -> Result<ImportReport, &'static str> {
    require_main_window(&window)?;
    operations.begin(ExclusiveOperation::StorageMigration)?;
    let result = state
        .plan
        .lock()
        .map_err(|_| "v1_import_state_unavailable")?
        .as_ref()
        .ok_or("v1_preview_required")?
        .commit(&state.destination, &nonce, &choices)
        .map_err(|_| "v1_import_failed")
        .and_then(|report| {
            if choices.presets {
                let presets_root = state.destination.join("config/presets");
                let mut imported_terms = Vec::new();
                if presets_root.is_dir() {
                    let entries =
                        fs::read_dir(&presets_root).map_err(|_| "v1_profile_vocabulary_failed")?;
                    for entry in entries.take(256) {
                        let path = entry.map_err(|_| "v1_profile_vocabulary_failed")?.path();
                        if path.extension().is_none_or(|extension| extension != "json") {
                            continue;
                        }
                        let bytes = fs::read(&path).map_err(|_| "v1_profile_vocabulary_failed")?;
                        if bytes.len() > 1_048_576 {
                            return Err("v1_profile_vocabulary_failed");
                        }
                        let preset: serde_json::Value = serde_json::from_slice(&bytes)
                            .map_err(|_| "v1_profile_vocabulary_failed")?;
                        let name = path
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .ok_or("v1_profile_vocabulary_failed")?;
                        imported_terms.extend(extract_v1_protected_terms(name, &preset, "en-US"));
                    }
                }
                personalization
                    .repository
                    .lock()
                    .map_err(|_| "personalization_state_unavailable")?
                    .add_imported_terms(imported_terms)
                    .map_err(|_| "v1_profile_vocabulary_failed")?;
            }
            Ok(report)
        });
    if let Ok(mut arbiter) = operations.arbiter.lock() {
        let _ = arbiter.finish(ExclusiveOperation::StorageMigration);
    }
    result
}

#[tauri::command]
fn reset_preview(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ProfileCoordinator>,
) -> Result<ResetPreviewView, &'static str> {
    require_main_window(&window)?;
    let nonce = format!(
        "reset-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    *state
        .reset_nonce
        .lock()
        .map_err(|_| "profile_state_unavailable")? = Some(nonce.clone());
    Ok(ResetPreviewView {
        nonce,
        categories: vec![
            "v2_settings".to_owned(),
            "v2_history".to_owned(),
            "v2_personalization".to_owned(),
            "v2_logs".to_owned(),
        ],
        excludes_v1: true,
        excludes_custom_models: true,
        excludes_credentials: true,
    })
}

#[tauri::command]
fn reset_commit(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ProfileCoordinator>,
    history: tauri::State<'_, HistoryCoordinator>,
    personalization: tauri::State<'_, PersonalizationCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    nonce: String,
) -> Result<ProfileView, &'static str> {
    require_main_window(&window)?;
    let expected = state
        .reset_nonce
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .take()
        .ok_or("reset_preview_required")?;
    if nonce != expected {
        return Err("reset_nonce_invalid");
    }
    operations.begin(ExclusiveOperation::StorageMigration)?;
    let result = (|| {
        personalization
            .repository
            .lock()
            .map_err(|_| "personalization_state_unavailable")?
            .reset()
            .map_err(|_| "personalization_reset_failed")?;
        *history
            .repository
            .lock()
            .map_err(|_| "history_state_unavailable")? = None;
        for path in [
            state.root.join("config/settings.json"),
            state.root.join("config/settings.json.bak"),
            state.root.join("data/speakeasy.sqlite3"),
            state.root.join("data/speakeasy.sqlite3-wal"),
            state.root.join("data/speakeasy.sqlite3-shm"),
        ] {
            if path.exists() {
                fs::remove_file(path).map_err(|_| "reset_remove_failed")?;
            }
        }
        let logs = state.root.join("logs");
        if logs.exists() {
            fs::remove_dir_all(logs).map_err(|_| "reset_remove_failed")?;
        }
        let settings = Settings::default();
        state
            .store
            .save(&settings)
            .map_err(|_| "profile_save_failed")?;
        *state
            .settings
            .lock()
            .map_err(|_| "profile_state_unavailable")? = settings;
        *state
            .load_error
            .lock()
            .map_err(|_| "profile_state_unavailable")? = None;
        let reopened = HistoryRepository::open(&history.database_path, HistoryPolicy::default())
            .map_err(|_| "history_recovery_required")?;
        *history
            .repository
            .lock()
            .map_err(|_| "history_state_unavailable")? = Some(reopened);
        *history
            .initialization_error
            .lock()
            .map_err(|_| "history_state_unavailable")? = None;
        history
            .session
            .lock()
            .map_err(|_| "history_state_unavailable")?
            .clear();
        state.view()
    })();
    if let Ok(mut arbiter) = operations.arbiter.lock() {
        let _ = arbiter.finish(ExclusiveOperation::StorageMigration);
    }
    result
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn diagnostics_status(
    window: tauri::WebviewWindow,
    models: tauri::State<'_, ModelCoordinator>,
    profile: tauri::State<'_, ProfileCoordinator>,
    granite: tauri::State<'_, GraniteEngineCoordinator>,
    capture: tauri::State<'_, CaptureWizardCoordinator>,
    hud: tauri::State<'_, CaptureHudCoordinator>,
    diagnostics: tauri::State<'_, DiagnosticsRuntimeCoordinator>,
) -> Result<DiagnosticsView, &'static str> {
    require_main_window(&window)?;
    let settings = profile
        .settings
        .lock()
        .map_err(|_| "profile_state_unavailable")?
        .clone();
    let (delivery_capability, delivery_reason) = if settings.delivery.auto_paste {
        ("commit_on_finish", "hotkey_auto_paste_enabled")
    } else if settings.delivery.auto_copy {
        ("auto_copy", "auto_copy_enabled")
    } else {
        (
            "result_view_only",
            "automatic_delivery_disabled_in_settings",
        )
    };
    // One resolver, asked once. This used to select the pack here and then
    // look the same pack up a second time in the manifest to reach its source
    // metadata -- two lookups that could disagree about which pack the answer
    // described. `granite_selection` resolves and flattens in one pass.
    let selection = granite_selection(&models.root.join("models"), granite.cuda_worker_available());
    let runtime_snapshot = diagnostics.snapshot();
    let (_, hud_source_reason) = hud.diagnostics()?;
    let final_source_reason = hud_source_reason.or(runtime_snapshot.final_source_reason);
    let capture_view = capture.view()?;
    let (engine, worker, runtime_name, provider, model_id, model_revision, model_source) =
        match selection {
            Some(selection) => (
                format!("{}:{}", selection.capabilities.runtime, selection.pack_id),
                granite.engine_reason().to_owned(),
                selection.capabilities.runtime.to_owned(),
                selection.capabilities.provider.to_owned(),
                selection.pack_id,
                selection.pack_revision,
                selection.source,
            ),
            None => (
                "engine_unresolved".to_owned(),
                "granite_worker_unavailable".to_owned(),
                "runtime_unresolved".to_owned(),
                "provider_unresolved".to_owned(),
                "model_not_installed".to_owned(),
                "revision_not_measured".to_owned(),
                "trusted_manifest_unresolved".to_owned(),
            ),
        };
    let device = capture_view
        .device_name
        .unwrap_or_else(|| "capture_device_not_selected".to_owned());
    Ok(DiagnosticsView {
        schema_version: DOMAIN_SCHEMA_VERSION,
        engine,
        worker,
        runtime: runtime_name,
        provider,
        rtf_median: runtime_snapshot.rtf_median,
        rtf_p95: runtime_snapshot.rtf_p95,
        latency_p50_ms: runtime_snapshot.latency_p50_ms,
        latency_p95_ms: runtime_snapshot.latency_p95_ms,
        audio_overflow_count: capture.audio_overflow_count(),
        device,
        vad: "manual_stop_only".to_owned(),
        delivery_capability: delivery_capability.to_owned(),
        delivery_reason: delivery_reason.to_owned(),
        model_id,
        model_revision,
        model_source,
        final_source_reason,
        recent_reason_codes: diagnostics.recent_reason_codes(),
        logs_sanitized: true,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn diagnostics_export(
    window: tauri::WebviewWindow,
    models: tauri::State<'_, ModelCoordinator>,
    profile: tauri::State<'_, ProfileCoordinator>,
    granite: tauri::State<'_, GraniteEngineCoordinator>,
    capture: tauri::State<'_, CaptureWizardCoordinator>,
    hud: tauri::State<'_, CaptureHudCoordinator>,
    diagnostics: tauri::State<'_, DiagnosticsRuntimeCoordinator>,
) -> Result<DiagnosticsExportView, &'static str> {
    require_main_window(&window)?;
    let directory = profile.root.join("diagnostics");
    let diagnostics = diagnostics_status(
        window, models, profile, granite, capture, hud, diagnostics,
    )?;
    fs::create_dir_all(&directory).map_err(|_| "diagnostics_export_failed")?;
    let file_name = format!(
        "speakeasy-diagnostics-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let path = directory.join(&file_name);
    let bytes = serde_json::to_vec_pretty(&diagnostics).map_err(|_| "diagnostics_export_failed")?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "diagnostics_export_failed")?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "diagnostics_export_failed")?;
    Ok(DiagnosticsExportView {
        file_name,
        categories: vec![
            "runtime".to_owned(),
            "performance".to_owned(),
            "audio".to_owned(),
            "delivery".to_owned(),
            "model_provenance".to_owned(),
            "reason_codes".to_owned(),
        ],
        contains_sensitive_content: false,
    })
}

#[tauri::command]
fn model_catalog(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
) -> Result<Vec<ModelCatalogItem>, String> {
    require_main_window(&window).map_err(str::to_owned)?;
    let manifest = bundled_manifest().map_err(|_| "catalog_unavailable")?;
    let manager = InstallManager::new(state.root.join("models"));
    Ok(manifest
        .packs()
        .iter()
        .filter(|pack| pack.is_install_eligible())
        .map(|pack| {
            let license = pack.licenses().first();
            ModelCatalogItem {
                id: pack.id().to_owned(),
                revision: pack.revision().to_owned(),
                display_name: pack.display_name().to_owned(),
                // An archive-based pack downloads its (possibly compressed)
                // archive; an archive-less, loose-file pack (Granite's Hugging
                // Face GGUFs) downloads exactly its required files, so their
                // sizes stand in for what an archive's own `bytes` would say.
                archive_bytes: pack.archive().map_or_else(
                    || pack.required_files().iter().map(RequiredFile::bytes).sum(),
                    Archive::bytes,
                ),
                installed_bytes: pack.installed_bytes(),
                confirmation_required: true,
                source_repository: pack.source().upstream_repository().to_owned(),
                source_revision: pack.source().upstream_revision().to_owned(),
                license_name: license.map_or("unknown", |item| item.name()).to_owned(),
                license_spdx: license.and_then(|item| item.spdx_id()).map(str::to_owned),
                license_url: license.map_or("", |item| item.text_url()).to_owned(),
                runtime: format!("{:?}", pack.runtime().name()).to_ascii_lowercase(),
                provider: format!("{:?}", pack.runtime().provider()).to_ascii_lowercase(),
                capabilities: pack
                    .capabilities()
                    .iter()
                    .map(|capability| {
                        format!(
                            "{}:{:?}{}",
                            capability.locale(),
                            capability.task(),
                            capability
                                .target_locale()
                                .map_or(String::new(), |target| format!(":{target}"))
                        )
                        .to_ascii_lowercase()
                    })
                    .collect(),
                hardware_evidence: "current_host_installation_only".to_owned(),
                downloadable: pack.is_downloadable(),
                installed: manager.is_present(&InstallSpec::from(pack)),
            }
        })
        .collect())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn model_hardware(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
    qualification: tauri::State<'_, GpuQualificationCoordinator>,
) -> Result<ModelHardwareView, &'static str> {
    require_main_window(&window)?;
    let snapshot = SafeStandardHardwareProbe.probe(&state.root);
    Ok(ModelHardwareView {
        operating_system: snapshot.operating_system,
        operating_system_build: snapshot.operating_system_build,
        architecture: snapshot.architecture,
        physical_cores: snapshot.physical_cores,
        logical_processors: snapshot.logical_processors,
        has_avx2: snapshot.has_avx2,
        total_memory_bytes: snapshot.total_memory_bytes,
        available_disk_bytes: snapshot.available_disk_bytes,
        adapters: snapshot
            .detected_adapters
            .into_iter()
            .map(|item| item.name)
            .collect(),
        // Still never true from inventory. The GPU probe can say a card is
        // admissible, but qualification means a model has run, and nothing here
        // has run one. `gpu_status` reports the difference.
        qualified: qualification.current(&NvmlGpuProbe.probe()).is_qualified(),
    })
}

/// Reports whether this machine can run the GPU backends, and why not when it
/// cannot.
///
/// Reads the probe on every call rather than caching at launch. Free VRAM moves
/// — other applications take and release it — and a driver can be installed
/// while the app is open, which is precisely the case where a blocked user
/// retries and should not have to restart to be re-examined.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn gpu_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
    qualification: tauri::State<'_, GpuQualificationCoordinator>,
    granite: tauri::State<'_, GraniteEngineCoordinator>,
) -> Result<GpuStatusView, &'static str> {
    require_main_window(&window)?;
    let selection = granite_selection(&state.root.join("models"), granite.cuda_worker_available());
    let snapshot = NvmlGpuProbe.probe();
    let decision = qualification.current(&snapshot);
    Ok(GpuStatusView::from_snapshot(
        &snapshot,
        selection.as_ref(),
        &decision,
    ))
}

