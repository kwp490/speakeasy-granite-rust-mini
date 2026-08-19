//! What this computer can actually run, decided per engine.
//!
//! Everything here is reused from `speakeasy-models` rather than rebuilt:
//! `SafeStandardHardwareProbe` for the inventory, `NvmlGpuProbe` for the card,
//! `admit_engines` for the two verdicts. This module's only job is to supply the
//! one thing those cannot know on their own — how much each engine needs to fit
//! — and to hold the answers together so setup can report them separately.
//!
//! Reported separately is the requirement, not a presentation choice. The two
//! engines use different runtimes and fail in opposite ways, so a machine that
//! runs one on the graphics card and the other on the processor is an ordinary
//! outcome. Collapsing that into a single "GPU: yes/no" would be wrong for one
//! of them every time it happened.

use std::ffi::OsStr;
use std::path::PathBuf;

use speakeasy_models::{
    GpuProbe, GpuQualification, GpuSnapshot, HardwareProbe, HardwareSnapshot, NvmlGpuProbe, Pack,
    PackRole, SafeStandardHardwareProbe, admit_engine, bundled_manifest,
};

/// Everything the compatibility step reports.
pub struct MachineReport {
    pub hardware: HardwareSnapshot,
    pub gpu: GpuSnapshot,
    /// One verdict now, not two. `EngineAdmissibility` existed to keep the
    /// streaming engine's answer and Granite's from being collapsed, because a
    /// machine could legitimately run one on the GPU and the other on the CPU.
    /// There is one engine, so there is one answer.
    pub admissibility: GpuQualification,
    /// What the graphics-card weights occupy, from the manifest.
    pub granite_weights_bytes: u64,
}

/// This product's name, wherever a path, a registry value or a Start Menu folder
/// has to carry it.
///
/// One constant because every one of these was independently the parent
/// product's string until 2026-08-18, and each was its own route to installing
/// over `SpeakEasy` or deleting it. A literal repeated five times is five
/// chances to fix four of them. `install::VERSION_KEY` and
/// `install::UNINSTALL_KEY` cannot use this — `concat!` takes literals, not
/// consts — so they stay literals and are pinned against it by test instead.
pub const PRODUCT: &str = "SpeakEasy Mini";

/// Where the app will be installed, or `None` when the profile does not say.
///
/// The disk figure has to be measured against the volume the files actually land
/// on, not the current directory — setup is commonly run from a Downloads folder
/// on a different drive from the one being installed to.
///
/// The leaf is this product's own name, and until 2026-08-18 it was the parent
/// product's. That made `%LOCALAPPDATA%\SpeakEasy` the default destination:
/// setup would have written `SpeakEasy Mini`'s executables over an existing
/// `SpeakEasy` installation, and because `uninstall` removes the install
/// directory whole, uninstalling Mini would then have taken `SpeakEasy` with it.
/// Nothing caught it because no installer had been built since the fork.
///
/// **`None` rather than a guess.** This returned `C:\` when `LOCALAPPDATA` was
/// unset, which is worse than the bug above: setup would have unpacked into the
/// drive root, registered it as the install location, and uninstall would then
/// have walked `C:\` removing what it found there. An absent profile variable is
/// not a condition to paper over with a plausible-looking path — `shortcut`'s
/// own missing-`APPDATA` handling already says as much — so every caller that
/// would write somewhere now has to say it cannot instead.
pub fn install_root() -> Option<PathBuf> {
    install_root_under(std::env::var_os("LOCALAPPDATA").as_deref())
}

/// The whole of [`install_root`]'s decision, with the environment handed in.
///
/// Split out to be testable at all: `LOCALAPPDATA` is process-global, and this
/// workspace is edition 2024 under `unsafe_code = "forbid"`, where
/// `std::env::set_var` is `unsafe`. A test cannot reach the real variable, so
/// the decision has to live somewhere a test can hand it one.
///
/// Empty counts as absent. `PathBuf::from("").join(PRODUCT)` is the bare
/// relative path `SpeakEasy Mini`, which would install into whatever directory
/// setup happened to be launched from.
fn install_root_under(local_app_data: Option<&OsStr>) -> Option<PathBuf> {
    let local = local_app_data?;
    if local.is_empty() {
        return None;
    }
    Some(PathBuf::from(local).join(PRODUCT))
}

/// Probe the machine.
///
/// One GPU snapshot, read once. `free_vram_bytes` moves — a browser and a
/// compositor were holding 6.7 GB on the machine `gpu.rs` was written on — so
/// every figure this report carries has to come from the same instant.
pub fn run() -> MachineReport {
    // Measured against the current directory when there is no install root to
    // measure against. The figure is only ever reported, and every path that
    // would actually write refuses separately on the same condition, so a
    // disk number about the wrong volume cannot reach an install.
    let hardware =
        SafeStandardHardwareProbe.probe(&install_root().unwrap_or_else(|| PathBuf::from(".")));
    let gpu = NvmlGpuProbe.probe();
    let granite_weights_bytes = cuda_weights_bytes();
    let admissibility = admit_engine(&gpu, granite_weights_bytes);
    MachineReport {
        hardware,
        gpu,
        admissibility,
        granite_weights_bytes,
    }
}

