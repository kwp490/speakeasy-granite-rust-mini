use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

const MAX_ENTRIES: usize = 1_024;
const MAX_TERM_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryOrigin {
    ImportedProfile,
    UserEntry,
    ExplicitCorrection,
}

impl DictionaryOrigin {
    const fn rank(self) -> u8 {
        match self {
            Self::ImportedProfile => 0,
            Self::UserEntry => 1,
            Self::ExplicitCorrection => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CasePolicy {
    Exact,
    InsensitiveCanonical,
    InsensitivePreserve,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    WholeUtterance,
    UnicodeWord,
    Grapheme,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DictionaryEntry {
    pub id: String,
    pub locale: String,
    pub source: String,
    pub replacement: String,
    pub case_policy: CasePolicy,
    pub boundary_policy: BoundaryPolicy,
    pub origin: DictionaryOrigin,
    pub precedence: i16,
    pub protected: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementProvenance {
    pub entry_id: String,
    pub input_start: usize,
    pub input_end: usize,
    pub output_start: usize,
    pub output_end: usize,
    pub protected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DictionaryValidationError {
    TooManyEntries,
    EmptyId,
    DuplicateId,
    InvalidLocale,
    InvalidTerm,
    ConflictingRule,
    Cycle(Vec<String>),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DictionarySet {
    entries: Vec<DictionaryEntry>,
}

impl DictionarySet {
    /// Constructs a validated rule set. Matching uses NFC and Unicode lowercase
    /// keys; stored/output strings remain NFC and are never compatibility-folded.
    ///
    /// # Errors
    ///
    /// Returns a validation error for invalid, conflicting, duplicate, cyclic,
    /// or oversized rules.
    pub fn new(mut entries: Vec<DictionaryEntry>) -> Result<Self, DictionaryValidationError> {
        if entries.len() > MAX_ENTRIES {
            return Err(DictionaryValidationError::TooManyEntries);
        }
        let mut ids = BTreeSet::new();
        let mut match_keys = BTreeSet::new();
        for entry in &mut entries {
            entry.id = entry.id.nfc().collect();
            entry.locale = entry.locale.trim().to_owned();
            entry.source = entry.source.nfc().collect();
            entry.replacement = entry.replacement.nfc().collect();
            if entry.id.trim().is_empty() {
                return Err(DictionaryValidationError::EmptyId);
            }
            if !ids.insert(entry.id.clone()) {
                return Err(DictionaryValidationError::DuplicateId);
            }
            if entry.locale.trim().is_empty() || entry.locale.len() > 35 {
                return Err(DictionaryValidationError::InvalidLocale);
            }
            if !valid_term(&entry.source) || !valid_term(&entry.replacement) {
                return Err(DictionaryValidationError::InvalidTerm);
            }
            let key = (
                locale_key(&entry.locale),
                match_key(&entry.source, entry.case_policy),
                entry.case_policy,
                entry.boundary_policy,
                entry.precedence,
                entry.origin.rank(),
            );
            if entry.enabled && !match_keys.insert(key) {
                return Err(DictionaryValidationError::ConflictingRule);
            }
        }
        detect_cycles(&entries)?;
        entries.sort_by(|left, right| {
            right
                .origin
                .rank()
                .cmp(&left.origin.rank())
                .then_with(|| right.precedence.cmp(&left.precedence))
                .then_with(|| right.source.len().cmp(&left.source.len()))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[DictionaryEntry] {
        &self.entries
    }

    /// Applies non-overlapping rules in deterministic precedence order.
    #[must_use]
    pub fn apply(&self, input: &str, locale: &str) -> (String, Vec<ReplacementProvenance>) {
        let normalized: String = input.nfc().collect();
        let mut candidates = Vec::new();
        for (rule_index, entry) in self.entries.iter().enumerate() {
            if !entry.enabled || locale_key(&entry.locale) != locale_key(locale) {
                continue;
            }
            candidates.extend(
                find_matches(&normalized, entry)
                    .into_iter()
                    .map(|(start, end)| (start, end, rule_index)),
            );
        }
        candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| right.1.cmp(&left.1))
        });

        let mut selected = Vec::new();
        let mut occupied = Vec::<(usize, usize)>::new();
        for candidate @ (start, end, _) in candidates {
            if occupied
                .iter()
                .all(|(used_start, used_end)| end <= *used_start || start >= *used_end)
            {
                occupied.push((start, end));
                selected.push(candidate);
            }
        }
        selected.sort_by_key(|candidate| candidate.0);

        let mut output = String::with_capacity(normalized.len());
        let mut provenance = Vec::new();
        let mut cursor = 0;
        for (start, end, rule_index) in selected {
            output.push_str(&normalized[cursor..start]);
            let entry = &self.entries[rule_index];
            let output_start = output.len();
            let matched = &normalized[start..end];
            let replacement = replacement_for_case(entry, matched);
            output.push_str(&replacement);
            provenance.push(ReplacementProvenance {
                entry_id: entry.id.clone(),
                input_start: start,
                input_end: end,
                output_start,
                output_end: output.len(),
                protected: entry.protected,
            });
            cursor = end;
        }
        output.push_str(&normalized[cursor..]);
        (output, provenance)
    }
}

fn valid_term(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TERM_BYTES
        && !value.chars().any(|character| {
            character == '\0' || (character.is_control() && character != '\n' && character != '\t')
        })
}

fn locale_key(locale: &str) -> String {
    locale.trim().replace('_', "-").to_lowercase()
}

fn match_key(value: &str, policy: CasePolicy) -> String {
    let normalized: String = value.nfc().collect();
    match policy {
        CasePolicy::Exact => normalized,
        CasePolicy::InsensitiveCanonical | CasePolicy::InsensitivePreserve => {
            normalized.to_lowercase()
        }
    }
}

fn detect_cycles(entries: &[DictionaryEntry]) -> Result<(), DictionaryValidationError> {
    let mut edges = BTreeMap::<(String, String), (String, String)>::new();
    let mut ids = BTreeMap::<(String, String), String>::new();
    for entry in entries.iter().filter(|entry| entry.enabled) {
        let locale = locale_key(&entry.locale);
        let source = entry.source.to_lowercase();
        let replacement = entry.replacement.to_lowercase();
        if source == replacement {
            continue;
        }
        edges.insert(
            (locale.clone(), source.clone()),
            (locale.clone(), replacement),
        );
        ids.insert((locale, source), entry.id.clone());
    }
    for start in edges.keys() {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = start.clone();
        while let Some(next) = edges.get(&cursor) {
            if !seen.insert(cursor.clone()) {
                let cycle = path
                    .into_iter()
                    .filter_map(|key| ids.get(&key).cloned())
                    .collect();
                return Err(DictionaryValidationError::Cycle(cycle));
            }
            path.push(cursor);
            cursor = next.clone();
        }
    }
    Ok(())
}

fn find_matches(input: &str, entry: &DictionaryEntry) -> Vec<(usize, usize)> {
    if entry.boundary_policy == BoundaryPolicy::WholeUtterance {
        return (matches_case(input, &entry.source, entry.case_policy)
            && input.len() == entry.source.len())
        .then_some((0, input.len()))
        .into_iter()
        .collect();
    }
    let mut matches = Vec::new();
    match entry.case_policy {
        CasePolicy::Exact => {
            for (start, _) in input.match_indices(&entry.source) {
                let end = start + entry.source.len();
                if boundary_ok(input, start, end, entry.boundary_policy) {
                    matches.push((start, end));
                }
            }
        }
        CasePolicy::InsensitiveCanonical | CasePolicy::InsensitivePreserve => {
            let needle = entry.source.to_lowercase();
            let character_count = entry.source.chars().count();
            for (start, _) in input.char_indices() {
                let end = input[start..]
                    .char_indices()
                    .nth(character_count)
                    .map_or(input.len(), |(offset, _)| start + offset);
                if input[start..end].to_lowercase() == needle
                    && boundary_ok(input, start, end, entry.boundary_policy)
                {
                    matches.push((start, end));
                }
            }
        }
    }
    matches
}

fn matches_case(input: &str, source: &str, policy: CasePolicy) -> bool {
    match policy {
        CasePolicy::Exact => input == source,
        CasePolicy::InsensitiveCanonical | CasePolicy::InsensitivePreserve => {
            input.to_lowercase() == source.to_lowercase()
        }
    }
}

fn boundary_ok(input: &str, start: usize, end: usize, policy: BoundaryPolicy) -> bool {
    match policy {
        BoundaryPolicy::WholeUtterance => start == 0 && end == input.len(),
        BoundaryPolicy::Grapheme => {
            let boundaries = input
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .chain(std::iter::once(input.len()))
                .collect::<BTreeSet<_>>();
            boundaries.contains(&start) && boundaries.contains(&end)
        }
        BoundaryPolicy::UnicodeWord => {
            let before = input[..start].chars().next_back();
            let first = input[start..end].chars().next();
            let last = input[start..end].chars().next_back();
            let after = input[end..].chars().next();
            boundary_pair(before, first) && boundary_pair(last, after)
        }
    }
}

fn boundary_pair(left: Option<char>, right: Option<char>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => is_word(left) != is_word(right),
        _ => true,
    }
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn replacement_for_case(entry: &DictionaryEntry, matched: &str) -> String {
    if entry.case_policy != CasePolicy::InsensitivePreserve {
        return entry.replacement.clone();
    }
    if matched
        .chars()
        .all(|character| !character.is_alphabetic() || character.is_uppercase())
    {
        return entry.replacement.to_uppercase();
    }
    if matched.chars().next().is_some_and(char::is_uppercase)
        && matched
            .chars()
            .skip(1)
            .all(|character| !character.is_alphabetic() || character.is_lowercase())
    {
        let mut graphemes = entry.replacement.graphemes(true);
        return graphemes.next().map_or_else(String::new, |first| {
            format!("{}{}", first.to_uppercase(), graphemes.collect::<String>())
        });
    }
    entry.replacement.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, source: &str, replacement: &str) -> DictionaryEntry {
        DictionaryEntry {
            id: id.to_owned(),
            locale: "en-US".to_owned(),
            source: source.to_owned(),
            replacement: replacement.to_owned(),
            case_policy: CasePolicy::InsensitiveCanonical,
            boundary_policy: BoundaryPolicy::UnicodeWord,
            origin: DictionaryOrigin::ExplicitCorrection,
            precedence: 100,
            protected: true,
            enabled: true,
        }
    }

    #[test]
    fn explicit_proper_name_changes_only_whole_term() {
        let set = DictionarySet::new(vec![entry("n", "open ai", "OpenAI")]).unwrap();
        let (output, provenance) = set.apply("open ai met an open air pilot", "en-US");
        assert_eq!(output, "OpenAI met an open air pilot");
        assert_eq!(provenance.len(), 1);
        assert!(provenance[0].protected);
    }

    #[test]
    fn precedence_longest_case_and_protected_spans_are_deterministic() {
        let mut lower = entry("lower", "granite", "stone");
        lower.origin = DictionaryOrigin::ImportedProfile;
        lower.precedence = 0;
        let mut higher = entry("higher", "granite speech", "Granite Speech");
        higher.case_policy = CasePolicy::InsensitivePreserve;
        let set = DictionarySet::new(vec![lower, higher]).unwrap();
        assert_eq!(
            set.apply("GRANITE SPEECH and granite", "en-US").0,
            "GRANITE SPEECH and stone"
        );
    }

    #[test]
    fn cycles_and_conflicts_fail_closed() {
        let first = entry("a", "alpha", "beta");
        let second = entry("b", "beta", "alpha");
        assert!(matches!(
            DictionarySet::new(vec![first, second]),
            Err(DictionaryValidationError::Cycle(_))
        ));
        let one = entry("a", "alpha", "one");
        let mut two = entry("b", "alpha", "two");
        two.precedence = one.precedence;
        assert_eq!(
            DictionarySet::new(vec![one, two]),
            Err(DictionaryValidationError::ConflictingRule)
        );
    }

    #[test]
    fn unicode_escape_boundaries_cover_cjk_and_rtl() {
        let mut cjk = entry(
            "cjk-escaped",
            "\u{6771}\u{4eac}",
            "\u{6771}\u{4eac}\u{90fd}",
        );
        cjk.locale = "ja".to_owned();
        cjk.boundary_policy = BoundaryPolicy::Grapheme;
        let mut rtl = entry(
            "rtl-escaped",
            "\u{0633}\u{0644}\u{0627}\u{0645}",
            "\u{0633}\u{064e}\u{0644}\u{0627}\u{0645}",
        );
        rtl.locale = "ar".to_owned();
        let set = DictionarySet::new(vec![cjk, rtl]).unwrap();
        assert_eq!(
            set.apply("\u{6771}\u{4eac}\u{3078}", "ja").0,
            "\u{6771}\u{4eac}\u{90fd}\u{3078}"
        );
        assert_eq!(
            set.apply(
                "\u{0633}\u{0644}\u{0627}\u{0645}\u{060c} \u{0639}\u{0627}\u{0644}\u{0645}",
                "ar"
            )
            .0,
            "\u{0633}\u{064e}\u{0644}\u{0627}\u{0645}\u{060c} \u{0639}\u{0627}\u{0644}\u{0645}"
        );
    }

    #[test]
    fn protected_term_postprocess_ab_improves_fixture_recall_without_unrelated_edits() {
        let set = DictionarySet::new(vec![entry("proper-ab", "open ai", "OpenAI")]).unwrap();
        let fixtures = [
            ("open ai", true),
            ("ask open ai today", true),
            ("open ai and rust", true),
            ("an open air market", false),
            ("ordinary unrelated text", false),
        ];
        let baseline_hits = fixtures
            .iter()
            .filter(|(text, expected)| *expected && text.contains("OpenAI"))
            .count();
        let transformed = fixtures
            .iter()
            .map(|(text, _)| set.apply(text, "en-US").0)
            .collect::<Vec<_>>();
        let transformed_hits = transformed
            .iter()
            .zip(fixtures)
            .filter(|(text, (_, expected))| *expected && text.contains("OpenAI"))
            .count();
        assert_eq!(baseline_hits, 0);
        assert_eq!(transformed_hits, 3);
        assert_eq!(transformed[3], "an open air market");
        assert_eq!(transformed[4], "ordinary unrelated text");
    }

    #[test]
    fn unicode_boundaries_cover_cjk_rtl_and_graphemes() {
        let mut cjk = entry("cjk", "東京", "東京都");
        cjk.locale = "ja".to_owned();
        cjk.boundary_policy = BoundaryPolicy::Grapheme;
        let mut rtl = entry("rtl", "سلام", "سَلام");
        rtl.locale = "ar".to_owned();
        let set = DictionarySet::new(vec![cjk, rtl]).unwrap();
        assert_eq!(set.apply("東京へ", "ja").0, "東京都へ");
        assert_eq!(set.apply("سلام، عالم", "ar").0, "سَلام، عالم");
        assert_eq!(set.apply("東京へ", "en-US").0, "東京へ");
    }
}
