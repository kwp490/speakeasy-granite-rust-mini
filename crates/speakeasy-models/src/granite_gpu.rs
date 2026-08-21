//! Whether this installation can run Granite on the graphics card, and whether
//! it is actually doing so.
//!
//! # Why this module exists
//!
//! Because the question was answered in three places that could disagree, and
//! on 2026-08-20 they did: a support log read
//! `engine=cpu_gpu_runtime_missing device=cpu installed=cuda`. Setup had
//! recorded a graphics-card installation, the runtime had correctly found no
//! graphics-card path, and the only thing that noticed was one field of one log
//! line. Every layer behaved as written; the *claim* was manufactured out of a
//! radio button nobody had disabled.
//!
//! Three facts are involved and they are genuinely independent. Conflating any
//! two of them is how the wrong answer gets built:
//!
//! 1. **Published.** Is a CUDA-capable Granite worker pinned in the trusted
//!    manifest at all? This is a fact about the *release*, not the machine.
//! 2. **Present.** Is that worker, and every runtime library it needs, on this
//!    disk? Windows resolves those libraries from the worker's own directory,
//!    and a worker missing one does not fall back to the processor — it fails.
//!    *When* it fails is the part worth knowing, and it is not always at
//!    startup: `cuBLAS` is an import of the image and a missing `cublas64_13`
//!    stops the process before `main`, but `cuBLASLt` is loaded by `cuBLAS` at
//!    the first matmul, so a payload missing only that one starts, loads two
//!    gigabytes of weights, and fails ~36 s later mid-dictation with
//!    `AdapterFailed`. Measured 2026-08-21. That late failure is the reason
//!    this gate is a *precondition* rather than something inferred from a
//!    worker that started.
//! 3. **Operational.** Is a live worker process actually holding a CUDA context
//!    on a device? Nothing static can answer this. The worker's compiled
//!    accelerators say what it *could* do, and a machine whose driver refuses,
//!    whose card is claimed by another process, or whose VRAM is exhausted will
//!    run the same binary on the CPU without complaint.
//!
//! The model pack answers **none** of the three, which is what the old code got
//! wrong: it asked the manifest for a CUDA `final-asr` pack. There is one GGUF
//! and the CUDA worker offloads that same file, so a CUDA pack entry would be a
//! duplicate of the CPU one and its presence would say nothing about whether a
//! GPU path exists.
//!
//! # What the caller gets
//!
//! [`inspect_gpu_payload`] answers 1 and 2 together, naming what is missing.
//! [`prove_cuda_context`] answers 3 against a live process id. Setup requires
//! all three before it will *record* a graphics-card installation, and the app
//! re-checks 3 at every warm so a claim that stops being true is reported
//! rather than remembered.

use std::path::Path;

use crate::gpu::GpuProbeFailure;
use crate::manifest::TrustedManifest;

/// The manifest id a published CUDA-capable Granite worker will carry.
///
/// Named here, as a constant, rather than left implicit — because the absence of
/// this artifact is the declaration that no graphics-card configuration exists,
/// and a declaration made by absence has to say which absence it is. On the day
/// a CUDA worker is built and pinned, it goes into the manifest's `artifacts`
/// under this id and every layer below starts answering `true` without a second
/// edit.
///
/// `native-runtime` rather than a pack: it is a binary this project builds and
/// pins by digest, which is exactly what that artifact kind is for, and it is
/// emphatically not a model.
pub const GRANITE_CUDA_WORKER_ARTIFACT_ID: &str = "granite-worker-cuda-windows-x64";

