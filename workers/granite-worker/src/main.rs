//! Granite Speech in its own supervised worker process.
//!
//! Speaks `speakeasy_domain::worker_protocol`'s framed-JSON protocol, which the
//! streaming worker spoke before it too, so the desktop app's
//! `ProcessWorkerClient` and `WorkerClient` boundary work for this engine
//! without modification — only the process on the other end of the pipe
//! differs.
//!
//! # Why this process links no ASR crate and no ONNX Runtime
//!
//! `unsafe_code = "forbid"` cannot protect against a C++ segfault inside
//! llama.cpp, so process isolation is the mitigation: a crash here kills this
//! process, not the app the user is mid-dictation in, and it keeps llama.cpp's
//! CUDA context away from ONNX Runtime's. Depending on `speakeasy-domain` alone
//! is what actually delivers that — the protocol types would be identical
//! either way, but the streaming crate pulled in `sherpa-onnx`, and linking that
//! into this binary would have re-introduced exactly the coupling process
//! isolation exists to avoid. That crate is gone; the rule it motivated stands,
//! because it is what keeps this binary's dependency surface llama.cpp alone.
//!
//! # Why the model loads at `LoadModel` and stays loaded
//!
//! This worker process is resident across many dictations — the desktop app
//! spawns it once and keeps it alive rather than per dictation — so
//! `LoadModel` loads the ~2 GB of weights via `GraniteModel::load` and holds
//! them for the process's lifetime; `FinishStream` reuses that same loaded
//! model and projector for every utterance rather than reloading. The
//! earlier shape (load-transcribe-destroy per dictation, mirroring
//! `speakeasy-granite`'s "deliberately wasteful" free functions) is still
//! available in that crate for one-shot callers, but this worker is not one.
//!
//! # Why there is no hash check here, on purpose
//!
//! `LoadModel` verifies file *presence*, not a SHA-256 against a trusted pin —
//! unlike the streaming worker's `verify_model_files`, which this was written
//! beside. That is not the manifest hole it was then:
//! `models/trusted-manifest.json` carries Granite packs, and the gap is closed
//! **caller-side** — `apps/desktop` hashes the pack's files with
//! `speakeasy_models::verify_pack_files` before ever spawning this process.
//! Widening the wire protocol to carry digests was considered and rejected
//! there: the manifest is the trust root either way, so a worker "verifying" a
//! digest the same caller handed it distrusts nobody. Do not read the absence
//! of a hash check here as an oversight.

use std::io;
use std::path::{Path, PathBuf};

use speakeasy_domain::{
    WORKER_PROTOCOL_VERSION, WorkerCommand, WorkerErrorCode, WorkerEvent, WorkerRequest,
    WorkerResponse, WorkerSessionId, read_frame, write_frame,
};
use speakeasy_granite::{GraniteError, GraniteModel, GraniteOptions, GraniteStage};

/// The only artifact this worker will load: the shipped quantization, `Q4_K_M`
/// since 2026-08-04, when it replaced `Q8_0` on measurement. A literal rather
/// than something read out of the manifest because this crate deliberately does
/// not link a manifest reader — the caller resolves the pack and hands the
/// resolved `model_root` over, as this module's doc comment explains. Changing
/// the shipped quantization therefore means changing this pair *and* the
/// manifest's `install_eligible` flags together; they are checked against each
/// other by `the_worker_artifact_id_matches_the_install_eligible_pack` in
/// `apps/desktop`'s `granite_engine` tests.
const GRANITE_ARTIFACT_ID: &str = "granite-speech-4.1-2b-q4_k_m";
/// Filenames as IBM's own published GGUF conversion names them, matching
/// `speakeasy-granite`'s hardware proof fixture layout under
/// `.tools/granite-speech-4.1-2b/`.
const GRANITE_MODEL_FILENAME: &str = "granite-speech-4.1-2b-Q4_K_M.gguf";
const GRANITE_PROJECTOR_FILENAME: &str = "mmproj-model-f16.gguf";

