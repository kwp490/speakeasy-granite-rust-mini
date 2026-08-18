//! The bounded local worker boundary, and the verdict on what it returns.
//!
//! This crate is what the desktop app uses to drive an inference worker child
//! process and to decide whether the transcript that came back is fit to
//! paste. It links no native libraries of its own -- the inference lives in
//! `workers/granite-worker`, on the far side of a framed-JSON pipe -- so it
//! builds and tests in seconds with no model, no GPU, and no toolchain beyond
//! rustc.
//!
//! It was `speakeasy-asr`, and it did link one: sherpa-onnx, for the streaming
//! recognizer that ran while the user spoke. That engine is gone, and with it
//! the ONNX Runtime dependency, the CUDA execution provider, the
//! `transcribe-cpp` canary, and the `System32\onnxruntime.dll` shadowing
//! hazard that came with them. What is left is the part that was never about
//! ASR: a protocol boundary, an ordering queue, and a plausibility gate.

#![allow(clippy::must_use_candidate)]

use std::sync::Mutex;

use speakeasy_domain::{
    AsrError, AsrLanguage, AsrRequest, AsrTask, BoxFuture, CancelToken, Clock, Deadline,
    DomainError, EngineCapabilities, ErrorCode, FinalAsr, FinalAsrMetrics, FinalTranscript,
    TranscriptProvenance, UtteranceAudio,
};

// The framed-JSON worker protocol lives in `speakeasy-domain` (see that
// crate's `worker_protocol` module) so a second inference runtime can speak it
// without linking ONNX Runtime. Re-exported here unchanged so this crate's own
// use of it below, and every existing caller's `use speakeasy_asr::{...}`,
// keep working without churn.
pub use speakeasy_domain::{
    MAX_AUDIO_SAMPLES_PER_REQUEST, MAX_FRAME_BYTES, ProtocolError, RequestId,
    WORKER_PROTOCOL_VERSION, WorkerClient, WorkerCommand, WorkerErrorCode, WorkerEvent,
    WorkerRequest, WorkerResponse, WorkerSessionId, read_frame, worker_response_is_terminal,
    write_frame,
};

mod finalization;
mod verdict;

pub use finalization::{
    DEFAULT_FINALIZATION_QUEUE_CAPACITY, FinalizationQueueError, OrderedFinalizationQueue,
};
pub use verdict::{
    FinalSourceReason, GraniteVerdict, SAMPLE_RATE_HZ, is_plausible, judge_granite_pass, tokens,
};

const RETAINED_PUSH_FRAME_SAMPLES: usize = 1_600;

/// Runs a `WorkerClient`-speaking engine over retained audio in batch, for the
/// delivered transcript.
///
/// Generic over the client on purpose, even though this app now has exactly
/// one engine to drive. It was written that way because there were two, and
/// keeping it that way costs nothing and is what makes the worker process
/// substitutable in tests: every test below drives it with a scripted
/// in-process client rather than a real llama.cpp child.
pub struct WorkerFinalAdapter<C, K> {
    client: Mutex<C>,
    clock: K,
    model_root: String,
    artifact_id: String,
    capabilities: EngineCapabilities,
}

impl<C, K> WorkerFinalAdapter<C, K> {
    pub fn new(
        client: C,
        clock: K,
        model_root: String,
        artifact_id: String,
        capabilities: EngineCapabilities,
    ) -> Self {
        Self {
            client: Mutex::new(client),
            clock,
            model_root,
            artifact_id,
            capabilities,
        }
    }

    /// Returns the owned worker client for composition-level unload/shutdown.
    ///
    /// # Errors
    ///
    /// Returns a recoverable error if the serialization lock was poisoned.
    pub fn into_client(self) -> Result<C, DomainError> {
        self.client
            .into_inner()
            .map_err(|_| domain_error(ErrorCode::AdapterFailed))
    }

    /// The clock this adapter was constructed with.
    ///
    /// `Deadline::after` and `Deadline::expired` only compare meaningfully
    /// against the *same* clock instance -- `SystemClock::now()` is relative
    /// to when that particular `SystemClock` was constructed
    /// (`crates/speakeasy-domain/src/contracts.rs`), not to a shared origin.
    /// A caller building a deadline for a call into this adapter must build
    /// it from *this* clock, not a fresh one of its own, or the deadline can
    /// read as already-expired the moment the adapter has been resident
    /// longer than the deadline's own duration. Exists because Known risk
    /// #12 (Phase 9.6, `docs/handoff/granite-final-pass.md`) was exactly
    /// that mistake: `run_granite_final_pass` built its 90 s deadline from a
    /// brand-new `SystemClock::default()` every call, while the resident
    /// adapter's own clock kept counting from whenever it first warmed --
    /// so any dictation more than 90 s after that warm failed instantly,
    /// deterministically, with no I/O ever attempted.
    pub const fn clock(&self) -> &K {
        &self.clock
    }
}

