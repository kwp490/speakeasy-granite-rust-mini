//! Removing an installation, and choosing how much of it to remove.
//!
//! NSIS asked four keep-or-remove questions as separate modal dialogs, plus a
//! fifth page for the downloaded CUDA runtime. Owner decision (2026-08-15): one
//! page, all five as checkboxes, every one defaulting to **keep** — which is
//! what `/SD IDYES` meant in the silent path, so the default behaviour is
//! unchanged. Seeing the whole scope of a deletion before confirming any of it
//! is the point; four sequential prompts answered blind is how someone removes
//! their transcript history without noticing.
//!
//! Program files are never optional. Everything below is user data, and the
//! distinction is the contract: uninstalling removes the program, and removes
//! user data only where asked.

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
    /// The ~2.3 GB of CUDA runtime fetched on demand into `proof/`.
    ///
    /// Separate from the program files even though it lives among them: it is
    /// expensive to re-fetch and nothing else replaces it, which is why NSIS
    /// left it behind by default and why `CLAUDE.md` records that "uninstall,
    /// install" is not a clean-machine test on any machine that ever fetched it.
    GpuRuntime,
}

impl Removable {
    pub const ALL: [Self; 5] = [
        Self::Configuration,
        Self::History,
        Self::Models,
        Self::Recovery,
        Self::GpuRuntime,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Configuration => 0,
            Self::History => 1,
            Self::Models => 2,
            Self::Recovery => 3,
            Self::GpuRuntime => 4,
        }
    }

    /// What the checkbox says.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "Settings and personalization",
            Self::History => "Transcript history",
            Self::Models => "Downloaded speech models",
            Self::Recovery => "Recovery backups",
            Self::GpuRuntime => "Downloaded graphics-card runtime (about 2.3 GB)",
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
    /// Exists for `--remove-all`, which is what leaves a genuinely clean machine
    /// — the state `CLAUDE.md` says is required to test first-run honestly,
    /// since an ordinary uninstall spares ~2.3 GB of runtime and makes the next
    /// setup look faster and simpler than it is for a real new user.
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
    remove_program_files(
        install_root,
        removals.includes(Removable::GpuRuntime),
        &vacate(install_root),
        &mut outcome,
    );
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

/// Directories under the install root that an ordinary uninstall spares whole.
///
/// The two working directories of the on-demand runtime download. They hold
/// part-fetched archives and their resume metadata, which is precisely what
/// makes an interrupted 2.97 GB download resumable rather than repeated.
///
/// `proof/` is *not* here, because it cannot be spared whole — see
/// [`INSTALLED_PROOF_FILES`].
const SPARED_WHOLE_WHEN_SPARING_RUNTIME: &[&str] =
    &[".cuda-runtime-download", ".cuda-runtime-stage"];

/// The one directory the install shares with the downloaded runtime.
const PROOF: &str = "proof";

/// What this installer places inside `proof/`, and therefore what it removes.
///
/// **The direction is inverted here, and only here.** Everywhere else an
/// uninstall removes what it does not recognise, so that a file added later
/// without updating a list is cleaned up rather than orphaned. Inside `proof/`
/// that rule is actively dangerous, because this is the one directory where an
/// unrecognised file is more likely to be 500 MB of fetched CUDA runtime than
/// anything of ours. The two mistakes are not the same size: leaving an unknown
/// file behind costs a few megabytes and is corrected by the next install, while
/// deleting one costs a 2.97 GB download over the user's connection.
///
/// Measured on this machine 2026-08-17, and the reason the rule is written this
/// way rather than as "spare `REQUIRED_RUNTIME_FILES`": `proof/` held three CUDA
/// **13** redistributables — 516 MB, staged by the interim
/// `scripts/Enable-GraniteCuda.ps1` — which appear in no list in this workspace
/// yet, because the CUDA Granite worker that needs them is unpublished. Sparing
/// by the known-fetched list would have deleted all three.
///
/// Pinned against `tauri.proof.conf.json`'s `bundle.resources` by
/// `apps/desktop/tests/scaffold.test.mjs`, so a payload file added without a
/// line here fails the gate instead of surviving every uninstall.
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
/// `granite-worker.cpu.exe` is left by `scripts/Enable-GraniteCuda.ps1`, which
/// renames the CPU worker aside before staging a CUDA one. That script is
/// labelled interim and retires when setup fetches a published worker; this
/// entry retires with it.
const KNOWN_PROOF_ORPHANS: &[&str] = &["granite-worker.cpu.exe"];

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

