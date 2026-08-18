//! Standalone file-integrity verification against a [`Pack`]'s trusted digests.
//!
//! `InstallManager::reverify` already does this for archive-based, app-owned
//! installs. This is for packs `InstallManager` cannot install at all yet --
//! Granite's archive-less, loose-GGUF packs (see `docs/handoff/
//! granite-final-pass.md`, Phase 5 and 6) -- where a caller resolves its own
//! `model_root` by hand and still needs to check the bytes there against the
//! manifest before trusting them. Deliberately duplicates
//! `workers/inference-worker`'s private `verify_file`: that copy is internal to
//! a worker binary, and this one has to be callable from `apps/desktop`
//! without depending on a worker's internals.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::Pack;

#[derive(Debug)]
pub enum PackVerificationError {
    /// A required file does not exist under `root`.
    Missing(String),
    /// A required file exists but its length does not match the manifest.
    LengthMismatch(String),
    /// A required file exists and matches in length but not in digest.
    HashMismatch(String),
    Io(io::Error),
}

impl Display for PackVerificationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "pack file verification failed: {self:?}")
    }
}

impl Error for PackVerificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PackVerificationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Verifies every one of `pack`'s [`RequiredFile`](crate::RequiredFile)s exists
/// under `root`, with the exact length and SHA-256 the manifest pins.
///
/// # Errors
///
/// Returns [`PackVerificationError`] on the first file that is missing, the
/// wrong length, or fails its digest check.
pub fn verify_pack_files(pack: &Pack, root: &Path) -> Result<(), PackVerificationError> {
    for required in pack.required_files() {
        let path = root.join(required.path());
        verify_file(&path, required.path(), required.bytes(), required.sha256())?;
    }
    Ok(())
}

fn verify_file(
    path: &Path,
    label: &str,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<(), PackVerificationError> {
    let mut file = File::open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            PackVerificationError::Missing(label.to_owned())
        } else {
            PackVerificationError::Io(error)
        }
    })?;
    let actual_bytes = file.metadata()?.len();
    if actual_bytes != expected_bytes {
        return Err(PackVerificationError::LengthMismatch(label.to_owned()));
    }
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)?;
    let actual_sha256 = format!("{:x}", digest.finalize());
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(PackVerificationError::HashMismatch(label.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;
    use crate::TrustedManifest;

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut digest = Sha256::new();
        digest.update(bytes);
        format!("{:x}", digest.finalize())
    }

    /// A single archive-less, loose-file pack (Granite's schema-v3 shape),
    /// with `required_files` swapped in by the caller -- mirrors the real
    /// Granite entry in `models/trusted-manifest.json` structurally, since
    /// `Pack`'s fields are private and only constructible by deserializing a
    /// full catalog.
    fn catalog_with(required_files: &Value) -> TrustedManifest {
        let catalog = json!({
            "schema_version": 3,
            "manifest_status": "admitted-catalog",
            "generated_utc": "2026-08-03T00:00:00Z",
            "install_eligible": true,
            "artifacts": [],
            "packs": [{
                "id": "verify-test-pack",
                "revision": "r1",
                "display_name": "Verify Test Pack",
                "role": "final-asr",
                "streaming": "offline",
                "install_eligible": true,
                "source": {
                    "upstream_repository": "https://example.test/upstream",
                    "upstream_revision": "a".repeat(40),
                    "conversion": {
                        "repository": "https://example.test/upstream",
                        "revision": "a".repeat(40),
                        "command": "no local conversion",
                        "tool_versions": ["synthetic=1.0.0"],
                        "provenance": "Synthetic deterministic test fixture"
                    }
                },
                "installed_bytes": 10_000_000,
                "required_files": required_files,
                "runtime": {
                    "name": "synthetic-runtime",
                    "version": "1.0.0",
                    "abi": "synthetic-abi-1",
                    "provider": "cpu",
                    "platform": "windows",
                    "architecture": "x86-64",
                    "decoder": "greedy-search-causal-lm",
                    "sample_rate_hz": 16000
                },
                "memory_evidence": [],
                "capabilities": [{
                    "locale": "en-US",
                    "task": "transcribe",
                    "target_locale": null,
                    "features": ["punctuation", "known-locale"]
                }],
                "licenses": [{
                    "component": "synthetic-model",
                    "spdx_id": "MIT",
                    "name": "MIT License",
                    "text_url": "https://example.test/license",
                    "attribution": "Synthetic fixture authors",
                    "modification_notice": "No modifications",
                    "redistribution": "allowed"
                }],
                "compatibility": {
                    "minimum_application_version": "0.1.0",
                    "maximum_application_version": "9.9.9",
                    "minimum_worker_version": "0.1.0",
                    "maximum_worker_version": "9.9.9"
                },
                "variant_group": "verify-test-pack"
            }],
            "limitations": []
        });
        TrustedManifest::parse_bundled(catalog.to_string().as_bytes())
            .expect("synthetic catalog must validate")
    }

    #[test]
    fn matching_files_pass_verification() {
        let dir = tempdir().unwrap();
        let content = b"granite model bytes";
        std::fs::File::create(dir.path().join("model.gguf"))
            .unwrap()
            .write_all(content)
            .unwrap();
        let manifest = catalog_with(&json!([{
            "path": "model.gguf",
            "bytes": content.len(),
            "sha256": sha256_hex(content)
        }]));
        assert!(verify_pack_files(&manifest.packs()[0], dir.path()).is_ok());
    }

    #[test]
    fn a_missing_file_is_reported_as_missing() {
        let dir = tempdir().unwrap();
        let manifest = catalog_with(&json!([{
            "path": "absent.gguf",
            "bytes": 4,
            "sha256": "0".repeat(64)
        }]));
        assert!(matches!(
            verify_pack_files(&manifest.packs()[0], dir.path()),
            Err(PackVerificationError::Missing(name)) if name == "absent.gguf"
        ));
    }

    #[test]
    fn a_length_mismatch_is_reported_before_hashing() {
        let dir = tempdir().unwrap();
        let content = b"short";
        std::fs::File::create(dir.path().join("model.gguf"))
            .unwrap()
            .write_all(content)
            .unwrap();
        let manifest = catalog_with(&json!([{
            "path": "model.gguf",
            "bytes": content.len() as u64 + 1,
            "sha256": sha256_hex(content)
        }]));
        assert!(matches!(
            verify_pack_files(&manifest.packs()[0], dir.path()),
            Err(PackVerificationError::LengthMismatch(name)) if name == "model.gguf"
        ));
    }

    #[test]
    fn a_hash_mismatch_is_reported_when_length_matches() {
        let dir = tempdir().unwrap();
        let content = b"tampered model bytes";
        std::fs::File::create(dir.path().join("model.gguf"))
            .unwrap()
            .write_all(content)
            .unwrap();
        let manifest = catalog_with(&json!([{
            "path": "model.gguf",
            "bytes": content.len(),
            "sha256": "0".repeat(64)
        }]));
        assert!(matches!(
            verify_pack_files(&manifest.packs()[0], dir.path()),
            Err(PackVerificationError::HashMismatch(name)) if name == "model.gguf"
        ));
    }
}
