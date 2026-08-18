//! `SpeakEasy`'s bootstrapper: one executable, two entry modes.
//!
//! Setup places the app on a machine that has nothing. Repair operates on one
//! where something is already wrong. They are a single binary because the owner
//! ruled out a third executable (2026-08-14), and the two modes are separated
//! here, at the entry point, rather than grown apart later — the brief this work
//! came from is explicit that retrofitting the second mode is much harder than
//! designing for it, and repair is the harder environment of the two.

// Console subsystem, for now, and the reason is measured rather than assumed.
//
// `#![windows_subsystem = "windows"]` is what a window-drawing binary wants:
// `CLAUDE.md` records a console window as one of three separate causes of the
// same silent bug, where anything `SpeakEasy` puts in the foreground becomes the
// delivery target and hijacks a dictation. But a GUI-subsystem process is not a
// usable command-line tool, and that is not a style objection — measured
// 2026-08-15 with the attribute enabled and `console.rs` attaching correctly:
//
//     $out = & speakeasy-bootstrapper verify <manifest>   # $out is EMPTY
//
// PowerShell does not wait for a GUI-subsystem process, so it captures nothing
// and returns before the process writes. `Test-InstallerLifecycle.ps1` parses
// `backup_manifest=` out of exactly that capture, and `docs/RUNBOOK.md`
// documents these as commands you type and read the output of. The switch
// therefore breaks the CLI contract the owner asked to preserve exactly.
//
// So the binary stays console-subsystem and setup re-launches itself detached
// instead: the wizard then runs in a process with no console at all, which is
// what the delivery-target hazard actually requires, while the verbs keep the
// synchronous stdout every caller of them already depends on. `CLAUDE.md`
// records the same creation-flag approach as the fix for the console-subsystem
// workers, so this is the established answer here rather than a new invention.
//
// The visible cost is a brief console flash when setup is double-clicked, before
// the detached wizard appears. Windows creates the console before any code runs,
// so no amount of care inside `main` removes it; only a second, GUI-subsystem
// binary would, and the owner ruled out a third executable.
//
// `console.rs` stays. It is correct, and it is what repair mode needs the moment
// this decision is revisited.

mod catalog;
mod console;
mod download;
mod install;
mod probe;
mod repair;
mod shortcut;
mod uninstall;
mod webview2;
mod wizard;

use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    match Mode::classify(&arguments) {
        Mode::Setup => relaunch_detached(),
        Mode::SetupDetached => wizard(),
        Mode::Install { install_root } => {
            place(install_root.as_deref(), console::ensure_attached())
        }
        Mode::Uninstall { silent, remove_all } => remove(silent, remove_all),
        Mode::Misuse { detail } => {
            repair::report(
                &catalog::arguments_not_understood(&detail),
                console::ensure_attached(),
                repair::Severity::Failure,
            );
            ExitCode::FAILURE
        }
        // Attached before the first write, never after: Rust resolves the
        // standard-output handle once and caches it.
        Mode::Repair(verbs) => repair::main(verbs, console::ensure_attached()),
    }
}

/// Which half of the bootstrapper an invocation is asking for.
///
/// Deliberately an enum over a boolean: the two halves have different failure
/// modes, different output surfaces and different amounts of Windows underneath
/// them, and naming them is what keeps the wizard from being bolted onto the
/// side of the CLI.
enum Mode<'a> {
    /// No arguments — a double-click, which is how nearly every user will ever
    /// run this. Re-launches itself detached and exits; see `DETACHED_ARGUMENT`.
    Setup,
    /// The detached relaunch. Draws the wizard.
    SetupDetached,
    /// Install without the wizard.
    ///
    /// Exists because installation has to be drivable by a script:
    /// `Test-InstallerLifecycle.ps1` proves the upgrade, same-version and
    /// downgrade refusals by running an install and reading its exit code, and
    /// a refusal that only a human can observe is a refusal nothing verifies.
    Install { install_root: Option<OsString> },
    /// What the Add/Remove Programs entry invokes. Keeps every optional item by
    /// default, matching what NSIS's `/SD IDYES` did in its silent path.
    Uninstall { silent: bool, remove_all: bool },
    /// A recognised verb whose remaining arguments were not understood.
    ///
    /// Distinct from [`Mode::Repair`], which is where an *unrecognised* verb
    /// goes to be rejected with the usage text it has always printed. This is
    /// the opposite case: setup knows what was asked for and refuses because it
    /// cannot be sure of the details.
    Misuse { detail: String },
    /// Anything else. The verb surface is preserved exactly from the absorbed
    /// `speakeasy-repair` so `docs/RUNBOOK.md` stays accurate, which means an
    /// unrecognised verb has to reach the repair parser to be rejected with the
    /// usage it has always printed.
    Repair(&'a [OsString]),
}

