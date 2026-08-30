//! Whether an install may proceed, and the state it reads to decide.
//!
//! This is the half of `installer-hooks.nsh` that was never about copying files.
//! NSIS refused a same-version reinstall and a downgrade by comparing a registry
//! stamp against `${VERSION}` — what *this* installer carries — and the comment
//! above that comparison records why a literal there was actively wrong: spelled
//! out, the refusal is correct for exactly one release and wrong for the next,
//! and the stamp written at install time would record a version that was never
//! on disk.
//!
//! [`decide`] is deliberately pure. The rules are the product's contract about
//! when it will and will not overwrite an existing installation, they are
//! exercised by `Test-InstallerLifecycle.ps1` against a real installer, and they
//! are the kind of logic that is easy to get subtly wrong and impossible to see
//! wrong from the outside — a downgrade refusal that silently permits is
//! indistinguishable from one that was never triggered.

use std::path::{Path, PathBuf};

use semver::Version;

/// Where the installed version is stamped.
///
/// Under this product's own name. Until 2026-08-18 it was keyed to the parent
/// product's name instead, inherited by the fork along with a justification
/// that cannot apply here: the key was shared with the NSIS hooks "because an
/// upgrade from a pre-bootstrapper install has to find the stamp its
/// predecessor wrote". `SpeakEasy Mini` has no predecessor. It has never
/// shipped, so no machine anywhere carries a Mini stamp to find.
///
/// What sharing it did instead was aim this product's writes at `SpeakEasy`'s
/// record: installing Mini overwrote the parent's version stamp, and Mini's
/// downgrade refusal compared its own version against whatever `SpeakEasy` had
/// installed -- so the two products could refuse each other's upgrades.
/// Uninstalling Mini then removed the record entirely (see
/// `uninstall.rs`'s removable list), leaving `SpeakEasy` installed and
/// unstamped, which is the state its own installer reads as a fresh machine.
pub const VERSION_KEY: &str = r"Software\SpeakEasy Mini\LocalDevelopment";
pub const VERSION_VALUE: &str = "Version";

/// The product's own key, which [`VERSION_KEY`] hangs beneath.
///
/// Named so an uninstall can clean it up. Deleting `VERSION_KEY` removes the
/// stamp and leaves this behind, empty and carrying the product's name -- which
/// is the registry's version of the empty directory tree the 2026-08-21
/// uninstall work went out of its way to remove, and it was still there after
/// the first real end-to-end removal on that same day. It is only ever deleted
/// when empty: something else putting a key here is not this uninstaller's to
/// guess about.
pub const VERSION_KEY_ROOT: &str = r"Software\SpeakEasy Mini";

/// What setup is allowed to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    /// Nothing is installed. Ordinary first run.
    Fresh,
    /// A strictly older version is installed.
    Upgrade { from: Version },
    /// `SpeakEasy` is running. Refused before anything else is considered,
    /// because the check is about files that cannot be replaced rather than
    /// about which version they are.
    RefuseRunning,
    /// Exactly this version is installed. Refused so that "install again" is
    /// never the accidental answer to a broken installation — repair is.
    RefuseSameVersion { installed: Version },
    /// A newer version is installed. Never automatic: going back is a decision
    /// with data implications, and the repair verbs are where it is made
    /// explicitly.
    RefuseDowngrade { installed: Version },
    /// A stamp exists but is not a version.
    ///
    /// Distinct from `Fresh` on purpose. A machine with an unreadable stamp has
    /// *something* installed, and treating that as a clean machine would
    /// overwrite it while reporting a first install.
    RefuseUnreadableStamp { found: String },
}

impl Decision {
    /// Whether setup may write to the install directory.
    pub const fn may_proceed(&self) -> bool {
        matches!(self, Self::Fresh | Self::Upgrade { .. })
    }
}

/// Read the machine and decide.
///
/// The two readings happen together and once, so the answer shown to the user is
/// the answer that was true at a single moment. Re-reading per navigation would
/// let the app be closed between the refusal and the explanation of it.
pub fn decide_now() -> Decision {
    decide(
        installed_stamp().as_deref(),
        &candidate_version(),
        app_is_running(),
    )
}