/// Mirrors `speakeasy_worker::SAMPLE_RATE_HZ`. Duplicated rather than shared,
/// because sharing it would mean this worker depending on the crate that drives
/// it. See this module's doc comment.
const SAMPLE_RATE_HZ: u32 = 16_000;
/// Must stay aligned with the desktop capture safety ceiling: two minutes at
/// the worker's fixed sample rate. The worker enforces this independently so a
/// malformed or compromised parent cannot grow one resident utterance without
/// bound.
const MAX_UTTERANCE_SAMPLES: usize = 120 * SAMPLE_RATE_HZ as usize;

/// One buffered utterance: Granite has nothing to say until `FinishStream`, so
/// every `PushAudio` in between just accumulates samples.
struct ActiveStream {
    session_id: WorkerSessionId,
    samples: Vec<f32>,
    next_sequence: u64,
    /// The terms this utterance's decode is biased toward, taken at
    /// `StartStream` and spent at `FinishStream`. Held per stream rather than
    /// per worker so one dictation's vocabulary cannot leak into the next: the
    /// model is resident across dictations, the bias is not.
    keywords: Vec<String>,
}

struct Worker {
    artifact_id: Option<String>,
    model_root: Option<PathBuf>,
    /// The loaded model and projector, held resident from a successful
    /// `LoadModel` until `UnloadModel` or `Shutdown`. `load_model`/
    /// `unload_model`/`shutdown` keep this in lockstep with `artifact_id`/
    /// `model_root`; a test may construct a `Worker` with those set but this
    /// left `None` to exercise a single handler (e.g. `push_audio`) without a
    /// real loaded model.
    model: Option<GraniteModel>,
    active: Option<ActiveStream>,
}

impl Worker {
    const fn new() -> Self {
        Self {
            artifact_id: None,
            model_root: None,
            model: None,
            active: None,
        }
    }

    fn handle(&mut self, request: &WorkerRequest) -> (Vec<WorkerEvent>, bool) {
        if let Err(error) = request.validate() {
            return (
                vec![WorkerEvent::Error {
                    code: if request.protocol_version == WORKER_PROTOCOL_VERSION {
                        WorkerErrorCode::InvalidRequest
                    } else {
                        WorkerErrorCode::ProtocolMismatch
                    },
                    message: error.to_string(),
                }],
                false,
            );
        }

        let result = match &request.command {
            // The one thing only this process can answer. llama.cpp's CUDA
            // backend is linked in by `speakeasy-granite`'s `cuda` feature, so
            // a host inspecting the filesystem cannot tell a GPU worker from a
            // CPU one -- and before this it assumed CPU and logged that while
            // Granite ran on the GPU.
            WorkerCommand::Hello => Ok(vec![WorkerEvent::Ready {
                worker_version: env!("CARGO_PKG_VERSION").to_owned(),
                compiled_accelerators: if speakeasy_granite::CUDA_ENABLED {
                    vec!["cuda".to_owned()]
                } else {
                    Vec::new()
                },
            }]),
            WorkerCommand::Health => Ok(vec![WorkerEvent::Healthy {
                artifact_id: self.artifact_id.clone(),
                stream_active: self.active.is_some(),
            }]),
            WorkerCommand::LoadModel {
                artifact_id,
                model_root,
            } => self.load_model(artifact_id, Path::new(model_root)),
            WorkerCommand::UnloadModel => self.unload_model(),
            WorkerCommand::StartStream {
                session_id,
                sample_rate_hz,
                keywords,
            } => self.start_stream(*session_id, *sample_rate_hz, keywords),
            WorkerCommand::PushAudio {
                session_id,
                sequence,
                samples,
            } => self.push_audio(*session_id, *sequence, samples),
            WorkerCommand::FinishStream { session_id } => self.finish_stream(*session_id),
            WorkerCommand::Cancel { session_id } => self.cancel(*session_id),
            WorkerCommand::Shutdown => {
                self.shutdown();
                return (vec![WorkerEvent::ShuttingDown], true);
            }
        };
        (result.unwrap_or_else(error_event), false)
    }

