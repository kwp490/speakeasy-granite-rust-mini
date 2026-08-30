/// Drops the resident Granite worker and warms a fresh one. The command
/// returns immediately; settings keeps reading `gpu_status` while the bounded
/// worker operation completes.
///
/// `gpu_override` used to sit beside this, letting the user pin the engine to
/// CPU or CUDA. It is gone, because Granite's provider is not a preference:
/// the GPU path exists only where a CUDA-capable *worker binary* was built,
/// and no setting can conjure one. A control offering a choice the machine
/// cannot honour is worse than no control -- it reports a state the engine
/// will not be in. What this machine actually resolved to is reported by
/// `diagnostics_status`, which reads it from the engine rather than from a
/// preference.
#[tauri::command]
fn gpu_retest(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), &'static str> {
    require_main_window(&window)?;
    app.state::<GraniteEngineCoordinator>().invalidate();
    warm_granite_engine(&app);
    Ok(())
}

fn model_install_payload(
    pack: &Pack,
    downloads_root: &Path,
) -> Result<ModelInstallPayload, &'static str> {
    if let Some(archive) = pack.archive() {
        return Ok(ModelInstallPayload::Archive(DownloadRequest {
            url: archive.url().ok_or("pack_is_not_downloadable")?.to_owned(),
            // Preserve the original archive cache path so interrupted
            // Nemotron downloads retain their resumable `.part` state.
            destination: downloads_root.join(format!("{}-{}.archive", pack.id(), pack.revision())),
            expected_bytes: archive.bytes(),
            expected_sha256: archive.sha256().to_owned(),
        }));
    }
    let loose_root = downloads_root.join(format!("{}-{}", pack.id(), pack.revision()));
    let files = pack
        .required_files()
        .iter()
        .map(|file| {
            let relative = PathBuf::from(file.path());
            Ok((
                relative.clone(),
                DownloadRequest {
                    url: file.url().ok_or("pack_is_not_downloadable")?.to_owned(),
                    destination: loose_root.join(relative),
                    expected_bytes: file.bytes(),
                    expected_sha256: file.sha256().to_owned(),
                },
            ))
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    if files.is_empty() {
        return Err("pack_is_not_downloadable");
    }
    Ok(ModelInstallPayload::Loose(files))
}

fn model_download_policy() -> DownloadPolicy {
    DownloadPolicy {
        redirect_hosts: vec![
            "github.com".to_owned(),
            "release-assets.githubusercontent.com".to_owned(),
            "developer.download.nvidia.com".to_owned(),
            // Granite's immutable Hugging Face URLs redirect to this exact Xet
            // CDN host. Both hops remain HTTPS and exact-host checked.
            "huggingface.co".to_owned(),
            "us.aws.cdn.hf.co".to_owned(),
        ],
        connect_deadline_ms: 30_000,
        read_deadline_ms: 120_000,
        overall_deadline_ms: 1_800_000,
        maximum_retries: 3,
        proxy_aware: true,
    }
}

fn execute_model_install(
    root: &Path,
    spec: &InstallSpec,
    payload: ModelInstallPayload,
    total_bytes: u64,
    status: &Arc<Mutex<ModelInstallView>>,
    download_slot: &Arc<Mutex<Option<ActiveDownload>>>,
    token: &CancelToken,
) -> Result<(), String> {
    let policy = model_download_policy();
    let manager = InstallManager::new(root.join("models"));
    match payload {
        ModelInstallPayload::Archive(request) => {
            download_to_file(&request, &policy, token).map_err(|error| format!("{error}"))?;
            ModelCoordinator::set_status(status, "installing", None);
            manager
                .install_archive(spec, &request.destination, token)
                .map_err(|error| format!("{error}"))?;
        }
        ModelInstallPayload::Loose(files) => {
            let mut completed_bytes = 0_u64;
            let mut installed_files = Vec::with_capacity(files.len());
            for (relative, request) in files {
                let mut part_path = request.destination.clone().into_os_string();
                part_path.push(".part");
                if let Ok(mut slot) = download_slot.lock() {
                    *slot = Some(ActiveDownload {
                        part_path: PathBuf::from(part_path),
                        completed_bytes,
                        total_bytes,
                    });
                }
                download_to_file(&request, &policy, token).map_err(|error| format!("{error}"))?;
                completed_bytes = completed_bytes.saturating_add(request.expected_bytes);
                installed_files.push(LooseInstallFile {
                    path: relative,
                    source: request.destination,
                });
            }
            ModelCoordinator::set_status(status, "installing", None);
            manager
                .install_loose_files(spec, &installed_files, token)
                .map_err(|error| format!("{error}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn model_install_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    id: String,
    revision: String,
    confirmed: bool,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    if !confirmed {
        return Err("confirmation_required");
    }
    let manifest = bundled_manifest().map_err(|_| "catalog_unavailable")?;
    let pack = manifest
        .packs()
        .iter()
        .find(|pack| pack.id() == id && pack.revision() == revision && pack.is_install_eligible())
        .ok_or("pack_not_admitted")?;
    let spec = InstallSpec::from(pack);
    let downloads_root = state.root.join("downloads");
    let payload = model_install_payload(pack, &downloads_root)?;
    let total_bytes = payload.total_bytes().ok_or("pack_is_not_downloadable")?;
    let first_destination = payload.requests()[0].destination.clone();
    let mut active = state.cancel.lock().map_err(|_| "model_state_unavailable")?;
    if active.is_some() {
        return Err("install_busy");
    }
    operations.begin(ExclusiveOperation::ModelInstall)?;
    let token = CancelToken::default();
    *active = Some(token.clone());
    drop(active);
    let root = state.root.clone();
    let status = Arc::clone(&state.status);
    let cancel_slot = Arc::clone(&state.cancel);
    let download_slot = Arc::clone(&state.active_download);
    let operation_arbiter = Arc::clone(&operations.arbiter);
    let app_handle = app.clone();
    // Record what is being transferred while we still know it, so progress is
    // read off this download rather than guessed at from the GPU probe.
    // Appended rather than `with_extension`, which would treat the `.5` in
    // `nemotron-3.5-...` as the extension boundary on any id without a suffix.
    let mut part_path = first_destination.into_os_string();
    part_path.push(".part");
    if let Ok(mut slot) = download_slot.lock() {
        *slot = Some(ActiveDownload {
            part_path: PathBuf::from(part_path),
            completed_bytes: 0,
            total_bytes,
        });
    }
    ModelCoordinator::set_status(&status, "downloading", None);
    thread::spawn(move || {
        let result = execute_model_install(
            &root,
            &spec,
            payload,
            total_bytes,
            &status,
            &download_slot,
            &token,
        );
        match result {
            Ok(()) => {
                ModelCoordinator::set_status(&status, "verified_on_disk", None);
                // Installation is the cold-start boundary, so the exact resolved
                // pack is warmed here rather than at the next dictation. The
                // warm establishes the device and the provider record that
                // `gpu_status` reports; it runs no inference, so it cannot and
                // does not claim the engine has executed on the card.
                warm_granite_engine(&app_handle);
            }
            Err(_error) if token.is_cancelled() => {
                ModelCoordinator::set_status(&status, "cancelled", None);
            }
            Err(error) => ModelCoordinator::set_status(&status, "failed", Some(error)),
        }
        if let Ok(mut slot) = cancel_slot.lock() {
            *slot = None;
        }
        if let Ok(mut slot) = download_slot.lock() {
            *slot = None;
        }
        if let Ok(mut arbiter) = operation_arbiter.lock() {
            let _ = arbiter.finish(ExclusiveOperation::ModelInstall);
        }
    });
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn model_install_cancel(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    let active = state.cancel.lock().map_err(|_| "model_state_unavailable")?;
    active.as_ref().ok_or("install_not_active")?.cancel();
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn model_install_status(app: tauri::AppHandle) -> ModelInstallView {
    let Some(state) = app.try_state::<ModelCoordinator>() else {
        return ModelInstallView {
            state: "verifying".to_owned(),
            error: None,
            bytes_downloaded: None,
            bytes_total: None,
        };
    };
    let mut view = state.status_snapshot();
    if view.state == "downloading"
        && let Ok(slot) = state.active_download.lock()
        && let Some(active) = slot.as_ref()
    {
        view.bytes_downloaded = Some(
            active.completed_bytes.saturating_add(
                fs::metadata(&active.part_path)
                    .ok()
                    .map_or(0, |metadata| metadata.len()),
            ),
        );
        view.bytes_total = Some(active.total_bytes);
    }
    view
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn model_remove(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ModelCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    id: String,
    revision: String,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    let manifest = bundled_manifest().map_err(|_| "catalog_unavailable")?;
    let pack = manifest
        .packs()
        .iter()
        .find(|pack| pack.id() == id && pack.revision() == revision)
        .ok_or("pack_not_admitted")?;
    operations.begin(ExclusiveOperation::ModelDelete)?;
    let result = InstallManager::new(state.root.join("models"))
        .delete(&InstallSpec::from(pack))
        .map_err(|_| "remove_failed");
    if let Ok(mut arbiter) = operations.arbiter.lock() {
        let _ = arbiter.finish(ExclusiveOperation::ModelDelete);
    }
    result
}

