use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::{
    DictionarySet, ReplacementProvenance, RuleCleanupConfig, SnippetSet, TriggerDisposition,
    apply_rule_cleanup,
};

pub const TRANSFORM_PIPELINE_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineMode {
    PlainText,
    SentenceCase,
    PreserveSpokenWording,
    Literal,
    Code,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatFeature {
    UnicodeNfc,
    SentenceCapitalization,
    Number,
    Date,
    Time,
    Currency,
    Measurement,
    List,
    SpokenPunctuation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatFeatureStatus {
    Qualified,
    DisabledUnqualified,
    BypassedByMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleQualification {
    pub requested_locale: String,
    pub effective_locale: Option<String>,
    pub label: &'static str,
    pub features: Vec<(FormatFeature, FormatFeatureStatus)>,
}

impl LocaleQualification {
    #[must_use]
    pub fn for_locale(locale: &str, mode: PipelineMode) -> Self {
        let english = locale.eq_ignore_ascii_case("en-US") || locale.eq_ignore_ascii_case("en_US");
        let bypass = matches!(
            mode,
            PipelineMode::Literal | PipelineMode::Code | PipelineMode::Terminal
        );
        let mut features = vec![
            (
                FormatFeature::UnicodeNfc,
                if english && !bypass {
                    FormatFeatureStatus::Qualified
                } else if bypass {
                    FormatFeatureStatus::BypassedByMode
                } else {
                    FormatFeatureStatus::DisabledUnqualified
                },
            ),
            (
                FormatFeature::SentenceCapitalization,
                if english && mode == PipelineMode::SentenceCase {
                    FormatFeatureStatus::Qualified
                } else if bypass {
                    FormatFeatureStatus::BypassedByMode
                } else {
                    FormatFeatureStatus::DisabledUnqualified
                },
            ),
        ];
        for feature in [
            FormatFeature::Number,
            FormatFeature::Date,
            FormatFeature::Time,
            FormatFeature::Currency,
            FormatFeature::Measurement,
            FormatFeature::List,
            FormatFeature::SpokenPunctuation,
        ] {
            features.push((
                feature,
                if bypass {
                    FormatFeatureStatus::BypassedByMode
                } else {
                    FormatFeatureStatus::DisabledUnqualified
                },
            ));
        }
        Self {
            requested_locale: locale.to_owned(),
            effective_locale: english.then(|| "en-US".to_owned()),
            label: if english {
                "qualified_en_us_limited"
            } else {
                "unqualified_identity"
            },
            features,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageKind {
    EngineNormalization,
    ExplicitDictionary,
    LocaleFormatting,
    SnippetResolution,
    RuleCleanup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageRecord {
    pub kind: StageKind,
    pub version: u16,
    pub input: String,
    pub output: String,
    pub changed: bool,
    pub provenance_codes: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct PipelineRequest<'a> {
    pub text: &'a str,
    pub locale: &'a str,
    pub mode: PipelineMode,
    pub utterance_final: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineResult {
    pub text: String,
    pub qualification: LocaleQualification,
    pub records: Vec<StageRecord>,
    pub replacements: Vec<ReplacementProvenance>,
    pub snippet_disposition: TriggerDisposition,
}

#[derive(Clone, Debug)]
pub struct TransformPipeline {
    dictionary: DictionarySet,
    snippets: SnippetSet,
}

impl TransformPipeline {
    #[must_use]
    pub const fn new(dictionary: DictionarySet, snippets: SnippetSet) -> Self {
        Self {
            dictionary,
            snippets,
        }
    }

    #[must_use]
    pub fn apply(&self, request: PipelineRequest<'_>) -> PipelineResult {
        self.apply_with_cleanup(request, RuleCleanupConfig::default())
    }

    #[must_use]
    pub fn apply_with_cleanup(
        &self,
        request: PipelineRequest<'_>,
        cleanup: RuleCleanupConfig,
    ) -> PipelineResult {
        let qualification = LocaleQualification::for_locale(request.locale, request.mode);
        let mut text = request.text.to_owned();
        let mut records = Vec::new();

        let normalized = if qualification.effective_locale.is_some() && !literal_mode(request.mode)
        {
            text.nfc().collect()
        } else {
            text.clone()
        };
        records.push(record(
            StageKind::EngineNormalization,
            &text,
            &normalized,
            vec![qualification.label.to_owned()],
        ));
        text = normalized;

        let (dictionary_output, replacements) = if literal_mode(request.mode) {
            (text.clone(), Vec::new())
        } else {
            self.dictionary.apply(&text, request.locale)
        };
        records.push(record(
            StageKind::ExplicitDictionary,
            &text,
            &dictionary_output,
            replacements
                .iter()
                .map(|replacement| format!("dictionary:{}", replacement.entry_id))
                .collect(),
        ));
        text = dictionary_output;

        let formatted = format_locale(&text, request.mode, &qualification);
        records.push(record(
            StageKind::LocaleFormatting,
            &text,
            &formatted,
            vec![qualification.label.to_owned()],
        ));
        text = formatted;

        let resolution =
            self.snippets
                .resolve(&text, request.utterance_final, literal_mode(request.mode));
        records.push(record(
            StageKind::SnippetResolution,
            &text,
            &resolution.text,
            vec![format!("snippet:{:?}", resolution.disposition)],
        ));
        text = resolution.text;

        let cleaned =
            apply_rule_cleanup(&text, request.locale, literal_mode(request.mode), cleanup);
        records.push(record(
            StageKind::RuleCleanup,
            &text,
            &cleaned.text,
            cleaned.provenance_codes,
        ));
        text = cleaned.text;

        PipelineResult {
            text,
            qualification,
            records,
            replacements,
            snippet_disposition: resolution.disposition,
        }
    }
}

fn literal_mode(mode: PipelineMode) -> bool {
    matches!(
        mode,
        PipelineMode::Literal | PipelineMode::Code | PipelineMode::Terminal
    )
}

fn format_locale(input: &str, mode: PipelineMode, qualification: &LocaleQualification) -> String {
    if qualification.effective_locale.as_deref() != Some("en-US")
        || mode != PipelineMode::SentenceCase
    {
        return input.to_owned();
    }
    let byte_index = input.len() - input.trim_start().len();
    let Some(character) = input[byte_index..].chars().next() else {
        return input.to_owned();
    };
    if !character.is_alphabetic() || character.is_uppercase() {
        return input.to_owned();
    }
    let mut output = String::with_capacity(input.len());
    output.push_str(&input[..byte_index]);
    output.extend(character.to_uppercase());
    output.push_str(&input[byte_index + character.len_utf8()..]);
    output
}

fn record(
    kind: StageKind,
    input: &str,
    output: &str,
    provenance_codes: Vec<String>,
) -> StageRecord {
    StageRecord {
        kind,
        version: TRANSFORM_PIPELINE_VERSION,
        input: input.to_owned(),
        output: output.to_owned(),
        changed: input != output,
        provenance_codes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoundaryPolicy, CasePolicy, DictionaryEntry, DictionaryOrigin, Snippet};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct GoldenCorpus {
        cases: Vec<GoldenCase>,
    }

    #[derive(Deserialize)]
    struct GoldenCase {
        #[serde(default)]
        locale: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        input: String,
        #[serde(default)]
        expected: Option<String>,
    }

    fn pipeline() -> TransformPipeline {
        TransformPipeline::new(
            DictionarySet::new(vec![DictionaryEntry {
                id: "proper".to_owned(),
                locale: "en-US".to_owned(),
                source: "open ai".to_owned(),
                replacement: "OpenAI".to_owned(),
                case_policy: CasePolicy::InsensitiveCanonical,
                boundary_policy: BoundaryPolicy::UnicodeWord,
                origin: DictionaryOrigin::ExplicitCorrection,
                precedence: 100,
                protected: true,
                enabled: true,
            }])
            .unwrap(),
            SnippetSet::new(vec![Snippet {
                id: "sig".to_owned(),
                name: "signature".to_owned(),
                body: "Regards,\nAda".to_owned(),
                enabled: true,
            }])
            .unwrap(),
        )
    }

    #[test]
    fn exact_transform_order_and_idempotence() {
        let pipeline = pipeline();
        let request = PipelineRequest {
            text: "open ai is useful",
            locale: "en-US",
            mode: PipelineMode::SentenceCase,
            utterance_final: true,
        };
        let first = pipeline.apply(request);
        assert_eq!(first.text, "OpenAI is useful");
        assert_eq!(
            first
                .records
                .iter()
                .map(|record| record.kind)
                .collect::<Vec<_>>(),
            vec![
                StageKind::EngineNormalization,
                StageKind::ExplicitDictionary,
                StageKind::LocaleFormatting,
                StageKind::SnippetResolution,
                StageKind::RuleCleanup,
            ]
        );
        let second = pipeline.apply(PipelineRequest {
            text: &first.text,
            locale: "en-US",
            mode: PipelineMode::SentenceCase,
            utterance_final: true,
        });
        assert_eq!(second.text, first.text);
    }

    #[test]
    fn en_us_golden_corpus_and_unqualified_identity() {
        let pipeline = pipeline();
        for (input, mode, expected) in [
            ("hello world", PipelineMode::PlainText, "hello world"),
            ("hello world", PipelineMode::SentenceCase, "Hello world"),
            (
                "comma is a word",
                PipelineMode::SentenceCase,
                "Comma is a word",
            ),
            (
                "$12.50 at 3:15",
                PipelineMode::SentenceCase,
                "$12.50 at 3:15",
            ),
            ("1/2/2030 5 km", PipelineMode::PlainText, "1/2/2030 5 km"),
            (
                "one two three",
                PipelineMode::PreserveSpokenWording,
                "one two three",
            ),
        ] {
            let result = pipeline.apply(PipelineRequest {
                text: input,
                locale: "en-US",
                mode,
                utterance_final: true,
            });
            assert_eq!(result.text, expected);
        }
        for locale in ["fr-FR", "de-DE", "es-ES", "pt-BR", "ja-JP", "zh-CN", "ar"] {
            let input = "ｅ́ ١٢ 東京، comma";
            let result = pipeline.apply(PipelineRequest {
                text: input,
                locale,
                mode: PipelineMode::SentenceCase,
                utterance_final: true,
            });
            assert_eq!(result.text, input);
            assert_eq!(result.qualification.label, "unqualified_identity");
        }
    }

    #[test]
    fn snippet_happens_after_dictionary_and_never_on_partial() {
        let pipeline = pipeline();
        let partial = pipeline.apply(PipelineRequest {
            text: "snippet signature",
            locale: "en-US",
            mode: PipelineMode::PlainText,
            utterance_final: false,
        });
        assert_eq!(partial.text, "snippet signature");
        let final_result = pipeline.apply(PipelineRequest {
            text: "snippet signature",
            locale: "en-US",
            mode: PipelineMode::PlainText,
            utterance_final: true,
        });
        assert_eq!(final_result.text, "Regards,\nAda");
    }

    #[test]
    fn literal_code_and_terminal_bypass_all_prose_mutation() {
        let pipeline = pipeline();
        for mode in [
            PipelineMode::Literal,
            PipelineMode::Code,
            PipelineMode::Terminal,
        ] {
            let input = "open ai\nsnippet signature";
            let result = pipeline.apply(PipelineRequest {
                text: input,
                locale: "en-US",
                mode,
                utterance_final: true,
            });
            assert_eq!(result.text, input);
        }
    }

    #[test]
    fn checked_in_golden_corpora_are_authoritative() {
        let pipeline = pipeline();
        let english: GoldenCorpus =
            serde_json::from_str(include_str!("../../../fixtures/transforms/en-US-v1.json"))
                .unwrap();
        for case in english.cases {
            let mode = match case.mode.as_deref().unwrap() {
                "plain_text" => PipelineMode::PlainText,
                "sentence_case" => PipelineMode::SentenceCase,
                "preserve_spoken_wording" => PipelineMode::PreserveSpokenWording,
                "literal" => PipelineMode::Literal,
                "code" => PipelineMode::Code,
                "terminal" => PipelineMode::Terminal,
                _ => panic!("unknown golden mode"),
            };
            let result = pipeline.apply(PipelineRequest {
                text: &case.input,
                locale: "en-US",
                mode,
                utterance_final: true,
            });
            assert_eq!(result.text, case.expected.unwrap());
        }
        let unqualified: GoldenCorpus = serde_json::from_str(include_str!(
            "../../../fixtures/transforms/unqualified-identity-v1.json"
        ))
        .unwrap();
        for case in unqualified.cases {
            let locale = case.locale.unwrap();
            let result = pipeline.apply(PipelineRequest {
                text: &case.input,
                locale: &locale,
                mode: PipelineMode::SentenceCase,
                utterance_final: true,
            });
            assert_eq!(result.text, case.input);
            assert_eq!(result.qualification.label, "unqualified_identity");
        }
    }
}
