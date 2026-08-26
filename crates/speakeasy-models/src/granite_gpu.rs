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
use crate::manifest::{NativeRuntimeSource, TrustedManifest};

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
/// rejected for the same reason. The cost of the superset is 551 KB, and a
/// refusal that names it can only arise from a payload that arrived incomplete
/// — the worker and these libraries are separate artifacts from separate hosts
/// (owner decision 2026-08-26: this project publishes the worker, NVIDIA's own
/// CDN serves the redistributables), so "fetched all three or none" is a
/// property of [`graphics_card_payload_sources`] and of `inspect_gpu_payload`
/// refusing until every file is there, not of the transport.
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

/// Whether a graphics-card configuration is something an installer could
/// install, asked of the release rather than of the machine.
///
/// # Why this is separate from [`inspect_gpu_payload`]
///
/// Those are two different questions and conflating them shipped a bug in
/// waiting. The wizard's provider page asks this one, **before** the payload has
/// been extracted: on a first install `proof/granite-worker.exe` does not exist
/// yet, so an "is it present on disk" check answers `WorkerNotInstalled` for
/// every fresh machine and the graphics-card option stays disabled no matter what
/// the manifest says. Item 3's plan assumed pinning the artifact would be enough;
/// it is not, and this is the second edit.
///
/// The reasoning that put a presence check on the wizard's path was that
/// "published alone would re-offer the option on a machine where the runtime
/// libraries never arrived". That case is real and is answered later and better:
/// setup runs its engine check *after* staging the payload and records the
/// provider from that verdict, and the app re-proves the context at every warm.
/// Neither needs the wizard to pre-empt it, and the wizard cannot do it correctly
/// anyway.
///
/// # What "installable" needs
///
/// Both halves of the payload pinned: the worker, and at least one CUDA
/// redistributable for the libraries it loads. A manifest naming a worker with no
/// libraries pinned beside it is not a publishable configuration — it is a
/// half-written catalog, and setup would stage a worker Windows cannot resolve the
/// imports for. `the_catalog_never_pins_a_worker_without_its_runtime` refuses that
/// state, so the second condition here is a floor rather than a live path.
///
/// Deliberately touches no disk. This is a fact about the release, and asking it
/// of a directory is what made it wrong.
///
/// # Errors
///
/// [`GpuPayloadRejection::WorkerNotPublished`] when this release carries no
/// complete graphics-card configuration.
pub fn gpu_configuration_is_installable(
    manifest: &TrustedManifest,
) -> Result<(), GpuPayloadRejection> {
    if manifest
        .native_runtimes()
        .all(|runtime| runtime.id != GRANITE_CUDA_WORKER_ARTIFACT_ID)
    {
        return Err(GpuPayloadRejection::WorkerNotPublished);
    }
    if required_cuda_runtime_files(manifest).is_empty() {
        return Err(GpuPayloadRejection::WorkerNotPublished);
    }
    Ok(())
}

