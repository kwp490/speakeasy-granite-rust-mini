#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppReadiness {
    Starting,
    NeedsModel,
    Ready,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    Idle,
    Arming,
    Capturing,
    Draining,
    Finalizing,
    Delivering,
    Delivered,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VadState {
    Silence,
    Speech,
    Hangover,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineState {
    Unavailable,
    Loading,
    Ready,
    Running,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryCapability {
    VerifiedRangeReplace,
    AppendOnlyLive,
    CommitOnFinish,
    ClipboardOnly,
    ResultViewOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppState {
    pub readiness: AppReadiness,
    pub session: SessionPhase,
    pub vad: VadState,
    pub engine: EngineState,
    pub delivery: DeliveryCapability,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            readiness: AppReadiness::Starting,
            session: SessionPhase::Idle,
            vad: VadState::Silence,
            engine: EngineState::Unavailable,
            delivery: DeliveryCapability::ResultViewOnly,
        }
    }
}

pub const fn transition_allowed(from: SessionPhase, to: SessionPhase) -> bool {
    use SessionPhase::{
        Arming, Cancelled, Capturing, Delivered, Delivering, Draining, Failed, Finalizing, Idle,
    };

    matches!(
        (from, to),
        (Idle, Arming)
            | (Arming, Capturing | Cancelled | Failed)
            | (Capturing, Draining | Cancelled | Failed)
            | (Draining, Finalizing | Cancelled | Failed)
            | (Finalizing, Delivering | Cancelled | Failed)
            | (Delivering, Delivered | Failed)
            | (Delivered | Cancelled | Failed, Idle)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table_rejects_skips_and_reentry() {
        assert!(transition_allowed(SessionPhase::Idle, SessionPhase::Arming));
        assert!(!transition_allowed(
            SessionPhase::Idle,
            SessionPhase::Capturing
        ));
        assert!(!transition_allowed(
            SessionPhase::Capturing,
            SessionPhase::Arming
        ));
        assert!(transition_allowed(SessionPhase::Failed, SessionPhase::Idle));
    }
}
