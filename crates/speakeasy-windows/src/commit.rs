use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use speakeasy_delivery::{DeliveryPlan, TargetObservation, classify_guard, validate_target};
use speakeasy_domain::{
    DeliveryCapability, DeliveryOutcome, DeliveryReceipt, DeliveryRefusal, DeliveryStrategy,
    TargetSnapshot,
};

use crate::{
    ClipboardWriter, activity::OWN_INPUT_TAG, capture_input_activity, wait_for_activation_modifiers,
};

const PASTE_EVENT_COUNT: u32 = 4;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(100);

enum CommitMode {
    /// Pre-activation snapshot plus full invalidation evidence.
    Validated {
        plan: DeliveryPlan,
        observation: TargetObservation,
    },
    /// Snapshot observed at commit time for explicit hotkey dictation.
    FocusedNow,
}

struct CommitRequest {
    mode: CommitMode,
    snapshot: TargetSnapshot,
    text: String,
    deadline: Instant,
    response: SyncSender<Result<DeliveryReceipt, DeliveryRefusal>>,
}

pub struct CommitWriter {
    requests: Option<SyncSender<CommitRequest>>,
    worker: Option<JoinHandle<()>>,
}

impl CommitWriter {
    /// Starts a bounded commit-on-finish worker without registering it with the desktop.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::Unsupported`] when the worker cannot start.
    pub fn spawn() -> Result<Self, DeliveryRefusal> {
        let (requests, receiver) = mpsc::sync_channel(2);
        let worker = thread::Builder::new()
            .name("speakeasy-commit".to_owned())
            .spawn(move || run_worker(&receiver))
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        Ok(Self {
            requests: Some(requests),
            worker: Some(worker),
        })
    }

    /// Executes one preselected clipboard-paste plan after complete revalidation.
    ///
    /// This adapter reports only that Windows accepted the tagged input events;
    /// it never claims that the target consumed or inserted the clipboard text.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for stale evidence, incomplete keyboard/activity
    /// evidence, queue pressure, clipboard failure, held modifiers, or ambiguous
    /// synthesized input.
    pub fn write(
        &self,
        plan: DeliveryPlan,
        snapshot: TargetSnapshot,
        observation: TargetObservation,
        text: String,
        deadline: Instant,
    ) -> Result<DeliveryReceipt, DeliveryRefusal> {
        self.submit(CommitRequest {
            mode: CommitMode::Validated { plan, observation },
            snapshot,
            text,
            deadline,
            response: mpsc::sync_channel(1).0,
        })
    }

    /// Pastes a final transcript into the target observed at commit time.
    ///
    /// The caller must observe `snapshot` after dictation ended, because an
    /// explicit activation hotkey necessarily changes the pre-activation input
    /// and hook epochs. Security guards, modifier release, and the tagged
    /// single `Ctrl+V` contract still apply.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal for guarded targets, incomplete keyboard/caret
    /// evidence, queue pressure, clipboard failure, held modifiers, or ambiguous
    /// synthesized input.
    pub fn write_focused(
        &self,
        snapshot: TargetSnapshot,
        text: String,
        deadline: Instant,
    ) -> Result<DeliveryReceipt, DeliveryRefusal> {
        self.submit(CommitRequest {
            mode: CommitMode::FocusedNow,
            snapshot,
            text,
            deadline,
            response: mpsc::sync_channel(1).0,
        })
    }

    fn submit(&self, mut request: CommitRequest) -> Result<DeliveryReceipt, DeliveryRefusal> {
        let deadline = request.deadline;
        let (response, result) = mpsc::sync_channel(1);
        request.response = response;
        self.requests
            .as_ref()
            .ok_or(DeliveryRefusal::Unsupported)?
            .try_send(request)
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        result
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| DeliveryRefusal::Unsupported)?
    }
}

impl Drop for CommitWriter {
    fn drop(&mut self) {
        self.requests.take();
        // The worker may be inside a clipboard or input OS call. Detach a live
        // worker rather than allowing shutdown to wait forever for it.
        drop(self.worker.take());
    }
}

fn run_worker(receiver: &Receiver<CommitRequest>) {
    let clipboard = ClipboardWriter::spawn();
    loop {
        let request = match receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let result = clipboard
            .as_ref()
            .map_err(|error| *error)
            .and_then(|clipboard| execute_commit(clipboard, &request));
        let _ = request.response.send(result);
    }
}

