//! Removing an installation, and choosing how much of it to remove.
//!
//! NSIS asked four keep-or-remove questions as separate modal dialogs, plus a
//! fifth page for the downloaded CUDA runtime. Owner decision (2026-08-15): one
//! page, all of them as checkboxes. Seeing the whole scope of a deletion before
//! confirming any of it is the point; sequential prompts answered blind is how
//! someone removes their transcript history without noticing.
//!
//! # The user-facing default is a clean machine (2026-08-21)
//!
//! It was keep-everything, inherited from `/SD IDYES`. Owner decision: an
//! uninstall should leave nothing behind, the checkbox that keeps the model
//! weights defaults to *removing* them, and keeping them is a **testing**
//! affordance — `--keep-user-data` — rather than the production path. A product
//! that leaves 2.14 GB of weights and a settings tree behind after the user
//! asked it to go is not uninstalled, it is hidden.
//!
//! [`Removals::default`] still selects **nothing**, and that is not a
//! contradiction: it is the *API* default, so a caller that forgets to ask
//! deletes nothing. The inversion is at the command line, where the question has
//! actually been asked.
//!
//! # What stopped being true
//!
//! `Removable::GpuRuntime` and the rule that spared `proof/` selectively were
//! both retired here. They protected an on-demand CUDA runtime download into the
//! install root that cost ~2.97 GB to repeat — and **this fork has no such
//! download**. It left with the streaming engine: nothing in the tree creates
//! `.cuda-runtime-download` or `.cuda-runtime-stage`, and the model weights live
//! under `%APPDATA%`, not here. So the rule's stated reason had been dead since
//! the fork, and the only thing it still spared was `scripts/Enable-GraniteCuda.ps1`'s
//! own leftovers — which that script removes itself with `-Revert`.
//!
//! Program files are never optional. Everything below is user data, and the
//! distinction is the contract: uninstalling removes the program, and removes
//! user data where asked — which is now by default.

use std::path::{Path, PathBuf};

/// One thing an uninstall may optionally remove.
///
/// An enum with a list rather than five booleans on a struct: the uninstall page
/// renders one checkbox per entry by iterating [`Removable::ALL`], so adding a
/// sixth thing to offer cannot produce a choice the page forgets to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Removable {
    Configuration,
    History,
    Models,
    Recovery,
    /// The diagnostic log and its one rotated generation.
    ///
    /// Added 2026-08-21, taking the slot `GpuRuntime` gave up. Not for symmetry:
    /// without it, `everything()` left the profile's own `logs` directory
    /// behind and therefore left the data root behind too, so an uninstall that
    /// removed the weights, the history, the settings and the recovery backups
    /// still could not report a clean machine.
    Logs,
}

impl Removable {
    pub const ALL: [Self; 5] = [
        Self::Configuration,
        Self::History,
        Self::Models,
        Self::Recovery,
        Self::Logs,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Configuration => 0,
            Self::History => 1,
            Self::Models => 2,
            Self::Recovery => 3,
            Self::Logs => 4,
        }
    }

    /// What the checkbox says.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "Settings and personalization",
            Self::History => "Transcript history",
            Self::Models => "Downloaded speech models",
            Self::Recovery => "Recovery backups",
            Self::Logs => "Diagnostic logs",
        }
    }
}

/// What the user chose to remove beyond the program itself.
///
/// [`Default`] selects nothing — keep everything — so a caller that forgets to
/// ask deletes nothing. The opposite default would make an omission destructive,
/// and this is the one place in the product where the wrong default cannot be
/// undone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Removals {
    selected: [bool; Removable::ALL.len()],
}

impl Removals {
    pub const fn includes(self, item: Removable) -> bool {
        self.selected[item.index()]
    }

    pub const fn select(&mut self, item: Removable, remove: bool) {
        self.selected[item.index()] = remove;
    }

    /// Select everything.
    ///
    /// What an ordinary uninstall now does, and what leaves a genuinely clean
    /// machine — the state `CLAUDE.md` says is required to test first-run
    /// honestly, since sparing the weights makes the next setup look faster and
    /// simpler than it is for a real new user. `--keep-user-data` is the opt-out,
    /// and exists for rapid install/uninstall cycles rather than for users.
    pub fn everything() -> Self {
        let mut removals = Self::default();
        for item in Removable::ALL {
            removals.select(item, true);
        }
        removals
    }
}

/// What an uninstall actually did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
    /// Files that could not be removed now but are not a failed uninstall.
    ///
    /// Exactly one thing ever lands here — this program's own executable, on a
    /// machine where it could not be moved out of the way. Deliberately separate
    /// from [`Outcome::failed`], because it must not turn a successful uninstall
    /// into a non-zero exit: the product *is* removed at that point, and a
    /// script proving uninstall works would otherwise fail over a two-megabyte
    /// residue of the uninstaller itself.
    pub left_behind: Vec<String>,
    /// Files removed from `proof/` that this installer did not put there.
    ///
    /// Reported by name rather than counted, because they are the one thing an
    /// uninstall removes that nobody declared: it was
    /// `scripts/Enable-GraniteCuda.ps1`'s staged CUDA libraries until setup
    /// began staging those itself on 2026-08-26, and it is whatever the next
    /// interim script leaves. They used to be spared forever and silently.
    /// Removing them without saying so would be the same silence pointing the
    /// other way.
    pub removed_unrecognised: Vec<String>,
}