impl<C: WorkerClient, K: Clock> WorkerFinalAdapter<C, K> {
    fn run(
        &self,
        audio: &UtteranceAudio,
        request: AsrRequest,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<FinalTranscript, AsrError> {
        let pass = BatchFinalPass {
            clock: &self.clock,
            model_root: &self.model_root,
            artifact_id: &self.artifact_id,
        };
        validate_batch_request(audio, &request)?;
        check_active(&self.clock, cancel, deadline)?;
        let mut client = self
            .client
            .lock()
            .map_err(|_| domain_error(ErrorCode::AdapterFailed))?;
        pass.run(&mut *client, audio, request, cancel, deadline)
    }
}

/// One utterance transcribed in batch over an already-open worker connection:
/// `LoadModel`, `StartStream`, the whole utterance in frames, `FinishStream`.
///
/// Factored out of [`WorkerFinalAdapter`] rather than left as a method on it
/// because two different adapters now need the identical pass over two
/// differently-owned clients: [`WorkerFinalAdapter`] owns a client of its own
/// (Granite's resident worker, and the retained pass's spawn-per-dictation
/// fallback), while [`StreamingPackAdapter`] shares one with whatever live
/// session is using it. Duplicating the sequence into the second one would
/// have meant two places for the delivered transcript's shape to drift apart —
/// and the shape is load-bearing (see the `final_lines.join(" ")` comment
/// below, and Phase 2's `NoSpeechDetected` split in
/// `docs/handoff/granite-final-pass.md`).
///
/// Borrows rather than owns every field so neither adapter has to clone its
/// identity per dictation.
pub(crate) struct BatchFinalPass<'a, K> {
    /// **The client's own clock, never a fresh one.** See
    /// [`WorkerFinalAdapter::clock`] for why that distinction is not cosmetic.
    pub(crate) clock: &'a K,
    pub(crate) model_root: &'a str,
    pub(crate) artifact_id: &'a str,
}

/// Rejects a request no engine could honour, before any worker is touched.
/// Shared by both adapters that run [`BatchFinalPass`], so neither can admit
/// an utterance the other would refuse.
pub(crate) fn validate_batch_request(
    audio: &UtteranceAudio,
    request: &AsrRequest,
) -> Result<(), AsrError> {
    if audio.session_id != request.session_id || audio.sample_rate_hz != 16_000 {
        return Err(domain_error(ErrorCode::InvalidData));
    }
    if request.language != AsrLanguage::English || request.task != AsrTask::Transcribe {
        return Err(domain_error(ErrorCode::InvalidData));
    }
    Ok(())
}

