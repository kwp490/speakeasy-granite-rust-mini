//! Settings, migration, and repository adapter boundary for `SpeakEasy`.

mod importer;
mod personalization;
mod recovery;
mod repository;
mod settings;

#[cfg(any(test, feature = "test-import"))]
pub use importer::ExplicitImportRoot;
pub use importer::{
    CollisionPolicy, ImportChoices, ImportError, ImportPlan, ImportPreview, ImportReport,
    ImportWarning, ProductionImportPlan, ProductionImportRoot,
};
pub use personalization::{
    PersonalizationError, PersonalizationRepository, extract_v1_protected_terms,
};
pub use recovery::{
    BackupManifest, FileRecord, HealthCheckOutcome, PendingUpdateStatus, RecoveryError,
    RestoreOutcome, clear_pending_update_after_health_checks, create_recovery_bundle,
    mark_update_pending, mark_update_pending_at, pending_update_status, restore_recovery_bundle,
    verified_installer_path, verify_recovery_bundle,
};
pub use repository::{
    DATABASE_SCHEMA_VERSION, HistoryPolicy, HistoryRepository, RepositoryError, ResultProvenance,
    SessionResultList, TranscriptResult,
};
pub use settings::{
    APP_CAPABILITY_EVIDENCE_SCHEMA_VERSION, ActivationHotkeyMode, AppCapabilityEvidence,
    AppDeliveryCapability, CloudPolishConsent, CloudPolishPreferences, DEFAULT_ACTIVATION_HOTKEY,
    DeliveryPreferences, HotkeyPreferences, HudDockEdge, HudDockPlacement,
    LiveDeliveryChoice, LoadOutcome, OnboardingProgress, PrivacyPreferences,
    SafeDeliveryPreference, Settings, SettingsError, SettingsStore, ThemePreference,
    WritingRulePreferences,
};
