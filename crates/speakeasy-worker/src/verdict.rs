//! Whether a Granite transcript is fit to deliver, and what to say when it is not.
//!
//! Granite Speech is a language model, not a transducer: given retained audio
//! it can paraphrase, answer the prompt instead of transcribing, summarise, or
//! return nothing at all. Nothing about its output format distinguishes "an
//! excellent transcript" from "a fluent hallucination" -- both are
//! grammatical, confident text. This module is what tells them apart before
//! one is pasted into the user's focused window.
//!
//! It used to have a second instrument. `FinalDivergence` scored a candidate
//! against the streaming pass's transcript, as a measurement-only reference in
//! offline rigs; it never gated delivery, because a bad reference had once
//! caused a correct Granite transcript to be rejected. With the streaming
//! engine gone there is no second transcript to diverge from, so the
//! reference-free gate below is not merely the sole *delivery* guard, it is
//! the only guard there is. That raises the stakes on [`is_plausible`]: what
//! it misses now reaches the user.
//!
//! The other half of the change is what a rejection *means*. This judgment
//! used to answer "should the caller fall back to re-running the retained
//! audio through the streaming engine?", and every reason below read as a
//! disclosure attached to a transcript that still arrived. There is no second
//! engine to fall back to. A rejection is now the end of the dictation, and
//! the reason is the error the user is shown -- so each one has to name a
//! condition somebody could act on.

use speakeasy_domain::{AsrError, ErrorCode, FinalTranscript};

/// Words, stripped to what a listener would call the same word: casing and
/// punctuation are not the signal this cares about.
pub fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric() || *character == '\'')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Sample rate every part of this pipeline works in. Retained audio arrives
/// at this rate and nothing downstream resamples, so it is also the divisor
/// that turns a sample count back into the seconds [`is_plausible`] weighs.
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// Provisional, and deliberately generous: this gate is a hallucination
/// catcher, not a speech-rate model. A fast speaker must never trip it.
const FREE_WORDS: f64 = 3.0;
const MAX_WORDS_PER_SECOND: f64 = 4.0;

/// Reference-free plausibility gate: no second transcript is needed to notice
/// that text says far more than the audio could plausibly contain. This is
/// what catches "Granite answered the prompt" over a near-silent utterance.
///
/// One-sided on purpose. It bounds how much a transcript may say and says
/// nothing about how little -- a transcript can be too long to be real, but a
/// short one is just a short dictation. The failure it exists for invents
/// words; it does not lose them.
pub fn is_plausible(candidate_words: usize, duration_seconds: f64) -> bool {
    let words = f64::from(u32::try_from(candidate_words).unwrap_or(u32::MAX));
    let allowed = FREE_WORDS + duration_seconds.max(0.0) * MAX_WORDS_PER_SECOND;
    words <= allowed
}

/// Why a dictation produced no delivered text.
///
/// The reason travels with the verdict rather than being re-derived later:
/// once a candidate has been discarded, what was wrong with it is not
/// deducible from what is left.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalSourceReason {
    /// The transcript said more than the audio's duration could hold, which
    /// is the signature of the model answering the prompt rather than
    /// transcribing. The audio is not recoverable from here -- it is the
    /// transcript that was wrong, not the recording.
    GraniteImplausible,
    /// The engine ran and returned no text at all. Distinct from
    /// [`Self::NoSpeech`]: something was heard, and nothing was written.
    GraniteEmpty,
    /// The engine errored. The specific `ErrorCode` travels beside this in
    /// [`GraniteVerdict::failure`] for the diagnostic log.
    GraniteFailed,
    /// Granite is not installed, or its worker is missing from this
    /// installation. The one-exe installer verifies both before it ever
    /// launches the app, so reaching this means the installation was
    /// modified after setup rather than that setup was skipped.
    GraniteUnavailable,
    /// The engine is quarantined after repeated crashes and was not
    /// attempted. Restarting the app clears the quarantine.
    GraniteQuarantined,
    /// The audio held no speech.
    NoSpeech,
}

