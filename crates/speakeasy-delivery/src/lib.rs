//! Bounded target-delivery policy and adapter contracts for `SpeakEasy`.

#![allow(clippy::must_use_candidate)]

mod live;

pub use live::{
    CONTROLLED_APPEND_ADAPTER_ID, CapabilityDecision, CapabilityEvidence, CapabilityRequest,
    InsertionLedger, LedgerBatch, LiveDeliveryAdapter, LiveDeliveryOutcome, LiveDeliveryPolicy,
    LiveDeliveryTransaction, OwnedRange, Reconciliation, reconcile_final,
};

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use speakeasy_domain::{
    DeliveryCapability, DeliveryRefusal, DeliveryStrategy, SessionId, TargetKind, TargetSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct DeliverySettings {
    pub auto_copy: bool,
    pub auto_paste: bool,
    pub restore_clipboard: bool,
    pub feedback_enabled: bool,
}

impl Default for DeliverySettings {
    fn default() -> Self {
        Self {
            auto_copy: false,
            auto_paste: false,
            restore_clipboard: false,
            feedback_enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardPolicy {
    pub maximum_snapshot_bytes: usize,
    pub maximum_snapshot_formats: usize,
    pub maximum_open_attempts: u8,
}

impl ClipboardPolicy {
    pub const fn conservative() -> Self {
        Self {
            maximum_snapshot_bytes: 4 * 1024 * 1024,
            maximum_snapshot_formats: 32,
            maximum_open_attempts: 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardSnapshotMetadata {
    pub bytes: usize,
    pub formats: usize,
    pub complete: bool,
}

impl ClipboardPolicy {
    pub const fn accepts_snapshot(self, snapshot: ClipboardSnapshotMetadata) -> bool {
        snapshot.complete
            && snapshot.bytes <= self.maximum_snapshot_bytes
            && snapshot.formats <= self.maximum_snapshot_formats
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TargetObservation {
    pub session_id: SessionId,
    pub window_handle: u64,
    pub process_id: u32,
    pub process_start_time: u64,
    pub thread_id: u32,
    pub element_matches: bool,
    pub selection_matches: bool,
    pub caret_matches: bool,
    pub content_matches: bool,
    pub integrity_matches: bool,
    pub keyboard_matches: bool,
    pub foreground: bool,
    pub user_input_seen: bool,
    pub hook_healthy: bool,
    pub own_input_distinguished: bool,
    pub hook_epoch: Option<u64>,
    pub modifiers_released: bool,
}

/// Revalidates all captured target identity and invalidation evidence.
///
/// # Errors
///
/// Returns the first typed mismatch; callers must freeze and must not try a writer.
pub fn validate_target(
    snapshot: &TargetSnapshot,
    observation: TargetObservation,
) -> Result<(), DeliveryRefusal> {
    if observation.session_id != snapshot.session_id {
        return Err(DeliveryRefusal::SessionMismatch);
    }
    if !observation.foreground {
        return Err(DeliveryRefusal::FocusChanged);
    }
    if observation.window_handle != snapshot.window_handle {
        return Err(DeliveryRefusal::FocusChanged);
    }
    if observation.process_id != snapshot.process_id {
        return Err(DeliveryRefusal::ProcessChanged);
    }
    if observation.process_start_time != snapshot.executable.process_start_time {
        return Err(DeliveryRefusal::WindowReused);
    }
    if observation.thread_id != snapshot.thread_id || !observation.element_matches {
        return Err(DeliveryRefusal::ElementChanged);
    }
    if !observation.selection_matches {
        return Err(DeliveryRefusal::SelectionChanged);
    }
    if !observation.caret_matches {
        return Err(DeliveryRefusal::CaretChanged);
    }
    if !observation.content_matches {
        return Err(DeliveryRefusal::ContentChanged);
    }
    if !observation.integrity_matches {
        return Err(DeliveryRefusal::IntegrityChanged);
    }
    if !observation.keyboard_matches {
        return Err(DeliveryRefusal::ContentChanged);
    }
    if observation.user_input_seen {
        return Err(DeliveryRefusal::UserInput);
    }
    if !observation.hook_healthy {
        return Err(DeliveryRefusal::HookUnavailable);
    }
    if !observation.own_input_distinguished {
        return Err(DeliveryRefusal::HookUnavailable);
    }
    let captured_hook_epoch = snapshot
        .hook_epoch
        .ok_or(DeliveryRefusal::HookUnavailable)?;
    let observed_hook_epoch = observation
        .hook_epoch
        .ok_or(DeliveryRefusal::HookUnavailable)?;
    if observed_hook_epoch != captured_hook_epoch {
        return Err(DeliveryRefusal::UserInput);
    }
    if !observation.modifiers_released {
        return Err(DeliveryRefusal::ModifierHeld);
    }
    Ok(())
}

/// Refuses targets that cannot safely receive clipboard or synthesized input.
///
/// # Errors
///
/// Returns a typed security or writability refusal.
pub fn classify_guard(snapshot: &TargetSnapshot) -> Result<(), DeliveryRefusal> {
    match snapshot.is_secure_desktop {
        Some(false) => {}
        Some(true) => return Err(DeliveryRefusal::SecureDesktop),
        None => return Err(DeliveryRefusal::Unsupported),
    }
    if snapshot.is_password {
        return Err(DeliveryRefusal::Password);
    }
    if snapshot.is_read_only {
        return Err(DeliveryRefusal::ReadOnly);
    }
    if matches!(
        snapshot.integrity,
        speakeasy_domain::IntegrityRelationship::TargetHigher
            | speakeasy_domain::IntegrityRelationship::Unknown
    ) {
        return Err(DeliveryRefusal::ElevatedTarget);
    }
    if snapshot.target_kind == TargetKind::UnknownSensitive {
        return Err(DeliveryRefusal::UnknownSensitive);
    }
    Ok(())
}

/// Selects exactly one delivery strategy before any write occurs.
///
/// # Errors
///
/// Returns a guard refusal or [`DeliveryRefusal::Unsupported`] for live modes.
pub fn select_strategy(
    snapshot: &TargetSnapshot,
    settings: DeliverySettings,
) -> Result<DeliveryStrategy, DeliveryRefusal> {
    classify_guard(snapshot)?;
    if snapshot.target_kind == TargetKind::Terminal {
        return Ok(if settings.auto_copy {
            DeliveryStrategy::Clipboard
        } else {
            DeliveryStrategy::ResultView
        });
    }
    match snapshot.capability {
        DeliveryCapability::ResultViewOnly => Ok(DeliveryStrategy::ResultView),
        DeliveryCapability::ClipboardOnly => {
            if settings.auto_copy {
                Ok(DeliveryStrategy::Clipboard)
            } else {
                Ok(DeliveryStrategy::ResultView)
            }
        }
        DeliveryCapability::CommitOnFinish => {
            if settings.auto_paste {
                Ok(DeliveryStrategy::ClipboardPaste)
            } else if settings.auto_copy {
                Ok(DeliveryStrategy::Clipboard)
            } else {
                Ok(DeliveryStrategy::ResultView)
            }
        }
        DeliveryCapability::AppendOnlyLive | DeliveryCapability::VerifiedRangeReplace => {
            Err(DeliveryRefusal::Unsupported)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardTransaction {
    original_sequence: u32,
    write_sequence: Option<u32>,
    snapshot: Option<ClipboardSnapshotMetadata>,
    restore_enabled: bool,
}

impl ClipboardTransaction {
    pub const fn begin(
        original_sequence: u32,
        snapshot: Option<ClipboardSnapshotMetadata>,
        restore_enabled: bool,
    ) -> Self {
        Self {
            original_sequence,
            write_sequence: None,
            snapshot,
            restore_enabled,
        }
    }

    #[must_use]
    pub const fn record_write(mut self, sequence: u32) -> Self {
        self.write_sequence = Some(sequence);
        self
    }

    pub const fn should_restore(
        self,
        policy: ClipboardPolicy,
        current_sequence: u32,
        consumption_verified: bool,
    ) -> bool {
        if !self.restore_enabled || !consumption_verified {
            return false;
        }
        let Some(write_sequence) = self.write_sequence else {
            return false;
        };
        let Some(snapshot) = self.snapshot else {
            return false;
        };
        current_sequence == write_sequence
            && write_sequence != self.original_sequence
            && policy.accepts_snapshot(snapshot)
    }
}

#[derive(Debug)]
pub struct DeliveryPlan {
    pub session_id: SessionId,
    pub capability: DeliveryCapability,
    pub strategy: DeliveryStrategy,
}

#[derive(Debug)]
pub struct DeliveryJob {
    pub snapshot: TargetSnapshot,
    pub observation: TargetObservation,
    pub settings: DeliverySettings,
    pub response: SyncSender<Result<DeliveryPlan, DeliveryRefusal>>,
}

pub struct DeliveryActor {
    sender: Option<SyncSender<DeliveryJob>>,
    worker: Option<JoinHandle<()>>,
}

impl DeliveryActor {
    /// Starts the bounded policy actor.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::Unsupported`] for zero capacity or thread failure.
    pub fn spawn(capacity: usize) -> Result<Self, DeliveryRefusal> {
        if capacity == 0 {
            return Err(DeliveryRefusal::Unsupported);
        }
        let (sender, receiver) = sync_channel(capacity);
        let worker = thread::Builder::new()
            .name("speakeasy-delivery-policy".to_owned())
            .spawn(move || run_actor(&receiver))
            .map_err(|_| DeliveryRefusal::Unsupported)?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    /// Attempts bounded admission without waiting or growing the queue.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryRefusal::Unsupported`] when full or disconnected.
    pub fn try_submit(&self, job: DeliveryJob) -> Result<(), DeliveryRefusal> {
        let Some(sender) = self.sender.as_ref() else {
            return Err(DeliveryRefusal::Unsupported);
        };
        match sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                Err(DeliveryRefusal::Unsupported)
            }
        }
    }
}

impl Drop for DeliveryActor {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_actor(receiver: &Receiver<DeliveryJob>) {
    while let Ok(job) = receiver.recv() {
        let result = validate_target(&job.snapshot, job.observation)
            .and_then(|()| select_strategy(&job.snapshot, job.settings))
            .map(|strategy| DeliveryPlan {
                session_id: job.snapshot.session_id,
                capability: job.snapshot.capability,
                strategy,
            });
        let _ = job.response.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speakeasy_domain::{
        ExecutableIdentity, IntegrityRelationship, KeyboardContext, UiaPatterns,
    };

    fn session(byte: u8) -> SessionId {
        SessionId::from_bytes([byte; 16])
    }

    fn snapshot() -> TargetSnapshot {
        TargetSnapshot {
            session_id: session(1),
            window_handle: 100,
            process_id: 200,
            thread_id: 300,
            executable: ExecutableIdentity {
                path: "C:\\Windows\\notepad.exe".to_owned(),
                process_start_time: 400,
            },
            integrity: IntegrityRelationship::Equal,
            element: None,
            target_kind: TargetKind::Standard,
            is_password: false,
            is_read_only: false,
            is_secure_desktop: Some(false),
            patterns: UiaPatterns::default(),
            selection: None,
            content_fingerprint: None,
            input_epoch: Some(500),
            hook_epoch: Some(600),
            keyboard: KeyboardContext {
                layout: Some(1),
                ime_open: Some(false),
                ime_composing: Some(false),
            },
            capability: DeliveryCapability::CommitOnFinish,
        }
    }

    fn observation() -> TargetObservation {
        TargetObservation {
            session_id: session(1),
            window_handle: 100,
            process_id: 200,
            process_start_time: 400,
            thread_id: 300,
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
            hook_epoch: Some(600),
            modifiers_released: true,
        }
    }

    #[test]
    fn auto_copy_and_auto_paste_are_independent() {
        let target = snapshot();
        assert_eq!(
            select_strategy(
                &target,
                DeliverySettings {
                    auto_copy: true,
                    ..DeliverySettings::default()
                }
            ),
            Ok(DeliveryStrategy::Clipboard)
        );
        assert_eq!(
            select_strategy(
                &target,
                DeliverySettings {
                    auto_paste: true,
                    ..DeliverySettings::default()
                }
            ),
            Ok(DeliveryStrategy::ClipboardPaste)
        );
    }

    #[test]
    fn live_capabilities_are_not_available_in_phase_five() {
        for capability in [
            DeliveryCapability::AppendOnlyLive,
            DeliveryCapability::VerifiedRangeReplace,
        ] {
            let mut target = snapshot();
            target.capability = capability;
            assert_eq!(
                select_strategy(&target, DeliverySettings::default()),
                Err(DeliveryRefusal::Unsupported)
            );
        }
    }

    #[test]
    fn every_target_invalidation_freezes_before_strategy_selection() {
        type Mutation = fn(&mut TargetObservation);

        let target = snapshot();
        let cases: [(Mutation, DeliveryRefusal); 14] = [
            (
                |value: &mut TargetObservation| value.foreground = false,
                DeliveryRefusal::FocusChanged,
            ),
            (
                |value: &mut TargetObservation| value.process_id += 1,
                DeliveryRefusal::ProcessChanged,
            ),
            (
                |value: &mut TargetObservation| value.process_start_time += 1,
                DeliveryRefusal::WindowReused,
            ),
            (
                |value: &mut TargetObservation| value.element_matches = false,
                DeliveryRefusal::ElementChanged,
            ),
            (
                |value: &mut TargetObservation| value.selection_matches = false,
                DeliveryRefusal::SelectionChanged,
            ),
            (
                |value: &mut TargetObservation| value.caret_matches = false,
                DeliveryRefusal::CaretChanged,
            ),
            (
                |value: &mut TargetObservation| value.content_matches = false,
                DeliveryRefusal::ContentChanged,
            ),
            (
                |value: &mut TargetObservation| value.integrity_matches = false,
                DeliveryRefusal::IntegrityChanged,
            ),
            (
                |value: &mut TargetObservation| value.keyboard_matches = false,
                DeliveryRefusal::ContentChanged,
            ),
            (
                |value: &mut TargetObservation| value.user_input_seen = true,
                DeliveryRefusal::UserInput,
            ),
            (
                |value: &mut TargetObservation| value.hook_healthy = false,
                DeliveryRefusal::HookUnavailable,
            ),
            (
                |value: &mut TargetObservation| value.own_input_distinguished = false,
                DeliveryRefusal::HookUnavailable,
            ),
            (
                |value: &mut TargetObservation| value.hook_epoch = Some(601),
                DeliveryRefusal::UserInput,
            ),
            (
                |value: &mut TargetObservation| value.modifiers_released = false,
                DeliveryRefusal::ModifierHeld,
            ),
        ];
        for (mutate, refusal) in cases {
            let mut current = observation();
            mutate(&mut current);
            assert_eq!(validate_target(&target, current), Err(refusal));
        }
    }

    #[test]
    fn sensitive_targets_refuse_clipboard_and_input() {
        let mut target = snapshot();
        target.is_password = true;
        assert_eq!(classify_guard(&target), Err(DeliveryRefusal::Password));
        target.is_password = false;
        target.is_secure_desktop = Some(true);
        assert_eq!(classify_guard(&target), Err(DeliveryRefusal::SecureDesktop));
        target.is_secure_desktop = None;
        assert_eq!(classify_guard(&target), Err(DeliveryRefusal::Unsupported));
        target.is_secure_desktop = Some(false);
        target.integrity = IntegrityRelationship::TargetHigher;
        assert_eq!(
            classify_guard(&target),
            Err(DeliveryRefusal::ElevatedTarget)
        );
    }

    #[test]
    fn terminal_commit_never_selects_a_target_mutation_strategy() {
        let mut target = snapshot();
        target.target_kind = TargetKind::Terminal;
        assert_eq!(
            select_strategy(
                &target,
                DeliverySettings {
                    auto_paste: true,
                    ..DeliverySettings::default()
                }
            ),
            Ok(DeliveryStrategy::ResultView)
        );
        assert_eq!(
            select_strategy(
                &target,
                DeliverySettings {
                    auto_copy: true,
                    auto_paste: true,
                    ..DeliverySettings::default()
                }
            ),
            Ok(DeliveryStrategy::Clipboard)
        );
    }

    #[test]
    fn clipboard_restoration_requires_complete_snapshot_verified_consumption_and_unchanged_sequence()
     {
        let policy = ClipboardPolicy::conservative();
        let transaction = ClipboardTransaction::begin(
            10,
            Some(ClipboardSnapshotMetadata {
                bytes: 100,
                formats: 2,
                complete: true,
            }),
            true,
        )
        .record_write(11);
        assert!(transaction.should_restore(policy, 11, true));
        assert!(!transaction.should_restore(policy, 12, true));
        assert!(!transaction.should_restore(policy, 11, false));
    }

    #[test]
    fn clipboard_restoration_rejects_large_delayed_or_incomplete_snapshots() {
        let policy = ClipboardPolicy::conservative();
        for snapshot in [
            ClipboardSnapshotMetadata {
                bytes: policy.maximum_snapshot_bytes + 1,
                formats: 1,
                complete: true,
            },
            ClipboardSnapshotMetadata {
                bytes: 1,
                formats: policy.maximum_snapshot_formats + 1,
                complete: true,
            },
            ClipboardSnapshotMetadata {
                bytes: 1,
                formats: 1,
                complete: false,
            },
        ] {
            assert!(
                !ClipboardTransaction::begin(1, Some(snapshot), true)
                    .record_write(2)
                    .should_restore(policy, 2, true)
            );
        }
    }

    #[test]
    fn bounded_actor_returns_a_plan_without_claiming_a_write() {
        let actor = DeliveryActor::spawn(1).expect("actor starts");
        let (response, result) = sync_channel(1);
        actor
            .try_submit(DeliveryJob {
                snapshot: snapshot(),
                observation: observation(),
                settings: DeliverySettings {
                    auto_copy: true,
                    ..DeliverySettings::default()
                },
                response,
            })
            .expect("job admitted");
        let plan = result.recv().expect("response").expect("accepted");
        assert_eq!(plan.session_id, session(1));
        assert_eq!(plan.capability, DeliveryCapability::CommitOnFinish);
        assert_eq!(plan.strategy, DeliveryStrategy::Clipboard);
    }
}
