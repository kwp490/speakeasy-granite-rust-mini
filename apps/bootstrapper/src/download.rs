//! What setup fetches, where it puts it, and how it reports progress honestly.
//!
//! Nothing here implements downloading. `crates/speakeasy-models` already owns a
//! durable, validator-bound resume — `.partial` plus sidecar metadata,
//! re-validated by `validate_resume_response` against the server's status, range
//! start, complete length and `ETag` before a single byte is appended — and
//! `InstallManager` already owns staged extraction, per-file digest verification
//! and atomic activation. Re-implementing either would be a regression, and the
//! resume half is the one most likely to look finished while being untested.
//! This module is the plan and the thread: which artifacts, in what order, into
//! which directory, and what the user is told while it happens.
//!
//! **The directory is the load-bearing part.** Setup downloads into the same
//! root the installed app reads from, or the work is invisible to it and the
//! first launch downloads everything again. That root is not the install root:
//! program files go to `%LOCALAPPDATA%\SpeakEasy`, models to
//! `%APPDATA%\ai.speakeasy.mini\model-lifecycle`, which is where
//! `composition.rs` points `InstallManager` at. `agrees_with_the_app` in the
//! tests below pins the two together, because nothing else would notice them
//! drifting apart — a mismatch downloads three gigabytes to a directory nobody
//! reads and reports success.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use speakeasy_domain::CancelToken;
use speakeasy_models::{
    DownloadPolicy, DownloadRequest, ExecutionProvider, GRANITE_CUDA_WORKER_ARTIFACT_ID,
    GpuPayloadRejection, InstallManager, InstallSpec, LooseInstallFile, NativeRuntimeSource, Pack,
    PackRole, bundled_manifest, download_to_file, gpu_configuration_is_installable,
    graphics_card_payload_sources,
};

use crate::{catalog, uninstall};

/// Where the shipped pack lands once setup has installed it.
///
/// `models/<id>/<revision>`, matching what `granite_engine` composes from
/// `ModelCoordinator`'s root in the app. Resolved from the manifest rather than
/// spelled out, so a pack swap moves this with it -- and returned as an
/// `Option` because every input can be absent: no manifest, no eligible pack, no
/// data directory.
#[must_use]
pub fn installed_model_root() -> Option<PathBuf> {
    let manifest = bundled_manifest().ok()?;
    let pack = manifest
        .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
        .ok()?;
    Some(
        model_lifecycle_root()?
            .join("models")
            .join(pack.id())
            .join(pack.revision()),
    )
}

/// Where the app keeps its models.
///
/// `model-lifecycle` under the app's data directory, matching
/// `composition.rs`'s `app_root.join("model-lifecycle")` exactly. Not derived
/// from Tauri's `app_data_dir`, because this binary has no Tauri in it; derived
/// from the same `%APPDATA%\<identifier>` shape, with the identifier pinned
/// against `tauri.conf.json` by the scaffold suite.
pub fn model_lifecycle_root() -> Option<PathBuf> {
    Some(uninstall::data_root()?.join("model-lifecycle"))
}

/// What a download failed for, in terms the catalog can turn into a sentence.
///
/// A string rather than an enum on purpose, for now: the underlying errors are
/// already typed in `speakeasy-models`, and setup's job is to carry the reason
/// through to the user rather than re-classify it. Every message that reaches
/// the window goes through `catalog`, per the UI guide.
pub type Failure = String;

/// One artifact setup will fetch and install.
pub struct Item {
    /// What the user is told is being fetched. Everyday register.
    pub label: &'static str,
    pub spec: InstallSpec,
    payload: Payload,
    /// Bytes to transfer. Not `installed_bytes`: an archive expands, and a
    /// progress bar that counts the expanded size stalls at the end.
    pub bytes: u64,
}

enum Payload {
    Archive(DownloadRequest),
    /// A pack published as loose files rather than one archive. Granite is this
    /// shape, and it matters for progress: several requests, each with its own
    /// resumable partial file, so "bytes so far" has to carry a completed base.
    Loose(Vec<(PathBuf, DownloadRequest)>),
}

impl Payload {
    fn requests(&self) -> Vec<&DownloadRequest> {
        match self {
            Self::Archive(request) => vec![request],
            Self::Loose(files) => files.iter().map(|(_, request)| request).collect(),
        }
    }
}

/// Everything setup intends to fetch, decided before any of it starts.
///
/// Decided up front so the step can state the total before the user commits to
/// it, and so a machine that cannot be served at all says so instead of failing
/// three gigabytes in.
pub struct Plan {
    pub items: Vec<Item>,
    pub total_bytes: u64,
    root: PathBuf,
}

impl Plan {
    /// Whether everything in this plan is already installed and verified.
    ///
    /// Checked against the installed tree rather than a flag, because the honest
    /// answer to "do I need to download this" is whether the files are there and
    /// match their digests. A re-run of setup over a good installation should
    /// transfer nothing and say so.
    pub fn already_satisfied(&self) -> bool {
        let manager = InstallManager::new(self.root.join("models"));
        self.items.iter().all(|item| manager.is_present(&item.spec))
    }
}