fn execute_commit(
    clipboard: &ClipboardWriter,
    request: &CommitRequest,
) -> Result<DeliveryReceipt, DeliveryRefusal> {
    let strategy = match &request.mode {
        CommitMode::Validated { plan, .. } => plan.strategy,
        CommitMode::FocusedNow => DeliveryStrategy::ClipboardPaste,
    };
    match &request.mode {
        CommitMode::Validated { .. } => {
            validate_commit_preflight(request)?;
            input_epoch_unchanged(&request.snapshot)?;
            wait_for_activation_modifiers(request.deadline)?;
            input_epoch_unchanged(&request.snapshot)?;
        }
        CommitMode::FocusedNow => {
            validate_focused_preflight(&request.snapshot)?;
            wait_for_activation_modifiers(request.deadline)?;
        }
    }

    let clipboard_receipt = clipboard.write_until(
        request.snapshot.session_id,
        request.text.clone(),
        request.deadline,
    )?;
    if matches!(request.mode, CommitMode::Validated { .. }) {
        input_epoch_unchanged(&request.snapshot)?;
    }
    let accepted = send_paste_shortcut()?;
    if accepted != PASTE_EVENT_COUNT {
        return Err(DeliveryRefusal::AmbiguousInput);
    }
    Ok(DeliveryReceipt {
        session_id: request.snapshot.session_id,
        capability: request.snapshot.capability,
        strategy,
        outcome: DeliveryOutcome::InputQueued,
        clipboard_sequence: clipboard_receipt.clipboard_sequence,
        input_events_accepted: Some(accepted),
        consumption_verified: false,
    })
}

fn validate_focused_preflight(snapshot: &TargetSnapshot) -> Result<(), DeliveryRefusal> {
    classify_guard(snapshot)?;
    if matches!(
        snapshot.capability,
        DeliveryCapability::ResultViewOnly | DeliveryCapability::AppendOnlyLive
    ) {
        return Err(DeliveryRefusal::Unsupported);
    }
    Ok(())
}

fn validate_commit_preflight(request: &CommitRequest) -> Result<(), DeliveryRefusal> {
    let CommitMode::Validated { plan, observation } = &request.mode else {
        return Err(DeliveryRefusal::Unsupported);
    };
    if plan.session_id != request.snapshot.session_id
        || plan.capability != request.snapshot.capability
        || plan.strategy != DeliveryStrategy::ClipboardPaste
        || request.snapshot.capability != DeliveryCapability::CommitOnFinish
    {
        return Err(DeliveryRefusal::Unsupported);
    }
    validate_target(&request.snapshot, *observation)?;
    classify_guard(&request.snapshot)?;
    if request.snapshot.keyboard.layout.is_none()
        || request.snapshot.keyboard.ime_open.is_none()
        || request.snapshot.keyboard.ime_composing != Some(false)
    {
        return Err(DeliveryRefusal::Unsupported);
    }
    let selection = request
        .snapshot
        .selection
        .as_ref()
        .ok_or(DeliveryRefusal::Unsupported)?;
    if selection.start.is_none()
        || selection.end.is_none()
        || selection.caret.is_none()
        || selection.range_fingerprint.is_none()
        || request.snapshot.content_fingerprint.is_none()
    {
        return Err(DeliveryRefusal::Unsupported);
    }
    request
        .snapshot
        .input_epoch
        .ok_or(DeliveryRefusal::HookUnavailable)?;
    request
        .snapshot
        .hook_epoch
        .ok_or(DeliveryRefusal::HookUnavailable)?;
    Ok(())
}

fn input_epoch_unchanged(snapshot: &TargetSnapshot) -> Result<(), DeliveryRefusal> {
    let captured = snapshot
        .input_epoch
        .ok_or(DeliveryRefusal::HookUnavailable)?;
    let current = capture_input_activity()?.tick();
    if current == captured {
        Ok(())
    } else {
        Err(DeliveryRefusal::UserInput)
    }
}

#[cfg(windows)]
fn paste_events() -> [winsafe::HwKbMouse; PASTE_EVENT_COUNT as usize] {
    use winsafe::{HwKbMouse, KEYBDINPUT, co};

    let key = |virtual_key, flags| {
        HwKbMouse::Kb(KEYBDINPUT {
            wVk: virtual_key,
            dwFlags: flags,
            dwExtraInfo: OWN_INPUT_TAG,
            ..KEYBDINPUT::default()
        })
    };
    [
        key(co::VK::CONTROL, co::KEYEVENTF::NoValue),
        key(co::VK::CHAR_V, co::KEYEVENTF::NoValue),
        key(co::VK::CHAR_V, co::KEYEVENTF::KEYUP),
        key(co::VK::CONTROL, co::KEYEVENTF::KEYUP),
    ]
}

#[cfg(windows)]
fn send_paste_shortcut() -> Result<u32, DeliveryRefusal> {
    winsafe::SendInput(&paste_events()).map_err(|_| DeliveryRefusal::AmbiguousInput)
}