impl<K: Clock> BatchFinalPass<'_, K> {
    pub(crate) fn run(
        &self,
        client: &mut impl WorkerClient,
        audio: &UtteranceAudio,
        request: AsrRequest,
        cancel: &CancelToken,
        deadline: Deadline,
    ) -> Result<FinalTranscript, AsrError> {
        request_expect(
            client,
            WorkerCommand::LoadModel {
                artifact_id: self.artifact_id.to_owned(),
                model_root: self.model_root.to_owned(),
            },
            cancel,
            deadline,
            |event| matches!(event, WorkerEvent::ModelLoaded { artifact_id } if artifact_id.as_str() == self.artifact_id),
        )?;
        let worker_session = worker_session_id(request.session_id);
        request_expect(
            client,
            WorkerCommand::StartStream {
                session_id: worker_session,
                sample_rate_hz: audio.sample_rate_hz,
            },
            cancel,
            deadline,
            |event| matches!(event, WorkerEvent::StreamStarted { session_id } if *session_id == worker_session),
        )?;

        let mut final_lines = Vec::new();
        let mut last_draft = None;
        let mut draft_revisions = 0usize;
        for (sequence, samples) in audio
            .samples
            .chunks(RETAINED_PUSH_FRAME_SAMPLES)
            .enumerate()
        {
            check_active(self.clock, cancel, deadline)?;
            let samples = samples
                .iter()
                .map(|sample| f32::from(*sample) / 32_768.0)
                .collect();
            let events = client.request(
                WorkerCommand::PushAudio {
                    session_id: worker_session,
                    sequence: sequence as u64,
                    samples,
                },
                cancel,
                deadline,
            )?;
            collect_transcripts(
                events,
                worker_session,
                &mut final_lines,
                &mut last_draft,
                &mut draft_revisions,
            )?;
        }
        check_active(self.clock, cancel, deadline)?;
        let events = client.request(
            WorkerCommand::FinishStream {
                session_id: worker_session,
            },
            cancel,
            deadline,
        )?;
        let finished = events.iter().any(
            |event| matches!(event, WorkerEvent::StreamFinished { session_id } if *session_id == worker_session),
        );
        collect_transcripts(
            events,
            worker_session,
            &mut final_lines,
            &mut last_draft,
            &mut draft_revisions,
        )?;
        if !finished {
            return Err(domain_error(ErrorCode::AdapterFailed));
        }

        let final_segments = final_lines.len();
        let (raw_text, provenance) = if final_segments == 0 {
            (
                last_draft.unwrap_or_default(),
                TranscriptProvenance::LastValidDraft,
            )
        } else {
            // A worker may emit more than one final segment per utterance (one
            // per sentence/pause boundary). For dictation delivered into a
            // single text field, join these with a space so one spoken
            // utterance stays on one line; joining with newlines split a single
            // sentence across lines at every mid-thought pause.
            // normalize_engine_text collapses the seams.
            (final_lines.join(" "), TranscriptProvenance::FinalizedStream)
        };
        let text = normalize_engine_text(&raw_text);
        if text.is_empty() {
            return Err(domain_error(ErrorCode::NoSpeechDetected));
        }
        Ok(FinalTranscript {
            session_id: request.session_id,
            raw_text,
            text,
            provenance,
            metrics: FinalAsrMetrics {
                input_samples: audio.samples.len(),
                final_segments,
                draft_revisions,
            },
        })
    }
}

impl<C: WorkerClient, K: Clock> FinalAsr for WorkerFinalAdapter<C, K> {
    fn capabilities(&self) -> EngineCapabilities {
        self.capabilities
    }

    fn transcribe(
        &self,
        audio: UtteranceAudio,
        request: AsrRequest,
        cancel: CancelToken,
        deadline: Deadline,
    ) -> BoxFuture<'_, Result<FinalTranscript, AsrError>> {
        Box::pin(async move { self.run(&audio, request, &cancel, deadline) })
    }
}

fn request_expect(
    client: &mut impl WorkerClient,
    command: WorkerCommand,
    cancel: &CancelToken,
    deadline: Deadline,
    expected: impl Fn(&WorkerEvent) -> bool,
) -> Result<(), DomainError> {
    let events = client.request(command, cancel, deadline)?;
    if events.iter().any(expected) && !events.iter().any(is_error) {
        Ok(())
    } else {
        Err(domain_error(ErrorCode::AdapterFailed))
    }
}

fn collect_transcripts(
    events: Vec<WorkerEvent>,
    session_id: WorkerSessionId,
    final_lines: &mut Vec<String>,
    last_draft: &mut Option<String>,
    draft_revisions: &mut usize,
) -> Result<(), DomainError> {
    for event in events {
        match event {
            WorkerEvent::Transcript {
                session_id: event_session,
                text,
                is_final,
                ..
            } if event_session == session_id => {
                if is_final {
                    if !text.trim().is_empty() {
                        final_lines.push(text);
                    }
                } else if !text.trim().is_empty() {
                    *last_draft = Some(text);
                    *draft_revisions += 1;
                }
            }
            WorkerEvent::Transcript { .. } | WorkerEvent::Error { .. } => {
                return Err(domain_error(ErrorCode::StaleEvent));
            }
            _ => {}
        }
    }
    Ok(())
}

fn normalize_engine_text(text: &str) -> String {
    // Collapse every run of whitespace (including any newlines a worker segment
    // may carry) into single spaces, yielding clean single-line dictation output.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn worker_session_id(session_id: speakeasy_domain::SessionId) -> WorkerSessionId {
    let bytes = session_id.into_bytes();
    WorkerSessionId(u64::from_le_bytes(
        bytes[..8].try_into().expect("fixed slice"),
    ))
}

pub(crate) fn check_active(
    clock: &impl Clock,
    cancel: &CancelToken,
    deadline: Deadline,
) -> Result<(), DomainError> {
    if cancel.is_cancelled() {
        Err(domain_error(ErrorCode::Cancelled))
    } else if deadline.expired(clock.now()) {
        Err(domain_error(ErrorCode::DeadlineExceeded))
    } else {
        Ok(())
    }
}