/// Marks the relaunch that actually draws the wizard.
///
/// Deliberately not a documented verb: it is an implementation detail of getting
/// a console-free process, and `docs/RUNBOOK.md`'s command surface is a contract.
/// A user typing it gets the wizard, which is harmless — it is what they would
/// have got by double-clicking.
const DETACHED_ARGUMENT: &str = "--wizard-detached";

impl<'a> Mode<'a> {
    /// Parse the command line, refusing anything not understood exactly.
    ///
    /// The strictness is not tidiness, and it was not free — it is here because
    /// the permissive version silently installed `SpeakEasy` into the wrong
    /// directory. Measured 2026-08-15: `Test-InstallerLifecycle.ps1` passed its
    /// install root through `Start-Process -ArgumentList`, which joins an array
    /// with spaces and quotes nothing, so
    ///
    ///     --install --install-root C:\Coding Projects\...\installer-lifecycle
    ///
    /// arrived as four arguments. A scan for the `--install-root` *pair* found
    /// one, took `C:\Coding` as the root, dropped the remainder on the floor and
    /// installed there — creating a directory at the top of the drive, writing
    /// 45 MB into it, reporting success and exiting zero. The wrongness was
    /// invisible to everything: the exit code was 0, the message was accurate
    /// about the root it had chosen, and the only symptom was a file the caller
    /// then failed to find where it expected.
    ///
    /// So an argument list is now either understood completely or refused. A
    /// mis-quoted path is a caller's bug, and the one thing setup must never do
    /// with it is guess a destination and write there.
    fn classify(arguments: &'a [OsString]) -> Self {
        match arguments {
            [] => Self::Setup,
            [only] if only == DETACHED_ARGUMENT => Self::SetupDetached,
            [first, rest @ ..] if first == "--install" => match rest {
                [] => Self::Install { install_root: None },
                [flag, root] if flag == "--install-root" => Self::Install {
                    install_root: Some(root.clone()),
                },
                unexpected => Self::misuse("--install", unexpected),
            },
            [first, rest @ ..] if first == "--uninstall" => {
                // `/S` as well as `--silent`: `Test-InstallerLifecycle.ps1`
                // drove `uninstall.exe /S`, and keeping the spelling means the
                // script's silent path is the same instruction it always was.
                const SILENT: &[&str] = &["--silent", "/S", "/s"];
                const REMOVE_ALL: &str = "--remove-all";

                if let Some(unexpected) = rest.iter().position(|argument| {
                    !SILENT.iter().any(|flag| argument == flag) && argument != REMOVE_ALL
                }) {
                    return Self::misuse("--uninstall", &rest[unexpected..]);
                }
                Self::Uninstall {
                    silent: rest
                        .iter()
                        .any(|argument| SILENT.iter().any(|flag| argument == flag)),
                    // Never implied by `--silent`. An unattended uninstall that
                    // deletes transcript history because nobody was there to say
                    // no is the exact failure the keep-by-default rule exists to
                    // stop.
                    remove_all: rest.iter().any(|argument| argument == REMOVE_ALL),
                }
            }
            _ => Self::Repair(arguments),
        }
    }

