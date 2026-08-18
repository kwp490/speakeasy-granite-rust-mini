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
                // Installation is the cold-start boundary. Warm the exact
                // resolved pack now, and let that warm perform the execution
                // smoke that can construct GPU Qualified evidence.
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

/// What the CUDA runtime costs and whether this machine has it.
///
/// Sizes come from the plan and presence from disk on every call, so the offer a
/// user sees cannot disagree with what is installed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn cuda_runtime_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, CudaRuntimeCoordinator>,
    runtime: tauri::State<'_, RuntimeWizardCoordinator>,
) -> Result<CudaRuntimeView, &'static str> {
    require_main_window(&window)?;
    let plan = runtime_wizard::cuda_runtime_plan().ok_or("catalog_unavailable")?;
    let proof = runtime.paths()?.proof;
    let (mut phase, error) = state
        .status
        .lock()
        .map_err(|_| "cuda_runtime_state_unavailable")?
        .clone();
    // Disk outranks the remembered phase. A previous session's install, or a
    // `proof/` somebody populated by hand, is just as installed as one this
    // process performed — and after a success the phase and the disk must not be
    // able to disagree.
    let installed_components = plan.installed_components(&proof);
    if phase.is_empty() || phase == "absent" || phase == "installed" || phase == "partial" {
        phase = if plan.is_complete(&proof) {
            "installed".to_owned()
        } else if installed_components.is_empty() {
            "absent".to_owned()
        } else {
            "partial".to_owned()
        };
    }
    let active = state
        .active
        .lock()
        .map_err(|_| "cuda_runtime_state_unavailable")?
        .clone();
    let (bytes_downloaded, bytes_total) = match (phase.as_str(), active) {
        ("downloading" | "installing", Some(active)) => (
            Some(
                active.bytes_completed.saturating_add(
                    fs::metadata(&active.part_path)
                        .map(|metadata| metadata.len())
                        .unwrap_or_default(),
                ),
            ),
            Some(active.total_bytes),
        ),
        _ => (None, None),
    };
    Ok(CudaRuntimeView {
        state: phase,
        error,
        // The offer appears only for a card that could use it. `admissible` and
        // not `qualified`: qualification means a model has executed here, and
        // the runtime this fetches is what executing would need.
        offered: speakeasy_models::admit(&NvmlGpuProbe.probe())
            .device()
            .is_some(),
        download_bytes: plan.download_bytes(),
        installed_bytes: plan.installed_bytes(),
        file_count: u32::try_from(plan.file_count()).unwrap_or(u32::MAX),
        installed_components: installed_components
            .into_iter()
            .map(|component| component.code().to_owned())
            .collect(),
        bytes_downloaded,
        bytes_total,
    })
}

