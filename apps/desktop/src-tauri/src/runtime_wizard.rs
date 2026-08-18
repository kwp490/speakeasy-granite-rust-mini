use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use speakeasy_domain::CancelToken;
use speakeasy_windows::CrashThrottle;

/// What an ordinary dictation needs: the streaming worker's resident model,
/// the Tauri host, and the operating system underneath both.
///
/// Raised from 2 GiB to 4 GiB in Phase 9 of
/// `docs/handoff/granite-final-pass.md`. The old number was stale against
/// measurement, not just against Granite: `inference-worker.exe`'s own
/// resident working set on this machine is **1,263 MiB**, sampled through a
/// full 12-clip transcription run, so the 2 GiB floor was under twice the
/// streaming worker's own cost before the host process or Windows itself were
/// counted at all. 4 GiB leaves the measured worker roughly its own size again
/// in headroom. The host and OS shares are *not* measured — this rig is not
/// the packaged app — so this is a defensible raise, not a derived optimum.
///
/// Deliberately **not** Granite's floor. Granite is a second resident
/// ~3.1 GiB process, and gating the whole of `run_retained_transcription` on
/// what *it* needs would take dictation away from machines that run the
/// streaming path perfectly well — this gate gets asked before the engine is
/// chosen, so a raise here refuses the dictation outright rather than
/// declining the second pass. Granite's own floor lives with Granite, in
/// `granite_engine::GRANITE_MINIMUM_TOTAL_MEMORY_BYTES`, where falling short
/// costs the user a second pass instead of their transcript.
const MINIMUM_TOTAL_MEMORY_BYTES: u64 = 4 * 1_024 * 1_024 * 1_024;
const LOCAL_POLISH_MINIMUM_MEMORY_BYTES: u64 = 8 * 1_024 * 1_024 * 1_024;

