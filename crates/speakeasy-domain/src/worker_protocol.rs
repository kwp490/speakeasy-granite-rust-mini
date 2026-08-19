//! Framed-JSON protocol for a supervised local worker process.
//!
//! This lives in the domain crate — otherwise dependency-free — rather than in
//! an engine crate, so a worker (Granite on llama.cpp, in
//! `workers/granite-worker`) can speak the wire protocol without linking an
//! engine's native libraries into a process that has no use for them. It was
//! moved here out of the streaming crate, `speakeasy-asr`, so that Granite's
//! process would not link ONNX Runtime; that crate is gone now and the reason
//! survives it — the protocol belongs to neither engine.
//!
//! The protocol itself is engine-agnostic on purpose: `LoadModel` names an
//! artifact and a directory, `StartStream`/`PushAudio`/`FinishStream` frame one
//! utterance, and an engine reads that the same way regardless of whether it
//! streams incrementally or buffers and transcribes once at `FinishStream`
//! (Granite).

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use crate::{CancelToken, Deadline, DomainError};

/// Bumped 1 → 2 on 2026-08-10, when [`WorkerEvent::Ready`] gained
/// `compiled_accelerators`.
///
/// The bump is the point, not a formality. A worker kills itself on any frame
/// it cannot deserialize (`read_frame` failing is fatal in both workers' run
/// loops), and `WorkerEvent` is `deny_unknown_fields`, so an unannounced field
/// or command breaks whichever side is older — silently in the worst case.
/// A version mismatch instead goes through `WorkerRequest::validate`, which
/// answers `WorkerErrorCode::ProtocolMismatch` *gracefully* from the handler
/// rather than dying in the reader.
///
/// That matters because host and worker no longer always ship together:
/// `scripts/Enable-GraniteCuda.ps1` deliberately stages a locally built Granite
/// worker over an installed one. A mismatched pair now reports what it is.
pub const WORKER_PROTOCOL_VERSION: u32 = 2;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_AUDIO_SAMPLES_PER_REQUEST: usize = 16_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct RequestId(pub u64);

