use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sha2::{Digest, Sha256};
use speakeasy_delivery::TargetObservation;
use speakeasy_domain::{
    DeliveryCapability, DeliveryRefusal, ExecutableIdentity, IntegrityRelationship,
    KeyboardContext, SelectionSnapshot, SessionId, TargetKind, TargetSnapshot, UiaElementIdentity,
    UiaPatterns,
};

use crate::{
    ActivityHookEvidence, ActivityMonitor, activation_modifiers_released, capture_input_activity,
};

const MAXIMUM_FINGERPRINT_CHARACTERS: i32 = 4_096;
const MAXIMUM_VISIBLE_RANGES: usize = 16;
const MAXIMUM_RANGE_OFFSET: i32 = 1_000_000;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(100);

thread_local! {
    /// The raw Win32 error from the most recent `inspect_current` call on this
    /// worker thread, when it failed on a raw OS call rather than a UIA
    /// refusal. Read back on the same thread immediately after the call, before
    /// the result crosses the `mpsc` channel to the caller — this exists only
    /// so a logged `TargetInaccessible` carries the numeric OS error that
    /// distinguishes it from a genuine `ElevatedTarget`. New Outlook runs
    /// sandboxed in an AppContainer and denies even
    /// `PROCESS_QUERY_LIMITED_INFORMATION`, which is not the same thing as
    /// actually running elevated, but both used to collapse to one refusal.
    static LAST_INSPECTION_OS_ERROR: Cell<Option<u32>> = const { Cell::new(None) };
}

