//! Bounded audio capture and worker-processing boundary for `SpeakEasy`.
//!
//! The capture-side API accepts native interleaved samples and performs only
//! conversion plus copies into preallocated atomic storage. Downmixing,
//! resampling, pre-roll, and utterance assembly happen on the worker side.

#![allow(clippy::must_use_candidate)]

mod callback;
mod cpal_capture;
mod cue;
mod format;
mod processing;
mod vad;

pub use callback::{
    CallbackCountersSnapshot, CallbackStamp, CallbackWrite, CallbackWriteStatus, CaptureCallback,
};
pub use cpal_capture::{
    CaptureFault, CaptureIdentity, CpalCaptureError, CpalCaptureRequest, CpalCaptureSession,
    InputDeviceDescriptor, enumerate_input_devices,
};
pub use cue::{RecordingFeedback, play_recording_feedback, render_cue};
pub use format::{
    ChannelPolicy, FormatError, NativeFormatPreference, NativeSampleFormat, NativeStreamCandidate,
    NativeStreamConfig, NegotiationError, NegotiationPreference, negotiate_native_format,
};
pub use processing::{
    AudioPipelineConfig, AudioWorker, PipelineBuildError, ProcessedAudioBlock,
    ProcessedSampleMetadata, ResamplerTailPolicy, UtteranceCompletion, UtteranceIssues,
    UtteranceStateError, WorkerCountersSnapshot, build_audio_pipeline,
};
pub use vad::{
    SILERO_VAD_ARTIFACT_ID, SILERO_VAD_FRAME_SAMPLES, SILERO_VAD_SAMPLE_RATE_HZ, SILERO_VAD_SHA256,
    SileroVadAdapter, VadCalibrationProfile, VadError, VadFrameEvent, VadInference,
    VadQualification,
};

pub use speakeasy_domain::{
    AudioChunkMetadata, AudioDiscontinuity, AudioTimestamp, CaptureStreamId,
};

#[cfg(test)]
mod tests;