/// Build the plan for this machine.
///
/// `provider` is what the compatibility step decided, and it does not select a
/// different model: there is one Granite pack and it is the CPU-variant GGUF
/// either way, because the CUDA worker offloads that same file. This is why
/// `engine=cpu_gpu_pack_not_installed device=cuda` is the correct state in the
/// app's log rather than a fault.
///
/// What the provider decides is whether the CUDA worker and the libraries it
/// loads are fetched alongside the weights. A machine that asked for the
/// processor fetches one item and a machine that asked for the graphics card
/// fetches four, and the difference is not cosmetic: nothing else in setup
/// puts a CUDA worker on the disk, so a plan that ignored the answer would
/// install the processor configuration and say nothing — which is the defect
/// the provider page's own disabling exists to prevent, arriving from the other
/// direction.
///
/// Nothing is fetched for a graphics-card install that this release cannot
/// serve. [`graphics_card_payload_sources`] answers empty in that case, so a
/// caller that somehow asks for CUDA against a manifest with no worker in it
/// gets the processor plan rather than a partial payload — and the same
/// question has already disabled the option upstream.
///
/// # Errors
///
/// Returns a catalog message when the manifest cannot be parsed, when no
/// install-eligible pack fills a role on the wanted provider, or when the app's
/// data directory cannot be located.
pub fn plan(provider: ExecutionProvider) -> Result<Plan, Failure> {
    let manifest = bundled_manifest().map_err(|_| catalog::CATALOG_UNAVAILABLE.to_owned())?;
    let root = model_lifecycle_root().ok_or_else(|| catalog::DATA_ROOT_UNLOCATABLE.to_owned())?;
    plan_from(&manifest, provider, root)
}

/// [`plan`], against a supplied catalog.
///
/// Split out on 2026-08-26 so the pinned-worker case could be tested *before*
/// the worker was published, which is how the graphics-card plan was proved at
/// all: the bug this path was written around — a wizard that offers the graphics
/// card only to machines that already have it — was found by simulating the pin
/// and could not have been found any other way on a machine with a worker staged
/// by hand.
///
/// It stays split now that the artifact is real, because the catalog is still the
/// input worth varying: `a_worker_without_its_libraries_is_not_a_fetchable_configuration`
/// covers a half-written manifest, and the next re-pin gets the same instrument
/// this one had.
fn plan_from(
    manifest: &speakeasy_models::TrustedManifest,
    provider: ExecutionProvider,
    root: PathBuf,
) -> Result<Plan, Failure> {
    let downloads = root.join("downloads");

    // The weights, first and on every machine: there is one Granite pack and it
    // is the CPU-variant GGUF either way, because the CUDA worker offloads that
    // same file. This is why `engine=cpu_gpu_pack_not_installed device=cuda` is
    // the correct state in the app's log rather than a fault.
    let pack = manifest
        .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
        .map_err(|error| {
            catalog::pack_unavailable(catalog::ARTIFACT_GRANITE, &error.to_string())
        })?;
    let mut items = vec![item_for(pack, &downloads)?];
    if provider == ExecutionProvider::Cuda {
        for source in graphics_card_payload_sources(manifest) {
            items.push(item_for_runtime(source, &downloads));
        }
    }

    let total_bytes = items.iter().map(|item| item.bytes).sum();
    Ok(Plan {
        items,
        total_bytes,
        root,
    })
}

/// Whether a graphics-card configuration is a thing setup could install, and
/// why not when it is not.
///
/// Asked before the choice is offered, not after it is made — and the
/// consequence of getting it wrong is the worst kind: a selectable option that
/// installs the CPU configuration anyway and says nothing. That is not
/// hypothetical. Until 2026-08-20 this asked the manifest for a CUDA `final-asr`
/// **pack**, which answers a different question: there is one GGUF and a CUDA
/// worker offloads that same file, so a pack entry would be a duplicate of the
/// CPU one and its presence says nothing about whether a GPU path exists. The
/// option was also never disabled, so selecting it wrote `installed=cuda` onto
/// an installation with no CUDA worker in it.
///
/// It asks `speakeasy_models::gpu_configuration_is_installable`, which is the one
/// place that fact lives.
///
/// **It used to ask `inspect_gpu_payload`, and that was wrong in a way no test
/// on a development machine could show.** That function answers "published *and*
/// present on disk", and this page is shown before the payload has been
/// extracted — so on a first install `proof/granite-worker.exe` does not exist,
/// the answer is `WorkerNotInstalled`, and the option stays disabled on every
/// fresh machine however the manifest is pinned. A machine that has already
/// staged a CUDA worker by hand answers `Ok(())` and looks correct, which is
/// exactly the machine this is developed on.
///
/// The presence check was there because "published alone would re-offer the
/// option on a machine where the runtime libraries never arrived". That case is
/// real and is answered later and better: `smoke::verify_engine` runs after the
/// payload is staged and the recorded provider comes from its verdict, and the
/// app re-proves the CUDA context at every warm. The wizard does not need to
/// pre-empt either, and it cannot do so correctly before the files exist.
///
/// # Errors
///
/// Returns the rejection, which the provider page turns into the sentence
/// naming which half is missing.
pub fn graphics_card_configuration_available() -> Result<(), GpuPayloadRejection> {
    let manifest = bundled_manifest().map_err(|_| GpuPayloadRejection::WorkerNotPublished)?;
    gpu_configuration_is_installable(&manifest)
}