/// Where the app's data lives.
///
/// `ai.speakeasy.mini`, and the identifier is the whole point of this
/// function. It said `ai.speakeasy.desktop` until 2026-08-18 -- the *parent*
/// product's identifier, inherited by the fork and never changed -- and two
/// things followed that nothing caught, because the installer had not been run
/// since the fork.
///
/// Setup downloads the model weights under this root, so ~2.3 GB went to
/// `%APPDATA%/ai.speakeasy.desktop/model-lifecycle/models/`. The app reads
/// `%APPDATA%/ai.speakeasy.mini/model-lifecycle/models/`, from Tauri's
/// `app_data_dir` and its own identifier. A verified dictation found them in
/// the second. So a fresh install would have downloaded the weights into
/// `SpeakEasy`'s directory and then reported Granite as not installed.
///
/// Worse in the other direction: uninstalling `SpeakEasy Mini` removes this
/// tree. Pointed at the parent's identifier, that is `SpeakEasy`'s data.
///
/// The whole reason this fork ships under its own identifier is to install and
/// run beside `SpeakEasy` without sharing settings, logs, a single-instance lock
/// or a shortcut. A hardcoded identifier is the one place that promise is kept
/// or broken, which is why it is spelled out here rather than derived.
pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("ai.speakeasy.mini"))
}

/// Remove an installation.
///
/// Order mirrors [`crate::install::perform`] reversed, and for the same reason:
/// the version stamp goes **first**, so a run interrupted halfway leaves a
/// machine that reads as "not installed" rather than one that reads as a
/// complete install with its files gone. The second state refuses to reinstall
/// over itself; the first recovers by installing again.
pub fn perform(install_root: &Path, removals: Removals) -> Outcome {
    let mut outcome = Outcome::default();

    clear_registration(&mut outcome);
    remove_shortcuts(&mut outcome);
    remove_program_files(install_root, &vacate(install_root), &mut outcome);
    remove_user_data(removals, &mut outcome);
    outcome
}

/// Where this process's own executable ended up before the files were removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunningImage {
    /// Not inside the install root, so it was never in the way. This is the case
    /// for a bootstrapper run from a downloads folder against an install
    /// elsewhere.
    Elsewhere,
    /// Moved out of the install root. The directory can be removed whole.
    Relocated,
    /// Still in the install root, and it cannot be removed while this process
    /// runs. Carries the canonical path so the delete walk can skip it, and the
    /// reason so the user is told which one it was.
    Retained { canonical: PathBuf, reason: String },
}

/// Get this process out of the way of its own uninstall.
///
/// Two separate holds, and both are invisible until the last `remove_dir` fails
/// with a message about neither of them.
///
/// - **The working directory.** Explorer gives a double-clicked program the
///   folder it lives in as its working directory, so an uninstall started from
///   the Start Menu or from `speakeasy-bootstrapper.exe` in Explorer is standing
///   *inside* the directory it is about to remove, and Windows will not remove a
///   directory that any process has open as its current one. Moved
///   unconditionally rather than conditionally: the check for whether it matters
///   is longer than the fix, and being in the temp directory is harmless.
/// - **The executable image.** Windows will not let a running program delete its
///   own file. Measured 2026-08-15: deleting refuses with access denied, but
///   **moving it out of the directory succeeds and the process keeps running** —
///   a rename does not invalidate the mapped image, which is the same property
///   self-updating programs rely on. So the uninstaller steps out of the install
///   root instead of leaving a hole in it.
///
/// Both obvious alternatives were rejected, and it is worth recording why:
///
/// - **Copy to `%TEMP%` and re-execute** — what NSIS's uninstaller did — makes
///   the exit code a lie. The parent has to exit before the child can delete the
///   parent's image, so the process a caller waits on returns *before* the work
///   is done. `Test-InstallerLifecycle.ps1` waits on exactly that process and
///   then immediately asserts the files are gone, so this trades a leftover file
///   for a race, which is the worse of the two.
/// - **`MoveFileEx` with `MOVEFILE_DELAY_UNTIL_REBOOT`** writes
///   `PendingFileRenameOperations` under `HKLM`, which needs the administrator
///   rights this install deliberately never asks for. It would also leave the
///   directory sitting there until the user next reboots.
fn vacate(install_root: &Path) -> RunningImage {
    let temporary = std::env::temp_dir();
    let _ = std::env::set_current_dir(&temporary);

    let Ok(canonical) = std::env::current_exe().and_then(|path| path.canonicalize()) else {
        return RunningImage::Elsewhere;
    };
    // Canonical on both sides. `install_root` arrives from a command line and
    // may be relative, differently cased or carry a trailing separator, and a
    // textual comparison that misses would leave the running image to be deleted
    // — which fails, reporting the uninstall as broken.
    let Ok(root) = install_root.canonicalize() else {
        return RunningImage::Elsewhere;
    };
    if !canonical.starts_with(&root) {
        return RunningImage::Elsewhere;
    }

    // Named for the process, so two uninstalls cannot collide, and left for the
    // system to sweep. Nothing can delete its own running image, so a residue
    // somewhere is unavoidable; `%TEMP%` is where it does no harm and where
    // Windows already cleans up after everyone else.
    let destination = temporary.join(format!("speakeasy-uninstall-{}.exe", std::process::id()));
    match std::fs::rename(&canonical, &destination) {
        Ok(()) => RunningImage::Relocated,
        // Reachable when `%TEMP%` is redirected to another volume: a rename
        // across volumes is not a rename, and Windows refuses it rather than
        // silently copying.
        Err(error) => RunningImage::Retained {
            canonical,
            reason: error.to_string(),
        },
    }
}

