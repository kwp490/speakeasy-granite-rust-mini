//! Retained boundary, intentionally not wired into the shipped app (2026-08-07).
//!
//! Diagnostic WAV export remains available for a future explicit debugging
//! workflow with disclosure and owner-only ACLs; no production caller enables
//! it today.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticWavPolicy {
    pub enabled: bool,
}

#[derive(Debug)]
pub struct DiagnosticWavConsent {
    destination: PathBuf,
}

impl DiagnosticWavConsent {
    /// Creates one-shot consent after the caller has shown and recorded the
    /// diagnostic-audio disclosure.
    ///
    /// # Errors
    ///
    /// Returns an error when disclosure was not acknowledged or the destination
    /// is not a new absolute `.wav` path under a canonical existing directory.
    pub fn after_disclosure(destination: &Path, disclosure_acknowledged: bool) -> io::Result<Self> {
        if !disclosure_acknowledged {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "diagnostic WAV disclosure was not acknowledged",
            ));
        }
        Ok(Self {
            destination: validate_destination(destination)?,
        })
    }
}

#[derive(Debug)]
pub struct DiagnosticWavFile {
    path: PathBuf,
}

impl DiagnosticWavFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Deletes the exact diagnostic file represented by this one-shot handle.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error when deletion fails.
    pub fn delete(self) -> io::Result<()> {
        std::fs::remove_file(self.path)
    }
}

/// Writes one explicitly consented mono PCM diagnostic file.
///
/// # Errors
///
/// Returns an error when the policy is disabled, safe exclusive creation or
/// owner-only ACL enforcement fails, input size overflows WAV limits, or I/O
/// cannot complete. Partial files are removed on post-creation failure.
pub fn save_diagnostic_wav(
    policy: DiagnosticWavPolicy,
    consent: DiagnosticWavConsent,
    sample_rate_hz: NonZeroU32,
    mono_samples: &[f32],
) -> io::Result<DiagnosticWavFile> {
    if !policy.enabled {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "diagnostic WAV writing is disabled",
        ));
    }
    let path = consent.destination;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    if let Err(error) = restrict_to_owner(&path) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    if let Err(error) = write_pcm16_wav(file, sample_rate_hz, mono_samples) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(DiagnosticWavFile { path })
}

fn validate_destination(destination: &Path) -> io::Result<PathBuf> {
    if !destination.is_absolute()
        || destination.extension().and_then(|value| value.to_str()) != Some("wav")
        || destination.file_name().is_none()
        || destination.exists()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "diagnostic WAV requires a new absolute .wav destination",
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    let canonical_parent = parent.canonicalize()?;
    Ok(canonical_parent.join(destination.file_name().expect("validated file name")))
}

#[cfg(windows)]
fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "diagnostic WAV export is not implemented: no owner-only ACL restriction exists yet",
    ))
}

#[cfg(not(windows))]
fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "diagnostic WAV ACL policy is Windows-only",
    ))
}

#[allow(clippy::cast_possible_truncation)]
fn write_pcm16_wav(file: File, sample_rate_hz: NonZeroU32, mono_samples: &[f32]) -> io::Result<()> {
    let data_bytes = u32::try_from(mono_samples.len().checked_mul(2).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "diagnostic WAV is too large")
    })?)
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "diagnostic WAV is too large"))?;
    let riff_bytes = 36_u32
        .checked_add(data_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "WAV size overflow"))?;
    let byte_rate = sample_rate_hz
        .get()
        .checked_mul(2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "WAV rate overflow"))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_bytes.to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&sample_rate_hz.get().to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&2_u16.to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    for &sample in mono_samples {
        let finite = if sample.is_finite() { sample } else { 0.0 };
        let pcm = (finite.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        writer.write_all(&pcm.to_le_bytes())?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_disabled_and_missing_disclosure_is_rejected_without_io() {
        assert!(!DiagnosticWavPolicy::default().enabled);
        let error = DiagnosticWavConsent::after_disclosure(
            Path::new(r"C:\not-created\diagnostic.wav"),
            false,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn relative_and_non_wav_destinations_are_rejected_without_io() {
        assert!(DiagnosticWavConsent::after_disclosure(Path::new("diagnostic.wav"), true).is_err());
        assert!(
            DiagnosticWavConsent::after_disclosure(
                Path::new(r"C:\not-created\diagnostic.txt"),
                true
            )
            .is_err()
        );
    }

    /// **Cannot pass on Windows, and that is a fact about the feature rather
    /// than about the test.** `restrict_to_owner` returns `Unsupported` on every
    /// platform — the owner-only ACL it promises does not exist — so
    /// `save_diagnostic_wav` always fails and this asserts a lifecycle nothing
    /// can complete. Un-ignoring it on 2026-08-28 produced exactly that:
    /// `called Result::unwrap() on an Err value: Unsupported`.
    ///
    /// It was ignored as "ordinary tests remain write-free", which is a policy
    /// this repository does not hold — `diagnostic_rotation_preserves_one_previous_generation`
    /// and several others take a `tempfile::tempdir()` and run in the gate. So
    /// the stated reason was not the real one, and it read as a convention
    /// somebody could relax rather than as a feature nobody has written. That is
    /// the same shape as the seven scaffold tests skipped with their inputs
    /// stubbed: an exclusion whose given reason is not the operative one.
    ///
    /// **Un-ignore this when `restrict_to_owner` is implemented, not before**,
    /// and it will then be a real test of the one path that deliberately puts a
    /// user's audio on disk.
    ///
    /// The `tempdir()` is kept regardless of the ignore. It wrote into a
    /// hand-named file under `std::env::temp_dir()`, where a panic between
    /// `save` and `delete` leaks a WAV of the user's audio; a test of a
    /// privacy-sensitive path must not be the thing that leaves one behind, and
    /// a `tempdir()` removes its tree on drop, panic or not.
    #[test]
    #[ignore = "unimplemented: restrict_to_owner has no owner-only ACL on any platform, so save_diagnostic_wav always returns Unsupported"]
    fn explicit_synthetic_wav_is_owner_restricted_and_deletable() {
        let root = tempfile::tempdir().expect("temporary diagnostic root");
        let destination = root
            .path()
            .join(format!("speakeasy-diagnostic-{}.wav", std::process::id()));
        let consent = DiagnosticWavConsent::after_disclosure(&destination, true).unwrap();
        let saved = save_diagnostic_wav(
            DiagnosticWavPolicy { enabled: true },
            consent,
            NonZeroU32::new(16_000).unwrap(),
            &[0.0; 512],
        )
        .unwrap();
        assert_eq!(saved.path().file_name(), destination.file_name());
        assert!(destination.metadata().unwrap().len() > 44);
        saved.delete().unwrap();
        assert!(!destination.exists());
    }
}
