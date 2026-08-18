//! Repair mode: the verb surface absorbed from `speakeasy-repair`.
//!
//! Preserved exactly — same verbs, same flags, same output shape — because
//! `docs/RUNBOOK.md` documents them as the recovery procedure and
//! `scripts/Test-InstallerLifecycle.ps1` drives them as a smoke test. Only the
//! binary carrying them changed. The GUI repair mode arrives on top of this
//! surface, never in place of it: a machine broken badly enough to need repair
//! is exactly the machine least able to draw a window.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use speakeasy_storage::{
    create_recovery_bundle, mark_update_pending, pending_update_status, restore_recovery_bundle,
    verified_installer_path, verify_recovery_bundle,
};

use crate::console::Destination;

/// The behavioural contract, in the words it has always been stated in.
///
/// Kept as prose rather than paraphrased on each print: every clause is a
/// property the code below actually has, and the absorption into the
/// bootstrapper is precisely the moment such a promise is easiest to soften by
/// accident.
pub const CONTRACT: &str = "These commands verify retained installers and backups, restore only into a\n\
     new or empty destination, and launch a verified reinstall only after an\n\
     explicit command. They never silently downgrade or overwrite existing user\n\
     data.";

/// How a result should be presented when there is no stream to print to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Information,
    Failure,
}

pub fn main(arguments: &[OsString], destination: Destination) -> ExitCode {
    match run(arguments) {
        Ok(message) => {
            report(&message, destination, Severity::Information);
            ExitCode::SUCCESS
        }
        Err(message) => {
            report(
                &format!("{}: {message}", env!("CARGO_PKG_NAME")),
                destination,
                Severity::Failure,
            );
            ExitCode::FAILURE
        }
    }
}

/// Put a result where the caller will actually see it.
///
/// The exit code is the contract for scripts and stays authoritative either way;
/// this is only about the human-readable half. A GUI-subsystem binary with no
/// console would otherwise succeed at printing into nothing, which is
/// indistinguishable from a command that had nothing to say — and this project
/// has shipped that failure before, in a cue that was silent because the check
/// that passed it was `exit 0`.
pub fn report(message: &str, destination: Destination, severity: Severity) {
    match destination {
        Destination::Stream => match severity {
            Severity::Information => println!("{message}"),
            Severity::Failure => eprintln!("{message}"),
        },
        Destination::None => {
            use winsafe::co::MB;
            use winsafe::prelude::*;

            let icon = match severity {
                Severity::Information => MB::ICONINFORMATION,
                Severity::Failure => MB::ICONEXCLAMATION,
            };
            // A message box is a window, and any window this process puts in the
            // foreground is a delivery-target candidate. Safe here and only here:
            // repair mode never runs a dictation, and this is the last thing the
            // process does. The wizard must not reuse it while a test dictation
            // is in flight.
            let _ = winsafe::HWND::NULL.MessageBox(message, "SpeakEasy", MB::OK | icon);
        }
    }
}

fn run(arguments: &[OsString]) -> Result<String, String> {
    let (command, rest) = arguments.split_first().ok_or_else(usage)?;
    match command.to_str() {
        // Additive, not a change to the six verbs. It exists because running
        // with no arguments used to list them and now opens the wizard, so
        // without this `docs/RUNBOOK.md` would have to document a way of
        // reading the command list that does not exist.
        Some("help" | "--help" | "-h") => Ok(format!("{CONTRACT}\n\n{}", usage())),
        Some("verify") => {
            let manifest = one_path(rest)?;
            let verified = verify_recovery_bundle(&manifest).map_err(debug_error)?;
            Ok(format!(
                "verified version={} data_files={} local_development_unsigned={}",
                verified.product_version,
                verified.data.len(),
                verified.local_development_unsigned
            ))
        }
        Some("backup") => backup(rest),
        Some("restore") => {
            let manifest = required_value(rest, "--manifest")?;
            let destination = required_value(rest, "--destination")?;
            let outcome = restore_recovery_bundle(Path::new(&manifest), Path::new(&destination))
                .map_err(debug_error)?;
            Ok(format!("restore={outcome:?}"))
        }
        Some("mark-pending") => {
            let data_root = required_value(rest, "--data-root")?;
            let version = required_utf8(rest, "--target-version")?;
            let manifest = required_value(rest, "--manifest")?;
            let marker = mark_update_pending(Path::new(&data_root), &version, Path::new(&manifest))
                .map_err(debug_error)?;
            Ok(format!("pending_marker={}", marker.display()))
        }
        Some("status") => {
            let data_root = required_value(rest, "--data-root")?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "system clock is before Unix epoch".to_owned())?
                .as_millis();
            let status = pending_update_status(
                Path::new(&data_root),
                i64::try_from(now).map_err(|_| "system clock cannot be represented".to_owned())?,
            )
            .map_err(debug_error)?;
            Ok(format!("pending_update={status:?}"))
        }
        Some("reinstall") => reinstall(rest),
        _ => Err(usage()),
    }
}