/// The one directory the install shares with anything staged by hand.
const PROOF: &str = "proof";

/// What this installer places inside `proof/`.
///
/// **No longer a spare list.** Until 2026-08-21 this was the *only* thing
/// removed from `proof/`, and everything else was deliberately left, on the
/// argument that an unrecognised file here was more likely to be 500 MB of
/// fetched CUDA runtime than anything of ours — a few megabytes orphaned
/// against a 2.97 GB re-download. That argument was already dead: the fetch it
/// protected left with the streaming engine, and nothing in this fork writes a
/// runtime into the install root at all. What the rule actually spared, forever
/// and silently, was `scripts/Enable-GraniteCuda.ps1`'s staged libraries.
///
/// So the list is now a *classification* rather than a filter: these are removed
/// as "program files", and whatever else is in `proof/` is removed too and
/// reported by name in [`Outcome::removed_unrecognised`]. Keeping the
/// distinction is what lets an uninstall say which of the two it did.
///
/// Pinned against `tauri.proof.conf.json`'s `bundle.resources` by
/// `apps/desktop/tests/scaffold.test.mjs`. That pin is worth *more* now, not
/// less: a payload file missing from here is no longer orphaned, it is reported
/// to the user as a file the installer did not recognise — which is a
/// confusing thing to say about a file the installer shipped.
///
/// One entry, because this product installs one engine. The streaming
/// worker and the five native libraries it linked — `inference-worker.exe`,
/// both ONNX Runtime DLLs, both sherpa APIs and `cargs.dll` — were listed
/// here until the fork removed the engine. They are *not* moved to
/// `KNOWN_PROOF_ORPHANS`: an orphan is a file some earlier build of this
/// product left behind, and no build of `ai.speakeasy.mini` ever staged one
/// of these. Naming them there would have the uninstaller hunt for files only
/// a *different* product's install root can hold.
const INSTALLED_PROOF_FILES: &[&str] = &["granite-worker.exe"];

/// Files a previous layout left in `proof/` that nothing places any more.
///
/// Named by hand, which is what the NSIS hooks did with the same problem, and
/// for the same reason: `copy_tree` merges rather than replaces — deliberately,
/// so the fetched runtime survives an upgrade — so an orphan is invisible to
/// every rule above and survives forever until something names it.
///
/// `granite-worker.cpu.exe` was left by `scripts/Enable-GraniteCuda.ps1`, which
/// renamed the CPU worker aside before staging a CUDA one.
///
/// **That script was retired on 2026-08-26 and this entry deliberately was
/// not**, against its own earlier note saying the two would go together. That
/// note assumed no machine would still be carrying the file, and that is false
/// for every machine the script ever ran on — the file is still in `proof/`
/// there, and `copy_tree` merges, so no upgrade removes it. Dropping the entry
/// would not leave the file behind; the second pass takes it either way. It
/// would move it into [`Outcome::removed_unrecognised`], which puts a question
/// in front of the user about a file this project's own tooling created. Naming
/// it here is the cheaper truth, and it costs one line until nobody has it.
const KNOWN_PROOF_ORPHANS: &[&str] = &["granite-worker.cpu.exe"];

/// What setup *stages* into `proof/` rather than ships there.
///
/// The CUDA redistributables. They are program files by every meaning that
/// matters here — this installer downloaded them, verified them against the
/// catalog's digests and copied them in — but they cannot go in
/// [`INSTALLED_PROOF_FILES`], which is pinned against the payload manifest's
/// `bundle.resources` and correctly holds only what the payload carries.
/// Without this list an uninstall reports the libraries setup itself placed
/// under [`Outcome::removed_unrecognised`], which is the exact confusing thing
/// that constant's own comment warns against.
///
/// Read from the manifest, never written out. The names are the same ones
/// [`speakeasy_models::inspect_gpu_payload`] checks for and the same ones
/// `download::stage_graphics_card_payload` copies, so all three cannot disagree
/// — a second hand-written list of DLL names is how `cudart64_12` and
/// `cudart64_13` came to name one requirement in this workspace.
///
/// It answers non-empty **today**, before any worker is published, because the
/// redistributables are pinned already. One consequence, small and deliberate:
/// libraries staged by `scripts/Enable-GraniteCuda.ps1` are now removed as
/// program files rather than named as unrecognised. They are still removed,
/// which is what the 2026-08-21 work was for, and once setup stages them itself
/// "unrecognised" would simply be false.
///
/// Empty when the catalog will not parse, which leaves the second pass to catch
/// the files by their own rule rather than skipping them.
fn staged_proof_files() -> Vec<String> {
    speakeasy_models::bundled_manifest()
        .map(|manifest| speakeasy_models::required_cuda_runtime_files(&manifest))
        .unwrap_or_default()
}

