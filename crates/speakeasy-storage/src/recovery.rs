use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{DATABASE_SCHEMA_VERSION, SettingsStore};

const RECOVERY_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 4 * 1_024 * 1_024;
const UPDATE_HEALTH_TIMEOUT_MS: i64 = 120_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileRecord {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub schema_version: u32,
    pub product_version: String,
    pub created_unix_ms: i64,
    pub local_development_unsigned: bool,
    pub installer: FileRecord,
    /// Optional attached-file integrity record. This does not establish a
    /// publisher identity or Authenticode authenticity.
    #[serde(alias = "signature")]
    pub signature_record: Option<FileRecord>,
    pub data: Vec<FileRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PendingUpdate {
    schema_version: u32,
    target_version: String,
    backup_manifest: String,
    marked_unix_ms: i64,
    health_timeout_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestoreOutcome {
    Restored { files: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HealthCheckOutcome {
    NoPendingUpdate,
    Cleared,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingUpdateStatus {
    None,
    AwaitingHealth { deadline_unix_ms: i64 },
    HealthTimedOut { backup_manifest: PathBuf },
}

#[derive(Debug)]
pub enum RecoveryError {
    Io(io::Error),
    Json(serde_json::Error),
    Sql(rusqlite::Error),
    InvalidInput(&'static str),
    VerificationFailed(String),
    DestinationNotEmpty,
    SignatureRequired,
    VersionMismatch,
}

impl From<io::Error> for RecoveryError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for RecoveryError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<rusqlite::Error> for RecoveryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

/// Creates one immutable installer/data recovery bundle while the caller owns
/// the application-update operation lease and the application is idle.
///
/// `SQLite` files are copied through `SQLite`'s online backup API. Other files are
/// copied as ordinary idle-state snapshots. Symlinks are rejected.
///
/// # Errors
///
/// Returns a typed error for unsafe paths, missing required signatures,
/// incompatible inputs, verification failures, or filesystem/database errors.
pub fn create_recovery_bundle(
    data_root: &Path,
    bundle_root: &Path,
    installer: &Path,
    signature: Option<&Path>,
    product_version: &str,
    created_unix_ms: i64,
    require_signature: bool,
) -> Result<PathBuf, RecoveryError> {
    validate_version(product_version)?;
    if require_signature && signature.is_none() {
        return Err(RecoveryError::SignatureRequired);
    }
    let data_root = canonical_directory(data_root)?;
    let installer = canonical_file(installer)?;
    let signature = signature.map(canonical_file).transpose()?;
    if !bundle_root.is_absolute() || bundle_root.exists() {
        return Err(RecoveryError::InvalidInput(
            "recovery bundle destination must be a new absolute path",
        ));
    }
    if bundle_root.starts_with(&data_root) {
        return Err(RecoveryError::InvalidInput(
            "recovery bundle must be outside the data root",
        ));
    }

    fs::create_dir_all(bundle_root.join("artifacts"))?;
    fs::create_dir_all(bundle_root.join("data"))?;
    let installer_name = installer
        .file_name()
        .ok_or(RecoveryError::InvalidInput("installer has no file name"))?;
    let installer_destination = bundle_root.join("artifacts").join(installer_name);
    copy_new_file(&installer, &installer_destination)?;
    let installer_record = record(bundle_root, &installer_destination)?;

    let signature_record = if let Some(source) = signature {
        let name = source
            .file_name()
            .ok_or(RecoveryError::InvalidInput("signature has no file name"))?;
        let destination = bundle_root.join("artifacts").join(name);
        copy_new_file(&source, &destination)?;
        Some(record(bundle_root, &destination)?)
    } else {
        None
    };

    let mut sources = Vec::new();
    enumerate_files(&data_root, &data_root, &mut sources)?;
    sources.sort();
    let mut data = Vec::with_capacity(sources.len());
    for source in sources {
        let relative = source
            .strip_prefix(&data_root)
            .map_err(|_| RecoveryError::InvalidInput("data path escaped root"))?;
        let destination = bundle_root.join("data").join(relative);
        if source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sqlite3"))
        {
            sqlite_backup(&source, &destination)?;
        } else {
            copy_new_file(&source, &destination)?;
        }
        data.push(record(&bundle_root.join("data"), &destination)?);
    }

    let manifest = BackupManifest {
        schema_version: RECOVERY_SCHEMA_VERSION,
        product_version: product_version.to_owned(),
        created_unix_ms,
        local_development_unsigned: signature_record.is_none(),
        installer: installer_record,
        signature_record,
        data,
    };
    let manifest_path = bundle_root.join("backup-manifest.json");
    write_json_new(&manifest_path, &manifest)?;
    verify_recovery_bundle(&manifest_path)?;
    Ok(manifest_path)
}

/// Verifies the manifest schema and every recorded artifact/data digest.
///
/// # Errors
///
/// Returns a typed error when the manifest/path is unsafe or any byte differs.
pub fn verify_recovery_bundle(manifest_path: &Path) -> Result<BackupManifest, RecoveryError> {
    let manifest_path = canonical_file(manifest_path)?;
    let root = manifest_path
        .parent()
        .ok_or(RecoveryError::InvalidInput("manifest has no parent"))?;
    let manifest: BackupManifest = read_bounded_json(&manifest_path)?;
    if manifest.schema_version != RECOVERY_SCHEMA_VERSION {
        return Err(RecoveryError::InvalidInput(
            "unsupported recovery manifest schema",
        ));
    }
    validate_version(&manifest.product_version)?;
    verify_record(root, &manifest.installer)?;
    if let Some(signature) = &manifest.signature_record {
        verify_record(root, signature)?;
    } else if !manifest.local_development_unsigned {
        return Err(RecoveryError::SignatureRequired);
    }
    for file in &manifest.data {
        verify_record(&root.join("data"), file)?;
    }
    Ok(manifest)
}

/// Returns the canonical installer path only after the complete bundle verifies.
///
/// # Errors
///
/// Returns a typed error when the bundle or installer cannot be verified.
pub fn verified_installer_path(manifest_path: &Path) -> Result<PathBuf, RecoveryError> {
    let manifest_path = canonical_file(manifest_path)?;
    let manifest = verify_recovery_bundle(&manifest_path)?;
    let root = manifest_path
        .parent()
        .ok_or(RecoveryError::InvalidInput("manifest has no parent"))?;
    canonical_file(&root.join(safe_relative(&manifest.installer.relative_path)?))
}

/// Restores only into a new or empty destination. Existing user data is never
/// overwritten; the caller must make any newer-data decision explicitly.
///
/// # Errors
///
/// Returns a typed error when verification fails, the destination is unsafe or
/// non-empty, or a filesystem operation fails.
pub fn restore_recovery_bundle(
    manifest_path: &Path,
    destination: &Path,
) -> Result<RestoreOutcome, RecoveryError> {
    let manifest = verify_recovery_bundle(manifest_path)?;
    if !destination.is_absolute() {
        return Err(RecoveryError::InvalidInput(
            "restore destination must be absolute",
        ));
    }
    if destination.exists() && fs::read_dir(destination)?.next().is_some() {
        return Err(RecoveryError::DestinationNotEmpty);
    }
    fs::create_dir_all(destination)?;
    let root = manifest_path
        .canonicalize()?
        .parent()
        .ok_or(RecoveryError::InvalidInput("manifest has no parent"))?
        .join("data");
    for file in &manifest.data {
        let relative = safe_relative(&file.relative_path)?;
        let source = root.join(&relative);
        let target = destination.join(relative);
        copy_new_file(&source, &target)?;
    }
    Ok(RestoreOutcome::Restored {
        files: manifest.data.len(),
    })
}

/// Records a future version and its verified, matching pre-update backup.
///
/// # Errors
///
/// Returns a typed error for invalid versions, missing/invalid backup
/// manifests, an existing marker, or filesystem failures.
pub fn mark_update_pending(
    data_root: &Path,
    target_version: &str,
    backup_manifest: &Path,
) -> Result<PathBuf, RecoveryError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| RecoveryError::InvalidInput("system clock is before Unix epoch"))?
        .as_millis();
    mark_update_pending_at(
        data_root,
        target_version,
        backup_manifest,
        i64::try_from(now)
            .map_err(|_| RecoveryError::InvalidInput("system clock cannot be represented"))?,
    )
}

/// Deterministic marker constructor used by recovery tests and update plumbing.
///
/// # Errors
///
/// Returns a typed error for invalid time/version, an invalid backup, an
/// existing marker, or filesystem failures.
pub fn mark_update_pending_at(
    data_root: &Path,
    target_version: &str,
    backup_manifest: &Path,
    marked_unix_ms: i64,
) -> Result<PathBuf, RecoveryError> {
    validate_version(target_version)?;
    if marked_unix_ms < 0 {
        return Err(RecoveryError::InvalidInput(
            "pending marker time is invalid",
        ));
    }
    let manifest = verify_recovery_bundle(backup_manifest)?;
    if manifest.product_version == target_version {
        return Err(RecoveryError::VersionMismatch);
    }
    let recovery = data_root.join("recovery");
    fs::create_dir_all(&recovery)?;
    let marker = recovery.join("pending-update.json");
    if marker.exists() {
        return Err(RecoveryError::InvalidInput("an update is already pending"));
    }
    let value = PendingUpdate {
        schema_version: RECOVERY_SCHEMA_VERSION,
        target_version: target_version.to_owned(),
        backup_manifest: backup_manifest
            .canonicalize()?
            .to_string_lossy()
            .into_owned(),
        marked_unix_ms,
        health_timeout_ms: UPDATE_HEALTH_TIMEOUT_MS,
    };
    write_json_new(&marker, &value)?;
    Ok(marker)
}

/// Reports a deterministic bad-launch health timeout without clearing state or
/// launching or downgrading anything.
///
/// # Errors
///
/// Returns a typed error when a present marker is invalid or unreadable.
pub fn pending_update_status(
    data_root: &Path,
    now_unix_ms: i64,
) -> Result<PendingUpdateStatus, RecoveryError> {
    let marker = data_root.join("recovery/pending-update.json");
    if !marker.exists() {
        return Ok(PendingUpdateStatus::None);
    }
    let pending: PendingUpdate = read_bounded_json(&marker)?;
    if pending.schema_version != RECOVERY_SCHEMA_VERSION
        || pending.marked_unix_ms < 0
        || pending.health_timeout_ms != UPDATE_HEALTH_TIMEOUT_MS
    {
        return Err(RecoveryError::InvalidInput("pending marker is invalid"));
    }
    let deadline = pending
        .marked_unix_ms
        .checked_add(pending.health_timeout_ms)
        .ok_or(RecoveryError::InvalidInput(
            "pending health deadline overflow",
        ))?;
    if now_unix_ms >= deadline {
        Ok(PendingUpdateStatus::HealthTimedOut {
            backup_manifest: PathBuf::from(pending.backup_manifest),
        })
    } else {
        Ok(PendingUpdateStatus::AwaitingHealth {
            deadline_unix_ms: deadline,
        })
    }
}

/// Clears the pending marker only after version, executable/resource,
/// settings, and `SQLite` integrity checks pass.
///
/// # Errors
///
/// Returns a typed error and preserves the marker when any health check fails.
pub fn clear_pending_update_after_health_checks(
    data_root: &Path,
    running_version: &str,
    app_executable: &Path,
    required_resources: &[PathBuf],
) -> Result<HealthCheckOutcome, RecoveryError> {
    let marker = data_root.join("recovery/pending-update.json");
    if !marker.exists() {
        return Ok(HealthCheckOutcome::NoPendingUpdate);
    }
    let pending: PendingUpdate = read_bounded_json(&marker)?;
    if pending.schema_version != RECOVERY_SCHEMA_VERSION
        || pending.target_version != running_version
    {
        return Err(RecoveryError::VersionMismatch);
    }
    canonical_file(app_executable)?;
    for resource in required_resources {
        canonical_file(resource)?;
    }
    let settings = data_root.join("config/settings.json");
    if settings.exists() {
        SettingsStore::new(settings)
            .load()
            .map_err(|_| RecoveryError::VerificationFailed("settings self-check failed".into()))?;
    }
    let database = data_root.join("history.sqlite3");
    if database.exists() {
        check_sqlite(&database)?;
    }
    verify_recovery_bundle(Path::new(&pending.backup_manifest))?;
    fs::remove_file(marker)?;
    Ok(HealthCheckOutcome::Cleared)
}

fn check_sqlite(path: &Path) -> Result<(), RecoveryError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(RecoveryError::VerificationFailed(
            "SQLite integrity check failed".into(),
        ));
    }
    let has_meta: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
        [],
        |row| row.get(0),
    )?;
    if has_meta != 0 {
        let version: i64 =
            connection.query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
                row.get(0)
            })?;
        if version < 1 || version > i64::from(DATABASE_SCHEMA_VERSION) {
            return Err(RecoveryError::VerificationFailed(
                "SQLite schema is incompatible".into(),
            ));
        }
    }
    Ok(())
}