/// The manifest ids of the CUDA redistributables a CUDA Granite worker needs.
///
/// They are the runtime half of "present". Both are already pinned by digest in
/// the trusted manifest — `cuFFT` and `cuDNN` were there too and left with ONNX
/// Runtime.
///
/// **This list is deliberately a superset of what the worker loads, and one of
/// the three files is never loaded at all.** Measured 2026-08-21 against the
/// worker this workspace builds: the image names `cublas64_13.dll` and
/// `nvcuda.dll`; it does not contain the string `cudart64_13.dll`, because ggml
/// links the CUDA runtime statically on Windows. With `cudart64_13.dll` deleted
/// from beside the worker and the CUDA Toolkit stripped from `PATH`, the worker
/// transcribed the fixture and NVML confirmed it holding a context on the
/// device. `cublasLt64_13.dll` is not named in the image either, but it *is*
/// required — see this module's header for the shape of that failure.
///
/// It stays pinned for two reasons. `CMAKE_CUDA_RUNTIME_LIBRARY` is one build
/// flag away from making it load-bearing again, and nothing anywhere would
/// notice the day it changed; and every file this catalog requires is a file
/// this catalog pins by digest, which is the property that lets presence imply
/// provenance. Accepting `cudart64_*.dll` by pattern was considered and
/// rejected for the same reason. The cost of the superset is 551 KB and a
/// refusal that cannot arise from a published payload, since the worker and its
/// libraries are pinned and shipped as one artifact.
///
/// Ids rather than file names, so the file names come from the manifest's own
/// `proof_files` and exist in exactly one place. A second hand-written list of
/// DLL names is how `cudart64_12` and `cudart64_13` came to both be referenced
/// in this workspace for the same requirement.
///
/// CUDA 13 since 2026-08-21. The 12.9 pin was inert for as long as nothing read
/// it, and became a refusal the day this function did: a worker built against
/// the only toolkit a developer here has loads `cudart64_13.dll`, so the payload
/// was rejected as `RuntimeFilesMissing` naming three libraries that were
/// present under their real names. `scripts/Get-CudaRuntime.ps1` produces the
/// entries these ids point at.
const CUDA_RUNTIME_ARTIFACT_IDS: &[&str] = &[
    "nvidia-cuda-cudart-windows-x64-13.3.29",
    "nvidia-libcublas-windows-x64-13.6.0.2",
];

/// Why this installation cannot run Granite on the graphics card.
///
/// Each variant is a different thing for a user or an operator to do, which is
/// the whole reason they are not one `false`:
/// [`Self::WorkerNotPublished`] is a fact about the release and nothing on this
/// machine will change it, [`Self::WorkerNotInstalled`] means re-run setup, and
/// [`Self::RuntimeFilesMissing`] names files that can be put back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuPayloadRejection {
    /// No CUDA-capable Granite worker is pinned in the trusted manifest, so
    /// there is nothing for any installer to install. Today this is every
    /// machine's answer.
    WorkerNotPublished,
    /// The manifest publishes one and this installation does not carry it.
    WorkerNotInstalled,
    /// The worker is here and at least one pinned runtime library is not.
    ///
    /// Carries the file names because they are the instruction. A CUDA build
    /// with `cublas64_13.dll` missing does not run slower — it does not start,
    /// and Windows' error for that names no file the user can act on.
    RuntimeFilesMissing(Vec<String>),
}

impl GpuPayloadRejection {
    /// A stable code for the diagnostic log and the UI catalog. Never a file
    /// path: the log is a privacy surface, and the file names travel separately
    /// where they are being shown to whoever can fix them.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::WorkerNotPublished => "gpu_worker_not_published",
            Self::WorkerNotInstalled => "gpu_worker_not_installed",
            Self::RuntimeFilesMissing(_) => "gpu_runtime_files_missing",
        }
    }
}