/// Turn one pack into a fetchable item.
///
/// The destination paths deliberately match what `apps/desktop`'s
/// `model_install_payload` produces, down to the `{id}-{revision}.archive`
/// filename. Not tidiness: those names are where the resumable `.part` state
/// lives, so an interrupted download started by setup is picked up by the app,
/// and one started by the app is picked up by setup. Diverge here and the bytes
/// are still on disk, and both sides start again from zero.
fn item_for(pack: &Pack, downloads: &Path) -> Result<Item, Failure> {
    let spec = InstallSpec::from(pack);
    if let Some(archive) = pack.archive() {
        let url = archive
            .url()
            .ok_or_else(|| catalog::pack_not_downloadable(pack.id()))?
            .to_owned();
        return Ok(Item {
            label: label_for(pack),
            bytes: archive.bytes(),
            payload: Payload::Archive(DownloadRequest {
                url,
                destination: downloads.join(format!("{}-{}.archive", pack.id(), pack.revision())),
                expected_bytes: archive.bytes(),
                expected_sha256: archive.sha256().to_owned(),
            }),
            spec,
        });
    }

    let loose_root = downloads.join(format!("{}-{}", pack.id(), pack.revision()));
    let mut files = Vec::new();
    let mut bytes = 0_u64;
    for file in pack.required_files() {
        let relative = PathBuf::from(file.path());
        let url = file
            .url()
            .ok_or_else(|| catalog::pack_not_downloadable(pack.id()))?
            .to_owned();
        bytes = bytes.saturating_add(file.bytes());
        files.push((
            relative.clone(),
            DownloadRequest {
                url,
                destination: loose_root.join(relative),
                expected_bytes: file.bytes(),
                expected_sha256: file.sha256().to_owned(),
            },
        ));
    }
    if files.is_empty() {
        return Err(catalog::pack_not_downloadable(pack.id()));
    }
    Ok(Item {
        label: label_for(pack),
        bytes,
        payload: Payload::Loose(files),
        spec,
    })
}

/// Turn one native-runtime artifact into a fetchable item.
///
/// Infallible where [`item_for`] is not, and that is a property of the schema
/// rather than an omission: a `native-runtime` artifact carries a URL and an
/// archive digest as required fields, so there is no loose-file form and no
/// "pinned but not downloadable" case to report. The `Option` that
/// [`Pack::archive`] returns has no counterpart here.
fn item_for_runtime(source: NativeRuntimeSource<'_>, downloads: &Path) -> Item {
    let spec = InstallSpec::from(source);
    Item {
        label: runtime_label(source.id),
        bytes: source.archive_bytes,
        payload: Payload::Archive(DownloadRequest {
            url: source.url.to_owned(),
            // The same `{id}-{revision}.archive` shape the packs use, so the
            // resumable `.part` beside it is found again by a second run of
            // setup. `version` is the revision for an artifact — see
            // `InstallSpec`'s conversion.
            destination: downloads.join(format!("{}-{}.archive", source.id, source.version)),
            expected_bytes: source.archive_bytes,
            expected_sha256: source.archive_sha256.to_owned(),
        }),
        spec,
    }
}

/// What to call one artifact of the graphics-card payload.
///
/// Matched on the id, and by substring for the two NVIDIA archives because their
/// ids carry a version that moves — `nvidia-cuda-cudart-windows-x64-13.3.29`
/// became that from a 12.9 spelling, and a label keyed to the whole string would
/// have silently become the fallback. The component name is the part that does
/// not move.
///
/// The fallback is deliberately generic and deliberately reachable: a catalog
/// that pins a third redistributable gets an honest vague label rather than a
/// confident wrong one, and a test refuses to let the shipped catalog reach it.
fn runtime_label(id: &str) -> &'static str {
    if id == GRANITE_CUDA_WORKER_ARTIFACT_ID {
        catalog::ARTIFACT_GPU_ENGINE
    } else if id.contains("cudart") {
        catalog::ARTIFACT_GPU_CUDA_RUNTIME
    } else if id.contains("cublas") {
        catalog::ARTIFACT_GPU_MATH_LIBRARY
    } else {
        catalog::ARTIFACT_GPU_SUPPORT_LIBRARY
    }
}

