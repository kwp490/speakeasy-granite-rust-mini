use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

use crate::{
    CancelToken, CorrelationId, Deadline, DeliveryCapability, DomainError, ErrorCode, SessionId,
    UtteranceId,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureRequest {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub deadline: Deadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSink;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureHandle {
    pub session_id: SessionId,
}

pub type AudioError = DomainError;

pub trait AudioSource: Send + Sync {
    fn start(
        &self,
        request: CaptureRequest,
        sink: AudioSink,
    ) -> BoxFuture<'_, Result<CaptureHandle, AudioError>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineCapabilities {
    pub execution: AsrExecution,
    pub streaming: AsrStreaming,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub language: AsrLanguage,
    pub task: AsrTask,
    pub features: &'static [AsrFeature],
    pub runtime: &'static str,
    pub runtime_abi: &'static str,
    pub provider: &'static str,
    pub artifact_revision: &'static str,
    pub license: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrExecution {
    Local,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrStreaming {
    Offline,
    TrueOnline,
    SimulatedOnline,
    BufferedOnline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrFeature {
    Punctuation,
    Case,
    Timestamps,
    KnownLocale,
    AutoLanguageId,
    Hotwords,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrLanguage {
    English,
    Other(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrTask {
    Transcribe,
    Translate,
}

/// One utterance's ask of the engine.
///
/// **Not `Copy` since `keywords` arrived**, and that is the point rather than a
/// cost: the terms are owned per request, so a caller cannot silently share one
/// dictation's bias with another by letting the request fall out of a `let`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrRequest {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub language: AsrLanguage,
    pub task: AsrTask,
    /// Terms to bias the decode toward, in the order they reach the prompt.
    ///
    /// Empty is the ordinary case and must stay byte-identical to no bias at
    /// all — see `speakeasy_granite::transcribe_prompt_with_keywords`, whose
    /// empty-list branch returns the unmodified instruction so the installer's
    /// pinned smoke transcript remains a control.
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocaleSelection {
    Known(String),
    AutoWithDisclosure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamingAsrOptions {
    pub locale: LocaleSelection,
    pub decoder: String,
    pub hotwords: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamingAsrRequest {
    pub asr: AsrRequest,
    pub options: StreamingAsrOptions,
    pub frame_capacity: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UtteranceAudio {
    pub session_id: SessionId,
    pub sample_rate_hz: u32,
    pub samples: Vec<i16>,
}

/// Engine identity captured when finalized audio enters the inference queue.
/// These values are diagnostic metadata, never user content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub runtime: String,
    pub artifact_id: String,
    pub provider: String,
}

impl EngineSnapshot {
    pub fn new(
        runtime: impl Into<String>,
        artifact_id: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            runtime: runtime.into(),
            artifact_id: artifact_id.into(),
            provider: provider.into(),
        }
    }
}

/// Immutable handoff from capture finalization to the ordered inference
/// consumer. Later coordinator mutations cannot change this job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalAudioJob {
    pub utterance_id: UtteranceId,
    pub audio: UtteranceAudio,
    pub request: AsrRequest,
    pub engine: EngineSnapshot,
}

impl FinalAudioJob {
    pub fn new(
        utterance_id: UtteranceId,
        audio: UtteranceAudio,
        request: AsrRequest,
        engine: EngineSnapshot,
    ) -> Self {
        Self {
            utterance_id,
            audio,
            request,
            engine,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalTranscript {
    pub session_id: SessionId,
    pub raw_text: String,
    pub text: String,
    pub provenance: TranscriptProvenance,
    pub metrics: FinalAsrMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptProvenance {
    FinalizedStream,
    LastValidDraft,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FinalAsrMetrics {
    pub input_samples: usize,
    pub final_segments: usize,
    pub draft_revisions: usize,
}

pub type AsrError = DomainError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimestampedAudioFrame {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub sequence: u64,
    pub first_sample_index: u64,
    pub producer_monotonic_ns: u64,
    pub sample_rate_hz: u32,
    pub samples: Vec<i16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointReason {
    Manual,
    QualifiedVad,
    MaximumDuration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HypothesisFinality {
    Mutable,
    Final,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamingHypothesis {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub revision: u64,
    pub text: String,
    pub finality: HypothesisFinality,
    pub audio_start_sample: u64,
    pub audio_end_sample: u64,
    pub received_monotonic_ns: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamingMetrics {
    pub frames_accepted: u64,
    pub samples_accepted: u64,
    pub hypotheses_accepted: u64,
    pub stale_revisions_rejected: u64,
    pub out_of_order_frames_rejected: u64,
    pub backpressure_events: u64,
    pub endpoints: u64,
    pub finish_requests: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamingHealthState {
    Starting,
    Running,
    Finishing,
    Finished,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamingHealth {
    pub session_id: SessionId,
    pub state: StreamingHealthState,
    pub metrics: StreamingMetrics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamingFinish {
    pub session_id: SessionId,
    pub finalized: Option<FinalTranscript>,
    pub metrics: StreamingMetrics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamingResult {
    Hypothesis(StreamingHypothesis),
    Endpoint {
        correlation_id: CorrelationId,
        session_id: SessionId,
        reason: EndpointReason,
    },
    Finished(StreamingFinish),
    Failed {
        correlation_id: CorrelationId,
        session_id: SessionId,
        error: AsrError,
    },
}

#[doc(hidden)]
pub enum StreamingControlMessage {
    Audio(TimestampedAudioFrame),
    Endpoint(EndpointReason),
    InputFinished,
    Finish(SyncSender<Result<StreamingFinish, AsrError>>),
    Health(SyncSender<StreamingHealth>),
    Cancel,
}

#[derive(Clone)]
pub struct StreamingSessionControl {
    correlation_id: CorrelationId,
    session_id: SessionId,
    sender: SyncSender<StreamingControlMessage>,
    cancel: CancelToken,
    backpressure_events: Arc<AtomicU64>,
}

impl StreamingSessionControl {
    pub fn bounded(
        correlation_id: CorrelationId,
        session_id: SessionId,
        capacity: usize,
        cancel: CancelToken,
    ) -> Option<(Self, StreamingControlReceiver)> {
        if capacity == 0 {
            return None;
        }
        let (sender, receiver) = sync_channel(capacity);
        let backpressure_events = Arc::new(AtomicU64::new(0));
        Some((
            Self {
                correlation_id,
                session_id,
                sender,
                cancel,
                backpressure_events: Arc::clone(&backpressure_events),
            },
            StreamingControlReceiver {
                receiver,
                backpressure_events,
            },
        ))
    }

    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Enqueues one correlated, timestamped frame without waiting for capacity.
    ///
    /// # Errors
    ///
    /// Returns backpressure, closed-session, or stale-identity status.
    pub fn push_audio(&self, frame: TimestampedAudioFrame) -> Result<(), StreamingSendError> {
        if frame.correlation_id != self.correlation_id || frame.session_id != self.session_id {
            return Err(StreamingSendError::StaleIdentity);
        }
        self.try_send(StreamingControlMessage::Audio(frame))
    }

    /// Marks an endpoint while leaving the stream open for post-roll/tail frames.
    ///
    /// # Errors
    ///
    /// Returns backpressure or closed-session status.
    pub fn endpoint(&self, reason: EndpointReason) -> Result<(), StreamingSendError> {
        self.try_send(StreamingControlMessage::Endpoint(reason))
    }

    /// Seals audio input after every final-drain/tail frame has been enqueued.
    ///
    /// # Errors
    ///
    /// Returns backpressure or closed-session status.
    pub fn input_finished(&self) -> Result<(), StreamingSendError> {
        self.try_send(StreamingControlMessage::InputFinished)
    }

    /// Requests authoritative stream finalization within a bounded wait.
    ///
    /// # Errors
    ///
    /// Returns a queue, deadline, cancellation, or engine error.
    pub fn finish(&self, timeout: Duration) -> Result<StreamingFinish, AsrError> {
        let (sender, receiver) = sync_channel(1);
        self.try_send(StreamingControlMessage::Finish(sender))
            .map_err(streaming_send_domain_error)?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| streaming_domain_error(ErrorCode::DeadlineExceeded))?
    }

    /// Queries worker-owned state and metrics within a bounded wait.
    ///
    /// # Errors
    ///
    /// Returns a queue or deadline error when health cannot be obtained.
    pub fn health(&self, timeout: Duration) -> Result<StreamingHealth, AsrError> {
        let (sender, receiver) = sync_channel(1);
        self.try_send(StreamingControlMessage::Health(sender))
            .map_err(streaming_send_domain_error)?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| streaming_domain_error(ErrorCode::DeadlineExceeded))
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        let _ = self.sender.try_send(StreamingControlMessage::Cancel);
    }

    fn try_send(&self, message: StreamingControlMessage) -> Result<(), StreamingSendError> {
        self.sender.try_send(message).map_err(|error| match error {
            TrySendError::Full(_) => {
                self.backpressure_events.fetch_add(1, Ordering::Relaxed);
                StreamingSendError::Backpressure
            }
            TrySendError::Disconnected(_) => StreamingSendError::Closed,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamingSendError {
    Backpressure,
    Closed,
    StaleIdentity,
}

fn streaming_send_domain_error(error: StreamingSendError) -> DomainError {
    streaming_domain_error(match error {
        StreamingSendError::Backpressure => ErrorCode::QueueFull,
        StreamingSendError::Closed => ErrorCode::AdapterFailed,
        StreamingSendError::StaleIdentity => ErrorCode::StaleEvent,
    })
}

const fn streaming_domain_error(code: ErrorCode) -> DomainError {
    DomainError {
        code,
        recoverable: true,
    }
}

pub struct StreamingControlReceiver {
    receiver: Receiver<StreamingControlMessage>,
    backpressure_events: Arc<AtomicU64>,
}

impl StreamingControlReceiver {
    /// Receives the next bounded control message.
    ///
    /// # Errors
    ///
    /// Returns timeout or disconnected status from the bounded channel.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<StreamingControlMessage, std::sync::mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    pub fn backpressure_events(&self) -> u64 {
        self.backpressure_events.load(Ordering::Relaxed)
    }
}

pub struct StreamingResultReceiver(Receiver<StreamingResult>);

impl StreamingResultReceiver {
    pub fn new(receiver: Receiver<StreamingResult>) -> Self {
        Self(receiver)
    }

    /// Receives the next correlated result.
    ///
    /// # Errors
    ///
    /// Returns timeout or disconnected status from the bounded channel.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<StreamingResult, std::sync::mpsc::RecvTimeoutError> {
        self.0.recv_timeout(timeout)
    }
}

pub struct StreamingSessionHandle {
    pub control: StreamingSessionControl,
    pub results: StreamingResultReceiver,
}

pub trait StreamingAsr: Send + Sync {
    fn capabilities(&self) -> EngineCapabilities;
    fn spawn_session(
        &self,
        request: StreamingAsrRequest,
    ) -> BoxFuture<'_, Result<StreamingSessionHandle, AsrError>>;
}

pub trait FinalAsr: Send + Sync {
    fn capabilities(&self) -> EngineCapabilities;
    fn transcribe(
        &self,
        audio: UtteranceAudio,
        request: AsrRequest,
        cancel: CancelToken,
        deadline: Deadline,
    ) -> BoxFuture<'_, Result<FinalTranscript, AsrError>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtectedSpan {
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolishLimits {
    pub maximum_input_bytes: usize,
    pub maximum_output_bytes: usize,
    pub maximum_expansion_numerator: usize,
    pub maximum_expansion_denominator: usize,
}

impl PolishLimits {
    pub const fn is_valid(self) -> bool {
        self.maximum_input_bytes > 0
            && self.maximum_output_bytes > 0
            && self.maximum_expansion_numerator > 0
            && self.maximum_expansion_denominator > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolishRequest {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub raw_text: String,
    pub system_prompt: String,
    pub protected_spans: Vec<ProtectedSpan>,
    pub limits: PolishLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolishedText {
    pub session_id: SessionId,
    pub text: String,
    pub provider_id: String,
    pub model_id: String,
    pub profile_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolishError {
    Cancelled,
    DeadlineExceeded,
    ConsentRequired,
    CredentialUnavailable,
    Offline,
    Network,
    Authentication,
    RateLimited,
    EmptyOutput,
    InvalidOutput,
    OversizedInput,
    OversizedOutput,
    ExpansionExceeded,
    ProtectedSpanChanged,
    UntrustedOutput,
    ProviderUnavailable,
    OutOfMemory,
}

pub trait PolishEngine: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn polish(
        &self,
        request: PolishRequest,
        cancel: CancelToken,
        deadline: Deadline,
    ) -> BoxFuture<'_, Result<PolishedText, PolishError>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutableIdentity {
    pub path: String,
    pub process_start_time: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrityRelationship {
    TargetLower,
    Equal,
    TargetHigher,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiaElementIdentity {
    pub runtime_id: Vec<i32>,
    pub control_type: u32,
    pub class_name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UiaPatterns {
    pub text: bool,
    pub text2: bool,
    pub value: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSnapshot {
    pub start: Option<i32>,
    pub end: Option<i32>,
    pub caret: Option<i32>,
    pub is_empty: bool,
    pub range_fingerprint: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardContext {
    pub layout: Option<u64>,
    pub ime_open: Option<bool>,
    pub ime_composing: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    Standard,
    Terminal,
    UnknownSensitive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetSnapshot {
    pub session_id: SessionId,
    pub window_handle: u64,
    pub process_id: u32,
    pub thread_id: u32,
    pub executable: ExecutableIdentity,
    pub integrity: IntegrityRelationship,
    pub element: Option<UiaElementIdentity>,
    pub target_kind: TargetKind,
    pub is_password: bool,
    pub is_read_only: bool,
    pub is_secure_desktop: Option<bool>,
    pub patterns: UiaPatterns,
    pub selection: Option<SelectionSnapshot>,
    pub content_fingerprint: Option<[u8; 32]>,
    pub input_epoch: Option<u32>,
    pub hook_epoch: Option<u64>,
    pub keyboard: KeyboardContext,
    pub capability: DeliveryCapability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryRequest {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryStrategy {
    ResultView,
    Clipboard,
    UnicodeInput,
    ClipboardPaste,
    ValuePattern,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    Retained,
    ClipboardWritten,
    InputQueued,
    Committed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryReceipt {
    pub session_id: SessionId,
    pub capability: DeliveryCapability,
    pub strategy: DeliveryStrategy,
    pub outcome: DeliveryOutcome,
    pub clipboard_sequence: Option<u32>,
    pub input_events_accepted: Option<u32>,
    pub consumption_verified: bool,
}

pub type DeliveryError = DomainError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryRefusal {
    Cancelled,
    DeadlineExceeded,
    SessionMismatch,
    FocusChanged,
    ProcessChanged,
    ElementChanged,
    SelectionChanged,
    CaretChanged,
    ContentChanged,
    IntegrityChanged,
    UserInput,
    HookUnavailable,
    AppClosed,
    WindowReused,
    Password,
    ReadOnly,
    SecureDesktop,
    ElevatedTarget,
    TargetInaccessible,
    UnknownSensitive,
    ModifierHeld,
    ClipboardBusy,
    ClipboardChanged,
    ClipboardSnapshotIncomplete,
    AmbiguousInput,
    Unsupported,
}

pub trait TextTarget: Send + Sync {
    fn inspect(&self, deadline: Deadline)
    -> BoxFuture<'_, Result<TargetSnapshot, DeliveryRefusal>>;
    fn deliver<'a>(
        &'a self,
        snapshot: &'a TargetSnapshot,
        request: DeliveryRequest,
        cancel: CancelToken,
        deadline: Deadline,
    ) -> BoxFuture<'a, Result<DeliveryReceipt, DeliveryError>>;
}
