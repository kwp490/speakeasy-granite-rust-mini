//! On-demand distribution of the CUDA execution provider and the NVIDIA
//! redistributables it loads.
//!
//! # Why this is not an `InstallManager` pack
//!
//! Every other artifact this crate installs lands in `<root>/<id>/<revision>`
//! and is activated by an atomic directory rename, so a half-finished install is
//! invisible by construction. The CUDA runtime cannot work that way: Windows
//! resolves a dynamically loaded DLL's dependencies using the **process** search
//! order — the executable's own directory, then `System32`, then `PATH` — so
//! cuBLAS, cuFFT and cuDNN have to sit in the same directory as
//! `inference-worker.exe`. There is no directory to swap, only files to add
//! beside binaries the installer already put there.
//!
//! `PATH` is not a weaker alternative, it is an actively worse one: a machine
//! with the CUDA Toolkit installed may carry a different `cublas64_12.dll` on
//! it, and loading that one would be the same silent-wrong-runtime failure as
//! `System32`'s ONNX Runtime winning over the staged one.
//!
//! So **completeness replaces atomicity**. [`CudaRuntimePlan::is_complete`] is
//! the activation gate, and it is true only when all fifteen required files are
//! present at the lengths the manifest records. An install interrupted anywhere
//! — killed process, full disk, pulled network — leaves an incomplete set, which
//! reads as *not installed*, so the app falls back to CPU and a retry overwrites
//! cleanly. That is the same presence-versus-verification split
//! [`crate::InstallManager::is_present`] draws, for the same reason: the question
//! "may this engine be selected?" is asked on the path to every dictation and
//! cannot afford to hash 2.45 GB.
//!
//! # Three sources across five archives
//!
//! The obvious reading is that a CUDA runtime comes from NVIDIA. Two thirds of
//! it does. `onnxruntime_providers_cuda.dll` is built by the sherpa-onnx project
//! and ships only in its CUDA release archive — neither sherpa CUDA archive
//! carries any CUDA or cuDNN redistributable, and no NVIDIA archive carries the
//! provider. The provider's archive is fetched **last** for that reason: it is
//! the file whose presence most looks like "CUDA is installed", so it is the
//! last thing to appear.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use speakeasy_domain::CancelToken;
use sysinfo::Disks;

use crate::archive::extract_required_files;
use crate::download::download_to_file;
use crate::{
    ArchiveExtractionError, DownloadError, DownloadPolicy, DownloadRequest, InstallFile,
    InstallSpec, TrustedManifest,
};

/// Which installable half of the runtime a file belongs to.
///
/// Both halves are required to run anything: this is a split in *how the bytes
/// are fetched*, not in what the user gets. Core without cuDNN cannot execute a
/// single node, so [`CudaRuntimePlan::is_complete`] demands both. What the split
/// buys is that 2.97 GB arrives as five independently verified, independently
/// resumable archives rather than one, and that a retry after a failure part way
/// through re-fetches only the archives whose files are not yet in place.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CudaRuntimeComponent {
    /// The ONNX Runtime CUDA execution provider plus the CUDA core math
    /// libraries its import table names.
    Core,
    /// cuDNN: the `cudnn64_9` dispatch stub and the nine sub-libraries it loads
    /// by name at run time. Those nine appear in no import table, which is
    /// exactly why they have to be listed rather than discovered.
    Cudnn,
}

impl CudaRuntimeComponent {
    /// A stable code for the UI and the diagnostic log.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Core => "cuda-core",
            Self::Cudnn => "cuda-cudnn",
        }
    }
}

/// One file the CUDA runtime requires, named by the archive it comes from.
struct RequiredRuntimeFile {
    component: CudaRuntimeComponent,
    artifact_id: &'static str,
    /// Path inside the archive, after the archive's own prefix is stripped. It
    /// must appear in that artifact's `proof_files`, because those digests are
    /// the only ones the manifest vouches for.
    archive_path: &'static str,
}