/// The dictation floor, for the one caller outside this module that has to
/// reason about it: `granite_engine` asserts its own, higher floor stays above
/// this one, because the moment that ordering inverts the split stops meaning
/// anything and nobody would notice.
#[cfg(test)]
pub const fn minimum_total_memory_bytes() -> u64 {
    MINIMUM_TOTAL_MEMORY_BYTES
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeRole {
    FinalAsr,
    /// Memory-budget gate for a local LLM polish engine. No local inference
    /// engine is wired in; only `FinalAsr` runs in the shipped app today.
    #[allow(dead_code)]
    LocalPolish,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub root: PathBuf,
    /// The directory holding the worker and every native library it loads.
    ///
    /// Named separately from `root` because it is where the CUDA runtime has to
    /// be installed, and *that* is forced rather than chosen: Windows resolves a
    /// dynamically loaded DLL's dependencies against the process search order,
    /// so cuBLAS and cuDNN must be in the worker's own directory.
    pub proof: PathBuf,
    /// The Granite worker binary. **Required**, and the inversion is the whole
    /// point of the fork: this was `Option`, "never in `required`", because
    /// Granite used to be a second final-pass engine layered over a streaming
    /// one that ordinary dictation actually depended on. It is now the only
    /// engine, so its absence is not a declined second pass — it is no
    /// dictation at all, and saying so here is what makes the failure legible.
    ///
    /// The three fields that were required alongside it — the streaming
    /// worker, ONNX Runtime and sherpa's C API — went with that engine. They
    /// outlived it in this struct, and because they are resolved before
    /// `granite_worker` and no longer exist to resolve, `paths()` returned
    /// `runtime_resources_unavailable` on every call: every dictation would
    /// have failed, in the one code path no test covers and the app had never
    /// been launched to exercise. ONNX Runtime's CUDA provider left with them;
    /// Granite's GPU support is llama.cpp's, and needs cudart and cuBLAS in
    /// `proof` rather than an execution-provider DLL.
    pub granite_worker: PathBuf,
}

pub struct RuntimeWizardCoordinator {
    resource_root: PathBuf,
    active: Mutex<Option<(RuntimeRole, CancelToken)>>,
    crashes: Mutex<CrashThrottle>,
    started_at: Instant,
}

impl RuntimeWizardCoordinator {
    pub fn new(resource_root: PathBuf) -> Self {
        Self {
            resource_root,
            active: Mutex::new(None),
            crashes: Mutex::new(
                CrashThrottle::new(3, Duration::from_mins(1))
                    .expect("static crash policy must be valid"),
            ),
            started_at: Instant::now(),
        }
    }

    pub fn begin(&self, total_memory_bytes: Option<u64>) -> Result<CancelToken, &'static str> {
        self.begin_role(RuntimeRole::FinalAsr, total_memory_bytes)
    }

    pub fn begin_role(
        &self,
        role: RuntimeRole,
        total_memory_bytes: Option<u64>,
    ) -> Result<CancelToken, &'static str> {
        if self
            .crashes
            .lock()
            .map_err(|_| "runtime_state_unavailable")?
            .is_quarantined()
        {
            return Err("runtime_worker_quarantined");
        }
        let required = match role {
            RuntimeRole::FinalAsr => MINIMUM_TOTAL_MEMORY_BYTES,
            RuntimeRole::LocalPolish => LOCAL_POLISH_MINIMUM_MEMORY_BYTES,
        };
        if total_memory_bytes.is_none_or(|bytes| bytes < required) {
            return Err("runtime_memory_budget_unavailable");
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| "runtime_state_unavailable")?;
        if active.is_some() {
            return Err("runtime_busy");
        }
        let cancel = CancelToken::default();
        *active = Some((role, cancel.clone()));
        Ok(cancel)
    }

    pub fn cancel(&self) -> Result<(), &'static str> {
        let active = self
            .active
            .lock()
            .map_err(|_| "runtime_state_unavailable")?;
        active.as_ref().ok_or("runtime_not_active")?.1.cancel();
        Ok(())
    }

    pub fn finish(&self) {
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }

    pub fn record_worker_failure(&self) {
        if let Ok(mut crashes) = self.crashes.lock() {
            let _ = crashes.record_crash(self.started_at.elapsed());
        }
    }

    pub fn recover_manually(&self) -> Result<(), &'static str> {
        if self
            .active
            .lock()
            .map_err(|_| "runtime_state_unavailable")?
            .is_some()
        {
            return Err("runtime_busy");
        }
        self.crashes
            .lock()
            .map_err(|_| "runtime_state_unavailable")?
            .reset();
        Ok(())
    }

    pub fn paths(&self) -> Result<RuntimePaths, &'static str> {
        let root = canonical_directory(&self.resource_root)?;
        let paths = RuntimePaths {
            proof: canonical_directory(&root.join("proof"))?,
            granite_worker: canonical_file(&root, "proof/granite-worker.exe")?,
            root: root.clone(),
        };
        // Canonicalization resolves symlinks and `..`, so this is asked after
        // it rather than of the joined path: the check is whether the file the
        // OS would actually open still lies under the resource root.
        if !paths.granite_worker.starts_with(&root) {
            return Err("runtime_resource_escape");
        }
        Ok(paths)
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, &'static str> {
    path.canonicalize()
        .map_err(|_| "runtime_resources_unavailable")
        .and_then(|path| {
            path.is_dir()
                .then_some(path)
                .ok_or("runtime_resources_unavailable")
        })
}

