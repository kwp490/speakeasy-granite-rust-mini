use speakeasy_domain::{
    DeliveryCapability, DeliveryOutcome, DeliveryReceipt, DeliveryRefusal, DeliveryStrategy,
    TargetSnapshot,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{TargetObservation, classify_guard, validate_target};

pub const CONTROLLED_APPEND_ADAPTER_ID: &str = "speakeasy-controlled-probe-v1";
pub const LIVE_EVIDENCE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityRequest {
    Disabled,
    AppendOnlyLive,
    VerifiedRangeReplace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityEvidence {
    pub schema_version: u16,
    pub app_id: String,
    pub app_version: String,
    pub adapter_id: String,
    pub qualification_id: String,
    pub qualified_capability: DeliveryCapability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDecision {
    pub requested: CapabilityRequest,
    pub effective: DeliveryCapability,
    pub downgrade_reason: Option<&'static str>,
}

pub struct LiveDeliveryPolicy;

impl LiveDeliveryPolicy {
    #[must_use]
    pub fn evaluate(
        requested: CapabilityRequest,
        app_id: &str,
        app_version: &str,
        adapter_id: &str,
        evidence: Option<&CapabilityEvidence>,
    ) -> CapabilityDecision {
        let fallback = |reason| CapabilityDecision {
            requested,
            effective: DeliveryCapability::CommitOnFinish,
            downgrade_reason: Some(reason),
        };
        if requested == CapabilityRequest::Disabled {
            return fallback("live_delivery_not_selected");
        }
        let Some(evidence) = evidence else {
            return fallback("live_delivery_evidence_missing");
        };
        if evidence.schema_version != LIVE_EVIDENCE_SCHEMA_VERSION {
            return fallback("live_delivery_evidence_version_mismatch");
        }
        if evidence.app_id != app_id
            || evidence.app_version != app_version
            || evidence.adapter_id != adapter_id
        {
            return fallback("live_delivery_target_version_mismatch");
        }
        if evidence.qualification_id.trim().is_empty() {
            return fallback("live_delivery_qualification_missing");
        }
        let requested_capability = match requested {
            CapabilityRequest::Disabled => DeliveryCapability::CommitOnFinish,
            CapabilityRequest::AppendOnlyLive => DeliveryCapability::AppendOnlyLive,
            CapabilityRequest::VerifiedRangeReplace => DeliveryCapability::VerifiedRangeReplace,
        };
        if evidence.qualified_capability != requested_capability {
            return fallback("live_delivery_capability_not_qualified");
        }
        if requested == CapabilityRequest::AppendOnlyLive
            && adapter_id != CONTROLLED_APPEND_ADAPTER_ID
        {
            return fallback("append_only_target_not_controlled");
        }
        CapabilityDecision {
            requested,
            effective: requested_capability,
            downgrade_reason: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRange {
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub exact_text: String,
}

impl OwnedRange {
    fn append(&mut self, text: &str, utf16_length: u32) {
        self.end_utf16 = self.end_utf16.saturating_add(utf16_length);
        self.exact_text.push_str(text);
    }

    fn replace(&mut self, text: &str, utf16_length: u32) {
        self.end_utf16 = self.start_utf16.saturating_add(utf16_length);
        self.exact_text.clear();
        self.exact_text.push_str(text);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerBatch {
    pub sequence: u64,
    pub exact_rendered_text: String,
    pub target_window: u64,
    pub target_process: u32,
    pub range_before: OwnedRange,
    pub range_after: OwnedRange,
    pub strategy: DeliveryStrategy,
    pub consumption_verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsertionLedger {
    session_id: speakeasy_domain::SessionId,
    rendered_text: String,
    owned_range: OwnedRange,
    batches: Vec<LedgerBatch>,
    frozen: bool,
}

impl InsertionLedger {
    #[must_use]
    pub fn new(snapshot: &TargetSnapshot, start_utf16: u32) -> Self {
        Self {
            session_id: snapshot.session_id,
            rendered_text: String::new(),
            owned_range: OwnedRange {
                start_utf16,
                end_utf16: start_utf16,
                exact_text: String::new(),
            },
            batches: Vec::new(),
            frozen: false,
        }
    }

    #[must_use]
    pub fn rendered_text(&self) -> &str {
        &self.rendered_text
    }

    #[must_use]
    pub const fn owned_range(&self) -> &OwnedRange {
        &self.owned_range
    }

    #[must_use]
    pub fn batches(&self) -> &[LedgerBatch] {
        &self.batches
    }

    #[must_use]
    pub const fn is_frozen(&self) -> bool {
        self.frozen
    }

    fn freeze(&mut self) {
        self.frozen = true;
    }

    fn record_append(
        &mut self,
        snapshot: &TargetSnapshot,
        text: &str,
        receipt: DeliveryReceipt,
    ) -> Result<(), DeliveryRefusal> {
        self.record(snapshot, text, receipt, false)
    }

    fn record_replace(
        &mut self,
        snapshot: &TargetSnapshot,
        text: &str,
        receipt: DeliveryReceipt,
    ) -> Result<(), DeliveryRefusal> {
        self.record(snapshot, text, receipt, true)
    }

    fn record(
        &mut self,
        snapshot: &TargetSnapshot,
        text: &str,
        receipt: DeliveryReceipt,
        replace: bool,
    ) -> Result<(), DeliveryRefusal> {
        if self.frozen
            || text.is_empty()
            || snapshot.session_id != self.session_id
            || receipt.session_id != self.session_id
            || receipt.outcome != DeliveryOutcome::Committed
            || !receipt.consumption_verified
        {
            return Err(DeliveryRefusal::AmbiguousInput);
        }
        let utf16_length =
            u32::try_from(text.encode_utf16().count()).map_err(|_| DeliveryRefusal::Unsupported)?;
        let before = self.owned_range.clone();
        if replace {
            self.rendered_text.clear();
            self.rendered_text.push_str(text);
            self.owned_range.replace(text, utf16_length);
        } else {
            self.rendered_text.push_str(text);
            self.owned_range.append(text, utf16_length);
        }
        self.batches.push(LedgerBatch {
            sequence: self.batches.len() as u64 + 1,
            exact_rendered_text: text.to_owned(),
            target_window: snapshot.window_handle,
            target_process: snapshot.process_id,
            range_before: before,
            range_after: self.owned_range.clone(),
            strategy: receipt.strategy,
            consumption_verified: receipt.consumption_verified,
        });
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reconciliation {
    Exact,
    AppendSuffix(String),
    Diverged,
}

#[must_use]
pub fn reconcile_final(ledger_rendered: &str, final_text: &str) -> Reconciliation {
    if ledger_rendered == final_text {
        return Reconciliation::Exact;
    }
    if !final_text.starts_with(ledger_rendered) {
        return Reconciliation::Diverged;
    }
    let boundary = ledger_rendered.len();
    let is_grapheme_boundary = final_text
        .grapheme_indices(true)
        .any(|(index, _)| index == boundary);
    if is_grapheme_boundary {
        Reconciliation::AppendSuffix(final_text[boundary..].to_owned())
    } else {
        Reconciliation::Diverged
    }
}

pub trait LiveDeliveryAdapter {
    fn adapter_id(&self) -> &str;
    /// Reobserves all target and activity evidence.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when observation is incomplete or unavailable.
    fn observe(&mut self, snapshot: &TargetSnapshot) -> Result<TargetObservation, DeliveryRefusal>;
    /// Reads the exact app-owned range.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when the range cannot be read completely.
    fn read_owned_range(&mut self, range: &OwnedRange) -> Result<String, DeliveryRefusal>;
    /// Selects only the exact app-owned range.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal when exact selection cannot be proven.
    fn select_owned_range(&mut self, range: &OwnedRange) -> Result<(), DeliveryRefusal>;
    /// Uses the already selected safe writer without cascading.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal or ambiguous outcome; queued input is not success.
    fn write_preselected(
        &mut self,
        capability: DeliveryCapability,
        strategy: DeliveryStrategy,
        text: &str,
    ) -> Result<DeliveryReceipt, DeliveryRefusal>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveDeliveryOutcome {
    NoMutation,
    Mutated(DeliveryReceipt),
    Frozen {
        reason: DeliveryRefusal,
        complete_final: String,
    },
}

pub struct LiveDeliveryTransaction {
    snapshot: TargetSnapshot,
    capability: DeliveryCapability,
    strategy: DeliveryStrategy,
    ledger: InsertionLedger,
}

impl LiveDeliveryTransaction {
    /// Creates a transaction only after an exact, evidence-backed policy decision.
    ///
    /// # Errors
    ///
    /// Refuses downgraded, terminal, sensitive, or adapter-mismatched targets.
    pub fn begin(
        snapshot: TargetSnapshot,
        start_utf16: u32,
        strategy: DeliveryStrategy,
        decision: &CapabilityDecision,
        adapter: &dyn LiveDeliveryAdapter,
    ) -> Result<Self, DeliveryRefusal> {
        classify_guard(&snapshot)?;
        if decision.downgrade_reason.is_some()
            || !matches!(
                decision.effective,
                DeliveryCapability::AppendOnlyLive | DeliveryCapability::VerifiedRangeReplace
            )
            || snapshot.capability != decision.effective
        {
            return Err(DeliveryRefusal::Unsupported);
        }
        if decision.effective == DeliveryCapability::AppendOnlyLive
            && adapter.adapter_id() != CONTROLLED_APPEND_ADAPTER_ID
        {
            return Err(DeliveryRefusal::Unsupported);
        }
        Ok(Self {
            ledger: InsertionLedger::new(&snapshot, start_utf16),
            snapshot,
            capability: decision.effective,
            strategy,
        })
    }

    #[must_use]
    pub const fn ledger(&self) -> &InsertionLedger {
        &self.ledger
    }

    /// Appends one engine-guaranteed immutable segment after exact revalidation.
    ///
    /// The caller cannot pass a mutable hypothesis through a different entrypoint:
    /// this operation is deliberately named and typed as immutable-segment input.
    pub fn append_immutable_segment(
        &mut self,
        adapter: &mut dyn LiveDeliveryAdapter,
        immutable_segment: &str,
    ) -> LiveDeliveryOutcome {
        if self.ledger.is_frozen()
            || self.capability != DeliveryCapability::AppendOnlyLive
            || immutable_segment.is_empty()
        {
            return self.freeze(
                DeliveryRefusal::Unsupported,
                self.ledger.rendered_text().to_owned(),
            );
        }
        match self.verify_append_anchor(adapter).and_then(|()| {
            adapter.write_preselected(self.capability, self.strategy, immutable_segment)
        }) {
            Ok(receipt) => {
                if self
                    .ledger
                    .record_append(&self.snapshot, immutable_segment, receipt)
                    .is_ok()
                {
                    LiveDeliveryOutcome::Mutated(receipt)
                } else {
                    self.freeze(
                        DeliveryRefusal::AmbiguousInput,
                        self.ledger.rendered_text().to_owned(),
                    )
                }
            }
            Err(reason) => self.freeze(reason, self.ledger.rendered_text().to_owned()),
        }
    }

    /// Reconciles an authoritative final against the exact rendered ledger.
    pub fn finish_append_only(
        &mut self,
        adapter: &mut dyn LiveDeliveryAdapter,
        final_text: &str,
    ) -> LiveDeliveryOutcome {
        if self.ledger.is_frozen() {
            return LiveDeliveryOutcome::Frozen {
                reason: DeliveryRefusal::AmbiguousInput,
                complete_final: final_text.to_owned(),
            };
        }
        match reconcile_final(self.ledger.rendered_text(), final_text) {
            Reconciliation::Exact => LiveDeliveryOutcome::NoMutation,
            Reconciliation::AppendSuffix(suffix) => self.append_immutable_segment(adapter, &suffix),
            Reconciliation::Diverged => {
                self.freeze(DeliveryRefusal::ContentChanged, final_text.to_owned())
            }
        }
    }

    /// Replaces only the exact re-readable, re-selectable app-owned range.
    pub fn replace_verified_range(
        &mut self,
        adapter: &mut dyn LiveDeliveryAdapter,
        replacement: &str,
    ) -> LiveDeliveryOutcome {
        if self.ledger.is_frozen()
            || self.capability != DeliveryCapability::VerifiedRangeReplace
            || replacement.is_empty()
        {
            return self.freeze(DeliveryRefusal::Unsupported, replacement.to_owned());
        }
        let operation = self.verify_owned_range(adapter).and_then(|()| {
            adapter.select_owned_range(self.ledger.owned_range())?;
            self.verify_owned_range(adapter)?;
            adapter.write_preselected(self.capability, self.strategy, replacement)
        });
        match operation {
            Ok(receipt) => {
                if self
                    .ledger
                    .record_replace(&self.snapshot, replacement, receipt)
                    .is_ok()
                {
                    LiveDeliveryOutcome::Mutated(receipt)
                } else {
                    self.freeze(DeliveryRefusal::AmbiguousInput, replacement.to_owned())
                }
            }
            Err(reason) => self.freeze(reason, replacement.to_owned()),
        }
    }

    fn verify_append_anchor(
        &self,
        adapter: &mut dyn LiveDeliveryAdapter,
    ) -> Result<(), DeliveryRefusal> {
        self.verify_owned_range(adapter)?;
        adapter.select_owned_range(&OwnedRange {
            start_utf16: self.ledger.owned_range.end_utf16,
            end_utf16: self.ledger.owned_range.end_utf16,
            exact_text: String::new(),
        })?;
        self.verify_owned_range(adapter)
    }

    fn verify_owned_range(
        &self,
        adapter: &mut dyn LiveDeliveryAdapter,
    ) -> Result<(), DeliveryRefusal> {
        let observation = adapter.observe(&self.snapshot)?;
        validate_target(&self.snapshot, observation)?;
        let current = adapter.read_owned_range(self.ledger.owned_range())?;
        if current != self.ledger.owned_range.exact_text {
            return Err(DeliveryRefusal::ContentChanged);
        }
        Ok(())
    }

    fn freeze(&mut self, reason: DeliveryRefusal, complete_final: String) -> LiveDeliveryOutcome {
        self.ledger.freeze();
        LiveDeliveryOutcome::Frozen {
            reason,
            complete_final,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speakeasy_domain::{
        ExecutableIdentity, IntegrityRelationship, KeyboardContext, SessionId, TargetKind,
        UiaPatterns,
    };

    struct FakeAdapter {
        id: &'static str,
        observation: TargetObservation,
        range_text: String,
        reject_selection: bool,
        queued_only: bool,
        observe_error: Option<DeliveryRefusal>,
        writes: Vec<String>,
    }

    impl LiveDeliveryAdapter for FakeAdapter {
        fn adapter_id(&self) -> &str {
            self.id
        }

        fn observe(
            &mut self,
            _snapshot: &TargetSnapshot,
        ) -> Result<TargetObservation, DeliveryRefusal> {
            self.observe_error.map_or(Ok(self.observation), Err)
        }

        fn read_owned_range(&mut self, _range: &OwnedRange) -> Result<String, DeliveryRefusal> {
            Ok(self.range_text.clone())
        }

        fn select_owned_range(&mut self, _range: &OwnedRange) -> Result<(), DeliveryRefusal> {
            if self.reject_selection {
                Err(DeliveryRefusal::SelectionChanged)
            } else {
                Ok(())
            }
        }

        fn write_preselected(
            &mut self,
            capability: DeliveryCapability,
            strategy: DeliveryStrategy,
            text: &str,
        ) -> Result<DeliveryReceipt, DeliveryRefusal> {
            self.writes.push(text.to_owned());
            if capability == DeliveryCapability::AppendOnlyLive {
                self.range_text.push_str(text);
            } else {
                self.range_text.clear();
                self.range_text.push_str(text);
            }
            Ok(DeliveryReceipt {
                session_id: self.observation.session_id,
                capability,
                strategy,
                outcome: if self.queued_only {
                    DeliveryOutcome::InputQueued
                } else {
                    DeliveryOutcome::Committed
                },
                clipboard_sequence: None,
                input_events_accepted: Some(1),
                consumption_verified: !self.queued_only,
            })
        }
    }

    fn snapshot(capability: DeliveryCapability) -> TargetSnapshot {
        let session_id = SessionId::from_bytes([7; 16]);
        TargetSnapshot {
            session_id,
            window_handle: 10,
            process_id: 20,
            thread_id: 30,
            executable: ExecutableIdentity {
                path: "controlled-probe.exe".to_owned(),
                process_start_time: 40,
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
                value: false,
            },
            selection: None,
            content_fingerprint: Some([1; 32]),
            input_epoch: Some(50),
            hook_epoch: Some(60),
            keyboard: KeyboardContext {
                layout: Some(1),
                ime_open: Some(false),
                ime_composing: Some(false),
            },
            capability,
        }
    }

    fn observation() -> TargetObservation {
        TargetObservation {
            session_id: SessionId::from_bytes([7; 16]),
            window_handle: 10,
            process_id: 20,
            process_start_time: 40,
            thread_id: 30,
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
            hook_epoch: Some(60),
            modifiers_released: true,
        }
    }

    fn evidence(capability: DeliveryCapability, adapter_id: &str) -> CapabilityEvidence {
        CapabilityEvidence {
            schema_version: LIVE_EVIDENCE_SCHEMA_VERSION,
            app_id: "controlled-probe".to_owned(),
            app_version: "1.0.0".to_owned(),
            adapter_id: adapter_id.to_owned(),
            qualification_id: "deterministic-only".to_owned(),
            qualified_capability: capability,
        }
    }

    fn decision(capability: DeliveryCapability, adapter_id: &str) -> CapabilityDecision {
        LiveDeliveryPolicy::evaluate(
            match capability {
                DeliveryCapability::AppendOnlyLive => CapabilityRequest::AppendOnlyLive,
                DeliveryCapability::VerifiedRangeReplace => CapabilityRequest::VerifiedRangeReplace,
                _ => CapabilityRequest::Disabled,
            },
            "controlled-probe",
            "1.0.0",
            adapter_id,
            Some(&evidence(capability, adapter_id)),
        )
    }

    fn adapter(id: &'static str) -> FakeAdapter {
        FakeAdapter {
            id,
            observation: observation(),
            range_text: String::new(),
            reject_selection: false,
            queued_only: false,
            observe_error: None,
            writes: Vec::new(),
        }
    }

    #[test]
    fn policy_is_default_off_exact_versioned_and_never_promotes() {
        let disabled =
            LiveDeliveryPolicy::evaluate(CapabilityRequest::Disabled, "a", "1", "x", None);
        assert_eq!(disabled.effective, DeliveryCapability::CommitOnFinish);
        assert_eq!(
            disabled.downgrade_reason,
            Some("live_delivery_not_selected")
        );

        let mismatch = LiveDeliveryPolicy::evaluate(
            CapabilityRequest::VerifiedRangeReplace,
            "controlled-probe",
            "2.0.0",
            "range-v1",
            Some(&evidence(
                DeliveryCapability::VerifiedRangeReplace,
                "range-v1",
            )),
        );
        assert_eq!(
            mismatch.downgrade_reason,
            Some("live_delivery_target_version_mismatch")
        );
        assert_eq!(mismatch.effective, DeliveryCapability::CommitOnFinish);
    }

    #[test]
    fn append_only_is_restricted_to_the_controlled_adapter() {
        let rejected = LiveDeliveryPolicy::evaluate(
            CapabilityRequest::AppendOnlyLive,
            "controlled-probe",
            "1.0.0",
            "arbitrary-app",
            Some(&evidence(
                DeliveryCapability::AppendOnlyLive,
                "arbitrary-app",
            )),
        );
        assert_eq!(
            rejected.downgrade_reason,
            Some("append_only_target_not_controlled")
        );
    }

    #[test]
    fn exact_grapheme_prefix_reconciliation_covers_unicode_and_crlf() {
        let cases = [
            ("hello ", "hello world"),
            ("👩‍💻", "👩‍💻 works"),
            ("e\u{301}", "e\u{301}lan"),
            ("漢字", "漢字です"),
            ("שלום", "שלום עולם"),
            ("a\r\n", "a\r\nb"),
        ];
        for (prefix, final_text) in cases {
            assert_eq!(
                reconcile_final(prefix, final_text),
                Reconciliation::AppendSuffix(final_text[prefix.len()..].to_owned())
            );
        }
        assert_eq!(reconcile_final("e", "e\u{301}"), Reconciliation::Diverged);
        assert_eq!(reconcile_final("é", "e\u{301}"), Reconciliation::Diverged);
    }

    #[test]
    fn append_final_suffix_never_backspaces_or_duplicates() {
        let mut writer = adapter(CONTROLLED_APPEND_ADAPTER_ID);
        let target = snapshot(DeliveryCapability::AppendOnlyLive);
        let mut transaction = LiveDeliveryTransaction::begin(
            target,
            0,
            DeliveryStrategy::UnicodeInput,
            &decision(
                DeliveryCapability::AppendOnlyLive,
                CONTROLLED_APPEND_ADAPTER_ID,
            ),
            &writer,
        )
        .expect("qualified transaction");
        assert!(matches!(
            transaction.append_immutable_segment(&mut writer, "hello "),
            LiveDeliveryOutcome::Mutated(_)
        ));
        assert!(matches!(
            transaction.finish_append_only(&mut writer, "hello world"),
            LiveDeliveryOutcome::Mutated(_)
        ));
        assert_eq!(writer.writes, ["hello ", "world"]);
        assert_eq!(transaction.ledger().rendered_text(), "hello world");
    }

    #[test]
    fn divergence_freezes_and_exposes_the_complete_final_without_correction() {
        let mut writer = adapter(CONTROLLED_APPEND_ADAPTER_ID);
        let target = snapshot(DeliveryCapability::AppendOnlyLive);
        let mut transaction = LiveDeliveryTransaction::begin(
            target,
            0,
            DeliveryStrategy::UnicodeInput,
            &decision(
                DeliveryCapability::AppendOnlyLive,
                CONTROLLED_APPEND_ADAPTER_ID,
            ),
            &writer,
        )
        .expect("qualified transaction");
        let _ = transaction.append_immutable_segment(&mut writer, "draft");
        assert_eq!(
            transaction.finish_append_only(&mut writer, "different final"),
            LiveDeliveryOutcome::Frozen {
                reason: DeliveryRefusal::ContentChanged,
                complete_final: "different final".to_owned(),
            }
        );
        assert_eq!(writer.writes, ["draft"]);
        assert!(transaction.ledger().is_frozen());
        assert!(matches!(
            transaction.finish_append_only(&mut writer, "late polished final"),
            LiveDeliveryOutcome::Frozen {
                complete_final,
                ..
            } if complete_final == "late polished final"
        ));
        assert_eq!(writer.writes, ["draft"]);
    }

    #[test]
    fn verified_replace_rereads_and_reselects_before_each_mutation() {
        let mut writer = adapter("range-v1");
        let target = snapshot(DeliveryCapability::VerifiedRangeReplace);
        let mut transaction = LiveDeliveryTransaction::begin(
            target,
            4,
            DeliveryStrategy::UnicodeInput,
            &decision(DeliveryCapability::VerifiedRangeReplace, "range-v1"),
            &writer,
        )
        .expect("qualified transaction");
        assert!(matches!(
            transaction.replace_verified_range(&mut writer, "👩‍💻\r\n漢字"),
            LiveDeliveryOutcome::Mutated(_)
        ));
        assert_eq!(transaction.ledger().owned_range().start_utf16, 4);
        assert_eq!(
            transaction.ledger().owned_range().end_utf16,
            4 + u32::try_from("👩‍💻\r\n漢字".encode_utf16().count()).expect("short fixture")
        );
        writer.range_text.push('!');
        assert_eq!(
            transaction.replace_verified_range(&mut writer, "safe final"),
            LiveDeliveryOutcome::Frozen {
                reason: DeliveryRefusal::ContentChanged,
                complete_final: "safe final".to_owned(),
            }
        );
        assert_eq!(writer.writes.len(), 1);
    }

    #[test]
    fn every_ambiguity_freezes_before_an_additional_write() {
        type Mutation = fn(&mut FakeAdapter);
        let cases: [Mutation; 10] = [
            |value| value.observation.foreground = false,
            |value| value.observation.caret_matches = false,
            |value| value.observation.selection_matches = false,
            |value| value.observation.content_matches = false,
            |value| value.observation.integrity_matches = false,
            |value| value.observation.keyboard_matches = false,
            |value| value.observation.user_input_seen = true,
            |value| value.observation.hook_healthy = false,
            |value| value.observation.own_input_distinguished = false,
            |value| value.reject_selection = true,
        ];
        for mutate in cases {
            let mut writer = adapter("range-v1");
            mutate(&mut writer);
            let target = snapshot(DeliveryCapability::VerifiedRangeReplace);
            let mut transaction = LiveDeliveryTransaction::begin(
                target,
                0,
                DeliveryStrategy::UnicodeInput,
                &decision(DeliveryCapability::VerifiedRangeReplace, "range-v1"),
                &writer,
            )
            .expect("qualified transaction");
            assert!(matches!(
                transaction.replace_verified_range(&mut writer, "final"),
                LiveDeliveryOutcome::Frozen { .. }
            ));
            assert!(writer.writes.is_empty());
        }
    }

    #[test]
    fn queued_input_is_never_ledger_success() {
        let mut writer = adapter(CONTROLLED_APPEND_ADAPTER_ID);
        writer.queued_only = true;
        let target = snapshot(DeliveryCapability::AppendOnlyLive);
        let mut transaction = LiveDeliveryTransaction::begin(
            target,
            0,
            DeliveryStrategy::UnicodeInput,
            &decision(
                DeliveryCapability::AppendOnlyLive,
                CONTROLLED_APPEND_ADAPTER_ID,
            ),
            &writer,
        )
        .expect("qualified transaction");
        assert_eq!(
            transaction.append_immutable_segment(&mut writer, "text"),
            LiveDeliveryOutcome::Frozen {
                reason: DeliveryRefusal::AmbiguousInput,
                complete_final: String::new(),
            }
        );
        assert!(transaction.ledger().rendered_text().is_empty());
    }

    #[test]
    fn complete_invalidation_matrix_is_fail_closed_before_mutation() {
        type Mutation = fn(&mut FakeAdapter);
        let cases: [(&str, Mutation); 15] = [
            ("mid_session_typing", |value| {
                value.observation.user_input_seen = true;
            }),
            ("caret_change", |value| {
                value.observation.caret_matches = false;
            }),
            ("selection_change", |value| {
                value.observation.selection_matches = false;
            }),
            ("focus_change", |value| {
                value.observation.foreground = false;
            }),
            ("app_close", |value| {
                value.observe_error = Some(DeliveryRefusal::AppClosed);
            }),
            ("hwnd_reuse", |value| {
                value.observation.process_start_time += 1;
            }),
            ("undo_redo_content", |value| {
                value.observation.content_matches = false;
            }),
            ("ime_composition", |value| {
                value.observation.keyboard_matches = false;
            }),
            ("lagging_stuck_uia", |value| {
                value.observe_error = Some(DeliveryRefusal::DeadlineExceeded);
            }),
            ("replacement_rejection", |value| {
                value.reject_selection = true;
            }),
            ("integrity_mismatch", |value| {
                value.observation.integrity_matches = false;
            }),
            ("hook_health_ambiguity", |value| {
                value.observation.hook_healthy = false;
            }),
            ("own_input_ambiguity", |value| {
                value.observation.own_input_distinguished = false;
            }),
            ("ambiguous_delivery", |value| {
                value.queued_only = true;
            }),
            ("identity_change", |value| {
                value.observation.element_matches = false;
            }),
        ];
        for (name, mutate) in cases {
            let mut writer = adapter("range-v1");
            mutate(&mut writer);
            let target = snapshot(DeliveryCapability::VerifiedRangeReplace);
            let mut transaction = LiveDeliveryTransaction::begin(
                target,
                0,
                DeliveryStrategy::UnicodeInput,
                &decision(DeliveryCapability::VerifiedRangeReplace, "range-v1"),
                &writer,
            )
            .expect("qualified deterministic transaction");
            let outcome = transaction.replace_verified_range(&mut writer, "final");
            assert!(
                matches!(outcome, LiveDeliveryOutcome::Frozen { .. }),
                "{name} must freeze"
            );
            if name != "ambiguous_delivery" {
                assert!(writer.writes.is_empty(), "{name} wrote before refusal");
            }
            assert!(
                transaction.ledger().rendered_text().is_empty(),
                "{name} entered unverified text in the ledger"
            );
        }
    }

    #[test]
    fn app_version_change_and_unqualified_modes_never_begin() {
        let writer = adapter("range-v1");
        let target = snapshot(DeliveryCapability::VerifiedRangeReplace);
        let changed_version = LiveDeliveryPolicy::evaluate(
            CapabilityRequest::VerifiedRangeReplace,
            "controlled-probe",
            "2.0.0",
            "range-v1",
            Some(&evidence(
                DeliveryCapability::VerifiedRangeReplace,
                "range-v1",
            )),
        );
        assert_eq!(
            LiveDeliveryTransaction::begin(
                target,
                0,
                DeliveryStrategy::UnicodeInput,
                &changed_version,
                &writer,
            )
            .err(),
            Some(DeliveryRefusal::Unsupported)
        );
    }
}
