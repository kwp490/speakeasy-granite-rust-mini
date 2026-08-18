use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct ExplicitImportRoot(PathBuf);

#[derive(Clone, Debug)]
pub struct ProductionImportRoot(PathBuf);

impl ProductionImportRoot {
    /// Resolves the one exact v1 `ProgramData` source. It never creates or writes it.
    ///
    /// # Errors
    ///
    /// Returns an invalid-root error when `ProgramData` is unavailable or the
    /// accepted path exists but is not a directory.
    pub fn detect() -> Result<Option<Self>, ImportError> {
        let program_data = std::env::var_os("ProgramData").ok_or(ImportError::InvalidRoot)?;
        let root = PathBuf::from(program_data).join("SpeakEasy AI Granite");
        if !root.exists() {
            return Ok(None);
        }
        if !root.is_dir() {
            return Err(ImportError::InvalidRoot);
        }
        Ok(Some(Self(root)))
    }
}

#[cfg(any(test, feature = "test-import"))]
impl ExplicitImportRoot {
    /// Creates a root only in tests or builds explicitly enabling fixture imports.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::InvalidRoot`] when the supplied path is not a directory.
    pub fn for_test(path: impl Into<PathBuf>) -> Result<Self, ImportError> {
        let path = path.into();
        if !path.is_dir() {
            return Err(ImportError::InvalidRoot);
        }
        Ok(Self(path))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportWarning {
    CorruptSettings,
    CorruptPreset(String),
    RunningV1,
    SharedProgramData,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionPolicy {
    KeepV2,
    ReplaceFromV1,
    RenameV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImportChoices {
    pub settings: bool,
    pub presets: bool,
    pub credentials_presence_only: bool,
    pub collision_policy: CollisionPolicy,
}

impl Default for ImportChoices {
    fn default() -> Self {
        Self {
            settings: true,
            presets: true,
            credentials_presence_only: true,
            collision_policy: CollisionPolicy::KeepV2,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImportPreview {
    pub nonce: String,
    pub source_fingerprint: String,
    pub settings_available: bool,
    pub preset_names: Vec<String>,
    pub warnings: Vec<String>,
    pub running_v1: bool,
}

#[derive(Clone, Debug)]
pub struct ImportPlan {
    source_root: ExplicitImportRoot,
    pub source_fingerprint: String,
    pub settings: Option<Value>,
    pub presets: Vec<(String, Value)>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImportReport {
    pub source_fingerprint: String,
    pub settings_written: bool,
    pub presets_written: usize,
    #[serde(default)]
    pub importer_version: u16,
    #[serde(default)]
    pub choices: Option<ImportChoices>,
    #[serde(default)]
    pub collisions_resolved: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub before_fingerprint: String,
    #[serde(default)]
    pub after_fingerprint: String,
}

#[derive(Debug)]
pub enum ImportError {
    Io(io::Error),
    InvalidRoot,
    SourceChanged,
    DestinationExists,
    InvalidNonce,
    ActivationFailed,
}

impl From<io::Error> for ImportError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl ImportPlan {
    /// Parses settings and presets and records an immutable source fingerprint.
    ///
    /// # Errors
    ///
    /// Returns an I/O or invalid-root error when the explicit fixture root cannot be read.
    pub fn inspect(source_root: ExplicitImportRoot) -> Result<Self, ImportError> {
        let source_fingerprint = fingerprint(&source_root.0)?;
        let settings_path = source_root.0.join("config/settings.json");
        let mut warnings = Vec::new();
        let settings = parse_optional_json(&settings_path, &mut warnings, None)?;
        let mut presets = Vec::new();
        let presets_root = source_root.0.join("config/presets");
        if presets_root.is_dir() {
            let mut paths = fs::read_dir(presets_root)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                })
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(ImportError::InvalidRoot)?
                    .to_owned();
                if let Some(value) = parse_optional_json(&path, &mut warnings, Some(&name))? {
                    presets.push((name, value));
                }
            }
        }
        Ok(Self {
            source_root,
            source_fingerprint,
            settings,
            presets,
            warnings,
        })
    }

    /// Commits a previously inspected plan only if its source remains unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error for source changes, existing destinations, invalid data, or I/O failures.
    pub fn commit(&self, destination: &Path) -> Result<ImportReport, ImportError> {
        if fingerprint(&self.source_root.0)? != self.source_fingerprint {
            return Err(ImportError::SourceChanged);
        }
        if destination.exists() {
            let report_path = destination.join(".speakeasy-import.json");
            let report_bytes = fs::read(report_path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    ImportError::DestinationExists
                } else {
                    ImportError::Io(error)
                }
            })?;
            let report: ImportReport = serde_json::from_slice(&report_bytes)
                .map_err(|_| ImportError::DestinationExists)?;
            if report.source_fingerprint == self.source_fingerprint {
                return Ok(report);
            }
            return Err(ImportError::DestinationExists);
        }
        let staging = destination.with_extension("import-staging");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(staging.join("presets"))?;
        if let Some(settings) = &self.settings {
            write_synced(
                &staging.join("settings.json"),
                &serde_json::to_vec_pretty(settings).map_err(|_| ImportError::InvalidRoot)?,
            )?;
        }
        for (name, preset) in &self.presets {
            write_synced(
                &staging.join("presets").join(name),
                &serde_json::to_vec_pretty(preset).map_err(|_| ImportError::InvalidRoot)?,
            )?;
        }
        let report = ImportReport {
            source_fingerprint: self.source_fingerprint.clone(),
            settings_written: self.settings.is_some(),
            presets_written: self.presets.len(),
            importer_version: 1,
            choices: None,
            collisions_resolved: Vec::new(),
            warnings: warning_codes(&self.warnings),
            before_fingerprint: self.source_fingerprint.clone(),
            after_fingerprint: self.source_fingerprint.clone(),
        };
        write_synced(
            &staging.join(".speakeasy-import.json"),
            &serde_json::to_vec_pretty(&report).map_err(|_| ImportError::InvalidRoot)?,
        )?;
        fs::rename(staging, destination)?;
        Ok(report)
    }
}

#[derive(Debug)]
pub struct ProductionImportPlan {
    source_root: ProductionImportRoot,
    snapshot_root: PathBuf,
    plan: ImportPlan,
    preview: ImportPreview,
}

impl ProductionImportPlan {
    /// Creates a nonce-bound preview from a private copy of the known v1 surface.
    ///
    /// # Errors
    ///
    /// Returns an I/O, source-change, or invalid-root error.
    pub fn inspect(
        source_root: ProductionImportRoot,
        running_v1: bool,
    ) -> Result<Self, ImportError> {
        let before = fingerprint_import_surface(&source_root.0)?;
        let snapshot_root =
            std::env::temp_dir().join(format!("speakeasy-v1-preview-{}", preview_nonce(&before)));
        fs::create_dir_all(snapshot_root.join("config/presets"))?;
        copy_optional_file(
            &source_root.0.join("config/settings.json"),
            &snapshot_root.join("config/settings.json"),
        )?;
        let presets = source_root.0.join("config/presets");
        if presets.is_dir() {
            for entry in fs::read_dir(&presets)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "json")
                {
                    fs::copy(
                        &path,
                        snapshot_root.join("config/presets").join(entry.file_name()),
                    )?;
                }
            }
        }
        let after = fingerprint_import_surface(&source_root.0)?;
        if before != after {
            fs::remove_dir_all(&snapshot_root)?;
            return Err(ImportError::SourceChanged);
        }
        let explicit = ExplicitImportRoot(snapshot_root.clone());
        let mut plan = ImportPlan::inspect(explicit)?;
        plan.source_fingerprint = before;
        plan.warnings.push(ImportWarning::SharedProgramData);
        if running_v1 {
            plan.warnings.push(ImportWarning::RunningV1);
        }
        let preview = ImportPreview {
            nonce: preview_nonce(&plan.source_fingerprint),
            source_fingerprint: plan.source_fingerprint.clone(),
            settings_available: plan.settings.is_some(),
            preset_names: plan.presets.iter().map(|(name, _)| name.clone()).collect(),
            warnings: warning_codes(&plan.warnings),
            running_v1,
        };
        Ok(Self {
            source_root,
            snapshot_root,
            plan,
            preview,
        })
    }

    #[must_use]
    pub fn preview(&self) -> &ImportPreview {
        &self.preview
    }

    /// Activates a complete v2-owned staged result and restores the prior v2
    /// directory if activation fails. The v1 source is only re-read for hashes.
    ///
    /// # Errors
    ///
    /// Returns an error for nonce/fingerprint mismatch, invalid data, collision,
    /// staging, activation, rollback, or other I/O failure.
    #[allow(clippy::too_many_lines)]
    pub fn commit(
        &self,
        destination: &Path,
        nonce: &str,
        choices: &ImportChoices,
    ) -> Result<ImportReport, ImportError> {
        if nonce != self.preview.nonce {
            return Err(ImportError::InvalidNonce);
        }
        let stage = destination.with_extension("migration-stage");
        let rollback = destination.with_extension("migration-rollback");
        if !destination.exists() && rollback.exists() {
            fs::rename(&rollback, destination)?;
        } else if destination.exists() && rollback.exists() {
            fs::remove_dir_all(&rollback)?;
        }
        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        let before = fingerprint_import_surface(&self.source_root.0)?;
        if before != self.plan.source_fingerprint {
            return Err(ImportError::SourceChanged);
        }
        let marker = destination.join(".speakeasy-import.json");
        if let Ok(bytes) = fs::read(&marker)
            && let Ok(report) = serde_json::from_slice::<ImportReport>(&bytes)
            && report.source_fingerprint == before
            && report.choices.as_ref() == Some(choices)
        {
            return Ok(report);
        }

        fs::create_dir_all(stage.join("config/presets"))?;
        if destination.exists() {
            copy_tree(destination, &stage)?;
        }

        let mut collisions = Vec::new();
        if choices.settings
            && let Some(settings) = &self.plan.settings
        {
            let target = stage.join("config/settings.json");
            if !target.exists() || choices.collision_policy == CollisionPolicy::ReplaceFromV1 {
                write_synced(
                    &target,
                    &serde_json::to_vec_pretty(settings).map_err(|_| ImportError::InvalidRoot)?,
                )?;
            } else {
                collisions.push("settings:kept_v2".to_owned());
            }
        }
        let mut presets_written = 0;
        if choices.presets {
            for (name, preset) in &self.plan.presets {
                let mut target = stage.join("config/presets").join(name);
                if target.exists() {
                    match choices.collision_policy {
                        CollisionPolicy::KeepV2 => {
                            collisions.push(format!("{name}:kept_v2"));
                            continue;
                        }
                        CollisionPolicy::ReplaceFromV1 => {
                            collisions.push(format!("{name}:replaced_from_v1"));
                        }
                        CollisionPolicy::RenameV1 => {
                            let stem = target
                                .file_stem()
                                .and_then(|value| value.to_str())
                                .ok_or(ImportError::InvalidRoot)?;
                            target = target.with_file_name(format!("{stem} (v1).json"));
                            collisions.push(format!("{name}:renamed_v1"));
                        }
                    }
                }
                write_synced(
                    &target,
                    &serde_json::to_vec_pretty(preset).map_err(|_| ImportError::InvalidRoot)?,
                )?;
                presets_written += 1;
            }
        }
        let after = fingerprint_import_surface(&self.source_root.0)?;
        if after != before {
            fs::remove_dir_all(stage)?;
            return Err(ImportError::SourceChanged);
        }
        let report = ImportReport {
            source_fingerprint: before.clone(),
            settings_written: choices.settings && self.plan.settings.is_some(),
            presets_written,
            importer_version: 1,
            choices: Some(choices.clone()),
            collisions_resolved: collisions,
            warnings: warning_codes(&self.plan.warnings),
            before_fingerprint: before,
            after_fingerprint: after,
        };
        write_synced(
            &stage.join(".speakeasy-migration-journal.json"),
            br#"{"schema_version":1,"state":"staged"}"#,
        )?;
        write_synced(
            &stage.join(".speakeasy-import.json"),
            &serde_json::to_vec_pretty(&report).map_err(|_| ImportError::InvalidRoot)?,
        )?;

        if destination.exists() {
            fs::rename(destination, &rollback)?;
        }
        if let Err(error) = fs::rename(&stage, destination) {
            if rollback.exists() {
                let _ = fs::rename(&rollback, destination);
            }
            return Err(ImportError::Io(error));
        }
        if rollback.exists() {
            fs::remove_dir_all(rollback)?;
        }
        let journal = destination.join(".speakeasy-migration-journal.json");
        if journal.exists() {
            fs::remove_file(journal)?;
        }
        Ok(report)
    }
}

impl Drop for ProductionImportPlan {
    fn drop(&mut self) {
        if self.snapshot_root.is_dir() {
            let _ = fs::remove_dir_all(&self.snapshot_root);
        }
    }
}

static NONCE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn preview_nonce(fingerprint: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(fingerprint.as_bytes());
    digest.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    digest.update(NONCE_SEQUENCE.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    format!("{:x}", digest.finalize())
}

fn warning_codes(warnings: &[ImportWarning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| match warning {
            ImportWarning::CorruptSettings => "corrupt_settings".to_owned(),
            ImportWarning::CorruptPreset(name) => format!("corrupt_preset:{name}"),
            ImportWarning::RunningV1 => "v1_running_source_may_change".to_owned(),
            ImportWarning::SharedProgramData => "shared_programdata_user_ambiguity".to_owned(),
        })
        .collect()
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), ImportError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&target_path)?;
            copy_tree(&source_path, &target_path)?;
        } else if source_path.is_file() {
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

fn copy_optional_file(source: &Path, destination: &Path) -> Result<(), ImportError> {
    if source.is_file() {
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn parse_optional_json(
    path: &Path,
    warnings: &mut Vec<ImportWarning>,
    preset_name: Option<&str>,
) -> Result<Option<Value>, ImportError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    if let Ok(value) = serde_json::from_slice(bytes) {
        Ok(Some(value))
    } else {
        warnings.push(preset_name.map_or(ImportWarning::CorruptSettings, |name| {
            ImportWarning::CorruptPreset(name.to_owned())
        }));
        Ok(None)
    }
}

fn fingerprint(root: &Path) -> Result<String, ImportError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path) in files {
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(fs::read(path)?);
        digest.update([0]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn fingerprint_import_surface(root: &Path) -> Result<String, ImportError> {
    let mut digest = Sha256::new();
    let settings = root.join("config/settings.json");
    if settings.is_file() {
        digest.update(b"config/settings.json\0");
        digest.update(fs::read(settings)?);
        digest.update([0]);
    }
    let presets = root.join("config/presets");
    if presets.is_dir() {
        let mut paths = fs::read_dir(presets)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|value| value == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(ImportError::InvalidRoot)?;
            digest.update(b"config/presets/");
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(fs::read(path)?);
            digest.update([0]);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ImportError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ImportError::InvalidRoot)?
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "speakeasy-import-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn importer_handles_bom_corruption_and_source_change() {
        let source = temp_root("source");
        fs::create_dir_all(source.join("config/presets")).expect("source");
        fs::write(
            source.join("config/settings.json"),
            b"\xef\xbb\xbf{\"language\":\"en\"}",
        )
        .expect("settings");
        fs::write(source.join("config/presets/broken.json"), b"{").expect("preset");
        let root = ExplicitImportRoot::for_test(&source).expect("test root");
        let plan = ImportPlan::inspect(root).expect("inspect");
        assert!(plan.settings.is_some());
        assert_eq!(plan.warnings.len(), 1);

        fs::write(source.join("config/settings.json"), b"{}").expect("mutate");
        assert!(matches!(
            plan.commit(&temp_root("destination")),
            Err(ImportError::SourceChanged)
        ));
        fs::remove_dir_all(source).expect("cleanup");
    }

    #[test]
    fn exact_repeat_is_idempotent_and_different_source_collides() {
        let source = temp_root("idempotent-source");
        let destination = temp_root("idempotent-destination");
        fs::create_dir_all(source.join("config/presets")).expect("source");
        fs::write(
            source.join("config/settings.json"),
            b"{\"language\":\"en\"}",
        )
        .expect("settings");
        fs::write(
            source.join("config/presets/one.json"),
            b"{\"name\":\"one\"}",
        )
        .expect("preset");
        let plan = ImportPlan::inspect(ExplicitImportRoot::for_test(&source).unwrap()).unwrap();
        let first = plan.commit(&destination).expect("first commit");
        let second = plan.commit(&destination).expect("idempotent commit");
        assert_eq!(first, second);

        let other = temp_root("other-source");
        fs::create_dir_all(other.join("config")).expect("other");
        fs::write(other.join("config/settings.json"), b"{}").expect("other settings");
        let other_plan =
            ImportPlan::inspect(ExplicitImportRoot::for_test(&other).unwrap()).unwrap();
        assert!(matches!(
            other_plan.commit(&destination),
            Err(ImportError::DestinationExists)
        ));
        fs::remove_dir_all(source).expect("cleanup source");
        fs::remove_dir_all(other).expect("cleanup other");
        fs::remove_dir_all(destination).expect("cleanup destination");
    }

    #[test]
    fn commit_preserves_source_and_refuses_unowned_destination() {
        let source = temp_root("preserved-source");
        let destination = temp_root("preserved-destination");
        fs::create_dir_all(source.join("config/presets")).expect("source");
        let settings = source.join("config/settings.json");
        let preset = source.join("config/presets/one.json");
        fs::write(&settings, b"{\"language\":\"en\"}").expect("settings");
        fs::write(&preset, b"{\"name\":\"one\"}").expect("preset");
        let before = fingerprint(&source).expect("source fingerprint");
        let plan = ImportPlan::inspect(ExplicitImportRoot::for_test(&source).unwrap()).unwrap();
        plan.commit(&destination).expect("commit");
        assert_eq!(fingerprint(&source).expect("source after"), before);
        assert_eq!(fs::read(settings).unwrap(), b"{\"language\":\"en\"}");
        assert_eq!(fs::read(preset).unwrap(), b"{\"name\":\"one\"}");

        let occupied = temp_root("occupied-destination");
        fs::create_dir_all(&occupied).expect("occupied destination");
        fs::write(occupied.join("owner-file"), b"do not replace").expect("owner file");
        assert!(matches!(
            plan.commit(&occupied),
            Err(ImportError::DestinationExists)
        ));
        assert_eq!(
            fs::read(occupied.join("owner-file")).unwrap(),
            b"do not replace"
        );

        fs::remove_dir_all(source).expect("cleanup source");
        fs::remove_dir_all(destination).expect("cleanup destination");
        fs::remove_dir_all(occupied).expect("cleanup occupied");
    }

    #[test]
    fn missing_explicit_source_is_rejected() {
        assert!(matches!(
            ExplicitImportRoot::for_test(temp_root("missing-source")),
            Err(ImportError::InvalidRoot)
        ));
    }

    #[test]
    fn production_preview_requires_nonce_and_resolves_collisions_idempotently() {
        let source = temp_root("production-source");
        let destination = temp_root("production-destination");
        fs::create_dir_all(source.join("config/presets")).unwrap();
        fs::write(source.join("config/settings.json"), br#"{"language":"en"}"#).unwrap();
        fs::write(
            source.join("config/presets/shared.json"),
            br#"{"name":"Imported"}"#,
        )
        .unwrap();
        fs::create_dir_all(destination.join("config/presets")).unwrap();
        fs::write(
            destination.join("config/settings.json"),
            br#"{"locale":"en-US"}"#,
        )
        .unwrap();
        fs::write(
            destination.join("config/presets/shared.json"),
            br#"{"name":"Existing"}"#,
        )
        .unwrap();

        let plan =
            ProductionImportPlan::inspect(ProductionImportRoot(source.clone()), true).unwrap();
        assert!(plan.preview().running_v1);
        assert!(
            plan.preview()
                .warnings
                .contains(&"v1_running_source_may_change".to_owned())
        );
        let choices = ImportChoices {
            collision_policy: CollisionPolicy::RenameV1,
            ..ImportChoices::default()
        };
        assert!(matches!(
            plan.commit(&destination, "wrong", &choices),
            Err(ImportError::InvalidNonce)
        ));
        let first = plan
            .commit(&destination, &plan.preview().nonce, &choices)
            .unwrap();
        let second = plan
            .commit(&destination, &plan.preview().nonce, &choices)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.before_fingerprint, first.after_fingerprint);
        assert!(destination.join("config/presets/shared (v1).json").exists());
        assert_eq!(
            fs::read(destination.join("config/settings.json")).unwrap(),
            br#"{"locale":"en-US"}"#
        );
        assert!(
            !destination
                .join(".speakeasy-migration-journal.json")
                .exists()
        );
        assert!(source.join("config/settings.json").exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn production_commit_refuses_source_mutation_and_preserves_v2() {
        let source = temp_root("mutation-source");
        let destination = temp_root("mutation-destination");
        fs::create_dir_all(source.join("config")).unwrap();
        fs::write(source.join("config/settings.json"), b"{}").unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("owner"), b"preserve").unwrap();
        let plan =
            ProductionImportPlan::inspect(ProductionImportRoot(source.clone()), false).unwrap();
        fs::write(source.join("config/settings.json"), b"{\"changed\":true}").unwrap();
        assert!(matches!(
            plan.commit(
                &destination,
                &plan.preview().nonce,
                &ImportChoices::default()
            ),
            Err(ImportError::SourceChanged)
        ));
        assert_eq!(fs::read(destination.join("owner")).unwrap(), b"preserve");
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn production_commit_recovers_interrupted_activation_before_retry() {
        let source = temp_root("crash-source");
        let destination = temp_root("crash-destination");
        fs::create_dir_all(source.join("config")).unwrap();
        fs::write(source.join("config/settings.json"), br#"{"language":"en"}"#).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("owner"), b"preserve").unwrap();
        let rollback = destination.with_extension("migration-rollback");
        let stage = destination.with_extension("migration-stage");
        fs::rename(&destination, &rollback).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("partial"), b"incomplete").unwrap();

        let plan =
            ProductionImportPlan::inspect(ProductionImportRoot(source.clone()), false).unwrap();
        let report = plan
            .commit(
                &destination,
                &plan.preview().nonce,
                &ImportChoices::default(),
            )
            .unwrap();
        assert!(report.settings_written);
        assert_eq!(fs::read(destination.join("owner")).unwrap(), b"preserve");
        assert!(!rollback.exists());
        assert!(!stage.exists());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(destination).unwrap();
    }
}
