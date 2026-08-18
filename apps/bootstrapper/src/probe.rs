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

/// Where the app will be installed.
///
/// The disk figure has to be measured against the volume the files actually land
/// on, not the current directory — setup is commonly run from a Downloads folder
/// on a different drive from the one being installed to.
pub fn install_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(
        || PathBuf::from("C:\\"),
        |local| PathBuf::from(local).join("SpeakEasy"),
    )
}

/// Probe the machine.
///
/// One GPU snapshot, read once. `free_vram_bytes` moves — a browser and a
/// compositor were holding 6.7 GB on the machine `gpu.rs` was written on — so
/// every figure this report carries has to come from the same instant.
pub fn run() -> MachineReport {
    let hardware = SafeStandardHardwareProbe.probe(&install_root());
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
