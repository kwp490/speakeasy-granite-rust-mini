use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use semver::Version;
use serde::Deserialize;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 3;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROOF_ARTIFACTS: usize = 512;
const MAX_PACKS: usize = 256;
const MAX_FILES_PER_ENTRY: usize = 1_024;

#[derive(Debug)]
pub enum ManifestError {
    TooLarge { actual: usize, maximum: usize },
    Json(serde_json::Error),
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    Invariant { path: String, message: String },
}

impl ManifestError {
    fn invariant(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Invariant {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl Display for ManifestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "manifest is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::Json(error) => write!(formatter, "manifest JSON is invalid: {error}"),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "unsupported manifest schema version {found}; supported version is {supported}"
            ),
            Self::Invariant { path, message } => {
                write!(formatter, "manifest invariant failed at {path}: {message}")
            }
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::TooLarge { .. }
            | Self::UnsupportedSchemaVersion { .. }
            | Self::Invariant { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum ManifestStatus {
    #[serde(rename = "phase-0b-proof-only")]
    Phase0bProofOnly,
    #[serde(rename = "admitted-catalog")]
    AdmittedCatalog,
    #[serde(rename = "release-catalog")]
    ReleaseCatalog,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedManifest {
    schema_version: u32,
    manifest_status: ManifestStatus,
    generated_utc: String,
    install_eligible: bool,
    artifacts: Vec<ProofArtifact>,
    packs: Vec<Pack>,
    limitations: Vec<String>,
    #[serde(skip)]
    trust_origin: TrustOrigin,
}

impl TrustedManifest {
    /// Parse a bounded JSON document for inspection and validate every data
    /// invariant. Caller-supplied bytes remain untrusted and cannot be selected.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] for oversized input, malformed/unknown JSON,
    /// unsupported schema versions, or invalid catalog/proof metadata.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge {
                actual: bytes.len(),
                maximum: MAX_MANIFEST_BYTES,
            });
        }

        let manifest: Self = serde_json::from_slice(bytes).map_err(ManifestError::Json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub(crate) fn parse_bundled(bytes: &[u8]) -> Result<Self, ManifestError> {
        let mut manifest = Self::parse(bytes)?;
        manifest.trust_origin = TrustOrigin::Bundled;
        Ok(manifest)
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn status(&self) -> ManifestStatus {
        self.manifest_status
    }

    pub fn generated_utc(&self) -> &str {
        &self.generated_utc
    }

    pub const fn is_install_eligible(&self) -> bool {
        self.install_eligible
    }

    pub fn proof_artifacts(&self) -> &[ProofArtifact] {
        &self.artifacts
    }

    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }

    pub fn limitations(&self) -> &[String] {
        &self.limitations
    }

    pub(crate) const fn has_bundled_trust(&self) -> bool {
        matches!(self.trust_origin, TrustOrigin::Bundled)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        if self.generated_utc.trim().is_empty() || !self.generated_utc.ends_with('Z') {
            return Err(ManifestError::invariant(
                "generated_utc",
                "must be a non-empty UTC timestamp ending in Z",
            ));
        }
        if self.manifest_status == ManifestStatus::Phase0bProofOnly && self.install_eligible {
            return Err(ManifestError::invariant(
                "install_eligible",
                "a proof-only manifest cannot authorize installation",
            ));
        }
        check_count("artifacts", self.artifacts.len(), MAX_PROOF_ARTIFACTS)?;
        check_count("packs", self.packs.len(), MAX_PACKS)?;

        let mut artifact_ids = HashSet::new();
        for (index, artifact) in self.artifacts.iter().enumerate() {
            let path = format!("artifacts[{index}]");
            artifact.validate(&path)?;
            if !artifact_ids.insert(artifact.id()) {
                return Err(ManifestError::invariant(
                    format!("{path}.id"),
                    "duplicate proof-artifact ID",
                ));
            }
        }

        let mut pack_revisions = HashSet::new();
        for (index, pack) in self.packs.iter().enumerate() {
            let path = format!("packs[{index}]");
            pack.validate(&path)?;
            if !pack_revisions.insert((pack.id.as_str(), pack.revision.as_str())) {
                return Err(ManifestError::invariant(
                    format!("{path}.revision"),
                    "duplicate pack ID and revision",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
enum TrustOrigin {
    #[default]
    CallerSupplied,
    Bundled,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProofArtifact {
    #[serde(rename = "native-runtime")]
    NativeRuntime {
        id: String,
        version: String,
        /// The upstream revision this binary was built from, when there is one.
        ///
        /// Optional because not every native runtime has one to give. sherpa
        /// publishes a git SHA; NVIDIA ships versioned redistributable archives
        /// built from no public tree, and `version` already identifies those
        /// exactly. The alternative was to put the archive's own SHA-256 here,
        /// which would satisfy the validator while saying something untrue
        /// about where the bytes came from.
        #[serde(default)]
        source_commit: Option<String>,
        url: String,
        archive_bytes: u64,
        archive_sha256: String,
        /// The directory every member of this archive sits under, stripped
        /// before a `proof_files` path is matched.
        ///
        /// A property of the archive's bytes rather than of our requirements,
        /// which is why it belongs here beside the digest that pins them: NVIDIA
        /// wraps each redistributable in a `<name>-<version>-archive/` directory
        /// and sherpa in its release-name directory, and a republish moves the
        /// URL, the digest and the prefix together. Optional because an archive
        /// with no wrapping directory is legitimate — the `strip_prefix` this
        /// feeds falls through to the raw path.
        #[serde(default)]
        archive_prefix: Option<String>,
        extracted_bytes: u64,
        licenses: Vec<String>,
        proof_files: Vec<RequiredFile>,
        proof_status: String,
    },
    #[serde(rename = "online-asr-model")]
    OnlineAsrModel {
        id: String,
        upstream_model: String,
        upstream_revision: String,
        conversion_source: String,
        url: String,
        archive_bytes: u64,
        archive_sha256: String,
        extracted_bytes: u64,
        license: String,
        files: Vec<RequiredFile>,
        classification: String,
        proof_status: String,
        release_status: String,
    },
    #[serde(rename = "offline-asr-model")]
    OfflineAsrModel {
        id: String,
        upstream_model: String,
        upstream_revision: String,
        conversion_source: String,
        url: String,
        archive_bytes: u64,
        archive_sha256: String,
        extracted_bytes: u64,
        license: String,
        files: Vec<RequiredFile>,
        classification: String,
        proof_status: String,
        release_status: String,
    },
    #[serde(rename = "vad-model")]
    VadModel {
        id: String,
        url: String,
        bytes: u64,
        sha256: String,
        license: String,
        proof_status: String,
        release_status: String,
    },
}

/// A native-runtime artifact's fetchable identity, for a caller that has to
/// install one rather than merely record that it was proved.
///
/// Exists because [`ProofArtifact`] exposed nothing but [`ProofArtifact::id`],
/// which is all a provenance record needs — and the CUDA runtime is the first
/// artifact the *app* fetches, so it needs the URL, the archive digest and the
/// per-file digests it has always carried. Borrowed rather than cloned: the
/// manifest outlives every install.
#[derive(Clone, Copy, Debug)]
pub struct NativeRuntimeSource<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub url: &'a str,
    pub archive_bytes: u64,
    pub archive_sha256: &'a str,
    /// Empty when the archive has no wrapping directory, which is the form
    /// [`std::path::Path::strip_prefix`] treats as a no-op.
    pub archive_prefix: &'a str,
    /// Every file this archive is pinned to contain. A caller installs a subset;
    /// nothing may be installed that is not in here, because these are the only
    /// digests the manifest vouches for.
    pub proof_files: &'a [RequiredFile],
}

impl ProofArtifact {
    pub fn id(&self) -> &str {
        match self {
            Self::NativeRuntime { id, .. }
            | Self::OnlineAsrModel { id, .. }
            | Self::OfflineAsrModel { id, .. }
            | Self::VadModel { id, .. } => id,
        }
    }

    /// This artifact as something installable, or `None` when it is not a
    /// native runtime.
    pub fn native_runtime_source(&self) -> Option<NativeRuntimeSource<'_>> {
        match self {
            Self::NativeRuntime {
                id,
                version,
                url,
                archive_bytes,
                archive_sha256,
                archive_prefix,
                proof_files,
                ..
            } => Some(NativeRuntimeSource {
                id,
                version,
                url,
                archive_bytes: *archive_bytes,
                archive_sha256,
                archive_prefix: archive_prefix.as_deref().unwrap_or_default(),
                proof_files,
            }),
            Self::OnlineAsrModel { .. } | Self::OfflineAsrModel { .. } | Self::VadModel { .. } => {
                None
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_identifier(&format!("{path}.id"), self.id())?;
        match self {
            Self::NativeRuntime {
                version,
                source_commit,
                url,
                archive_bytes,
                archive_sha256,
                archive_prefix,
                extracted_bytes,
                licenses,
                proof_files,
                proof_status,
                ..
            } => {
                check_non_empty(&format!("{path}.version"), version)?;
                if let Some(source_commit) = source_commit {
                    check_immutable_revision(&format!("{path}.source_commit"), source_commit)?;
                }
                if let Some(archive_prefix) = archive_prefix {
                    check_relative_path(&format!("{path}.archive_prefix"), archive_prefix)?;
                }
                check_https_url(&format!("{path}.url"), url)?;
                check_positive(&format!("{path}.archive_bytes"), *archive_bytes)?;
                check_sha256(&format!("{path}.archive_sha256"), archive_sha256)?;
                check_positive(&format!("{path}.extracted_bytes"), *extracted_bytes)?;
                check_non_empty_list(&format!("{path}.licenses"), licenses)?;
                check_files(
                    &format!("{path}.proof_files"),
                    proof_files,
                    *extracted_bytes,
                )?;
                check_non_empty(&format!("{path}.proof_status"), proof_status)?;
            }
            Self::OnlineAsrModel {
                upstream_model,
                upstream_revision,
                conversion_source,
                url,
                archive_bytes,
                archive_sha256,
                extracted_bytes,
                license,
                files,
                classification,
                proof_status,
                release_status,
                ..
            }
            | Self::OfflineAsrModel {
                upstream_model,
                upstream_revision,
                conversion_source,
                url,
                archive_bytes,
                archive_sha256,
                extracted_bytes,
                license,
                files,
                classification,
                proof_status,
                release_status,
                ..
            } => {
                check_non_empty(&format!("{path}.upstream_model"), upstream_model)?;
                check_immutable_revision(&format!("{path}.upstream_revision"), upstream_revision)?;
                check_non_empty(&format!("{path}.conversion_source"), conversion_source)?;
                check_https_url(&format!("{path}.url"), url)?;
                check_positive(&format!("{path}.archive_bytes"), *archive_bytes)?;
                check_sha256(&format!("{path}.archive_sha256"), archive_sha256)?;
                check_positive(&format!("{path}.extracted_bytes"), *extracted_bytes)?;
                check_non_empty(&format!("{path}.license"), license)?;
                check_files(&format!("{path}.files"), files, *extracted_bytes)?;
                check_non_empty(&format!("{path}.classification"), classification)?;
                check_non_empty(&format!("{path}.proof_status"), proof_status)?;
                if !matches!(
                    release_status.as_str(),
                    "not-selected" | "selected-not-release-qualified"
                ) {
                    return Err(ManifestError::invariant(
                        format!("{path}.release_status"),
                        "proof artifacts must be unselected or selected without release qualification",
                    ));
                }
            }
            Self::VadModel {
                url,
                bytes,
                sha256,
                license,
                proof_status,
                release_status,
                ..
            } => {
                check_https_url(&format!("{path}.url"), url)?;
                check_positive(&format!("{path}.bytes"), *bytes)?;
                check_sha256(&format!("{path}.sha256"), sha256)?;
                check_non_empty(&format!("{path}.license"), license)?;
                check_non_empty(&format!("{path}.proof_status"), proof_status)?;
                if release_status != "not-selected" {
                    return Err(ManifestError::invariant(
                        format!("{path}.release_status"),
                        "proof artifacts must remain not-selected",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    id: String,
    revision: String,
    display_name: String,
    role: PackRole,
    streaming: StreamingClassification,
    install_eligible: bool,
    source: SourceProvenance,
    /// The single archive this pack installs from, when it has one.
    ///
    /// Schema v3. `None` means the pack has no wrapping archive at all —
    /// Hugging Face serves Granite's GGUFs as loose files, so there is
    /// nothing to strip an [`archive_prefix`](Self::archive_prefix) from or to
    /// check one combined digest against. Each entry in `required_files`
    /// carries its own [`RequiredFile::url`] instead.
    #[serde(default)]
    archive: Option<Archive>,
    #[serde(default)]
    archive_prefix: String,
    installed_bytes: u64,
    required_files: Vec<RequiredFile>,
    runtime: RuntimeRequirement,
    memory_evidence: Vec<MemoryEvidence>,
    capabilities: Vec<Capability>,
    licenses: Vec<LicenseNotice>,
    compatibility: CompatibilityRange,
    variant_group: Option<String>,
    chunk_size_ms: Option<u32>,
}

impl Pack {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn role(&self) -> PackRole {
        self.role
    }

    pub const fn streaming(&self) -> StreamingClassification {
        self.streaming
    }

    pub const fn is_install_eligible(&self) -> bool {
        self.install_eligible
    }

    pub const fn installed_bytes(&self) -> u64 {
        self.installed_bytes
    }

    pub fn source(&self) -> &SourceProvenance {
        &self.source
    }

    pub const fn archive(&self) -> Option<&Archive> {
        self.archive.as_ref()
    }

    pub fn archive_prefix(&self) -> &str {
        &self.archive_prefix
    }

    /// Whether this pack can be fetched rather than only supplied on disk:
    /// either its archive is downloadable, or — for an archive-less,
    /// loose-file pack — every required file carries its own URL.
    pub fn is_downloadable(&self) -> bool {
        match &self.archive {
            Some(archive) => archive.is_downloadable(),
            None => {
                !self.required_files.is_empty()
                    && self.required_files.iter().all(|file| file.url.is_some())
            }
        }
    }

    pub fn required_files(&self) -> &[RequiredFile] {
        &self.required_files
    }

    pub fn runtime(&self) -> &RuntimeRequirement {
        &self.runtime
    }

    pub fn memory_evidence(&self) -> &[MemoryEvidence] {
        &self.memory_evidence
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn licenses(&self) -> &[LicenseNotice] {
        &self.licenses
    }

    pub fn compatibility(&self) -> &CompatibilityRange {
        &self.compatibility
    }

    pub fn variant_group(&self) -> Option<&str> {
        self.variant_group.as_deref()
    }

    pub const fn chunk_size_ms(&self) -> Option<u32> {
        self.chunk_size_ms
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_identifier(&format!("{path}.id"), &self.id)?;
        check_identifier(&format!("{path}.revision"), &self.revision)?;
        check_non_empty(&format!("{path}.display_name"), &self.display_name)?;
        if self.role == PackRole::StreamingAsr
            && self.streaming != StreamingClassification::TrueOnline
        {
            return Err(ManifestError::invariant(
                format!("{path}.streaming"),
                "a streaming-asr pack must be classified true-online",
            ));
        }
        if self.role == PackRole::Vad && self.streaming != StreamingClassification::NotApplicable {
            return Err(ManifestError::invariant(
                format!("{path}.streaming"),
                "a VAD pack must use not-applicable streaming classification",
            ));
        }
        self.source.validate(&format!("{path}.source"))?;
        if let Some(archive) = &self.archive {
            archive.validate(&format!("{path}.archive"))?;
        }
        if !self.archive_prefix.is_empty() {
            check_relative_path(&format!("{path}.archive_prefix"), &self.archive_prefix)?;
        }
        check_positive(&format!("{path}.installed_bytes"), self.installed_bytes)?;
        check_files(
            &format!("{path}.required_files"),
            &self.required_files,
            self.installed_bytes,
        )?;
        self.runtime.validate(&format!("{path}.runtime"))?;
        check_count(
            &format!("{path}.memory_evidence"),
            self.memory_evidence.len(),
            128,
        )?;
        for (index, evidence) in self.memory_evidence.iter().enumerate() {
            evidence.validate(&format!("{path}.memory_evidence[{index}]"))?;
        }
        self.validate_capabilities(path)?;
        self.validate_licenses(path)?;
        self.compatibility
            .validate(&format!("{path}.compatibility"))?;
        if let Some(group) = &self.variant_group {
            check_identifier(&format!("{path}.variant_group"), group)?;
        }
        if self.chunk_size_ms == Some(0) {
            return Err(ManifestError::invariant(
                format!("{path}.chunk_size_ms"),
                "must be greater than zero when present",
            ));
        }
        Ok(())
    }

    fn validate_capabilities(&self, path: &str) -> Result<(), ManifestError> {
        check_count(
            &format!("{path}.capabilities"),
            self.capabilities.len(),
            512,
        )?;
        if self.capabilities.is_empty() {
            return Err(ManifestError::invariant(
                format!("{path}.capabilities"),
                "at least one capability is required",
            ));
        }
        let mut capabilities = HashSet::new();
        for (index, capability) in self.capabilities.iter().enumerate() {
            capability.validate(&format!("{path}.capabilities[{index}]"))?;
            if !capabilities.insert((
                capability.locale.as_str(),
                capability.task,
                capability.target_locale.as_deref(),
            )) {
                return Err(ManifestError::invariant(
                    format!("{path}.capabilities[{index}]"),
                    "duplicate locale/task/target capability",
                ));
            }
        }
        Ok(())
    }

    fn validate_licenses(&self, path: &str) -> Result<(), ManifestError> {
        check_count(&format!("{path}.licenses"), self.licenses.len(), 128)?;
        if self.licenses.is_empty() {
            return Err(ManifestError::invariant(
                format!("{path}.licenses"),
                "at least one license/notice record is required",
            ));
        }
        let mut components = HashSet::new();
        for (index, license) in self.licenses.iter().enumerate() {
            license.validate(&format!("{path}.licenses[{index}]"))?;
            if !components.insert(license.component.as_str()) {
                return Err(ManifestError::invariant(
                    format!("{path}.licenses[{index}].component"),
                    "duplicate license component",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    upstream_repository: String,
    upstream_revision: String,
    conversion: ConversionProvenance,
}

impl SourceProvenance {
    pub fn upstream_repository(&self) -> &str {
        &self.upstream_repository
    }

    pub fn upstream_revision(&self) -> &str {
        &self.upstream_revision
    }

    pub fn conversion(&self) -> &ConversionProvenance {
        &self.conversion
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_https_url(
            &format!("{path}.upstream_repository"),
            &self.upstream_repository,
        )?;
        check_immutable_revision(
            &format!("{path}.upstream_revision"),
            &self.upstream_revision,
        )?;
        self.conversion.validate(&format!("{path}.conversion"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversionProvenance {
    repository: String,
    revision: String,
    command: String,
    tool_versions: Vec<String>,
    provenance: String,
}

impl ConversionProvenance {
    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn tool_versions(&self) -> &[String] {
        &self.tool_versions
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_https_url(&format!("{path}.repository"), &self.repository)?;
        check_immutable_revision(&format!("{path}.revision"), &self.revision)?;
        check_non_empty(&format!("{path}.command"), &self.command)?;
        check_non_empty_list(&format!("{path}.tool_versions"), &self.tool_versions)?;
        check_non_empty(&format!("{path}.provenance"), &self.provenance)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// Where the archive can be fetched, when it can be fetched at all.
    ///
    /// `None` means the bytes exist but are not published anywhere: the pack
    /// can be installed from an archive the caller supplies on disk, and can
    /// never be downloaded. The self-exported CUDA model is the case this
    /// exists for — float ONNX for this model is published by nobody, so the
    /// only copy is the one produced locally.
    ///
    /// `bytes` and `sha256` stay required either way. Not knowing where an
    /// archive came from is survivable; not knowing whether it is the right
    /// archive is not.
    #[serde(default)]
    url: Option<String>,
    bytes: u64,
    sha256: String,
}

impl Archive {
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Whether this archive can be fetched rather than supplied.
    pub const fn is_downloadable(&self) -> bool {
        self.url.is_some()
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        if let Some(url) = &self.url {
            check_https_url(&format!("{path}.url"), url)?;
        }
        check_positive(&format!("{path}.bytes"), self.bytes)?;
        check_sha256(&format!("{path}.sha256"), &self.sha256)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredFile {
    path: String,
    bytes: u64,
    sha256: String,
    /// Where this exact file can be fetched on its own, when it can be.
    ///
    /// Schema v3. Every file this project has pinned before Granite arrived
    /// once inside a single archive, so one URL at the archive's level was
    /// enough. Hugging Face serves Granite's GGUFs as loose files with no
    /// wrapping archive, so a pack with no [`Archive`] (see
    /// [`Pack::archive`]) has to carry a URL per file instead.
    #[serde(default)]
    url: Option<String>,
}

impl RequiredFile {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PackRole {
    StreamingAsr,
    FinalAsr,
    Translation,
    Vad,
}

impl PackRole {
    /// The spelling this role has in the manifest JSON.
    ///
    /// Errors that name a role are read next to `trusted-manifest.json`, so
    /// they use the manifest's vocabulary rather than Rust's — `streaming-asr`
    /// is greppable in the file the reader is about to open, `StreamingAsr` is
    /// not.
    pub const fn as_manifest_str(self) -> &'static str {
        match self {
            Self::StreamingAsr => "streaming-asr",
            Self::FinalAsr => "final-asr",
            Self::Translation => "translation",
            Self::Vad => "vad",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum StreamingClassification {
    TrueOnline,
    Offline,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Architecture {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionProvider {
    Cpu,
    DirectMl,
    Cuda,
}

impl ExecutionProvider {
    /// The spelling this provider has in the manifest JSON.
    ///
    /// Same reasoning as [`PackRole::as_manifest_str`]: a selection error that
    /// names a provider is read next to `trusted-manifest.json`, so it should
    /// use a word that can be grepped for in the file the reader is about to
    /// open.
    pub const fn as_manifest_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::DirectMl => "direct-ml",
            Self::Cuda => "cuda",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirement {
    name: String,
    version: String,
    abi: String,
    provider: ExecutionProvider,
    platform: Platform,
    architecture: Architecture,
    decoder: String,
    sample_rate_hz: u32,
}

impl RuntimeRequirement {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn abi(&self) -> &str {
        &self.abi
    }

    pub const fn provider(&self) -> ExecutionProvider {
        self.provider
    }

    pub const fn platform(&self) -> Platform {
        self.platform
    }

    pub const fn architecture(&self) -> Architecture {
        self.architecture
    }

    pub fn decoder(&self) -> &str {
        &self.decoder
    }

    pub const fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_non_empty(&format!("{path}.name"), &self.name)?;
        Version::parse(&self.version).map_err(|error| {
            ManifestError::invariant(
                format!("{path}.version"),
                format!("must be semantic version: {error}"),
            )
        })?;
        check_non_empty(&format!("{path}.abi"), &self.abi)?;
        check_non_empty(&format!("{path}.decoder"), &self.decoder)?;
        if self.sample_rate_hz == 0 {
            return Err(ManifestError::invariant(
                format!("{path}.sample_rate_hz"),
                "must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEvidence {
    evidence_id: String,
    private_working_set_min_bytes: u64,
    private_working_set_max_bytes: u64,
    vram_min_bytes: Option<u64>,
    vram_max_bytes: Option<u64>,
}

impl MemoryEvidence {
    pub fn evidence_id(&self) -> &str {
        &self.evidence_id
    }

    pub const fn private_working_set_range(&self) -> (u64, u64) {
        (
            self.private_working_set_min_bytes,
            self.private_working_set_max_bytes,
        )
    }

    pub const fn vram_range(&self) -> Option<(u64, u64)> {
        match (self.vram_min_bytes, self.vram_max_bytes) {
            (Some(minimum), Some(maximum)) => Some((minimum, maximum)),
            _ => None,
        }
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_identifier(&format!("{path}.evidence_id"), &self.evidence_id)?;
        check_range(
            &format!("{path}.private_working_set"),
            self.private_working_set_min_bytes,
            self.private_working_set_max_bytes,
        )?;
        match (self.vram_min_bytes, self.vram_max_bytes) {
            (None, None) => Ok(()),
            (Some(minimum), Some(maximum)) => {
                check_range(&format!("{path}.vram"), minimum, maximum)
            }
            _ => Err(ManifestError::invariant(
                format!("{path}.vram"),
                "minimum and maximum must either both be present or both be absent",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Task {
    Transcribe,
    Translate,
    VoiceActivityDetection,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityFeature {
    Punctuation,
    Timestamps,
    KnownLocale,
    AutomaticLanguageIdentification,
    Hotwords,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    locale: String,
    task: Task,
    target_locale: Option<String>,
    features: Vec<CapabilityFeature>,
}

impl Capability {
    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub const fn task(&self) -> Task {
        self.task
    }

    pub fn target_locale(&self) -> Option<&str> {
        self.target_locale.as_deref()
    }

    pub fn features(&self) -> &[CapabilityFeature] {
        &self.features
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_locale(&format!("{path}.locale"), &self.locale)?;
        match self.task {
            Task::Translate => {
                let target = self.target_locale.as_deref().ok_or_else(|| {
                    ManifestError::invariant(
                        format!("{path}.target_locale"),
                        "translation requires a target locale",
                    )
                })?;
                check_locale(&format!("{path}.target_locale"), target)?;
                if target.eq_ignore_ascii_case(&self.locale) {
                    return Err(ManifestError::invariant(
                        format!("{path}.target_locale"),
                        "translation source and target locales must differ",
                    ));
                }
            }
            Task::Transcribe | Task::VoiceActivityDetection => {
                if self.target_locale.is_some() {
                    return Err(ManifestError::invariant(
                        format!("{path}.target_locale"),
                        "only translation may declare a target locale",
                    ));
                }
            }
        }

        let mut features = HashSet::new();
        if self
            .features
            .iter()
            .any(|feature| !features.insert(*feature))
        {
            return Err(ManifestError::invariant(
                format!("{path}.features"),
                "duplicate capability feature",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RedistributionDecision {
    Allowed,
    ReviewRequired,
    Prohibited,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseNotice {
    component: String,
    spdx_id: Option<String>,
    name: String,
    text_url: String,
    attribution: String,
    modification_notice: String,
    redistribution: RedistributionDecision,
}

impl LicenseNotice {
    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn spdx_id(&self) -> Option<&str> {
        self.spdx_id.as_deref()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text_url(&self) -> &str {
        &self.text_url
    }

    pub fn attribution(&self) -> &str {
        &self.attribution
    }

    pub fn modification_notice(&self) -> &str {
        &self.modification_notice
    }

    pub const fn redistribution(&self) -> RedistributionDecision {
        self.redistribution
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        check_identifier(&format!("{path}.component"), &self.component)?;
        if let Some(spdx_id) = &self.spdx_id {
            check_non_empty(&format!("{path}.spdx_id"), spdx_id)?;
        }
        check_non_empty(&format!("{path}.name"), &self.name)?;
        check_https_url(&format!("{path}.text_url"), &self.text_url)?;
        check_non_empty(&format!("{path}.attribution"), &self.attribution)?;
        check_non_empty(
            &format!("{path}.modification_notice"),
            &self.modification_notice,
        )
    }
}

/// A pack's declared application/worker version window.
///
/// The maxima are optional, and **no pack in `models/trusted-manifest.json`
/// declares one** — see `bundled_packs_declare_no_version_ceiling`. They used to:
/// every pack capped both at the then-current product version, which made an
/// ordinary version bump silently fatal. `select_exact` rejects an out-of-range
/// pack with `SelectionError::Incompatible`, so a build one patch above the
/// ceiling installed cleanly and then refused to select any ASR pack at all —
/// the app came up and could not transcribe. The ceiling had to be raised in
/// lockstep with every release, and nothing failed at build time when it was
/// not.
///
/// The fields stay because a genuine upper bound is a real thing to be able to
/// express — a pack that is actually known to break above some version. What is
/// gone is declaring one *by default*, where it encoded no knowledge and only
/// armed a trap.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
pub struct CompatibilityRange {
    minimum_application_version: VersionString,
    maximum_application_version: Option<VersionString>,
    minimum_worker_version: VersionString,
    maximum_worker_version: Option<VersionString>,
}

impl CompatibilityRange {
    pub fn minimum_application_version(&self) -> &Version {
        &self.minimum_application_version.0
    }

    pub fn maximum_application_version(&self) -> Option<&Version> {
        self.maximum_application_version
            .as_ref()
            .map(|version| &version.0)
    }

    pub fn minimum_worker_version(&self) -> &Version {
        &self.minimum_worker_version.0
    }

    pub fn maximum_worker_version(&self) -> Option<&Version> {
        self.maximum_worker_version
            .as_ref()
            .map(|version| &version.0)
    }

    fn validate(&self, path: &str) -> Result<(), ManifestError> {
        if self
            .maximum_application_version
            .as_ref()
            .is_some_and(|maximum| self.minimum_application_version.0 > maximum.0)
        {
            return Err(ManifestError::invariant(
                format!("{path}.maximum_application_version"),
                "must be greater than or equal to the minimum",
            ));
        }
        if self
            .maximum_worker_version
            .as_ref()
            .is_some_and(|maximum| self.minimum_worker_version.0 > maximum.0)
        {
            return Err(ManifestError::invariant(
                format!("{path}.maximum_worker_version"),
                "must be greater than or equal to the minimum",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct VersionString(Version);

impl<'de> Deserialize<'de> for VersionString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Version::parse(&raw)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

fn check_count(path: &str, actual: usize, maximum: usize) -> Result<(), ManifestError> {
    if actual > maximum {
        return Err(ManifestError::invariant(
            path,
            format!("contains {actual} entries; maximum is {maximum}"),
        ));
    }
    Ok(())
}

fn check_non_empty(path: &str, value: &str) -> Result<(), ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::invariant(path, "must not be empty"));
    }
    Ok(())
}

fn check_non_empty_list(path: &str, values: &[String]) -> Result<(), ManifestError> {
    if values.is_empty() {
        return Err(ManifestError::invariant(path, "must not be empty"));
    }
    for (index, value) in values.iter().enumerate() {
        check_non_empty(&format!("{path}[{index}]"), value)?;
    }
    Ok(())
}

fn check_identifier(path: &str, value: &str) -> Result<(), ManifestError> {
    if value.is_empty()
        || value.len() > 160
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
        })
    {
        return Err(ManifestError::invariant(
            path,
            "must contain only lowercase ASCII letters, digits, dash, underscore, or dot",
        ));
    }
    Ok(())
}

fn check_immutable_revision(path: &str, value: &str) -> Result<(), ManifestError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManifestError::invariant(
            path,
            "must be a lowercase 40- or 64-character immutable hexadecimal revision",
        ));
    }
    Ok(())
}

fn check_sha256(path: &str, value: &str) -> Result<(), ManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManifestError::invariant(
            path,
            "must be a lowercase 64-character SHA-256 digest",
        ));
    }
    Ok(())
}

fn check_https_url(path: &str, value: &str) -> Result<(), ManifestError> {
    let remainder = value
        .strip_prefix("https://")
        .ok_or_else(|| ManifestError::invariant(path, "must use an absolute HTTPS URL"))?;
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || authority.contains('@')
        || value.contains('#')
        || value.chars().any(char::is_whitespace)
    {
        return Err(ManifestError::invariant(
            path,
            "must have a host and no credentials, fragment, or whitespace",
        ));
    }
    Ok(())
}

fn check_positive(path: &str, value: u64) -> Result<(), ManifestError> {
    if value == 0 {
        return Err(ManifestError::invariant(path, "must be greater than zero"));
    }
    Ok(())
}

fn check_range(path: &str, minimum: u64, maximum: u64) -> Result<(), ManifestError> {
    if minimum == 0 || maximum < minimum {
        return Err(ManifestError::invariant(
            path,
            "must have a non-zero minimum no greater than the maximum",
        ));
    }
    Ok(())
}

fn check_files(
    path: &str,
    files: &[RequiredFile],
    installed_bytes: u64,
) -> Result<(), ManifestError> {
    check_count(path, files.len(), MAX_FILES_PER_ENTRY)?;
    if files.is_empty() {
        return Err(ManifestError::invariant(path, "must not be empty"));
    }

    let mut destination_keys = HashSet::new();
    let mut total_bytes = 0_u64;
    for (index, file) in files.iter().enumerate() {
        let file_path = format!("{path}[{index}]");
        check_relative_path(&format!("{file_path}.path"), &file.path)?;
        check_positive(&format!("{file_path}.bytes"), file.bytes)?;
        check_sha256(&format!("{file_path}.sha256"), &file.sha256)?;
        if let Some(url) = &file.url {
            check_https_url(&format!("{file_path}.url"), url)?;
        }
        let destination_key = file.path.replace('\\', "/").to_ascii_lowercase();
        if !destination_keys.insert(destination_key) {
            return Err(ManifestError::invariant(
                format!("{file_path}.path"),
                "duplicates another Windows destination key",
            ));
        }
        total_bytes = total_bytes
            .checked_add(file.bytes)
            .ok_or_else(|| ManifestError::invariant(path, "required-file byte total overflowed"))?;
    }
    if total_bytes > installed_bytes {
        return Err(ManifestError::invariant(
            path,
            "required-file byte total exceeds installed size",
        ));
    }
    Ok(())
}

fn check_relative_path(path: &str, value: &str) -> Result<(), ManifestError> {
    if value.is_empty() || value.starts_with(['/', '\\']) || value.contains(':') {
        return Err(ManifestError::invariant(
            path,
            "must be a non-empty relative path without drive or stream syntax",
        ));
    }
    for component in value.split(['/', '\\']) {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with([' ', '.'])
        {
            return Err(ManifestError::invariant(
                path,
                "contains an unsafe or aliasing path component",
            ));
        }
    }
    Ok(())
}

fn check_locale(path: &str, value: &str) -> Result<(), ManifestError> {
    if value.is_empty()
        || value.len() > 35
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ManifestError::invariant(
            path,
            "must be a bounded BCP-47-style ASCII locale",
        ));
    }
    Ok(())
}
