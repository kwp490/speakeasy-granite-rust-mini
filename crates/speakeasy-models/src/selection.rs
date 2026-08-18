use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::compatibility::resolve_pack;
use crate::{
    Capability, CompatibilityContext, CompatibilityIssue, ExecutionProvider, LicenseNotice, Pack,
    PackRole, RedistributionDecision, TrustedManifest,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactPackRequest<'a> {
    pub id: &'a str,
    pub revision: &'a str,
}

#[derive(Debug)]
pub struct ExactPackSelection<'a> {
    pack: &'a Pack,
}

impl<'a> ExactPackSelection<'a> {
    pub fn pack(&self) -> &'a Pack {
        self.pack
    }

    pub fn capabilities(&self) -> &'a [Capability] {
        self.pack.capabilities()
    }

    pub fn licenses(&self) -> &'a [LicenseNotice] {
        self.pack.licenses()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    CallerSuppliedManifest,
    ManifestNotInstallEligible,
    PackNotFound { id: String, revision: String },
    PackNotInstallEligible { id: String, revision: String },
    RedistributionNotAllowed { components: Vec<String> },
    Incompatible { issues: Vec<CompatibilityIssue> },
}

impl Display for SelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallerSuppliedManifest => formatter
                .write_str("caller-supplied manifest bytes are not a trusted selection root"),
            Self::ManifestNotInstallEligible => {
                formatter.write_str("the trusted manifest does not authorize installation")
            }
            Self::PackNotFound { id, revision } => {
                write!(formatter, "exact pack {id}@{revision} was not found")
            }
            Self::PackNotInstallEligible { id, revision } => {
                write!(
                    formatter,
                    "exact pack {id}@{revision} is not install eligible"
                )
            }
            Self::RedistributionNotAllowed { components } => write!(
                formatter,
                "redistribution is not allowed for components: {}",
                components.join(", ")
            ),
            Self::Incompatible { issues } => {
                write!(
                    formatter,
                    "exact pack is incompatible ({} issue(s))",
                    issues.len()
                )
            }
        }
    }
}

impl Error for SelectionError {}

/// Why selecting the pack for a role and provider did not yield exactly one
/// pack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoleSelectionError {
    NoneAdmitted {
        role: PackRole,
        provider: ExecutionProvider,
    },
    Ambiguous {
        role: PackRole,
        provider: ExecutionProvider,
        ids: Vec<String>,
    },
}

impl Display for RoleSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoneAdmitted { role, provider } => write!(
                formatter,
                "no install-eligible {} pack is admitted for the {} provider",
                role.as_manifest_str(),
                provider.as_manifest_str()
            ),
            Self::Ambiguous {
                role,
                provider,
                ids,
            } => write!(
                formatter,
                "{} install-eligible {} packs are admitted for the {} provider ({}); \
                 the caller must name one",
                ids.len(),
                role.as_manifest_str(),
                provider.as_manifest_str(),
                ids.join(", ")
            ),
        }
    }
}

impl Error for RoleSelectionError {}

