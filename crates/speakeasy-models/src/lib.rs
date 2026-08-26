//! Trusted, metadata-only model catalog boundary for `SpeakEasy`.
//!
//! This crate owns trusted model metadata and the bounded model-install lifecycle.
//! It does not load an ASR runtime or grant filesystem/network authority to the UI.

#![allow(clippy::must_use_candidate)]

mod archive;
mod compatibility;
mod download;
mod gpu;
mod granite_gpu;
mod hardware;
mod lifecycle;
mod manager;
mod manifest;
mod recommendation;
mod resume;
mod selection;
mod verify;
mod views;

pub use archive::ArchiveExtractionError;
pub use compatibility::{CompatibilityContext, CompatibilityIssue, CompatibilityResolution};
pub use download::{DownloadError, DownloadRequest, DownloadResult, download_to_file};
pub use gpu::{
    ComputeCapability, CudaDevice, ExecutionEvidence, GpuProbe, GpuProbeFailure, GpuQualification,
    GpuRejection, GpuSnapshot, MINIMUM_COMPUTE_CAPABILITY, NvmlGpuProbe, admit, admit_engine,
};
pub use granite_gpu::{
    CudaContextProbe, CudaContextProof, GRANITE_CUDA_WORKER_ARTIFACT_ID, GpuPayloadRejection,
    NvmlCudaContextProbe, gpu_configuration_is_installable, inspect_gpu_payload,
    prove_cuda_context, required_cuda_runtime_files,
};
pub use hardware::{DetectedAdapter, HardwareProbe, HardwareSnapshot, SafeStandardHardwareProbe};
pub use lifecycle::{
    ArchiveEntry, ArchiveEntryKind, ArchiveLimits, ArchiveValidationError, InstallFailure,
    InstallState, RuntimeEvidence, RuntimeState, ValidatedArchivePlan, validate_archive_plan,
};
pub use manager::{
    ArtifactLease, InstallError, InstallFile, InstallManager, InstallSpec, LooseInstallFile,
    StagedArtifact,
};
pub use manifest::{
    Architecture, Archive, Capability, CapabilityFeature, CompatibilityRange, ConversionProvenance,
    ExecutionProvider, LicenseNotice, ManifestError, ManifestStatus, MemoryEvidence,
    NativeRuntimeSource, Pack, PackRole, Platform, ProofArtifact, RedistributionDecision,
    RequiredFile, RuntimeRequirement, SourceProvenance, StreamingClassification, Task,
    TrustedManifest,
};
pub use recommendation::{ConfirmationDisclosure, recommendation_disclosure};
pub use resume::{
    DownloadPolicy, ResumeDecision, ResumeMetadata, ResumeResponse, validate_resume_response,
};
pub use selection::{ExactPackRequest, ExactPackSelection, RoleSelectionError, SelectionError};
pub use verify::{PackVerificationError, verify_pack_files};
pub use views::{CapabilityView, LicenseNoticeView};

/// The only production trust root supported by this phase: bytes compiled into
/// the signed Rust binary. A loose file is never substituted at runtime.
pub const BUNDLED_TRUSTED_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../models/trusted-manifest.json");

/// Parse and validate the manifest bytes compiled into this crate.
///
/// # Errors
///
/// Returns [`ManifestError`] when the compiled bytes do not match the supported
/// schema or violate a trust invariant.
pub fn bundled_manifest() -> Result<TrustedManifest, ManifestError> {
    TrustedManifest::parse_bundled(BUNDLED_TRUSTED_MANIFEST_BYTES)
}

#[cfg(test)]
mod tests;
