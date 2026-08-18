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
    EngineAdmissibility, ExecutionProvider, GpuProbe, GpuSnapshot, HardwareProbe, HardwareSnapshot,
    NvmlGpuProbe, Pack, PackRole, SafeStandardHardwareProbe, admit_engines, bundled_manifest,
};

/// Everything the compatibility step reports.
pub struct MachineReport {
    pub hardware: HardwareSnapshot,
    pub gpu: GpuSnapshot,
    pub admissibility: EngineAdmissibility,
    /// What each engine's graphics-card weights occupy, from the manifest.
    pub streaming_weights_bytes: u64,
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
/// One GPU snapshot for both engines, deliberately. `free_vram_bytes` moves —
/// a browser and a compositor were holding 6.7 GB on the machine `gpu.rs` was
/// written on — so probing twice could admit one engine on memory the other has
/// already been told it can have.
pub fn run() -> MachineReport {
    let hardware = SafeStandardHardwareProbe.probe(&install_root());
    let gpu = NvmlGpuProbe.probe();
    let (streaming_weights_bytes, granite_weights_bytes) = cuda_weights_bytes();
    let admissibility = admit_engines(&gpu, streaming_weights_bytes, granite_weights_bytes);
    MachineReport {
        hardware,
        gpu,
        admissibility,
        streaming_weights_bytes,
        granite_weights_bytes,
    }
}

/// How much VRAM each engine's weights occupy, read from the pinned manifest.
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
fn cuda_weights_bytes() -> (u64, u64) {
    let Ok(manifest) = bundled_manifest() else {
        // A manifest this binary cannot parse is a real problem, but not one to
        // resolve by inventing sizes. Zero required means VRAM never rejects,
        // leaving the capability floor and the execution check as the gates —
        // failing open here is safe because neither of those is bypassed.
        return (0, 0);
    };
    let largest = |role: PackRole, provider: Option<ExecutionProvider>| {
        manifest
            .packs()
            .iter()
            .filter(|pack| {
                pack.role() == role
                    && pack.is_install_eligible()
                    && provider.is_none_or(|wanted| pack.runtime().provider() == wanted)
            })
            // Largest rather than first: more than one revision of a role can be
            // eligible, and the conservative figure is the one that keeps a
            // borderline card from being told it fits.
            .map(Pack::installed_bytes)
            .max()
            .unwrap_or(0)
    };
    (
        // Streaming has a real CUDA pack — a self-exported float ONNX model,
        // published because upstream ships int8 only and the CUDA provider does
        // not implement the int8 operators.
        largest(PackRole::StreamingAsr, Some(ExecutionProvider::Cuda)),
        // Granite has none, and asking for one returns zero. There is no
        // separate Granite GPU pack by design: the CUDA worker offloads the
        // *CPU-variant* GGUF, which is why `engine=cpu_gpu_pack_not_installed
        // device=cuda` is the correct state in the log rather than a fault.
        // Filtering by provider here reported "0.0 GB of weights" for a model
        // that is over three gigabytes — measured 2026-08-15, in the wizard.
        largest(PackRole::FinalAsr, None),
    )
}