fn fingerprint_text(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn classify_input_desktop_access(accessible: bool) -> bool {
    !accessible
}

fn bounded_document_offset(moved: i32) -> Option<i32> {
    (moved <= 0 && moved > -MAXIMUM_RANGE_OFFSET).then_some(-moved)
}

#[cfg(windows)]
fn observe_secure_desktop() -> bool {
    classify_input_desktop_access(
        winsafe::HDESK::OpenInputDesktop(None, false, winsafe::co::DESKTOP_RIGHTS::READOBJECTS)
            .is_ok(),
    )
}

#[derive(Debug)]
struct InspectRequest {
    session_id: SessionId,
    activity: ActivityHookEvidence,
    response: SyncSender<Result<TargetSnapshot, DeliveryRefusal>>,
}

pub struct TargetObserver {
    activity: ActivityMonitor,
    requests: Option<SyncSender<InspectRequest>>,
    worker: Option<JoinHandle<()>>,
    last_os_error: Arc<Mutex<Option<u32>>>,
}

impl TargetObserver {
    /// Starts the bounded, windowless UI Automation observation worker.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::Unsupported`] when the worker thread cannot start.
    pub fn spawn() -> Result<Self, DeliveryRefusal> {
        let activity = ActivityMonitor::spawn()?;
        let (requests, receiver) = mpsc::sync_channel(4);
        let last_os_error = Arc::new(Mutex::new(None));
        let worker_last_os_error = Arc::clone(&last_os_error);
        let worker = thread::Builder::new()
            .name("speakeasy-uia-mta".to_owned())
            .spawn(move || run_worker(&receiver, &worker_last_os_error))
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        Ok(Self {
            activity,
            requests: Some(requests),
            worker: Some(worker),
            last_os_error,
        })
    }

    /// The numeric Win32 error behind the most recent `TargetInaccessible`
    /// refusal, if the last inspection failed that way. Sanitized: a bare
    /// `GetLastError` code carries no target content, unlike the OS-provided
    /// message text this deliberately drops. Exists so the disk log can say
    /// *why* an OS call denied access instead of only naming the refusal.
    pub fn last_os_error(&self) -> Option<u32> {
        self.last_os_error.lock().ok().and_then(|guard| *guard)
    }

    /// How long a single UIA inspection may take before it is abandoned.
    ///
    /// Generous on purpose. A snapshot's cost is set by the *target* application's
    /// accessibility implementation, not by ours, and measured real cost spans two
    /// orders of magnitude: 68 ms into an empty Notepad, 1.7 s into VS Code, 12.8 s
    /// into a `WebView2` window. A tight deadline would therefore refuse delivery
    /// into ordinary editors, which is a worse failure than waiting. This exists
    /// only to stop a wedged or hostile provider blocking forever — which is what
    /// an unbounded `recv` did.
    const INSPECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

    /// Captures the current foreground target and focused UIA element.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the bounded queue, worker, foreground
    /// identity, process security, or focused element cannot be observed safely,
    /// or when the inspection exceeds the inspection deadline.
    pub fn inspect(&self, session_id: SessionId) -> Result<TargetSnapshot, DeliveryRefusal> {
        let (response, result) = mpsc::sync_channel(1);
        self.requests
            .as_ref()
            .ok_or(DeliveryRefusal::Unsupported)?
            .try_send(InspectRequest {
                session_id,
                activity: self.activity.evidence(),
                response,
            })
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        // Bounded rather than an unbounded receive. A UIA call reaches into another process
        // and can hang there indefinitely; when it did, it took the caller with
        // it. Timing out is reported as `Unsupported` — the same refusal an
        // unobservable target already gives — so a caller that cannot get a
        // snapshot behaves identically whether the provider refused or stalled.
        result
            .recv_timeout(Self::INSPECT_DEADLINE)
            .map_err(|_| DeliveryRefusal::Unsupported)?
    }

    /// Reobserves the foreground target and compares all available invalidation evidence.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the current target cannot be observed.
    pub fn observe(&self, snapshot: &TargetSnapshot) -> Result<TargetObservation, DeliveryRefusal> {
        let current = self.inspect(snapshot.session_id)?;
        let captured_selection = snapshot.selection.as_ref();
        let current_selection = current.selection.as_ref();
        Ok(TargetObservation {
            session_id: current.session_id,
            window_handle: current.window_handle,
            process_id: current.process_id,
            process_start_time: current.executable.process_start_time,
            thread_id: current.thread_id,
            element_matches: current.element == snapshot.element,
            selection_matches: selection_range_matches(captured_selection, current_selection),
            caret_matches: captured_selection.and_then(|value| value.caret)
                == current_selection.and_then(|value| value.caret),
            content_matches: current.content_fingerprint == snapshot.content_fingerprint,
            integrity_matches: current.integrity == snapshot.integrity,
            keyboard_matches: current.keyboard == snapshot.keyboard,
            foreground: current.window_handle == snapshot.window_handle,
            user_input_seen: current.input_epoch != snapshot.input_epoch
                || current.hook_epoch != snapshot.hook_epoch,
            hook_healthy: current.hook_epoch.is_some(),
            // The current safe winsafe boundary cannot inspect KBDLLHOOKSTRUCT
            // extra-info without adding workspace-forbidden unsafe code.
            // Keep automatic/live mutation unreachable instead of guessing.
            own_input_distinguished: false,
            hook_epoch: current.hook_epoch,
            modifiers_released: activation_modifiers_released(),
        })
    }
}

fn selection_range_matches(
    captured: Option<&SelectionSnapshot>,
    current: Option<&SelectionSnapshot>,
) -> bool {
    match (captured, current) {
        (Some(captured), Some(current)) => {
            captured.start == current.start
                && captured.end == current.end
                && captured.is_empty == current.is_empty
                && captured.range_fingerprint == current.range_fingerprint
        }
        (None, None) => true,
        _ => false,
    }
}

fn classify_capability(snapshot: &TargetSnapshot) -> DeliveryCapability {
    if speakeasy_delivery::classify_guard(snapshot).is_err() {
        return DeliveryCapability::ResultViewOnly;
    }
    if snapshot.target_kind == TargetKind::Terminal {
        return DeliveryCapability::ClipboardOnly;
    }
    let complete_range = snapshot.selection.as_ref().is_some_and(|selection| {
        selection.start.is_some()
            && selection.end.is_some()
            && selection.caret.is_some()
            && selection.range_fingerprint.is_some()
    });
    if snapshot.patterns.text
        && complete_range
        && snapshot.content_fingerprint.is_some()
        && snapshot.input_epoch.is_some()
        && snapshot.hook_epoch.is_some()
        && snapshot.keyboard.layout.is_some()
        && snapshot.keyboard.ime_open.is_some()
        && snapshot.keyboard.ime_composing == Some(false)
    {
        DeliveryCapability::CommitOnFinish
    } else {
        DeliveryCapability::ClipboardOnly
    }
}

impl Drop for TargetObserver {
    fn drop(&mut self) {
        self.requests.take();
        // UIA calls execute inside another process and cannot be cancelled from
        // this thread. Dropping a live handle detaches it; joining here would
        // turn the already-bounded inspect timeout back into an exit hang.
        drop(self.worker.take());
    }
}

fn run_worker(receiver: &Receiver<InspectRequest>, last_os_error: &Mutex<Option<u32>>) {
    loop {
        let request = match receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let result = inspect_current(request.session_id, request.activity);
        if result.is_err()
            && let Ok(mut guard) = last_os_error.lock()
        {
            *guard = LAST_INSPECTION_OS_ERROR.with(Cell::get);
        }
        let _ = request.response.send(result);
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_lines)]
fn inspect_current(
    session_id: SessionId,
    activity: ActivityHookEvidence,
) -> Result<TargetSnapshot, DeliveryRefusal> {
    use uiautomation::UIAutomation;
    use uiautomation::patterns::{UITextPattern, UITextRange, UIValuePattern};
    use uiautomation::types::{TextPatternRangeEndpoint, TextUnit};

    fn document_offset(range: &UITextRange, endpoint: TextPatternRangeEndpoint) -> Option<i32> {
        let probe = range.clone();
        match endpoint {
            TextPatternRangeEndpoint::Start => probe
                .move_endpoint_by_range(
                    TextPatternRangeEndpoint::End,
                    &probe,
                    TextPatternRangeEndpoint::Start,
                )
                .ok()?,
            TextPatternRangeEndpoint::End => probe
                .move_endpoint_by_range(
                    TextPatternRangeEndpoint::Start,
                    &probe,
                    TextPatternRangeEndpoint::End,
                )
                .ok()?,
        }
        bounded_document_offset(
            probe
                .move_text(TextUnit::Character, -MAXIMUM_RANGE_OFFSET)
                .ok()?,
        )
    }

    /// Queries whether `process`'s token is elevated (integrity High or
    /// System).
    ///
    /// This deliberately queries `TokenIntegrityLevel` rather than the more
    /// obvious `TokenElevation`. `winsafe`'s generic `GetTokenInformation`
    /// always probes the required buffer size with a zero-length call
    /// first, which is correct for variable-size info classes but not for
    /// `TokenElevation`: that class is a fixed 4-byte value, and Windows
    /// answers a zero-length probe against it with `ERROR_BAD_LENGTH`
    /// instead of the `ERROR_INSUFFICIENT_BUFFER` the generic path expects,
    /// so the probe always fails closed. `TokenIntegrityLevel` carries a
    /// variable-length SID and does not have this problem, and its label
    /// (Low/Medium/High/System) is an equivalent signal for this check.
    fn elevated(process: &winsafe::HPROCESS) -> Option<bool> {
        let token = process.OpenProcessToken(winsafe::co::TOKEN::QUERY).ok()?;
        let information = token
            .GetTokenInformation(winsafe::co::TOKEN_INFORMATION_CLASS::IntegrityLevel)
            .ok()?;
        let winsafe::TokenInfo::IntegrityLevel(label) = information else {
            return None;
        };
        let sid = label.Label.Sid()?;
        Some(
            winsafe::IsWellKnownSid(sid, winsafe::co::WELL_KNOWN_SID_TYPE::HighLabel)
                || winsafe::IsWellKnownSid(sid, winsafe::co::WELL_KNOWN_SID_TYPE::SystemLabel),
        )
    }

    LAST_INSPECTION_OS_ERROR.with(|cell| cell.set(None));
    let foreground = winsafe::HWND::GetForegroundWindow().ok_or(DeliveryRefusal::Unsupported)?;
    let (thread_id, process_id) = foreground.GetWindowThreadProcessId();
    // A failure here is not necessarily a higher-integrity target: a packaged,
    // AppContainer-sandboxed process (New Outlook for Windows is one) commonly
    // denies even `PROCESS_QUERY_LIMITED_INFORMATION` to an unpackaged caller
    // with `ERROR_ACCESS_DENIED`, which is a sandbox boundary, not elevation.
    // `ElevatedTarget` stays reserved for the case where a snapshot was
    // actually obtained and the integrity comparison found the target higher.
    let process = winsafe::HPROCESS::OpenProcess(
        winsafe::co::PROCESS::QUERY_LIMITED_INFORMATION,
        false,
        process_id,
    )
    .map_err(|error| {
        LAST_INSPECTION_OS_ERROR.with(|cell| cell.set(Some(u32::from(error))));
        DeliveryRefusal::TargetInaccessible
    })?;
    let process_start_time = process
        .GetProcessTimes()
        .map(|times| u64::from(times.0))
        .map_err(|_| DeliveryRefusal::Unsupported)?;
    let target_elevated = elevated(&process);
    let current_process = winsafe::HPROCESS::GetCurrentProcess();
    let current_elevated = elevated(&current_process);
    let integrity = match (current_elevated, target_elevated) {
        (Some(false), Some(true)) => IntegrityRelationship::TargetHigher,
        (Some(true), Some(false)) => IntegrityRelationship::TargetLower,
        (Some(left), Some(right)) if left == right => IntegrityRelationship::Equal,
        _ => IntegrityRelationship::Unknown,
    };

    let automation = UIAutomation::new().map_err(|_| DeliveryRefusal::Unsupported)?;
    let element = automation
        .get_focused_element()
        .map_err(|_| DeliveryRefusal::Unsupported)?;
    if element.get_process_id().ok() != Some(process_id) {
        return Err(DeliveryRefusal::ElementChanged);
    }

    let value_pattern = element.get_pattern::<UIValuePattern>().ok();
    let text_pattern = element.get_pattern::<UITextPattern>().ok();
    let text = text_pattern.is_some();
    let text2 = text_pattern
        .as_ref()
        .is_some_and(|pattern| pattern.get_caret_range().is_ok());
    let selection = text_pattern.as_ref().and_then(|pattern| {
        let ranges = pattern.get_selection().ok()?;
        if ranges.len() != 1 {
            return None;
        }
        let range = &ranges[0];
        let selected_text = range.get_text(MAXIMUM_FINGERPRINT_CHARACTERS).ok()?;
        let caret = pattern.get_caret_range().ok().and_then(|(active, range)| {
            active
                .then(|| document_offset(&range, TextPatternRangeEndpoint::Start))
                .flatten()
        });
        let caret_active = caret.is_some();
        (caret_active || !selected_text.is_empty()).then_some(SelectionSnapshot {
            start: document_offset(range, TextPatternRangeEndpoint::Start),
            end: document_offset(range, TextPatternRangeEndpoint::End),
            caret,
            is_empty: selected_text.is_empty(),
            range_fingerprint: Some(fingerprint_text(&selected_text)),
        })
    });
    let content_fingerprint = text_pattern.as_ref().and_then(|pattern| {
        let ranges = pattern.get_visible_ranges().ok()?;
        let mut hasher = Sha256::new();
        let mut observed = false;
        for range in ranges.iter().take(MAXIMUM_VISIBLE_RANGES) {
            let text = range.get_text(MAXIMUM_FINGERPRINT_CHARACTERS).ok()?;
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text.as_bytes());
            observed = true;
        }
        observed.then(|| hasher.finalize().into())
    });
    let is_password = element.is_password().unwrap_or(true);
    let is_read_only = value_pattern
        .as_ref()
        .map_or(!element.is_enabled().unwrap_or(false), |pattern| {
            pattern.is_readonly().unwrap_or(true)
        });
    let path = process
        .QueryFullProcessImageName(winsafe::co::PROCESS_NAME::default())
        .map_err(|_| DeliveryRefusal::Unsupported)?;
    let path_lower = path.to_ascii_lowercase();
    let target_kind = if path_lower.ends_with("windowsterminal.exe")
        || path_lower.ends_with("powershell.exe")
        || path_lower.ends_with("pwsh.exe")
        || path_lower.ends_with("cmd.exe")
    {
        TargetKind::Terminal
    } else {
        TargetKind::Standard
    };
    let window_handle = format!("{foreground:x}")
        .trim_start_matches("0x")
        .parse::<u64>()
        .unwrap_or_default();

    let mut snapshot = TargetSnapshot {
        session_id,
        window_handle,
        process_id,
        thread_id,
        executable: ExecutableIdentity {
            path,
            process_start_time,
        },
        integrity,
        element: Some(UiaElementIdentity {
            runtime_id: element
                .get_runtime_id()
                .map_err(|_| DeliveryRefusal::Unsupported)?,
            control_type: element
                .get_control_type()
                .map_err(|_| DeliveryRefusal::Unsupported)? as u32,
            class_name: element.get_classname().unwrap_or_default(),
        }),
        target_kind,
        is_password,
        is_read_only,
        is_secure_desktop: Some(observe_secure_desktop()),
        patterns: UiaPatterns {
            text,
            text2,
            value: value_pattern.is_some(),
        },
        selection,
        content_fingerprint,
        input_epoch: capture_input_activity()
            .ok()
            .map(crate::InputActivityEpoch::tick),
        hook_epoch: activity.healthy.then_some(activity.epoch),
        keyboard: KeyboardContext {
            layout: None,
            ime_open: None,
            ime_composing: None,
        },
        capability: DeliveryCapability::ResultViewOnly,
    };
    snapshot.capability = classify_capability(&snapshot);
    Ok(snapshot)
}