/// Every file the CUDA execution provider needs at run time.
///
/// From `dumpbin /dependents` rather than from guesswork. The provider imports
/// only `cublasLt64_12`, `cublas64_12`, `cufft64_11`, `cudart64_12`,
/// `cudnn64_9` and `onnxruntime_providers_shared` — notably **no** cuRAND,
/// cuSOLVER or cuSPARSE, and `onnxruntime_providers_shared.dll` is absent here
/// because the installer already ships it.
///
/// This list lives in code rather than in the manifest on purpose: it states
/// what *our* binaries need, which is a fact about this workspace, while the
/// manifest states what the archives *contain*, which is a fact about bytes
/// somebody else published. `scripts/Get-GpuRuntime.ps1` draws the same line —
/// it names the DLLs and reads every digest from the manifest.
///
/// Ordered as installed, so the provider DLL's archive is last.
const REQUIRED_RUNTIME_FILES: &[RequiredRuntimeFile] = &[
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Core,
        artifact_id: "nvidia-cuda-cudart-windows-x64-12.9.79",
        archive_path: "bin/cudart64_12.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Core,
        artifact_id: "nvidia-libcublas-windows-x64-12.9.2.10",
        archive_path: "bin/cublas64_12.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Core,
        artifact_id: "nvidia-libcublas-windows-x64-12.9.2.10",
        archive_path: "bin/cublasLt64_12.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Core,
        artifact_id: "nvidia-libcufft-windows-x64-11.4.1.4",
        archive_path: "bin/cufft64_11.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_graph64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_ops64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_cnn64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_engines_precompiled64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_engines_runtime_compiled64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_engines_tensor_ir64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_heuristic64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_adv64_9.dll",
    },
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Cudnn,
        artifact_id: "nvidia-cudnn-windows-x64-9.25.0.15-cuda12",
        archive_path: "bin/x64/cudnn_ext64_9.dll",
    },
    // Last, and from sherpa rather than NVIDIA. See the module note.
    RequiredRuntimeFile {
        component: CudaRuntimeComponent::Core,
        artifact_id: "sherpa-onnx-runtime-windows-x64-cuda-12.x-cudnn-9.x-1.13.4",
        archive_path: "lib/onnxruntime_providers_cuda.dll",
    },
];

#[derive(Debug)]
pub enum CudaRuntimeError {
    /// The manifest does not pin something [`REQUIRED_RUNTIME_FILES`] names.
    /// A build-time mistake rather than a runtime condition, and fatal on
    /// purpose: the alternative is downloading a file whose digest nothing
    /// vouches for.
    ManifestIncomplete(String),
    /// Two required files would install under the same name beside the worker.
    /// Only reachable by editing the table above, and caught before any
    /// download because the loser would silently overwrite the winner.
    NameCollision(String),
    InsufficientDisk {
        required: u64,
        available: u64,
    },
    Download(DownloadError),
    Extraction(ArchiveExtractionError),
    /// A file could not be moved into place because the worker still has it
    /// loaded. Windows locks a mapped image, so replacing a DLL already in use
    /// requires shutting the engine down first.
    RuntimeInUse(PathBuf),
    /// Every archive was fetched and extracted and the set is still incomplete.
    /// Means the table and the manifest disagree with what the archives
    /// actually contained, which must not be reported as success.
    StillIncomplete,
    Cancelled,
    Io(io::Error),
}

impl Display for CudaRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "cuda runtime install failed: {self:?}")
    }
}

