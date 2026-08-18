pub const SILERO_VAD_ARTIFACT_ID: &str = "silero-vad-sherpa-asr-models-2025-07-11";
pub const SILERO_VAD_SHA256: &str =
    "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";
pub const SILERO_VAD_SAMPLE_RATE_HZ: u32 = 16_000;
pub const SILERO_VAD_FRAME_SAMPLES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VadQualification {
    Qualified,
    Unqualified,
    Unavailable,
}

impl VadQualification {
    pub const fn automatic_endpointing(self) -> bool {
        matches!(self, Self::Qualified)
    }

    pub const fn manual_stop_available(self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VadCalibrationProfile {
    pub speech_threshold: f32,
    pub soft_speech_threshold: f32,
    pub minimum_speech_frames: u16,
    pub minimum_soft_speech_frames: u16,
    pub minimum_silence_frames: u16,
    pub hangover_frames: u16,
}

impl VadCalibrationProfile {
    pub const PROOF_ONLY: Self = Self {
        speech_threshold: 0.5,
        soft_speech_threshold: 0.35,
        minimum_speech_frames: 8,
        minimum_soft_speech_frames: 12,
        minimum_silence_frames: 16,
        hangover_frames: 3,
    };

    /// # Errors
    ///
    /// Returns [`VadError::InvalidProfile`] for invalid thresholds or frame counts.
    pub fn validate(self) -> Result<Self, VadError> {
        if !(0.0..=1.0).contains(&self.speech_threshold)
            || !(0.0..=self.speech_threshold).contains(&self.soft_speech_threshold)
            || self.minimum_speech_frames == 0
            || self.minimum_soft_speech_frames < self.minimum_speech_frames
            || self.minimum_silence_frames == 0
        {
            return Err(VadError::InvalidProfile);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VadError {
    InvalidProfile,
    InvalidFrameLength { expected: usize, actual: usize },
    InferenceFailed,
    InvalidProbability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VadFrameEvent {
    Unavailable,
    Unqualified,
    Silence,
    SpeechCandidate,
    SpeechStarted,
    Speech,
    Hangover,
    SpeechEnded,
    FalseStart,
}

pub trait VadInference: Send {
    /// # Errors
    ///
    /// Returns [`VadError::InferenceFailed`] when runtime state cannot be reset.
    fn reset(&mut self) -> Result<(), VadError>;

    /// # Errors
    ///
    /// Returns [`VadError::InferenceFailed`] when the pinned runtime cannot infer.
    fn infer_probability(&mut self, samples: &[f32]) -> Result<f32, VadError>;
}

pub struct SileroVadAdapter<I> {
    inference: I,
    qualification: VadQualification,
    profile: VadCalibrationProfile,
    speech_active: bool,
    candidate_frames: u16,
    candidate_has_strong_speech: bool,
    silence_frames: u16,
}

impl<I: VadInference> SileroVadAdapter<I> {
    /// # Errors
    ///
    /// Returns [`VadError::InvalidProfile`] when calibration values are invalid.
    pub fn new(
        inference: I,
        qualification: VadQualification,
        profile: VadCalibrationProfile,
    ) -> Result<Self, VadError> {
        Ok(Self {
            inference,
            qualification,
            profile: profile.validate()?,
            speech_active: false,
            candidate_frames: 0,
            candidate_has_strong_speech: false,
            silence_frames: 0,
        })
    }

    pub const fn qualification(&self) -> VadQualification {
        self.qualification
    }

    /// # Errors
    ///
    /// Propagates an inference-runtime reset failure.
    pub fn reset(&mut self) -> Result<(), VadError> {
        self.inference.reset()?;
        self.speech_active = false;
        self.candidate_frames = 0;
        self.candidate_has_strong_speech = false;
        self.silence_frames = 0;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error for a non-512-sample frame, an inference failure, or a
    /// non-finite/out-of-range probability.
    pub fn accept_frame(&mut self, samples: &[f32]) -> Result<VadFrameEvent, VadError> {
        if samples.len() != SILERO_VAD_FRAME_SAMPLES {
            return Err(VadError::InvalidFrameLength {
                expected: SILERO_VAD_FRAME_SAMPLES,
                actual: samples.len(),
            });
        }
        match self.qualification {
            VadQualification::Unavailable => return Ok(VadFrameEvent::Unavailable),
            VadQualification::Unqualified => return Ok(VadFrameEvent::Unqualified),
            VadQualification::Qualified => {}
        }

        let probability = self.inference.infer_probability(samples)?;
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err(VadError::InvalidProbability);
        }
        if probability >= self.profile.soft_speech_threshold {
            return Ok(self.accept_speech_probability(probability));
        }
        Ok(self.accept_silence())
    }

    fn accept_speech_probability(&mut self, probability: f32) -> VadFrameEvent {
        self.silence_frames = 0;
        if self.speech_active {
            return VadFrameEvent::Speech;
        }
        self.candidate_frames = self.candidate_frames.saturating_add(1);
        self.candidate_has_strong_speech |= probability >= self.profile.speech_threshold;
        let required_frames = if self.candidate_has_strong_speech {
            self.profile.minimum_speech_frames
        } else {
            self.profile.minimum_soft_speech_frames
        };
        if self.candidate_frames >= required_frames {
            self.speech_active = true;
            self.candidate_frames = 0;
            self.candidate_has_strong_speech = false;
            VadFrameEvent::SpeechStarted
        } else {
            VadFrameEvent::SpeechCandidate
        }
    }

    fn accept_silence(&mut self) -> VadFrameEvent {
        if !self.speech_active {
            if self.candidate_frames > 0 {
                self.candidate_frames = 0;
                self.candidate_has_strong_speech = false;
                return VadFrameEvent::FalseStart;
            }
            return VadFrameEvent::Silence;
        }
        self.silence_frames = self.silence_frames.saturating_add(1);
        let endpoint_frames = self
            .profile
            .minimum_silence_frames
            .saturating_add(self.profile.hangover_frames);
        if self.silence_frames >= endpoint_frames {
            self.speech_active = false;
            self.silence_frames = 0;
            VadFrameEvent::SpeechEnded
        } else {
            VadFrameEvent::Hangover
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FirstSampleInference {
        resets: usize,
    }

    impl VadInference for FirstSampleInference {
        fn reset(&mut self) -> Result<(), VadError> {
            self.resets += 1;
            Ok(())
        }

        fn infer_probability(&mut self, samples: &[f32]) -> Result<f32, VadError> {
            Ok(samples[0])
        }
    }

    fn frame(probability: f32) -> [f32; SILERO_VAD_FRAME_SAMPLES] {
        [probability; SILERO_VAD_FRAME_SAMPLES]
    }

    fn adapter() -> SileroVadAdapter<FirstSampleInference> {
        SileroVadAdapter::new(
            FirstSampleInference::default(),
            VadQualification::Qualified,
            VadCalibrationProfile {
                speech_threshold: 0.5,
                soft_speech_threshold: 0.3,
                minimum_speech_frames: 2,
                minimum_soft_speech_frames: 3,
                minimum_silence_frames: 2,
                hangover_frames: 1,
            },
        )
        .unwrap()
    }

    #[test]
    fn qualification_disables_automatic_endpointing_but_never_manual_stop() {
        for qualification in [
            VadQualification::Qualified,
            VadQualification::Unqualified,
            VadQualification::Unavailable,
        ] {
            assert_eq!(
                qualification.automatic_endpointing(),
                qualification == VadQualification::Qualified
            );
            assert!(qualification.manual_stop_available());
        }
    }

    #[test]
    fn silence_noise_soft_speech_and_hangover_follow_profile() {
        let mut vad = adapter();
        assert_eq!(
            vad.accept_frame(&frame(0.1)).unwrap(),
            VadFrameEvent::Silence
        );
        assert_eq!(
            vad.accept_frame(&frame(0.35)).unwrap(),
            VadFrameEvent::SpeechCandidate
        );
        assert_eq!(
            vad.accept_frame(&frame(0.35)).unwrap(),
            VadFrameEvent::SpeechCandidate
        );
        assert_eq!(
            vad.accept_frame(&frame(0.35)).unwrap(),
            VadFrameEvent::SpeechStarted
        );
        assert_eq!(
            vad.accept_frame(&frame(0.2)).unwrap(),
            VadFrameEvent::Hangover
        );
        assert_eq!(
            vad.accept_frame(&frame(0.2)).unwrap(),
            VadFrameEvent::Hangover
        );
        assert_eq!(
            vad.accept_frame(&frame(0.2)).unwrap(),
            VadFrameEvent::SpeechEnded
        );
    }

    #[test]
    fn short_candidate_is_a_false_start_and_reset_clears_state() {
        let mut vad = adapter();
        assert_eq!(
            vad.accept_frame(&frame(0.8)).unwrap(),
            VadFrameEvent::SpeechCandidate
        );
        assert_eq!(
            vad.accept_frame(&frame(0.1)).unwrap(),
            VadFrameEvent::FalseStart
        );
        vad.reset().unwrap();
        assert_eq!(
            vad.accept_frame(&frame(0.1)).unwrap(),
            VadFrameEvent::Silence
        );
    }

    #[test]
    fn unqualified_adapter_does_not_infer_or_emit_endpoint_events() {
        let mut vad = SileroVadAdapter::new(
            FirstSampleInference::default(),
            VadQualification::Unqualified,
            VadCalibrationProfile::PROOF_ONLY,
        )
        .unwrap();
        assert_eq!(
            vad.accept_frame(&frame(0.9)).unwrap(),
            VadFrameEvent::Unqualified
        );
    }
}