#[cfg(not(windows))]
fn send_paste_shortcut() -> Result<u32, DeliveryRefusal> {
    Err(DeliveryRefusal::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use speakeasy_domain::{
        ExecutableIdentity, IntegrityRelationship, KeyboardContext, SessionId, TargetKind,
        UiaPatterns,
    };

    fn request() -> CommitRequest {
        let session_id = SessionId::from_bytes([7; 16]);
        CommitRequest {
            mode: CommitMode::Validated {
                plan: DeliveryPlan {
                    session_id,
                    capability: DeliveryCapability::CommitOnFinish,
                    strategy: DeliveryStrategy::ClipboardPaste,
                },
                observation: TargetObservation {
                    session_id,
                    window_handle: 1,
                    process_id: 2,
                    process_start_time: 4,
                    thread_id: 3,
                    element_matches: true,
                    selection_matches: true,
                    caret_matches: true,
                    content_matches: true,
                    integrity_matches: true,
                    keyboard_matches: true,
                    foreground: true,
                    user_input_seen: false,
                    hook_healthy: true,
                    own_input_distinguished: true,
                    hook_epoch: Some(6),
                    modifiers_released: true,
                },
            },
            snapshot: TargetSnapshot {
                session_id,
                window_handle: 1,
                process_id: 2,
                thread_id: 3,
                executable: ExecutableIdentity {
                    path: "notepad.exe".to_owned(),
                    process_start_time: 4,
                },
                integrity: IntegrityRelationship::Equal,
                element: None,
                target_kind: TargetKind::Standard,
                is_password: false,
                is_read_only: false,
                is_secure_desktop: Some(false),
                patterns: UiaPatterns::default(),
                selection: Some(speakeasy_domain::SelectionSnapshot {
                    start: Some(0),
                    end: Some(0),
                    caret: Some(0),
                    is_empty: true,
                    range_fingerprint: Some([8; 32]),
                }),
                content_fingerprint: Some([9; 32]),
                input_epoch: Some(5),
                hook_epoch: Some(6),
                keyboard: KeyboardContext {
                    layout: Some(6),
                    ime_open: Some(false),
                    ime_composing: Some(false),
                },
                capability: DeliveryCapability::CommitOnFinish,
            },
            text: "safe final".to_owned(),
            deadline: Instant::now(),
            response: mpsc::sync_channel(1).0,
        }
    }

    fn observation_mut(request: &mut CommitRequest) -> &mut TargetObservation {
        match &mut request.mode {
            CommitMode::Validated { observation, .. } => observation,
            CommitMode::FocusedNow => panic!("validated commit request expected"),
        }
    }

    #[test]
    fn focused_commit_refuses_guarded_and_result_view_only_targets() {
        let mut current = request();
        assert_eq!(validate_focused_preflight(&current.snapshot), Ok(()));

        current.snapshot.capability = DeliveryCapability::ResultViewOnly;
        assert_eq!(
            validate_focused_preflight(&current.snapshot),
            Err(DeliveryRefusal::Unsupported)
        );
        current.snapshot.capability = DeliveryCapability::CommitOnFinish;
        current.snapshot.is_password = true;
        assert_eq!(
            validate_focused_preflight(&current.snapshot),
            Err(DeliveryRefusal::Password)
        );
        current.snapshot.is_password = false;
        current.snapshot.is_secure_desktop = Some(true);
        assert_eq!(
            validate_focused_preflight(&current.snapshot),
            Err(DeliveryRefusal::SecureDesktop)
        );
    }

    #[test]
    fn commit_preflight_requires_complete_activity_keyboard_and_hook_evidence() {
        let mut current = request();
        assert_eq!(validate_commit_preflight(&current), Ok(()));

        current.snapshot.input_epoch = None;
        assert_eq!(
            validate_commit_preflight(&current),
            Err(DeliveryRefusal::HookUnavailable)
        );
        current.snapshot.input_epoch = Some(5);
        current.snapshot.hook_epoch = None;
        assert_eq!(
            validate_commit_preflight(&current),
            Err(DeliveryRefusal::HookUnavailable)
        );
        current.snapshot.hook_epoch = Some(6);
        observation_mut(&mut current).hook_healthy = false;
        assert_eq!(
            validate_commit_preflight(&current),
            Err(DeliveryRefusal::HookUnavailable)
        );
        observation_mut(&mut current).hook_healthy = true;
        current.snapshot.keyboard.ime_composing = None;
        assert_eq!(
            validate_commit_preflight(&current),
            Err(DeliveryRefusal::Unsupported)
        );
    }

    #[cfg(windows)]
    #[test]
    fn paste_events_are_exactly_one_tagged_ctrl_v_without_enter() {
        let events = paste_events();
        assert_eq!(events.len(), PASTE_EVENT_COUNT as usize);
        let keys = events.map(|event| match event {
            winsafe::HwKbMouse::Kb(key) => key,
            _ => panic!("paste event must be keyboard input"),
        });
        assert!(keys.iter().all(|key| key.dwExtraInfo == OWN_INPUT_TAG));
        assert_eq!(keys[0].wVk, winsafe::co::VK::CONTROL);
        assert_eq!(keys[1].wVk, winsafe::co::VK::CHAR_V);
        assert_eq!(keys[2].wVk, winsafe::co::VK::CHAR_V);
        assert_eq!(keys[3].wVk, winsafe::co::VK::CONTROL);
        assert!(keys.iter().all(|key| key.wVk != winsafe::co::VK::RETURN));
    }
}