#[cfg(not(windows))]
fn inspect_current(
    _session_id: SessionId,
    _activity: ActivityHookEvidence,
) -> Result<TargetSnapshot, DeliveryRefusal> {
    Err(DeliveryRefusal::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_snapshot() -> TargetSnapshot {
        TargetSnapshot {
            session_id: SessionId::from_bytes([1; 16]),
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
            patterns: UiaPatterns {
                text: true,
                text2: true,
                value: true,
            },
            selection: Some(SelectionSnapshot {
                start: Some(0),
                end: Some(0),
                caret: Some(0),
                is_empty: true,
                range_fingerprint: Some([5; 32]),
            }),
            content_fingerprint: Some([6; 32]),
            input_epoch: Some(7),
            hook_epoch: Some(8),
            keyboard: KeyboardContext {
                layout: Some(9),
                ime_open: Some(false),
                ime_composing: Some(false),
            },
            capability: DeliveryCapability::ResultViewOnly,
        }
    }

    #[test]
    fn fingerprints_are_deterministic_and_content_sensitive() {
        assert_eq!(
            fingerprint_text("résumé\r\n🙂"),
            fingerprint_text("résumé\r\n🙂")
        );
        assert_ne!(
            fingerprint_text("selection a"),
            fingerprint_text("selection b")
        );
    }

    #[test]
    fn inaccessible_input_desktop_is_classified_as_secure() {
        assert!(!classify_input_desktop_access(true));
        assert!(classify_input_desktop_access(false));
    }

    #[test]
    fn document_offsets_are_accepted_only_inside_the_bound() {
        assert_eq!(bounded_document_offset(-42), Some(42));
        assert_eq!(bounded_document_offset(0), Some(0));
        assert_eq!(bounded_document_offset(1), None);
        assert_eq!(bounded_document_offset(-MAXIMUM_RANGE_OFFSET), None);
    }

    #[test]
    fn range_revalidation_compares_offsets_shape_and_fingerprint() {
        let fingerprint = Some(fingerprint_text("selected"));
        let captured = SelectionSnapshot {
            start: Some(1),
            end: Some(9),
            caret: Some(9),
            is_empty: false,
            range_fingerprint: fingerprint,
        };
        assert!(selection_range_matches(Some(&captured), Some(&captured)));
        let changed = SelectionSnapshot {
            end: Some(10),
            ..captured.clone()
        };
        assert!(!selection_range_matches(Some(&captured), Some(&changed)));
    }

    #[test]
    fn dropping_a_stuck_worker_does_not_wait_for_uia() {
        let started = std::time::Instant::now();
        let observer = TargetObserver {
            activity: ActivityMonitor::placeholder_for_test(),
            requests: None,
            worker: Some(thread::spawn(|| thread::sleep(Duration::from_millis(250)))),
            last_os_error: Arc::new(Mutex::new(None)),
        };
        drop(observer);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn capability_requires_complete_commit_evidence_and_refuses_sensitive_targets() {
        let mut snapshot = capability_snapshot();
        assert_eq!(
            classify_capability(&snapshot),
            DeliveryCapability::CommitOnFinish
        );
        snapshot.keyboard.layout = None;
        assert_eq!(
            classify_capability(&snapshot),
            DeliveryCapability::ClipboardOnly
        );
        snapshot.is_password = true;
        assert_eq!(
            classify_capability(&snapshot),
            DeliveryCapability::ResultViewOnly
        );
    }
}