    fn load_model(&mut self, artifact_id: &str, model_root: &Path) -> WorkerResult {
        if self.active.is_some() {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "cannot replace a model while a stream is active",
            ));
        }
        // `WorkerFinalAdapter::run_locked` sends `LoadModel` before every
        // single dictation, unconditionally -- it has no way to know whether
        // this process already has a model loaded. As in the streaming worker's
        // own `load_model`: when the same artifact is already resident, this is
        // a fast no-op rather than a second ~2 GB load, which is the entire
        // point of keeping the model loaded across dictations at all.
        if self.artifact_id.as_deref() == Some(artifact_id) && self.model.is_some() {
            return Ok(vec![WorkerEvent::ModelLoaded {
                artifact_id: artifact_id.to_owned(),
            }]);
        }
        if artifact_id != GRANITE_ARTIFACT_ID {
            return Err(worker_error(
                WorkerErrorCode::ArtifactNotTrusted,
                "artifact is not the Granite pack this worker serves",
            ));
        }
        for filename in [GRANITE_MODEL_FILENAME, GRANITE_PROJECTOR_FILENAME] {
            if !model_root.join(filename).is_file() {
                return Err(worker_error(
                    WorkerErrorCode::ArtifactInvalid,
                    "a required Granite model file is missing",
                ));
            }
        }
        let model = GraniteModel::load(
            &model_root.join(GRANITE_MODEL_FILENAME),
            &model_root.join(GRANITE_PROJECTOR_FILENAME),
            &GraniteOptions::default(),
        )
        .map_err(|error| granite_worker_error(&error))?;
        self.artifact_id = Some(artifact_id.to_owned());
        self.model_root = Some(model_root.to_path_buf());
        self.model = Some(model);
        Ok(vec![WorkerEvent::ModelLoaded {
            artifact_id: artifact_id.to_owned(),
        }])
    }

    fn unload_model(&mut self) -> WorkerResult {
        if self.active.is_some() {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "cannot unload a model while a stream is active",
            ));
        }
        self.artifact_id = None;
        self.model_root = None;
        self.model = None;
        Ok(vec![WorkerEvent::ModelUnloaded])
    }

    fn start_stream(
        &mut self,
        session_id: WorkerSessionId,
        sample_rate_hz: u32,
        keywords: &[String],
    ) -> WorkerResult {
        if self.active.is_some() {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "a stream is already active",
            ));
        }
        if sample_rate_hz != SAMPLE_RATE_HZ {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                format!("sample_rate_hz must be {SAMPLE_RATE_HZ}"),
            ));
        }
        if self.artifact_id.is_none() {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "no model is loaded",
            ));
        }
        self.active = Some(ActiveStream {
            session_id,
            samples: Vec::new(),
            next_sequence: 0,
            keywords: keywords.to_vec(),
        });
        Ok(vec![WorkerEvent::StreamStarted { session_id }])
    }

    fn push_audio(
        &mut self,
        session_id: WorkerSessionId,
        sequence: u64,
        samples: &[f32],
    ) -> WorkerResult {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| worker_error(WorkerErrorCode::InvalidState, "no stream is active"))?;
        if active.session_id != session_id {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "session ID does not match the active stream",
            ));
        }
        if active.next_sequence != sequence {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                format!(
                    "audio sequence must be {}; received {sequence}",
                    active.next_sequence
                ),
            ));
        }
        let new_length = active
            .samples
            .len()
            .checked_add(samples.len())
            .ok_or_else(|| {
                worker_error(WorkerErrorCode::InvalidState, "audio stream is too large")
            })?;
        if new_length > MAX_UTTERANCE_SAMPLES {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                format!("audio stream exceeds the {MAX_UTTERANCE_SAMPLES} sample limit"),
            ));
        }
        active.samples.extend_from_slice(samples);
        active.next_sequence += 1;
        Ok(vec![WorkerEvent::AudioAccepted {
            session_id,
            sequence,
        }])
    }

    fn finish_stream(&mut self, session_id: WorkerSessionId) -> WorkerResult {
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| worker_error(WorkerErrorCode::InvalidState, "no stream is active"))?;
        if active.session_id != session_id {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "session ID does not match the active stream",
            ));
        }
        // `start_stream` refuses to begin a stream without a loaded model, so
        // this cannot be `None` here — asserted rather than re-checked as a
        // fallible error path for a state that cannot arise.
        let model = self
            .model
            .as_ref()
            .expect("a loaded model outlives every stream it started");

        // The only place this worker departs from `GraniteOptions::default`.
        // The prompt is built in `speakeasy-granite` beside `TRANSCRIBE_PROMPT`
        // rather than here, because the model's prompt contract belongs with
        // the model — and because an empty list has to reproduce the default
        // instruction byte for byte, which is easier to keep true in one place.
        let options = GraniteOptions {
            prompt: speakeasy_granite::transcribe_prompt_with_keywords(&active.keywords),
            ..GraniteOptions::default()
        };
        let outcome = model.transcribe_samples(&active.samples, &options);
        // Consumes the buffered utterance regardless of outcome: a failed
        // transcription still ends the stream rather than leaving stale audio
        // for the next one to inherit.
        self.active = None;

        let text = outcome.map_err(|error| granite_worker_error(&error))?;
        Ok(vec![
            WorkerEvent::Transcript {
                session_id,
                line_id: 1,
                text,
                is_final: true,
            },
            WorkerEvent::StreamFinished { session_id },
        ])
    }

    fn cancel(&mut self, session_id: WorkerSessionId) -> WorkerResult {
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| worker_error(WorkerErrorCode::InvalidState, "no stream is active"))?;
        if active.session_id != session_id {
            return Err(worker_error(
                WorkerErrorCode::InvalidState,
                "session ID does not match the active stream",
            ));
        }
        self.active = None;
        Ok(vec![WorkerEvent::Cancelled { session_id }])
    }

    fn shutdown(&mut self) {
        self.active = None;
        self.artifact_id = None;
        self.model_root = None;
        self.model = None;
    }
}