/// Put a downloaded graphics-card payload beside the app's worker.
///
/// Called after [`crate::install::perform`] and not before, and the order is the
/// whole reason this is a separate step. `perform` merges the payload tree over
/// the install root, and the payload carries the **CPU** worker under the same
/// name — so a CUDA worker placed first is overwritten by the copy, silently,
/// and the engine check then proves the processor on a machine that asked for
/// and downloaded the card. That is exactly the failure
/// `scripts/Enable-GraniteCuda.ps1` warned about for a reinstall, which this
/// step also fixes: an upgrade re-lays the CPU worker and this puts the CUDA one
/// back, every time, instead of leaving Granite silently on the processor.
///
/// Copied out of the installed artifacts rather than moved: they stay under
/// `model-lifecycle/models`, verified and re-usable, so a repair or an upgrade
/// re-stages without a download. Flattened to base names, because Windows
/// resolves a DLL against the loading process's own directory and nowhere else —
/// the same reduction [`speakeasy_models::required_cuda_runtime_files`] makes,
/// which is what lets the check and the placement agree about the file names.
///
/// `Ok(false)` means there was nothing to stage: a processor installation, or a
/// release with no graphics-card configuration in it. Not an error, and not
/// silent either — the caller has the answer and the engine check reports what
/// it actually found afterwards.
///
/// # Errors
///
/// Returns a catalog message when an artifact this release publishes was
/// expected on disk and is not, or when a file cannot be copied. Deliberately
/// **not** best-effort: a partial payload is the one state that fails ~36 s into
/// a dictation rather than at startup, so setup would rather say it could not
/// finish than leave that behind.
pub fn stage_graphics_card_payload(
    provider: ExecutionProvider,
    install_root: &Path,
) -> Result<bool, Failure> {
    if provider != ExecutionProvider::Cuda {
        // The answer, and not merely what is on the disk. A machine that had a
        // graphics-card install before this one still has the artifacts under
        // `model-lifecycle` — an uninstall with `--keep-user-data` keeps them,
        // and so does installing over an existing profile — so presence alone
        // would stage a CUDA worker onto an installation whose owner had just
        // chosen the processor, and the engine check would then dutifully prove
        // and record the card. Nothing would report it, because nothing would be
        // wrong: every layer would be describing what it found.
        return Ok(false);
    }
    let Ok(manifest) = bundled_manifest() else {
        // Unreadable catalog is reported by the download step, which runs first
        // and could not have planned anything. Saying it twice, on a later page,
        // would read as a second fault.
        return Ok(false);
    };
    let sources = graphics_card_payload_sources(&manifest);
    if sources.is_empty() {
        return Ok(false);
    }
    let root = model_lifecycle_root().ok_or_else(|| catalog::DATA_ROOT_UNLOCATABLE.to_owned())?;
    let models = root.join("models");
    let manager = InstallManager::new(&models);
    let proof = install_root.join("proof");

    // Asked of all of them before any of them is copied. None present is not an
    // error: a user can reach this page having cancelled the transfer, and the
    // engine check that follows reports the processor with the reason rather
    // than this step guessing at one.
    let specs: Vec<InstallSpec> = sources.into_iter().map(InstallSpec::from).collect();
    if !specs.iter().any(|spec| manager.is_present(spec)) {
        return Ok(false);
    }
    std::fs::create_dir_all(&proof).map_err(|error| {
        catalog::gpu_staging_failed(catalog::ARTIFACT_GPU_ENGINE, &error.to_string())
    })?;
    // **The worker goes last, and the order is the safety property.** Every
    // failure here leaves the install root part-way through, and the two
    // orderings leave very different machines behind. Libraries first: a failure
    // then leaves the processor worker the payload placed, with some unused DLLs
    // beside it that nothing loads — an installation that works, which is what
    // this step's own failure message promises. Worker first: a failure leaves a
    // CUDA worker with no libraries, and that does not run slower, it does not
    // start at all, and Windows names no file. `graphics_card_payload_sources`
    // returns the worker first because that is the order to *fetch* in; this is
    // the order to *place* in, and they are not the same question.
    for spec in specs.iter().rev() {
        let label = runtime_label(&spec.id);
        if !manager.is_present(spec) {
            // One half arrived and the other did not. Named rather than
            // shrugged at: this is the state that starts and then fails at the
            // first matmul.
            return Err(catalog::gpu_staging_failed(
                label,
                &format!("{} was not installed.", spec.id),
            ));
        }
        let installed = models.join(&spec.id).join(&spec.revision);
        for file in &spec.required_files {
            let Some(name) = file.path.file_name() else {
                continue;
            };
            place_beside_the_worker(&installed.join(&file.path), &proof.join(name))
                .map_err(|error| catalog::gpu_staging_failed(label, &error))?;
        }
    }
    Ok(true)
}

/// Copy one file into `proof/`, atomically as far as any reader is concerned.
///
/// Through a temporary beside the destination and a rename, rather than
/// [`std::fs::copy`] straight over it. A copy that fails half way leaves a
/// **truncated** file under the real name, and the file this matters most for is
/// `granite-worker.exe`: half a CUDA worker is neither the CUDA worker nor the
/// processor one the payload placed, and it fails in the shape that names no
/// file. A rename either happened or did not.
///
/// The temporary is removed on failure so a retry — pressing Next again — does
/// not accumulate them, and it sits in the destination directory rather than
/// `%TEMP%` because a rename across volumes is not a rename and Windows refuses
/// it rather than silently copying.
fn place_beside_the_worker(source: &Path, destination: &Path) -> Result<(), String> {
    let mut staging = destination.to_path_buf().into_os_string();
    staging.push(".incoming");
    let staging = PathBuf::from(staging);
    let outcome = std::fs::copy(source, &staging)
        .and_then(|_| std::fs::rename(&staging, destination))
        .map_err(|error| format!("{}: {error}", destination.display()));
    if outcome.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    outcome
}

fn label_for(pack: &Pack) -> &'static str {
    match pack.role() {
        PackRole::StreamingAsr => catalog::ARTIFACT_STREAMING,
        _ => catalog::ARTIFACT_GRANITE,
    }
}

