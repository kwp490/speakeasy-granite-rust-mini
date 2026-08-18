use std::collections::{BTreeMap, VecDeque};

use crate::{DeliveryCapability, EngineState, SessionId};

/// Every boundary at which a test can deterministically inject a bounded fault.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FaultBoundary {
    CoordinatorQueue,
    AudioQueue,
    StreamingQueue,
    ResultQueue,
    DeliveryQueue,
    CoordinatorActor,
    AudioActor,
    EngineActor,
    DeliveryActor,
    FilesystemRead,
    FilesystemWrite,
    FilesystemRename,
    FilesystemSync,
    ModelVerify,
    ModelInstall,
    RuntimeLoad,
    RuntimeProvider,
    WorkerSpawn,
    WorkerProtocol,
    WorkerCrash,
    WorkerHang,
    WorkerOutOfMemory,
    TargetInspect,
    UiaObserve,
    InputWrite,
    ClipboardOpen,
    ClipboardWrite,
    ClipboardSequence,
    CredentialRead,
    CredentialWrite,
    NetworkConnect,
    NetworkRead,
    OptionalStorageOpen,
    OptionalStorageMigrate,
    OptionalStorageWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InjectedFault {
    Busy,
    Closed,
    Timeout,
    Denied,
    Missing,
    Disconnected,
    Overflow,
    Corrupt,
    Tampered,
    DiskFull,
    OutOfMemory,
    Crash,
    ProtocolMismatch,
    Stale,
    Changed,
    TooNew,
}

/// A deterministic fault script. Each entry is consumed once and total storage
/// is bounded at construction.
#[derive(Debug)]
pub struct FaultScript {
    maximum_entries: usize,
    entries: BTreeMap<FaultBoundary, VecDeque<InjectedFault>>,
    remaining: usize,
}

impl FaultScript {
    pub const MAXIMUM_ENTRIES: usize = 256;

    pub fn new(maximum_entries: usize) -> Option<Self> {
        (1..=Self::MAXIMUM_ENTRIES)
            .contains(&maximum_entries)
            .then(|| Self {
                maximum_entries,
                entries: BTreeMap::new(),
                remaining: 0,
            })
    }

    /// Adds one fault to the bounded deterministic script.
    ///
    /// # Errors
    ///
    /// Returns [`FaultScriptError::CapacityExceeded`] when the declared
    /// capacity is already occupied.
    pub fn schedule(
        &mut self,
        boundary: FaultBoundary,
        fault: InjectedFault,
    ) -> Result<(), FaultScriptError> {
        if self.remaining == self.maximum_entries {
            return Err(FaultScriptError::CapacityExceeded);
        }
        self.entries.entry(boundary).or_default().push_back(fault);
        self.remaining += 1;
        Ok(())
    }

    pub fn take(&mut self, boundary: FaultBoundary) -> Option<InjectedFault> {
        let fault = self.entries.get_mut(&boundary)?.pop_front();
        if fault.is_some() {
            self.remaining -= 1;
        }
        if self.entries.get(&boundary).is_some_and(VecDeque::is_empty) {
            self.entries.remove(&boundary);
        }
        fault
    }