    /// Describe what could not be understood, quoted so a lost space is visible.
    ///
    /// The quoting is the point. `C:\Coding` and `Projects\speakeasy` reported as
    /// bare text read like one path that got wrapped; reported as `"C:\Coding"
    /// "Projects\speakeasy"` they read as what they are, which is the difference
    /// between a user seeing their own quoting mistake and filing a bug.
    fn misuse(verb: &str, unexpected: &[OsString]) -> Self {
        Self::Misuse {
            detail: format!(
                "{verb} {}",
                unexpected
                    .iter()
                    .map(|argument| format!("\"{}\"", argument.to_string_lossy()))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    }
}

/// Start the wizard in a process that has no console, and get out of the way.
///
/// `DETACHED_PROCESS` rather than `CREATE_NO_WINDOW`: the latter keeps a console
/// the process simply cannot show, which is enough to avoid a stray window but
/// leaves the wizard owning a console it has no use for. Neither inherits this
/// process's console, which is the property that matters.
fn relaunch_detached() -> ExitCode {
    /// `DETACHED_PROCESS`, from `processthreadsapi.h`. Spelled here because
    /// `std` exposes the flags as a bare `u32` and this workspace forbids the
    /// `unsafe` that a Windows-crate constant would otherwise cost.
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    use std::os::windows::process::CommandExt;

    let Ok(executable) = std::env::current_exe() else {
        // Nothing sensible left to do: without our own path there is no wizard
        // to launch, and the user double-clicked expecting one.
        return ExitCode::FAILURE;
    };
    match std::process::Command::new(executable)
        .arg(DETACHED_ARGUMENT)
        .creation_flags(DETACHED_PROCESS)
        // Null, not inherited, and this is not tidiness. `Command` inherits the
        // standard handles by default, so a wizard launched from a script kept
        // the caller's stdout pipe open and the caller blocked until the user
        // finished setup — measured 2026-08-15 as a two-minute hang on
        // `& speakeasy-bootstrapper | Out-Null`, where the parent had already
        // exited in 37 ms and only the detached child still held the pipe.
        // Detaching from the console does not detach from inherited handles.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        // Deliberately not waited on. This process exists only to shed its
        // console, and holding the terminal open until the user finishes setup
        // would undo the point of detaching.
        Ok(_) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Install without the wizard.
///
/// Exit code is the contract, and every refusal is non-zero: a script proving
/// that a downgrade is refused reads this, and the previous NSIS installer
/// signalled the same refusals the same way.
fn place(install_root: Option<&std::ffi::OsStr>, destination: console::Destination) -> ExitCode {
    let decision = install::decide_now();
    if !decision.may_proceed() {
        repair::report(
            &catalog::describe_install_decision(&decision),
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    }
    let root = install_root.map_or_else(probe::install_root, std::path::PathBuf::from);
    let Some(payload) = install::payload_directory() else {
        repair::report(
            catalog::PAYLOAD_UNLOCATABLE,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    };
    match install::perform(&payload, &root) {
        Ok(()) => {
            repair::report(
                &format!(
                    "installed version={} root={}",
                    env!("CARGO_PKG_VERSION"),
                    root.display()
                ),
                destination,
                repair::Severity::Information,
            );
            ExitCode::SUCCESS
        }
        Err(reason) => {
            repair::report(
                &catalog::install_failed(&reason),
                destination,
                repair::Severity::Failure,
            );
            ExitCode::FAILURE
        }
    }
}

/// Uninstall.
///
/// Refuses while the app is running, for the same reason installing does: files
/// held open cannot be removed, and a partial uninstall is worse than none.
///
/// Silent keeps every optional item, which is what `/SD IDYES` meant in the NSIS
/// path this replaces — an unattended uninstall must never be the thing that
/// deletes a user's transcript history. The interactive path is the wizard's
/// uninstall page, which is where the five choices are actually offered.
fn remove(silent: bool, remove_all: bool) -> ExitCode {
    if install::app_is_running() {
        repair::report(
            catalog::UNINSTALL_REFUSED_RUNNING,
            if silent {
                console::ensure_attached()
            } else {
                console::Destination::None
            },
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    }
    if !silent && !remove_all {
        // The interactive page is the next increment; until it exists this must
        // not silently pick the destructive answers, so it picks the safe ones
        // and says which.
        repair::report(
            &catalog::uninstall_keeps_user_data(),
            console::Destination::None,
            repair::Severity::Information,
        );
    }
    let removals = if remove_all {
        uninstall::Removals::everything()
    } else {
        uninstall::Removals::default()
    };
    // Resolved before `perform`, which is not incidental: `perform` clears the
    // registration first, so reading the recorded location afterwards would find
    // the key already gone and silently fall back to the default directory.
    let root = install::installed_location().unwrap_or_else(probe::install_root);
    let outcome = uninstall::perform(&root, removals);
    let destination = console::ensure_attached();
    if outcome.failed.is_empty() {
        repair::report(
            &catalog::describe_uninstall(&outcome),
            destination,
            repair::Severity::Information,
        );
        ExitCode::SUCCESS
    } else {
        repair::report(
            &catalog::describe_uninstall(&outcome),
            destination,
            repair::Severity::Failure,
        );
        ExitCode::FAILURE
    }
}

/// The wizard itself, in the console-free process `relaunch_detached` created.
fn wizard() -> ExitCode {
    match wizard::Wizard::new().run() {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            // Nowhere to print — this process has no console by construction —
            // and a wizard that vanishes without a word is the failure mode the
            // user cannot report. `Destination::None` routes to a dialog.
            repair::report(
                &format!("{}: {error}", env!("CARGO_PKG_NAME")),
                console::Destination::None,
                repair::Severity::Failure,
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(arguments: &[&str]) -> Mode<'static> {
        // Leaked so the borrow outlives the call, which is what lets a test read
        // the classification without threading a lifetime through every case.
        let owned: &'static [OsString] = Box::leak(
            arguments
                .iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        Mode::classify(owned)
    }

    #[test]
    fn an_install_root_split_on_its_spaces_is_refused_rather_than_guessed() {
        // The regression this whole parser exists for. `Start-Process
        // -ArgumentList` joins an array with spaces and quotes nothing, so an
        // install root under `C:\Coding Projects\...` arrived as two arguments.
        // The permissive parser took the first fragment, installed 45 MB into a
        // directory it created at the top of the drive, and exited zero.
        let mode = classify(&[
            "--install",
            "--install-root",
            r"C:\Coding",
            r"Projects\speakeasy\target\installer-lifecycle",
        ]);

        let Mode::Misuse { detail } = mode else {
            panic!("a split install root must never be accepted as a destination");
        };
        // Both fragments have to appear, individually quoted: the reader's own
        // command looks correct to them, and seeing where it was split is the
        // only thing that identifies the fault as their quoting.
        assert!(detail.contains(r#""C:\Coding""#), "{detail}");
        assert!(
            detail.contains(r#""Projects\speakeasy\target\installer-lifecycle""#),
            "{detail}"
        );
    }

    #[test]
    fn a_quoted_install_root_arrives_whole() {
        let mode = classify(&["--install", "--install-root", r"C:\Coding Projects\out"]);

        let Mode::Install {
            install_root: Some(root),
        } = mode
        else {
            panic!("a single well-formed root must be accepted");
        };
        assert_eq!(root, OsString::from(r"C:\Coding Projects\out"));
    }

    #[test]
    fn install_without_a_root_uses_the_default() {
        assert!(matches!(
            classify(&["--install"]),
            Mode::Install { install_root: None }
        ));
    }

    #[test]
    fn uninstall_accepts_its_flags_in_any_order_and_refuses_anything_else() {
        // `/S` is the spelling `Test-InstallerLifecycle.ps1` has always used.
        assert!(matches!(
            classify(&["--uninstall", "/S"]),
            Mode::Uninstall {
                silent: true,
                remove_all: false
            }
        ));
        assert!(matches!(
            classify(&["--uninstall", "--remove-all", "--silent"]),
            Mode::Uninstall {
                silent: true,
                remove_all: true
            }
        ));
        // Silence must never imply removing user data, whatever the order.
        assert!(matches!(
            classify(&["--uninstall"]),
            Mode::Uninstall {
                silent: false,
                remove_all: false
            }
        ));
        assert!(matches!(
            classify(&["--uninstall", "--remove-everything"]),
            Mode::Misuse { .. }
        ));
    }

    #[test]
    fn an_unrecognised_verb_still_reaches_the_repair_parser() {
        // The repair verb surface is preserved exactly (owner decision), which
        // includes how it rejects things: an unknown verb has to reach repair to
        // be answered with the usage text `docs/RUNBOOK.md` documents, rather
        // than being intercepted here.
        assert!(matches!(classify(&["wobble"]), Mode::Repair(_)));
        assert!(matches!(
            classify(&["verify", "manifest.json"]),
            Mode::Repair(_)
        ));
    }

    #[test]
    fn no_arguments_is_setup_and_the_detached_marker_draws_the_wizard() {
        assert!(matches!(classify(&[]), Mode::Setup));
        assert!(matches!(
            classify(&[DETACHED_ARGUMENT]),
            Mode::SetupDetached
        ));
    }
}