type WorkerResult = Result<Vec<WorkerEvent>, WorkerFailure>;

struct WorkerFailure {
    code: WorkerErrorCode,
    message: String,
}

fn worker_error(code: WorkerErrorCode, message: impl Into<String>) -> WorkerFailure {
    WorkerFailure {
        code,
        message: message.into(),
    }
}

/// Maps a Granite failure stage onto the shared worker error vocabulary.
/// Load-time stages (the backend, the model, the projector) are reported as
/// `RuntimeUnavailable`; everything downstream of a successfully loaded model
/// is `InferenceFailed`.
fn granite_worker_error(error: &GraniteError) -> WorkerFailure {
    let code = match error.stage() {
        GraniteStage::Backend | GraniteStage::ModelLoad | GraniteStage::ProjectorLoad => {
            WorkerErrorCode::RuntimeUnavailable
        }
        GraniteStage::AudioUnsupported
        | GraniteStage::AudioDecode
        | GraniteStage::Tokenize
        | GraniteStage::Evaluate
        | GraniteStage::Generate
        | GraniteStage::Detokenize => WorkerErrorCode::InferenceFailed,
    };
    worker_error(code, error.to_string())
}

fn error_event(error: WorkerFailure) -> Vec<WorkerEvent> {
    vec![WorkerEvent::Error {
        code: error.code,
        message: error.message,
    }]
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut worker = Worker::new();
    loop {
        let request: WorkerRequest = match read_frame(&mut input) {
            Ok(request) => request,
            Err(speakeasy_domain::ProtocolError::Io(error))
                if error.kind() == io::ErrorKind::UnexpectedEof =>
            {
                return Ok(());
            }
            Err(error) => return Err(Box::new(error)),
        };
        let (events, should_exit) = worker.handle(&request);
        for event in events {
            write_frame(
                &mut output,
                &WorkerResponse {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: request.request_id,
                    event,
                },
            )?;
        }
        if should_exit {
            return Ok(());
        }
    }
}