fn clear_registration(outcome: &mut Outcome) {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let user = RegKey::predef(HKEY_CURRENT_USER);
    for (key, label) in [
        (crate::install::VERSION_KEY, "version record"),
        (crate::install::UNINSTALL_KEY, "Add/Remove Programs entry"),
    ] {
        match user.delete_subkey_all(key) {
            Ok(()) => outcome.removed.push(label.to_owned()),
            // Absent is success: an uninstall that reports failure because the
            // thing was already gone teaches users to ignore its output.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => outcome.failed.push(format!("{label}: {error}")),
        }
    }
    remove_key_if_empty(&user, crate::install::VERSION_KEY_ROOT);
}

/// Remove a registry key only if nothing is under it.
///
/// The counterpart of [`remove_directory_if_empty`], and it exists for the same
/// reason: the version stamp lives at `Software\SpeakEasy Mini\LocalDevelopment`,
/// so deleting it leaves an empty `Software\SpeakEasy Mini` behind with the
/// product's name on it. Found on 2026-08-21, in the first uninstall ever run
/// against a real installation rather than a staged root -- everything else was
/// gone and that was still there.
///
/// Silent about failure, again like its filesystem counterpart: a key that still
/// holds something is not a fault, and a key this cannot open is one it has no
/// business deleting.
fn remove_key_if_empty(user: &winreg::RegKey, path: &str) {
    use winreg::enums::{KEY_READ, KEY_WRITE};

    let Ok(key) = user.open_subkey_with_flags(path, KEY_READ) else {
        return;
    };
    if key.enum_keys().next().is_some() || key.enum_values().next().is_some() {
        return;
    }
    // Dropped before the delete: an open handle on the key is exactly what makes
    // `delete_subkey` fail, and this function holds one.
    drop(key);
    let _ = user
        .open_subkey_with_flags("", KEY_WRITE)
        .and_then(|root| root.delete_subkey(path));
}

fn remove_shortcuts(outcome: &mut Outcome) {
    let Some(folder) = crate::shortcut::start_menu_folder() else {
        return;
    };
    if !folder.exists() {
        return;
    }
    match std::fs::remove_dir_all(&folder) {
        Ok(()) => outcome.removed.push("Start Menu shortcuts".to_owned()),
        Err(error) => outcome
            .failed
            .push(format!("Start Menu shortcuts: {error}")),
    }
}

/// Remove the program, and everything else under the install root.
///
/// Still a walk rather than one `remove_dir_all`, and the reason is now a single
/// one: this program's own executable may be sitting among them, and one
/// undeletable file must not fail the removal of everything beside it. The
/// second reason — sparing a fetched runtime — is gone with the fetch.
///
/// The directory itself goes last and only if it is empty, so it survives
/// exactly when something in it could not be removed, which is the one case
/// where its survival is information.
fn remove_program_files(install_root: &Path, image: &RunningImage, outcome: &mut Outcome) {
    if !install_root.exists() {
        return;
    }
    let retained = match image {
        RunningImage::Retained { canonical, .. } => Some(canonical.as_path()),
        RunningImage::Elsewhere | RunningImage::Relocated => None,
    };

    let Ok(entries) = std::fs::read_dir(install_root) else {
        outcome
            .failed
            .push("program files: could not be listed".to_owned());
        return;
    };
    let mut removed_anything = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.eq_ignore_ascii_case(PROOF) {
            // Still emptied entry by entry rather than removed whole, but no
            // longer selectively: the point is now to *name* what was in there
            // that this installer did not put there, which a `remove_dir_all`
            // cannot do.
            removed_anything |= empty_proof_directory(&entry.path(), outcome);
            continue;
        }
        let path = entry.path();
        // By canonical path rather than by name: the running image has already
        // been established to be somewhere under this root, and skipping a
        // top-level entry that merely shares its name would silently spare the
        // wrong file.
        if retained.is_some_and(|kept| path.canonicalize().is_ok_and(|entry| entry == kept)) {
            continue;
        }
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed_anything = true,
            Err(error) => outcome
                .failed
                .push(format!("{}: {error}", entry.file_name().to_string_lossy())),
        }
    }
    // Claimed only when something actually went, and this is the difference
    // between honest output and the reverse. Reported unconditionally, it said
    // "Removed: program files" for a directory it had not touched a single file
    // in — which is how an uninstall that was pointed at the wrong directory
    // read as a complete success (measured 2026-08-15).
    if removed_anything {
        outcome.removed.push("program files".to_owned());
    }
    // `KEPT_WITH_GPU_RUNTIME` was reported here, for a `proof/` that survived
    // because the fetched runtime inside it was spared. Nothing is spared here
    // now, so a surviving `proof/` means something could not be removed -- which
    // is already in `outcome.failed`, said once, by whichever entry failed.
    if let RunningImage::Retained { canonical, reason } = image {
        outcome
            .left_behind
            .push(format!("{} ({reason})", canonical.display()));
    }
    // Only ever succeeds on an empty directory, which is the intended test: a
    // spared runtime or a retained executable is exactly the case where the
    // folder has to stay.
    let _ = std::fs::remove_dir(install_root);
}

