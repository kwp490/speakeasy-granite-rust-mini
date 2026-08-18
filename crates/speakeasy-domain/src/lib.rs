//! Pure domain boundary for `SpeakEasy`.

#![allow(clippy::must_use_candidate)]

mod activation;
mod audio;
mod contracts;
mod coordinator;
mod ids;
mod ports;
mod reliability;
mod state;
mod worker_protocol;

pub use activation::{ActivationEffect, ActivationInput, ActivationReducer, ActivationStopReason};
pub use audio::{
    AUDIO_CONTRACT_SCHEMA_VERSION, AudioChunkMetadata, AudioDiscontinuity, AudioTimestamp,
    CaptureStreamId,
};
pub use contracts::{
    ActivationMode, AppCommand, AppEvent, BoundedQueueConfig, CancelToken, Clock, Deadline,
    DomainError, ErrorCode, FailurePoint, MonotonicTime, SystemClock, bounded_channel,
};
pub use coordinator::{
    DOMAIN_SCHEMA_VERSION, DomainEvent, IngressEvent, Reducer, ReducerDisposition,
};
pub use ids::{CorrelationId, ProducerId, SessionId, UtteranceId};
pub use ports::{
    AsrError, AsrExecution, AsrFeature, AsrLanguage, AsrRequest, AsrStreaming, AsrTask, AudioError,
    AudioSink, AudioSource, BoxFuture, CaptureHandle, CaptureRequest, DeliveryError,
    DeliveryOutcome, DeliveryReceipt, DeliveryRefusal, DeliveryRequest, DeliveryStrategy,
    EndpointReason, EngineCapabilities, EngineSnapshot, ExecutableIdentity, FinalAsr,
    FinalAsrMetrics, FinalAudioJob, FinalTranscript, HypothesisFinality, IntegrityRelationship,
    KeyboardContext, LocaleSelection, PolishEngine, PolishError, PolishLimits, PolishRequest,
    PolishedText, ProtectedSpan, SelectionSnapshot, StreamingAsr, StreamingAsrOptions,
    StreamingAsrRequest, StreamingControlMessage, StreamingControlReceiver, StreamingFinish,
    StreamingHealth, StreamingHealthState, StreamingHypothesis, StreamingMetrics, StreamingResult,
    StreamingResultReceiver, StreamingSendError, StreamingSessionControl, StreamingSessionHandle,
    TargetKind, TargetSnapshot, TextTarget, TimestampedAudioFrame, TranscriptProvenance,
    UiaElementIdentity, UiaPatterns, UtteranceAudio,
};
pub use reliability::{
    DegradationAction, DegradationDecision, DegradationReason, ExclusiveOperation, FaultBoundary,
    FaultScript, FaultScriptError, InjectedFault, LifecycleController, LifecycleEffects,
    LifecycleEvent, OperationArbiter, OperationDisposition, SHUTDOWN_ORDER, ShutdownStep,
    degradation_decision,
};
pub use state::{
    AppReadiness, AppState, DeliveryCapability, EngineState, SessionPhase, VadState,
    transition_allowed,
};
pub use worker_protocol::{
    MAX_AUDIO_SAMPLES_PER_REQUEST, MAX_FRAME_BYTES, ProtocolError, RequestId,
    WORKER_PROTOCOL_VERSION, WorkerClient, WorkerCommand, WorkerErrorCode, WorkerEvent,
    WorkerRequest, WorkerResponse, WorkerSessionId, read_frame, worker_response_is_terminal,
    write_frame,
};
