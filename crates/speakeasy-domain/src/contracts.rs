use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use crate::{CorrelationId, SessionId, SessionPhase};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicTime(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deadline {
    pub at: MonotonicTime,
}

impl Deadline {
    pub const fn expired(self, now: MonotonicTime) -> bool {
        now.0 >= self.at.0
    }

    pub fn after(clock: &impl Clock, duration: Duration) -> Self {
        let additional = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        Self {
            at: MonotonicTime(clock.now().0.saturating_add(additional)),
        }
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> MonotonicTime;
}

#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> MonotonicTime {
        MonotonicTime(u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX))
    }
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> MonotonicTime {
        (**self).now()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundedQueueConfig {
    pub capacity: usize,
    pub send_timeout: Duration,
}

impl BoundedQueueConfig {
    pub const fn new(capacity: usize, send_timeout: Duration) -> Option<Self> {
        if capacity == 0 {
            return None;
        }
        Some(Self {
            capacity,
            send_timeout,
        })
    }
}

pub fn bounded_channel<T>(config: BoundedQueueConfig) -> (SyncSender<T>, Receiver<T>) {
    sync_channel(config.capacity)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationMode {
    Toggle,
    PushToTalk,
    HandsFree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    AudioStart,
    Finalize,
    Delivery,
    SettingsSave,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppCommand {
    Activate {
        correlation_id: CorrelationId,
        session_id: SessionId,
        mode: ActivationMode,
        deadline: Deadline,
    },
    Stop {
        correlation_id: CorrelationId,
        session_id: SessionId,
        deadline: Deadline,
    },
    Cancel {
        correlation_id: CorrelationId,
        session_id: SessionId,
    },
    InjectFailure(FailurePoint),
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    StateChanged {
        correlation_id: CorrelationId,
        session_id: Option<SessionId>,
        phase: SessionPhase,
    },
    TranscriptAvailable {
        correlation_id: CorrelationId,
        session_id: SessionId,
        character_count: usize,
    },
    Failure {
        correlation_id: CorrelationId,
        session_id: Option<SessionId>,
        error: DomainError,
    },
    ShutdownComplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    AppNotReady,
    SessionAlreadyActive,
    InvalidTransition,
    StaleEvent,
    QueueFull,
    Cancelled,
    DeadlineExceeded,
    AdapterFailed,
    InvalidData,
    TooNew,
    Unauthorized,
    /// The pass completed and produced no text because the audio held no
    /// speech, not because anything malfunctioned. Distinct from
    /// `AdapterFailed` so silence never counts toward worker quarantine.
    NoSpeechDetected,
    /// A second-pass engine (e.g. Granite) was not attempted because it is
    /// quarantined after repeated crashes. Distinct from `AdapterFailed` so a
    /// caller can disclose "not attempted" rather than "failed."
    EngineQuarantined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainError {
    pub code: ErrorCode,
    pub recoverable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_and_queue_capacity_must_be_bounded() {
        let first = CancelToken::default();
        let second = first.clone();
        first.cancel();
        assert!(second.is_cancelled());
        assert!(BoundedQueueConfig::new(0, Duration::ZERO).is_none());
        assert!(BoundedQueueConfig::new(1, Duration::ZERO).is_some());

        let clock = SystemClock::default();
        let deadline = Deadline::after(&clock, Duration::from_secs(1));
        assert!(!deadline.expired(clock.now()));
    }

    #[test]
    fn bounded_channel_reports_backpressure_without_growing() {
        let config = BoundedQueueConfig::new(1, Duration::ZERO).expect("bounded config");
        let (sender, receiver) = bounded_channel(config);
        sender.try_send(1).expect("first item");
        assert!(matches!(
            sender.try_send(2),
            Err(std::sync::mpsc::TrySendError::Full(2))
        ));
        assert_eq!(receiver.recv().expect("receive"), 1);
    }
}
