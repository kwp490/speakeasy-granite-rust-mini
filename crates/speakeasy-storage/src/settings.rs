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
    /// Keys this build does not read, kept so `save` writes them back.
    ///
    /// A profile written before cloud polish was retired carries a
    /// `cloud_polish` object here. Nothing reads or validates it; it is
    /// preserved rather than dropped so an older build opening the same profile
    /// finds what it wrote.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
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
            extensions: BTreeMap::new(),
        }
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
    /// A deliberate, revocable choice to run a worker other than the one setup
    /// installed. `None` — the default, and every profile written before this
    /// field existed — means "use what setup installed", which is exactly
    /// today's behavior with nothing new to opt into.
    ///
    /// This is not `install-provider.txt`: that file is a permanent record of
    /// what setup's own engine check proved; this is a mutable preference read
    /// fresh at every warm, and it can only ever select a binary setup already
    /// staged and verified. See `RuntimePaths::granite_worker_alternate`.
    #[serde(default)]
    pub engine_provider_override: Option<EngineProvider>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

/// A worker binary the in-app switch can select, when setup staged it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineProvider {
    Cpu,
    Cuda,
}

impl EngineProvider {
    /// The same vocabulary `install-provider.txt` and `recorded_provider`
    /// already speak, so a caller can compare the two without a second mapping.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }
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
            privacy: PrivacyPreferences::default(),
            writing_rules: WritingRulePreferences::default(),
            startup_with_windows: false,
            theme: ThemePreference::System,
            preferred_capture_device_id: None,
            hud_dock: HudDockPlacement::default(),
            engine_provider_override: None,
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
        // A missing primary beside a backup means a save did not finish, or the
        // file was removed. Either way the backup is the user's profile and
        // defaults would silently replace it.
        if !self.path.exists() {
            if self.backup_path.exists() {
                return read_settings(&self.backup_path)
                    .map(|settings| (settings, LoadOutcome::RecoveredFromBackup));
            }
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
        }
        // `fs::rename` replaces an existing destination on Windows
        // (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). Removing the primary
        // first would open a window in which no primary exists.
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
    fn a_missing_primary_recovers_from_the_backup_instead_of_defaulting() {
        let root = temp_path("missing-primary");
        let store = SettingsStore::new(root.join("settings.json"));
        let first = Settings {
            locale: "fr-FR".to_owned(),
            ..Settings::default()
        };
        store.save(&first).expect("first save");
        store.save(&first).expect("second save writes the backup");
        fs::remove_file(&store.path).expect("remove primary");

        let (recovered, outcome) = store.load().expect("backup recovery");
        assert_eq!(outcome, LoadOutcome::RecoveredFromBackup);
        assert_eq!(recovered.locale, "fr-FR");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn save_replaces_an_existing_primary_and_refreshes_the_backup() {
        let root = temp_path("replace");
        let store = SettingsStore::new(root.join("settings.json"));
        let first = Settings {
            locale: "fr-FR".to_owned(),
            ..Settings::default()
        };
        store.save(&first).expect("first save");
        let second = Settings {
            locale: "de-DE".to_owned(),
            ..Settings::default()
        };
        store.save(&second).expect("replacing save");

        assert_eq!(store.load().expect("load").0.locale, "de-DE");
        assert_eq!(
            read_settings(&store.backup_path).expect("backup").locale,
            "fr-FR"
        );
        assert!(!store.path.with_extension("json.tmp").exists());
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

        // The `onboarding` object above is no longer a field. It has to keep
        // loading rather than failing the parse, and it lands in `extensions`
        // where `save` writes it back -- so removing the in-app setup wizard
        // does not quietly discard part of a profile written before it went.
        assert!(
            existing.extensions.contains_key("onboarding"),
            "a retired field must survive in extensions"
        );
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

        // A profile written before this field existed must load as "use what
        // setup installed" -- the same behavior as before this field existed --
        // not as an unconfigured or invalid state.
        assert_eq!(existing.engine_provider_override, None);
    }

    #[test]
    fn engine_provider_override_round_trips_and_defaults_to_none() {
        let settings = Settings {
            engine_provider_override: Some(EngineProvider::Cuda),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let reloaded: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            reloaded.engine_provider_override,
            Some(EngineProvider::Cuda)
        );
        assert_eq!(EngineProvider::Cuda.code(), "cuda");
        assert_eq!(EngineProvider::Cpu.code(), "cpu");

        assert_eq!(Settings::default().engine_provider_override, None);
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
    fn a_retired_cloud_polish_object_loads_and_survives_a_save() {
        let root = temp_path("retired-polish");
        let store = SettingsStore::new(root.join("settings.json"));
        fs::create_dir_all(&root).expect("root");
        // Enabled with no provider or consent: the retired validation refused
        // this, and a profile holding it must still open now that nothing reads it.
        fs::write(
            &store.path,
            br#"{"schema_version":1,"locale":"en-US","queue_capacity":8,
                "privacy":{"persisted_history_enabled":true,"history_retention_days":14,
                "cloud_polish":{"enabled":true,"per_app_profiles":{"a.exe":"x"}}}}"#,
        )
        .expect("write");

        let (loaded, outcome) = store.load().expect("load");
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(loaded.privacy.persisted_history_enabled);
        assert_eq!(loaded.privacy.history_retention_days, 14);
        store.save(&loaded).expect("save");

        let written: Value =
            serde_json::from_slice(&fs::read(&store.path).expect("read")).expect("json");
        assert_eq!(
            written["privacy"]["cloud_polish"]["per_app_profiles"]["a.exe"],
            Value::String("x".to_owned())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
