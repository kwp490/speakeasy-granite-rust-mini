use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use fs2::FileExt;
use sha2::{Digest, Sha256};
use speakeasy_domain::CancelToken;
use sysinfo::Disks;

use crate::archive::extract_required_files;
use crate::{Archive, ArchiveEntry, ArchiveEntryKind, ArchiveLimits, Pack, validate_archive_plan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallSpec {
    pub id: String,
    pub revision: String,
    pub archive_prefix: PathBuf,
    pub archive_bytes: u64,
    pub archive_sha256: String,
    pub installed_bytes: u64,
    pub required_files: Vec<InstallFile>,
}

impl From<&Pack> for InstallSpec {
    fn from(pack: &Pack) -> Self {
        let archive = pack.archive();
        Self {
            id: pack.id().to_owned(),
            revision: pack.revision().to_owned(),
            archive_prefix: PathBuf::from(pack.archive_prefix()),
            // Zero means the pack is made from loose required files. Manifest
            // validation requires every real archive to have a positive size.
            archive_bytes: archive.map_or(0, Archive::bytes),
            archive_sha256: archive.map_or_else(String::new, |item| item.sha256().to_owned()),
            installed_bytes: pack.installed_bytes(),
            required_files: pack
                .required_files()
                .iter()
                .map(|file| InstallFile {
                    path: PathBuf::from(file.path()),
                    bytes: file.bytes(),
                    sha256: file.sha256().to_owned(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedArtifact {
    pub archive: Vec<u8>,
    pub files: Vec<(PathBuf, Vec<u8>)>,
}

/// One already-downloaded loose file and the relative path it must occupy in
/// an installed pack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LooseInstallFile {
    pub path: PathBuf,
    pub source: PathBuf,
}

#[derive(Debug)]
pub enum InstallError {
    Busy,
    InvalidSpec(&'static str),
    ArchiveMismatch,
    ArchiveExtraction(crate::ArchiveExtractionError),
    InsufficientDisk { required: u64, available: u64 },
    MissingOrUnexpectedFiles,
    FileMismatch(PathBuf),
    Cancelled,
    InUse,
    LockPoisoned,
    NotAppOwned,
    Io(io::Error),
}

impl Display for InstallError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "model install failed: {self:?}")
    }
}

impl Error for InstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::ArchiveExtraction(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<crate::ArchiveExtractionError> for InstallError {
    fn from(error: crate::ArchiveExtractionError) -> Self {
        Self::ArchiveExtraction(error)
    }
}

#[derive(Debug)]
pub struct InstallManager {
    root: PathBuf,
    leases: Arc<Mutex<HashMap<(String, String), usize>>>,
}

#[derive(Debug)]
pub struct ArtifactLease {
    key: (String, String),
    leases: Arc<Mutex<HashMap<(String, String), usize>>>,
}

impl Drop for ArtifactLease {
    fn drop(&mut self) {
        let Ok(mut leases) = self.leases.lock() else {
            return;
        };
        if let Some(count) = leases.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                leases.remove(&self.key);
            }
        }
    }
}

impl InstallManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            leases: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Verifies and atomically activates a staged artifact.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, hash/length mismatch, lock
    /// contention, unsafe ownership, or filesystem failure.
    pub fn install(
        &self,
        spec: &InstallSpec,
        staged: &StagedArtifact,
    ) -> Result<PathBuf, InstallError> {
        validate_identifier(&spec.id)?;
        validate_identifier(&spec.revision)?;
        fs::create_dir_all(&self.root)?;
        self.preflight_disk(spec)?;
        let _lock = ArtifactLock::acquire(
            &self.root,
            format!("{}:{}@{}", self.root.display(), spec.id, spec.revision),
        )?;
        verify_staged(spec, staged)?;
        let final_path = self.install_path(spec);
        let stage = self.root.join(format!(
            ".stage-{}-{}-{}",
            spec.id,
            spec.revision,
            std::process::id()
        ));
        let rollback = self
            .root
            .join(format!(".rollback-{}-{}", spec.id, spec.revision));
        remove_app_owned_dir(&self.root, &stage)?;
        fs::create_dir_all(&stage)?;
        let result = (|| {
            for (relative, bytes) in &staged.files {
                let destination = stage.join(relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)?;
                output.write_all(bytes)?;
                output.sync_all()?;
            }
            activate_stage(&self.root, &stage, &final_path, &rollback, false)?;
            Ok(final_path.clone())
        })();
        if result.is_err() {
            let _ = remove_app_owned_dir(&self.root, &stage);
        }
        result
    }

    /// Validates a trusted archive, streams only exact required files to a
    /// private stage, and atomically activates the completed revision.
    ///
    /// # Errors
    ///
    /// Returns an error for archive mismatch, hostile metadata, cancellation,
    /// lock contention, required-file mismatch, or filesystem failure.
    pub fn install_archive(
        &self,
        spec: &InstallSpec,
        archive_path: &Path,
        cancel: &CancelToken,
    ) -> Result<PathBuf, InstallError> {
        validate_identifier(&spec.id)?;
        validate_identifier(&spec.revision)?;
        if spec.archive_bytes == 0 || spec.archive_sha256.is_empty() {
            return Err(InstallError::InvalidSpec(
                "archive installation requires archive metadata",
            ));
        }
        verify_archive_file(spec, archive_path)?;
        fs::create_dir_all(&self.root)?;
        self.preflight_disk(spec)?;
        let _lock = ArtifactLock::acquire(
            &self.root,
            format!("{}:{}@{}", self.root.display(), spec.id, spec.revision),
        )?;
        let final_path = self.install_path(spec);
        let stage = self.root.join(format!(
            ".stage-{}-{}-{}",
            spec.id,
            spec.revision,
            std::process::id()
        ));
        let rollback = self
            .root
            .join(format!(".rollback-{}-{}", spec.id, spec.revision));
        remove_app_owned_dir(&self.root, &stage)?;
        fs::create_dir_all(&stage)?;
        let result = (|| {
            extract_required_files(archive_path, &stage, spec, cancel)?;
            activate_stage(&self.root, &stage, &final_path, &rollback, false)?;
            Ok(final_path.clone())
        })();
        if result.is_err() {
            let _ = remove_app_owned_dir(&self.root, &stage);
        }
        result
    }

    /// Streams a complete set of loose downloads into a private stage, checks
    /// every trusted length and digest, and atomically activates the revision.
    ///
    /// # Errors
    ///
    /// Returns an error for archive metadata, an unsafe or incomplete file
    /// plan, cancellation, mismatch, lock contention, or filesystem failure.
    pub fn install_loose_files(
        &self,
        spec: &InstallSpec,
        files: &[LooseInstallFile],
        cancel: &CancelToken,
    ) -> Result<PathBuf, InstallError> {
        validate_identifier(&spec.id)?;
        validate_identifier(&spec.revision)?;
        if spec.archive_bytes != 0 || !spec.archive_sha256.is_empty() {
            return Err(InstallError::InvalidSpec(
                "loose-file installation must not carry archive metadata",
            ));
        }
        validate_loose_file_plan(spec, files)?;
        fs::create_dir_all(&self.root)?;
        self.preflight_disk(spec)?;
        let _lock = ArtifactLock::acquire(
            &self.root,
            format!("{}:{}@{}", self.root.display(), spec.id, spec.revision),
        )?;
        let final_path = self.install_path(spec);
        let stage = self.root.join(format!(
            ".stage-{}-{}-{}",
            spec.id,
            spec.revision,
            std::process::id()
        ));
        let rollback = self
            .root
            .join(format!(".rollback-{}-{}", spec.id, spec.revision));
        remove_app_owned_dir(&self.root, &stage)?;
        fs::create_dir_all(&stage)?;
        let result = (|| {
            for required in &spec.required_files {
                if cancel.is_cancelled() {
                    return Err(InstallError::Cancelled);
                }
                let source = files
                    .iter()
                    .find(|file| file.path == required.path)
                    .ok_or(InstallError::MissingOrUnexpectedFiles)?;
                copy_verified_file(
                    required,
                    &source.source,
                    &stage.join(&required.path),
                    cancel,
                )?;
            }
            activate_stage(&self.root, &stage, &final_path, &rollback, false)?;
            Ok(final_path.clone())
        })();
        if result.is_err() {
            let _ = remove_app_owned_dir(&self.root, &stage);
        }
        result
    }

    /// Rehashes every required installed file before runtime activation.
    ///
    /// # Errors
    ///
    /// Returns an error when a required file is absent, changed, or unreadable.
    pub fn reverify(&self, spec: &InstallSpec) -> Result<(), InstallError> {
        let path = self.install_path(spec);
        let files = spec
            .required_files
            .iter()
            .map(|file| fs::read(path.join(&file.path)).map(|bytes| (file.path.clone(), bytes)))
            .collect::<Result<Vec<_>, _>>()?;
        verify_files(spec, &files)
    }

    /// Whether every required file is on disk at its recorded length.
    ///
    /// This is a **presence check, not a verification.** It stats each required
    /// file and compares lengths; it never reads or hashes a byte. Presence is
    /// the weaker claim on purpose, and it must not be substituted for
    /// [`Self::reverify`] at an activation boundary — a pack that passes here
    /// and fails there is a corrupted install, and it has to fail.
    ///
    /// It exists because "is this pack installed?" is a question the *selection*
    /// path has to ask, and `reverify` cannot answer it at that price: it reads
    /// every required file into memory and SHA-256s it, which is 2.5 GB for the
    /// CUDA streaming pack. Selection asking that question by hashing would put
    /// a multi-second read on the path to every dictation.
    ///
    /// So the division is: this decides *which* pack to reach for, `reverify`
    /// decides whether its bytes may be trusted.
    pub fn is_present(&self, spec: &InstallSpec) -> bool {
        let path = self.install_path(spec);
        spec.required_files.iter().all(|file| {
            fs::metadata(path.join(&file.path)).is_ok_and(|metadata| metadata.len() == file.bytes)
        })
    }

    /// Acquires a reference-counted lease after reverification.
    ///
    /// # Errors
    ///
    /// Returns an error when reverification or lease locking fails.
    pub fn lease(&self, spec: &InstallSpec) -> Result<ArtifactLease, InstallError> {
        self.reverify(spec)?;
        let key = (spec.id.clone(), spec.revision.clone());
        *self
            .leases
            .lock()
            .map_err(|_| InstallError::LockPoisoned)?
            .entry(key.clone())
            .or_default() += 1;
        Ok(ArtifactLease {
            key,
            leases: Arc::clone(&self.leases),
        })
    }

    /// Deletes exactly one app-owned inactive pack revision.
    ///
    /// # Errors
    ///
    /// Returns an error when the pack is leased, busy, outside the app-owned
    /// root, or cannot be removed.
    pub fn delete(&self, spec: &InstallSpec) -> Result<(), InstallError> {
        let key = (spec.id.clone(), spec.revision.clone());
        if self
            .leases
            .lock()
            .map_err(|_| InstallError::LockPoisoned)?
            .contains_key(&key)
        {
            return Err(InstallError::InUse);
        }
        fs::create_dir_all(&self.root)?;
        let _lock = ArtifactLock::acquire(
            &self.root,
            format!("{}:{}@{}", self.root.display(), spec.id, spec.revision),
        )?;
        remove_app_owned_dir(&self.root, &self.install_path(spec))
    }

    /// Confirms the target volume can hold the archive, new stage, and a
    /// rollback copy of an existing revision.
    ///
    /// # Errors
    ///
    /// Returns an error when arithmetic overflows, the volume is unknown, or
    /// available space is below the conservative requirement.
    pub fn preflight_disk(&self, spec: &InstallSpec) -> Result<u64, InstallError> {
        let absolute_root = if self.root.is_absolute() {
            self.root.clone()
        } else {
            std::env::current_dir()?.join(&self.root)
        };
        let disks = Disks::new_with_refreshed_list();
        let available = disks
            .list()
            .iter()
            .filter(|disk| absolute_root.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().components().count())
            .map(sysinfo::Disk::available_space)
            .ok_or(InstallError::InvalidSpec(
                "target volume could not be inventoried",
            ))?;
        self.preflight_disk_with_available(spec, available)
    }

    fn preflight_disk_with_available(
        &self,
        spec: &InstallSpec,
        available: u64,
    ) -> Result<u64, InstallError> {
        let rollback = if self.install_path(spec).exists() {
            spec.installed_bytes
        } else {
            0
        };
        let transfer_bytes = if spec.archive_bytes == 0 {
            spec.installed_bytes
        } else {
            spec.archive_bytes
        };
        let required = transfer_bytes
            .checked_add(spec.installed_bytes)
            .and_then(|bytes| bytes.checked_add(rollback))
            .ok_or(InstallError::InvalidSpec("disk requirement overflow"))?;
        if available < required {
            return Err(InstallError::InsufficientDisk {
                required,
                available,
            });
        }
        Ok(required)
    }

    /// Removes abandoned sibling staging directories under the app-owned root.
    ///
    /// # Errors
    ///
    /// Returns an error when directory enumeration or safe removal fails.
    pub fn cleanup_abandoned_stages(&self) -> Result<usize, InstallError> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut removed = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && entry.file_name().to_string_lossy().starts_with(".stage-")
            {
                remove_app_owned_dir(&self.root, &entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Returns the app-owned activation path for an admitted install spec.
    /// Callers must still reverify the pack before runtime use.
    pub fn installed_path(&self, spec: &InstallSpec) -> PathBuf {
        self.install_path(spec)
    }

    fn install_path(&self, spec: &InstallSpec) -> PathBuf {
        self.root.join(&spec.id).join(&spec.revision)
    }
}

fn validate_loose_file_plan(
    spec: &InstallSpec,
    files: &[LooseInstallFile],
) -> Result<(), InstallError> {
    let allowed_files: HashSet<_> = spec
        .required_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let entries = spec.required_files.iter().map(|file| ArchiveEntry {
        path: file.path.clone(),
        kind: ArchiveEntryKind::File,
        compressed_bytes: file.bytes,
        extracted_bytes: file.bytes,
    });
    validate_archive_plan(
        entries,
        &allowed_files,
        ArchiveLimits {
            maximum_files: spec.required_files.len(),
            maximum_extracted_bytes: spec.installed_bytes,
            maximum_compression_ratio: 1,
        },
    )
    .map_err(|_| InstallError::InvalidSpec("unsafe or colliding required-file path"))?;
    let actual: HashSet<_> = files.iter().map(|file| file.path.as_path()).collect();
    let expected: HashSet<_> = spec
        .required_files
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    if files.len() != spec.required_files.len() || actual != expected {
        return Err(InstallError::MissingOrUnexpectedFiles);
    }
    Ok(())
}

fn copy_verified_file(
    required: &InstallFile,
    source: &Path,
    destination: &Path,
    cancel: &CancelToken,
) -> Result<(), InstallError> {
    let mut input = fs::File::open(source)?;
    if input.metadata()?.len() != required.bytes {
        return Err(InstallError::FileMismatch(required.path.clone()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024].into_boxed_slice();
    loop {
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .ok_or_else(|| InstallError::FileMismatch(required.path.clone()))?;
        if copied > required.bytes {
            return Err(InstallError::FileMismatch(required.path.clone()));
        }
        hasher.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    if copied != required.bytes
        || !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&required.sha256)
    {
        return Err(InstallError::FileMismatch(required.path.clone()));
    }
    Ok(())
}

fn verify_staged(spec: &InstallSpec, staged: &StagedArtifact) -> Result<(), InstallError> {
    if staged.archive.len() as u64 != spec.archive_bytes
        || digest(&staged.archive) != spec.archive_sha256
    {
        return Err(InstallError::ArchiveMismatch);
    }
    verify_files(spec, &staged.files)
}

fn verify_archive_file(spec: &InstallSpec, path: &Path) -> Result<(), InstallError> {
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() != spec.archive_bytes {
        return Err(InstallError::ArchiveMismatch);
    }
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    if !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&spec.archive_sha256) {
        return Err(InstallError::ArchiveMismatch);
    }
    Ok(())
}

fn verify_files(spec: &InstallSpec, files: &[(PathBuf, Vec<u8>)]) -> Result<(), InstallError> {
    let allowed_files: HashSet<_> = spec
        .required_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let entries = files.iter().map(|(path, bytes)| ArchiveEntry {
        path: path.clone(),
        kind: ArchiveEntryKind::File,
        compressed_bytes: bytes.len() as u64,
        extracted_bytes: bytes.len() as u64,
    });
    validate_archive_plan(
        entries,
        &allowed_files,
        ArchiveLimits {
            maximum_files: spec.required_files.len(),
            maximum_extracted_bytes: spec.installed_bytes,
            maximum_compression_ratio: 1,
        },
    )
    .map_err(|_| InstallError::InvalidSpec("unsafe or colliding required-file path"))?;

    let expected: HashSet<_> = spec
        .required_files
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    let actual: HashSet<_> = files.iter().map(|(path, _)| path.as_path()).collect();
    if expected != actual
        || files
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum::<u64>()
            > spec.installed_bytes
    {
        return Err(InstallError::MissingOrUnexpectedFiles);
    }
    for required in &spec.required_files {
        let bytes = files
            .iter()
            .find(|(path, _)| path == &required.path)
            .map(|(_, bytes)| bytes)
            .ok_or(InstallError::MissingOrUnexpectedFiles)?;
        if bytes.len() as u64 != required.bytes || digest(bytes) != required.sha256 {
            return Err(InstallError::FileMismatch(required.path.clone()));
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_identifier(identifier: &str) -> Result<(), InstallError> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(InstallError::InvalidSpec(
            "artifact identifiers must be safe path components",
        ));
    }
    Ok(())
}

fn activate_stage(
    root: &Path,
    stage: &Path,
    final_path: &Path,
    rollback: &Path,
    inject_activation_failure: bool,
) -> Result<(), InstallError> {
    if final_path.exists() {
        remove_app_owned_dir(root, rollback)?;
        fs::rename(final_path, rollback)?;
    }
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let activation = if inject_activation_failure {
        Err(io::Error::other("injected activation failure"))
    } else {
        fs::rename(stage, final_path)
    };
    if let Err(error) = activation {
        if rollback.exists() {
            fs::rename(rollback, final_path)?;
        }
        return Err(InstallError::Io(error));
    }
    remove_app_owned_dir(root, rollback)
}

fn remove_app_owned_dir(root: &Path, target: &Path) -> Result<(), InstallError> {
    if !target.exists() {
        return Ok(());
    }
    let root = fs::canonicalize(root)?;
    let target = fs::canonicalize(target)?;
    if target == root || !target.starts_with(&root) {
        return Err(InstallError::NotAppOwned);
    }
    fs::remove_dir_all(target)?;
    Ok(())
}

static INSTALL_LOCKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct ArtifactLock {
    key: String,
    file: fs::File,
}

impl ArtifactLock {
    fn acquire(root: &Path, key: String) -> Result<Self, InstallError> {
        let mut locks = INSTALL_LOCKS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map_err(|_| InstallError::LockPoisoned)?;
        if !locks.insert(key.clone()) {
            return Err(InstallError::Busy);
        }
        let lock_result = (|| {
            let lock_root = root.join(".locks");
            fs::create_dir_all(&lock_root)?;
            let lock_name = format!("{:x}.lock", Sha256::digest(key.as_bytes()));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_root.join(lock_name))?;
            file.try_lock_exclusive().map_err(|error| {
                let windows_contention =
                    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33));
                if error.kind() == io::ErrorKind::WouldBlock || windows_contention {
                    InstallError::Busy
                } else {
                    InstallError::Io(error)
                }
            })?;
            Ok::<_, InstallError>(file)
        })();
        match lock_result {
            Ok(file) => Ok(Self { key, file }),
            Err(error) => {
                locks.remove(&key);
                Err(error)
            }
        }
    }
}

impl Drop for ArtifactLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        if let Some(locks) = INSTALL_LOCKS.get()
            && let Ok(mut locks) = locks.lock()
        {
            locks.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArchiveExtractionError;
    use bzip2::write::BzEncoder;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};
    use tar::{Builder, Header};

    fn fixture() -> (InstallSpec, StagedArtifact) {
        let archive = b"synthetic archive".to_vec();
        let model = b"model bytes".to_vec();
        (
            InstallSpec {
                id: "synthetic".to_owned(),
                revision: "r1".to_owned(),
                archive_prefix: PathBuf::new(),
                archive_bytes: archive.len() as u64,
                archive_sha256: digest(&archive),
                installed_bytes: model.len() as u64,
                required_files: vec![InstallFile {
                    path: PathBuf::from("model.bin"),
                    bytes: model.len() as u64,
                    sha256: digest(&model),
                }],
            },
            StagedArtifact {
                archive,
                files: vec![(PathBuf::from("model.bin"), model)],
            },
        )
    }

    /// How the fixture archive is compressed.
    ///
    /// Both are real: the self-exported CUDA pack ships as `.tar.gz` and
    /// sherpa-onnx publishes its model packs as `.tar.bz2`, so the installer
    /// has to read either.
    #[derive(Clone, Copy)]
    enum Packing {
        Gzip,
        Bzip2,
    }

    fn archive_fixture(path: &Path, hostile_link: bool) -> InstallSpec {
        archive_fixture_packed(path, hostile_link, Packing::Gzip)
    }

    fn archive_fixture_packed(path: &Path, hostile_link: bool, packing: Packing) -> InstallSpec {
        let model = b"model bytes";
        // Built uncompressed first so one tar can be handed to either encoder;
        // the two encoders are different types and cannot share a Builder.
        let mut archive = Builder::new(Vec::new());
        if hostile_link {
            let mut header = Header::new_gnu();
            header.set_entry_type(tar::EntryType::symlink());
            header.set_size(0);
            header.set_mode(0o777);
            header.set_cksum();
            archive
                .append_link(&mut header, "model-link.bin", "model.bin")
                .expect("link");
        }
        let mut header = Header::new_gnu();
        header.set_size(model.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "model.bin", model.as_slice())
            .expect("model");
        let tar = archive.into_inner().expect("finish tar");
        let output = fs::File::create(path).expect("archive file");
        match packing {
            Packing::Gzip => {
                let mut encoder = GzEncoder::new(output, Compression::default());
                encoder.write_all(&tar).expect("gzip tar");
                encoder.finish().expect("finish gzip");
            }
            Packing::Bzip2 => {
                let mut encoder = BzEncoder::new(output, bzip2::Compression::default());
                encoder.write_all(&tar).expect("bzip2 tar");
                encoder.finish().expect("finish bzip2");
            }
        }
        let bytes = fs::read(path).expect("archive bytes");
        InstallSpec {
            id: "archive-synthetic".to_owned(),
            revision: "r1".to_owned(),
            archive_prefix: PathBuf::new(),
            archive_bytes: bytes.len() as u64,
            archive_sha256: digest(&bytes),
            installed_bytes: model.len() as u64,
            required_files: vec![InstallFile {
                path: PathBuf::from("model.bin"),
                bytes: model.len() as u64,
                sha256: digest(model),
            }],
        }
    }

    #[test]
    fn atomic_install_reverify_lease_delete_and_cleanup() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let (spec, staged) = fixture();
        let installed = manager.install(&spec, &staged).expect("install");
        assert!(installed.join("model.bin").is_file());
        manager.reverify(&spec).expect("reverify");
        let lease = manager.lease(&spec).expect("lease");
        assert!(matches!(manager.delete(&spec), Err(InstallError::InUse)));
        drop(lease);
        manager.delete(&spec).expect("delete after lease");

        let abandoned = temp.path().join("models/.stage-abandoned");
        fs::create_dir_all(&abandoned).expect("stage");
        assert_eq!(manager.cleanup_abandoned_stages().expect("cleanup"), 1);
    }

    #[test]
    fn presence_tracks_installedness_without_standing_in_for_verification() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let (spec, staged) = fixture();

        assert!(
            !manager.is_present(&spec),
            "nothing is installed, so selection must not reach for this pack"
        );
        manager.install(&spec, &staged).expect("install");
        assert!(manager.is_present(&spec), "an installed pack is present");

        // The distinction the name promises: same length, different bytes.
        // Presence is a cheap routing question and answers yes; `reverify` is
        // the trust boundary and answers no. If this ever agreed with
        // `reverify` here, `is_present` would have quietly become a security
        // check that does not hash anything.
        let file = manager.installed_path(&spec).join("model.bin");
        let mut swapped = fs::read(&file).expect("installed bytes");
        swapped[0] ^= 0xFF;
        fs::write(&file, &swapped).expect("corrupt in place");
        assert!(manager.is_present(&spec), "same length, so still present");
        assert!(
            manager.reverify(&spec).is_err(),
            "but the hash check must still refuse it"
        );

        // A short read is absence, not corruption: the length no longer matches.
        fs::write(&file, b"tru").expect("truncate");
        assert!(!manager.is_present(&spec));
    }

    #[test]
    fn failed_replacement_keeps_known_good_revision() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let (spec, staged) = fixture();
        manager.install(&spec, &staged).expect("initial install");
        let mut corrupt = staged.clone();
        corrupt.files[0].1.push(0);
        assert!(manager.install(&spec, &corrupt).is_err());
        manager.reverify(&spec).expect("known good remains");
    }

    #[test]
    fn loose_files_are_verified_and_activated_only_as_a_complete_pack() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let first = b"first model file";
        let second = b"second model file";
        let first_source = temp.path().join("first.download");
        let second_source = temp.path().join("second.download");
        fs::write(&first_source, first).expect("first download");
        fs::write(&second_source, second).expect("second download");
        let spec = InstallSpec {
            id: "loose-synthetic".to_owned(),
            revision: "r1".to_owned(),
            archive_prefix: PathBuf::new(),
            archive_bytes: 0,
            archive_sha256: String::new(),
            installed_bytes: (first.len() + second.len()) as u64,
            required_files: vec![
                InstallFile {
                    path: PathBuf::from("model.bin"),
                    bytes: first.len() as u64,
                    sha256: digest(first),
                },
                InstallFile {
                    path: PathBuf::from("projector.bin"),
                    bytes: second.len() as u64,
                    sha256: digest(second),
                },
            ],
        };
        let mut files = vec![
            LooseInstallFile {
                path: PathBuf::from("model.bin"),
                source: first_source,
            },
            LooseInstallFile {
                path: PathBuf::from("projector.bin"),
                source: second_source.clone(),
            },
        ];

        fs::write(&second_source, b"corrupt model file!").expect("corrupt second download");
        assert!(matches!(
            manager.install_loose_files(&spec, &files, &CancelToken::default()),
            Err(InstallError::FileMismatch(path)) if path == Path::new("projector.bin")
        ));
        assert!(!manager.installed_path(&spec).exists());
        assert_eq!(manager.cleanup_abandoned_stages().expect("clean stages"), 0);

        fs::write(&second_source, second).expect("restore second download");
        let installed = manager
            .install_loose_files(&spec, &files, &CancelToken::default())
            .expect("install loose files");
        assert_eq!(fs::read(installed.join("model.bin")).unwrap(), first);
        assert_eq!(fs::read(installed.join("projector.bin")).unwrap(), second);
        manager.reverify(&spec).expect("reverify loose pack");

        files.pop();
        assert!(matches!(
            manager.install_loose_files(&spec, &files, &CancelToken::default()),
            Err(InstallError::MissingOrUnexpectedFiles)
        ));
        manager
            .reverify(&spec)
            .expect("incomplete replacement preserves installed pack");
    }

    #[test]
    fn cancelled_loose_install_never_exposes_a_partial_pack() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let bytes = b"model bytes";
        let source = temp.path().join("model.download");
        fs::write(&source, bytes).expect("download");
        let spec = InstallSpec {
            id: "loose-cancelled".to_owned(),
            revision: "r1".to_owned(),
            archive_prefix: PathBuf::new(),
            archive_bytes: 0,
            archive_sha256: String::new(),
            installed_bytes: bytes.len() as u64,
            required_files: vec![InstallFile {
                path: PathBuf::from("model.bin"),
                bytes: bytes.len() as u64,
                sha256: digest(bytes),
            }],
        };
        let cancel = CancelToken::default();
        cancel.cancel();

        assert!(matches!(
            manager.install_loose_files(
                &spec,
                &[LooseInstallFile {
                    path: PathBuf::from("model.bin"),
                    source,
                }],
                &cancel,
            ),
            Err(InstallError::Cancelled)
        ));
        assert!(!manager.is_present(&spec));
    }

    #[test]
    fn granite_pack_converts_to_a_loose_install_spec() {
        let manifest = crate::bundled_manifest().expect("bundled manifest");
        let pack = manifest
            .packs()
            .iter()
            .find(|pack| pack.id() == "granite-speech-4.1-2b-q4_k_m-cpu")
            .expect("Granite Q4 pack");
        let spec = InstallSpec::from(pack);

        assert_eq!(spec.archive_bytes, 0);
        assert!(spec.archive_sha256.is_empty());
        assert_eq!(spec.required_files.len(), 2);
        assert_eq!(spec.installed_bytes, 2_298_601_952);
    }

    #[test]
    fn install_spec_cannot_escape_staging_root() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let (mut spec, mut staged) = fixture();
        spec.required_files[0].path = PathBuf::from("../outside.bin");
        staged.files[0].0 = PathBuf::from("../outside.bin");
        assert!(matches!(
            manager.install(&spec, &staged),
            Err(InstallError::InvalidSpec(_))
        ));
        assert!(!temp.path().join("outside.bin").exists());
    }

    #[test]
    fn archive_install_streams_exact_files_and_rejects_links() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let archive_path = temp.path().join("valid.tar.gz");
        let spec = archive_fixture(&archive_path, false);
        let installed = manager
            .install_archive(&spec, &archive_path, &CancelToken::default())
            .expect("archive install");
        assert_eq!(
            fs::read(installed.join("model.bin")).unwrap(),
            b"model bytes"
        );

        manager.delete(&spec).expect("remove valid install");
        let hostile_path = temp.path().join("hostile.tar.gz");
        let hostile_spec = archive_fixture(&hostile_path, true);
        assert!(matches!(
            manager.install_archive(&hostile_spec, &hostile_path, &CancelToken::default()),
            Err(InstallError::ArchiveExtraction(_))
        ));
        assert!(!temp.path().join("models/archive-synthetic/r1").exists());
    }

    #[test]
    fn a_bzip2_archive_installs_the_same_as_a_gzip_one() {
        // sherpa-onnx publishes every model pack as .tar.bz2, so this is the
        // format the CPU pack actually arrives in. The extension is deliberately
        // wrong here: compression is detected from the archive's leading bytes,
        // not from what the manifest happened to call the file.
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let archive_path = temp.path().join("misnamed.tar.gz");
        let spec = archive_fixture_packed(&archive_path, false, Packing::Bzip2);

        let installed = manager
            .install_archive(&spec, &archive_path, &CancelToken::default())
            .expect("bzip2 archive install");

        assert_eq!(
            fs::read(installed.join("model.bin")).unwrap(),
            b"model bytes"
        );
    }

    #[test]
    fn an_archive_in_no_recognized_compression_is_refused_before_parsing() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let archive_path = temp.path().join("plain.tar.gz");
        // Bytes in no compression this installer knows. The spec is built to
        // match them exactly, so integrity passes and the refusal has to come
        // from the compression check rather than from a digest mismatch — the
        // point is that an unknown envelope is caught, not that corruption is.
        let raw = b"not a compressed archive at all";
        fs::write(&archive_path, raw).expect("write");
        let model = b"model bytes";
        let spec = InstallSpec {
            id: "uncompressed-synthetic".to_owned(),
            revision: "r1".to_owned(),
            archive_prefix: PathBuf::new(),
            archive_bytes: raw.len() as u64,
            archive_sha256: digest(raw),
            installed_bytes: model.len() as u64,
            required_files: vec![InstallFile {
                path: PathBuf::from("model.bin"),
                bytes: model.len() as u64,
                sha256: digest(model),
            }],
        };

        assert!(matches!(
            manager.install_archive(&spec, &archive_path, &CancelToken::default()),
            Err(InstallError::ArchiveExtraction(
                ArchiveExtractionError::UnsupportedCompression
            ))
        ));
    }

    #[test]
    fn disk_preflight_accounts_for_archive_stage_and_rollback() {
        let temp = tempfile::tempdir().expect("temp root");
        let manager = InstallManager::new(temp.path().join("models"));
        let (spec, staged) = fixture();
        let initial_required = spec.archive_bytes + spec.installed_bytes;
        assert!(matches!(
            manager.preflight_disk_with_available(&spec, initial_required - 1),
            Err(InstallError::InsufficientDisk { .. })
        ));
        manager.install(&spec, &staged).expect("initial install");
        let replacement_required = initial_required + spec.installed_bytes;
        assert_eq!(
            manager
                .preflight_disk_with_available(&spec, replacement_required)
                .expect("replacement space"),
            replacement_required
        );
    }

    #[test]
    fn externally_held_file_lock_blocks_install() {
        let temp = tempfile::tempdir().expect("temp root");
        let root = temp.path().join("models");
        fs::create_dir_all(root.join(".locks")).expect("lock root");
        let manager = InstallManager::new(&root);
        let (spec, staged) = fixture();
        let key = format!("{}:{}@{}", root.display(), spec.id, spec.revision);
        let ready = temp.path().join("lock-ready");
        let release = temp.path().join("lock-release");
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "manager::tests::lock_holder_process",
                "--nocapture",
            ])
            .env("SPEAKEASY_TEST_LOCK_ROOT", &root)
            .env("SPEAKEASY_TEST_LOCK_KEY", &key)
            .env("SPEAKEASY_TEST_LOCK_READY", &ready)
            .env("SPEAKEASY_TEST_LOCK_RELEASE", &release)
            .spawn()
            .expect("spawn lock holder");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "child did not acquire lock");
        let result = manager.install(&spec, &staged);
        fs::write(&release, b"release").expect("release holder");
        assert!(child.wait().expect("wait for holder").success());
        assert!(
            matches!(result, Err(InstallError::Busy)),
            "unexpected install result: {result:?}"
        );
    }

    #[test]
    fn lock_holder_process() {
        let Some(root) = std::env::var_os("SPEAKEASY_TEST_LOCK_ROOT").map(PathBuf::from) else {
            return;
        };
        let key = std::env::var("SPEAKEASY_TEST_LOCK_KEY").expect("lock key");
        let ready = PathBuf::from(std::env::var_os("SPEAKEASY_TEST_LOCK_READY").expect("ready"));
        let release =
            PathBuf::from(std::env::var_os("SPEAKEASY_TEST_LOCK_RELEASE").expect("release"));
        let lock_name = format!("{:x}.lock", Sha256::digest(key.as_bytes()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(".locks").join(lock_name))
            .expect("lock file");
        file.lock_exclusive().expect("acquire child lock");
        fs::write(ready, b"ready").expect("signal ready");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !release.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(release.exists(), "parent did not release holder");
        file.unlock().expect("unlock child lock");
    }

    #[test]
    fn rollback_path_obstruction_preserves_known_good_revision() {
        let temp = tempfile::tempdir().expect("temp root");
        let root = temp.path().join("models");
        let manager = InstallManager::new(&root);
        let (spec, staged) = fixture();
        manager.install(&spec, &staged).expect("initial install");
        fs::write(root.join(".rollback-synthetic-r1"), b"obstruction")
            .expect("rollback obstruction");
        assert!(manager.install(&spec, &staged).is_err());
        manager.reverify(&spec).expect("known good remains");
    }

    #[test]
    fn activation_failure_restores_rolled_back_revision() {
        let temp = tempfile::tempdir().expect("temp root");
        let root = temp.path().join("models");
        let manager = InstallManager::new(&root);
        let (spec, staged) = fixture();
        let final_path = manager.install(&spec, &staged).expect("initial install");
        let stage = root.join(".stage-injected");
        let rollback = root.join(".rollback-synthetic-r1");
        fs::create_dir(&stage).expect("injected stage");
        fs::write(stage.join("model.bin"), b"replacement").expect("replacement");
        assert!(activate_stage(&root, &stage, &final_path, &rollback, true).is_err());
        manager.reverify(&spec).expect("known good restored");
        assert!(!rollback.exists());
        assert!(stage.exists());
    }
}
