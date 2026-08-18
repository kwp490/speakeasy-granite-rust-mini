use crate::{ActivationMode, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationInput {
    TogglePressed { session_id: SessionId },
    PushToTalkPressed { session_id: SessionId },
    PushToTalkReleased { session_id: SessionId },
    HandsFreePressed { session_id: SessionId },
    VadEndpoint { session_id: SessionId },
    ManualStop { session_id: SessionId },
    Cancel { session_id: SessionId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationStopReason {
    TogglePressed,
    PushToTalkReleased,
    VadEndpoint,
    Manual,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationEffect {
    Start {
        session_id: SessionId,
        mode: ActivationMode,
    },
    Stop {
        session_id: SessionId,
        reason: ActivationStopReason,
    },
    Ignored,
    StaleSession,
    SessionAlreadyActive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveActivation {
    session_id: SessionId,
    mode: ActivationMode,
}

#[derive(Debug, Default)]
pub struct ActivationReducer {
    active: Option<ActiveActivation>,
    push_to_talk_held: bool,
}

impl ActivationReducer {
    pub const fn active_session(&self) -> Option<SessionId> {
        match self.active {
            Some(active) => Some(active.session_id),
            None => None,
        }
    }

    pub const fn active_mode(&self) -> Option<ActivationMode> {
        match self.active {
            Some(active) => Some(active.mode),
            None => None,
        }
    }

    pub fn apply(&mut self, input: ActivationInput) -> ActivationEffect {
        match input {
            ActivationInput::TogglePressed { session_id } => self.start_or_toggle_stop(session_id),
            ActivationInput::PushToTalkPressed { session_id } => self.ptt_down(session_id),
            ActivationInput::PushToTalkReleased { session_id } => self.ptt_up(session_id),
            ActivationInput::HandsFreePressed { session_id } => {
                self.start(session_id, ActivationMode::HandsFree)
            }
            ActivationInput::VadEndpoint { session_id } => self.vad_endpoint(session_id),
            ActivationInput::ManualStop { session_id } => {
                self.stop(session_id, ActivationStopReason::Manual)
            }
            ActivationInput::Cancel { session_id } => {
                self.stop(session_id, ActivationStopReason::Cancelled)
            }
        }
    }

    fn start_or_toggle_stop(&mut self, session_id: SessionId) -> ActivationEffect {
        if self.active.is_some_and(|active| {
            active.session_id == session_id && active.mode == ActivationMode::Toggle
        }) {
            return self.stop(session_id, ActivationStopReason::TogglePressed);
        }
        self.start(session_id, ActivationMode::Toggle)
    }

    fn start(&mut self, session_id: SessionId, mode: ActivationMode) -> ActivationEffect {
        if self.active.is_some() {
            return ActivationEffect::SessionAlreadyActive;
        }
        self.active = Some(ActiveActivation { session_id, mode });
        self.push_to_talk_held = mode == ActivationMode::PushToTalk;
        ActivationEffect::Start { session_id, mode }
    }

    fn ptt_down(&mut self, session_id: SessionId) -> ActivationEffect {
        if self.active.is_some_and(|active| {
            active.session_id == session_id && active.mode == ActivationMode::PushToTalk
        }) && self.push_to_talk_held
        {
            return ActivationEffect::Ignored;
        }
        self.start(session_id, ActivationMode::PushToTalk)
    }

    fn ptt_up(&mut self, session_id: SessionId) -> ActivationEffect {
        let Some(active) = self.active else {
            return ActivationEffect::Ignored;
        };
        if active.session_id != session_id {
            return ActivationEffect::StaleSession;
        }
        if active.mode != ActivationMode::PushToTalk || !self.push_to_talk_held {
            return ActivationEffect::Ignored;
        }
        self.stop(session_id, ActivationStopReason::PushToTalkReleased)
    }

    fn vad_endpoint(&mut self, session_id: SessionId) -> ActivationEffect {
        let Some(active) = self.active else {
            return ActivationEffect::Ignored;
        };
        if active.session_id != session_id {
            return ActivationEffect::StaleSession;
        }
        if active.mode != ActivationMode::HandsFree {
            return ActivationEffect::Ignored;
        }
        self.stop(session_id, ActivationStopReason::VadEndpoint)
    }

    fn stop(&mut self, session_id: SessionId, reason: ActivationStopReason) -> ActivationEffect {
        let Some(active) = self.active else {
            return ActivationEffect::Ignored;
        };
        if active.session_id != session_id {
            return ActivationEffect::StaleSession;
        }
        self.active = None;
        self.push_to_talk_held = false;
        ActivationEffect::Stop { session_id, reason }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(byte: u8) -> SessionId {
        SessionId::from_bytes([byte; 16])
    }

    #[test]
    fn toggle_starts_and_second_press_stops_the_same_session() {
        let session_id = session(1);
        let mut reducer = ActivationReducer::default();
        assert_eq!(
            reducer.apply(ActivationInput::TogglePressed { session_id }),
            ActivationEffect::Start {
                session_id,
                mode: ActivationMode::Toggle
            }
        );
        assert_eq!(
            reducer.apply(ActivationInput::TogglePressed { session_id }),
            ActivationEffect::Stop {
                session_id,
                reason: ActivationStopReason::TogglePressed
            }
        );
    }

    #[test]
    fn push_to_talk_ignores_repeat_and_stops_on_matching_key_up() {
        let session_id = session(2);
        let mut reducer = ActivationReducer::default();
        assert!(matches!(
            reducer.apply(ActivationInput::PushToTalkPressed { session_id }),
            ActivationEffect::Start {
                mode: ActivationMode::PushToTalk,
                ..
            }
        ));
        assert_eq!(
            reducer.apply(ActivationInput::PushToTalkPressed { session_id }),
            ActivationEffect::Ignored
        );
        assert_eq!(
            reducer.apply(ActivationInput::VadEndpoint { session_id }),
            ActivationEffect::Ignored
        );
        assert_eq!(
            reducer.apply(ActivationInput::PushToTalkReleased { session_id }),
            ActivationEffect::Stop {
                session_id,
                reason: ActivationStopReason::PushToTalkReleased
            }
        );
    }

    #[test]
    fn hands_free_accepts_vad_endpoint_but_manual_stop_is_always_available() {
        let hands_free_session = session(3);
        let toggle_session = session(4);
        let mut reducer = ActivationReducer::default();
        assert!(matches!(
            reducer.apply(ActivationInput::HandsFreePressed {
                session_id: hands_free_session
            }),
            ActivationEffect::Start {
                mode: ActivationMode::HandsFree,
                ..
            }
        ));
        assert_eq!(
            reducer.apply(ActivationInput::VadEndpoint {
                session_id: hands_free_session
            }),
            ActivationEffect::Stop {
                session_id: hands_free_session,
                reason: ActivationStopReason::VadEndpoint
            }
        );
        assert!(matches!(
            reducer.apply(ActivationInput::TogglePressed {
                session_id: toggle_session
            }),
            ActivationEffect::Start { .. }
        ));
        assert_eq!(
            reducer.apply(ActivationInput::ManualStop {
                session_id: toggle_session
            }),
            ActivationEffect::Stop {
                session_id: toggle_session,
                reason: ActivationStopReason::Manual
            }
        );
    }

    #[test]
    fn mismatched_session_cannot_stop_active_capture() {
        let active = session(5);
        let stale = session(6);
        let mut reducer = ActivationReducer::default();
        reducer.apply(ActivationInput::PushToTalkPressed { session_id: active });
        assert_eq!(
            reducer.apply(ActivationInput::PushToTalkReleased { session_id: stale }),
            ActivationEffect::StaleSession
        );
        assert_eq!(reducer.active_session(), Some(active));
    }
}