/// How much VRAM the weights occupy, read from the pinned manifest.
///
/// **A floor, and reported as one.** `installed_bytes` is what the artifact
/// occupies on disk, which is the weights — it is not the working set, because
/// activations and the runtime's own allocations are on top of it and neither
/// has been measured here. So a card that clears this has room for the weights
/// and nothing has been proven beyond that, which is exactly the admissible-but-
/// untested state the GPU module already models. The execution check is what
/// settles it.
///
/// Read from the manifest rather than written down here so the number cannot
/// drift from the artifact it describes: these change with every pack revision,
/// and a constant in this file would keep reporting the old one.
fn cuda_weights_bytes() -> u64 {
    let Ok(manifest) = bundled_manifest() else {
        // A manifest this binary cannot parse is a real problem, but not one to
        // resolve by inventing sizes. Zero required means VRAM never rejects,
        // leaving the capability floor and the execution check as the gates —
        // failing open here is safe because neither of those is bypassed.
        return 0;
    };
    let largest = |role: PackRole| {
        manifest
            .packs()
            .iter()
            .filter(|pack| pack.role() == role && pack.is_install_eligible())
            // Largest rather than first: more than one revision of a role can be
            // eligible, and the conservative figure is the one that keeps a
            // borderline card from being told it fits.
            .map(Pack::installed_bytes)
            .max()
            .unwrap_or(0)
    };
    // Deliberately not filtered by provider. There is no separate Granite GPU
    // pack: the CUDA worker offloads the *CPU-variant* GGUF, so asking for a
    // CUDA pack returns zero — and filtering that way once reported "0.0 GB of
    // weights" for a model over three gigabytes, measured 2026-08-15 in the
    // wizard.
    largest(PackRole::FinalAsr)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The install directory's leaf is this product's, not the parent's.
    ///
    /// Pinned because the defect it guards against shipped: the leaf was
    /// `SpeakEasy` until 2026-08-18, which pointed setup at an existing install
    /// of the parent product — and `uninstall` removes the install directory
    /// whole, so uninstalling this app would have deleted that one.
    ///
    /// `Test-InstallerLifecycle.ps1` cannot catch this. It passes
    /// `--install-root` explicitly, so it never exercises the default at all.
    #[test]
    fn the_install_root_leaf_is_this_product_and_sits_under_local_app_data() {
        let root = install_root_under(Some(OsStr::new(r"C:\Users\alice\AppData\Local")))
            .expect("a set LOCALAPPDATA resolves");

        assert_eq!(
            root,
            PathBuf::from(r"C:\Users\alice\AppData\Local\SpeakEasy Mini"),
            "install root must be PRODUCT under LOCALAPPDATA"
        );
        assert_eq!(
            root.file_name().expect("leaf"),
            OsStr::new(PRODUCT),
            "leaf must be PRODUCT, not the parent's name"
        );
        assert_ne!(
            root.file_name().expect("leaf"),
            OsStr::new("SpeakEasy"),
            "must not install into the parent's directory"
        );
    }

    /// An absent or empty `LOCALAPPDATA` refuses instead of guessing.
    ///
    /// Both cases used to produce a writable path: absent gave `C:\`, and empty
    /// gave the bare relative `SpeakEasy Mini`. The first would have unpacked
    /// into the drive root and left `uninstall` walking it; the second would
    /// have installed into whatever directory setup was launched from.
    #[test]
    fn an_unset_or_empty_local_app_data_yields_no_install_root() {
        assert_eq!(install_root_under(None), None, "unset must refuse");
        assert_eq!(
            install_root_under(Some(OsStr::new(""))),
            None,
            "empty must refuse, not go relative"
        );
        assert_ne!(
            install_root_under(None),
            Some(PathBuf::from(r"C:\")),
            "drive root is the guess this replaced"
        );
    }

    /// Every identity string carries `PRODUCT`.
    ///
    /// The two registry keys are `const` and cannot be built from `PRODUCT`
    /// (`concat!` takes literals), so this is what keeps them in step. Each of
    /// these was independently the parent product's until 2026-08-18, and each
    /// was its own way to write into or delete that installation.
    #[test]
    fn the_registry_identity_keys_name_this_product() {
        assert!(
            crate::install::VERSION_KEY.contains(PRODUCT),
            "version stamp must be under PRODUCT: {}",
            crate::install::VERSION_KEY
        );
        assert!(
            crate::install::UNINSTALL_KEY.ends_with("ai.speakeasy.mini"),
            "ARP entry must be this identifier: {}",
            crate::install::UNINSTALL_KEY
        );
    }
}
