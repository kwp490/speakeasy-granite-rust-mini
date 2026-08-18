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

    #[test]
    #[ignore = "explicit synthetic diagnostic file lifecycle; ordinary tests remain write-free"]
    fn explicit_synthetic_wav_is_owner_restricted_and_deletable() {
        let destination = std::env::temp_dir().join(format!(
            "speakeasy-diagnostic-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
