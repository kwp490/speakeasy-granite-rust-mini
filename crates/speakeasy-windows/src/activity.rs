use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use speakeasy_domain::DeliveryRefusal;

static HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);

const MESSAGE_POLL_INTERVAL: Duration = Duration::from_millis(5);
pub(crate) const OWN_INPUT_TAG: usize = 0x5345_5632;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityHookEvidence {
    pub epoch: u64,
    pub healthy: bool,
}

pub struct ActivityMonitor {
    stop: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ActivityMonitor {
    #[cfg(test)]
    pub(crate) fn placeholder_for_test() -> Self {
        Self {
            stop: None,
            worker: None,
        }
    }

    /// Starts singleton low-level keyboard and mouse hooks on a message-loop worker.
    ///
    /// Hook callbacks retain no key, pointer, target, or document content.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::HookUnavailable`] when hooks cannot be installed
    /// or another monitor already owns them.
    pub fn spawn() -> Result<Self, DeliveryRefusal> {
        if HOOK_ACTIVE.swap(true, Ordering::AcqRel) {
            return Err(DeliveryRefusal::HookUnavailable);
        }
        let (stop, stop_receiver) = mpsc::sync_channel(1);
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("speakeasy-activity-hook".to_owned())
            .spawn(move || run_hook_worker(&stop_receiver, &ready))
            .map_err(|_| {
                HOOK_ACTIVE.store(false, Ordering::Release);
                DeliveryRefusal::HookUnavailable
            })?;
        if let Ok(Ok(())) = ready_receiver.recv() {
            Ok(Self {
                stop: Some(stop),
                worker: Some(worker),
            })
        } else {
            let _ = worker.join();
            HOOK_ACTIVE.store(false, Ordering::Release);
            Err(DeliveryRefusal::HookUnavailable)
        }
    }

    #[must_use]
    pub fn evidence(&self) -> ActivityHookEvidence {
        ActivityHookEvidence {
            epoch: HOOK_EPOCH.load(Ordering::Acquire),
            healthy: self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished()),
        }
    }
}

impl Drop for ActivityMonitor {
    fn drop(&mut self) {
        let owned_hook = self.stop.is_some() || self.worker.is_some();
        if let Some(stop) = self.stop.take() {
            let _ = stop.try_send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if owned_hook {
            HOOK_ACTIVE.store(false, Ordering::Release);
        }
    }
}

#[cfg(windows)]
extern "system" fn keyboard_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    // winsafe 0.0.27 does not expose the low-level hook payload through a
    // safe typed API. With workspace unsafe code forbidden, production treats
    // the marker as unavailable and therefore invalidates conservatively.
    record_hook_activity(code, None);
    chain_hook(code, wparam, lparam)
}

#[cfg(windows)]
extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    record_hook_activity(code, None);
    chain_hook(code, wparam, lparam)
}

fn record_hook_activity(code: i32, extra_info: Option<usize>) {
    if code >= 0 && extra_info != Some(OWN_INPUT_TAG) {
        HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(windows)]
fn chain_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    let chain_code = if code < 0 {
        winsafe::co::WH::MSGFILTER
    } else {
        winsafe::co::WH::JOURNALRECORD
    };
    winsafe::CallNextHookEx(chain_code, wparam, lparam)
}

#[cfg(windows)]
fn run_hook_worker(receiver: &mpsc::Receiver<()>, ready: &SyncSender<Result<(), ()>>) {
    let keyboard =
        winsafe::HHOOK::SetWindowsHookEx(winsafe::co::WH::KEYBOARD_LL, keyboard_hook, None, None);
    let mouse = winsafe::HHOOK::SetWindowsHookEx(winsafe::co::WH::MOUSE_LL, mouse_hook, None, None);
    let (Ok(mut keyboard), Ok(mut mouse)) = (keyboard, mouse) else {
        let _ = ready.send(Err(()));
        return;
    };
    let _ = ready.send(Ok(()));
    let mut message = winsafe::MSG::default();
    while receiver.try_recv().is_err() {
        while winsafe::PeekMessage(&mut message, None, 0, 0, winsafe::co::PM::REMOVE) {}
        thread::sleep(MESSAGE_POLL_INTERVAL);
    }
    let _ = keyboard.UnhookWindowsHookEx();
    let _ = mouse.UnhookWindowsHookEx();
}

#[cfg(not(windows))]
fn run_hook_worker(_receiver: &mpsc::Receiver<()>, ready: &SyncSender<Result<(), ()>>) {
    let _ = ready.send(Err(()));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputActivityEpoch(u32);

impl InputActivityEpoch {
    #[must_use]
    pub const fn tick(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn changed_since(self, captured: Self) -> bool {
        self.0 != captured.0
    }
}

/// Captures the system tick of the most recent keyboard or pointer input.
///
/// The observation contains no key, pointer, or document content.
///
/// # Errors
///
/// Returns [`DeliveryRefusal::Unsupported`] when Windows cannot provide an
/// activity epoch or when called on another platform.
pub fn capture_input_activity() -> Result<InputActivityEpoch, DeliveryRefusal> {
    #[cfg(windows)]
    {
        winsafe::GetLastInputInfo()
            .map(|information| InputActivityEpoch(information.dwTime))
            .map_err(|_| DeliveryRefusal::Unsupported)
    }
    #[cfg(not(windows))]
    {
        Err(DeliveryRefusal::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_epoch_detects_change_without_input_content() {
        let captured = InputActivityEpoch(10);
        assert!(!InputActivityEpoch(10).changed_since(captured));
        assert!(InputActivityEpoch(11).changed_since(captured));
        assert!(InputActivityEpoch(0).changed_since(InputActivityEpoch(u32::MAX)));
    }

    #[test]
    fn hook_epoch_changes_only_for_actionable_callbacks() {
        let before = HOOK_EPOCH.load(Ordering::Acquire);
        record_hook_activity(-1, None);
        assert_eq!(HOOK_EPOCH.load(Ordering::Acquire), before);
        record_hook_activity(0, Some(OWN_INPUT_TAG));
        assert_eq!(HOOK_EPOCH.load(Ordering::Acquire), before);
        record_hook_activity(0, None);
        assert_eq!(HOOK_EPOCH.load(Ordering::Acquire), before + 1);
    }

    #[cfg(windows)]
    #[test]
    fn activity_monitor_reports_health_and_enforces_single_ownership() {
        let monitor = ActivityMonitor::spawn().expect("activity hooks");
        assert!(monitor.evidence().healthy);
        assert!(matches!(
            ActivityMonitor::spawn(),
            Err(DeliveryRefusal::HookUnavailable)
        ));
        drop(monitor);
        let replacement = ActivityMonitor::spawn().expect("replacement activity hooks");
        assert!(replacement.evidence().healthy);
    }
}
