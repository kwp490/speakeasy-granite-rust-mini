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
    DownloadPolicy, DownloadRequest, ExecutionProvider, InstallManager, InstallSpec,
    LooseInstallFile, Pack, PackRole, bundled_manifest, download_to_file,
};

use crate::{catalog, uninstall};

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
/// What the provider will decide is whether the CUDA worker and its two
/// libraries are fetched alongside the weights. That fetch is not wired yet —
/// the worker has to be published and pinned by digest first — so today this
/// plans the weights alone and a GPU machine gets the same list as a CPU one.
///
/// # Errors
///
/// Returns a catalog message when the manifest cannot be parsed, when no
/// install-eligible pack fills a role on the wanted provider, or when the app's
/// data directory cannot be located.
pub fn plan(provider: ExecutionProvider) -> Result<Plan, Failure> {
    let manifest = bundled_manifest().map_err(|_| catalog::CATALOG_UNAVAILABLE.to_owned())?;
    let root = model_lifecycle_root().ok_or_else(|| catalog::DATA_ROOT_UNLOCATABLE.to_owned())?;
    let downloads = root.join("downloads");

    // One item today. The GPU worker and its two CUDA libraries join it here
    // once they are published and pinned by digest, which is why `provider` is
    // taken and not yet read — the weights are the same file either way.
    let _ = provider;
    let pack = manifest
        .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
        .map_err(|error| {
            catalog::pack_unavailable(catalog::ARTIFACT_GRANITE, &error.to_string())
        })?;
    let items = vec![item_for(pack, &downloads)?];

    let total_bytes = items.iter().map(|item| item.bytes).sum();
    Ok(Plan {
        items,
        total_bytes,
        root,
    })
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
    /// property this whole module exists to preserve, and the one the brief says
    /// is most likely to be faked, so it is worth stating where it is relied on.
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
    /// The second item is a real future state rather than a thing that was
    /// deleted: `plan` takes `provider` and deliberately ignores it, waiting on
    /// the CUDA worker being published and pinned by digest (item 3 in the
    /// handoff). When that lands this count becomes 2 and the label list gains
    /// the worker beside the weights.
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

        // A GPU machine gets the same list today, and that is the current
        // decision rather than an oversight — the weights are the same file
        // either way and the CUDA worker is unpublished. When that changes,
        // this is where the divergence has to show up.
        let gpu = plan(ExecutionProvider::Cuda).expect("a GPU machine must also yield a plan");
        assert_eq!(
            gpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            cpu.items.iter().map(|item| item.label).collect::<Vec<_>>(),
            "the provider does not select a different model yet"
        );

        // Transfer size, not installed size. Counting the larger figure would
        // leave the bar short of the end when the transfer actually finished.
        assert_eq!(
            cpu.total_bytes,
            cpu.items.iter().map(|item| item.bytes).sum::<u64>()
        );
        assert!(cpu.total_bytes > 0);
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