/// Decide whether this installer may run.
///
/// `installed` is the raw registry stamp, exactly as read — parsing happens here
/// so that an unparseable value is a decision rather than a panic at the call
/// site. `candidate` is what this binary carries, which is
/// `env!("CARGO_PKG_VERSION")` at every real call site and a literal only in
/// tests.
pub fn decide(installed: Option<&str>, candidate: &Version, running: bool) -> Decision {
    // First, and before any version reasoning. A running app cannot have its
    // files replaced, and answering "you already have this version" to someone
    // whose real problem is that it is open sends them to the wrong fix.
    if running {
        return Decision::RefuseRunning;
    }
    let Some(stamp) = installed else {
        return Decision::Fresh;
    };
    let trimmed = stamp.trim();
    if trimmed.is_empty() {
        // NSIS wrote an empty string rather than deleting the value in some
        // paths, and `ReadRegStr` returns empty for a missing value too, so an
        // empty stamp has always meant "nothing installed" here.
        return Decision::Fresh;
    }
    let Ok(installed) = Version::parse(trimmed) else {
        return Decision::RefuseUnreadableStamp {
            found: trimmed.to_owned(),
        };
    };
    match installed.cmp(candidate) {
        std::cmp::Ordering::Less => Decision::Upgrade { from: installed },
        std::cmp::Ordering::Equal => Decision::RefuseSameVersion { installed },
        std::cmp::Ordering::Greater => Decision::RefuseDowngrade { installed },
    }
}

/// The version this binary carries.
///
/// Read from the package version rather than written down, for the reason the
/// NSIS comment gives: a literal is correct for one release and wrong for every
/// one after it.
pub fn candidate_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("the crate's own version is set by cargo and is always semver")
}

/// Read the installed-version stamp.
pub fn installed_stamp() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(VERSION_KEY)
        .ok()?
        .get_value::<String, _>(VERSION_VALUE)
        .ok()
}

/// Where the installation on this machine actually is.
///
/// Read from the `InstallLocation` that [`register_uninstall`] wrote, not
/// assumed to be [`crate::probe::install_root`]. The two are the same for every
/// ordinary install and different for exactly the cases that matter: an install
/// directed elsewhere with `--install-root`, which is how
/// `Test-InstallerLifecycle.ps1` proves the whole lifecycle without touching the
/// real one.
///
/// This was assumed rather than read, and the result was an uninstall that
/// reported "`SpeakEasy` has been removed" having deleted nothing at all
/// (measured 2026-08-15): it removed the registration and the shortcuts, which
/// are global, then went looking for program files in the default directory —
/// which held only the spared GPU runtime — while the installation it was
/// launched from sat untouched somewhere else. Nothing errored, and the exit
/// code was zero.
///
/// `None` when nothing is registered, which is the ordinary "not installed"
/// case; the caller falls back to the default so that an uninstall can still
/// tidy up after a registration that was already removed.
pub fn installed_location() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let location = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?
        .get_value::<String, _>("InstallLocation")
        .ok()?;
    if location.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(location))
}

/// The installed app, if there is one on disk to start.
///
/// Resolved from the recorded `InstallLocation` first, exactly as
/// [`installed_location`] describes, so setup started with `--install-root`
/// launches the app it just placed rather than one in the default directory.
///
/// The `is_file` check is the difference between setup saying it started the
/// app and setup having started it: `Command::spawn` on a missing path fails,
/// but so does a great deal else, and this is the one cause worth telling
/// apart — it means the install did not put the executable where it recorded.
#[must_use]
pub fn installed_app_executable() -> Option<PathBuf> {
    let root = installed_location().or_else(crate::probe::install_root)?;
    let executable = root.join(APP_EXE);
    executable.is_file().then_some(executable)
}

/// Whether `SpeakEasy Mini` is running.
///
/// One executable name. This carried a second, `speakeasy-v2-preview.exe`,
/// because the *parent* product's legacy preview shared its install directory
/// and a machine still running it held files that could not be replaced. No
/// build of `SpeakEasy Mini` was ever called that, and its install directory is
/// not shared with either, so keeping the entry only risked refusing a Mini
/// install because something unrelated was running.
pub fn app_is_running() -> bool {
    const RUNNING_NAMES: &[&str] = &["ai-speakeasy-mini.exe"];

    let system = sysinfo::System::new_all();
    system.processes().values().any(|process| {
        process.name().to_str().is_some_and(|name| {
            RUNNING_NAMES
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
        })
    })
}

/// Place the payload and register the installation.
///
/// Order is deliberate and is the reverse of how it fails. Files first, because
/// everything else describes them; the version stamp last, because it is what
/// every future run reads to decide whether an installation exists. A stamp
/// written before the files would make a failed install look complete, and the
/// next run would refuse to repair it as a same-version reinstall.
pub fn perform(payload: &Path, install_root: &Path) -> Result<(), String> {
    if !payload.is_dir() {
        return Err(format!(
            "no payload to install: {} does not exist",
            payload.display()
        ));
    }
    copy_tree(payload, install_root)?;
    register_uninstall(install_root)?;
    create_shortcut(install_root)?;
    write_stamp(&candidate_version())?;
    Ok(())
}