pub(crate) const fn is_error(event: &WorkerEvent) -> bool {
    matches!(event, WorkerEvent::Error { .. })
}

pub(crate) const fn domain_error(code: ErrorCode) -> DomainError {
    DomainError {
        code,
        recoverable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use speakeasy_domain::{CorrelationId, MonotonicTime, SessionId as DomainSessionId};

    #[derive(Clone, Copy)]
    struct FixedClock(MonotonicTime);

    impl Clock for FixedClock {
        fn now(&self) -> MonotonicTime {
            self.0
        }
    }

    struct FakeWorkerClient {
        responses: VecDeque<Vec<WorkerEvent>>,
    }

    impl WorkerClient for FakeWorkerClient {
        fn request(
            &mut self,
            _command: WorkerCommand,
            _cancel: &CancelToken,
            _deadline: Deadline,
        ) -> Result<Vec<WorkerEvent>, DomainError> {
            self.responses
                .pop_front()
                .ok_or_else(|| domain_error(ErrorCode::AdapterFailed))
        }
    }

    fn domain_session() -> DomainSessionId {
        DomainSessionId::from_bytes([7; 16])
    }

    fn asr_request() -> AsrRequest {
        AsrRequest {
            correlation_id: CorrelationId::from_bytes([3; 16]),
            session_id: domain_session(),
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        }
    }

    const TEST_ARTIFACT_ID: &str = "test-streaming-pack";

    const fn test_capabilities() -> EngineCapabilities {
        EngineCapabilities {
            execution: speakeasy_domain::AsrExecution::Local,
            streaming: speakeasy_domain::AsrStreaming::TrueOnline,
            sample_rate_hz: 16_000,
            channels: 1,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
            features: &[speakeasy_domain::AsrFeature::Punctuation],
            runtime: "sherpa-onnx",
            runtime_abi: "sherpa-onnx-c-api-1.13.4",
            provider: "test-provider",
            artifact_revision: "test-revision",
            license: "Apache-2.0",
        }
    }

    fn adapter(
        responses: Vec<Vec<WorkerEvent>>,
    ) -> WorkerFinalAdapter<FakeWorkerClient, FixedClock> {
        WorkerFinalAdapter::new(
            FakeWorkerClient {
                responses: responses.into(),
            },
            FixedClock(MonotonicTime(1)),
            "trusted-model".to_owned(),
            TEST_ARTIFACT_ID.to_owned(),
            test_capabilities(),
        )
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        let mut future = pin!(future);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[test]
    fn finalized_worker_output_becomes_normalized_recoverable_result() {
        let worker_session = worker_session_id(domain_session());
        let adapter = adapter(vec![
            vec![WorkerEvent::ModelLoaded {
                artifact_id: TEST_ARTIFACT_ID.to_owned(),
            }],
            vec![WorkerEvent::StreamStarted {
                session_id: worker_session,
            }],
            vec![
                WorkerEvent::Transcript {
                    session_id: worker_session,
                    line_id: 1,
                    text: "  final text  ".to_owned(),
                    is_final: true,
                },
                WorkerEvent::AudioAccepted {
                    session_id: worker_session,
                    sequence: 0,
                },
            ],
            vec![WorkerEvent::StreamFinished {
                session_id: worker_session,
            }],
        ]);
        let result = block_on(adapter.transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![1; 160],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect("final result");
        assert_eq!(result.raw_text, "  final text  ");
        assert_eq!(result.text, "final text");
        assert_eq!(result.provenance, TranscriptProvenance::FinalizedStream);
        assert_eq!(result.metrics.input_samples, 160);
    }

    #[test]
    fn multiple_final_segments_join_on_one_line_for_dictation_delivery() {
        // A worker may split a single spoken utterance into several final
        // segments at pause boundaries; they must be delivered as one
        // space-joined line, never split across newlines (the hotkey/retained
        // path regression).
        let worker_session = worker_session_id(domain_session());
        let adapter = adapter(vec![
            vec![WorkerEvent::ModelLoaded {
                artifact_id: TEST_ARTIFACT_ID.to_owned(),
            }],
            vec![WorkerEvent::StreamStarted {
                session_id: worker_session,
            }],
            vec![
                WorkerEvent::Transcript {
                    session_id: worker_session,
                    line_id: 1,
                    text: "What foreign governments".to_owned(),
                    is_final: true,
                },
                WorkerEvent::Transcript {
                    session_id: worker_session,
                    line_id: 2,
                    text: "that could be construed as authoritarian.".to_owned(),
                    is_final: true,
                },
                WorkerEvent::AudioAccepted {
                    session_id: worker_session,
                    sequence: 0,
                },
            ],
            vec![WorkerEvent::StreamFinished {
                session_id: worker_session,
            }],
        ]);
        let result = block_on(adapter.transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![1; 160],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect("final result");
        assert_eq!(
            result.text,
            "What foreign governments that could be construed as authoritarian."
        );
        assert!(!result.text.contains('\n'));
        assert_eq!(result.metrics.final_segments, 2);
    }

    #[test]
    fn retained_audio_uses_proven_hundred_millisecond_frames() {
        let worker_session = worker_session_id(domain_session());
        let adapter = adapter(vec![
            vec![WorkerEvent::ModelLoaded {
                artifact_id: TEST_ARTIFACT_ID.to_owned(),
            }],
            vec![WorkerEvent::StreamStarted {
                session_id: worker_session,
            }],
            vec![WorkerEvent::AudioAccepted {
                session_id: worker_session,
                sequence: 0,
            }],
            vec![
                WorkerEvent::Transcript {
                    session_id: worker_session,
                    line_id: 1,
                    text: "framed result".to_owned(),
                    is_final: true,
                },
                WorkerEvent::AudioAccepted {
                    session_id: worker_session,
                    sequence: 1,
                },
            ],
            vec![WorkerEvent::StreamFinished {
                session_id: worker_session,
            }],
        ]);
        let result = block_on(adapter.transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![1; RETAINED_PUSH_FRAME_SAMPLES + 1],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect("1,601 samples must be sent as two native frames");
        assert_eq!(result.text, "framed result");
    }

    #[test]
    fn last_valid_draft_is_explicit_and_empty_results_are_rejected() {
        let worker_session = worker_session_id(domain_session());
        let with_draft = adapter(vec![
            vec![WorkerEvent::ModelLoaded {
                artifact_id: TEST_ARTIFACT_ID.to_owned(),
            }],
            vec![WorkerEvent::StreamStarted {
                session_id: worker_session,
            }],
            vec![WorkerEvent::Transcript {
                session_id: worker_session,
                line_id: 1,
                text: "draft".to_owned(),
                is_final: false,
            }],
            vec![WorkerEvent::StreamFinished {
                session_id: worker_session,
            }],
        ]);
        let result = block_on(with_draft.transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![0; 16],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect("draft fallback");
        assert_eq!(result.provenance, TranscriptProvenance::LastValidDraft);

        let empty = adapter(vec![
            vec![WorkerEvent::ModelLoaded {
                artifact_id: TEST_ARTIFACT_ID.to_owned(),
            }],
            vec![WorkerEvent::StreamStarted {
                session_id: worker_session,
            }],
            vec![],
            vec![WorkerEvent::StreamFinished {
                session_id: worker_session,
            }],
        ]);
        let error = block_on(empty.transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![0; 16],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect_err("empty output must not be fabricated");
        assert_eq!(error.code, ErrorCode::NoSpeechDetected);
    }

    #[test]
    fn cancellation_and_deadline_fail_before_worker_mutation() {
        let cancelled = CancelToken::default();
        cancelled.cancel();
        let error = block_on(adapter(vec![]).transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![0],
            },
            asr_request(),
            cancelled,
            Deadline {
                at: MonotonicTime(10),
            },
        ))
        .expect_err("cancelled");
        assert_eq!(error.code, ErrorCode::Cancelled);

        let error = block_on(adapter(vec![]).transcribe(
            UtteranceAudio {
                session_id: domain_session(),
                sample_rate_hz: 16_000,
                samples: vec![0],
            },
            asr_request(),
            CancelToken::default(),
            Deadline {
                at: MonotonicTime(1),
            },
        ))
        .expect_err("deadline");
        assert_eq!(error.code, ErrorCode::DeadlineExceeded);
    }

    #[test]
    fn unsupported_language_and_task_are_refused_before_worker_mutation() {
        for (language, task) in [
            (AsrLanguage::Other("fr-FR"), AsrTask::Transcribe),
            (AsrLanguage::English, AsrTask::Translate),
        ] {
            let mut unsupported = asr_request();
            unsupported.language = language;
            unsupported.task = task;
            let error = block_on(adapter(vec![]).transcribe(
                UtteranceAudio {
                    session_id: domain_session(),
                    sample_rate_hz: 16_000,
                    samples: vec![0],
                },
                unsupported,
                CancelToken::default(),
                Deadline {
                    at: MonotonicTime(10),
                },
            ))
            .expect_err("unsupported request");
            assert_eq!(error.code, ErrorCode::InvalidData);
        }
    }
}