/// Remove the program, optionally sparing the downloaded GPU runtime.
///
/// Always a selective walk, never a `remove_dir_all` of the whole root, and the
/// two reasons are unrelated: `proof/` holds both app-owned files and ~2.3 GB of
/// fetched runtime, and only the fetched ones are expensive to replace; and this
/// program's own executable may be sitting among them, in which case one
/// undeletable file would fail the removal of everything else beside it.
///
/// The directory itself goes last and only if it is empty, so it survives
/// exactly when something in it was deliberately kept.
fn remove_program_files(
    install_root: &Path,
    remove_gpu_runtime: bool,
    image: &RunningImage,
    outcome: &mut Outcome,
) {
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
        if !remove_gpu_runtime {
            if name.eq_ignore_ascii_case(PROOF) {
                // The only directory emptied selectively rather than kept or
                // removed whole: our workers and the fetched runtime live side
                // by side in it, and they have opposite costs to get wrong.
                removed_anything |= remove_installed_files_from_proof(&entry.path(), outcome);
                continue;
            }
            if SPARED_WHOLE_WHEN_SPARING_RUNTIME
                .iter()
                .any(|keep| name.eq_ignore_ascii_case(keep))
            {
                continue;
            }
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
    // Only when there is something there to keep. Reported unconditionally, this
    // claimed to have kept a runtime on machines that never fetched one.
    if !remove_gpu_runtime && install_root.join("proof").exists() {
        outcome
            .kept
            .push(crate::catalog::KEPT_WITH_GPU_RUNTIME.to_owned());
    }
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

/// Take this installer's own files out of `proof/`, and leave everything else.
///
/// Returns whether anything went, so the caller can avoid claiming a removal it
/// did not perform.
///
/// Removes by name and never by walking, which is the whole point: see
/// [`INSTALLED_PROOF_FILES`] for why an unrecognised file in this directory is
/// spared rather than cleaned up. The directory itself is removed when the last
/// file leaves it, so a machine that never fetched the runtime — the ordinary
/// case — ends an uninstall with no `proof/` at all rather than an empty one.
fn remove_installed_files_from_proof(proof: &Path, outcome: &mut Outcome) -> bool {
    if !proof.is_dir() {
        return false;
    }
    let mut removed_anything = false;
    for name in INSTALLED_PROOF_FILES.iter().chain(KNOWN_PROOF_ORPHANS) {
        let path = proof.join(name);
        if !path.exists() {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed_anything = true,
            Err(error) => outcome.failed.push(format!("{PROOF}/{name}: {error}")),
        }
    }
    // Succeeds only when nothing was left, which is exactly the condition for
    // wanting it gone.
    let _ = std::fs::remove_dir(proof);
    removed_anything
}

fn remove_user_data(removals: Removals, outcome: &mut Outcome) {
    let Some(root) = data_root() else {
        return;
    };
    // Paths mirror what the NSIS uninstall hook removed, so an uninstall after
    // an upgrade from a pre-bootstrapper install still finds everything.
    let targets: [(Removable, &str, &[&str]); 4] = [
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
    ];
    for (item, label, relatives) in targets {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_optional_is_removed_by_default() {
        // The default has to be keep. An uninstall that deletes user data
        // because a caller forgot to ask is unrecoverable, and this is the one
        // place in the product where the wrong default cannot be undone.
        let removals = Removals::default();

        for item in Removable::ALL {
            assert!(
                !removals.includes(item),
                "{} must default to keep",
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
    fn sparing_the_runtime_still_takes_this_installer_s_files_out_of_proof() {
        // The defect: `proof/` was spared whole, so every worker and speech DLL
        // this installer placed survived an uninstall that reported the program
        // removed. Nine app-owned files among twenty-six, measured on a real
        // install 2026-08-15.
        let root = std::env::temp_dir().join("speakeasy-uninstall-proof-split");
        let _ = std::fs::remove_dir_all(&root);
        let proof = root.join("proof");
        std::fs::create_dir_all(&proof).expect("proof");
        for ours in INSTALLED_PROOF_FILES {
            std::fs::write(proof.join(ours), b"ours").expect("installed file");
        }
        std::fs::write(proof.join("granite-worker.cpu.exe"), b"orphan").expect("orphan");
        // Fetched, and expensive: the required runtime, plus the CUDA 13 trio
        // that no list in this workspace names yet. Sparing by the known-fetched
        // list rather than by ours would delete the second group.
        for fetched in [
            "cudnn64_9.dll",
            "onnxruntime_providers_cuda.dll",
            "cublas64_13.dll",
            "cublasLt64_13.dll",
            "cudart64_13.dll",
        ] {
            std::fs::write(proof.join(fetched), b"fetched").expect("fetched file");
        }

        let mut outcome = Outcome::default();
        remove_program_files(&root, false, &RunningImage::Elsewhere, &mut outcome);

        for ours in INSTALLED_PROOF_FILES {
            assert!(
                !proof.join(ours).exists(),
                "{ours} is ours to remove and must not survive an uninstall"
            );
        }
        assert!(
            !proof.join("granite-worker.cpu.exe").exists(),
            "named orphan"
        );
        for fetched in [
            "cudnn64_9.dll",
            "onnxruntime_providers_cuda.dll",
            "cublas64_13.dll",
            "cublasLt64_13.dll",
            "cudart64_13.dll",
        ] {
            assert!(
                proof.join(fetched).is_file(),
                "{fetched} was downloaded; deleting it costs 2.97 GB"
            );
        }
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(
            outcome
                .kept
                .iter()
                .any(|kept| kept == "downloaded graphics-card runtime"),
            "{:?}",
            outcome.kept
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_machine_that_never_fetched_the_runtime_keeps_no_proof_directory() {
        // The ordinary case, and the one a "spare proof/ whole" rule got wrong
        // in the other direction: a user who never enabled the graphics card had
        // an empty directory left behind, and an uninstall claiming to have kept
        // a runtime that was never there.
        let root = std::env::temp_dir().join("speakeasy-uninstall-proof-only-ours");
        let _ = std::fs::remove_dir_all(&root);
        let proof = root.join("proof");
        std::fs::create_dir_all(&proof).expect("proof");
        for ours in INSTALLED_PROOF_FILES {
            std::fs::write(proof.join(ours), b"ours").expect("installed file");
        }
        std::fs::write(root.join("ai-speakeasy-mini.exe"), b"app").expect("app");

        let mut outcome = Outcome::default();
        remove_program_files(&root, false, &RunningImage::Elsewhere, &mut outcome);

        assert!(!root.exists(), "nothing was kept, so nothing may be left");
        assert!(
            !outcome
                .kept
                .iter()
                .any(|kept| kept.contains("graphics-card")),
            "a runtime never fetched must not be kept: {:?}",
            outcome.kept
        );
    }

    #[test]
    fn sparing_the_gpu_runtime_keeps_proof_and_removes_the_rest() {
        let root = std::env::temp_dir().join("speakeasy-uninstall-spare-runtime");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("proof");
        std::fs::create_dir_all(root.join("notices")).expect("notices");
        std::fs::write(root.join("proof").join("cudart64_12.dll"), b"big").expect("runtime file");
        std::fs::write(root.join("ai-speakeasy-mini.exe"), b"app").expect("app");

        let mut outcome = Outcome::default();
        remove_program_files(&root, false, &RunningImage::Elsewhere, &mut outcome);

        assert!(
            root.join("proof").join("cudart64_12.dll").is_file(),
            "the expensive runtime must survive an ordinary uninstall"
        );
        assert!(!root.join("ai-speakeasy-mini.exe").exists());
        assert!(!root.join("notices").exists());
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_the_gpu_runtime_takes_the_whole_directory() {
        let root = std::env::temp_dir().join("speakeasy-uninstall-full");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("proof");
        std::fs::write(root.join("proof").join("cudart64_12.dll"), b"big").expect("runtime file");

        let mut outcome = Outcome::default();
        remove_program_files(&root, true, &RunningImage::Elsewhere, &mut outcome);

        assert!(!root.exists());
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    }

    #[test]
    fn the_install_directory_itself_is_removed_when_nothing_is_kept() {
        // The directory used to survive every uninstall, because the running
        // uninstaller was standing in it. An install root left behind is not
        // cosmetic: `CLAUDE.md` records that "uninstall, install" has to be a
        // real clean-machine test, and a folder that outlives its uninstall is
        // where the next install's orphans come from.
        let root = std::env::temp_dir().join("speakeasy-uninstall-directory");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("proof");
        std::fs::write(root.join("proof").join("worker.exe"), b"worker").expect("worker");
        std::fs::write(root.join("speakeasy-bootstrapper.exe"), b"setup").expect("bootstrapper");

        let mut outcome = Outcome::default();
        remove_program_files(&root, true, &RunningImage::Relocated, &mut outcome);

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
        let root = std::env::temp_dir().join("speakeasy-uninstall-retained-image");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        let image = root.join("speakeasy-bootstrapper.exe");
        std::fs::write(&image, b"setup").expect("bootstrapper");
        std::fs::write(root.join("ai-speakeasy-mini.exe"), b"app").expect("app");
        let canonical = image.canonicalize().expect("canonical image");

        let mut outcome = Outcome::default();
        remove_program_files(
            &root,
            true,
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
        // but holds only spared items. It reported "Removed: program files"
        // having deleted nothing, which is how an uninstall aimed at the wrong
        // directory read as a complete success with exit code zero.
        let root = std::env::temp_dir().join("speakeasy-uninstall-nothing-to-do");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proof")).expect("proof");
        std::fs::write(root.join("proof").join("cudart64_12.dll"), b"big").expect("runtime file");

        let mut outcome = Outcome::default();
        remove_program_files(&root, false, &RunningImage::Elsewhere, &mut outcome);

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
    fn an_absent_installation_is_not_a_failure() {
        // Uninstalling something already gone is the ordinary result of running
        // it twice, and reporting failure there teaches users to ignore output
        // that matters the one time it does not.
        let absent = std::env::temp_dir().join("speakeasy-uninstall-absent");
        let _ = std::fs::remove_dir_all(&absent);

        let mut outcome = Outcome::default();
        remove_program_files(&absent, true, &RunningImage::Elsewhere, &mut outcome);

        assert!(outcome.failed.is_empty());
        assert!(outcome.removed.is_empty());
    }
}