/// A worker-local utterance identifier: a plain counter scoped to one worker
/// process, distinct from [`crate::SessionId`], which is a 16-byte identifier
/// scoped to the whole app. Naming it `WorkerSessionId` rather than reusing
/// `SessionId` is the whole reason the type exists in this module at all —
/// The streaming crate used to get away with the bare name because the two only
/// collided across crate boundaries; sharing one crate makes that collision
/// real.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct WorkerSessionId(pub u64);

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRequest {
    pub protocol_version: u32,
    pub request_id: RequestId,
    pub command: WorkerCommand,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerCommand {
    Hello,
    Health,
    LoadModel {
        artifact_id: String,
        model_root: String,
    },
    UnloadModel,
    StartStream {
        session_id: WorkerSessionId,
        sample_rate_hz: u32,
    },
    PushAudio {
        session_id: WorkerSessionId,
        sequence: u64,
        samples: Vec<f32>,
    },
    FinishStream {
        session_id: WorkerSessionId,
    },
    Cancel {
        session_id: WorkerSessionId,
    },
    Shutdown,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResponse {
    pub protocol_version: u32,
    pub request_id: RequestId,
    pub event: WorkerEvent,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerEvent {
    Ready {
        worker_version: String,
        /// Which inference backends are **compiled into** this binary.
        ///
        /// Only a worker can answer this. Granite's CUDA backend is linked in
        /// by `speakeasy-granite`'s `cuda` feature rather than loaded beside
        /// the executable, so there is no file to stat and nothing about the
        /// filesystem can tell — which is exactly why the host used to carry a
        /// hardcoded `false` and report the wrong device.
        ///
        /// Compile-time only, and deliberately not "what will this run on".
        /// The streaming worker resolves its ONNX Runtime providers from DLLs
        /// at run time, so it reports nothing here and
        /// `RuntimeWizardCoordinator::cuda_runtime_available` stays the
        /// authority for that engine.
        ///
        /// `default` so a future field addition within v2 does not need
        /// another bump; the version guards the shape, this guards the tail.
        #[serde(default)]
        compiled_accelerators: Vec<String>,
    },
    ModelLoaded {
        artifact_id: String,
    },
    Healthy {
        artifact_id: Option<String>,
        stream_active: bool,
    },
    ModelUnloaded,
    StreamStarted {
        session_id: WorkerSessionId,
    },
    AudioAccepted {
        session_id: WorkerSessionId,
        sequence: u64,
    },
    Transcript {
        session_id: WorkerSessionId,
        line_id: u64,
        text: String,
        is_final: bool,
    },
    StreamFinished {
        session_id: WorkerSessionId,
    },
    Cancelled {
        session_id: WorkerSessionId,
    },
    ShuttingDown,
    Error {
        code: WorkerErrorCode,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerErrorCode {
    InvalidRequest,
    ProtocolMismatch,
    InvalidState,
    ArtifactNotTrusted,
    ArtifactInvalid,
    RuntimeUnavailable,
    InferenceFailed,
}

/// Deadline-aware request boundary implemented by the supervised worker owner.
pub trait WorkerClient: Send {
    /// Sends one command and returns its correlated response batch.
    ///
    /// # Errors
    ///
    /// Returns a recoverable domain error for cancellation, deadline expiry,
    /// protocol mismatch, worker failure, or unavailable transport.
    fn request(
        &mut self,
        command: WorkerCommand,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<Vec<WorkerEvent>, DomainError>;
}

pub const fn worker_response_is_terminal(command: &WorkerCommand, event: &WorkerEvent) -> bool {
    matches!(event, WorkerEvent::Error { .. })
        || matches!(
            (command, event),
            (WorkerCommand::Hello, WorkerEvent::Ready { .. })
                | (WorkerCommand::Health, WorkerEvent::Healthy { .. })
                | (
                    WorkerCommand::LoadModel { .. },
                    WorkerEvent::ModelLoaded { .. }
                )
                | (WorkerCommand::UnloadModel, WorkerEvent::ModelUnloaded)
                | (
                    WorkerCommand::StartStream { .. },
                    WorkerEvent::StreamStarted { .. }
                )
                | (
                    WorkerCommand::PushAudio { .. },
                    WorkerEvent::AudioAccepted { .. }
                )
                | (
                    WorkerCommand::FinishStream { .. },
                    WorkerEvent::StreamFinished { .. }
                )
                | (WorkerCommand::Cancel { .. }, WorkerEvent::Cancelled { .. })
                | (WorkerCommand::Shutdown, WorkerEvent::ShuttingDown)
        )
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    FrameTooLarge { actual: usize, maximum: usize },
    Json(serde_json::Error),
    Invalid(String),
}

impl Display for ProtocolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "worker protocol I/O failed: {error}"),
            Self::FrameTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "worker frame is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::Json(error) => write!(formatter, "worker frame JSON is invalid: {error}"),
            Self::Invalid(message) => write!(formatter, "worker request is invalid: {message}"),
        }
    }
}

impl Error for ProtocolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::FrameTooLarge { .. } | Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl WorkerRequest {
    /// Validates invariants that are not expressible in the serialized shape.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::Invalid`] for unsupported versions, invalid
    /// identifiers, unsupported rates, or unsafe audio payloads.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != WORKER_PROTOCOL_VERSION {
            return Err(ProtocolError::Invalid(format!(
                "unsupported protocol version {}",
                self.protocol_version
            )));
        }
        match &self.command {
            WorkerCommand::LoadModel {
                artifact_id,
                model_root,
            } => {
                if artifact_id.is_empty() || artifact_id.len() > 128 {
                    return Err(ProtocolError::Invalid(
                        "artifact_id must contain 1 to 128 bytes".to_owned(),
                    ));
                }
                if model_root.is_empty() || model_root.len() > 32_768 {
                    return Err(ProtocolError::Invalid(
                        "model_root must contain 1 to 32768 bytes".to_owned(),
                    ));
                }
            }
            WorkerCommand::StartStream { sample_rate_hz, .. } => {
                if !(8_000..=192_000).contains(sample_rate_hz) {
                    return Err(ProtocolError::Invalid(
                        "sample_rate_hz must be between 8000 and 192000".to_owned(),
                    ));
                }
            }
            WorkerCommand::PushAudio { samples, .. } => {
                if samples.is_empty() || samples.len() > MAX_AUDIO_SAMPLES_PER_REQUEST {
                    return Err(ProtocolError::Invalid(format!(
                        "audio payload must contain 1 to {MAX_AUDIO_SAMPLES_PER_REQUEST} samples"
                    )));
                }
                if samples.iter().any(|sample| !sample.is_finite()) {
                    return Err(ProtocolError::Invalid(
                        "audio payload contains a non-finite sample".to_owned(),
                    ));
                }
            }
            WorkerCommand::Hello
            | WorkerCommand::Health
            | WorkerCommand::UnloadModel
            | WorkerCommand::FinishStream { .. }
            | WorkerCommand::Cancel { .. }
            | WorkerCommand::Shutdown => {}
        }
        Ok(())
    }
}