/// Copy a directory tree over whatever is already there.
///
/// **Merges**, and that is a deliberate trade rather than an oversight. It has
/// to: on a graphics-card installation `proof/` holds ~493 MiB of NVIDIA
/// libraries and a 54 MiB CUDA worker that setup fetched, and wiping the
/// directory on every upgrade would re-download all of it. The model weights are
/// not here — they live under `%APPDATA%` — so this figure is the runtime only.
///
/// The cost is orphans — a file from a previous layout that the current one does
/// not include survives, and it is still loadable. Observed 2026-08-15 on a real
/// install: `granite-worker.cpu.exe`, left by the interim
/// `scripts/Enable-GraniteCuda.ps1`, was still sitting in `proof/` afterwards.
/// The NSIS hooks handled the same problem by naming known orphans and deleting
/// them by hand. Whatever replaces that here has to know which files are
/// app-owned and which were fetched, and cannot simply clear the directory.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|error| format!("could not write {}: {error}", target.display()))?;
        }
    }
    Ok(())
}

fn write_stamp(version: &Version) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey_with_flags(VERSION_KEY, KEY_WRITE)
        .map_err(|error| error.to_string())?;
    key.set_value(VERSION_VALUE, &version.to_string())
        .map_err(|error| error.to_string())
}

/// Register in Add/Remove Programs.
///
/// Tauri's bundler generated this and nobody had to write it. Losing it is the
/// quietest part of dropping NSIS: the app works perfectly and simply cannot be
/// found in Settings, which reads as an app that refuses to uninstall.
fn register_uninstall(install_root: &Path) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};

    let bootstrapper = install_root.join("speakeasy-bootstrapper.exe");
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey_with_flags(UNINSTALL_KEY, KEY_WRITE)
        .map_err(|error| error.to_string())?;
    let set = |name: &str, value: &str| {
        key.set_value(name, &value.to_owned())
            .map_err(|error| error.to_string())
    };
    set("DisplayName", crate::probe::PRODUCT)?;
    set("DisplayVersion", &candidate_version().to_string())?;
    set("Publisher", crate::probe::PRODUCT)?;
    set("InstallLocation", &install_root.to_string_lossy())?;
    set(
        "UninstallString",
        &format!("\"{}\" --uninstall", bootstrapper.display()),
    )?;
    set("DisplayIcon", &install_root.join(APP_EXE).to_string_lossy())?;
    // No repair or modify entry: repair is the CLI verbs, not a Settings button,
    // and offering one that does nothing is worse than offering none.
    key.set_value("NoModify", &1u32)
        .map_err(|error| error.to_string())?;
    key.set_value("NoRepair", &1u32)
        .map_err(|error| error.to_string())
}

fn create_shortcut(install_root: &Path) -> Result<(), String> {
    let Some(folder) = crate::shortcut::start_menu_folder() else {
        // No APPDATA. The install is still valid without a shortcut, and
        // failing the whole thing over a convenience would be the wrong trade.
        return Ok(());
    };
    let link = folder.join("SpeakEasy Mini.lnk");
    let target = install_root.join(APP_EXE);
    crate::shortcut::create(&link, &target, "SpeakEasy Mini dictation")
        .map_err(|error| error.to_string())?;

    // Read it back. A `.lnk` that saved without error but points somewhere else
    // is a valid shortcut file and looks correct in Explorer — the only way to
    // tell is to ask it. This project has been bitten more than once by treating
    // "the call returned" as evidence the thing happened.
    let written = crate::shortcut::target_of(&link).map_err(|error| error.to_string())?;
    if written.eq_ignore_ascii_case(&target.to_string_lossy()) {
        Ok(())
    } else {
        Err(format!(
            "the Start Menu shortcut was written but points at {written}"
        ))
    }
}

/// Where Add/Remove Programs looks, for a current-user installation.
///
/// Keyed by this product's own identifier. It was keyed by
/// `ai.speakeasy.desktop` until 2026-08-18, which meant setup registered
/// `SpeakEasy Mini` under *`SpeakEasy`'s* Add/Remove Programs entry -- overwriting
/// its `DisplayName`, its version and its uninstall command -- and Mini's
/// uninstaller then deleted that entry, leaving the parent product installed
/// and unlisted. See `uninstall::data_root` for the same inherited identifier
/// doing the same damage to the data directory.
pub const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ai.speakeasy.mini";