impl Error for CudaRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Download(error) => Some(error),
            Self::Extraction(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for CudaRuntimeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DownloadError> for CudaRuntimeError {
    fn from(error: DownloadError) -> Self {
        Self::Download(error)
    }
}

impl From<ArchiveExtractionError> for CudaRuntimeError {
    fn from(error: ArchiveExtractionError) -> Self {
        Self::Extraction(error)
    }
}

/// One required file, resolved to the name it installs under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CudaRuntimeFile {
    pub component: CudaRuntimeComponent,
    /// The flat file name this lands under, beside `inference-worker.exe`.
    /// Derived from the archive path's last component: the directory an archive
    /// happens to nest a DLL in (`bin/`, `bin/x64/`, `lib/`) is that
    /// publisher's layout, and the loader only ever looks in one directory.
    pub installed_name: String,
    /// Path inside its archive, prefix already stripped.
    pub archive_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// One archive to fetch, and the files to take out of it.
#[derive(Clone, Debug)]
pub struct CudaRuntimeArchive {
    pub artifact_id: String,
    pub component: CudaRuntimeComponent,
    pub url: String,
    /// Verification for the transfer and the extraction, in the shape the
    /// existing archive reader already takes.
    pub spec: InstallSpec,
    files: Vec<CudaRuntimeFile>,
}

impl CudaRuntimeArchive {
    pub fn files(&self) -> &[CudaRuntimeFile] {
        &self.files
    }

    /// A stable, filesystem-safe name for this archive on disk.
    pub fn download_file_name(&self) -> String {
        format!("{}.archive", self.artifact_id)
    }
}

/// What the CUDA runtime consists of, resolved against the trusted manifest.
#[derive(Clone, Debug)]
pub struct CudaRuntimePlan {
    archives: Vec<CudaRuntimeArchive>,
}

/// What the installer is doing, for a caller that renders progress.
#[derive(Clone, Copy, Debug)]
pub enum CudaRuntimeEvent<'a> {
    /// About to fetch one archive. `bytes_completed` is how much of the whole
    /// plan's download total is already done, so a caller can render one bar
    /// across all five archives by adding the current `.part` length to it.
    Downloading {
        archive: &'a CudaRuntimeArchive,
        part_path: &'a Path,
        bytes_completed: u64,
    },
    /// Extracting and moving one archive's files into place.
    Installing { archive: &'a CudaRuntimeArchive },
    /// This archive's files were already in place, so it was not fetched.
    Skipped { archive: &'a CudaRuntimeArchive },
}

/// Where an install puts things. All three are on the same volume by
/// construction, because moving 2.45 GB across volumes is a copy.
#[derive(Clone, Debug)]
pub struct CudaRuntimePaths {
    /// Where the DLLs must end up: the directory holding
    /// `inference-worker.exe`.
    pub proof: PathBuf,
    /// Where archives are fetched to. Each is deleted as soon as its files are
    /// extracted, so peak usage is one archive rather than all five.
    pub downloads: PathBuf,
    /// Where files are extracted and length-and-digest checked before being
    /// moved in.
    pub stage: PathBuf,
}

impl CudaRuntimePlan {
    /// Resolves every required file against the manifest.
    ///
    /// # Errors
    ///
    /// [`CudaRuntimeError::ManifestIncomplete`] when the manifest does not pin
    /// a required artifact or file, and [`CudaRuntimeError::NameCollision`] when
    /// two required files would install under one name.
    pub fn resolve(manifest: &TrustedManifest) -> Result<Self, CudaRuntimeError> {
        let mut order: Vec<&str> = Vec::new();
        let mut grouped: HashMap<&str, Vec<CudaRuntimeFile>> = HashMap::new();
        let mut installed_names: HashMap<String, &str> = HashMap::new();

        for required in REQUIRED_RUNTIME_FILES {
            let source = manifest
                .proof_artifacts()
                .iter()
                .find(|artifact| artifact.id() == required.artifact_id)
                .and_then(crate::ProofArtifact::native_runtime_source)
                .ok_or_else(|| {
                    CudaRuntimeError::ManifestIncomplete(format!(
                        "no native-runtime artifact {}",
                        required.artifact_id
                    ))
                })?;
            // The digest has to come from the manifest entry, so a path this
            // table names but the manifest does not pin is fatal rather than
            // something to fetch unverified.
            let pinned = source
                .proof_files
                .iter()
                .find(|file| file.path() == required.archive_path)
                .ok_or_else(|| {
                    CudaRuntimeError::ManifestIncomplete(format!(
                        "{} does not pin {}",
                        required.artifact_id, required.archive_path
                    ))
                })?;
            let installed_name = Path::new(required.archive_path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    CudaRuntimeError::ManifestIncomplete(format!(
                        "{} has no file name",
                        required.archive_path
                    ))
                })?
                .to_owned();
            if let Some(previous) = installed_names.insert(installed_name.clone(), source.id) {
                return Err(CudaRuntimeError::NameCollision(format!(
                    "{installed_name} is provided by both {previous} and {}",
                    source.id
                )));
            }
            if !grouped.contains_key(source.id) {
                order.push(source.id);
            }
            grouped.entry(source.id).or_default().push(CudaRuntimeFile {
                component: required.component,
                installed_name,
                archive_path: PathBuf::from(required.archive_path),
                bytes: pinned.bytes(),
                sha256: pinned.sha256().to_owned(),
            });
        }

        let archives = order
            .into_iter()
            .map(|artifact_id| {
                let files = grouped.remove(artifact_id).unwrap_or_default();
                let source = manifest
                    .proof_artifacts()
                    .iter()
                    .find(|artifact| artifact.id() == artifact_id)
                    .and_then(crate::ProofArtifact::native_runtime_source)
                    .ok_or_else(|| {
                        CudaRuntimeError::ManifestIncomplete(format!(
                            "no native-runtime artifact {artifact_id}"
                        ))
                    })?;
                // An archive never spans components -- the grouping is by
                // publisher and the split follows it -- so the first file's
                // component describes the archive.
                let component = files
                    .first()
                    .map_or(CudaRuntimeComponent::Core, |file| file.component);
                Ok(CudaRuntimeArchive {
                    artifact_id: artifact_id.to_owned(),
                    component,
                    url: source.url.to_owned(),
                    spec: InstallSpec {
                        id: artifact_id.to_owned(),
                        revision: source.version.to_owned(),
                        archive_prefix: PathBuf::from(source.archive_prefix),
                        archive_bytes: source.archive_bytes,
                        archive_sha256: source.archive_sha256.to_owned(),
                        installed_bytes: files.iter().map(|file| file.bytes).sum(),
                        required_files: files
                            .iter()
                            .map(|file| InstallFile {
                                path: file.archive_path.clone(),
                                bytes: file.bytes,
                                sha256: file.sha256.clone(),
                            })
                            .collect(),
                    },
                    files,
                })
            })
            .collect::<Result<Vec<_>, CudaRuntimeError>>()?;

        Ok(Self { archives })
    }

    pub fn archives(&self) -> &[CudaRuntimeArchive] {
        &self.archives
    }

    /// Every required file, across all archives.
    pub fn files(&self) -> impl Iterator<Item = &CudaRuntimeFile> {
        self.archives.iter().flat_map(CudaRuntimeArchive::files)
    }

    /// How many bytes a user downloads to install this.
    pub fn download_bytes(&self) -> u64 {
        self.archives
            .iter()
            .map(|archive| archive.spec.archive_bytes)
            .sum()
    }

    /// How much disk the installed runtime occupies.
    pub fn installed_bytes(&self) -> u64 {
        self.files().map(|file| file.bytes).sum()
    }

    pub fn file_count(&self) -> usize {
        self.files().count()
    }

    /// Download and installed totals for one component, for a UI that shows the
    /// split.
    pub fn component_bytes(&self, component: CudaRuntimeComponent) -> (u64, u64) {
        let download = self
            .archives
            .iter()
            .filter(|archive| archive.component == component)
            .map(|archive| archive.spec.archive_bytes)
            .sum();
        let installed = self
            .files()
            .filter(|file| file.component == component)
            .map(|file| file.bytes)
            .sum();
        (download, installed)
    }

    /// **The activation gate: is the whole runtime present?**
    ///
    /// A presence check, never a verification — it stats each file and compares
    /// lengths without reading a byte, because it is asked on the path to every
    /// warm and the set is 2.45 GB. The digests were enforced when the bytes
    /// were extracted, which is where a trust boundary belongs.
    ///
    /// It is deliberately all-or-nothing. Fourteen of fifteen files is not a
    /// degraded CUDA runtime, it is a CPU install with 2 GB of dead weight, and
    /// reporting it as available would hand the worker a provider whose
    /// dependencies cannot resolve.
    pub fn is_complete(&self, proof_dir: &Path) -> bool {
        self.files().all(|file| file.is_present(proof_dir))
    }

    /// Which components are fully present. For disclosure, not for activation:
    /// [`Self::is_complete`] is what decides whether CUDA may be selected.
    pub fn installed_components(&self, proof_dir: &Path) -> Vec<CudaRuntimeComponent> {
        [CudaRuntimeComponent::Core, CudaRuntimeComponent::Cudnn]
            .into_iter()
            .filter(|component| {
                self.files()
                    .filter(|file| file.component == *component)
                    .all(|file| file.is_present(proof_dir))
            })
            .collect()
    }

    /// Fetches and installs everything not already in place.
    ///
    /// Archives are handled one at a time — fetch, extract, verify, move in,
    /// delete the archive — rather than all fetched and then all installed.
    /// That keeps peak disk to one archive plus its own files instead of the
    /// whole 2.97 GB, and it costs nothing in safety: a partial result is
    /// exactly what [`Self::is_complete`] refuses.
    ///
    /// An archive whose files are already present is skipped, so a retry after
    /// a failure part way through fetches only what is missing.
    ///
    /// # Errors
    ///
    /// See [`CudaRuntimeError`]. [`CudaRuntimeError::RuntimeInUse`] means the
    /// worker holds a DLL this would replace and the engine must be shut down
    /// first.
    pub fn install(
        &self,
        paths: &CudaRuntimePaths,
        policy: &DownloadPolicy,
        cancel: &CancelToken,
        observe: &dyn Fn(CudaRuntimeEvent<'_>),
    ) -> Result<(), CudaRuntimeError> {
        self.preflight_disk(paths)?;
        fs::create_dir_all(&paths.proof)?;
        fs::create_dir_all(&paths.downloads)?;
        let mut bytes_completed = 0_u64;
        for archive in &self.archives {
            if cancel.is_cancelled() {
                return Err(CudaRuntimeError::Cancelled);
            }
            if archive
                .files
                .iter()
                .all(|file| file.is_present(&paths.proof))
            {
                bytes_completed = bytes_completed.saturating_add(archive.spec.archive_bytes);
                observe(CudaRuntimeEvent::Skipped { archive });
                continue;
            }
            let destination = paths.downloads.join(archive.download_file_name());
            let mut part_path = destination.clone().into_os_string();
            part_path.push(".part");
            let part_path = PathBuf::from(part_path);
            observe(CudaRuntimeEvent::Downloading {
                archive,
                part_path: &part_path,
                bytes_completed,
            });
            // `download_to_file` enforces the archive length and digest before
            // it renames the `.part` into place, so nothing below ever sees
            // bytes the manifest did not vouch for.
            download_to_file(
                &DownloadRequest {
                    url: archive.url.clone(),
                    destination: destination.clone(),
                    expected_bytes: archive.spec.archive_bytes,
                    expected_sha256: archive.spec.archive_sha256.clone(),
                },
                policy,
                cancel,
            )?;
            observe(CudaRuntimeEvent::Installing { archive });
            let result = install_one(archive, &destination, paths, cancel);
            // The archive is scratch either way: on success it is spent, and on
            // failure keeping 1.9 GB around to resume a transfer that already
            // completed buys nothing.
            let _ = fs::remove_file(&destination);
            result?;
            bytes_completed = bytes_completed.saturating_add(archive.spec.archive_bytes);
        }
        if !self.is_complete(&paths.proof) {
            return Err(CudaRuntimeError::StillIncomplete);
        }
        Ok(())
    }

    /// Refuses before the first byte when the volume cannot hold the result.
    ///
    /// Requires the installed total plus the largest single archive, because
    /// archives are fetched and discarded one at a time rather than all held at
    /// once.
    fn preflight_disk(&self, paths: &CudaRuntimePaths) -> Result<(), CudaRuntimeError> {
        let largest = self
            .archives
            .iter()
            .map(|archive| archive.spec.archive_bytes)
            .max()
            .unwrap_or_default();
        let required = self.installed_bytes().saturating_add(largest);
        let absolute = if paths.proof.is_absolute() {
            paths.proof.clone()
        } else {
            std::env::current_dir()?.join(&paths.proof)
        };
        let disks = Disks::new_with_refreshed_list();
        let Some(available) = disks
            .list()
            .iter()
            .filter(|disk| absolute.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().components().count())
            .map(sysinfo::Disk::available_space)
        else {
            // An uninventoriable volume is not evidence of a full one, and
            // refusing here would block the install on every filesystem
            // `sysinfo` does not recognise. The transfer fails loudly on its
            // own if space really does run out.
            return Ok(());
        };
        if available < required {
            return Err(CudaRuntimeError::InsufficientDisk {
                required,
                available,
            });
        }
        Ok(())
    }
}

/// Extracts one archive into a private stage and moves its files in.
///
/// Extraction goes to the stage rather than straight to `proof/` because
/// [`extract_required_files`] writes with `create_new`, and because a file that
/// fails its digest must never have existed beside the worker even briefly.
fn install_one(
    archive: &CudaRuntimeArchive,
    archive_path: &Path,
    paths: &CudaRuntimePaths,
    cancel: &CancelToken,
) -> Result<(), CudaRuntimeError> {
    let stage = paths.stage.join(&archive.artifact_id);
    remove_stage(&stage)?;
    let result = (|| {
        extract_required_files(archive_path, &stage, &archive.spec, cancel)?;
        for file in &archive.files {
            if cancel.is_cancelled() {
                return Err(CudaRuntimeError::Cancelled);
            }
            let staged = stage.join(&file.archive_path);
            // Every byte was hashed on the way out of the archive; this catches
            // a truncated write between then and now.
            if fs::metadata(&staged).map(|data| data.len()).ok() != Some(file.bytes) {
                return Err(CudaRuntimeError::StillIncomplete);
            }
            let destination = paths.proof.join(&file.installed_name);
            fs::rename(&staged, &destination).map_err(|error| {
                // Windows refuses to replace a mapped image. Reported as its own
                // case because the fix is "shut the engine down", which no other
                // IO failure here implies.
                if destination.exists() {
                    CudaRuntimeError::RuntimeInUse(destination.clone())
                } else {
                    CudaRuntimeError::Io(error)
                }
            })?;
        }
        Ok(())
    })();
    let _ = remove_stage(&stage);
    result
}

impl CudaRuntimeFile {
    /// Present at its recorded length. See [`CudaRuntimePlan::is_complete`] for
    /// why length and not digest.
    fn is_present(&self, proof_dir: &Path) -> bool {
        fs::metadata(proof_dir.join(&self.installed_name))
            .is_ok_and(|metadata| metadata.len() == self.bytes)
    }
}

fn remove_stage(stage: &Path) -> Result<(), CudaRuntimeError> {
    match fs::remove_dir_all(stage) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CudaRuntimeError::Io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled_manifest;

    fn plan() -> CudaRuntimePlan {
        CudaRuntimePlan::resolve(&bundled_manifest().expect("bundled manifest"))
            .expect("the shipped manifest must pin the whole CUDA runtime")
    }

    /// The figures the handoff commits to, asserted against the manifest rather
    /// than restated. If someone bumps a CUDA version without updating both,
    /// this is what says so.
    #[test]
    fn the_shipped_manifest_pins_the_whole_runtime_at_the_recorded_totals() {
        let plan = plan();
        assert_eq!(plan.file_count(), 15);
        assert_eq!(plan.archives().len(), 5);
        assert_eq!(plan.download_bytes(), 2_966_873_013);
        assert_eq!(plan.installed_bytes(), 2_451_924_088);

        let (core_download, core_installed) = plan.component_bytes(CudaRuntimeComponent::Core);
        let (cudnn_download, cudnn_installed) = plan.component_bytes(CudaRuntimeComponent::Cudnn);
        // Core is four archives because the provider DLL comes from sherpa, not
        // from NVIDIA -- three sources across five archives.
        assert_eq!(core_download, 1_062_420_913);
        assert_eq!(core_installed, 1_334_518_808);
        assert_eq!(cudnn_download, 1_904_452_100);
        assert_eq!(cudnn_installed, 1_117_405_280);
        assert_eq!(core_download + cudnn_download, plan.download_bytes());
        assert_eq!(core_installed + cudnn_installed, plan.installed_bytes());
    }

    /// Every archive prefix has to be right or extraction silently matches
    /// nothing, and every URL has to be a host the download policy admits.
    #[test]
    fn every_archive_carries_a_prefix_and_a_pinned_digest() {
        for archive in plan().archives() {
            assert!(
                !archive.spec.archive_prefix.as_os_str().is_empty(),
                "{} has no archive_prefix, so its members would not be found",
                archive.artifact_id
            );
            assert_eq!(archive.spec.archive_sha256.len(), 64);
            assert!(archive.url.starts_with("https://"));
            assert!(!archive.files().is_empty());
            assert_eq!(
                archive.spec.installed_bytes,
                archive.files().iter().map(|file| file.bytes).sum::<u64>()
            );
        }
    }

    /// The provider DLL is the file whose presence most reads as "CUDA is
    /// installed", so its archive must be the last one fetched. Ordering is the
    /// only thing protecting a reader that checks for it alone -- and one did:
    /// `ensure_ready` gated CUDA on this single file.
    #[test]
    fn the_execution_provider_is_the_last_thing_installed() {
        let plan = plan();
        let last = plan.archives().last().expect("archives");
        assert_eq!(
            last.artifact_id,
            "sherpa-onnx-runtime-windows-x64-cuda-12.x-cudnn-9.x-1.13.4"
        );
        assert_eq!(
            last.files()
                .iter()
                .map(|file| file.installed_name.as_str())
                .collect::<Vec<_>>(),
            ["onnxruntime_providers_cuda.dll"]
        );
    }

    /// The nine `cudnn64_9` sub-libraries load by name at run time and appear in
    /// no import table, so nothing but this list keeps them installed.
    #[test]
    fn the_cudnn_dispatch_stub_brings_its_sub_libraries() {
        let plan = plan();
        let mut cudnn = plan
            .files()
            .filter(|file| file.component == CudaRuntimeComponent::Cudnn)
            .map(|file| file.installed_name.clone())
            .collect::<Vec<_>>();
        cudnn.sort();
        assert_eq!(
            cudnn,
            [
                "cudnn64_9.dll",
                "cudnn_adv64_9.dll",
                "cudnn_cnn64_9.dll",
                "cudnn_engines_precompiled64_9.dll",
                "cudnn_engines_runtime_compiled64_9.dll",
                "cudnn_engines_tensor_ir64_9.dll",
                "cudnn_ext64_9.dll",
                "cudnn_graph64_9.dll",
                "cudnn_heuristic64_9.dll",
                "cudnn_ops64_9.dll",
            ]
        );
    }

    /// Everything installs flat beside the worker, because the loader searches
    /// one directory. Two archives nesting a same-named DLL under different
    /// paths would collide silently, so resolution refuses it.
    #[test]
    fn every_file_installs_flat_under_a_distinct_name() {
        let plan = plan();
        let mut names = plan
            .files()
            .map(|file| file.installed_name.clone())
            .collect::<Vec<_>>();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "installed names must be unique");
        for file in plan.files() {
            assert!(
                !file.installed_name.contains('/') && !file.installed_name.contains('\\'),
                "{} must install flat",
                file.installed_name
            );
            assert!(file.archive_path.parent().is_some());
        }
    }

    /// **Completeness is the activation gate, so it has to fail on 14 of 15.**
    ///
    /// Written against files of the recorded lengths rather than real DLLs: the
    /// claim under test is "all present at their recorded lengths", and the
    /// digests are enforced during extraction, not here.
    #[test]
    fn a_runtime_missing_one_file_is_not_available_and_a_short_file_is_absence() {
        let plan = plan();
        let temp = tempfile::tempdir().expect("temp");
        let proof = temp.path().join("proof");
        fs::create_dir_all(&proof).expect("proof dir");

        assert!(
            !plan.is_complete(&proof),
            "an empty proof dir has no runtime"
        );
        assert!(plan.installed_components(&proof).is_empty());

        let files: Vec<_> = plan.files().collect();
        // Sparse rather than 2.45 GB of zeroes: presence is a length check, so
        // set_len is exactly as convincing as writing the bytes and does not
        // need the disk.
        let write = |file: &CudaRuntimeFile, length: u64| {
            let handle = fs::File::create(proof.join(&file.installed_name)).expect("create");
            handle.set_len(length).expect("set length");
        };
        for file in files.iter().skip(1) {
            write(file, file.bytes);
        }
        assert!(
            !plan.is_complete(&proof),
            "one missing file must read as not installed, not as a degraded runtime"
        );

        write(files[0], files[0].bytes);
        assert!(plan.is_complete(&proof), "the whole set is installed");
        assert_eq!(
            plan.installed_components(&proof),
            [CudaRuntimeComponent::Core, CudaRuntimeComponent::Cudnn]
        );

        // A truncated file is absence, matching `InstallManager::is_present`.
        write(files[0], files[0].bytes - 1);
        assert!(!plan.is_complete(&proof));
    }

    /// Each component is reported present only when all of its own files are,
    /// which is what lets the UI say "core installed, cuDNN still missing"
    /// without ever implying CUDA can run.
    #[test]
    fn one_complete_component_does_not_make_the_runtime_available() {
        let plan = plan();
        let temp = tempfile::tempdir().expect("temp");
        let proof = temp.path().join("proof");
        fs::create_dir_all(&proof).expect("proof dir");
        for file in plan
            .files()
            .filter(|file| file.component == CudaRuntimeComponent::Cudnn)
        {
            let handle = fs::File::create(proof.join(&file.installed_name)).expect("create");
            handle.set_len(file.bytes).expect("set length");
        }
        assert_eq!(
            plan.installed_components(&proof),
            [CudaRuntimeComponent::Cudnn]
        );
        assert!(
            !plan.is_complete(&proof),
            "cuDNN alone cannot execute a node, so CUDA must not be selectable"
        );
    }

    /// **Every prefix, against the real archives.**
    ///
    /// This is the test the unit tests above cannot be: a mistyped
    /// `archive_prefix` is non-empty, so it passes all of them, and then matches
    /// no member at install time and fails as `MissingRequiredFiles` on a user's
    /// machine. Nothing short of real bytes distinguishes a correct prefix from a
    /// plausible one.
    ///
    /// It runs the production install path -- prefix stripping, per-file length
    /// and digest enforcement, flattening, and the move into `proof/` -- for all
    /// five archives, and asserts the result is a complete runtime. Download is
    /// the only step skipped, because the archives are already on disk and were
    /// hash-verified by `scripts/Get-GpuRuntime.ps1`.
    ///
    /// Ignored because it needs those archives and writes 2.45 GB.
    #[test]
    #[ignore = "requires .tools/gpu-runtime/download and .tools/sherpa-onnx populated by scripts/Get-GpuRuntime.ps1"]
    fn the_real_archives_install_flat_into_proof_at_their_pinned_prefixes() {
        let plan = plan();
        let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools");
        let temp = tempfile::tempdir().expect("temp");
        let paths = CudaRuntimePaths {
            proof: temp.path().join("proof"),
            downloads: temp.path().join("downloads"),
            stage: temp.path().join("stage"),
        };
        fs::create_dir_all(&paths.proof).expect("proof dir");

        for archive in plan.archives() {
            // Where `Get-GpuRuntime.ps1` leaves each one, found by URL leaf.
            let leaf = archive.url.rsplit('/').next().expect("url leaf");
            let candidates = [
                tools.join("gpu-runtime/download").join(leaf),
                tools.join("sherpa-onnx").join(leaf),
            ];
            let source = candidates
                .iter()
                .find(|path| path.is_file())
                .unwrap_or_else(|| panic!("missing archive for {}: {leaf}", archive.artifact_id));
            install_one(archive, source, &paths, &CancelToken::default()).unwrap_or_else(|error| {
                panic!(
                    "{} failed to install from {}: {error:?} -- MissingRequiredFiles here \
                         means archive_prefix {:?} does not match the archive's real layout",
                    archive.artifact_id,
                    source.display(),
                    archive.spec.archive_prefix,
                )
            });
        }

        assert!(
            plan.is_complete(&paths.proof),
            "all five archives installed, so the runtime must read as complete"
        );
        // Flat, and nothing else: the loader searches one directory, and a
        // leftover `bin/` would mean the stage was not fully drained.
        let mut installed: Vec<String> = fs::read_dir(&paths.proof)
            .expect("read proof")
            .map(|entry| {
                let entry = entry.expect("entry");
                assert!(entry.file_type().expect("file type").is_file());
                entry.file_name().to_string_lossy().into_owned()
            })
            .collect();
        installed.sort();
        let mut expected: Vec<String> = plan
            .files()
            .map(|file| file.installed_name.clone())
            .collect();
        expected.sort();
        assert_eq!(installed, expected);
    }

    /// A required path the manifest does not pin must be fatal, not fetched
    /// unverified. Proved by resolving against a manifest with the artifacts
    /// removed rather than by trusting the code path's shape.
    #[test]
    fn a_required_file_the_manifest_does_not_pin_refuses_to_resolve() {
        let raw = String::from_utf8(crate::BUNDLED_TRUSTED_MANIFEST_BYTES.to_vec())
            .expect("manifest is utf-8");
        let stripped = raw.replace("bin/x64/cudnn_graph64_9.dll", "bin/x64/renamed64_9.dll");
        assert_ne!(raw, stripped, "the fixture must actually change something");
        let manifest =
            TrustedManifest::parse_bundled(stripped.as_bytes()).expect("still a valid manifest");
        let error =
            CudaRuntimePlan::resolve(&manifest).expect_err("an unpinned required file must refuse");
        assert!(
            matches!(error, CudaRuntimeError::ManifestIncomplete(ref message)
                if message.contains("cudnn_graph64_9.dll")),
            "got {error:?}"
        );
    }
}