/// The file name of every runtime library a CUDA Granite worker needs beside it.
///
/// Read out of the manifest's pinned `proof_files`, and reduced to base names:
/// NVIDIA buries each library in a directory of its own — `bin/` in CUDA 12.9,
/// `bin/x64/` in 13.x — and what has to sit next to the worker is the DLL
/// itself. Reducing rather than stripping a known prefix is why the 13.x move
/// cost this function nothing. Sorted and de-duplicated so two artifacts naming
/// the same library produce one requirement.
///
/// Empty when the manifest carries neither redistributable, which is itself a
/// meaningful answer — it means this catalog has nothing pinned to check
/// against, and [`inspect_gpu_payload`] refuses on the worker before it gets
/// here.
#[must_use]
pub fn required_cuda_runtime_files(manifest: &TrustedManifest) -> Vec<String> {
    let mut files: Vec<String> = manifest
        .native_runtimes()
        .filter(|runtime| CUDA_RUNTIME_ARTIFACT_IDS.contains(&runtime.id))
        .flat_map(|runtime| {
            runtime.proof_files.iter().filter_map(|file| {
                Path::new(file.path())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Whether the graphics-card configuration is published **and** present.
///
/// `worker_directory` is where the Granite worker executable lives — `proof/`
/// under the install root. That is forced rather than chosen: Windows resolves a
/// dynamically loaded DLL's dependencies against the loading process's own
/// directory first, so the CUDA libraries have to be beside the worker and
/// nowhere else.
///
/// Deliberately **not** an answer to "will it run". A complete payload on a
/// machine whose driver refuses still runs on the CPU, silently, and
/// [`prove_cuda_context`] is the only thing that catches that. Setup requires
/// both.
///
/// # Errors
///
/// Returns the first rejection that applies, in the order published → worker
/// present → runtime present. The order matters: a machine missing all three
/// should be told the release has no GPU worker, not handed a list of DLLs to
/// find for a binary that does not exist.
pub fn inspect_gpu_payload(
    manifest: &TrustedManifest,
    worker_directory: &Path,
    worker_file_name: &str,
) -> Result<(), GpuPayloadRejection> {
    if manifest
        .native_runtimes()
        .all(|runtime| runtime.id != GRANITE_CUDA_WORKER_ARTIFACT_ID)
    {
        return Err(GpuPayloadRejection::WorkerNotPublished);
    }
    if !worker_directory.join(worker_file_name).is_file() {
        return Err(GpuPayloadRejection::WorkerNotInstalled);
    }
    let missing: Vec<String> = required_cuda_runtime_files(manifest)
        .into_iter()
        .filter(|name| !worker_directory.join(name).is_file())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(GpuPayloadRejection::RuntimeFilesMissing(missing))
    }
}

/// The process ids currently holding a CUDA compute context, per NVML.
///
/// A trait so the answer can be staged. The condition this exists to catch —
/// a complete CUDA payload that runs on the CPU anyway — cannot be produced on
/// demand by any machine this is developed on, and a test that can only assert
/// what the developer's own card happens to do is not a test of the logic.
///
/// Used through `&dyn` rather than a generic parameter, and that is what lets it
/// be *threaded* rather than merely injected one call deep: the app's warm path
/// carries it as a field of `GraniteEnvironment`, beside the recorded provider it
/// gets compared against. A generic there would infect the environment struct,
/// both entry points that take one, and every test that builds one, so the probe
/// would have stayed hardcoded at the bottom — which is where it was, and which
/// is why the app's own `cuda_unverified` had never been produced.
///
/// `Send + Sync` because the environment holding it crosses an `await` inside a
/// Tauri command, and Tauri requires that future to be `Send`. Stated as a
/// supertrait rather than written into every reference to the trait object: an
/// implementation that could not be shared across threads has no use here, since
/// the only question it answers is a driver query about a process id.
pub trait CudaContextProbe: Send + Sync {
    /// Every pid NVML reports as running compute work on any device.
    ///
    /// # Errors
    ///
    /// Returns why NVML could not be asked. That is deliberately not the same
    /// as "no process holds a context": a driver that will not answer must not
    /// be reported as proof of absence.
    fn compute_process_ids(&self) -> Result<Vec<u32>, GpuProbeFailure>;
}

/// Whether a specific process is running on the graphics card.
///
/// Three answers, not two, and the third is the point: NVML being unavailable
/// is not evidence that a process is on the CPU. Recording a graphics-card
/// installation on [`Self::ProbeUnavailable`] would be the manufactured claim
/// this module exists to prevent; recording a *fault* on it would blame a
/// working install for a driver query. It is neither, and the caller says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CudaContextProof {
    /// NVML lists this process as holding a compute context.
    Holding,
    /// NVML answered, and this process is not among the processes on any
    /// device. The definitive negative.
    NotHolding,
    /// NVML could not be asked, so nothing is proven either way.
    ProbeUnavailable(GpuProbeFailure),
}

impl CudaContextProof {
    /// A stable code for the log.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Holding => "cuda_context_held",
            Self::NotHolding => "cuda_context_absent",
            Self::ProbeUnavailable(_) => "cuda_context_unprovable",
        }
    }

    /// Whether this is proof, rather than merely not a refutation.
    pub const fn is_proven(self) -> bool {
        matches!(self, Self::Holding)
    }
}

/// Ask NVML whether `process_id` is on the graphics card.
///
/// The pid, not the binary's name or path: a name matches a second copy of the
/// same worker started by something else, and the question is about *this*
/// process. This is the "proven operational" half — a CUDA-built worker that
/// could not initialize CUDA looks exactly like a CPU one from the outside, and
/// llama.cpp reports the fallback in its own stderr rather than as an error.
pub fn prove_cuda_context(probe: &dyn CudaContextProbe, process_id: u32) -> CudaContextProof {
    match probe.compute_process_ids() {
        Ok(pids) if pids.contains(&process_id) => CudaContextProof::Holding,
        Ok(_) => CudaContextProof::NotHolding,
        Err(failure) => CudaContextProof::ProbeUnavailable(failure),
    }
}

/// The real NVML-backed probe.
pub struct NvmlCudaContextProbe;

#[cfg(windows)]
impl CudaContextProbe for NvmlCudaContextProbe {
    fn compute_process_ids(&self) -> Result<Vec<u32>, GpuProbeFailure> {
        use nvml_wrapper::Nvml;
        use nvml_wrapper::error::NvmlError;

        let nvml = match Nvml::init() {
            Ok(nvml) => nvml,
            Err(NvmlError::LibloadingError(_) | NvmlError::NotFound) => {
                return Err(GpuProbeFailure::LibraryMissing);
            }
            Err(_) => return Err(GpuProbeFailure::InitializationFailed),
        };
        let count = nvml
            .device_count()
            .map_err(|_| GpuProbeFailure::QueryFailed)?;
        let mut pids = Vec::new();
        // Every device, because a multi-card box can put the worker on any of
        // them, and a query that failed on one card must not hide a context held
        // on another. A device that will not answer is skipped for the same
        // reason `nvml_snapshot` skips it.
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            if let Ok(processes) = device.running_compute_processes() {
                pids.extend(processes.into_iter().map(|process| process.pid));
            }
        }
        Ok(pids)
    }
}