fn canonical_file(root: &Path, relative: &str) -> Result<PathBuf, &'static str> {
    root.join(relative)
        .canonicalize()
        .map_err(|_| "runtime_resources_unavailable")
        .and_then(|path| {
            path.is_file()
                .then_some(path)
                .ok_or("runtime_resources_unavailable")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_serializes_and_enforces_memory_floor() {
        let scheduler = RuntimeWizardCoordinator::new(PathBuf::from("missing"));
        assert_eq!(
            scheduler
                .begin(Some(MINIMUM_TOTAL_MEMORY_BYTES - 1))
                .expect_err("insufficient memory must fail"),
            "runtime_memory_budget_unavailable"
        );
        let first = scheduler
            .begin(Some(MINIMUM_TOTAL_MEMORY_BYTES))
            .expect("first lease");
        assert_eq!(
            scheduler
                .begin(Some(MINIMUM_TOTAL_MEMORY_BYTES))
                .expect_err("second lease must fail"),
            "runtime_busy"
        );
        first.cancel();
        scheduler.finish();
        assert!(scheduler.begin(Some(MINIMUM_TOTAL_MEMORY_BYTES)).is_ok());
    }

    #[test]
    fn final_asr_and_local_polish_never_load_together_and_oom_recovers() {
        let scheduler = RuntimeWizardCoordinator::new(PathBuf::from("missing"));
        scheduler
            .begin_role(RuntimeRole::FinalAsr, Some(16 * 1_024 * 1_024 * 1_024))
            .unwrap();
        assert_eq!(
            scheduler
                .begin_role(RuntimeRole::LocalPolish, Some(16 * 1_024 * 1_024 * 1_024))
                .expect_err("concurrent model must be refused"),
            "runtime_busy"
        );
        scheduler.finish();
        assert_eq!(
            scheduler
                .begin_role(RuntimeRole::LocalPolish, Some(4 * 1_024 * 1_024 * 1_024))
                .expect_err("low memory must fail before load"),
            "runtime_memory_budget_unavailable"
        );
        assert!(scheduler.begin(Some(MINIMUM_TOTAL_MEMORY_BYTES)).is_ok());
    }

    #[test]
    fn missing_runtime_resources_fail_closed() {
        let scheduler = RuntimeWizardCoordinator::new(PathBuf::from("missing-runtime-root"));
        assert_eq!(scheduler.paths(), Err("runtime_resources_unavailable"));
    }

    /// The regression this module shipped with, and the reason it survived:
    /// every existing `paths()` test passed a root that does not exist, so
    /// they all asserted the error and none of them asserted the success. The
    /// struct went on requiring the streaming engine's three binaries after
    /// the fork deleted them, and `paths()` therefore failed for a *staged*
    /// root too — every dictation would have ended in
    /// `runtime_resources_unavailable`, which no test and no launch had ever
    /// exercised.
    ///
    /// So this asserts the positive case against a root laid out the way
    /// `Stage-DevRuntime.ps1` and the installer's payload both lay it out, and
    /// pins that the Granite worker is what makes the difference: present, it
    /// resolves; absent, it fails closed. A file that is merely *named* is not
    /// enough — `canonical_file` requires it to exist on disk.
    #[test]
    fn a_staged_root_resolves_and_the_granite_worker_is_what_makes_it_one() {
        let root = tempfile::tempdir().expect("resource root");
        let proof = root.path().join("proof");
        std::fs::create_dir_all(&proof).expect("proof directory");

        // A `proof/` with no worker in it is the shape an interrupted install
        // leaves behind, and it must not resolve.
        let scheduler = RuntimeWizardCoordinator::new(root.path().to_path_buf());
        assert_eq!(
            scheduler.paths(),
            Err("runtime_resources_unavailable"),
            "a proof directory without the worker must fail closed"
        );

        std::fs::write(proof.join("granite-worker.exe"), b"not a real binary")
            .expect("staged worker");

        let paths = scheduler.paths().expect("a staged root must resolve");
        assert!(
            paths.granite_worker.is_file(),
            "granite_worker must name a file that exists"
        );
        assert_eq!(
            paths
                .granite_worker
                .file_name()
                .and_then(|name| name.to_str()),
            Some("granite-worker.exe")
        );
        assert!(
            paths.granite_worker.starts_with(&paths.root),
            "the worker must resolve under the resource root"
        );
        assert_eq!(paths.granite_worker.parent(), Some(paths.proof.as_path()));
    }

    #[test]
    fn repeated_failures_quarantine_until_explicit_idle_recovery() {
        let scheduler = RuntimeWizardCoordinator::new(PathBuf::from("missing"));
        scheduler.record_worker_failure();
        scheduler.record_worker_failure();
        scheduler.record_worker_failure();
        assert_eq!(
            scheduler
                .begin(Some(MINIMUM_TOTAL_MEMORY_BYTES))
                .expect_err("quarantine must refuse a lease"),
            "runtime_worker_quarantined"
        );
        scheduler.recover_manually().expect("manual recovery");
        assert!(scheduler.begin(Some(MINIMUM_TOTAL_MEMORY_BYTES)).is_ok());
        assert_eq!(scheduler.recover_manually(), Err("runtime_busy"));
        scheduler.finish();
    }
}
