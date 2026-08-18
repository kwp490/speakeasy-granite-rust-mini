use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    DictionaryEntry, DictionarySet, DictionaryValidationError, Snippet, SnippetError, SnippetSet,
};

pub const PERSONALIZATION_SCHEMA_VERSION: u16 = 1;
const MAX_IMPORT_BYTES: usize = 1_048_576;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalizationBundle {
    pub schema_version: u16,
    pub transform_pipeline_version: u16,
    pub dictionary: Vec<DictionaryEntry>,
    pub snippets: Vec<Snippet>,
    #[serde(default)]
    pub contacts: Option<serde_json::Value>,
}

impl Default for PersonalizationBundle {
    fn default() -> Self {
        Self {
            schema_version: PERSONALIZATION_SCHEMA_VERSION,
            transform_pipeline_version: crate::TRANSFORM_PIPELINE_VERSION,
            dictionary: Vec::new(),
            snippets: Vec::new(),
            contacts: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPolicy {
    KeepExisting,
    ReplaceExisting,
    RenameImported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictKind {
    DictionaryId,
    DictionaryMatch,
    SnippetId,
    SnippetName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportConflict {
    pub kind: ConflictKind,
    pub existing_id: String,
    pub imported_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportPreview {
    pub fingerprint_sha256: String,
    pub dictionary_count: usize,
    pub snippet_count: usize,
    pub conflicts: Vec<ImportConflict>,
    pub contacts_imported: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportError {
    Oversized,
    InvalidJson,
    TooNew(u64),
    UnsupportedVersion,
    ContactsDisabled,
    InvalidDictionary(DictionaryValidationError),
    InvalidSnippet(SnippetError),
    UnresolvedConflicts,
    PreviewMismatch,
}

#[derive(Clone, Debug)]
pub struct ImportPlan {
    imported: PersonalizationBundle,
    preview: ImportPreview,
}

impl ImportPlan {
    /// Parses and validates one bounded inert personalization document.
    ///
    /// # Errors
    ///
    /// Returns an import error for malformed, oversized, too-new, executable,
    /// contacts-bearing, or otherwise invalid content.
    pub fn parse(bytes: &[u8], existing: &PersonalizationBundle) -> Result<Self, ImportError> {
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(ImportError::Oversized);
        }
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ImportError::InvalidJson)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or(ImportError::InvalidJson)?;
        if version > u64::from(PERSONALIZATION_SCHEMA_VERSION) {
            return Err(ImportError::TooNew(version));
        }
        if version != u64::from(PERSONALIZATION_SCHEMA_VERSION) {
            return Err(ImportError::UnsupportedVersion);
        }
        let imported: PersonalizationBundle =
            serde_json::from_value(value).map_err(|_| ImportError::InvalidJson)?;
        validate_bundle(&imported)?;
        validate_bundle(existing)?;
        if imported.contacts.is_some() {
            return Err(ImportError::ContactsDisabled);
        }
        let conflicts = conflicts(existing, &imported);
        let preview = ImportPreview {
            fingerprint_sha256: hex_digest(bytes),
            dictionary_count: imported.dictionary.len(),
            snippet_count: imported.snippets.len(),
            conflicts,
            contacts_imported: false,
        };
        Ok(Self { imported, preview })
    }

    #[must_use]
    pub fn preview(&self) -> &ImportPreview {
        &self.preview
    }

    /// Builds the complete post-import value after a matching preview.
    ///
    /// # Errors
    ///
    /// Returns an import error when the preview fingerprint does not match or
    /// the merged result violates dictionary/snippet/version constraints.
    pub fn commit(
        &self,
        existing: &PersonalizationBundle,
        fingerprint: &str,
        policy: ImportPolicy,
    ) -> Result<PersonalizationBundle, ImportError> {
        if fingerprint != self.preview.fingerprint_sha256 {
            return Err(ImportError::PreviewMismatch);
        }
        if !self.preview.conflicts.is_empty() && policy == ImportPolicy::KeepExisting {
            // Keep is a resolution, not an implicit overwrite.
        }
        let mut result = existing.clone();
        merge_dictionary(&mut result.dictionary, &self.imported.dictionary, policy);
        merge_snippets(&mut result.snippets, &self.imported.snippets, policy);
        validate_bundle(&result)?;
        Ok(result)
    }
}

fn validate_bundle(bundle: &PersonalizationBundle) -> Result<(), ImportError> {
    if bundle.schema_version != PERSONALIZATION_SCHEMA_VERSION
        || bundle.transform_pipeline_version != crate::TRANSFORM_PIPELINE_VERSION
    {
        return Err(ImportError::UnsupportedVersion);
    }
    if bundle.contacts.is_some() {
        return Err(ImportError::ContactsDisabled);
    }
    DictionarySet::new(bundle.dictionary.clone()).map_err(ImportError::InvalidDictionary)?;
    SnippetSet::new(bundle.snippets.clone()).map_err(ImportError::InvalidSnippet)?;
    Ok(())
}

fn conflicts(
    existing: &PersonalizationBundle,
    imported: &PersonalizationBundle,
) -> Vec<ImportConflict> {
    let mut result = Vec::new();
    for incoming in &imported.dictionary {
        for current in &existing.dictionary {
            let kind = if current.id == incoming.id {
                Some(ConflictKind::DictionaryId)
            } else if current.locale.eq_ignore_ascii_case(&incoming.locale)
                && current.source.to_lowercase() == incoming.source.to_lowercase()
                && current.case_policy == incoming.case_policy
                && current.boundary_policy == incoming.boundary_policy
            {
                Some(ConflictKind::DictionaryMatch)
            } else {
                None
            };
            if let Some(kind) = kind {
                result.push(ImportConflict {
                    kind,
                    existing_id: current.id.clone(),
                    imported_id: incoming.id.clone(),
                });
            }
        }
    }
    for incoming in &imported.snippets {
        for current in &existing.snippets {
            let kind = if current.id == incoming.id {
                Some(ConflictKind::SnippetId)
            } else if current.name.eq_ignore_ascii_case(&incoming.name) {
                Some(ConflictKind::SnippetName)
            } else {
                None
            };
            if let Some(kind) = kind {
                result.push(ImportConflict {
                    kind,
                    existing_id: current.id.clone(),
                    imported_id: incoming.id.clone(),
                });
            }
        }
    }
    result
}

fn merge_dictionary(
    existing: &mut Vec<DictionaryEntry>,
    imported: &[DictionaryEntry],
    policy: ImportPolicy,
) {
    let mut used = existing
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    for incoming in imported {
        let matching = existing.iter().position(|current| {
            current.id == incoming.id
                || (current.locale.eq_ignore_ascii_case(&incoming.locale)
                    && current.source.to_lowercase() == incoming.source.to_lowercase()
                    && current.case_policy == incoming.case_policy
                    && current.boundary_policy == incoming.boundary_policy)
        });
        match (matching, policy) {
            (Some(_), ImportPolicy::KeepExisting) => {}
            (Some(index), ImportPolicy::ReplaceExisting) => existing[index] = incoming.clone(),
            (Some(_), ImportPolicy::RenameImported) => {
                let mut renamed = incoming.clone();
                renamed.id = unique_id(&incoming.id, &mut used);
                existing.push(renamed);
            }
            (None, _) => {
                used.insert(incoming.id.clone());
                existing.push(incoming.clone());
            }
        }
    }
}

fn merge_snippets(existing: &mut Vec<Snippet>, imported: &[Snippet], policy: ImportPolicy) {
    let mut ids = existing
        .iter()
        .map(|snippet| snippet.id.clone())
        .collect::<BTreeSet<_>>();
    let mut names = existing
        .iter()
        .map(|snippet| (snippet.name.to_lowercase(), snippet.id.clone()))
        .collect::<BTreeMap<_, _>>();
    for incoming in imported {
        let matching = existing.iter().position(|current| {
            current.id == incoming.id || current.name.eq_ignore_ascii_case(&incoming.name)
        });
        match (matching, policy) {
            (Some(_), ImportPolicy::KeepExisting) => {}
            (Some(index), ImportPolicy::ReplaceExisting) => existing[index] = incoming.clone(),
            (Some(_), ImportPolicy::RenameImported) => {
                let mut renamed = incoming.clone();
                renamed.id = unique_id(&incoming.id, &mut ids);
                renamed.name = unique_name(&incoming.name, &mut names);
                existing.push(renamed);
            }
            (None, _) => {
                ids.insert(incoming.id.clone());
                names.insert(incoming.name.to_lowercase(), incoming.id.clone());
                existing.push(incoming.clone());
            }
        }
    }
}

fn unique_id(base: &str, used: &mut BTreeSet<String>) -> String {
    for number in 1..=1_024 {
        let candidate = format!("{base}-imported-{number}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{base}-imported-overflow")
}

fn unique_name(base: &str, used: &mut BTreeMap<String, String>) -> String {
    for number in 1..=256 {
        let suffix = format!("-{number}");
        let keep = 64usize.saturating_sub(suffix.len());
        let mut prefix = base.chars().take(keep).collect::<String>();
        prefix.push_str(&suffix);
        if let std::collections::btree_map::Entry::Vacant(entry) = used.entry(prefix.to_lowercase())
        {
            entry.insert(String::new());
            return prefix;
        }
    }
    "imported-overflow".to_owned()
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoundaryPolicy, CasePolicy, DictionaryOrigin};

    fn entry(id: &str, source: &str, replacement: &str) -> DictionaryEntry {
        DictionaryEntry {
            id: id.to_owned(),
            locale: "en-US".to_owned(),
            source: source.to_owned(),
            replacement: replacement.to_owned(),
            case_policy: CasePolicy::Exact,
            boundary_policy: BoundaryPolicy::UnicodeWord,
            origin: DictionaryOrigin::UserEntry,
            precedence: 0,
            protected: true,
            enabled: true,
        }
    }

    #[test]
    fn hostile_oversized_too_new_contacts_and_unknown_fields_fail_closed() {
        let existing = PersonalizationBundle::default();
        assert!(matches!(
            ImportPlan::parse(&vec![b'x'; MAX_IMPORT_BYTES + 1], &existing),
            Err(ImportError::Oversized)
        ));
        assert!(matches!(
            ImportPlan::parse(br#"{"schema_version":99}"#, &existing),
            Err(ImportError::TooNew(99))
        ));
        let contacts = br#"{"schema_version":1,"transform_pipeline_version":1,"dictionary":[],"snippets":[],"contacts":[]}"#;
        assert!(matches!(
            ImportPlan::parse(contacts, &existing),
            Err(ImportError::ContactsDisabled)
        ));
        let executable = br#"{"schema_version":1,"transform_pipeline_version":1,"dictionary":[],"snippets":[],"command":"calc"}"#;
        assert!(matches!(
            ImportPlan::parse(executable, &existing),
            Err(ImportError::InvalidJson)
        ));
    }

    #[test]
    fn conflict_preview_is_required_and_commit_is_atomic_value_construction() {
        let existing = PersonalizationBundle {
            dictionary: vec![entry("one", "open ai", "OpenAI")],
            ..PersonalizationBundle::default()
        };
        let imported = PersonalizationBundle {
            dictionary: vec![entry("two", "open ai", "Open AI")],
            ..PersonalizationBundle::default()
        };
        let bytes = serde_json::to_vec(&imported).unwrap();
        let plan = ImportPlan::parse(&bytes, &existing).unwrap();
        assert_eq!(plan.preview().conflicts.len(), 1);
        assert_eq!(
            plan.commit(&existing, "wrong", ImportPolicy::ReplaceExisting),
            Err(ImportError::PreviewMismatch)
        );
        assert_eq!(existing.dictionary[0].replacement, "OpenAI");
        let committed = plan
            .commit(
                &existing,
                &plan.preview().fingerprint_sha256,
                ImportPolicy::ReplaceExisting,
            )
            .unwrap();
        assert_eq!(committed.dictionary[0].replacement, "Open AI");
    }

    #[test]
    fn action_snippet_import_is_rejected_without_execution() {
        let bytes = br#"{"schema_version":1,"transform_pipeline_version":1,"dictionary":[],"snippets":[{"id":"x","name":"x","body":"<enter>","enabled":true}],"contacts":null}"#;
        assert!(matches!(
            ImportPlan::parse(bytes, &PersonalizationBundle::default()),
            Err(ImportError::InvalidSnippet(
                SnippetError::ForbiddenPlaceholder(_)
            ))
        ));
    }
}