    pub const fn remaining(&self) -> usize {
        self.remaining
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultScriptError {
    CapacityExceeded,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DegradationReason {
    MicrophoneDenied,
    MicrophoneMissing,
    MicrophoneDisconnected,
    AudioOverflow,
    StreamingUnavailable,
    FinalEngineTimeout,
    FinalEngineCrashed,
    WorkerOutOfMemory,
    WorkerQuarantined,
    ProviderLost,
    ModelMissing,
    ModelCorrupt,
    ModelTampered,
    ModelUpdateInterrupted,
    TargetChanged,
    UserInputChanged,
    SensitiveTarget,
    ElevatedTarget,
    ClipboardBusy,
    ClipboardChanged,
    RemoteUnavailable,
    OptionalStorageUnavailable,
    SleepInterrupted,
    ShutdownInterrupted,
    SettingsTooNew,
    DatabaseTooNew,
    ProtocolTooNew,
    DiskFull,
    OperationConflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DegradationAction {
    RetryMicrophone,
    ChooseMicrophone,
    FinishManually,
    ContinueFinalOnly,
    UseRecoverableDraft,
    RetryOnCpu,
    VerifyOrReinstallModel,
    OpenResultView,
    CopyExplicitly,
    ContinueLocalRaw,
    ContinueWithoutHistory,
    ResumeAndRetry,
    RestartAfterDictation,
    ManualWorkerRecovery,
    FreeDiskSpace,
    UpdateOptionalSurface,
    WaitForDictation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DegradationDecision {
    pub reason: DegradationReason,
    pub reason_code: &'static str,
    pub action: DegradationAction,
    pub action_code: &'static str,
    pub engine: Option<EngineState>,
    pub delivery: Option<DeliveryCapability>,
    pub preserve_audio: bool,
    pub preserve_transcript: bool,
    pub automatic_retry: bool,
}

#[allow(clippy::too_many_lines)]
pub const fn degradation_decision(reason: DegradationReason) -> DegradationDecision {
    use DegradationAction as Action;
    use DegradationReason as Reason;
    let (reason_code, action, action_code, engine, delivery, preserve_audio, preserve_transcript) =
        match reason {
            Reason::MicrophoneDenied => (
                "degradation.microphone_denied",
                Action::RetryMicrophone,
                "action.review_microphone_permission",
                None,
                None,
                true,
                true,
            ),
            Reason::MicrophoneMissing | Reason::MicrophoneDisconnected => (
                "degradation.microphone_unavailable",
                Action::ChooseMicrophone,
                "action.choose_microphone",
                None,
                None,
                true,
                true,
            ),
            Reason::AudioOverflow => (
                "degradation.audio_overflow",
                Action::FinishManually,
                "action.finish_and_review",
                None,
                None,
                true,
                true,
            ),
            Reason::StreamingUnavailable => (
                "degradation.streaming_unavailable",
                Action::ContinueFinalOnly,
                "action.continue_final_only",
                Some(EngineState::Ready),
                None,
                true,
                true,
            ),
            Reason::FinalEngineTimeout | Reason::FinalEngineCrashed | Reason::WorkerOutOfMemory => {
                (
                    "degradation.final_engine_failed",
                    Action::UseRecoverableDraft,
                    "action.review_recoverable_result",
                    Some(EngineState::Failed),
                    Some(DeliveryCapability::ResultViewOnly),
                    true,
                    true,
                )
            }
            Reason::WorkerQuarantined => (
                "degradation.worker_quarantined",
                Action::ManualWorkerRecovery,
                "action.recover_worker",
                Some(EngineState::Failed),
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::ProviderLost => (
                "degradation.accelerator_lost",
                Action::RetryOnCpu,
                "action.retry_on_cpu",
                Some(EngineState::Loading),
                None,
                true,
                true,
            ),
            Reason::ModelMissing
            | Reason::ModelCorrupt
            | Reason::ModelTampered
            | Reason::ModelUpdateInterrupted => (
                "degradation.model_unavailable",
                Action::VerifyOrReinstallModel,
                "action.verify_model",
                Some(EngineState::Unavailable),
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::TargetChanged | Reason::UserInputChanged => (
                "degradation.target_changed",
                Action::OpenResultView,
                "action.open_result_view",
                None,
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::SensitiveTarget | Reason::ElevatedTarget => (
                "degradation.target_refused",
                Action::OpenResultView,
                "action.open_private_result",
                None,
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::ClipboardBusy | Reason::ClipboardChanged => (
                "degradation.clipboard_unavailable",
                Action::CopyExplicitly,
                "action.retry_copy",
                None,
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::RemoteUnavailable => (
                "degradation.optional_network_unavailable",
                Action::ContinueLocalRaw,
                "action.use_local_result",
                None,
                None,
                true,
                true,
            ),
            Reason::OptionalStorageUnavailable
            | Reason::DatabaseTooNew
            | Reason::SettingsTooNew => (
                "degradation.optional_storage_unavailable",
                Action::ContinueWithoutHistory,
                "action.continue_without_history",
                None,
                None,
                true,
                true,
            ),
            Reason::SleepInterrupted => (
                "degradation.lifecycle_interrupted",
                Action::ResumeAndRetry,
                "action.retry_after_resume",
                None,
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::ShutdownInterrupted => (
                "degradation.shutdown_in_progress",
                Action::RestartAfterDictation,
                "action.restart_later",
                None,
                Some(DeliveryCapability::ResultViewOnly),
                true,
                true,
            ),
            Reason::ProtocolTooNew => (
                "degradation.protocol_too_new",
                Action::UpdateOptionalSurface,
                "action.update_optional_component",
                None,
                None,
                true,
                true,
            ),
            Reason::DiskFull => (
                "degradation.disk_full",
                Action::FreeDiskSpace,
                "action.free_disk_space",
                None,
                None,
                true,
                true,
            ),
            Reason::OperationConflict => (
                "degradation.dictation_busy",
                Action::WaitForDictation,
                "action.retry_after_dictation",
                None,
                None,
                true,
                true,
            ),
        };
    DegradationDecision {
        reason,
        reason_code,
        action,
        action_code,
        engine,
        delivery,
        preserve_audio,
        preserve_transcript,
        automatic_retry: matches!(reason, Reason::ProviderLost),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExclusiveOperation {
    Dictation(SessionId),
    ModelInstall,
    ModelUpdate,
    ModelDelete,
    ApplicationUpdate,
    StorageMigration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationDisposition {
    Started,
    AlreadyOwned,
    Deferred {
        owner: ExclusiveOperation,
        retry_after_ms: u32,
    },
}

/// Serializes operations that could invalidate audio, models, runtime bytes, or
/// storage while a dictation session owns recoverable state.
#[derive(Debug, Default)]
pub struct OperationArbiter {
    owner: Option<ExclusiveOperation>,
}

impl OperationArbiter {
    pub fn begin(&mut self, operation: ExclusiveOperation) -> OperationDisposition {
        match self.owner {
            None => {
                self.owner = Some(operation);
                OperationDisposition::Started
            }
            Some(owner) if owner == operation => OperationDisposition::AlreadyOwned,
            Some(owner) => OperationDisposition::Deferred {
                owner,
                retry_after_ms: 1_000,
            },
        }
    }

    pub fn finish(&mut self, operation: ExclusiveOperation) -> bool {
        if self.owner == Some(operation) {
            self.owner = None;
            true
        } else {
            false
        }
    }

    pub const fn owner(&self) -> Option<ExclusiveOperation> {
        self.owner
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEvent {
    Suspend,
    Resume,
    Lock,
    Unlock,
    RdpDisconnected,
    RdpConnected,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct LifecycleEffects {
    pub accept_activations: bool,
    pub cancel_capture: bool,
    pub invalidate_target: bool,
    pub reregister_hotkey: bool,
    pub retry_selected_microphone_only: bool,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct LifecycleController {
    accept_activations: bool,
    suspended: bool,
    locked: bool,
    rdp_connected: bool,
    shutting_down: bool,
}

impl Default for LifecycleController {
    fn default() -> Self {
        Self {
            accept_activations: true,
            suspended: false,
            locked: false,
            rdp_connected: true,
            shutting_down: false,
        }
    }
}

impl LifecycleController {
    pub fn apply(&mut self, event: LifecycleEvent) -> LifecycleEffects {
        use LifecycleEvent as Event;
        match event {
            Event::Suspend => self.suspended = true,
            Event::Resume => self.suspended = false,
            Event::Lock => self.locked = true,
            Event::Unlock => self.locked = false,
            Event::RdpDisconnected => self.rdp_connected = false,
            Event::RdpConnected => self.rdp_connected = true,
            Event::Shutdown => self.shutting_down = true,
        }
        self.accept_activations =
            !self.suspended && !self.locked && self.rdp_connected && !self.shutting_down;
        LifecycleEffects {
            accept_activations: self.accept_activations,
            cancel_capture: matches!(
                event,
                Event::Suspend | Event::Lock | Event::RdpDisconnected | Event::Shutdown
            ),
            invalidate_target: !matches!(
                event,
                Event::Resume | Event::Unlock | Event::RdpConnected
            ),
            reregister_hotkey: matches!(event, Event::Resume | Event::Unlock | Event::RdpConnected)
                && self.accept_activations,
            retry_selected_microphone_only: matches!(
                event,
                Event::Resume | Event::Unlock | Event::RdpConnected
            ) && self.accept_activations,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownStep {
    StopActivations,
    CancelCapture,
    DrainAudio,
    CancelEngines,
    SettleDelivery,
    CheckpointStorage,
    TerminateWorkerTrees,
    CloseUi,
}

pub const SHUTDOWN_ORDER: [ShutdownStep; 8] = [
    ShutdownStep::StopActivations,
    ShutdownStep::CancelCapture,
    ShutdownStep::DrainAudio,
    ShutdownStep::CancelEngines,
    ShutdownStep::SettleDelivery,
    ShutdownStep::CheckpointStorage,
    ShutdownStep::TerminateWorkerTrees,
    ShutdownStep::CloseUi,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CorrelationId, DOMAIN_SCHEMA_VERSION, IngressEvent, ProducerId, Reducer,
        ReducerDisposition, SessionPhase,
    };

    fn session(byte: u8) -> SessionId {
        SessionId::from_bytes([byte; 16])
    }

    #[test]
    fn every_fault_boundary_is_bounded_and_one_shot() {
        let boundaries = [
            FaultBoundary::CoordinatorQueue,
            FaultBoundary::AudioQueue,
            FaultBoundary::StreamingQueue,
            FaultBoundary::ResultQueue,
            FaultBoundary::DeliveryQueue,
            FaultBoundary::CoordinatorActor,
            FaultBoundary::AudioActor,
            FaultBoundary::EngineActor,
            FaultBoundary::DeliveryActor,
            FaultBoundary::FilesystemRead,
            FaultBoundary::FilesystemWrite,
            FaultBoundary::FilesystemRename,
            FaultBoundary::FilesystemSync,
            FaultBoundary::ModelVerify,
            FaultBoundary::ModelInstall,
            FaultBoundary::RuntimeLoad,
            FaultBoundary::RuntimeProvider,
            FaultBoundary::WorkerSpawn,
            FaultBoundary::WorkerProtocol,
            FaultBoundary::WorkerCrash,
            FaultBoundary::WorkerHang,
            FaultBoundary::WorkerOutOfMemory,
            FaultBoundary::TargetInspect,
            FaultBoundary::UiaObserve,
            FaultBoundary::InputWrite,
            FaultBoundary::ClipboardOpen,
            FaultBoundary::ClipboardWrite,
            FaultBoundary::ClipboardSequence,
            FaultBoundary::CredentialRead,
            FaultBoundary::CredentialWrite,
            FaultBoundary::NetworkConnect,
            FaultBoundary::NetworkRead,
            FaultBoundary::OptionalStorageOpen,
            FaultBoundary::OptionalStorageMigrate,
            FaultBoundary::OptionalStorageWrite,
        ];
        let mut script = FaultScript::new(boundaries.len()).expect("bounded script");
        for boundary in boundaries {
            script.schedule(boundary, InjectedFault::Timeout).unwrap();
        }
        assert_eq!(script.remaining(), boundaries.len());
        assert_eq!(
            script.schedule(FaultBoundary::AudioQueue, InjectedFault::Overflow),
            Err(FaultScriptError::CapacityExceeded)
        );
        for boundary in boundaries {
            assert_eq!(script.take(boundary), Some(InjectedFault::Timeout));
            assert_eq!(script.take(boundary), None);
        }
        assert_eq!(script.remaining(), 0);
    }

    #[test]
    fn failure_matrix_preserves_recoverable_results_and_safe_delivery() {
        use DegradationReason as Reason;
        let reasons = [
            Reason::MicrophoneDenied,
            Reason::MicrophoneMissing,
            Reason::MicrophoneDisconnected,
            Reason::AudioOverflow,
            Reason::StreamingUnavailable,
            Reason::FinalEngineTimeout,
            Reason::FinalEngineCrashed,
            Reason::WorkerOutOfMemory,
            Reason::WorkerQuarantined,
            Reason::ProviderLost,
            Reason::ModelMissing,
            Reason::ModelCorrupt,
            Reason::ModelTampered,
            Reason::ModelUpdateInterrupted,
            Reason::TargetChanged,
            Reason::UserInputChanged,
            Reason::SensitiveTarget,
            Reason::ElevatedTarget,
            Reason::ClipboardBusy,
            Reason::ClipboardChanged,
            Reason::RemoteUnavailable,
            Reason::OptionalStorageUnavailable,
            Reason::SleepInterrupted,
            Reason::ShutdownInterrupted,
            Reason::SettingsTooNew,
            Reason::DatabaseTooNew,
            Reason::ProtocolTooNew,
            Reason::DiskFull,
            Reason::OperationConflict,
        ];
        for reason in reasons {
            let decision = degradation_decision(reason);
            assert!(decision.preserve_transcript, "{reason:?}");
            assert!(decision.preserve_audio, "{reason:?}");
            assert!(decision.reason_code.starts_with("degradation."));
            assert!(decision.action_code.starts_with("action."));
        }
        for reason in [
            Reason::TargetChanged,
            Reason::UserInputChanged,
            Reason::SensitiveTarget,
            Reason::ElevatedTarget,
            Reason::ClipboardBusy,
            Reason::ClipboardChanged,
        ] {
            assert_eq!(
                degradation_decision(reason).delivery,
                Some(DeliveryCapability::ResultViewOnly)
            );
        }
    }

    #[test]
    fn dictation_defers_all_invalidating_operations_until_exact_owner_finishes() {
        let active = ExclusiveOperation::Dictation(session(1));
        let mut arbiter = OperationArbiter::default();
        assert_eq!(arbiter.begin(active), OperationDisposition::Started);
        for operation in [
            ExclusiveOperation::ModelInstall,
            ExclusiveOperation::ModelUpdate,
            ExclusiveOperation::ModelDelete,
            ExclusiveOperation::ApplicationUpdate,
            ExclusiveOperation::StorageMigration,
        ] {
            assert!(matches!(
                arbiter.begin(operation),
                OperationDisposition::Deferred { owner, .. } if owner == active
            ));
        }
        assert!(!arbiter.finish(ExclusiveOperation::Dictation(session(2))));
        assert_eq!(arbiter.owner(), Some(active));
        assert!(arbiter.finish(active));
        assert_eq!(
            arbiter.begin(ExclusiveOperation::ApplicationUpdate),
            OperationDisposition::Started
        );
    }

    #[test]
    fn power_lock_and_rdp_cycles_recover_hotkey_without_stale_activation() {
        let mut lifecycle = LifecycleController::default();
        for (away, back) in [
            (LifecycleEvent::Suspend, LifecycleEvent::Resume),
            (LifecycleEvent::Lock, LifecycleEvent::Unlock),
            (
                LifecycleEvent::RdpDisconnected,
                LifecycleEvent::RdpConnected,
            ),
        ] {
            let effects = lifecycle.apply(away);
            assert!(!effects.accept_activations);
            assert!(effects.cancel_capture);
            assert!(effects.invalidate_target);
            let effects = lifecycle.apply(back);
            assert!(effects.accept_activations);
            assert!(effects.reregister_hotkey);
            assert!(effects.retry_selected_microphone_only);
        }
        let effects = lifecycle.apply(LifecycleEvent::Shutdown);
        assert!(!effects.accept_activations);
        assert!(effects.cancel_capture);
        assert!(!lifecycle.apply(LifecycleEvent::Resume).accept_activations);
    }

    #[test]
    fn shutdown_order_is_literal_and_stable() {
        assert_eq!(
            SHUTDOWN_ORDER,
            [
                ShutdownStep::StopActivations,
                ShutdownStep::CancelCapture,
                ShutdownStep::DrainAudio,
                ShutdownStep::CancelEngines,
                ShutdownStep::SettleDelivery,
                ShutdownStep::CheckpointStorage,
                ShutdownStep::TerminateWorkerTrees,
                ShutdownStep::CloseUi,
            ]
        );
    }

    #[test]
    fn two_hundred_complete_sessions_keep_identity_and_resource_state_bounded() {
        let mut reducer = Reducer::default();
        let producer_id = ProducerId::from_bytes([9; 16]);
        let mut sequence = 0_u64;
        for index in 0..200_u8 {
            let session_id = SessionId::from_bytes([index.wrapping_add(1); 16]);
            assert_eq!(
                reducer.begin_session(session_id),
                ReducerDisposition::Applied
            );
            for phase in [
                SessionPhase::Capturing,
                SessionPhase::Draining,
                SessionPhase::Finalizing,
                SessionPhase::Delivering,
                SessionPhase::Delivered,
            ] {
                sequence += 1;
                let ingress = IngressEvent {
                    schema_version: DOMAIN_SCHEMA_VERSION,
                    correlation_id: CorrelationId::from_bytes(session_id.into_bytes()),
                    session_id: Some(session_id),
                    producer_id,
                    source_sequence: sequence,
                    producer_monotonic_ns: sequence,
                    payload: (),
                };
                assert_eq!(
                    reducer.apply(ingress, phase, sequence).0,
                    ReducerDisposition::Applied
                );
            }
            sequence += 1;
            let reset = IngressEvent {
                schema_version: DOMAIN_SCHEMA_VERSION,
                correlation_id: CorrelationId::from_bytes(session_id.into_bytes()),
                session_id: None,
                producer_id,
                source_sequence: sequence,
                producer_monotonic_ns: sequence,
                payload: (),
            };
            assert_eq!(
                reducer.apply(reset, SessionPhase::Idle, sequence).0,
                ReducerDisposition::Applied
            );
            assert_eq!(reducer.active_session(), None);
            assert_eq!(reducer.tracked_producers(), 1);
        }
        assert_eq!(reducer.state().session, SessionPhase::Idle);
        assert_eq!(reducer.tracked_producers(), 1);
    }
}