/// Empty `proof/` completely, naming what was in there that we did not install.
///
/// Returns whether anything went, so the caller can avoid claiming a removal it
/// did not perform.
///
/// Two passes, and the order is the whole design. The declared names go first and
/// count as program files; whatever survives that pass is, by definition, not
/// ours, and goes second into [`Outcome::removed_unrecognised`] where the caller
/// can say so. One `remove_dir_all` would leave the same empty directory and be
/// unable to tell anyone which of the two it had just deleted 493 MB of.
///
/// The directory itself is removed when the last file leaves it, so an ordinary
/// uninstall ends with no `proof/` at all rather than an empty one.
fn empty_proof_directory(proof: &Path, outcome: &mut Outcome) -> bool {
    if !proof.is_dir() {
        return false;
    }
    let mut removed_anything = false;
    let staged = staged_proof_files();
    let ours = INSTALLED_PROOF_FILES
        .iter()
        .copied()
        .chain(KNOWN_PROOF_ORPHANS.iter().copied())
        .chain(staged.iter().map(String::as_str));
    for name in ours {
        let path = proof.join(name);
        if !path.exists() {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed_anything = true,
            Err(error) => outcome.failed.push(format!("{PROOF}/{name}: {error}")),
        }
    }
    // Whatever is left. Directories too: nothing this installer ships puts one
    // here, so a directory in `proof/` is exactly as unrecognised as a file and
    // leaving it would defeat the point.
    if let Ok(entries) = std::fs::read_dir(proof) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    removed_anything = true;
                    outcome.removed_unrecognised.push(name);
                }
                Err(error) => outcome.failed.push(format!("{PROOF}/{name}: {error}")),
            }
        }
    }
    // Succeeds only when nothing was left, which is exactly the condition for
    // wanting it gone.
    let _ = std::fs::remove_dir(proof);
    removed_anything
}

/// What is in `proof/` that this installer did not put there, without removing it.
///
/// Exists so the confirmation can *name* the files before anything is deleted.
/// Asking afterwards would be a report; asking first is a prompt, and the owner's
/// requirement is that unknown files are confirmed rather than announced.
///
/// Sorted, because this is read aloud to a person and the order a directory
/// listing happens to come back in is not a reason to show one file before
/// another.
#[must_use]
pub fn unrecognised_proof_files(install_root: &Path) -> Vec<String> {
    let proof = install_root.join(PROOF);
    let Ok(entries) = std::fs::read_dir(&proof) else {
        return Vec::new();
    };
    let staged = staged_proof_files();
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            !INSTALLED_PROOF_FILES
                .iter()
                .copied()
                .chain(KNOWN_PROOF_ORPHANS.iter().copied())
                .chain(staged.iter().map(String::as_str))
                .any(|known| name.eq_ignore_ascii_case(known))
        })
        .collect();
    names.sort();
    names
}

fn remove_user_data(removals: Removals, outcome: &mut Outcome) {
    let Some(root) = data_root() else {
        return;
    };
    remove_user_data_under(&root, removals, outcome);
}

