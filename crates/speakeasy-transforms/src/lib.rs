//! Pure, versioned, deterministic text personalization.
//!
//! This crate has no filesystem, UI, model-runtime, delivery, or execution
//! authority. All outputs are inert Unicode strings.

mod cleanup;
mod dictionary;
mod hotwords;
mod import;
mod pipeline;
mod snippets;

pub use cleanup::{RuleCleanupConfig, RuleCleanupMode, RuleCleanupResult, apply_rule_cleanup};
pub use dictionary::{
    BoundaryPolicy, CasePolicy, DictionaryEntry, DictionaryOrigin, DictionarySet,
    DictionaryValidationError, ReplacementProvenance,
};
pub use hotwords::{
    DecoderIdentity, HotwordDecision, HotwordMeasurement, HotwordMeasurementStatus,
    decide_hotword_path,
};
pub use import::{
    ConflictKind, ImportConflict, ImportError, ImportPlan, ImportPolicy, ImportPreview,
    PERSONALIZATION_SCHEMA_VERSION, PersonalizationBundle,
};
pub use pipeline::{
    FormatFeature, FormatFeatureStatus, LocaleQualification, PipelineMode, PipelineRequest,
    PipelineResult, StageKind, StageRecord, TRANSFORM_PIPELINE_VERSION, TransformPipeline,
};
pub use snippets::{Snippet, SnippetError, SnippetResolution, SnippetSet, TriggerDisposition};