fn sqlite_backup(source: &Path, destination: &Path) -> Result<(), RecoveryError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if destination.exists() {
        return Err(RecoveryError::InvalidInput("backup destination exists"));
    }
    let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.backup("main", destination, None)?;
    OpenOptions::new()
        .read(true)
        .open(destination)?
        .sync_all()?;
    Ok(())
}

fn enumerate_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), RecoveryError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(RecoveryError::InvalidInput(
                "symlinks are not recoverable data",
            ));
        }
        let path = entry.path();
        if file_type.is_dir() {
            enumerate_files(root, &path, output)?;
        } else if file_type.is_file() {
            path.strip_prefix(root)
                .map_err(|_| RecoveryError::InvalidInput("data path escaped root"))?;
            output.push(path);
        }
    }
    Ok(())
}

fn verify_record(root: &Path, expected: &FileRecord) -> Result<(), RecoveryError> {
    let path = root.join(safe_relative(&expected.relative_path)?);
    let actual = record(root, &path)?;
    if actual != *expected {
        return Err(RecoveryError::VerificationFailed(
            expected.relative_path.clone(),
        ));
    }
    Ok(())
}

fn record(root: &Path, path: &Path) -> Result<FileRecord, RecoveryError> {
    let canonical_root = root.canonicalize()?;
    let canonical_path = canonical_file(path)?;
    let relative = canonical_path
        .strip_prefix(&canonical_root)
        .map_err(|_| RecoveryError::InvalidInput("record escaped recovery root"))?;
    let mut file = File::open(&canonical_path)?;
    let bytes = file.metadata()?.len();
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)?;
    Ok(FileRecord {
        relative_path: relative.to_string_lossy().replace('\\', "/"),
        bytes,
        sha256: format!("{:x}", digest.finalize()),
    })
}

