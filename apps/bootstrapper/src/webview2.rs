//! Whether the `WebView2` runtime this app cannot start without is present.
//!
//! The shipped NSIS installer provisioned it with Tauri's silent download
//! bootstrapper. Replacing NSIS meant replacing that too, and doing so ran into
//! a policy conflict worth recording rather than quietly resolving:
//!
//! - Fetch-first says download what can be downloaded.
//! - `models/trusted-manifest.json` says nothing is downloaded that is not
//!   pinned by URL, byte length and SHA-256, and that a mismatch is a
//!   supply-chain event rather than something to retry through.
//!
//! Microsoft's Evergreen Bootstrapper is served from a permanent redirect whose
//! bytes change by design, so it cannot be pinned. Owner decision (2026-08-15):
//! **the pinning rule wins and setup downloads nothing here.** It detects, and
//! when the runtime is absent it says so and refuses to claim the app will work.
//!
//! That is also the better failure. A missing `WebView2` makes the app fail to
//! start with no useful message; naming the cause during setup turns a silent
//! dead end into one sentence a user can act on.

/// Where the Evergreen runtime records itself.
///
/// Three locations, because the runtime registers differently depending on how
/// it arrived: per-machine installs land under `HKLM` (and under `WOW6432Node`
/// on 64-bit Windows, which is where a 64-bit runtime actually writes), and
/// per-user installs land under `HKCU`. Checking only one of them reports a
/// missing runtime on a machine that has it — a false negative that would send
/// users to install something they already have.
/// Rooted at `SOFTWARE\`, which is not decoration. Written without it this
/// resolved nothing and reported the runtime missing on a machine that had
/// 151.0.4129.78 installed and had been running the app an hour earlier —
/// caught 2026-08-15 by running the wizard, not by any test, because a registry
/// key that is absent and a registry key whose path is wrong are the same
/// `Err` and produce the same confident "not installed".
const CLIENT_KEY: &str =
    r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// The runtime's version, when it is installed.
///
/// `pv` is the value Microsoft documents as the presence check. An empty or
/// `0.0.0.0` value means registered-but-not-installed, which is a real state
/// after a failed install and must not read as present.
pub fn installed_version() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};

    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let candidates = [
        machine.open_subkey_with_flags(CLIENT_KEY, KEY_READ | KEY_WOW64_32KEY),
        machine.open_subkey_with_flags(CLIENT_KEY, KEY_READ),
        user.open_subkey_with_flags(CLIENT_KEY, KEY_READ),
    ];
    candidates
        .into_iter()
        .flatten()
        .filter_map(|key| key.get_value::<String, _>("pv").ok())
        .find(|version| is_installed_version(version))
}

/// Whether a `pv` value means the runtime is actually there.
fn is_installed_version(version: &str) -> bool {
    let trimmed = version.trim();
    !trimmed.is_empty() && trimmed != "0.0.0.0"
}

/// Whether the app will be able to start once installed.
pub fn is_present() -> bool {
    installed_version().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_but_uninstalled_runtime_is_not_present() {
        // The state left behind by a failed or removed install. Reading it as
        // present would let setup promise an app that cannot start.
        assert!(!is_installed_version("0.0.0.0"));
        assert!(!is_installed_version(""));
        assert!(!is_installed_version("   "));
        assert!(is_installed_version("120.0.2210.91"));
    }

    #[test]
    fn the_client_key_is_rooted_where_the_runtime_registers() {
        // The only part of this module a unit test can pin. Presence depends on
        // the host, so no test can assert the runtime is found — but a path that
        // silently resolves nothing is the failure that actually happened, and
        // it is a property of the string.
        assert!(
            CLIENT_KEY.starts_with(r"SOFTWARE\"),
            "the key must be rooted at SOFTWARE\\ or it resolves nothing under \
             either hive and reports a missing runtime on every machine"
        );
        assert!(
            CLIENT_KEY.ends_with('}'),
            "the client GUID must terminate the path"
        );
    }

    #[test]
    fn the_real_check_answers_without_panicking() {
        // Every registry branch is absent on some machine, and this runs on all
        // of them. Asserts nothing about the result: the answer is a property of
        // the host, not of the code.
        let _ = is_present();
    }
}
