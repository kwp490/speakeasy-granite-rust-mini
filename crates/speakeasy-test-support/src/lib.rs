//! Deterministic fake and fixture support boundary for `SpeakEasy` tests.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use speakeasy_domain::{Clock, MonotonicTime};
use speakeasy_windows::{
    CredentialKey, CredentialManager, CredentialStatus, ShortcutOwner, ShortcutRegistration,
};

#[derive(Debug, Default)]
pub struct FakeClock(AtomicU64);

impl FakeClock {
    pub fn advance_to(&self, monotonic_ns: u64) {
        self.0.store(monotonic_ns, Ordering::Release);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> MonotonicTime {
        MonotonicTime(self.0.load(Ordering::Acquire))
    }
}

#[derive(Debug, Default)]
pub struct FakeCredentialManager {
    statuses: BTreeMap<(String, String), CredentialStatus>,
}

impl FakeCredentialManager {
    #[must_use]
    pub fn with_status(mut self, key: CredentialKey, status: CredentialStatus) -> Self {
        self.statuses.insert((key.service, key.username), status);
        self
    }
}

impl CredentialManager for FakeCredentialManager {
    fn status(&self, key: &CredentialKey) -> CredentialStatus {
        self.statuses
            .get(&(key.service.clone(), key.username.clone()))
            .copied()
            .unwrap_or(CredentialStatus::Missing)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FakeShortcutOwner {
    pub registration: ShortcutRegistration,
}

impl ShortcutOwner for FakeShortcutOwner {
    fn register_activation(&self) -> ShortcutRegistration {
        self.registration
    }

    fn unregister_activation(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_fake_reports_presence_without_secret_values() {
        let key = CredentialKey {
            service: "speakeasy".into(),
            username: "openai_api_key".into(),
        };
        let fake =
            FakeCredentialManager::default().with_status(key.clone(), CredentialStatus::Present);
        assert_eq!(fake.status(&key), CredentialStatus::Present);
    }

    #[test]
    fn fake_clock_and_shortcut_are_deterministic() {
        let clock = FakeClock::default();
        clock.advance_to(42);
        assert_eq!(clock.now(), MonotonicTime(42));
        let shortcut = FakeShortcutOwner {
            registration: ShortcutRegistration::RegisteredNoOp,
        };
        assert_eq!(
            shortcut.register_activation(),
            ShortcutRegistration::RegisteredNoOp
        );
    }
}