/// Fetches and installs the CUDA execution provider and its dependencies.
///
/// Never silent: `confirmed` is the user having seen the size, exactly as
/// `model_install_start` requires it.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn cuda_runtime_install_start(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, CudaRuntimeCoordinator>,
    runtime: tauri::State<'_, RuntimeWizardCoordinator>,
    operations: tauri::State<'_, OperationCoordinator>,
    confirmed: bool,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    if !confirmed {
        return Err("confirmation_required");
    }
    let plan = runtime_wizard::cuda_runtime_plan().ok_or("catalog_unavailable")?;
    let paths = runtime.cuda_runtime_paths()?;
    // Refused rather than wasted: 2.97 GB for a machine whose card cannot run it
    // is the one download nobody can justify after the fact.
    if speakeasy_models::admit(&NvmlGpuProbe.probe())
        .device()
        .is_none()
    {
        return Err("gpu_not_admissible");
    }
    let mut active = state
        .cancel
        .lock()
        .map_err(|_| "cuda_runtime_state_unavailable")?;
    if active.is_some() {
        return Err("install_busy");
    }
    // The same exclusion a pack install takes, so this cannot race one or start
    // mid-dictation. Both write hundreds of megabytes to the same volume, and
    // this one shuts the engine down.
    operations.begin(ExclusiveOperation::ModelInstall)?;
    let token = CancelToken::default();
    *active = Some(token.clone());
    drop(active);

    // **Before any file is moved.** `proof/` is writable while the app runs, so
    // DLLs can be added live — but a DLL the worker has already mapped is locked,
    // and a repair or re-install would fail on the rename. Releasing the resident
    // worker first makes the replace case work, and it is also what makes the new
    // runtime take effect on the next dictation: the next `ensure_ready` rebuilds
    // the adapter and re-resolves the provider from disk.
    app.state::<GraniteEngineCoordinator>().shutdown();

    let status = Arc::clone(&state.status);
    let cancel_slot = Arc::clone(&state.cancel);
    let active_slot = Arc::clone(&state.active);
    let operation_arbiter = Arc::clone(&operations.arbiter);
    let handle = app.clone();
    CudaRuntimeCoordinator::set_status(&status, "downloading", None);
    thread::spawn(move || {
        let policy = DownloadPolicy {
            redirect_hosts: vec![
                "github.com".to_owned(),
                "release-assets.githubusercontent.com".to_owned(),
                // The provider DLL comes from sherpa's GitHub release; the CUDA
                // and cuDNN redistributables from NVIDIA's own host. Nothing is
                // rehosted, so both have to be admitted, and exactly — this list
                // is all that stands between a redirect and an arbitrary target.
                "developer.download.nvidia.com".to_owned(),
            ],
            connect_deadline_ms: 30_000,
            read_deadline_ms: 120_000,
            // Per archive, not for the whole 2.97 GB: the largest is 1.9 GB, and
            // a single deadline across all five would fail an honest slow link.
            overall_deadline_ms: 3_600_000,
            maximum_retries: 3,
            proxy_aware: true,
        };
        let observed_status = Arc::clone(&status);
        let observed_active = Arc::clone(&active_slot);
        let result = plan.install(&paths, &policy, &token, &move |event| match event {
            speakeasy_models::CudaRuntimeEvent::Downloading {
                part_path,
                bytes_completed,
                ..
            } => {
                CudaRuntimeCoordinator::set_status(&observed_status, "downloading", None);
                if let Ok(mut slot) = observed_active.lock() {
                    *slot = Some(ActiveRuntimeDownload {
                        part_path: part_path.to_path_buf(),
                        bytes_completed,
                        total_bytes: plan.download_bytes(),
                    });
                }
            }
            speakeasy_models::CudaRuntimeEvent::Installing { .. } => {
                CudaRuntimeCoordinator::set_status(&observed_status, "installing", None);
            }
            speakeasy_models::CudaRuntimeEvent::Skipped { .. } => {}
        });
        match result {
            Ok(()) => {
                CudaRuntimeCoordinator::set_status(&status, "installed", None);
                // The runtime changed which pack resolves, and readiness was
                // computed before it existed.
                let available = handle
                    .state::<RuntimeWizardCoordinator>()
                    .cuda_runtime_available();
                handle
                    .state::<ModelCoordinator>()
                    .refresh_readiness(available);
            }
            Err(_) if token.is_cancelled() => {
                CudaRuntimeCoordinator::set_status(&status, "cancelled", None);
            }
            Err(error) => CudaRuntimeCoordinator::set_status(
                &status,
                "failed",
                Some(cuda_runtime_error_code(&error).to_owned()),
            ),
        }
        // **On every outcome, not just success.** The engine was shut down before
        // the first file moved, and a cold engine is not a neutral state: the
        // transcriber renders `cold` as "Loading model" and *disables* the record
        // button, because a press there would block on the mutex a model load
        // holds. So leaving it cold does not merely postpone the new runtime, it
        // takes dictation away until something else warms it — which is exactly
        // what happened on the installed build: the fetch succeeded and the
        // record button never became pressable again.
        //
        // Re-warming here restores it in every case. On success it loads on the
        // GPU, which is the "takes effect on the next dictation" half of the
        // decision; after a failure or a cancellation it loads on CPU exactly as
        // before, so a refused install costs the user nothing. This is the same
        // invalidate-then-warm pair a dead worker already uses.
        warm_granite_engine(&handle);
        if let Ok(mut slot) = cancel_slot.lock() {
            *slot = None;
        }
        if let Ok(mut slot) = active_slot.lock() {
            *slot = None;
        }
        if let Ok(mut arbiter) = operation_arbiter.lock() {
            let _ = arbiter.finish(ExclusiveOperation::ModelInstall);
        }
    });
    Ok(())
}

/// A stable code per failure, never a path or a URL.
///
/// The runtime install is the first thing in the app that can fail with a
/// filesystem path in hand, and this reaches the diagnostic log, which is a
/// privacy surface. It is also what lets the UI say something actionable:
/// "restart the app" and "free up disk" are different instructions, and both are
/// different from "the download failed".
const fn cuda_runtime_error_code(error: &speakeasy_models::CudaRuntimeError) -> &'static str {
    use speakeasy_models::CudaRuntimeError as Failure;
    match error {
        Failure::ManifestIncomplete(_) => "cuda_runtime_manifest_incomplete",
        Failure::NameCollision(_) => "cuda_runtime_name_collision",
        Failure::InsufficientDisk { .. } => "cuda_runtime_insufficient_disk",
        Failure::Download(_) => "cuda_runtime_download_failed",
        Failure::Extraction(_) => "cuda_runtime_verification_failed",
        Failure::RuntimeInUse(_) => "cuda_runtime_in_use",
        Failure::StillIncomplete => "cuda_runtime_incomplete",
        Failure::Cancelled => "cuda_runtime_cancelled",
        Failure::Io(_) => "cuda_runtime_write_failed",
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn cuda_runtime_install_cancel(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, CudaRuntimeCoordinator>,
) -> Result<(), &'static str> {
    require_main_window(&window)?;
    let active = state
        .cancel
        .lock()
        .map_err(|_| "cuda_runtime_state_unavailable")?;
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