fn main() {
    if run().is_err() {
        eprintln!("granite_worker_failed=internal");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speakeasy_domain::RequestId;

    fn request(command: WorkerCommand) -> WorkerRequest {
        WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: RequestId(1),
            command,
        }
    }

    #[test]
    fn hello_reports_version_without_loading_native_code() {
        let (events, should_exit) = Worker::new().handle(&request(WorkerCommand::Hello));
        assert!(!should_exit);
        assert!(matches!(events.as_slice(), [WorkerEvent::Ready { .. }]));
    }

    #[test]
    fn stream_commands_fail_closed_before_model_load() {
        let (events, should_exit) = Worker::new().handle(&request(WorkerCommand::StartStream {
            session_id: WorkerSessionId(42),
            sample_rate_hz: SAMPLE_RATE_HZ,
            keywords: Vec::new(),
        }));
        assert!(!should_exit);
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::InvalidState,
                ..
            }]
        ));
    }

    #[test]
    fn unknown_artifact_is_refused_without_touching_disk() {
        let (events, should_exit) = Worker::new().handle(&request(WorkerCommand::LoadModel {
            artifact_id: "unknown-artifact".to_owned(),
            model_root: "missing".to_owned(),
        }));
        assert!(!should_exit);
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::ArtifactNotTrusted,
                ..
            }]
        ));
    }

    #[test]
    fn known_artifact_with_missing_files_is_invalid() {
        let (events, should_exit) = Worker::new().handle(&request(WorkerCommand::LoadModel {
            artifact_id: GRANITE_ARTIFACT_ID.to_owned(),
            model_root: "definitely-does-not-exist".to_owned(),
        }));
        assert!(!should_exit);
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::ArtifactInvalid,
                ..
            }]
        ));
    }

    #[test]
    fn health_reports_unloaded_runtime_and_unload_is_idempotent() {
        let mut worker = Worker::new();
        let (events, should_exit) = worker.handle(&request(WorkerCommand::Health));
        assert!(!should_exit);
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Healthy {
                artifact_id: None,
                stream_active: false
            }]
        ));

        let (events, should_exit) = worker.handle(&request(WorkerCommand::UnloadModel));
        assert!(!should_exit);
        assert!(matches!(events.as_slice(), [WorkerEvent::ModelUnloaded]));
    }

    #[test]
    fn push_audio_enforces_the_active_session_and_strict_sequencing() {
        let mut worker = Worker {
            artifact_id: Some(GRANITE_ARTIFACT_ID.to_owned()),
            model_root: Some(PathBuf::from(".")),
            model: None,
            active: Some(ActiveStream {
                session_id: WorkerSessionId(1),
                samples: Vec::new(),
                next_sequence: 0,
                keywords: Vec::new(),
            }),
        };

        let (events, _) = worker.handle(&request(WorkerCommand::PushAudio {
            session_id: WorkerSessionId(2),
            sequence: 0,
            samples: vec![0.0],
        }));
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::InvalidState,
                ..
            }]
        ));

        let (events, _) = worker.handle(&request(WorkerCommand::PushAudio {
            session_id: WorkerSessionId(1),
            sequence: 5,
            samples: vec![0.0],
        }));
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::InvalidState,
                ..
            }]
        ));

        let (events, _) = worker.handle(&request(WorkerCommand::PushAudio {
            session_id: WorkerSessionId(1),
            sequence: 0,
            samples: vec![0.25, -0.25],
        }));
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::AudioAccepted {
                session_id: WorkerSessionId(1),
                sequence: 0
            }]
        ));
        assert_eq!(
            worker.active.as_ref().expect("still active").samples,
            vec![0.25, -0.25]
        );
    }

    #[test]
    fn push_audio_refuses_a_frame_past_the_two_minute_limit() {
        let mut worker = Worker {
            artifact_id: Some(GRANITE_ARTIFACT_ID.to_owned()),
            model_root: Some(PathBuf::from(".")),
            model: None,
            active: Some(ActiveStream {
                session_id: WorkerSessionId(1),
                samples: vec![0.0; MAX_UTTERANCE_SAMPLES],
                next_sequence: 0,
                keywords: Vec::new(),
            }),
        };

        let (events, _) = worker.handle(&request(WorkerCommand::PushAudio {
            session_id: WorkerSessionId(1),
            sequence: 0,
            samples: vec![0.0],
        }));
        assert!(matches!(
            events.as_slice(),
            [WorkerEvent::Error {
                code: WorkerErrorCode::InvalidState,
                ..
            }]
        ));
        assert_eq!(
            worker
                .active
                .as_ref()
                .expect("stream remains active")
                .samples
                .len(),
            MAX_UTTERANCE_SAMPLES
        );
    }
}