fn safe_relative(value: &str) -> Result<PathBuf, RecoveryError> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(RecoveryError::InvalidInput("manifest path is unsafe"));
    }
    Ok(path)
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<(), RecoveryError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<(), RecoveryError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_bounded_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, RecoveryError> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_MANIFEST_BYTES {
        return Err(RecoveryError::InvalidInput("manifest is too large"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(RecoveryError::Json)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, RecoveryError> {
    if !path.is_absolute() {
        return Err(RecoveryError::InvalidInput("path must be absolute"));
    }
    let path = path.canonicalize()?;
    if !path.is_dir() {
        return Err(RecoveryError::InvalidInput("directory is unavailable"));
    }
    Ok(path)
}

fn canonical_file(path: &Path) -> Result<PathBuf, RecoveryError> {
    if !path.is_absolute() {
        return Err(RecoveryError::InvalidInput("path must be absolute"));
    }
    let path = path.canonicalize()?;
    if !path.is_file() {
        return Err(RecoveryError::InvalidInput("file is unavailable"));
    }
    Ok(path)
}

fn validate_version(version: &str) -> Result<(), RecoveryError> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(RecoveryError::InvalidInput("product version is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn unsigned_local_bundle_verifies_and_restore_refuses_existing_data() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let installer = temp.path().join("SpeakEasy setup.exe");
        write(&data.join("config/settings-note.txt"), b"preserve me");
        write(&installer, b"local development installer");
        let bundle = temp.path().join("recovery bundle");
        let manifest =
            create_recovery_bundle(&data, &bundle, &installer, None, "1.0.0", 7, false).unwrap();
        let verified = verify_recovery_bundle(&manifest).unwrap();
        assert!(verified.local_development_unsigned);
        assert_eq!(verified.data.len(), 1);

        let restore = temp.path().join("restored Unicode \u{6570}\u{636e}");
        assert_eq!(
            restore_recovery_bundle(&manifest, &restore).unwrap(),
            RestoreOutcome::Restored { files: 1 }
        );
        assert_eq!(
            fs::read(restore.join("config/settings-note.txt")).unwrap(),
            b"preserve me"
        );
        assert!(matches!(
            restore_recovery_bundle(&manifest, &restore),
            Err(RecoveryError::DestinationNotEmpty)
        ));
    }

    #[test]
    fn tampering_and_missing_required_signature_fail_closed() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let installer = temp.path().join("installer.exe");
        write(&data.join("models/model.bin"), b"model");
        write(&installer, b"installer");
        assert!(matches!(
            create_recovery_bundle(
                &data,
                &temp.path().join("signed"),
                &installer,
                None,
                "1.0.0",
                0,
                true
            ),
            Err(RecoveryError::SignatureRequired)
        ));
        let bundle = temp.path().join("unsigned");
        let manifest =
            create_recovery_bundle(&data, &bundle, &installer, None, "1.0.0", 0, false).unwrap();
        fs::write(bundle.join("data/models/model.bin"), b"malicious").unwrap();
        assert!(matches!(
            verify_recovery_bundle(&manifest),
            Err(RecoveryError::VerificationFailed(_))
        ));
    }

    #[test]
    fn signature_record_is_an_integrity_record_and_reads_legacy_manifests() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let installer = temp.path().join("installer.exe");
        let signature = temp.path().join("installer.p7s");
        write(&data.join("settings.json"), b"settings");
        write(&installer, b"installer");
        write(&signature, b"detached signature bytes");
        let bundle = temp.path().join("signed");
        let manifest = create_recovery_bundle(
            &data,
            &bundle,
            &installer,
            Some(&signature),
            "1.0.0",
            0,
            true,
        )
        .unwrap();

        let json = fs::read_to_string(&manifest).unwrap();
        assert!(json.contains("\"signature_record\""));
        assert!(!json.contains("\"signature\":"));
        assert!(
            verify_recovery_bundle(&manifest)
                .unwrap()
                .signature_record
                .is_some()
        );

        let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
        let record = legacy
            .as_object_mut()
            .unwrap()
            .remove("signature_record")
            .unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .insert("signature".to_owned(), record);
        fs::write(&manifest, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        assert!(
            verify_recovery_bundle(&manifest)
                .unwrap()
                .signature_record
                .is_some()
        );
    }

    #[test]
    fn pending_marker_survives_failed_health_and_clears_only_after_pass() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let installer = temp.path().join("installer.exe");
        let app = temp.path().join("speakeasy.exe");
        write(&data.join("placeholder"), b"x");
        write(&installer, b"old");
        write(&app, b"new");
        let bundle = temp.path().join("bundle");
        let manifest =
            create_recovery_bundle(&data, &bundle, &installer, None, "0.9.0", 0, false).unwrap();
        let marker = mark_update_pending_at(&data, "1.0.0", &manifest, 10).unwrap();
        assert_eq!(
            pending_update_status(&data, 120_009).unwrap(),
            PendingUpdateStatus::AwaitingHealth {
                deadline_unix_ms: 120_010
            }
        );
        assert!(matches!(
            pending_update_status(&data, 120_010).unwrap(),
            PendingUpdateStatus::HealthTimedOut { .. }
        ));
        assert!(clear_pending_update_after_health_checks(&data, "0.9.0", &app, &[]).is_err());
        assert!(marker.exists());
        assert_eq!(
            clear_pending_update_after_health_checks(&data, "1.0.0", &app, &[]).unwrap(),
            HealthCheckOutcome::Cleared
        );
        assert!(!marker.exists());
    }
}
