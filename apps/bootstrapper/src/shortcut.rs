//! Start Menu shortcuts.
//!
//! The one job in the NSIS replacement that genuinely needs COM: a `.lnk` is a
//! structured-storage file, not something to write by hand, and `IShellLink` is
//! the only supported way to produce one. `winsafe` binds it safely, which is
//! what keeps this reachable under the workspace's `unsafe_code = "forbid"`.
//!
//! NSIS created one shortcut, to the repair tool. The bootstrapper is now both
//! setup and repair, so the shortcut it creates points at itself — and the
//! wording matters: a user looking for the thing that fixes a broken install
//! should not have to know it is also the thing that installed it.

use std::path::{Path, PathBuf};

use winsafe::prelude::*;
use winsafe::{self as w, CoCreateInstance, CoInitializeEx, IPersistFile, IShellLink, co};

/// The Start Menu folder this product owns.
///
/// Under the user's own profile, matching the current-user install: nothing here
/// writes to the machine-wide Start Menu, which would need elevation this
/// deliberately does not request.
pub fn start_menu_folder() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(crate::probe::PRODUCT)
    })
}

/// Create (or replace) a `.lnk`.
///
/// `IPersistFile::Save` overwrites, which is what an upgrade wants: a shortcut
/// left pointing at a previous layout is the failure this replaces, and it is
/// silent — the icon still appears and still launches something.
pub fn create(link_path: &Path, target: &Path, description: &str) -> w::AnyResult<()> {
    if let Some(parent) = link_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Apartment-threaded, and the guard is held for the whole function: COM must
    // be initialized on this thread before `CoCreateInstance` and must outlive
    // every interface created from it. Dropping it early would release the
    // library while the link is still alive.
    let _com = CoInitializeEx(co::COINIT::APARTMENTTHREADED | co::COINIT::DISABLE_OLE1DDE)?;

    let link = CoCreateInstance::<IShellLink>(
        &co::CLSID::ShellLink,
        None::<&IShellLink>,
        co::CLSCTX::INPROC_SERVER,
    )?;
    link.SetPath(&target.to_string_lossy())?;
    link.SetDescription(description)?;
    // The install directory, so the app resolves anything it looks for beside
    // itself — `proof/` in particular — regardless of where the shortcut is
    // invoked from.
    if let Some(parent) = target.parent() {
        link.SetWorkingDirectory(&parent.to_string_lossy())?;
    }

    let persist = link.QueryInterface::<IPersistFile>()?;
    persist.Save(Some(&link_path.to_string_lossy()), true)?;
    Ok(())
}

/// Read a `.lnk` back and report what it points at.
///
/// Exists for verification rather than for the product: a shortcut that was
/// written but points somewhere wrong looks identical to a correct one until
/// someone clicks it, and "the file exists" is not evidence that it resolves.
pub fn target_of(link_path: &Path) -> w::AnyResult<String> {
    let _com = CoInitializeEx(co::COINIT::APARTMENTTHREADED | co::COINIT::DISABLE_OLE1DDE)?;
    let link = CoCreateInstance::<IShellLink>(
        &co::CLSID::ShellLink,
        None::<&IShellLink>,
        co::CLSCTX::INPROC_SERVER,
    )?;
    link.QueryInterface::<IPersistFile>()?
        .Load(&link_path.to_string_lossy(), co::STGM::READ)?;
    // `SLGP::RAWPATH` — the stored path, not one the shell has resolved or
    // relocated for us. Verification wants what was written down.
    Ok(link.GetPath(None, co::SLGP::RAWPATH)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_shortcut_points_where_it_was_told_to() {
        // Round-trips through the real shell COM object rather than asserting
        // the file exists. A `.lnk` that was created but points at the wrong
        // place is byte-for-byte a valid shortcut and looks correct in Explorer;
        // only reading the target back distinguishes them. This is the same
        // class of mistake as an upgrade leaving a shortcut on the old layout,
        // which is what this module exists to stop.
        let directory = std::env::temp_dir().join("speakeasy-shortcut-roundtrip");
        std::fs::create_dir_all(&directory).expect("temp directory");
        let link = directory.join("SpeakEasy Mini Setup and Repair.lnk");
        // A real file, so the target is something the shell can store rather
        // than a path it may normalise differently.
        let target = directory.join("speakeasy-bootstrapper.exe");
        std::fs::write(&target, b"not a real executable").expect("target file");

        create(&link, &target, "SpeakEasy Mini setup and repair").expect("shortcut created");
        let read_back = target_of(&link).expect("shortcut read back");

        assert!(link.is_file(), "the shortcut file must exist");
        assert!(
            read_back.eq_ignore_ascii_case(&target.to_string_lossy()),
            "shortcut points at {read_back}, expected {}",
            target.display()
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