impl TrustedManifest {
    /// Select the one install-eligible pack filling `role`.
    ///
    /// Callers used to reach for a pack by writing its id inline and taking the
    /// first `find` hit. With one pack in the manifest that was
    /// indistinguishable from correct. It stops being correct the moment a
    /// second pack exists: `find` would keep returning whichever pack the JSON
    /// array happened to list first, so the engine a dictation ran on would be
    /// decided by array order — no error, no log line, and nothing in a review
    /// diff to show that the meaning of an untouched call site had changed.
    ///
    /// So ambiguity is an error here rather than a silent pick. A manifest that
    /// admits two packs for one role is under-specified, and the only safe
    /// reading of it is to refuse and say which packs collided. This is the
    /// same fail-closed posture the rest of the trust boundary takes: the
    /// manifest already refuses a duplicate id-and-revision pair outright.
    ///
    /// Role is the key rather than id because it is what the call sites
    /// actually mean — the live HUD wants whatever fills `streaming-asr`, not
    /// one specific pack — and because it survives replacing the model, which
    /// is exactly what happened once already.
    ///
    /// **Provider is part of the key, and ambiguity is scoped within it.** The
    /// migration ships two packs in the same role — float on CUDA and int8 on
    /// CPU — which are alternatives for different machines rather than rival
    /// answers to one question. Without the provider in the key those two would
    /// collide as `Ambiguous` and the app would refuse to select anything at
    /// all. With it, "two CUDA streaming packs" is still an error, because that
    /// genuinely is an under-specified manifest.
    ///
    /// This deliberately does **not** fall back from CUDA to CPU. Which provider
    /// a machine should use depends on the GPU probe and, later, on a user
    /// override, and those decisions carry disclosure obligations — a fallback
    /// hidden inside a lookup would silently answer a question the product has
    /// to answer out loud. So the caller names the provider and handles
    /// [`RoleSelectionError::NoneAdmitted`], which on a GPU-preferred path is an
    /// ordinary "not installed" rather than a fault.
    ///
    /// # Errors
    ///
    /// Returns [`RoleSelectionError::NoneAdmitted`] when no install-eligible
    /// pack fills the role on `provider`, and [`RoleSelectionError::Ambiguous`]
    /// when more than one does.
    pub fn select_sole_install_eligible(
        &self,
        role: PackRole,
        provider: ExecutionProvider,
    ) -> Result<&Pack, RoleSelectionError> {
        let mut filling = self.packs().iter().filter(|pack| {
            pack.role() == role
                && pack.runtime().provider() == provider
                && pack.is_install_eligible()
        });
        let first = filling
            .next()
            .ok_or(RoleSelectionError::NoneAdmitted { role, provider })?;
        let mut ids: Vec<String> = filling.map(|pack| pack.id().to_owned()).collect();
        if ids.is_empty() {
            return Ok(first);
        }
        ids.insert(0, first.id().to_owned());
        Err(RoleSelectionError::Ambiguous {
            role,
            provider,
            ids,
        })
    }
}

impl TrustedManifest {
    /// Select one exact admitted pack without aliases, fuzzy matching, or
    /// revision/provider fallback.
    ///
    /// # Errors
    ///
    /// Returns [`SelectionError`] when bytes were caller supplied, the manifest
    /// or pack is not install eligible, the exact identity is absent,
    /// redistribution is not allowed, or the supplied
    /// application/worker/runtime context is incompatible.
    pub fn select_exact<'a>(
        &'a self,
        request: ExactPackRequest<'_>,
        context: &CompatibilityContext,
    ) -> Result<ExactPackSelection<'a>, SelectionError> {
        if !self.has_bundled_trust() {
            return Err(SelectionError::CallerSuppliedManifest);
        }
        if !self.is_install_eligible() {
            return Err(SelectionError::ManifestNotInstallEligible);
        }

        let pack = self
            .packs()
            .iter()
            .find(|pack| pack.id() == request.id && pack.revision() == request.revision)
            .ok_or_else(|| SelectionError::PackNotFound {
                id: request.id.to_owned(),
                revision: request.revision.to_owned(),
            })?;

        if !pack.is_install_eligible() {
            return Err(SelectionError::PackNotInstallEligible {
                id: request.id.to_owned(),
                revision: request.revision.to_owned(),
            });
        }

        let blocked_components: Vec<String> = pack
            .licenses()
            .iter()
            .filter(|license| license.redistribution() != RedistributionDecision::Allowed)
            .map(|license| license.component().to_owned())
            .collect();
        if !blocked_components.is_empty() {
            return Err(SelectionError::RedistributionNotAllowed {
                components: blocked_components,
            });
        }

        let resolution = resolve_pack(pack, context);
        if !resolution.is_compatible() {
            return Err(SelectionError::Incompatible {
                issues: resolution.issues().to_vec(),
            });
        }

        Ok(ExactPackSelection { pack })
    }
}