impl FinalSourceReason {
    /// A stable code for the UI and the diagnostic log. Naming the engine is
    /// fine here -- this is a local, on-device log with no transcript text,
    /// device name, or path in it, unlike the content-bearing fields it must
    /// never carry.
    pub const fn code(self) -> &'static str {
        match self {
            Self::GraniteImplausible => "granite_implausible",
            Self::GraniteEmpty => "granite_empty",
            Self::GraniteFailed => "granite_failed",
            Self::GraniteUnavailable => "granite_unavailable",
            Self::GraniteQuarantined => "granite_quarantined",
            Self::NoSpeech => "no_speech",
        }
    }
}

/// The outcome of judging one Granite pass.
///
/// `delivered` and `reason` are mutually exclusive and exhaustive: exactly one
/// is `Some`. The candidate is kept alongside even when it was rejected,
/// because the diagnostic log records *that* a candidate was discarded and the
/// caller must not have to re-run anything to find out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraniteVerdict {
    /// What the engine returned, whether or not it was fit to deliver.
    pub candidate: Option<FinalTranscript>,
    /// The transcript to paste. `None` means the dictation failed.
    pub delivered: Option<FinalTranscript>,
    /// The engine's own error code, when it errored rather than returned.
    pub failure: Option<ErrorCode>,
    /// Why nothing was delivered. `None` exactly when `delivered` is `Some`.
    pub reason: Option<FinalSourceReason>,
}

/// Judges the one pass this app has.
///
/// Stateless, and it never re-runs anything itself -- it turns "what the
/// engine did" into "what the user gets", and on rejection names the
/// condition to report.
pub fn judge_granite_pass(pass: Result<Option<FinalTranscript>, AsrError>) -> GraniteVerdict {
    let (candidate, failure) = match pass {
        Ok(candidate) => (candidate, None),
        Err(error) => (None, Some(error.code)),
    };
    let (delivered, reason) = decide(candidate.as_ref(), failure);
    GraniteVerdict {
        candidate,
        delivered,
        failure,
        reason,
    }
}