/// [`remove_user_data`] against a given profile root.
///
/// Split out so the "nothing is left" half is testable. It reads `%APPDATA%`,
/// which a test cannot redirect without changing it for every other test in the
/// process — and the behaviour worth pinning here is precisely the one that only
/// shows up on a real tree: that the directories themselves go, not just the
/// files inside them.
/// What each removable item is, under the profile root.
///
/// Extracted from [`remove_user_data_under`] on 2026-08-21 so that [`measure`]
/// reads the same table the deletion does. A page that showed a size derived
/// from a second copy of these paths could name a figure for one set of files
/// and delete another, which is the shape of defect this repository keeps
/// finding one layer up from where it was introduced.
///
/// Paths mirror what the NSIS uninstall hook removed, so an uninstall after an
/// upgrade from a pre-bootstrapper install still finds everything.
const fn targets() -> [(Removable, &'static str, &'static [&'static str]); 5] {
    [
        (Removable::Configuration, "configuration", &["config"]),
        (
            Removable::History,
            "transcript history",
            &[
                "data/speakeasy.sqlite3",
                "data/speakeasy.sqlite3-wal",
                "data/speakeasy.sqlite3-shm",
            ],
        ),
        (Removable::Models, "installed models", &["model-lifecycle"]),
        (Removable::Recovery, "recovery backups", &["recovery"]),
        // The whole directory, not the two files by name. `speakeasy.log` and
        // its one rotated generation are what rotation writes today, and a log
        // name that changes must not start surviving uninstalls silently.
        (Removable::Logs, "diagnostic logs", &["logs"]),
    ]
}

/// What one removable item currently occupies, in bytes.
///
/// `None` when there is no profile root, or nothing of this item on disk —
/// which is a different statement from zero and must not be rendered as
/// "0 bytes" beside a checkbox.
///
/// Exists for the uninstall page's models checkbox. That one item is the only
/// large, invisible cost in the list, and the page names its figure so the user
/// can see what they are agreeing to; the other four are kilobytes and a size
/// beside each of them buries the one that matters. **Measured rather than
/// written down** — the label this replaces was inherited from an item that
/// claimed "about 2.3 GB" for a download this fork never had.
pub fn measure(item: Removable) -> Option<u64> {
    let root = data_root()?;
    let (_, _, relatives) = targets().into_iter().find(|(kind, _, _)| *kind == item)?;
    let mut total = 0_u64;
    let mut found = false;
    for relative in relatives {
        let path = root.join(relative);
        if path.exists() {
            found = true;
            total = total.saturating_add(size_on_disk(&path));
        }
    }
    found.then_some(total)
}

/// Bytes under a path, following directories and ignoring what cannot be read.
///
/// A file it cannot stat contributes nothing rather than aborting the walk: this
/// figure exists to tell a user roughly what they are about to free, and
/// refusing to show any number because one file was locked would be worse than
/// showing a slightly low one. It is never used to decide anything.
fn size_on_disk(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        // A symlink or reparse point. Not followed: its target is somewhere
        // else, and counting it here would attribute another directory's bytes
        // to this checkbox.
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| size_on_disk(&entry.path()))
        .fold(0, u64::saturating_add)
}

fn remove_user_data_under(root: &Path, removals: Removals, outcome: &mut Outcome) {
    for (item, label, relatives) in targets() {
        if !removals.includes(item) {
            outcome.kept.push(label.to_owned());
            continue;
        }
        let mut failed = false;
        for relative in relatives {
            let path = root.join(relative);
            if !path.exists() {
                continue;
            }
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(error) = result {
                outcome.failed.push(format!("{label}: {error}"));
                failed = true;
            }
        }
        if !failed {
            outcome.removed.push(label.to_owned());
        }
    }
    // Last, and only when empty. This is the difference between "we deleted the
    // things we know about" and "nothing is left", which is what the user asked
    // for -- and it fails harmlessly on a keep-user-data run, where the
    // directory still holds what was kept. `data/` survives on a history
    // removal too: the database goes and the directory it sat in does not,
    // because something else may put a file there later and guessing is how a
    // future subsystem's data gets deleted by an unrelated checkbox.
    remove_directory_if_empty(&root.join("data"));
    remove_directory_if_empty(root);
}

/// Remove a directory only if nothing is in it.
///
/// Deliberately silent about failure: a directory that still holds something is
/// the ordinary outcome of a keep-user-data uninstall, not a fault, and
/// reporting it would put a scary line in front of a user who asked to keep
/// exactly that.
fn remove_directory_if_empty(directory: &Path) {
    if directory
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_none())
    {
        let _ = std::fs::remove_dir(directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_default_removes_nothing_and_everything_removes_all_of_it() {
        // The *API* default has to be keep, and still is. A caller that forgot to
        // ask must delete nothing, because this is the one place in the product
        // where the wrong default cannot be undone.
        //
        // The user-facing default inverted on 2026-08-21 and this test did not,
        // deliberately: the inversion belongs at the command line, where the
        // question has actually been put to somebody. `main::removals_for`
        // builds `everything()` unless `--keep-user-data` says otherwise, and
        // `the_command_line_removes_everything_unless_told_to_keep_user_data`
        // pins that end. It said so before that test existed, which is why the
        // half that actually deletes a user's weights was the unguarded half.
        let removals = Removals::default();
        for item in Removable::ALL {
            assert!(
                !removals.includes(item),
                "{} must default to keep",
                item.label()
            );
        }

        // And the other end of it: `everything()` has to actually mean all of
        // them, or an uninstall promising a clean machine would quietly spare
        // whichever slot was added last.
        let all = Removals::everything();
        for item in Removable::ALL {
            assert!(
                all.includes(item),
                "{} must be removed by everything()",
                item.label()
            );
        }
    }

    #[test]
    fn every_removable_has_a_distinct_slot_and_a_label() {
        // The index mapping is hand-written, so two entries sharing a slot would
        // silently make one checkbox control the other — a user unticking
        // "transcript history" would then delete it anyway.
        let mut removals = Removals::default();
        for item in Removable::ALL {
            removals.select(item, true);
            assert!(removals.includes(item));
            removals.select(item, false);
            assert!(!removals.includes(item));
            assert!(!item.label().is_empty());
        }
        for item in Removable::ALL {
            assert!(!removals.includes(item), "selection leaked between items");
        }
    }

    #[test]
    fn nothing_survives_proof_and_what_we_did_not_place_there_is_named() {
        // Two defects, in opposite directions, one after the other.
        //
        // First `proof/` was spared **whole**, so every worker and speech DLL
        // this installer placed survived an uninstall that reported the program
        // removed -- nine app-owned files among twenty-six, measured on a real
        // install 2026-08-15. That was fixed by removing only our own names.
        //
        // Which produced the second: everything else was then spared forever, on
        // the argument that an unknown file here was a fetched CUDA runtime worth
        // 2.97 GB. This fork has no such fetch. What the rule actually preserved
        // was `Enable-GraniteCuda.ps1`'s staged libraries -- 493 MB, measured
        // 2026-08-21 -- through every uninstall, on a machine the user believed
        // was clean.
        //
        // So: everything goes, and what was not ours is *named* rather than
        // silently deleted, which is what lets the confirmation ask about it.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-proof-split-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let proof = root.join("proof");
        std::fs::create_dir_all(&proof).expect("proof");
        for ours in INSTALLED_PROOF_FILES {
            std::fs::write(proof.join(ours), b"ours").expect("installed file");
        }
        std::fs::write(proof.join("granite-worker.cpu.exe"), b"orphan").expect("orphan");
        // **The CUDA libraries changed sides on 2026-08-26.** They used to be
        // the canonical unrecognised file here, because only
        // `Enable-GraniteCuda.ps1` ever put one in `proof/`. Setup stages them
        // itself now, from the digests in its own catalog, so calling them files
        // "this installer did not put there" would be false — and it would put a
        // question in front of every graphics-card user about the two libraries
        // their installation cannot run without.
        // Never empty: the redistributables are pinned already, worker or no
        // worker, so this list has entries on today's catalog.
        let staged = staged_proof_files();
        assert!(!staged.is_empty(), "no staged proof files");
        for name in &staged {
            std::fs::write(proof.join(name), b"staged").expect("staged library");
        }
        // Something genuinely foreign, so the second pass is still exercised
        // rather than merely reached — and a directory, because nothing this
        // installer puts in `proof/` is one and leaving it would defeat the point
        // just as surely as leaving a file.
        std::fs::write(proof.join("someone-elses.dll"), b"theirs").expect("foreign file");
        std::fs::create_dir_all(proof.join("leftover-dir")).expect("leftover directory");

        // Asked before the removal, because that is when the confirmation needs
        // it. Nothing this installer places or stages may appear; everything
        // else must.
        let mut unrecognised = unrecognised_proof_files(&root);
        unrecognised.sort();
        let mut expected = vec!["leftover-dir".to_owned(), "someone-elses.dll".to_owned()];
        expected.sort();
        assert_eq!(unrecognised, expected);

        let mut outcome = Outcome::default();
        remove_program_files(&root, &RunningImage::Elsewhere, &mut outcome);

        assert!(!root.exists(), "the install root must not survive");
        outcome.removed_unrecognised.sort();
        assert_eq!(
            outcome.removed_unrecognised, expected,
            "everything not ours must be removed and reported by name"
        );
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(
            !outcome
                .kept
                .iter()
                .any(|kept| kept.contains("graphics-card")),
            "nothing in the install directory is kept any more: {:?}",
            outcome.kept
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_install_with_nothing_staged_leaves_no_proof_directory_and_names_nothing() {
        // The ordinary case: only our own files, so `removed_unrecognised` has to
        // be empty. A list that named something here would put a line in front of
        // every user asking about files that were always the installer's own.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-proof-only-ours-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let proof = root.join("proof");
        std::fs::create_dir_all(&proof).expect("proof");
        for ours in INSTALLED_PROOF_FILES {
            std::fs::write(proof.join(ours), b"ours").expect("installed file");
        }
        std::fs::write(root.join("ai-speakeasy-mini.exe"), b"app").expect("app");

        assert!(unrecognised_proof_files(&root).is_empty());

        let mut outcome = Outcome::default();
        remove_program_files(&root, &RunningImage::Elsewhere, &mut outcome);

        assert!(!root.exists(), "nothing was kept, so nothing may be left");
        assert!(
            outcome.removed_unrecognised.is_empty(),
            "{:?}",
            outcome.removed_unrecognised
        );
    }

    /// An empty key with the product's name on it is still a residue.
    ///
    /// The counterpart of the empty-directory rule, and it exists because the
    /// first uninstall run against a real installation rather than a staged root
    /// left exactly this: `Software\SpeakEasy Mini`, no values, no subkeys, after
    /// everything else on the machine was gone. Deleting `VERSION_KEY` takes
    /// `LocalDevelopment` and stops there.
    ///
    /// Its own key rather than the product's, so a failing test cannot damage a
    /// real installation, and so this can be run on a machine that has one.
    #[test]
    fn an_emptied_product_key_is_removed_and_an_occupied_one_is_not() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;

        // Removed even if an assertion below panics. The previous version
        // cleaned up on the success path only, so a failing run left a key
        // behind under the product's own name in the developer's registry.
        struct RemoveOnDrop<'a>(&'a str);
        impl Drop for RemoveOnDrop<'_> {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(self.0);
            }
        }

        let user = RegKey::predef(HKEY_CURRENT_USER);
        // Unique per process, because `HKCU` is shared by every `cargo test`
        // running as this user and a fixed name made two of them fight.
        let path = format!(
            r"Software\SpeakEasy Mini Uninstall Test {}",
            std::process::id()
        );
        let path = path.as_str();
        let _cleanup = RemoveOnDrop(path);

        user.create_subkey(path).expect("create the empty key");
        remove_key_if_empty(&user, path);
        assert!(
            user.open_subkey(path).is_err(),
            "an empty key must not survive"
        );

        // Occupied, and therefore not this uninstaller's to guess about.
        user.create_subkey(format!(r"{path}\Something"))
            .expect("create an occupant");
        remove_key_if_empty(&user, path);
        assert!(
            user.open_subkey(path).is_ok(),
            "a key holding something must survive"
        );
        user.delete_subkey_all(path).expect("clean up");
    }

    #[test]
    fn the_install_directory_itself_is_removed_when_nothing_is_kept() {
        // The directory used to survive every uninstall, because the running
        // uninstaller was standing in it. An install root left behind is not
        // cosmetic: `CLAUDE.md` records that "uninstall, install" has to be a
        // real clean-machine test, and a folder that outlives its uninstall is
        // where the next install's orphans come from.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-directory-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("proof");
        std::fs::write(root.join("proof").join("worker.exe"), b"worker").expect("worker");
        std::fs::write(root.join("speakeasy-bootstrapper.exe"), b"setup").expect("bootstrapper");

        let mut outcome = Outcome::default();
        remove_program_files(&root, &RunningImage::Relocated, &mut outcome);

        assert!(!root.exists(), "the install directory must not survive");
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(outcome.left_behind.is_empty());
    }

    #[test]
    fn a_retained_running_image_is_spared_and_reported_without_failing_the_uninstall() {
        // The fallback path, for a machine where the running executable could not
        // be moved out of the way. Everything else must still go, and the
        // leftover must not land in `failed`: `main::remove` turns a non-empty
        // `failed` into a non-zero exit, and a product that is genuinely removed
        // apart from two megabytes of its own uninstaller has not failed.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-retained-image-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        let image = root.join("speakeasy-bootstrapper.exe");
        std::fs::write(&image, b"setup").expect("bootstrapper");
        std::fs::write(root.join("ai-speakeasy-mini.exe"), b"app").expect("app");
        let canonical = image.canonicalize().expect("canonical image");

        let mut outcome = Outcome::default();
        remove_program_files(
            &root,
            &RunningImage::Retained {
                canonical,
                reason: "the file could not be moved".to_owned(),
            },
            &mut outcome,
        );

        assert!(image.is_file(), "the running image must be spared");
        assert!(!root.join("ai-speakeasy-mini.exe").exists());
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.left_behind.len(), 1, "{:?}", outcome.left_behind);
        assert!(
            outcome.left_behind[0].contains("the file could not be moved"),
            "the reason has to reach the user: {:?}",
            outcome.left_behind
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_nothing_is_never_reported_as_removing_the_program() {
        // The exact shape of the 2026-08-15 defect: an install root that exists
        // and yields nothing. It reported "Removed: program files" having deleted
        // nothing, which is how an uninstall aimed at the wrong directory read as
        // a complete success with exit code zero.
        //
        // The scenario used to be "a root holding only spared items", because a
        // spared CUDA runtime was the realistic way to reach it. Nothing is
        // spared any more, so an **empty** root is what is left -- and it is
        // still the realistic case, since it is exactly what pointing an
        // uninstall at the wrong directory produces.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-nothing-to-do-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("install root");

        let mut outcome = Outcome::default();
        remove_program_files(&root, &RunningImage::Elsewhere, &mut outcome);

        assert!(
            !outcome.removed.iter().any(|item| item == "program files"),
            "nothing was deleted, so nothing may be claimed: {:?}",
            outcome.removed
        );
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(
            crate::catalog::describe_uninstall(&outcome)
                .contains("does not appear to be installed"),
            "the message must not announce a removal that did not happen"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_user_data_leaves_no_profile_directory_and_keeping_it_leaves_all_of_it() {
        // The point of the 2026-08-21 inversion, stated as an assertion: after an
        // ordinary uninstall there is nothing left. Both halves are here because
        // each is a way to get this wrong -- removing everything but the empty
        // directories that held it reports a clean machine and leaves a tree, and
        // a `--keep-user-data` run that removed anything would delete the weights
        // it exists to preserve.
        let root = std::env::temp_dir().join(format!(
            "speakeasy-uninstall-profile-{}",
            std::process::id()
        ));
        let stage = || {
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("config")).expect("config");
            std::fs::create_dir_all(root.join("data")).expect("data");
            std::fs::create_dir_all(root.join("model-lifecycle/models")).expect("models");
            std::fs::create_dir_all(root.join("recovery")).expect("recovery");
            std::fs::create_dir_all(root.join("logs")).expect("logs");
            std::fs::write(root.join("config/install-provider.txt"), b"cpu").expect("marker");
            std::fs::write(root.join("data/speakeasy.sqlite3"), b"db").expect("database");
            std::fs::write(root.join("model-lifecycle/models/weights.gguf"), b"w")
                .expect("weights");
            std::fs::write(root.join("recovery/backup.json"), b"{}").expect("backup");
            std::fs::write(root.join("logs/speakeasy.log"), b"line").expect("log");
        };

        stage();
        let mut outcome = Outcome::default();
        remove_user_data_under(&root, Removals::everything(), &mut outcome);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        // The whole point of the inversion: not "the files are gone" but
        // "the tree is gone". An empty profile directory left behind is what the
        // previous behaviour produced and it reads as clean.
        assert!(!root.exists(), "no profile directory may survive");

        stage();
        let mut kept = Outcome::default();
        remove_user_data_under(&root, Removals::default(), &mut kept);
        assert!(kept.failed.is_empty(), "{:?}", kept.failed);
        for relative in [
            "config/install-provider.txt",
            "data/speakeasy.sqlite3",
            "model-lifecycle/models/weights.gguf",
            "recovery/backup.json",
            "logs/speakeasy.log",
        ] {
            assert!(
                root.join(relative).is_file(),
                "--keep-user-data must keep {relative}"
            );
        }
        assert!(kept.removed.is_empty(), "{:?}", kept.removed);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_absent_installation_is_not_a_failure() {
        // Uninstalling something already gone is the ordinary result of running
        // it twice, and reporting failure there teaches users to ignore output
        // that matters the one time it does not.
        let absent =
            std::env::temp_dir().join(format!("speakeasy-uninstall-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&absent);

        let mut outcome = Outcome::default();
        remove_program_files(&absent, &RunningImage::Elsewhere, &mut outcome);

        assert!(outcome.failed.is_empty());
        assert!(outcome.removed.is_empty());
    }
}