/// The download policy setup uses.
///
/// The same hosts, deadlines and retry count the app uses for exactly these
/// artifacts (`model_download_policy` in `apps/desktop`). Duplicated rather than
/// shared for now because the two crates do not otherwise depend on each other;
/// `the_policy_matches_the_app` pins them together so a host added on one side
/// and not the other fails the gate rather than a user's download.
fn policy() -> DownloadPolicy {
    DownloadPolicy {
        redirect_hosts: vec![
            "github.com".to_owned(),
            "release-assets.githubusercontent.com".to_owned(),
            "developer.download.nvidia.com".to_owned(),
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

/// What the window shows while a run is in flight.
///
/// Shared with the worker thread, read on a timer by the UI thread. Atomics
/// rather than a mutex around the whole thing so a poll can never block the
/// message loop behind a thread that is mid-write.
#[derive(Default)]
pub struct Progress {
    /// Bytes belonging to items that finished. The in-flight item's own bytes
    /// are measured from its partial file instead, because `download_to_file`
    /// reports nothing until it returns.
    completed_bytes: AtomicU64,
    /// Index into the plan's items, for "2 of 3".
    current: AtomicUsize,
    /// Re-checking an artifact that is already fully on disk.
    ///
    /// Its own state because it is neither of the other two and it is not brief:
    /// `download_to_file` short-circuits a complete destination by digesting it,
    /// which for the 2.3 GB streaming archive measured 24 seconds. Reported as
    /// "Downloading — 0 MB of 4.4 GB" it was a false statement twice over, and it
    /// is the exact reading a user takes as a stalled transfer.
    verifying: AtomicBool,
    installing: AtomicBool,
    finished: AtomicBool,
    /// The partial file of whatever is transferring now.
    partial: Mutex<Option<PathBuf>>,
    failure: Mutex<Option<Failure>>,
}

impl Progress {
    /// Bytes transferred so far, including the part of the current file.
    ///
    /// The in-flight figure is the size of the `.part` file on disk. That is how
    /// `apps/desktop` reports it too, and it is the only source available: the
    /// download function takes no progress callback, and adding one would mean
    /// changing a crate the app depends on for a wizard's progress bar.
    ///
    /// Saturating, and never decreasing within an item, because a resumed
    /// download's partial file starts at the size it reached last time — which
    /// is the point of it.
    pub fn bytes(&self) -> u64 {
        let in_flight = self
            .partial
            .lock()
            .ok()
            .and_then(|path| path.clone())
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |metadata| metadata.len());
        self.completed_bytes
            .load(Ordering::Relaxed)
            .saturating_add(in_flight)
    }

    pub fn current(&self) -> usize {
        self.current.load(Ordering::Relaxed)
    }

    /// Whether the current item is being extracted and verified rather than
    /// transferred.
    ///
    /// Worth its own state because it is where a long silence happens: a 453 MB
    /// archive expands to 653 MB and every file in it is digested, with the
    /// progress bar unable to move. A user watching a frozen bar decides setup
    /// has hung, and they are the only one who can tell us otherwise.
    pub fn installing(&self) -> bool {
        self.installing.load(Ordering::Relaxed)
    }

    pub fn verifying(&self) -> bool {
        self.verifying.load(Ordering::Relaxed)
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn failure(&self) -> Option<Failure> {
        self.failure.lock().ok().and_then(|slot| slot.clone())
    }
}

/// A download in flight.
pub struct Run {
    pub progress: Arc<Progress>,
    cancel: CancelToken,
    pub total_bytes: u64,
    pub labels: Vec<&'static str>,
}

impl Run {
    /// Stop the transfer.
    ///
    /// Safe to lose bytes over, because none are lost: the partial file and its
    /// resume metadata are written as the transfer goes, so a cancelled run
    /// continues from where it stopped rather than starting again. That is the
    /// property this whole module exists to preserve, and the one most likely to
    /// be faked, so it is worth stating where it is relied on.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// Start fetching, on a worker thread.
///
/// A thread rather than the message loop, and this is not a preference. The
/// install step already blocks the loop while it copies, which is tolerable for
/// a few seconds of local file copying; blocking it for a multi-gigabyte
/// transfer would give Windows a window that does not answer, which it paints
/// over with "Not Responding" and offers to kill.
pub fn start(plan: Plan) -> Run {
    let cancel = CancelToken::default();
    let progress = Arc::new(Progress::default());
    let run = Run {
        progress: Arc::clone(&progress),
        cancel: cancel.clone(),
        total_bytes: plan.total_bytes,
        labels: plan.items.iter().map(|item| item.label).collect(),
    };
    let worker_cancel = cancel;
    std::thread::spawn(move || {
        let outcome = execute(&plan, &progress, &worker_cancel);
        if let Err(failure) = outcome
            && let Ok(mut slot) = progress.failure.lock()
        {
            *slot = Some(failure);
        }
        // Set last, and after the failure slot, so a UI poll that sees
        // `finished` can trust `failure` to have already been written. The
        // opposite order reports a successful finish for one poll interval and
        // then contradicts itself.
        progress.finished.store(true, Ordering::Relaxed);
    });
    run
}

fn execute(plan: &Plan, progress: &Progress, cancel: &CancelToken) -> Result<(), Failure> {
    let policy = policy();
    let manager = InstallManager::new(plan.root.join("models"));
    let mut completed = 0_u64;

    for (index, item) in plan.items.iter().enumerate() {
        progress.current.store(index, Ordering::Relaxed);
        progress.installing.store(false, Ordering::Relaxed);

        // Already there and verified: transfer nothing. A user who runs setup
        // twice, or who cancelled after the first artifact, must not be charged
        // for the first one again.
        if manager.is_present(&item.spec) {
            completed = completed.saturating_add(item.bytes);
            progress.completed_bytes.store(completed, Ordering::Relaxed);
            continue;
        }

        for request in item.payload.requests() {
            if let Ok(mut slot) = progress.partial.lock() {
                *slot = Some(partial_path(&request.destination));
            }
            // Decided before the call rather than reported by it: this is the
            // one condition under which `download_to_file` transfers nothing and
            // spends its whole time digesting, and it is the same condition the
            // function itself tests on entry. Reading it here costs one `stat`
            // and is the difference between saying what is happening and saying
            // the opposite of it.
            progress
                .verifying
                .store(is_already_complete(request), Ordering::Relaxed);
            download_to_file(request, &policy, cancel)
                .map_err(|error| catalog::download_failed(item.label, &error.to_string()))?;
            progress.verifying.store(false, Ordering::Relaxed);
            completed = completed.saturating_add(request.expected_bytes);
            progress.completed_bytes.store(completed, Ordering::Relaxed);
        }
        if let Ok(mut slot) = progress.partial.lock() {
            *slot = None;
        }

        progress.installing.store(true, Ordering::Relaxed);
        install(&manager, item, cancel)
            .map_err(|error| catalog::install_of_artifact_failed(item.label, &error))?;
    }
    progress.installing.store(false, Ordering::Relaxed);
    progress.current.store(plan.items.len(), Ordering::Relaxed);
    Ok(())
}

fn install(manager: &InstallManager, item: &Item, cancel: &CancelToken) -> Result<(), String> {
    match &item.payload {
        Payload::Archive(request) => manager
            .install_archive(&item.spec, &request.destination, cancel)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Payload::Loose(files) => {
            let staged = files
                .iter()
                .map(|(relative, request)| LooseInstallFile {
                    path: relative.clone(),
                    source: request.destination.clone(),
                })
                .collect::<Vec<_>>();
            manager
                .install_loose_files(&item.spec, &staged, cancel)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
    }
}

/// Whether the destination is already the full artifact, so the coming call
/// will digest rather than transfer.
///
/// Length only, not the digest: computing the hash here to decide what to say
/// about computing the hash would double the cost of the very thing being
/// described. A file of the right length that fails its digest is re-fetched by
/// `download_to_file` anyway, and the phase then corrects itself on the next
/// poll.
fn is_already_complete(request: &DownloadRequest) -> bool {
    std::fs::metadata(&request.destination)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == request.expected_bytes)
}

/// The partial file `download_to_file` writes beside a destination.
///
/// Appended rather than `with_extension`, which would treat the `.5` in
/// `nemotron-3.5-...` as the extension boundary and produce a path that never
/// exists — the progress bar would then sit at zero for a 453 MB transfer and
/// jump to full, which reads exactly like a download that did not resume.
fn partial_path(destination: &Path) -> PathBuf {
    let mut path = destination.to_path_buf().into_os_string();
    path.push(".part");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_root_agrees_with_the_app() {
        // `composition.rs` builds `app_data_dir().join("model-lifecycle")` and
        // hands `InstallManager` `root.join("models")`. If these two ever
        // disagree, setup downloads gigabytes into a directory the app does not
        // read, reports success, and the first launch downloads them again --
        // with nothing anywhere reporting a fault.
        let Some(root) = model_lifecycle_root() else {
            // No APPDATA is not a machine this product runs on, and a test that
            // silently passes on one would be worse than absent.
            panic!("APPDATA must resolve on any machine this runs on");
        };
        assert!(
            root.ends_with("ai.speakeasy.mini\\model-lifecycle"),
            "{root:?}"
        );
    }

    #[test]
    fn a_partial_path_appends_rather_than_replacing_an_extension() {
        // The pack ids carry a dot in their version. `with_extension` would turn
        // `nemotron-3.5-streaming-en-cpu-560ms.archive` into
        // `nemotron-3.part`, and the progress read would find nothing.
        let destination = Path::new(r"C:\downloads\nemotron-3.5-streaming-en-cpu-560ms.archive");
        assert_eq!(
            partial_path(destination),
            PathBuf::from(r"C:\downloads\nemotron-3.5-streaming-en-cpu-560ms.archive.part")
        );
    }

    /// One engine, so one item — and the count is the assertion that will
    /// notice when that stops being true.
    ///
    /// This test demanded two items, "one streaming pack and one Granite pack",
    /// until 2026-08-18. It had been failing since the fork removed the
    /// streaming engine, and nobody saw it: it lives in the bootstrapper's
    /// **binary** target, and every command in `docs/handoff/CURRENT.md` ran
    /// `cargo test --workspace --lib`, which builds no `--bin` targets at all.
    /// The same shape as the recorded "a whole crate went red unnoticed", one
    /// level down — a target filter rather than a crate list.
    ///
    /// The second item is no longer a future state in the *code* — `plan` reads
    /// `provider` now — but it is still one in the *catalog*, because nothing is
    /// published. So a graphics-card plan is one item today, for a stated reason
    /// rather than by omission, and
    /// `the_graphics_card_plan_fetches_the_worker_and_its_libraries` exercises
    /// the other half against a simulated pin.
    #[test]
    fn the_plan_names_one_engine_and_totals_its_transfer_size() {
        // Not shadowed as `plan`: this test calls the function twice, and a
        // binding of the same name makes the second call a type error.
        let cpu = plan(ExecutionProvider::Cpu).expect("the bundled manifest must yield a plan");
        assert_eq!(cpu.items.len(), 1, "one Granite pack, and nothing else");
        assert_eq!(
            cpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            vec![catalog::ARTIFACT_GRANITE]
        );

        // And a graphics-card machine gets more, which is the whole point of the
        // provider being read. This assertion was the reverse — "nothing
        // published means nothing extra to fetch" — until the worker was pinned
        // on 2026-08-26, and it inverting is what said so.
        let gpu = plan(ExecutionProvider::Cuda).expect("a GPU machine must also yield a plan");
        assert!(
            gpu.items.len() > cpu.items.len(),
            "a published worker means a graphics-card machine fetches more: {:?}",
            gpu.items.iter().map(|item| item.label).collect::<Vec<_>>()
        );
        assert_eq!(
            cpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            vec![catalog::ARTIFACT_GRANITE],
            "and asking for the processor still fetches the weights alone"
        );

        // Transfer size, not installed size. Counting the larger figure would
        // leave the bar short of the end when the transfer actually finished.
        assert_eq!(
            cpu.total_bytes,
            cpu.items.iter().map(|item| item.bytes).sum::<u64>()
        );
        assert!(cpu.total_bytes > 0);
    }

    /// What a graphics-card machine will fetch on the day the worker is pinned.
    ///
    /// Proved by simulating the pin, because that is the only instrument that
    /// works before publication and because the last thing this path got wrong
    /// was invisible on a machine that already had a CUDA worker staged by hand.
    /// This one is the same shape: the developer machine cannot tell a plan that
    /// reads `provider` from one that ignores it, since both produce one item
    /// against the shipped catalog.
    #[test]
    fn the_graphics_card_plan_fetches_the_worker_and_its_libraries() {
        let manifest = staged_manifest_publishing_the_cuda_worker();
        let root = std::env::temp_dir().join("speakeasy-plan-simulated-pin");

        let cpu = plan_from(&manifest, ExecutionProvider::Cpu, root.clone())
            .expect("a processor plan must be buildable");
        assert_eq!(
            cpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            vec![catalog::ARTIFACT_GRANITE],
            "asking for the processor must not fetch a graphics-card payload, whatever \
             the catalog publishes"
        );

        let gpu = plan_from(&manifest, ExecutionProvider::Cuda, root)
            .expect("a graphics-card plan must be buildable");
        assert_eq!(
            gpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            vec![
                catalog::ARTIFACT_GRANITE,
                catalog::ARTIFACT_GPU_ENGINE,
                catalog::ARTIFACT_GPU_CUDA_RUNTIME,
                catalog::ARTIFACT_GPU_MATH_LIBRARY,
            ],
            "the weights, then the engine, then the libraries it loads"
        );
        assert!(
            gpu.total_bytes > cpu.total_bytes,
            "a graphics-card install transfers more, and the step states the total \
             before the user commits to it"
        );

        // Every extra item is a verifiable archive with somewhere to put it. A
        // request with no digest downloads and installs whatever arrives.
        for item in gpu.items.iter().skip(1) {
            for request in item.payload.requests() {
                assert_eq!(request.expected_sha256.len(), 64, "{}", request.url);
                assert!(request.expected_bytes > 0, "{}", request.url);
                assert!(
                    request.destination.starts_with(&gpu.root),
                    "downloads belong under the app's own model root: {:?}",
                    request.destination
                );
            }
            assert!(
                !item.spec.required_files.is_empty(),
                "an artifact with no required files installs nothing and reports success"
            );
        }
    }

    /// A payload that was never downloaded stages nothing, and creates nothing.
    ///
    /// Two cases that must both be quiet, and for different reasons. **The
    /// processor** is the answer being honoured: this machine may well have the
    /// artifacts installed under `model-lifecycle` from an earlier
    /// graphics-card install, and staging them onto an installation whose owner
    /// just chose the processor would be silently overriding them — the engine
    /// check would then prove and record the card, with nothing anywhere
    /// reporting it. **The graphics card with nothing fetched** is a cancelled
    /// or interrupted transfer, which is not this step's to diagnose; the engine
    /// check that follows names it.
    ///
    /// Either way nothing is *created*. An install root that comes back with an
    /// empty `proof/` in it — a `create_dir_all` before the question is asked —
    /// is how a machine ends up carrying the shape of a configuration it does not
    /// have, and `unrecognised_proof_files` then has a directory to explain.
    ///
    /// This test used to assert the same silence because the release published
    /// no worker at all. It has published one since 2026-08-26, so the silence
    /// now comes from the two conditions above rather than from an empty
    /// catalog, and it is a stronger check for it.
    #[test]
    fn a_payload_that_was_not_downloaded_stages_nothing_and_creates_nothing() {
        let root = std::env::temp_dir().join("speakeasy-stage-gpu-nothing-fetched");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("an install root to stage into");

        assert_eq!(
            stage_graphics_card_payload(ExecutionProvider::Cpu, &root),
            Ok(false),
            "the answer was the processor, whatever is on this disk"
        );
        assert!(
            !root.join("proof").exists(),
            "the processor answer must be given before any directory is made"
        );

        // The graphics-card half is only meaningful on a machine that has not
        // installed the artifacts, which is every machine but one mid-way
        // through a real setup run. Skipped rather than faked where they are
        // present: asserting `Ok(false)` there would be asserting the opposite
        // of what this function is for.
        let installed = model_lifecycle_root().map(|root| {
            let manager = InstallManager::new(root.join("models"));
            let manifest = bundled_manifest().expect("the bundled manifest must parse");
            graphics_card_payload_sources(&manifest)
                .into_iter()
                .map(InstallSpec::from)
                .any(|spec| manager.is_present(&spec))
        });
        if installed == Some(false) {
            assert_eq!(
                stage_graphics_card_payload(ExecutionProvider::Cuda, &root),
                Ok(false),
                "nothing was fetched, so there is nothing to stage and no fault to report"
            );
            assert!(!root.join("proof").exists(), "and still nothing is created");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file arrives whole or not at all, and leaves no litter either way.
    ///
    /// The case this is for is `granite-worker.exe`. A [`std::fs::copy`] that
    /// fails part way leaves a truncated file under the real name, and half a
    /// CUDA worker is neither the CUDA worker nor the processor one the payload
    /// placed — it fails to start, and Windows' error for that names no file.
    /// The unreadable-source half is the one that can be provoked: a directory
    /// where a file should be.
    #[test]
    fn a_staged_file_arrives_whole_and_leaves_no_temporary() {
        let root = std::env::temp_dir().join("speakeasy-stage-atomically");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a staging root");
        let destination = root.join("granite-worker.exe");
        std::fs::write(&destination, b"the processor worker").expect("the file being replaced");

        let source = root.join("incoming.exe");
        std::fs::write(&source, b"the graphics-card worker").expect("a source file");
        assert_eq!(place_beside_the_worker(&source, &destination), Ok(()));
        assert_eq!(
            std::fs::read(&destination).expect("the replaced file"),
            b"the graphics-card worker"
        );

        // And a failure leaves the previous file untouched, with no `.incoming`
        // beside it for `unrecognised_proof_files` to ask the user about.
        let unreadable = root.join("a-directory-not-a-file");
        std::fs::create_dir_all(&unreadable).expect("an unreadable source");
        assert!(place_beside_the_worker(&unreadable, &destination).is_err());
        assert_eq!(
            std::fs::read(&destination).expect("the file must survive"),
            b"the graphics-card worker"
        );
        assert!(
            !root.join("granite-worker.exe.incoming").exists(),
            "the temporary must not survive a failure"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two artifacts must never print the same line.
    ///
    /// The download step lists these one per line and names one per progress
    /// line, so a shared label prints the same sentence twice — which reads as
    /// setup having lost count. The fallback label exists for a catalog that
    /// pins something these names do not cover, and reaching it on the *shipped*
    /// catalog means a re-pin renamed an artifact out from under the mapping.
    #[test]
    fn every_graphics_card_artifact_gets_its_own_name() {
        let manifest = staged_manifest_publishing_the_cuda_worker();
        let labels: Vec<&str> = graphics_card_payload_sources(&manifest)
            .iter()
            .map(|source| runtime_label(source.id))
            .collect();

        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "duplicate labels: {labels:?}");
        assert!(
            !labels.contains(&catalog::ARTIFACT_GPU_SUPPORT_LIBRARY),
            "an artifact this catalog pins fell through to the generic name: {labels:?}"
        );
    }

    /// A catalog that publishes a CUDA Granite worker.
    ///
    /// **This no longer stages anything.** It spliced a renamed copy of another
    /// artifact in, until 2026-08-26, because none was published and the tests
    /// above had nothing real to plan from. One is published now, so the shipped
    /// catalog *is* the fixture — and pushing a second entry under the same id
    /// would make `TrustedManifest::parse` refuse the whole document, which is
    /// how these two tests announced the change rather than quietly passing
    /// against a forgery.
    ///
    /// The `serde_json` dev-dependency this needed went with it. What replaced
    /// it is a real assertion: the premise is checked rather than constructed,
    /// so a re-pin that renames the artifact fails here instead of silently
    /// planning one item.
    fn staged_manifest_publishing_the_cuda_worker() -> speakeasy_models::TrustedManifest {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        assert!(
            gpu_configuration_is_installable(&manifest).is_ok(),
            "this fixture's whole premise is a published worker"
        );
        manifest
    }

    #[test]
    fn every_planned_url_is_one_the_policy_would_follow() {
        // A pinned artifact whose host is not in the redirect allow list fails
        // at transfer time, on a user's machine, after setup has already
        // promised to fetch it. The manifest and the policy are edited in
        // different files by different changes, so nothing else pairs them.
        let policy = policy();
        for provider in [ExecutionProvider::Cpu, ExecutionProvider::Cuda] {
            let Ok(plan) = plan(provider) else { continue };
            for item in &plan.items {
                for request in item.payload.requests() {
                    let host = request
                        .url
                        .split('/')
                        .nth(2)
                        .expect("a pinned url must have a host");
                    assert!(
                        policy.redirect_hosts.iter().any(|allowed| allowed == host),
                        "{host} is pinned in the manifest but not allowed by the download policy"
                    );
                }
            }
        }
    }
}