/// Writes one little-endian length-prefixed JSON frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when serialization, size validation, or I/O fails.
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), ProtocolError> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            actual: payload.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        actual: payload.len(),
        maximum: MAX_FRAME_BYTES,
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Reads one little-endian length-prefixed JSON frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when framing, deserialization, size validation,
/// or I/O fails.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl Read,
) -> Result<T, ProtocolError> {
    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            actual: length,
            maximum: MAX_FRAME_BYTES,
        });
    }
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> WorkerRequest {
        WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: RequestId(7),
            command: WorkerCommand::Hello,
        }
    }

    #[test]
    fn frame_round_trip_preserves_request() {
        let request = hello();
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).expect("request must encode");
        let decoded: WorkerRequest =
            read_frame(&mut bytes.as_slice()).expect("request must decode");
        assert_eq!(decoded, request);
        decoded.validate().expect("request must validate");
    }

    /// The device a worker reports has to survive the wire, because it is the
    /// only way the host can learn it — Granite's CUDA backend is linked into
    /// the binary, so there is no file to stat.
    #[test]
    fn ready_carries_the_accelerators_compiled_into_the_worker() {
        let response = WorkerResponse {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: RequestId(3),
            event: WorkerEvent::Ready {
                worker_version: "1.2.0".to_owned(),
                compiled_accelerators: vec!["cuda".to_owned()],
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &response).expect("response must encode");
        let decoded: WorkerResponse =
            read_frame(&mut bytes.as_slice()).expect("response must decode");
        assert_eq!(decoded, response);
    }

    /// A worker that predates the field is a CPU worker, and must decode as one
    /// rather than failing. `deny_unknown_fields` governs the other direction,
    /// which is what the version bump is for.
    #[test]
    fn a_ready_without_accelerators_decodes_as_no_accelerators() {
        let json = br#"{"type":"ready","worker_version":"1.2.0"}"#;
        let event: WorkerEvent = serde_json::from_slice(json).expect("older ready must decode");
        assert_eq!(
            event,
            WorkerEvent::Ready {
                worker_version: "1.2.0".to_owned(),
                compiled_accelerators: Vec::new(),
            }
        );
    }

    /// The bump is load-bearing rather than bookkeeping: a mismatched pair has
    /// to be *reportable*. `validate` is what makes it so, because it runs in
    /// the handler and answers `ProtocolMismatch`, while a shape mismatch would
    /// fail in `read_frame` and kill the worker outright.
    #[test]
    fn a_request_from_another_protocol_version_is_rejected_by_validate() {
        let stale = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION - 1,
            request_id: RequestId(9),
            command: WorkerCommand::Hello,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &stale).expect("request must encode");
        let decoded: WorkerRequest =
            read_frame(&mut bytes.as_slice()).expect("request must decode");
        decoded
            .validate()
            .expect_err("a foreign protocol version must not validate");
    }

    #[test]
    fn frame_rejects_declared_oversize_before_allocation() {
        let bytes = u32::try_from(MAX_FRAME_BYTES + 1)
            .expect("test frame length fits")
            .to_le_bytes();
        assert!(matches!(
            read_frame::<WorkerRequest>(&mut bytes.as_slice()),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn request_rejects_version_rate_and_audio_violations() {
        let mut request = hello();
        request.protocol_version = WORKER_PROTOCOL_VERSION + 1;
        assert!(request.validate().is_err());

        request.protocol_version = WORKER_PROTOCOL_VERSION;
        request.command = WorkerCommand::StartStream {
            session_id: WorkerSessionId(1),
            sample_rate_hz: 1,
        };
        assert!(request.validate().is_err());

        request.command = WorkerCommand::PushAudio {
            session_id: WorkerSessionId(1),
            sequence: 0,
            samples: vec![f32::NAN],
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let payload =
            br#"{"protocol_version":1,"request_id":7,"command":{"type":"hello"},"extra":true}"#;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(payload);
        assert!(matches!(
            read_frame::<WorkerRequest>(&mut bytes.as_slice()),
            Err(ProtocolError::Json(_))
        ));
    }
}
