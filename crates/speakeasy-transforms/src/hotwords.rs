use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderIdentity {
    pub pack_id: String,
    pub artifact_revision: String,
    pub runtime: String,
    pub runtime_version: String,
    pub decoder: String,
    pub tokenizer_sha256: String,
    pub locale: String,
    pub manifest_declares_hotwords: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotwordMeasurementStatus {
    Unmeasured,
    Failed,
    Passed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HotwordMeasurement {
    pub evidence_id: String,
    pub exact_decoder: DecoderIdentity,
    pub repetitions: u32,
    pub protected_term_recall_without: Option<f64>,
    pub protected_term_recall_with: Option<f64>,
    pub latency_p95_without_ms: Option<u64>,
    pub latency_p95_with_ms: Option<u64>,
    pub churn_without: Option<f64>,
    pub churn_with: Option<f64>,
    pub status: HotwordMeasurementStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotwordDecision {
    DecoderFeedQualified { evidence_id: String },
    FinalPostprocessOnly { reason: &'static str },
}

/// Allows decoder hotwords only when manifest support and passed measurement
/// refer to the byte-exact decoder/tokenizer identity currently in use.
#[must_use]
pub fn decide_hotword_path(
    active: &DecoderIdentity,
    measurement: Option<&HotwordMeasurement>,
) -> HotwordDecision {
    if !active.manifest_declares_hotwords {
        return HotwordDecision::FinalPostprocessOnly {
            reason: "manifest_does_not_declare_hotwords",
        };
    }
    let Some(measurement) = measurement else {
        return HotwordDecision::FinalPostprocessOnly {
            reason: "hotword_measurement_missing",
        };
    };
    if measurement.status != HotwordMeasurementStatus::Passed
        || measurement.repetitions == 0
        || measurement.exact_decoder != *active
    {
        return HotwordDecision::FinalPostprocessOnly {
            reason: "exact_decoder_measurement_not_passed",
        };
    }
    if measurement.protected_term_recall_with < measurement.protected_term_recall_without
        || measurement.latency_p95_with_ms.is_none()
        || measurement.churn_with.is_none()
    {
        return HotwordDecision::FinalPostprocessOnly {
            reason: "hotword_gain_or_cost_incomplete",
        };
    }
    HotwordDecision::DecoderFeedQualified {
        evidence_id: measurement.evidence_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_decoder() -> DecoderIdentity {
        DecoderIdentity {
            pack_id: "nemotron-3.5-streaming-en-cpu".to_owned(),
            artifact_revision: "560ms-int8-2026-06-11".to_owned(),
            runtime: "sherpa-onnx".to_owned(),
            runtime_version: "1.13.4".to_owned(),
            decoder: "true-online-cache-aware-transducer".to_owned(),
            tokenizer_sha256: "32be3ebfabfff475d64d7829b435f1c7856a1c497907def5c41d54ca9f1eccfd"
                .to_owned(),
            locale: "en-US".to_owned(),
            manifest_declares_hotwords: false,
        }
    }

    #[test]
    fn manifest_not_declaring_hotwords_is_postprocess_only() {
        assert_eq!(
            decide_hotword_path(&fixture_decoder(), None),
            HotwordDecision::FinalPostprocessOnly {
                reason: "manifest_does_not_declare_hotwords"
            }
        );
    }

    #[test]
    fn mismatched_tokenizer_or_unmeasured_gain_never_feeds_decoder() {
        let mut active = fixture_decoder();
        active.manifest_declares_hotwords = true;
        let measurement = HotwordMeasurement {
            evidence_id: "fixture".to_owned(),
            exact_decoder: fixture_decoder(),
            repetitions: 100,
            protected_term_recall_without: Some(0.4),
            protected_term_recall_with: Some(0.9),
            latency_p95_without_ms: Some(10),
            latency_p95_with_ms: Some(11),
            churn_without: Some(1.0),
            churn_with: Some(1.1),
            status: HotwordMeasurementStatus::Passed,
        };
        assert!(matches!(
            decide_hotword_path(&active, Some(&measurement)),
            HotwordDecision::FinalPostprocessOnly { .. }
        ));
    }
}
