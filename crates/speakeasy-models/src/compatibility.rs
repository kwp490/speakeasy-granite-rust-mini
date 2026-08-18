use semver::Version;

use crate::{Architecture, ExecutionProvider, Pack, Platform, TrustedManifest};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityContext {
    pub application_version: Version,
    pub worker_version: Version,
    pub platform: Platform,
    pub architecture: Architecture,
    pub provider: ExecutionProvider,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibilityIssue {
    ApplicationTooOld {
        minimum: Version,
        actual: Version,
    },
    ApplicationTooNew {
        maximum: Version,
        actual: Version,
    },
    WorkerTooOld {
        minimum: Version,
        actual: Version,
    },
    WorkerTooNew {
        maximum: Version,
        actual: Version,
    },
    PlatformMismatch {
        required: Platform,
        actual: Platform,
    },
    ArchitectureMismatch {
        required: Architecture,
        actual: Architecture,
    },
    ProviderMismatch {
        required: ExecutionProvider,
        actual: ExecutionProvider,
    },
}

#[derive(Debug)]
pub struct CompatibilityResolution<'a> {
    pack: &'a Pack,
    issues: Vec<CompatibilityIssue>,
}

impl<'a> CompatibilityResolution<'a> {
    pub fn pack(&self) -> &'a Pack {
        self.pack
    }

    pub fn issues(&self) -> &[CompatibilityIssue] {
        &self.issues
    }

    pub fn is_compatible(&self) -> bool {
        self.issues.is_empty()
    }
}

impl TrustedManifest {
    pub fn resolve_compatibility<'a>(
        &'a self,
        context: &CompatibilityContext,
    ) -> Vec<CompatibilityResolution<'a>> {
        self.packs()
            .iter()
            .map(|pack| resolve_pack(pack, context))
            .collect()
    }
}

pub(crate) fn resolve_pack<'a>(
    pack: &'a Pack,
    context: &CompatibilityContext,
) -> CompatibilityResolution<'a> {
    let range = pack.compatibility();
    let runtime = pack.runtime();
    let mut issues = Vec::new();

    if context.application_version < *range.minimum_application_version() {
        issues.push(CompatibilityIssue::ApplicationTooOld {
            minimum: range.minimum_application_version().clone(),
            actual: context.application_version.clone(),
        });
    }
    if range
        .maximum_application_version()
        .is_some_and(|maximum| context.application_version > *maximum)
    {
        issues.push(CompatibilityIssue::ApplicationTooNew {
            maximum: range
                .maximum_application_version()
                .cloned()
                .expect("checked"),
            actual: context.application_version.clone(),
        });
    }
    if context.worker_version < *range.minimum_worker_version() {
        issues.push(CompatibilityIssue::WorkerTooOld {
            minimum: range.minimum_worker_version().clone(),
            actual: context.worker_version.clone(),
        });
    }
    if range
        .maximum_worker_version()
        .is_some_and(|maximum| context.worker_version > *maximum)
    {
        issues.push(CompatibilityIssue::WorkerTooNew {
            maximum: range.maximum_worker_version().cloned().expect("checked"),
            actual: context.worker_version.clone(),
        });
    }
    if context.platform != runtime.platform() {
        issues.push(CompatibilityIssue::PlatformMismatch {
            required: runtime.platform(),
            actual: context.platform,
        });
    }
    if context.architecture != runtime.architecture() {
        issues.push(CompatibilityIssue::ArchitectureMismatch {
            required: runtime.architecture(),
            actual: context.architecture,
        });
    }
    if context.provider != runtime.provider() {
        issues.push(CompatibilityIssue::ProviderMismatch {
            required: runtime.provider(),
            actual: context.provider,
        });
    }

    CompatibilityResolution { pack, issues }
}
