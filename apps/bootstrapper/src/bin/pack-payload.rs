//! Turns the built bootstrapper into `SpeakEasyMiniSetup.exe`.
//!
//! One command, called by `scripts/Build-LocalInstaller.ps1`:
//!
//! ```text
//! pack-payload <payload directory> <bootstrapper> <output>
//! ```
//!
//! It copies the bootstrapper to the output path and appends the payload
//! directory to it, so the result is one file a user can download and run.
//!
//! # Why a second binary rather than a few lines of PowerShell
//!
//! Because the format would then have two implementations. A writer in the
//! packaging script and a reader in the installer agree right up until somebody
//! edits one of them, and the disagreement does not surface on the build
//! machine — it surfaces on a user's, as setup reporting a damaged download of
//! a file that downloaded perfectly. `payload.rs` is compiled into this binary
//! directly so that `append_archive` and `read_archive` are the same source.
//!
//! Console subsystem and a real exit code: the packaging script reads both.

// `payload.rs` belongs to `main.rs`'s module tree, and a `src/bin` target is its
// own crate root that cannot see it. Including the file is what keeps the format
// single-sourced; the alternative is restructuring the bootstrapper around a
// `lib.rs` for the sake of one build-time tool, which is a great deal of churn
// for no behaviour.
//
// Setup's half of the module — extracting a staged payload — is unused here,
// exactly as this binary's half is unused there.
#[allow(dead_code)]
#[path = "../payload.rs"]
mod payload;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [source, bootstrapper, output] = arguments.as_slice() else {
        eprintln!("usage: pack-payload <payload directory> <bootstrapper> <output>");
        return ExitCode::FAILURE;
    };
    match pack(
        &PathBuf::from(source),
        &PathBuf::from(bootstrapper),
        &PathBuf::from(output),
    ) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("pack-payload: {reason}");
            ExitCode::FAILURE
        }
    }
}

fn pack(source: &Path, bootstrapper: &Path, output: &Path) -> Result<String, String> {
    let entries = payload::collect(source).map_err(|error| error.to_string())?;
    if entries.is_empty() {
        // An empty payload packs and appends perfectly happily, and produces an
        // installer that runs, reports success and installs nothing. Refusing is
        // the only thing that separates "the payload was not built" from "the
        // payload was built".
        return Err(format!(
            "there are no files under {}, so the installer would carry nothing",
            source.display()
        ));
    }
    // Copied rather than appended to in place, so the plain bootstrapper stays a
    // plain bootstrapper. Running the packer twice over one file would otherwise
    // leave two archives, of which only the last is findable.
    std::fs::copy(bootstrapper, output).map_err(|error| error.to_string())?;
    payload::append_archive(output, &entries).map_err(|error| error.to_string())?;

    // Read back through the reader the installer will use. A packer that reports
    // success on an archive its own reader refuses is the exact instrument this
    // project keeps finding: the build looks green and the failure lands on the
    // user.
    let read = payload::read_archive(output)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the packed file does not read back as carrying a payload".to_owned())?;
    if read.len() != entries.len() {
        return Err(format!(
            "packed {} files and read back {}",
            entries.len(),
            read.len()
        ));
    }
    let bytes: u64 = entries.iter().map(|entry| entry.bytes.len() as u64).sum();
    Ok(format!(
        "packed {} files ({bytes} bytes) into {}",
        entries.len(),
        output.display()
    ))
}