fn backup(arguments: &[OsString]) -> Result<String, String> {
    let data_root = required_value(arguments, "--data-root")?;
    let bundle_root = required_value(arguments, "--bundle-root")?;
    let installer = required_value(arguments, "--installer")?;
    let version = required_utf8(arguments, "--version")?;
    let created = required_utf8(arguments, "--created-unix-ms")?
        .parse::<i64>()
        .map_err(|_| "--created-unix-ms must be an integer".to_owned())?;
    let signature = optional_value(arguments, "--signature");
    let require_signature = has_flag(arguments, "--require-signature");
    let manifest = create_recovery_bundle(
        Path::new(&data_root),
        Path::new(&bundle_root),
        Path::new(&installer),
        signature.as_deref().map(Path::new),
        &version,
        created,
        require_signature,
    )
    .map_err(debug_error)?;
    Ok(format!("backup_manifest={}", manifest.display()))
}

fn reinstall(arguments: &[OsString]) -> Result<String, String> {
    let manifest_path = required_value(arguments, "--manifest")?;
    let manifest = verify_recovery_bundle(Path::new(&manifest_path)).map_err(debug_error)?;
    // This project is never signed (owner decision, 2026-08-14), so this branch
    // is the ordinary path rather than an edge case. The flag stays required
    // anyway: it is what makes reinstalling an unverifiable artifact a thing the
    // operator asked for in writing rather than something the tool decided.
    if manifest.local_development_unsigned
        && !has_flag(arguments, "--allow-unsigned-local-development")
    {
        return Err("unsigned local-development artifact requires the explicit \
             --allow-unsigned-local-development flag"
            .to_owned());
    }
    let installer = verified_installer_path(Path::new(&manifest_path)).map_err(debug_error)?;
    if !installer.is_absolute() {
        return Err("verified installer path was not absolute".to_owned());
    }
    let parent = installer
        .parent()
        .ok_or_else(|| "verified installer has no parent".to_owned())?;
    let status = Command::new(&installer)
        .current_dir(parent)
        .status()
        .map_err(|error| format!("failed to launch verified installer: {error}"))?;
    if !status.success() {
        return Err(format!("verified installer exited with {status}"));
    }
    Ok(format!(
        "explicit_reinstall_completed version={}",
        manifest.product_version
    ))
}

fn one_path(arguments: &[OsString]) -> Result<PathBuf, String> {
    if arguments.len() != 1 {
        return Err(usage());
    }
    Ok(PathBuf::from(&arguments[0]))
}

fn required_value(arguments: &[OsString], name: &str) -> Result<OsString, String> {
    optional_value(arguments, name).ok_or_else(|| format!("missing {name}"))
}

fn required_utf8(arguments: &[OsString], name: &str) -> Result<String, String> {
    required_value(arguments, name)?
        .into_string()
        .map_err(|_| format!("{name} must be valid Unicode"))
}

fn optional_value(arguments: &[OsString], name: &str) -> Option<OsString> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == OsStr::new(name))
        .map(|pair| pair[1].clone())
}

fn has_flag(arguments: &[OsString], name: &str) -> bool {
    arguments
        .iter()
        .any(|argument| argument == OsStr::new(name))
}

fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

pub fn usage() -> String {
    "usage: speakeasy-bootstrapper <verify MANIFEST | status --data-root PATH | backup \
     --data-root PATH --bundle-root PATH --installer PATH --version VERSION \
     --created-unix-ms N [--signature PATH] [--require-signature] | restore --manifest PATH \
     --destination NEW_OR_EMPTY_PATH | mark-pending --data-root PATH \
     --target-version VERSION --manifest PATH | reinstall --manifest PATH \
     [--allow-unsigned-local-development]>"
        .to_owned()
}
