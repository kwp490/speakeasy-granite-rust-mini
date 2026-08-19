use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const APP_CAPABILITY_EVIDENCE_SCHEMA_VERSION: u16 = 1;
/// The shipped global shortcut.
///
/// Not `Ctrl+Alt+L`, which is what `SpeakEasy` uses. The two apps can be
/// installed side by side -- different identifier, different `%APPDATA%`,
/// different single-instance lock -- and a global shortcut is the one resource
/// they would still contend for. Whichever registered second would simply not
/// get the key, and `hotkey_status` would report a conflict the user had no
/// obvious reason to expect.
pub const DEFAULT_ACTIVATION_HOTKEY: &str = "Ctrl+Alt+P";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct OnboardingProgress {
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub current_step: u8,
    #[serde(default)]
    pub privacy_reviewed: bool,
    #[serde(default)]
    pub microphone_checked: bool,
    #[serde(default)]
    pub hotkey_checked: bool,
    #[serde(default)]
    pub model_choice_reviewed: bool,
    #[serde(default)]
    pub try_it_completed: bool,
    #[serde(default)]
    pub delivery_choice_reviewed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrivacyPreferences {
    #[serde(default)]
    pub persisted_history_enabled: bool,
    #[serde(default = "default_retention_days")]
    pub history_retention_days: u16,
    #[serde(default)]
    pub history_plaintext_disclosure_accepted: bool,
    #[serde(default)]
    pub disk_logging_enabled: bool,
    #[serde(default)]
    pub cloud_polish: CloudPolishPreferences,
}

const fn default_retention_days() -> u16 {
    30
}

impl Default for PrivacyPreferences {
    fn default() -> Self {
        Self {
            persisted_history_enabled: false,
            history_retention_days: default_retention_days(),
            history_plaintext_disclosure_accepted: false,
            disk_logging_enabled: true,
            cloud_polish: CloudPolishPreferences::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudPolishPreferences {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub active_profile_id: Option<String>,
    #[serde(default = "default_polish_before_commit")]
    pub before_commit: bool,
    #[serde(default)]
    pub consent: Option<CloudPolishConsent>,
    #[serde(default)]
    pub per_app_profiles: BTreeMap<String, String>,
}

impl Default for CloudPolishPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_id: None,
            model_id: None,
            active_profile_id: None,
            before_commit: true,
            consent: None,
            per_app_profiles: BTreeMap::new(),
        }
    }
}

const fn default_polish_before_commit() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudPolishConsent {
    pub provider_id: String,
    pub endpoint: String,
    pub privacy_policy_version: String,
    pub prompt_behavior_version: u16,
    pub credential_generation: u64,
    pub granted_unix_ms: u64,
}

impl CloudPolishPreferences {
    #[must_use]
    pub fn consent_is_current(
        &self,
        provider_id: &str,
        endpoint: &str,
        privacy_policy_version: &str,
        prompt_behavior_version: u16,
        credential_generation: u64,
    ) -> bool {
        self.enabled
            && self.consent.as_ref().is_some_and(|receipt| {
                receipt.provider_id == provider_id
                    && receipt.endpoint == endpoint
                    && receipt.privacy_policy_version == privacy_policy_version
                    && receipt.prompt_behavior_version == prompt_behavior_version
                    && receipt.credential_generation == credential_generation
            })
    }

    pub fn reset_consent(&mut self) {
        self.consent = None;
        self.enabled = false;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveDeliveryChoice {
    #[default]
    Disabled,
    AppendOnlyLive,
    VerifiedRangeReplace,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeDeliveryPreference {
    #[default]
    ResultViewOnly,
    ExplicitCopy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppCapabilityEvidence {
    pub schema_version: u16,
    pub app_version: String,
    pub adapter_id: String,
    pub qualification_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppDeliveryCapability {
    #[serde(default)]
    pub user_choice: LiveDeliveryChoice,
    #[serde(default)]
    pub evidence: Option<AppCapabilityEvidence>,
    #[serde(default)]
    pub downgrade_reason: Option<String>,
}

impl Default for AppDeliveryCapability {
    fn default() -> Self {
        Self {
            user_choice: LiveDeliveryChoice::Disabled,
            evidence: None,
            downgrade_reason: Some("live_delivery_not_selected".to_owned()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct DeliveryPreferences {
    #[serde(default)]
    pub safe_preference: SafeDeliveryPreference,
    #[serde(default)]
    pub auto_copy: bool,
    #[serde(default)]
    pub auto_paste: bool,
    #[serde(default)]
    pub restore_clipboard: bool,
    #[serde(default = "default_feedback_enabled")]
    pub feedback_enabled: bool,
    #[serde(default)]
    pub app_capabilities: BTreeMap<String, AppDeliveryCapability>,
}

const fn default_feedback_enabled() -> bool {
    true
}

impl Default for DeliveryPreferences {
    fn default() -> Self {
        Self {
            safe_preference: SafeDeliveryPreference::ResultViewOnly,
            auto_copy: false,
            auto_paste: true,
            restore_clipboard: false,
            feedback_enabled: true,
            app_capabilities: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationHotkeyMode {
    #[default]
    Toggle,
    PushToTalk,
    HandsFree,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HotkeyPreferences {
    #[serde(default = "default_hotkey_enabled")]
    pub enabled: bool,
    #[serde(default = "default_activation_binding")]
    pub activation_binding: String,
    #[serde(default)]
    pub activation_mode: ActivationHotkeyMode,
}

const fn default_hotkey_enabled() -> bool {
    true
}

fn default_activation_binding() -> String {
    DEFAULT_ACTIVATION_HOTKEY.to_owned()
}

impl Default for HotkeyPreferences {
    fn default() -> Self {
        Self {
            enabled: default_hotkey_enabled(),
            activation_binding: default_activation_binding(),
            activation_mode: ActivationHotkeyMode::Toggle,
        }
    }
}

impl DeliveryPreferences {
    /// Removes an app-specific choice and all associated evidence.
    pub fn reset_app_capability(&mut self, app_id: &str) -> bool {
        self.app_capabilities.remove(app_id).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u16,
    pub locale: String,
    pub queue_capacity: usize,
    #[serde(default)]
    pub delivery: DeliveryPreferences,
    #[serde(default)]
    pub hotkey: HotkeyPreferences,
    #[serde(default)]
    pub onboarding: OnboardingProgress,
    #[serde(default)]
    pub privacy: PrivacyPreferences,
    #[serde(default)]
    pub writing_rules: WritingRulePreferences,
    #[serde(default)]
    pub startup_with_windows: bool,
    #[serde(default)]
    pub theme: ThemePreference,
    /// The capture device the user last explicitly selected, so hotkey-triggered
    /// dictation (which has no UI context to ask) can use the same device instead
    /// of guessing at the OS-reported default.
    #[serde(default)]
    pub preferred_capture_device_id: Option<String>,
    /// Where the user last put the dock. Presentation only.
    ///
    /// There were three of these: a placement for the large transcriber, a
    /// mode saying which of the two HUDs was showing, and this. The large HUD
    /// and the mode are gone, so a profile written by that app carries two
    /// fields this one does not read -- which is fine and deliberate. They
    /// land in `extensions` through the flattened catch-all rather than
    /// failing the parse, so an older profile still opens.
    #[serde(default)]
    pub hud_dock: HudDockPlacement,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

/// Which screen edge the side dock is flush against.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudDockEdge {
    Left,
    #[default]
    Right,
}

/// Persisted placement for the side dock.
///
/// Position-only: the dock's width is fixed, so only the
/// edge it is flush against and its vertical position are worth remembering.
/// Every field is optional/defaulted so a profile that has never seen the
/// dock stays valid.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HudDockPlacement {
    #[serde(default)]
    pub edge: HudDockEdge,
    #[serde(default)]
    pub position_y: Option<i32>,
    /// Identifies the monitor the position was recorded against: discard rather than
    /// restore off-screen when the recorded monitor is gone.
    #[serde(default)]
    pub monitor_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            locale: "en-US".to_owned(),
            queue_capacity: 8,
            delivery: DeliveryPreferences::default(),
            hotkey: HotkeyPreferences::default(),
            onboarding: OnboardingProgress::default(),
            privacy: PrivacyPreferences::default(),
            writing_rules: WritingRulePreferences::default(),
            startup_with_windows: false,
            theme: ThemePreference::System,
            preferred_capture_device_id: None,
            hud_dock: HudDockPlacement::default(),
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct WritingRulePreferences {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub filler_words: bool,
    #[serde(default)]
    pub immediate_repetitions: bool,
    #[serde(default)]
    pub self_corrections: bool,
    #[serde(default)]
    pub spoken_lists: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadOutcome {
    Loaded,
    RecoveredFromBackup,
    DefaultedMissing,
}

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    Corrupt,
    TooNew(u64),
    Invalid,
}

impl From<io::Error> for SettingsError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
    backup_path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let backup_path = path.with_extension("json.bak");
        Self { path, backup_path }
    }

    /// Loads validated settings without replacing corrupt or unsupported input.
    ///
    /// # Errors
    ///
    /// Returns an I/O, corruption, validation, or too-new-schema error when neither
    /// the primary file nor an eligible backup can be loaded.
    pub fn load(&self) -> Result<(Settings, LoadOutcome), SettingsError> {
        if !self.path.exists() {
            return Ok((Settings::default(), LoadOutcome::DefaultedMissing));
        }

        match read_settings(&self.path) {
            Ok(settings) => Ok((settings, LoadOutcome::Loaded)),
            Err(primary_error @ (SettingsError::Corrupt | SettingsError::Invalid)) => {
                if self.backup_path.exists() {
                    read_settings(&self.backup_path)
                        .map(|settings| (settings, LoadOutcome::RecoveredFromBackup))
                } else {
                    Err(primary_error)
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Writes settings through a flushed same-directory temporary and backup file.
    ///
    /// # Errors
    ///
    /// Returns an I/O or validation error and leaves the previous backup available.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        validate(settings)?;
        let parent = self.path.parent().ok_or(SettingsError::Invalid)?;
        fs::create_dir_all(parent)?;
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(settings).map_err(|_| SettingsError::Invalid)?;
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;

        if self.path.exists() {
            fs::copy(&self.path, &self.backup_path)?;
            OpenOptions::new()
                .write(true)
                .open(&self.backup_path)?
                .sync_all()?;
            fs::remove_file(&self.path)?;
        }
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

fn read_settings(path: &Path) -> Result<Settings, SettingsError> {
    let bytes = fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|_| SettingsError::Corrupt)?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or(SettingsError::Invalid)?;
    if version > u64::from(SETTINGS_SCHEMA_VERSION) {
        return Err(SettingsError::TooNew(version));
    }
    let settings = serde_json::from_value(value).map_err(|_| SettingsError::Invalid)?;
    validate(&settings)?;
    Ok(settings)
}

fn validate(settings: &Settings) -> Result<(), SettingsError> {
    if settings.schema_version != SETTINGS_SCHEMA_VERSION
        || settings.locale.trim().is_empty()
        || settings.queue_capacity == 0
        || settings.queue_capacity > 1_024
        || !(1..=365).contains(&settings.privacy.history_retention_days)
        || settings.onboarding.current_step > 7
    {
        return Err(SettingsError::Invalid);
    }
    for (app_id, capability) in &settings.delivery.app_capabilities {
        if app_id.trim().is_empty() || app_id.len() > 256 {
            return Err(SettingsError::Invalid);
        }
        if let Some(evidence) = &capability.evidence
            && (evidence.schema_version != APP_CAPABILITY_EVIDENCE_SCHEMA_VERSION
                || evidence.app_version.trim().is_empty()
                || evidence.adapter_id.trim().is_empty()
                || evidence.qualification_id.trim().is_empty())
        {
            return Err(SettingsError::Invalid);
        }
    }
    let polish = &settings.privacy.cloud_polish;
    if polish.enabled
        && (polish.provider_id.as_deref().is_none_or(str::is_empty)
            || polish.model_id.as_deref().is_none_or(str::is_empty)
            || polish
                .active_profile_id
                .as_deref()
                .is_none_or(str::is_empty)
            || polish.consent.is_none())
    {
        return Err(SettingsError::Invalid);
    }
    if polish
        .provider_id
        .as_ref()
        .is_some_and(|value| value.len() > 128)
        || polish
            .model_id
            .as_ref()
            .is_some_and(|value| value.len() > 128)
        || polish
            .active_profile_id
            .as_ref()
            .is_some_and(|value| value.len() > 128)
        || polish.per_app_profiles.iter().any(|(executable, profile)| {
            executable.is_empty()
                || executable.len() > 32_768
                || profile.is_empty()
                || profile.len() > 128
        })
    {
        return Err(SettingsError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "speakeasy-settings-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn save_preserves_unknown_keys_and_recovers_corrupt_primary() {
        let root = temp_path("recovery");
        let store = SettingsStore::new(root.join("settings.json"));
        let mut first = Settings::default();
        first.extensions.insert("future".into(), Value::Bool(true));
        store.save(&first).expect("first save");
        store.save(&Settings::default()).expect("second save");
        fs::write(&store.path, b"{").expect("corrupt primary");

        let (recovered, outcome) = store.load().expect("backup recovery");
        assert_eq!(outcome, LoadOutcome::RecoveredFromBackup);
        assert_eq!(recovered.extensions.get("future"), Some(&Value::Bool(true)));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn too_new_settings_are_preserved_and_never_defaulted() {
        let root = temp_path("too-new");
        let store = SettingsStore::new(root.join("settings.json"));
        fs::create_dir_all(&root).expect("root");
        fs::write(
            &store.path,
            br#"{"schema_version":2,"locale":"en-US","queue_capacity":8}"#,
        )
        .expect("write");

        assert!(matches!(store.load(), Err(SettingsError::TooNew(2))));
        assert!(store.path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn legacy_settings_gain_safe_independent_delivery_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"schema_version":1,"locale":"en-US","queue_capacity":8}"#)
                .expect("legacy settings");
        assert_eq!(settings.delivery, DeliveryPreferences::default());
        assert!(!settings.delivery.auto_copy);
        assert!(settings.delivery.auto_paste);
        assert!(!settings.delivery.restore_clipboard);
        assert!(settings.delivery.feedback_enabled);
        assert!(settings.delivery.app_capabilities.is_empty());
        assert_eq!(settings.hotkey, HotkeyPreferences::default());
        assert!(settings.hotkey.enabled);
        assert_eq!(
            settings.hotkey.activation_binding,
            DEFAULT_ACTIVATION_HOTKEY
        );
        assert_eq!(
            settings.hotkey.activation_mode,
            ActivationHotkeyMode::Toggle
        );
    }

    #[test]
    fn adding_hud_placement_never_makes_a_completed_profile_setup_incomplete() {
        // A profile written before the compact transcriber existed. It has
        // finished onboarding and chosen its history, retention, delivery and
        // hotkey settings. Reading it back must change none of that: the new
        // presentation field defaults and everything else survives untouched.
        let existing: Settings = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "locale": "en-US",
                "queue_capacity": 8,
                "onboarding": {
                    "completed": true,
                    "current_step": 7,
                    "privacy_reviewed": true,
                    "microphone_checked": true,
                    "hotkey_checked": true,
                    "model_choice_reviewed": true,
                    "try_it_completed": true,
                    "delivery_choice_reviewed": true
                },
                "privacy": { "persisted_history_enabled": true, "history_retention_days": 14 },
                "hotkey": { "activation_binding": "Ctrl+Alt+K", "enabled": true },
                "preferred_capture_device_id": "microphone-7"
            }"#,
        )
        .expect("a profile written before HUD placement existed still loads");

        assert!(
            existing.onboarding.completed,
            "migration must not re-open onboarding"
        );
        assert_eq!(existing.onboarding.current_step, 7);
        assert!(existing.privacy.persisted_history_enabled);
        assert_eq!(existing.privacy.history_retention_days, 14);
        assert_eq!(existing.hotkey.activation_binding, "Ctrl+Alt+K");
        assert_eq!(
            existing.preferred_capture_device_id.as_deref(),
            Some("microphone-7")
        );

        // Absent placement means "compute the default", not "unconfigured".
        assert_eq!(existing.hud_dock, HudDockPlacement::default());
        assert_eq!(existing.hud_dock.edge, HudDockEdge::Right);
        assert!(existing.hud_dock.position_y.is_none());
    }

    #[test]
    fn hud_dock_placement_round_trips_edge_and_position_without_a_size() {
        let settings = Settings {
            hud_dock: HudDockPlacement {
                edge: HudDockEdge::Left,
                position_y: Some(-220),
                monitor_id: Some("\\\\.\\DISPLAY1".to_owned()),
            },
            ..Settings::default()
        };
        let encoded = serde_json::to_string(&settings).expect("settings serialize");
        let decoded: Settings = serde_json::from_str(&encoded).expect("settings round-trip");
        assert_eq!(decoded.hud_dock, settings.hud_dock);
        assert_eq!(decoded.hud_dock.position_y, Some(-220));
        assert!(
            !encoded.contains("\"width\"") && !encoded.contains("\"height\""),
            "the dock is fixed-size; there is no size to persist"
        );
    }

    #[test]
    fn per_app_capability_requires_explicit_choice_and_can_be_reset() {
        let mut settings = Settings::default();
        assert!(settings.delivery.app_capabilities.is_empty());
        settings.delivery.app_capabilities.insert(
            "controlled-probe".to_owned(),
            AppDeliveryCapability {
                user_choice: LiveDeliveryChoice::AppendOnlyLive,
                evidence: Some(AppCapabilityEvidence {
                    schema_version: APP_CAPABILITY_EVIDENCE_SCHEMA_VERSION,
                    app_version: "1.0.0".to_owned(),
                    adapter_id: "speakeasy-controlled-probe-v1".to_owned(),
                    qualification_id: "campaign-a-row-1".to_owned(),
                }),
                downgrade_reason: Some("interactive_qualification_pending".to_owned()),
            },
        );
        assert!(validate(&settings).is_ok());
        assert!(settings.delivery.reset_app_capability("controlled-probe"));
        assert!(settings.delivery.app_capabilities.is_empty());
    }

    #[test]
    fn per_app_evidence_version_fails_closed() {
        let mut settings = Settings::default();
        settings.delivery.app_capabilities.insert(
            "controlled-probe".to_owned(),
            AppDeliveryCapability {
                user_choice: LiveDeliveryChoice::AppendOnlyLive,
                evidence: Some(AppCapabilityEvidence {
                    schema_version: APP_CAPABILITY_EVIDENCE_SCHEMA_VERSION + 1,
                    app_version: "1.0.0".to_owned(),
                    adapter_id: "speakeasy-controlled-probe-v1".to_owned(),
                    qualification_id: "future".to_owned(),
                }),
                downgrade_reason: None,
            },
        );
        assert!(matches!(validate(&settings), Err(SettingsError::Invalid)));
    }

    #[test]
    fn cloud_polish_is_off_before_commit_by_default_and_consent_resets() {
        let mut settings = Settings::default();
        let polish = &mut settings.privacy.cloud_polish;
        assert!(!polish.enabled);
        assert!(polish.before_commit);
        polish.enabled = true;
        polish.provider_id = Some("openai".to_owned());
        polish.model_id = Some("legacy-model".to_owned());
        polish.active_profile_id = Some("technical".to_owned());
        polish.consent = Some(CloudPolishConsent {
            provider_id: "openai".to_owned(),
            endpoint: "https://api.openai.com/v1/responses".to_owned(),
            privacy_policy_version: "2026-07".to_owned(),
            prompt_behavior_version: 1,
            credential_generation: 1,
            granted_unix_ms: 1,
        });
        assert!(polish.consent_is_current(
            "openai",
            "https://api.openai.com/v1/responses",
            "2026-07",
            1,
            1
        ));
        assert!(!polish.consent_is_current(
            "openai",
            "https://api.openai.com/v1/responses",
            "2026-07",
            1,
            2
        ));
        polish.reset_consent();
        assert!(!polish.enabled);
        assert!(polish.consent.is_none());
        assert!(validate(&settings).is_ok());
    }
}
