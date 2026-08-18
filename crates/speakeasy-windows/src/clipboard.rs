use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use speakeasy_domain::{
    DeliveryCapability, DeliveryOutcome, DeliveryReceipt, DeliveryRefusal, DeliveryStrategy,
    SessionId, TargetSnapshot,
};

const MAXIMUM_OPEN_ATTEMPTS: u8 = 5;
const OPEN_DEADLINE: Duration = Duration::from_millis(100);
const RESULT_DEADLINE: Duration = Duration::from_secs(3);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(100);

fn retry_clipboard_open<T>(
    deadline: Instant,
    mut operation: impl FnMut() -> Result<T, DeliveryRefusal>,
) -> Result<T, DeliveryRefusal> {
    for attempt in 1..=MAXIMUM_OPEN_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(DeliveryRefusal::ClipboardBusy)
                if attempt < MAXIMUM_OPEN_ATTEMPTS && Instant::now() < deadline =>
            {
                thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
    Err(DeliveryRefusal::ClipboardBusy)
}

#[derive(Debug)]
struct ClipboardRequest {
    session_id: SessionId,
    text: String,
    deadline: Instant,
    response: SyncSender<Result<DeliveryReceipt, DeliveryRefusal>>,
}

pub struct ClipboardWriter {
    requests: Option<SyncSender<ClipboardRequest>>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardWriter {
    /// Starts the bounded clipboard worker.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::Unsupported`] if the worker cannot start.
    pub fn spawn() -> Result<Self, DeliveryRefusal> {
        let (requests, receiver) = mpsc::sync_channel(4);
        let worker = thread::Builder::new()
            .name("speakeasy-clipboard".to_owned())
            .spawn(move || run_worker(&receiver))
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        Ok(Self {
            requests: Some(requests),
            worker: Some(worker),
        })
    }

    /// Writes a non-empty final result as Unicode text without restoring or pasting it.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for secure targets, empty text, queue pressure,
    /// clipboard contention, or a failed sequence-number transition.
    pub fn write(
        &self,
        snapshot: &TargetSnapshot,
        text: String,
    ) -> Result<DeliveryReceipt, DeliveryRefusal> {
        speakeasy_delivery::classify_guard(snapshot)?;
        self.write_until(snapshot.session_id, text, Instant::now() + RESULT_DEADLINE)
    }

    /// Writes an explicitly requested recoverable result as Unicode text.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for empty text, queue pressure, clipboard
    /// contention, or a failed sequence-number transition.
    pub fn write_result(
        &self,
        session_id: SessionId,
        text: String,
    ) -> Result<DeliveryReceipt, DeliveryRefusal> {
        self.write_until(session_id, text, Instant::now() + RESULT_DEADLINE)
    }

    pub(crate) fn write_until(
        &self,
        session_id: SessionId,
        text: String,
        deadline: Instant,
    ) -> Result<DeliveryReceipt, DeliveryRefusal> {
        if text.is_empty() {
            return Err(DeliveryRefusal::Unsupported);
        }
        let (response, result) = mpsc::sync_channel(1);
        self.requests
            .as_ref()
            .ok_or(DeliveryRefusal::Unsupported)?
            .try_send(ClipboardRequest {
                session_id,
                text,
                deadline,
                response,
            })
            .map_err(|_| DeliveryRefusal::ClipboardBusy)?;
        result
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| DeliveryRefusal::Unsupported)?
    }
}

impl Drop for ClipboardWriter {
    fn drop(&mut self) {
        self.requests.take();
        // A native clipboard provider can wedge inside an OS call. Detach a
        // live worker rather than turning shutdown into an unbounded join.
        drop(self.worker.take());
    }
}

fn run_worker(receiver: &Receiver<ClipboardRequest>) {
    loop {
        let request = match receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let response = request.response.clone();
        let _ = response.send(write_clipboard(&request));
    }
}

#[cfg(windows)]
fn write_clipboard(request: &ClipboardRequest) -> Result<DeliveryReceipt, DeliveryRefusal> {
    use winsafe::prelude::*;

    fn sequence(deadline: Instant) -> Result<u32, DeliveryRefusal> {
        let clipboard = retry_clipboard_open(deadline, || {
            winsafe::HWND::NULL
                .OpenClipboard()
                .map_err(|_| DeliveryRefusal::ClipboardBusy)
        })?;
        Ok(clipboard.GetClipboardSequenceNumber())
    }

    let deadline = request.deadline.min(Instant::now() + OPEN_DEADLINE);
    let before = sequence(deadline)?;
    let clipboard = retry_clipboard_open(deadline, || {
        uiautomation::clipboards::Clipboard::open().map_err(|_| DeliveryRefusal::ClipboardBusy)
    })?;
    clipboard
        .set_text(&request.text)
        .map_err(|_| DeliveryRefusal::ClipboardBusy)?;
    drop(clipboard);
    let after = sequence(deadline)?;
    if after == before {
        return Err(DeliveryRefusal::ClipboardChanged);
    }
    Ok(DeliveryReceipt {
        session_id: request.session_id,
        capability: DeliveryCapability::ClipboardOnly,
        strategy: DeliveryStrategy::Clipboard,
        outcome: DeliveryOutcome::ClipboardWritten,
        clipboard_sequence: Some(after),
        input_events_accepted: None,
        consumption_verified: false,
    })
}

#[cfg(not(windows))]
fn write_clipboard(_request: &ClipboardRequest) -> Result<DeliveryReceipt, DeliveryRefusal> {
    Err(DeliveryRefusal::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_open_retries_transient_contention() {
        let mut attempts = 0;
        let result = retry_clipboard_open(Instant::now() + Duration::from_secs(1), || {
            attempts += 1;
            if attempts < 3 {
                Err(DeliveryRefusal::ClipboardBusy)
            } else {
                Ok(42)
            }
        });
        assert_eq!(result, Ok(42));
        assert_eq!(attempts, 3);
    }

    #[test]
    fn clipboard_open_stops_at_attempt_limit() {
        let mut attempts = 0;
        let result = retry_clipboard_open::<()>(Instant::now() + Duration::from_secs(1), || {
            attempts += 1;
            Err(DeliveryRefusal::ClipboardBusy)
        });
        assert_eq!(result, Err(DeliveryRefusal::ClipboardBusy));
        assert_eq!(attempts, usize::from(MAXIMUM_OPEN_ATTEMPTS));
    }

    #[test]
    fn clipboard_open_does_not_retry_past_deadline() {
        let mut attempts = 0;
        let result = retry_clipboard_open::<()>(Instant::now(), || {
            attempts += 1;
            Err(DeliveryRefusal::ClipboardBusy)
        });
        assert_eq!(result, Err(DeliveryRefusal::ClipboardBusy));
        assert_eq!(attempts, 1);
    }
}
