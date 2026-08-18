use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleCleanupMode {
    #[default]
    Off,
    Conservative,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RuleCleanupConfig {
    pub mode: RuleCleanupMode,
    pub filler_words: bool,
    pub immediate_repetitions: bool,
    pub self_corrections: bool,
    pub spoken_lists: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleCleanupResult {
    pub text: String,
    pub qualification: &'static str,
    pub provenance_codes: Vec<String>,
}

#[must_use]
pub fn apply_rule_cleanup(
    input: &str,
    locale: &str,
    literal: bool,
    config: RuleCleanupConfig,
) -> RuleCleanupResult {
    if literal {
        return unchanged(input, "bypassed_literal");
    }
    if config.mode == RuleCleanupMode::Off {
        return unchanged(input, "disabled");
    }
    if !locale.eq_ignore_ascii_case("en-US") && !locale.eq_ignore_ascii_case("en_US") {
        return unchanged(input, "unqualified_identity");
    }

    let mut text = input.to_owned();
    let mut provenance = Vec::new();
    if config.filler_words {
        let output = remove_fillers(&text);
        if output != text {
            provenance.push("rule:filler_en_us_v1".to_owned());
            text = output;
        }
    }
    if config.immediate_repetitions {
        let output = collapse_repetitions(&text);
        if output != text {
            provenance.push("rule:repetition_en_us_v1".to_owned());
            text = output;
        }
    }
    if config.self_corrections {
        let output = resolve_self_correction(&text);
        if output != text {
            provenance.push("rule:self_correction_en_us_v1".to_owned());
            text = output;
        }
    }
    if config.spoken_lists {
        let output = format_spoken_list(&text);
        if output != text {
            provenance.push("rule:list_en_us_v1".to_owned());
            text = output;
        }
    }
    RuleCleanupResult {
        text,
        qualification: "qualified_en_us_rules_v1",
        provenance_codes: provenance,
    }
}

fn unchanged(input: &str, qualification: &'static str) -> RuleCleanupResult {
    RuleCleanupResult {
        text: input.to_owned(),
        qualification,
        provenance_codes: vec![format!("rule_cleanup:{qualification}")],
    }
}

fn remove_fillers(input: &str) -> String {
    let mut output = Vec::new();
    let mut removed = false;
    for token in input.split_whitespace() {
        let bare = token
            .trim_matches([',', '.', ';', ':'])
            .to_ascii_lowercase();
        if matches!(bare.as_str(), "um" | "uh" | "erm") {
            removed = true;
            continue;
        }
        output.push(token);
    }
    if removed {
        normalize_spaces_and_punctuation(&output.join(" "))
    } else {
        input.to_owned()
    }
}

fn collapse_repetitions(input: &str) -> String {
    if input.contains(['\r', '\n']) {
        return input.to_owned();
    }
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    let mut output: Vec<&str> = Vec::with_capacity(tokens.len());
    for token in tokens {
        let same = output.last().is_some_and(|previous| {
            normalize_token(previous).eq_ignore_ascii_case(&normalize_token(token))
        });
        if !same {
            output.push(token);
        }
    }
    output.join(" ")
}

fn resolve_self_correction(input: &str) -> String {
    const MARKERS: [&str; 2] = [", I mean ", " I mean "];
    for marker in MARKERS {
        if let Some(index) = input.find(marker) {
            let before = &input[..index];
            let replacement = &input[index + marker.len()..];
            if !before.trim().is_empty()
                && !replacement.trim().is_empty()
                && !replacement.contains(" I mean ")
            {
                return replacement.trim_start().to_owned();
            }
        }
    }
    input.to_owned()
}

fn format_spoken_list(input: &str) -> String {
    const PREFIX: &str = "list item ";
    const NEXT: &str = " next item ";
    let Some(rest) = input.strip_prefix(PREFIX) else {
        return input.to_owned();
    };
    let items = rest.split(NEXT).map(str::trim).collect::<Vec<_>>();
    if items.len() < 2 || items.iter().any(|item| item.is_empty()) {
        return input.to_owned();
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_token(token: &str) -> String {
    token
        .trim_matches([',', '.', ';', ':', '!', '?'])
        .to_ascii_lowercase()
}

fn normalize_spaces_and_punctuation(input: &str) -> String {
    input
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" ;", ";")
        .replace(" :", ":")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> RuleCleanupConfig {
        RuleCleanupConfig {
            mode: RuleCleanupMode::Conservative,
            filler_words: true,
            immediate_repetitions: true,
            self_corrections: true,
            spoken_lists: true,
        }
    }

    #[test]
    fn qualified_rules_are_deterministic_and_idempotent() {
        for (input, expected) in [
            ("um hello uh world", "hello world"),
            ("the the result", "the result"),
            ("Tuesday, I mean Wednesday", "Wednesday"),
            ("list item apples next item pears", "- apples\n- pears"),
        ] {
            let first = apply_rule_cleanup(input, "en-US", false, all());
            assert_eq!(first.text, expected);
            let second = apply_rule_cleanup(&first.text, "en-US", false, all());
            assert_eq!(second.text, first.text);
        }
    }

    #[test]
    fn off_literal_and_unqualified_are_exact_identity() {
        let input = "um list item one next item two";
        assert_eq!(
            apply_rule_cleanup(input, "en-US", false, RuleCleanupConfig::default()).text,
            input
        );
        assert_eq!(apply_rule_cleanup(input, "en-US", true, all()).text, input);
        assert_eq!(apply_rule_cleanup(input, "fr-FR", false, all()).text, input);
    }

    #[test]
    fn ordinary_words_and_ambiguous_corrections_remain_literal() {
        for input in [
            "the umami is good",
            "I mean what I say",
            "comma is a word",
            "list items are useful",
        ] {
            assert_eq!(apply_rule_cleanup(input, "en-US", false, all()).text, input);
        }
    }
}