#[cfg(not(windows))]
impl CudaContextProbe for NvmlCudaContextProbe {
    fn compute_process_ids(&self) -> Result<Vec<u32>, GpuProbeFailure> {
        Err(GpuProbeFailure::LibraryMissing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled_manifest;

    struct StagedProbe(Result<Vec<u32>, GpuProbeFailure>);

    impl CudaContextProbe for StagedProbe {
        fn compute_process_ids(&self) -> Result<Vec<u32>, GpuProbeFailure> {
            self.0.clone()
        }
    }

    #[test]
    fn the_shipped_catalog_publishes_no_graphics_card_worker() {
        // The declaration itself, asserted rather than described. This is what
        // makes "no graphics-card configuration exists" a checkable fact instead
        // of a comment, and it is the assertion that flips on the day a CUDA
        // worker is pinned -- at which point the failing test is the reminder
        // that the wizard, the packager and the marker all now have work to do.
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let root = std::env::temp_dir().join("speakeasy-gpu-payload-unpublished");
        assert_eq!(
            inspect_gpu_payload(&manifest, &root, "granite-worker.exe"),
            Err(GpuPayloadRejection::WorkerNotPublished),
            "no CUDA Granite worker is published, so no installation may claim one"
        );
    }

    #[test]
    fn the_cuda_runtime_requirement_comes_from_the_manifests_own_digests() {
        // Base names, from `proof_files`, so this cannot drift from what the
        // downloader verifies. The workspace previously named `cudart64_13.dll`
        // in one place and pinned `cudart64_12.dll` in another.
        //
        // Named by hand here on purpose, and it is the only place they are. This
        // is what makes the catalog's CUDA major an explicit decision rather
        // than whatever the last person to touch the manifest happened to pin:
        // reading the names out of the manifest to assert against the manifest
        // would pass on an empty list.
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let required = required_cuda_runtime_files(&manifest);
        // cudart is required although this build never loads it -- statically
        // linked by ggml, and proved unnecessary on 2026-08-21 by deleting it
        // and watching the worker transcribe on the card anyway. The superset is
        // deliberate; `CUDA_RUNTIME_ARTIFACT_IDS` carries the argument.
        assert!(
            required.contains(&"cudart64_13.dll".to_owned()),
            "cudart is pinned in the catalog and must be required: {required:?}"
        );
        assert!(
            required.contains(&"cublas64_13.dll".to_owned()),
            "cuBLAS is pinned in the catalog and must be required: {required:?}"
        );
        // Unlike cudart, this one is real, and it fails late: measured
        // 2026-08-21, a worker without it starts, loads the weights, and dies
        // ~36 s in at the first matmul. Requiring it up front is what keeps that
        // out of a dictation.
        assert!(
            required.contains(&"cublasLt64_13.dll".to_owned()),
            "cuBLASLt is loaded by cuBLAS at run time and must be required too: {required:?}"
        );
        assert!(
            required.iter().all(|name| !name.contains('/')),
            "requirements must be file names beside the worker, not archive paths: {required:?}"
        );
    }

    #[test]
    fn a_present_worker_with_no_runtime_libraries_names_every_missing_file() {
        // The regression this whole module is for, one layer down: a CUDA worker
        // that cannot start because Windows cannot resolve its imports. The
        // failure Windows gives names no file, so the rejection has to.
        let manifest = staged_manifest_publishing_the_cuda_worker();
        let root = std::env::temp_dir().join("speakeasy-gpu-payload-no-dlls");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("staging directory");
        std::fs::write(root.join("granite-worker.exe"), b"worker").expect("worker");

        let rejection = inspect_gpu_payload(&manifest, &root, "granite-worker.exe")
            .expect_err("a worker with no CUDA libraries beside it must be refused");

        let GpuPayloadRejection::RuntimeFilesMissing(missing) = rejection else {
            panic!("expected the runtime-files rejection, got {rejection:?}");
        };
        assert_eq!(missing, required_cuda_runtime_files(&manifest));
    }

    #[test]
    fn a_published_worker_that_was_not_installed_is_its_own_rejection() {
        let manifest = staged_manifest_publishing_the_cuda_worker();
        let root = std::env::temp_dir().join("speakeasy-gpu-payload-absent-worker");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("staging directory");

        assert_eq!(
            inspect_gpu_payload(&manifest, &root, "granite-worker.exe"),
            Err(GpuPayloadRejection::WorkerNotInstalled)
        );
    }

    #[test]
    fn a_complete_payload_is_accepted_and_is_still_not_proof_it_runs() {
        let manifest = staged_manifest_publishing_the_cuda_worker();
        let root = std::env::temp_dir().join("speakeasy-gpu-payload-complete");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("staging directory");
        std::fs::write(root.join("granite-worker.exe"), b"worker").expect("worker");
        for name in required_cuda_runtime_files(&manifest) {
            std::fs::write(root.join(name), b"dll").expect("runtime library");
        }

        assert_eq!(
            inspect_gpu_payload(&manifest, &root, "granite-worker.exe"),
            Ok(())
        );
        // And the second half is still unanswered, which is the point of the
        // two being separate calls.
        assert_eq!(
            prove_cuda_context(&StagedProbe(Ok(vec![4321])), 1234),
            CudaContextProof::NotHolding
        );
    }

    #[test]
    fn a_driver_that_will_not_answer_proves_nothing_in_either_direction() {
        // Neither `Holding` nor `NotHolding`. An unavailable probe read as a
        // negative would report a fault on a working GPU install; read as a
        // positive it would manufacture the claim this module exists to stop.
        let proof = prove_cuda_context(&StagedProbe(Err(GpuProbeFailure::LibraryMissing)), 1234);
        assert_eq!(
            proof,
            CudaContextProof::ProbeUnavailable(GpuProbeFailure::LibraryMissing)
        );
        assert!(!proof.is_proven());
        assert_eq!(proof.code(), "cuda_context_unprovable");
    }

    #[test]
    fn a_pid_holding_a_context_is_proof_and_only_that_pid_is() {
        let probe = StagedProbe(Ok(vec![10, 20, 30]));
        assert_eq!(prove_cuda_context(&probe, 20), CudaContextProof::Holding);
        // Not the binary's name and not "some worker is on the GPU": a second
        // copy of the same executable started by something else would satisfy a
        // name match and say nothing about this process.
        assert_eq!(prove_cuda_context(&probe, 21), CudaContextProof::NotHolding);
    }

    #[test]
    fn the_packager_and_the_models_crate_require_the_same_cuda_libraries() {
        // Two readers of one fact, pinned against each other rather than hoped
        // about. `scripts/GraniteWorkerProvider.ps1` refuses to package a CUDA
        // worker without these libraries beside it; this module refuses to
        // report the payload complete without them. Naming different artifacts
        // would let a payload pass packaging and be rejected at install, or
        // worse, the reverse.
        let script = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/GraniteWorkerProvider.ps1"
        ))
        .expect("the packaging helper must be readable");
        for id in CUDA_RUNTIME_ARTIFACT_IDS {
            assert!(
                script.contains(id),
                "the packager must require {id} too, or the two lists have drifted"
            );
        }
        // And the worker's own artifact id, which is what makes "published" one
        // thing rather than two.
        assert!(
            std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../models/trusted-manifest.json"
            ))
            .expect("the catalog must be readable")
            .contains(GRANITE_CUDA_WORKER_ARTIFACT_ID),
            "the catalog must name the artifact whose absence is the declaration"
        );
    }

    /// The bundled manifest with a CUDA Granite worker artifact spliced in.
    ///
    /// Built by editing the shipped JSON rather than by hand, so the staged
    /// manifest cannot drift from the real schema — a hand-written fixture that
    /// stops parsing takes its tests green with it, since they would fail for
    /// the wrong reason.
    fn staged_manifest_publishing_the_cuda_worker() -> TrustedManifest {
        let source = include_str!("../../../models/trusted-manifest.json");
        let mut document: serde_json::Value =
            serde_json::from_str(source).expect("the shipped manifest must parse as JSON");
        let cudart = document["artifacts"][0].clone();
        let mut worker = cudart;
        worker["id"] = serde_json::Value::String(GRANITE_CUDA_WORKER_ARTIFACT_ID.to_owned());
        document["artifacts"]
            .as_array_mut()
            .expect("artifacts is an array")
            .push(worker);
        TrustedManifest::parse(
            serde_json::to_string(&document)
                .expect("re-serializing the manifest")
                .as_bytes(),
        )
        .expect("the staged manifest must still be valid")
    }
}
