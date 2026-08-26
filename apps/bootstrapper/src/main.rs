//! `SpeakEasy`'s bootstrapper: one executable, two entry modes.
//!
//! Setup places the app on a machine that has nothing. Repair operates on one
//! where something is already wrong. They are a single binary because the owner
//! ruled out a third executable (2026-08-14), and the two modes are separated
//! here, at the entry point, rather than grown apart later: retrofitting the
//! second mode is much harder than designing for it, and repair is the harder
//! environment of the two.

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
// Shared verbatim with `bin/pack-payload.rs`, which compiles the same file to
// *write* the archive setup *reads*. Each binary therefore sees the other's
// half as unused, and neither is: single-sourcing the format is the whole
// reason the packer is a Rust binary rather than four lines of PowerShell.
#[allow(dead_code)]
mod payload;
mod probe;
mod repair;
mod seed;
mod shortcut;
mod smoke;
mod uninstall;
mod uninstall_page;
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
        Mode::Uninstall {
            silent,
            keep_user_data,
        } => remove(silent, keep_user_data),
        Mode::VerifyProvider { install_root } => {
            verify_provider(install_root.as_deref(), console::ensure_attached())
        }
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
    /// What the Add/Remove Programs entry invokes. Removes everything by
    /// default; `--keep-user-data` is the opt-out.
    ///
    /// The default inverted on 2026-08-21 (owner decision). It kept every
    /// optional item, matching NSIS's `/SD IDYES`, which meant an uninstall left
    /// 2.14 GB of model weights, a settings tree, the transcript history and the
    /// diagnostic logs behind — and reported success. Keeping them is a *testing*
    /// affordance, for rapid install/uninstall cycles that would otherwise
    /// re-download the weights every time, and `Test-InstallerLifecycle.ps1` and
    /// `Test-SetupWizard.ps1` are the callers that want it.
    Uninstall { silent: bool, keep_user_data: bool },
    /// Re-run the engine check against an installed build and re-record what it
    /// proved.
    ///
    /// Exists so that `install-provider.txt` keeps having exactly one writer.
    /// `scripts/Enable-GraniteCuda.ps1` used to stage a CUDA worker over an
    /// installed processor build, which changed the answer to a question only
    /// setup had ever asked — and the alternative was a PowerShell script that
    /// read NVML
    /// and wrote the marker itself. That would be a second implementation of the
    /// three-gate proof, free to drift from this one, and the defect this whole
    /// surface exists to prevent was a *second* source of truth for the same
    /// claim. So the script calls this, and this calls `smoke::verify_engine`
    /// and `seed::record_installed_provider` — the same two functions the
    /// wizard's last page calls, in the same order.
    ///
    /// It is also the cheap remedy for `gpu_install_not_operational`: re-proving
    /// the engine no longer costs a reinstall.
    VerifyProvider { install_root: Option<OsString> },
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
                const KEEP_USER_DATA: &str = "--keep-user-data";

                if let Some(unexpected) = rest.iter().position(|argument| {
                    !SILENT.iter().any(|flag| argument == flag) && argument != KEEP_USER_DATA
                }) {
                    return Self::misuse("--uninstall", &rest[unexpected..]);
                }
                Self::Uninstall {
                    silent: rest
                        .iter()
                        .any(|argument| SILENT.iter().any(|flag| argument == flag)),
                    // `--remove-all` stood here and meant the opposite. It is
                    // **not** accepted as an alias: it named the thorough
                    // behaviour, that behaviour is now the default, and a flag
                    // that silently means "do what you were going to do anyway"
                    // is how a caller comes to believe it is still choosing.
                    // Anyone who passes it gets the misuse refusal and reads
                    // this change; nothing in the tree passed it.
                    keep_user_data: rest.iter().any(|argument| argument == KEEP_USER_DATA),
                }
            }
            // Deliberately the same shape as `--install`, including the
            // refusal: a verb that takes a path must refuse an argument list it
            // cannot consume whole, because a caller who lost a space to
            // `Start-Process -ArgumentList` and a caller who meant two
            // arguments are indistinguishable from here.
            [first, rest @ ..] if first == "--verify-provider" => match rest {
                [] => Self::VerifyProvider { install_root: None },
                [flag, root] if flag == "--install-root" => Self::VerifyProvider {
                    install_root: Some(root.clone()),
                },
                unexpected => Self::misuse("--verify-provider", unexpected),
            },
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
        // The tone is for the wizard's labels; a console line carries its
        // severity in the exit code and in `Severity` instead.
        let (message, _) = catalog::describe_install_decision(&decision);
        repair::report(&message, destination, repair::Severity::Failure);
        return ExitCode::FAILURE;
    }
    // An explicit `--install-root` wins; without one the profile has to say.
    // Refusing here is the whole point of `install_root` returning an `Option`:
    // the old default was `C:\`, and `install::perform` would have copied the
    // payload into the drive root and registered it as the install location.
    let Some(root) = install_root
        .map(std::path::PathBuf::from)
        .or_else(probe::install_root)
    else {
        repair::report(
            catalog::INSTALL_ROOT_UNLOCATABLE,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    };
    // Held for the whole install: when setup carries its payload inside itself,
    // dropping this deletes the directory the next line reads from.
    let payload = match payload::stage() {
        Ok(payload) => payload,
        Err(failure) => {
            repair::report(
                &catalog::describe_payload_failure(&failure),
                destination,
                repair::Severity::Failure,
            );
            return ExitCode::FAILURE;
        }
    };
    match install::perform(payload.directory(), &root) {
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

/// Re-prove which provider an installed build runs on, and record it.
///
/// The three gates are not re-implemented here. This resolves two paths, calls
/// [`smoke::verify_engine`], and turns its verdict into a
/// [`seed::record_installed_provider`] call — which is what makes
/// `install-provider.txt` still have exactly one writer with two callers, rather
/// than two writers. See [`Mode::VerifyProvider`].
///
/// # Nothing is written unless something was proved
///
/// Only `Verified` records. A mismatch means the engine ran and produced the
/// wrong words, and an unavailable verdict means it did not run at all; writing
/// `cpu` for either would be the same manufactured claim as writing `cuda` from
/// a radio button, one step milder. An absent or stale file reads as
/// `unrecorded` or is compared and reported, and both of those are honest.
///
/// # Why it refuses while the app is running
///
/// Not for file locks — it opens nothing the app holds. For the *answer*: the
/// proof is NVML listing this worker's own pid as holding a compute context, and
/// a resident worker belonging to the running app is a second process on the
/// same card. That does not make this check wrong, but a card with only enough
/// free memory for one of them makes it wrong in the direction that matters,
/// recording `cpu` for an installation that is fine. This was "the second of
/// two" until 2026-08-26, the other being `Enable-GraniteCuda.ps1`'s own
/// refusal; that script is retired, so it is the only one now — worth knowing
/// before anyone decides it is redundant.
fn verify_provider(
    install_root: Option<&std::ffi::OsStr>,
    destination: console::Destination,
) -> ExitCode {
    if install::app_is_running() {
        repair::report(
            catalog::UNINSTALL_REFUSED_RUNNING,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    }
    let Some(root) = install_root
        .map(std::path::PathBuf::from)
        .or_else(probe::install_root)
    else {
        repair::report(
            catalog::INSTALL_ROOT_UNLOCATABLE,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    };
    let Some(model_root) = download::installed_model_root() else {
        repair::report(
            catalog::DATA_ROOT_UNLOCATABLE,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    };
    let worker = smoke::staged_worker(&root);
    match smoke::verify_engine(&worker, &model_root) {
        smoke::Verdict::Verified { provider, evidence } => {
            let recorded = seed::record_installed_provider(match provider {
                smoke::ProvenProvider::GraphicsCard => seed::Provider::GraphicsCard,
                smoke::ProvenProvider::Processor => seed::Provider::Processor,
            });
            if !recorded {
                repair::report(
                    catalog::PROVIDER_NOT_RECORDED,
                    destination,
                    repair::Severity::Failure,
                );
                return ExitCode::FAILURE;
            }
            repair::report(
                &catalog::provider_recorded(
                    match provider {
                        smoke::ProvenProvider::GraphicsCard => "cuda",
                        smoke::ProvenProvider::Processor => "cpu",
                    },
                    evidence.code(),
                ),
                destination,
                repair::Severity::Information,
            );
            ExitCode::SUCCESS
        }
        smoke::Verdict::Mismatch { .. } => {
            repair::report(
                catalog::PROVIDER_VERIFY_MISMATCH,
                destination,
                repair::Severity::Failure,
            );
            ExitCode::FAILURE
        }
        smoke::Verdict::Unavailable { .. } => {
            // A worker that will not start, and one specific cause that can be
            // named. Anything else gets the general advice, because guessing at
            // a cause is how a user is sent to fix the wrong thing.
            let message = match smoke::gpu_payload_rejection(&worker) {
                Some(speakeasy_models::GpuPayloadRejection::RuntimeFilesMissing(files)) => {
                    catalog::provider_verify_runtime_missing(&files)
                }
                _ => catalog::PROVIDER_VERIFY_UNAVAILABLE.to_owned(),
            };
            repair::report(&message, destination, repair::Severity::Failure);
            ExitCode::FAILURE
        }
    }
}

/// Uninstall.
///
/// Refuses while the app is running, for the same reason installing does: files
/// held open cannot be removed, and a partial uninstall is worse than none.
///
/// **Removes everything unless told not to** (owner decision, 2026-08-21). It
/// kept every optional item, which is what `/SD IDYES` meant in the NSIS path
/// this replaces, and the result was an uninstall that left 2.14 GB of weights, a
/// settings tree, the transcript history and the logs behind while reporting
/// success. `--keep-user-data` is the opt-out and exists for rapid
/// install/uninstall cycles rather than for users.
///
/// The interactive path **asks first**, on [`uninstall_page`]: one page, a check
/// box per removable item, every box checked, and no second dialog behind it.
/// That is what makes inverting the default safe — the destructive answer is
/// only taken where somebody was there to see the whole scope named. A silent
/// run cannot ask, so it proceeds: `/S` is a caller asserting it already knows.
fn remove(silent: bool, keep_user_data: bool) -> ExitCode {
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
    // What a *silent* run removes. The interactive path replaces this below
    // with what the user chose on the page; it is computed here because the
    // refusals above return before either.
    let removals = if keep_user_data {
        uninstall::Removals::default()
    } else {
        uninstall::Removals::everything()
    };
    // Resolved before `perform`, which is not incidental: `perform` clears the
    // registration first, so reading the recorded location afterwards would find
    // the key already gone and silently fall back to the default directory.
    let destination = console::ensure_attached();
    // No recorded location and no profile directory means there is nothing this
    // can safely remove. It used to fall through to `C:\` here, where
    // `uninstall::perform` would have walked the drive root deleting what it
    // recognised.
    let Some(root) = install::installed_location().or_else(probe::install_root) else {
        repair::report(
            catalog::INSTALL_ROOT_UNLOCATABLE,
            destination,
            repair::Severity::Failure,
        );
        return ExitCode::FAILURE;
    };
    // Asked before anything is deleted, so the page can name the files in
    // `proof/` rather than the report having to explain them afterwards.
    //
    // The page is the confirmation. It has no second dialog behind it, and
    // `Cancel`, the close box and a window that could not be drawn all arrive
    // here as `None` -- because a page nobody saw is not consent.
    let removals = if silent {
        removals
    } else {
        // Seeded with what the caller asked for, and that is the fix for a real
        // data loss on 2026-08-26. This discarded `removals` and the page
        // hardcoded every box checked, so `--uninstall --keep-user-data` without
        // `--silent` drew a page primed to delete the profile — 4.28 GB of
        // weights, the settings tree and the vocabulary — from a command whose
        // name says the opposite. The flag worked only alongside `/S`, which is
        // the one combination both proof scripts pass, so nothing exercised the
        // other. A flag stating an intention has to reach the control that acts
        // on it.
        let Some(chosen) =
            uninstall_page::ask(&uninstall::unrecognised_proof_files(&root), removals)
        else {
            repair::report(
                catalog::UNINSTALL_CANCELLED,
                console::Destination::None,
                repair::Severity::Information,
            );
            // Success: the user asked a question and got the answer they chose.
            // A failure code here would make a cancelled uninstall look like a
            // broken one to anything scripting it.
            return ExitCode::SUCCESS;
        };
        chosen
    };
    let outcome = uninstall::perform(&root, removals);
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
        // `/S` is the spelling `Test-InstallerLifecycle.ps1` has always used, and
        // on its own it now means remove everything -- an unattended uninstall is
        // a caller asserting it already knows what it asked for.
        assert!(matches!(
            classify(&["--uninstall", "/S"]),
            Mode::Uninstall {
                silent: true,
                keep_user_data: false
            }
        ));
        assert!(matches!(
            classify(&["--uninstall", "--keep-user-data", "--silent"]),
            Mode::Uninstall {
                silent: true,
                keep_user_data: true
            }
        ));
        assert!(matches!(
            classify(&["--uninstall"]),
            Mode::Uninstall {
                silent: false,
                keep_user_data: false
            }
        ));
        // `--remove-all` meant the opposite of `--keep-user-data` and is
        // deliberately **not** an alias for the new default: a flag that means
        // "do what you were going to do anyway" lets a caller keep believing it
        // is choosing. Refused, so whoever passes it reads the change.
        assert!(matches!(
            classify(&["--uninstall", "--remove-all"]),
            Mode::Misuse { .. }
        ));
        assert!(matches!(
            classify(&["--uninstall", "--remove-everything"]),
            Mode::Misuse { .. }
        ));
    }

    #[test]
    fn verify_provider_takes_an_install_root_or_none_and_refuses_a_split_one() {
        // The same three cases `--install` has, for the same reason: this verb
        // also takes a path, and `Start-Process -ArgumentList` quotes nothing
        // while this repository's own path has a space in it. A verb that took
        // the first fragment would re-prove the provider of a directory nobody
        // named -- or, finding no worker there, record nothing and report a
        // broken install.
        assert!(matches!(
            classify(&["--verify-provider"]),
            Mode::VerifyProvider { install_root: None }
        ));
        let Mode::VerifyProvider {
            install_root: Some(root),
        } = classify(&[
            "--verify-provider",
            "--install-root",
            r"C:\Program Files\App",
        ])
        else {
            panic!("a single well-formed root must be accepted");
        };
        assert_eq!(root, OsString::from(r"C:\Program Files\App"));
        assert!(matches!(
            classify(&[
                "--verify-provider",
                "--install-root",
                r"C:\Coding",
                r"Projects\app"
            ]),
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
