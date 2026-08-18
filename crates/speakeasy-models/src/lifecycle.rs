use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallState {
    Absent,
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
    },
    Verifying,
    Installing,
    VerifiedOnDisk,
    Cancelled,
    Failed(InstallFailure),
}

impl InstallState {
    /// Applies one transition from the install-state table.
    ///
    /// # Errors
    ///
    /// Returns an error when the transition is not permitted.
    pub fn transition(self, next: Self) -> Result<Self, &'static str> {
        let allowed = matches!(
            (&self, &next),
            (Self::Absent, Self::Downloading { .. })
                | (
                    Self::Downloading { .. },
                    Self::Downloading { .. } | Self::Verifying | Self::Cancelled | Self::Failed(_)
                )
                | (
                    Self::Verifying,
                    Self::Installing | Self::Cancelled | Self::Failed(_)
                )
                | (
                    Self::Installing,
                    Self::VerifiedOnDisk | Self::Cancelled | Self::Failed(_)
                )
                | (
                    Self::Cancelled | Self::Failed(_),
                    Self::Absent | Self::Downloading { .. }
                )
                | (Self::VerifiedOnDisk, Self::Absent)
        );
        if allowed {
            Ok(next)
        } else {
            Err("invalid install-state transition")
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallFailure {
    pub reason: String,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeState {
    VerifiedOnDisk,
    RuntimeSmokeTesting,
    Ready(RuntimeEvidence),
    Failed(InstallFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeEvidence {
    pub artifact_id: String,
    pub runtime_abi: String,
    pub provider: String,
    pub inference_sample_count: usize,
}

impl RuntimeState {
    /// Applies one transition from the runtime-admission table.
    ///
    /// # Errors
    ///
    /// Returns an error when runtime smoke is bypassed or a transition is not
    /// permitted from the current state.
    pub fn transition(self, next: Self) -> Result<Self, &'static str> {
        let allowed = matches!(
            (&self, &next),
            (
                Self::VerifiedOnDisk | Self::Failed(_),
                Self::RuntimeSmokeTesting
            ) | (Self::RuntimeSmokeTesting, Self::Ready(_) | Self::Failed(_))
                | (Self::Ready(_), Self::RuntimeSmokeTesting | Self::Failed(_))
        );
        if allowed {
            Ok(next)
        } else {
            Err("invalid runtime-state transition")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveEntryKind {
    File,
    Directory,
    Symlink,
    HardLink,
    ReparsePoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    pub path: PathBuf,
    pub kind: ArchiveEntryKind,
    pub compressed_bytes: u64,
    pub extracted_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveLimits {
    pub maximum_files: usize,
    pub maximum_extracted_bytes: u64,
    pub maximum_compression_ratio: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedArchivePlan {
    entries: Vec<ArchiveEntry>,
    extracted_bytes: u64,
}

impl ValidatedArchivePlan {
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    pub const fn extracted_bytes(&self) -> u64 {
        self.extracted_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveValidationError {
    InvalidPath {
        path: PathBuf,
        reason: &'static str,
    },
    UnsupportedEntry {
        path: PathBuf,
        kind: ArchiveEntryKind,
    },
    UnexpectedExecutable {
        path: PathBuf,
    },
    DestinationCollision {
        path: PathBuf,
    },
    TooManyFiles {
        actual: usize,
        maximum: usize,
    },
    ExtractedSizeExceeded {
        actual: u64,
        maximum: u64,
    },
    CompressionRatioExceeded {
        path: PathBuf,
        actual: u64,
        maximum: u64,
    },
    SizeOverflow,
}

impl Display for ArchiveValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "archive validation failed: {self:?}")
    }
}

impl Error for ArchiveValidationError {}

/// Validates the complete archive directory before extraction begins.
///
/// # Errors
///
/// Returns the first unsafe entry, collision, or configured-limit violation.
pub fn validate_archive_plan<S: std::hash::BuildHasher>(
    entries: impl IntoIterator<Item = ArchiveEntry>,
    allowed_files: &HashSet<PathBuf, S>,
    limits: ArchiveLimits,
) -> Result<ValidatedArchivePlan, ArchiveValidationError> {
    let mut validated = Vec::new();
    let mut destination_keys = HashSet::new();
    let mut extracted_bytes = 0_u64;
    let mut file_count = 0_usize;

    for entry in entries {
        let key = windows_destination_key(&entry.path)?;
        if !destination_keys.insert(key) {
            return Err(ArchiveValidationError::DestinationCollision { path: entry.path });
        }
        match entry.kind {
            ArchiveEntryKind::File => {
                file_count = file_count.saturating_add(1);
                if file_count > limits.maximum_files {
                    return Err(ArchiveValidationError::TooManyFiles {
                        actual: file_count,
                        maximum: limits.maximum_files,
                    });
                }
                if !allowed_files.contains(&entry.path) && is_executable_path(&entry.path) {
                    return Err(ArchiveValidationError::UnexpectedExecutable { path: entry.path });
                }
                if entry.compressed_bytes == 0 && entry.extracted_bytes > 0 {
                    return Err(ArchiveValidationError::CompressionRatioExceeded {
                        path: entry.path,
                        actual: u64::MAX,
                        maximum: limits.maximum_compression_ratio,
                    });
                }
                let ratio = entry.extracted_bytes / entry.compressed_bytes.max(1);
                if ratio > limits.maximum_compression_ratio {
                    return Err(ArchiveValidationError::CompressionRatioExceeded {
                        path: entry.path,
                        actual: ratio,
                        maximum: limits.maximum_compression_ratio,
                    });
                }
                extracted_bytes = extracted_bytes
                    .checked_add(entry.extracted_bytes)
                    .ok_or(ArchiveValidationError::SizeOverflow)?;
                if extracted_bytes > limits.maximum_extracted_bytes {
                    return Err(ArchiveValidationError::ExtractedSizeExceeded {
                        actual: extracted_bytes,
                        maximum: limits.maximum_extracted_bytes,
                    });
                }
            }
            ArchiveEntryKind::Directory => {}
            ArchiveEntryKind::Symlink
            | ArchiveEntryKind::HardLink
            | ArchiveEntryKind::ReparsePoint => {
                return Err(ArchiveValidationError::UnsupportedEntry {
                    path: entry.path,
                    kind: entry.kind,
                });
            }
        }
        validated.push(entry);
    }

    Ok(ValidatedArchivePlan {
        entries: validated,
        extracted_bytes,
    })
}

fn windows_destination_key(path: &Path) -> Result<String, ArchiveValidationError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid_path(path, "path must be non-empty and relative"));
    }
    let mut key = String::new();
    for component in path.components() {
        let Component::Normal(raw_component) = component else {
            return Err(invalid_path(path, "path traversal or prefix is forbidden"));
        };
        let Some(component) = raw_component.to_str() else {
            return Err(invalid_path(path, "path must be valid Unicode"));
        };
        if component.ends_with(['.', ' ']) {
            return Err(invalid_path(path, "trailing dot or space is forbidden"));
        }
        if component.contains(':') {
            return Err(invalid_path(path, "alternate data streams are forbidden"));
        }
        let stem = component.split('.').next().unwrap_or_default();
        if is_reserved_windows_name(stem) {
            return Err(invalid_path(
                path,
                "reserved Windows device name is forbidden",
            ));
        }
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(&component.nfkc().collect::<String>().to_lowercase());
    }
    Ok(key)
}

fn invalid_path(path: &Path, reason: &'static str) -> ArchiveValidationError {
    ArchiveValidationError::InvalidPath {
        path: path.to_path_buf(),
        reason,
    }
}

fn is_reserved_windows_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || upper
            .strip_prefix("COM")
            .is_some_and(is_reserved_device_number)
        || upper
            .strip_prefix("LPT")
            .is_some_and(is_reserved_device_number)
}

fn is_reserved_device_number(suffix: &str) -> bool {
    matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

fn is_executable_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "dll" | "com" | "bat" | "cmd" | "ps1" | "msi" | "scr"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> ArchiveEntry {
        ArchiveEntry {
            path: PathBuf::from(path),
            kind: ArchiveEntryKind::File,
            compressed_bytes: 10,
            extracted_bytes: 20,
        }
    }

    fn limits() -> ArchiveLimits {
        ArchiveLimits {
            maximum_files: 4,
            maximum_extracted_bytes: 100,
            maximum_compression_ratio: 10,
        }
    }

    #[test]
    fn archive_plan_rejects_windows_aliases_and_unsafe_types() {
        let allowed = HashSet::new();
        for path in [
            "../model.bin",
            "C:/model.bin",
            "model:stream",
            "NUL.bin",
            "model. ",
        ] {
            assert!(
                validate_archive_plan([entry(path)], &allowed, limits()).is_err(),
                "{path}"
            );
        }

        let mut link = entry("model.bin");
        link.kind = ArchiveEntryKind::Symlink;
        assert!(matches!(
            validate_archive_plan([link], &allowed, limits()),
            Err(ArchiveValidationError::UnsupportedEntry { .. })
        ));

        assert!(matches!(
            validate_archive_plan([entry("Model.bin"), entry("model.bin")], &allowed, limits()),
            Err(ArchiveValidationError::DestinationCollision { .. })
        ));
        assert!(matches!(
            validate_archive_plan(
                [entry("caf\u{e9}.bin"), entry("cafe\u{301}.bin")],
                &allowed,
                limits()
            ),
            Err(ArchiveValidationError::DestinationCollision { .. })
        ));
    }

    #[test]
    fn archive_plan_bounds_files_sizes_ratios_and_executables() {
        let allowed = HashSet::new();
        assert!(matches!(
            validate_archive_plan([entry("unexpected.dll")], &allowed, limits()),
            Err(ArchiveValidationError::UnexpectedExecutable { .. })
        ));

        let mut bomb = entry("model.bin");
        bomb.extracted_bytes = 101;
        assert!(validate_archive_plan([bomb], &allowed, limits()).is_err());

        let plan = validate_archive_plan([entry("model.bin")], &allowed, limits())
            .expect("bounded regular file must validate");
        assert_eq!(plan.extracted_bytes(), 20);
    }

    #[test]
    fn lifecycle_stops_at_verified_on_disk() {
        let state = InstallState::Absent
            .transition(InstallState::Downloading {
                received_bytes: 0,
                total_bytes: 10,
            })
            .expect("download")
            .transition(InstallState::Verifying)
            .expect("verify")
            .transition(InstallState::Installing)
            .expect("install")
            .transition(InstallState::VerifiedOnDisk)
            .expect("verified");
        assert!(state.clone().transition(InstallState::Verifying).is_err());
        assert_eq!(
            state.transition(InstallState::Absent),
            Ok(InstallState::Absent)
        );
    }
}
