//! Bounded, ordered handoff for finalized audio.
//!
//! Capture owns the real-time path. This queue starts after capture has
//! drained and sealed an utterance, so backpressure can wait here without ever
//! blocking the device callback. One consumer is intentional: native ASR
//! models are commonly single-flight, and delivery must follow utterance ID
//! order even when inference latency varies.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;

use speakeasy_domain::{AsrRequest, EngineSnapshot, FinalAudioJob, UtteranceAudio, UtteranceId};

pub const DEFAULT_FINALIZATION_QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalizationQueueError {
    InvalidCapacity,
    ConsumerStopped,
}

/// A bounded queue whose jobs are consumed by exactly one worker thread.
#[derive(Clone)]
pub struct OrderedFinalizationQueue {
    sender: SyncSender<FinalAudioJob>,
    next_id: Arc<AtomicU64>,
    depth: Arc<AtomicUsize>,
}

impl std::fmt::Debug for OrderedFinalizationQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OrderedFinalizationQueue")
            .field("depth", &self.depth())
            .finish_non_exhaustive()
    }
}

impl OrderedFinalizationQueue {
    /// Creates the queue and starts its sole ordered consumer.
    ///
    /// A processor panic is contained so one bad job cannot strand every later
    /// dictation behind a dead consumer.
    ///
    /// # Errors
    ///
    /// Returns `InvalidCapacity` for an unbounded/zero-capacity request or
    /// `ConsumerStopped` if the worker thread cannot be started.
    pub fn new<F>(capacity: usize, process: F) -> Result<Self, FinalizationQueueError>
    where
        F: Fn(FinalAudioJob) + Send + 'static,
    {
        if capacity == 0 {
            return Err(FinalizationQueueError::InvalidCapacity);
        }
        let (sender, receiver) = sync_channel(capacity);
        let depth = Arc::new(AtomicUsize::new(0));
        let worker_depth = Arc::clone(&depth);
        thread::Builder::new()
            .name("speakeasy-finalization".to_owned())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    worker_depth.fetch_sub(1, Ordering::AcqRel);
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(job)));
                }
            })
            .map_err(|_| FinalizationQueueError::ConsumerStopped)?;
        Ok(Self {
            sender,
            next_id: Arc::new(AtomicU64::new(1)),
            depth,
        })
    }

    /// Enqueues finalized audio. This waits while the bounded queue is full;
    /// the caller is already off the capture callback path, so no final
    /// dictation is silently dropped.
    ///
    /// # Errors
    ///
    /// Returns `ConsumerStopped` if the sole consumer has exited before the
    /// job can be accepted.
    pub fn submit(
        &self,
        audio: UtteranceAudio,
        request: AsrRequest,
        engine: EngineSnapshot,
    ) -> Result<UtteranceId, FinalizationQueueError> {
        let id = UtteranceId::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        let job = FinalAudioJob::new(id, audio, request, engine);
        self.depth.fetch_add(1, Ordering::AcqRel);
        if self.sender.send(job).is_err() {
            self.depth.fetch_sub(1, Ordering::AcqRel);
            return Err(FinalizationQueueError::ConsumerStopped);
        }
        Ok(id)
    }

    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    use speakeasy_domain::{AsrLanguage, AsrTask, CorrelationId, SessionId};

    fn job_audio(byte: i16) -> UtteranceAudio {
        UtteranceAudio {
            session_id: SessionId::from_bytes([u8::try_from(byte).expect("test byte"); 16]),
            sample_rate_hz: 16_000,
            samples: vec![byte],
        }
    }

    fn request(audio: &UtteranceAudio) -> AsrRequest {
        AsrRequest {
            correlation_id: CorrelationId::from_bytes(audio.session_id.into_bytes()),
            session_id: audio.session_id,
            language: AsrLanguage::English,
            task: AsrTask::Transcribe,
        }
    }

    #[test]
    fn invalid_capacity_is_rejected() {
        let result = OrderedFinalizationQueue::new(0, |_| {});
        assert!(matches!(
            result,
            Err(FinalizationQueueError::InvalidCapacity)
        ));
    }

    #[test]
    fn one_consumer_preserves_utterance_order_and_never_overlaps() {
        let completed = std::sync::Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let maximum_active = std::sync::Arc::new(AtomicUsize::new(0));
        let panicked = std::sync::Arc::new(AtomicBool::new(false));
        let process_completed = std::sync::Arc::clone(&completed);
        let process_active = std::sync::Arc::clone(&active);
        let process_maximum = std::sync::Arc::clone(&maximum_active);
        let process_panicked = std::sync::Arc::clone(&panicked);
        let queue = OrderedFinalizationQueue::new(5, move |job| {
            let current = process_active.fetch_add(1, Ordering::AcqRel) + 1;
            process_maximum.fetch_max(current, Ordering::AcqRel);
            if current != 1 {
                process_panicked.store(true, Ordering::Release);
            }
            std::thread::sleep(Duration::from_millis(2));
            let (items, wake) = &*process_completed;
            items.lock().unwrap().push(job.utterance_id.value());
            wake.notify_all();
            process_active.fetch_sub(1, Ordering::AcqRel);
        })
        .expect("queue");

        for value in 0..20_i16 {
            let audio = job_audio(value);
            queue
                .submit(
                    audio.clone(),
                    request(&audio),
                    EngineSnapshot::new("test", "ordered", "cpu"),
                )
                .expect("submit must apply bounded backpressure, not drop");
        }

        let (items, wake) = &*completed;
        let mut items = items.lock().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while items.len() != 20 {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "ordered consumer did not drain");
            (items, _) = wake.wait_timeout(items, remaining).unwrap();
        }
        assert_eq!(items.as_slice(), (1..=20).collect::<Vec<_>>().as_slice());
        assert_eq!(maximum_active.load(Ordering::Acquire), 1);
        assert!(!panicked.load(Ordering::Acquire));
        assert_eq!(queue.depth(), 0);
    }
}
