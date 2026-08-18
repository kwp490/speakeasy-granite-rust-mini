use crate::{CredentialKey, CredentialManager, CredentialStatus};

pub const LEGACY_OPENAI_PRIMARY: CredentialKeyRef =
    CredentialKeyRef::new("speakeasy", "openai_api_key");
pub const LEGACY_OPENAI_FALLBACK: CredentialKeyRef =
    CredentialKeyRef::new("dictator", "openai_api_key");
pub const LEGACY_REMOTE_TOKEN: CredentialKeyRef =
    CredentialKeyRef::new("speakeasy", "remote_asr_token");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CredentialKeyRef {
    pub service: &'static str,
    pub username: &'static str,
}

impl CredentialKeyRef {
    pub const fn new(service: &'static str, username: &'static str) -> Self {
        Self { service, username }
    }

    fn owned(self) -> CredentialKey {
        CredentialKey {
            service: self.service.to_owned(),
            username: self.username.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyCredentialSource {
    PrimaryService,
    LegacyService,
    Missing,
    AccessDenied,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyCredentialReport {
    pub openai: LegacyCredentialSource,
    pub remote: LegacyCredentialSource,
}

trait SecretStore: Send + Sync {
    fn read(&self, key: &CredentialKey) -> Result<String, CredentialStatus>;
}

struct CredentialAdapter<S> {
    store: S,
}

impl<S: SecretStore> CredentialAdapter<S> {
    fn new(store: S) -> Self {
        Self { store }
    }

    fn legacy_report(&self) -> LegacyCredentialReport {
        let primary = self.status(&LEGACY_OPENAI_PRIMARY.owned());
        let fallback = self.status(&LEGACY_OPENAI_FALLBACK.owned());
        let openai = match (primary, fallback) {
            (CredentialStatus::Present, _) => LegacyCredentialSource::PrimaryService,
            (CredentialStatus::Missing, CredentialStatus::Present) => {
                LegacyCredentialSource::LegacyService
            }
            (CredentialStatus::AccessDenied, _) | (_, CredentialStatus::AccessDenied) => {
                LegacyCredentialSource::AccessDenied
            }
            (CredentialStatus::Unavailable, _) | (_, CredentialStatus::Unavailable) => {
                LegacyCredentialSource::Unavailable
            }
            _ => LegacyCredentialSource::Missing,
        };
        LegacyCredentialReport {
            openai,
            remote: source_for_status(self.status(&LEGACY_REMOTE_TOKEN.owned()), false),
        }
    }
}

impl<S: SecretStore> CredentialManager for CredentialAdapter<S> {
    fn status(&self, key: &CredentialKey) -> CredentialStatus {
        match self.store.read(key) {
            Ok(_) => CredentialStatus::Present,
            Err(status) => status,
        }
    }
}

fn source_for_status(status: CredentialStatus, legacy: bool) -> LegacyCredentialSource {
    match status {
        CredentialStatus::Present if legacy => LegacyCredentialSource::LegacyService,
        CredentialStatus::Present => LegacyCredentialSource::PrimaryService,
        CredentialStatus::Missing => LegacyCredentialSource::Missing,
        CredentialStatus::AccessDenied => LegacyCredentialSource::AccessDenied,
        CredentialStatus::Unavailable => LegacyCredentialSource::Unavailable,
    }
}

#[cfg(windows)]
pub struct WindowsCredentialStore;

#[cfg(windows)]
impl SecretStore for WindowsCredentialStore {
    fn read(&self, key: &CredentialKey) -> Result<String, CredentialStatus> {
        let entry =
            keyring::Entry::new(&key.service, &key.username).map_err(classify_keyring_error)?;
        entry.get_password().map_err(classify_keyring_error)
    }
}

#[cfg(windows)]
#[allow(clippy::needless_pass_by_value)]
fn classify_keyring_error(error: keyring::Error) -> CredentialStatus {
    match error {
        keyring::Error::NoEntry => CredentialStatus::Missing,
        keyring::Error::NoStorageAccess(_) => CredentialStatus::AccessDenied,
        _ => CredentialStatus::Unavailable,
    }
}

#[cfg(windows)]
pub struct WindowsCredentialManager(CredentialAdapter<WindowsCredentialStore>);

#[cfg(windows)]
impl Default for WindowsCredentialManager {
    fn default() -> Self {
        Self(CredentialAdapter::new(WindowsCredentialStore))
    }
}

#[cfg(windows)]
impl WindowsCredentialManager {
    pub fn legacy_report(&self) -> LegacyCredentialReport {
        self.0.legacy_report()
    }
}

#[cfg(windows)]
impl CredentialManager for WindowsCredentialManager {
    fn status(&self, key: &CredentialKey) -> CredentialStatus {
        self.0.status(key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeStore {
        values: Mutex<BTreeMap<(String, String), String>>,
        denied: Mutex<bool>,
    }

    impl SecretStore for FakeStore {
        fn read(&self, key: &CredentialKey) -> Result<String, CredentialStatus> {
            if *self.denied.lock().unwrap() {
                return Err(CredentialStatus::AccessDenied);
            }
            self.values
                .lock()
                .unwrap()
                .get(&(key.service.clone(), key.username.clone()))
                .cloned()
                .ok_or(CredentialStatus::Missing)
        }
    }

    #[test]
    fn legacy_lookup_is_exact_read_only_and_reports_fallback_service() {
        let store = FakeStore::default();
        store.values.lock().unwrap().insert(
            ("dictator".into(), "openai_api_key".into()),
            "fixture".into(),
        );
        let adapter = CredentialAdapter::new(store);
        assert_eq!(
            adapter.legacy_report().openai,
            LegacyCredentialSource::LegacyService
        );
        assert_eq!(
            adapter.status(&LEGACY_OPENAI_PRIMARY.owned()),
            CredentialStatus::Missing
        );
    }

    #[test]
    fn denied_store_never_becomes_missing_or_present() {
        let store = FakeStore::default();
        *store.denied.lock().unwrap() = true;
        let adapter = CredentialAdapter::new(store);
        assert_eq!(
            adapter.legacy_report().openai,
            LegacyCredentialSource::AccessDenied
        );
    }
}