/// Every artifact an installer has to fetch to put Granite on the graphics
/// card, in the order it should be fetched.
///
/// The worker first, then the redistributables it loads. That order is for the
/// progress bar rather than for correctness — nothing is usable until all of
/// them are installed — but a user watching "1 of 3" wants the item named
/// "graphics-card engine" to be the one that is clearly the point.
///
/// Empty when the configuration is not installable, so a caller that forgets to
/// ask [`gpu_configuration_is_installable`] first plans nothing rather than
/// planning half a payload. That is the only failure mode worth guarding here:
/// a worker with no libraries beside it does not start, and Windows names no
/// file when it does not.
///
/// The ids live in this module and nowhere else, which is the same rule
/// [`required_cuda_runtime_files`] follows and for the same reason — a second
/// hand-written list is how `cudart64_12` and `cudart64_13` came to name one
/// requirement.
#[must_use]
pub fn graphics_card_payload_sources(manifest: &TrustedManifest) -> Vec<NativeRuntimeSource<'_>> {
    if gpu_configuration_is_installable(manifest).is_err() {
        return Vec::new();
    }
    let worker = manifest
        .native_runtimes()
        .filter(|runtime| runtime.id == GRANITE_CUDA_WORKER_ARTIFACT_ID);
    let runtime = manifest
        .native_runtimes()
        .filter(|runtime| CUDA_RUNTIME_ARTIFACT_IDS.contains(&runtime.id));
    worker.chain(runtime).collect()
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
    // The published half, through the one function that answers it, so the two
    // checks cannot disagree about what a published configuration is.
    gpu_configuration_is_installable(manifest)?;
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

    /// The shipped catalog publishes a graphics-card worker, and this is where
    /// that became true.
    ///
    /// **This test used to assert the opposite**, and its own comment promised it
    /// would flip "on the day a CUDA worker is pinned -- at which point the
    /// failing test is the reminder that the wizard, the packager and the marker
    /// all now have work to do". That day was 2026-08-26 and it did exactly
    /// that, alongside six others.
    ///
    /// What it pins now is the half that stayed the same: publishing is not
    /// installing. A machine with no worker on disk is refused
    /// [`GpuPayloadRejection::WorkerNotInstalled`] and not `Ok(())`, so nothing
    /// downstream can read "the release has one" as "this computer has one".
    #[test]
    fn the_shipped_catalog_publishes_a_graphics_card_worker_that_is_not_installed_everywhere() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        assert_eq!(
            gpu_configuration_is_installable(&manifest),
            Ok(()),
            "the catalog pins a worker and its redistributables"
        );

        let root = std::env::temp_dir().join("speakeasy-gpu-payload-not-installed");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            inspect_gpu_payload(&manifest, &root, "granite-worker.exe"),
            Err(GpuPayloadRejection::WorkerNotInstalled),
            "published is not installed, and an empty machine must be told which"
        );
    }

    /// Installable is a question about the release, and must not touch the disk.
    ///
    /// This is the assertion the bug needed and did not have. The wizard asked
    /// `inspect_gpu_payload`, which also requires the worker to be *present* --
    /// and on a first install it is not, because the payload has not been
    /// extracted when the provider page is shown. So the option would have stayed
    /// disabled on every fresh machine however the manifest was pinned, while
    /// answering `Ok(())` on any machine with a worker staged by hand: which is
    /// the machine this is developed on, and the reason nothing here could have
    /// caught it.
    ///
    /// So the directory below is deliberately one that does not exist. A future
    /// change that reintroduces a presence check fails here rather than on a
    /// user's first install.
    #[test]
    fn installable_asks_the_release_and_never_the_disk() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let absent = std::env::temp_dir().join("speakeasy-gpu-installable-no-such-directory");
        assert!(!absent.exists(), "the fixture directory must not exist");

        // Installable, against a directory that does not exist. Before the pin
        // both answers were `WorkerNotPublished` and this test could only show
        // that they agreed; now they disagree, which is the split it was written
        // for actually visible. This is the exact pair of answers a first
        // install produces on the provider page.
        assert_eq!(
            gpu_configuration_is_installable(&manifest),
            Ok(()),
            "the release publishes a graphics-card configuration"
        );
        assert_eq!(
            inspect_gpu_payload(&manifest, &absent, "granite-worker.exe"),
            Err(GpuPayloadRejection::WorkerNotInstalled),
            "and the disk does not have it yet, which is a different answer"
        );
    }

    /// A worker pinned without its libraries is a half-written catalog.
    ///
    /// Setup would stage a binary whose imports Windows cannot resolve, and that
    /// failure names no file: the process does not start and there is nothing to
    /// act on. The libraries are fetched from NVIDIA's own CDN rather than shipped
    /// beside the worker (owner decision 2026-08-26), which makes this two
    /// independent manifest entries that have to move together -- exactly the
    /// shape that let `cudart64_12` and `cudart64_13` disagree for months.
    #[test]
    fn the_catalog_never_pins_a_worker_without_its_runtime() {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        let worker_pinned = manifest
            .native_runtimes()
            .any(|runtime| runtime.id == GRANITE_CUDA_WORKER_ARTIFACT_ID);
        if worker_pinned {
            assert!(
                !required_cuda_runtime_files(&manifest).is_empty(),
                "a published CUDA worker needs its redistributables pinned beside it"
            );
        }
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

    /// Every artifact of the graphics-card payload is fetchable, engine first.
    ///
    /// The "nothing until published" half of this test went away with the pin on
    /// 2026-08-26 — it asserted the shipped catalog had no sources, which is now
    /// false. The property it protected is still worth holding and is checked
    /// where it can be: `gpu_configuration_is_installable` gates this function,
    /// and `a_worker_without_its_libraries_is_not_a_fetchable_configuration`
    /// below drives the empty case through a manifest that has half a payload.
    /// That case matters because two NVIDIA archives with no worker are not a
    /// partial install, they are 400 MB of libraries nothing will ever load.
    #[test]
    fn every_part_of_the_graphics_card_payload_is_fetchable_and_the_engine_leads() {
        let staged = staged_manifest_publishing_the_cuda_worker();
        let sources = graphics_card_payload_sources(&staged);
        let ids: Vec<&str> = sources.iter().map(|source| source.id).collect();
        assert_eq!(
            ids.first().copied(),
            Some(GRANITE_CUDA_WORKER_ARTIFACT_ID),
            "the engine leads, because it is the item the progress bar should name first: {ids:?}"
        );
        let mut rest = ids[1..].to_vec();
        rest.sort_unstable();
        let mut expected = CUDA_RUNTIME_ARTIFACT_IDS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            rest, expected,
            "every pinned redistributable is fetched, not a subset of them"
        );

        // And each one is actually fetchable. A source with no URL or no digest
        // would plan a download that cannot start or cannot be verified, and the
        // schema is what stops that -- so this is checking the schema still does.
        for source in &sources {
            assert!(source.url.starts_with("https://"), "{}", source.id);
            assert_eq!(source.archive_sha256.len(), 64, "{}", source.id);
            assert!(source.archive_bytes > 0, "{}", source.id);
            assert!(!source.proof_files.is_empty(), "{}", source.id);
        }
    }

    /// Half a payload is not a fetchable configuration.
    ///
    /// The empty answer this used to get from the shipped catalog for free, now
    /// driven deliberately: a manifest with the worker and no redistributables.
    /// Nothing may be planned from it, because two NVIDIA archives with no
    /// worker — or a worker with no libraries — is not a partial graphics-card
    /// install. It is bytes nothing will ever load, and in the worker's case a
    /// binary Windows cannot resolve the imports for, which fails naming no file.
    ///
    /// Built by *removing* entries from the shipped JSON rather than by writing
    /// a manifest by hand, so the fixture cannot drift from the schema.
    #[test]
    fn a_worker_without_its_libraries_is_not_a_fetchable_configuration() {
        let source = include_str!("../../../models/trusted-manifest.json");
        let mut document: serde_json::Value =
            serde_json::from_str(source).expect("the shipped manifest must parse as JSON");
        let artifacts = document["artifacts"]
            .as_array_mut()
            .expect("artifacts is an array");
        artifacts.retain(|artifact| artifact["id"] == GRANITE_CUDA_WORKER_ARTIFACT_ID);
        assert_eq!(artifacts.len(), 1, "the worker alone must survive the trim");
        let manifest = TrustedManifest::parse(
            serde_json::to_string(&document)
                .expect("re-serializing the manifest")
                .as_bytes(),
        )
        .expect("a manifest may be half-written and still be valid JSON");

        assert!(
            required_cuda_runtime_files(&manifest).is_empty(),
            "the fixture's premise is that no redistributable is pinned"
        );
        assert_eq!(
            gpu_configuration_is_installable(&manifest),
            Err(GpuPayloadRejection::WorkerNotPublished),
            "a worker with nothing to load is not a publishable configuration"
        );
        assert!(
            graphics_card_payload_sources(&manifest).is_empty(),
            "and nothing may be planned from it"
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

    /// A manifest that publishes a CUDA Granite worker.
    ///
    /// **This no longer stages anything.** It synthesised the artifact by
    /// cloning another and renaming it, from the fork until 2026-08-26, because
    /// none was published and the tests below had nothing real to run against.
    /// One is published now, so the shipped catalog *is* the fixture — and
    /// splicing a second entry under the same id would make
    /// [`TrustedManifest::parse`] refuse the whole document, which is how these
    /// three tests announced the change rather than quietly measuring nothing.
    ///
    /// Kept as a named function rather than inlined at each call site: it says
    /// what the callers depend on, and it is where a future re-pin that renames
    /// the artifact gets caught once instead of three times.
    fn staged_manifest_publishing_the_cuda_worker() -> TrustedManifest {
        let manifest = bundled_manifest().expect("the bundled manifest must parse");
        assert!(
            gpu_configuration_is_installable(&manifest).is_ok(),
            "this fixture's whole premise is a published worker"
        );
        manifest
    }
}