fn decide(
    candidate: Option<&FinalTranscript>,
    failure: Option<ErrorCode>,
) -> (Option<FinalTranscript>, Option<FinalSourceReason>) {
    if let Some(code) = failure {
        let reason = if code == ErrorCode::EngineQuarantined {
            FinalSourceReason::GraniteQuarantined
        } else if code == ErrorCode::NoSpeechDetected {
            FinalSourceReason::NoSpeech
        } else {
            FinalSourceReason::GraniteFailed
        };
        return (None, Some(reason));
    }

    let Some(candidate) = candidate else {
        return (None, Some(FinalSourceReason::GraniteUnavailable));
    };

    if candidate.text.is_empty() {
        return (None, Some(FinalSourceReason::GraniteEmpty));
    }

    let candidate_words = tokens(&candidate.text).len();
    let duration_seconds =
        f64::from(u32::try_from(candidate.metrics.input_samples).unwrap_or(u32::MAX))
            / f64::from(SAMPLE_RATE_HZ);
    if !is_plausible(candidate_words, duration_seconds) {
        return (None, Some(FinalSourceReason::GraniteImplausible));
    }

    (Some(candidate.clone()), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_error;
    use speakeasy_domain::{FinalAsrMetrics, SessionId, TranscriptProvenance};

    fn transcript(text: &str, input_samples: usize) -> FinalTranscript {
        FinalTranscript {
            session_id: SessionId::from_bytes([9; 16]),
            raw_text: text.to_owned(),
            text: text.to_owned(),
            provenance: TranscriptProvenance::FinalizedStream,
            metrics: FinalAsrMetrics {
                input_samples,
                ..FinalAsrMetrics::default()
            },
        }
    }

    #[test]
    fn tokens_fold_case_and_strip_punctuation_but_keep_apostrophes() {
        assert_eq!(
            tokens("Ever tried? EVER failed. Don't matter!"),
            vec!["ever", "tried", "ever", "failed", "don't", "matter"]
        );
        assert_eq!(tokens("   "), Vec::<String>::new());
    }

    #[test]
    fn granite_answering_the_prompt_over_near_silence_is_implausible() {
        assert!(!is_plausible(20, 1.2));
    }

    #[test]
    fn real_speech_at_a_normal_pace_is_plausible() {
        assert!(is_plausible(24, 8.0));
    }

    #[test]
    fn every_reason_has_a_distinct_stable_code() {
        let reasons = [
            FinalSourceReason::GraniteImplausible,
            FinalSourceReason::GraniteEmpty,
            FinalSourceReason::GraniteFailed,
            FinalSourceReason::GraniteUnavailable,
            FinalSourceReason::GraniteQuarantined,
            FinalSourceReason::NoSpeech,
        ];
        let codes: Vec<&str> = reasons.iter().map(|reason| reason.code()).collect();
        for code in &codes {
            assert!(!code.is_empty());
            assert_eq!(codes.iter().filter(|other| *other == code).count(), 1);
        }
    }

    #[test]
    fn a_plausible_transcript_is_delivered_with_no_reason() {
        let granite = transcript("ever tried ever failed", 32_000);
        let verdict = judge_granite_pass(Ok(Some(granite.clone())));
        assert_eq!(verdict.delivered, Some(granite.clone()));
        assert_eq!(verdict.candidate, Some(granite));
        assert_eq!(verdict.reason, None);
        assert_eq!(verdict.failure, None);
    }

    #[test]
    fn an_engine_failure_delivers_nothing_and_names_the_code() {
        let verdict = judge_granite_pass(Err(domain_error(ErrorCode::DeadlineExceeded)));
        assert_eq!(verdict.delivered, None);
        assert_eq!(verdict.failure, Some(ErrorCode::DeadlineExceeded));
        assert_eq!(verdict.reason, Some(FinalSourceReason::GraniteFailed));
    }

    #[test]
    fn quarantine_and_silence_are_reported_as_themselves_not_as_a_generic_failure() {
        // Both arrive as engine errors, and both have something specific and
        // actionable to say -- restart the app, or speak. Folding either into
        // `GraniteFailed` would report a fixable condition as a crash.
        let quarantined = judge_granite_pass(Err(domain_error(ErrorCode::EngineQuarantined)));
        assert_eq!(
            quarantined.reason,
            Some(FinalSourceReason::GraniteQuarantined)
        );

        let silence = judge_granite_pass(Err(domain_error(ErrorCode::NoSpeechDetected)));
        assert_eq!(silence.reason, Some(FinalSourceReason::NoSpeech));
    }

    #[test]
    fn an_absent_engine_is_distinguished_from_an_engine_that_returned_nothing() {
        // `Ok(None)` is "Granite was never run"; an empty transcript is
        // "Granite ran and wrote nothing". Under the old two-engine design
        // both merely picked the fallback and the difference was cosmetic.
        // Now they are what the user is told, and they call for different
        // things -- reinstall versus try again.
        let unavailable = judge_granite_pass(Ok(None));
        assert_eq!(unavailable.delivered, None);
        assert_eq!(
            unavailable.reason,
            Some(FinalSourceReason::GraniteUnavailable)
        );

        let mut blank = transcript("real speech", 32_000);
        blank.text = String::new();
        let empty = judge_granite_pass(Ok(Some(blank)));
        assert_eq!(empty.delivered, None);
        assert_eq!(empty.reason, Some(FinalSourceReason::GraniteEmpty));
    }

    #[test]
    fn a_transcript_longer_than_its_audio_could_hold_is_rejected_but_kept() {
        // Two seconds of audio cannot carry sixty words. The candidate is
        // still returned beside the verdict: the log records that a candidate
        // existed and was discarded, which is a different event from the
        // engine never having produced one.
        let words = "word ".repeat(60);
        let hallucination = transcript(words.trim(), 32_000);
        let verdict = judge_granite_pass(Ok(Some(hallucination.clone())));
        assert_eq!(verdict.delivered, None);
        assert_eq!(verdict.reason, Some(FinalSourceReason::GraniteImplausible));
        assert_eq!(verdict.candidate, Some(hallucination));
    }
}
