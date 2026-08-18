use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

const MAX_SNIPPETS: usize = 256;
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub body: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnippetError {
    TooMany,
    InvalidId,
    InvalidName,
    InvalidBody,
    Collision(String),
    ForbiddenPlaceholder(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerDisposition {
    NotTriggered,
    Expanded,
    EscapedLiteral,
    Cancelled,
    Ambiguous,
    Missing,
    DeferredPartial,
    DisabledInLiteralMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnippetResolution {
    pub text: String,
    pub disposition: TriggerDisposition,
    pub snippet_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SnippetSet {
    snippets: Vec<Snippet>,
}

impl SnippetSet {
    /// Constructs a validated inert snippet set.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identifiers/names/bodies, collisions,
    /// forbidden action placeholders, or excessive entries.
    pub fn new(mut snippets: Vec<Snippet>) -> Result<Self, SnippetError> {
        if snippets.len() > MAX_SNIPPETS {
            return Err(SnippetError::TooMany);
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeMap::new();
        for snippet in &mut snippets {
            snippet.id = snippet.id.nfc().collect();
            snippet.name = snippet.name.nfc().collect();
            snippet.body = snippet.body.nfc().collect();
            if snippet.id.trim().is_empty()
                || snippet.id.len() > 128
                || !ids.insert(snippet.id.clone())
            {
                return Err(SnippetError::InvalidId);
            }
            if !valid_name(&snippet.name) {
                return Err(SnippetError::InvalidName);
            }
            validate_body(&snippet.body)?;
            let key = snippet.name.to_lowercase();
            if snippet.enabled && names.insert(key.clone(), snippet.id.clone()).is_some() {
                return Err(SnippetError::Collision(key));
            }
        }
        snippets.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { snippets })
    }

    #[must_use]
    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    /// Resolves only an entire finalized utterance. The grammar is:
    /// `snippet <name>`, `literal snippet <name>`, or `snippet cancel`.
    #[must_use]
    pub fn resolve(
        &self,
        input: &str,
        utterance_final: bool,
        literal_mode: bool,
    ) -> SnippetResolution {
        let normalized: String = input.nfc().collect();
        if !utterance_final {
            return resolution(normalized, TriggerDisposition::DeferredPartial, None);
        }
        let trimmed = normalized.trim();
        if let Some(name) = trimmed.strip_prefix("literal snippet ") {
            return resolution(
                format!("snippet {}", name.trim()),
                TriggerDisposition::EscapedLiteral,
                None,
            );
        }
        if literal_mode && trimmed.starts_with("snippet ") {
            return resolution(normalized, TriggerDisposition::DisabledInLiteralMode, None);
        }
        let Some(name) = trimmed.strip_prefix("snippet ") else {
            return resolution(normalized, TriggerDisposition::NotTriggered, None);
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("cancel") {
            return resolution(String::new(), TriggerDisposition::Cancelled, None);
        }
        if !valid_name(name) {
            return resolution(normalized, TriggerDisposition::Ambiguous, None);
        }
        let matches = self
            .snippets
            .iter()
            .filter(|snippet| snippet.enabled && snippet.name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [snippet] => resolution(
                snippet.body.clone(),
                TriggerDisposition::Expanded,
                Some(snippet.id.clone()),
            ),
            [] => resolution(normalized, TriggerDisposition::Missing, None),
            _ => resolution(normalized, TriggerDisposition::Ambiguous, None),
        }
    }
}

fn resolution(
    text: String,
    disposition: TriggerDisposition,
    snippet_id: Option<String>,
) -> SnippetResolution {
    SnippetResolution {
        text,
        disposition,
        snippet_id,
    }
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        && name.len() <= 64
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        })
}

fn validate_body(body: &str) -> Result<(), SnippetError> {
    if body.is_empty()
        || body.len() > MAX_BODY_BYTES
        || body.chars().any(|character| {
            character == '\0'
                || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        })
    {
        return Err(SnippetError::InvalidBody);
    }
    let lower = body.to_lowercase();
    let forbidden = [
        "${",
        "{{",
        "<cursor",
        "<caret",
        "<enter",
        "<command",
        "<action",
        "<open_url",
        "<launch",
        "<open_file",
        "[[cursor",
        "[[caret",
        "[[enter",
        "[[command",
        "[[action",
        "[[open_url",
        "[[launch",
        "[[open_file",
    ];
    if let Some(marker) = forbidden.iter().find(|marker| lower.contains(**marker)) {
        return Err(SnippetError::ForbiddenPlaceholder((*marker).to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> SnippetSet {
        SnippetSet::new(vec![Snippet {
            id: "sig".to_owned(),
            name: "signature".to_owned(),
            body: "Regards,\nAda".to_owned(),
            enabled: true,
        }])
        .unwrap()
    }

    #[test]
    fn expansion_occurs_only_at_final_utterance_boundary() {
        assert_eq!(
            set().resolve("snippet signature", false, false).disposition,
            TriggerDisposition::DeferredPartial
        );
        let final_result = set().resolve("snippet signature", true, false);
        assert_eq!(final_result.text, "Regards,\nAda");
        assert_eq!(final_result.disposition, TriggerDisposition::Expanded);
    }

    #[test]
    fn escaping_cancellation_literal_and_terminal_safe_text_are_inert() {
        assert_eq!(
            set().resolve("literal snippet signature", true, false).text,
            "snippet signature"
        );
        assert_eq!(
            set().resolve("snippet cancel", true, false).disposition,
            TriggerDisposition::Cancelled
        );
        assert_eq!(
            set().resolve("snippet signature", true, true).disposition,
            TriggerDisposition::DisabledInLiteralMode
        );
        assert!(
            !set()
                .resolve("snippet signature", true, false)
                .text
                .ends_with('\n')
        );
    }

    #[test]
    fn every_action_placeholder_family_is_rejected() {
        for body in [
            "${cursor}",
            "{{action}}",
            "<cursor>",
            "<caret:left>",
            "<enter>",
            "<command:dir>",
            "<action:x>",
            "<open_url:https://example.test>",
            "<launch:calc>",
            "<open_file:c:\\x>",
            "[[command:ls]]",
        ] {
            assert!(matches!(
                SnippetSet::new(vec![Snippet {
                    id: "x".to_owned(),
                    name: "x".to_owned(),
                    body: body.to_owned(),
                    enabled: true,
                }]),
                Err(SnippetError::ForbiddenPlaceholder(_))
            ));
        }
    }

    #[test]
    fn names_are_exact_and_collision_safe() {
        let snippets = vec![
            Snippet {
                id: "a".to_owned(),
                name: "sig".to_owned(),
                body: "a".to_owned(),
                enabled: true,
            },
            Snippet {
                id: "b".to_owned(),
                name: "SIG".to_owned(),
                body: "b".to_owned(),
                enabled: true,
            },
        ];
        assert!(matches!(
            SnippetSet::new(snippets),
            Err(SnippetError::InvalidName)
        ));
    }
}
