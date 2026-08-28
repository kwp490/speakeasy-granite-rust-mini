//! What this machine reports about its graphics card, beside what the registry
//! claims for the same one.
//!
//! **This is the evidence for reading VRAM from NVML rather than from
//! `speakeasy_models::hardware`**: run it on any card above 4 GB and the two
//! numbers disagree, with the registry pinned just under 4 GB. That is a
//! property of the registry's `dedicated_memory`, not of any card, and it is why
//! admission asks NVML.
//!
//! ```text
//! cargo run -p speakeasy-models --example gpu_inventory
//! ```
//!
//! It was a `#[test] #[ignore]`d as "diagnostic: reports host hardware, asserts
//! nothing" until 2026-08-28. A thing that asserts nothing is not a test, and
//! carrying it as one inflated the ignored count with something no run could
//! ever fail — the same shape as the seven scaffold tests that were skipped with
//! their inputs stubbed. `the_real_probe_answers_without_panicking_on_any_machine`
//! is the actual test of this path and runs in the gate; this is the instrument
//! beside it, and `speakeasy-audio`'s `cue_diagnostics` is the established
//! precedent for where an instrument lives.

use speakeasy_models::{GpuProbe, HardwareProbe, NvmlGpuProbe, SafeStandardHardwareProbe, admit};

fn main() {
    let snapshot = NvmlGpuProbe.probe();
    println!("driver_version={:?}", snapshot.driver_version);
    println!("unavailable={:?}", snapshot.unavailable);
    for device in &snapshot.devices {
        println!(
            "nvml name={} capability={} total_vram={} free_vram={}",
            device.name, device.compute_capability, device.total_vram_bytes, device.free_vram_bytes
        );
    }
    for adapter in SafeStandardHardwareProbe
        .probe(std::path::Path::new("C:\\"))
        .detected_adapters
    {
        println!(
            "registry name={} driver={:?} dedicated_memory={:?}",
            adapter.name, adapter.driver_version, adapter.dedicated_memory_bytes
        );
    }
    println!("decision={}", admit(&snapshot).code());
}