/// The app executable, as `tauri.conf.json`'s product name produces it.
const APP_EXE: &str = "ai-speakeasy-mini.exe";

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).expect("test version")
    }

    #[test]
    fn placing_a_missing_payload_fails_before_anything_is_registered() {
        // The order in `perform` is what makes this safe to assert: a payload
        // that is not there must not leave a version stamp behind, because the
        // next run would read it and refuse to install over a machine that has
        // nothing on it.
        // A path inside a directory that exists, pointing at a file that does
        // not. Derived from a `TempDir` rather than named in the shared temp
        // directory, so nothing is assumed about what another process left
        // lying around under a guessable name.
        let absent_dir = tempfile::tempdir().expect("a temporary directory");
        let absent = absent_dir.path().join("payload-that-does-not-exist");
        let destination = std::env::temp_dir().join(format!(
            "speakeasy-install-target-unused-{}",
            std::process::id()
        ));

        let outcome = perform(&absent, &destination);

        assert!(outcome.is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn a_payload_tree_is_copied_whole() {
        // A real temporary directory, bound so it is removed when this
        // test ends -- on a panic too, which a fixed or pid-suffixed name
        // under the shared temp directory never was.
        let root_dir = tempfile::tempdir().expect("a temporary directory");
        let root = root_dir.path().to_path_buf();
        let source = root.join("from");
        let destination = root.join("to");
        std::fs::create_dir_all(source.join("proof")).expect("source tree");
        std::fs::write(source.join("app.exe"), b"app").expect("file");
        std::fs::write(source.join("proof").join("worker.exe"), b"worker").expect("nested file");

        copy_tree(&source, &destination).expect("copied");

        assert_eq!(
            std::fs::read(destination.join("app.exe")).expect("copied file"),
            b"app"
        );
        assert_eq!(
            std::fs::read(destination.join("proof").join("worker.exe")).expect("nested"),
            b"worker"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_running_app_is_refused_before_any_version_is_considered() {
        // Otherwise a user whose only problem is that the app is open is told
        // they already have this version, which sends them to the wrong fix.
        assert_eq!(
            decide(Some("1.0.0"), &version("9.9.9"), true),
            Decision::RefuseRunning
        );
        assert_eq!(
            decide(None, &version("1.4.2"), true),
            Decision::RefuseRunning
        );
    }

    #[test]
    fn same_version_and_downgrade_are_refused_and_upgrade_is_not() {
        assert_eq!(
            decide(Some("1.4.2"), &version("1.4.2"), false),
            Decision::RefuseSameVersion {
                installed: version("1.4.2")
            }
        );
        assert_eq!(
            decide(Some("2.0.0"), &version("1.4.2"), false),
            Decision::RefuseDowngrade {
                installed: version("2.0.0")
            }
        );
        assert_eq!(
            decide(Some("1.4.1"), &version("1.4.2"), false),
            Decision::Upgrade {
                from: version("1.4.1")
            }
        );
    }

    #[test]
    fn versions_compare_as_versions_not_as_strings() {
        // "1.10.0" sorts below "1.9.0" as text. A string comparison here would
        // call a real upgrade a downgrade and refuse it, on exactly the release
        // where the minor version first reaches double digits.
        assert_eq!(
            decide(Some("1.9.0"), &version("1.10.0"), false),
            Decision::Upgrade {
                from: version("1.9.0")
            }
        );
    }

    #[test]
    fn an_unreadable_stamp_is_not_treated_as_a_clean_machine() {
        // A machine with a corrupt stamp has something installed. Calling it
        // fresh would overwrite it while reporting a first install.
        let decision = decide(Some("not-a-version"), &version("1.4.2"), false);

        assert_eq!(
            decision,
            Decision::RefuseUnreadableStamp {
                found: "not-a-version".to_owned()
            }
        );
        assert!(!decision.may_proceed());
    }

    #[test]
    fn an_absent_or_empty_stamp_is_a_fresh_install() {
        // `ReadRegStr` returns empty for a missing value, and NSIS wrote an
        // empty string rather than deleting in some paths, so both have always
        // meant the same thing here.
        assert_eq!(decide(None, &version("1.4.2"), false), Decision::Fresh);
        assert_eq!(decide(Some(""), &version("1.4.2"), false), Decision::Fresh);
        assert_eq!(
            decide(Some("  "), &version("1.4.2"), false),
            Decision::Fresh
        );
    }

    #[test]
    fn only_fresh_and_upgrade_may_write() {
        assert!(Decision::Fresh.may_proceed());
        assert!(
            Decision::Upgrade {
                from: version("1.0.0")
            }
            .may_proceed()
        );
        assert!(!Decision::RefuseRunning.may_proceed());
        assert!(
            !Decision::RefuseSameVersion {
                installed: version("1.4.2")
            }
            .may_proceed()
        );
        assert!(
            !Decision::RefuseDowngrade {
                installed: version("2.0.0")
            }
            .may_proceed()
        );
    }

    #[test]
    fn the_carried_version_parses() {
        // `candidate_version` panics on a bad package version by design; this is
        // what makes that panic unreachable rather than merely unlikely.
        assert_eq!(candidate_version().to_string(), env!("CARGO_PKG_VERSION"));
    }
}
