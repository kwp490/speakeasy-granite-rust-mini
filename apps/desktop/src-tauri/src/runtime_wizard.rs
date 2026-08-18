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
    pub worker: PathBuf,
    pub onnx_runtime: PathBuf,
    /// sherpa-onnx's own shared library. Its presence stands in for the whole
    /// sherpa runtime the worker links against, and is required on every
    /// install — this is what makes the CPU pack work.
    pub sherpa_c_api: PathBuf,
    /// The CUDA execution provider, when this install staged it. Optional
    /// rather than required: the CUDA and cuDNN redistributables are large
    /// enough that bundling them unconditionally is its own distribution
    /// problem (see the GPU migration handoff's "Runtime pack" decision), so
    /// an install without them is CPU-only rather than broken.
    pub onnxruntime_providers_cuda: Option<PathBuf>,
    /// The Granite worker binary, when this install staged it. Optional and
    /// never in `required`: Granite is a second, independent final-pass
    /// engine, not a dependency of ordinary dictation, so its absence must
    /// never break a install that never asked for it (see
    /// `docs/handoff/granite-final-pass.md`, Phase 6).
    pub granite_worker: Option<PathBuf>,
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
            worker: canonical_file(&root, "proof/inference-worker.exe")?,
            onnx_runtime: canonical_file(&root, "proof/onnxruntime.dll")?,
            sherpa_c_api: canonical_file(&root, "proof/sherpa-onnx-c-api.dll")?,
            onnxruntime_providers_cuda: canonical_file(
                &root,
                "proof/onnxruntime_providers_cuda.dll",
            )
            .ok(),
            granite_worker: canonical_file(&root, "proof/granite-worker.exe").ok(),
            root: root.clone(),
        };
        let mut required = vec![&paths.worker, &paths.onnx_runtime, &paths.sherpa_c_api];
        if let Some(cuda) = &paths.onnxruntime_providers_cuda {
            required.push(cuda);
        }
        if let Some(granite_worker) = &paths.granite_worker {
            required.push(granite_worker);
        }
        if required.iter().any(|path| !path.starts_with(&root)) {
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
