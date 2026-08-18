use std::collections::BTreeMap;

use crate::{AppState, CorrelationId, ProducerId, SessionId, SessionPhase, transition_allowed};

pub const DOMAIN_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngressEvent<T> {
    pub schema_version: u16,
    pub correlation_id: CorrelationId,
    pub session_id: Option<SessionId>,
    pub producer_id: ProducerId,
    pub source_sequence: u64,
    pub producer_monotonic_ns: u64,
    pub payload: T,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEvent<T> {
    pub schema_version: u16,
    pub correlation_id: CorrelationId,
    pub session_id: Option<SessionId>,
    pub coordinator_sequence: u64,
    pub received_monotonic_ns: u64,
    pub audio_sample_index: Option<u64>,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReducerDisposition {
    Applied,
    DuplicateOrOutOfOrder,
    StaleSession,
    InvalidTransition,
    ProducerCapacityExceeded,
}

const MAXIMUM_PRODUCERS: usize = 64;

#[derive(Debug, Default)]
pub struct Reducer {
    state: AppState,
    active_session: Option<SessionId>,
    last_source_sequence: BTreeMap<ProducerId, u64>,
    coordinator_sequence: u64,
}

impl Reducer {
    pub const fn state(&self) -> AppState {
        self.state
    }

    pub const fn active_session(&self) -> Option<SessionId> {
        self.active_session
    }

    pub fn begin_session(&mut self, session_id: SessionId) -> ReducerDisposition {
        if self.active_session.is_some()
            || !transition_allowed(self.state.session, SessionPhase::Arming)
        {
            return ReducerDisposition::InvalidTransition;
        }
        self.active_session = Some(session_id);
        self.state.session = SessionPhase::Arming;
        ReducerDisposition::Applied
    }

    pub fn apply<T>(
        &mut self,
        ingress: IngressEvent<T>,
        next_phase: SessionPhase,
        received_monotonic_ns: u64,
    ) -> (ReducerDisposition, Option<DomainEvent<T>>) {
        if ingress.session_id.is_some() && ingress.session_id != self.active_session {
            return (ReducerDisposition::StaleSession, None);
        }

        if self
            .last_source_sequence
            .get(&ingress.producer_id)
            .is_some_and(|last| ingress.source_sequence <= *last)
        {
            return (ReducerDisposition::DuplicateOrOutOfOrder, None);
        }

        if !self.last_source_sequence.contains_key(&ingress.producer_id)
            && self.last_source_sequence.len() == MAXIMUM_PRODUCERS
        {
            return (ReducerDisposition::ProducerCapacityExceeded, None);
        }

        if !transition_allowed(self.state.session, next_phase) {
            return (ReducerDisposition::InvalidTransition, None);
        }

        self.last_source_sequence
            .insert(ingress.producer_id, ingress.source_sequence);
        self.coordinator_sequence = self.coordinator_sequence.saturating_add(1);
        self.state.session = next_phase;

        if matches!(
            next_phase,
            SessionPhase::Delivered | SessionPhase::Cancelled | SessionPhase::Failed
        ) {
            self.active_session = None;
        }

        let event = DomainEvent {
            schema_version: DOMAIN_SCHEMA_VERSION,
            correlation_id: ingress.correlation_id,
            session_id: ingress.session_id,
            coordinator_sequence: self.coordinator_sequence,
            received_monotonic_ns,
            audio_sample_index: None,
            payload: ingress.payload,
        };
        (ReducerDisposition::Applied, Some(event))
    }

    pub fn tracked_producers(&self) -> usize {
        self.last_source_sequence.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> [u8; 16] {
        [byte; 16]
    }

    fn ingress(session_id: SessionId, sequence: u64) -> IngressEvent<&'static str> {
        IngressEvent {
            schema_version: DOMAIN_SCHEMA_VERSION,
            correlation_id: CorrelationId::from_bytes(id(1)),
            session_id: Some(session_id),
            producer_id: ProducerId::from_bytes(id(2)),
            source_sequence: sequence,
            producer_monotonic_ns: 10,
            payload: "redacted",
        }
    }

    #[test]
    fn reducer_rejects_stale_duplicate_and_skipped_events() {
        let active = SessionId::from_bytes(id(3));
        let stale = SessionId::from_bytes(id(4));
        let mut reducer = Reducer::default();
        assert_eq!(reducer.begin_session(active), ReducerDisposition::Applied);

        assert_eq!(
            reducer
                .apply(ingress(stale, 1), SessionPhase::Capturing, 20)
                .0,
            ReducerDisposition::StaleSession
        );
        assert_eq!(
            reducer
                .apply(ingress(active, 1), SessionPhase::Finalizing, 20)
                .0,
            ReducerDisposition::InvalidTransition
        );

        let (disposition, event) = reducer.apply(ingress(active, 1), SessionPhase::Capturing, 20);
        assert_eq!(disposition, ReducerDisposition::Applied);
        assert_eq!(event.expect("event").coordinator_sequence, 1);
        assert_eq!(
            reducer
                .apply(ingress(active, 1), SessionPhase::Draining, 30)
                .0,
            ReducerDisposition::DuplicateOrOutOfOrder
        );
    }

    #[test]
    fn completed_session_releases_single_session_guard() {
        let session = SessionId::from_bytes(id(3));
        let mut reducer = Reducer::default();
        assert_eq!(reducer.begin_session(session), ReducerDisposition::Applied);

        for (sequence, phase) in [
            (1, SessionPhase::Capturing),
            (2, SessionPhase::Draining),
            (3, SessionPhase::Finalizing),
            (4, SessionPhase::Delivering),
            (5, SessionPhase::Delivered),
        ] {
            assert_eq!(
                reducer
                    .apply(ingress(session, sequence), phase, sequence * 10)
                    .0,
                ReducerDisposition::Applied
            );
        }

        assert_eq!(reducer.active_session(), None);
    }
}
