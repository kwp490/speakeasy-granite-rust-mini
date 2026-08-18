//! Nvidia GPU inventory and the qualification decision built on it.
//!
//! This sits beside [`crate::hardware`] rather than inside it, and the split is
//! the point. `HardwareSnapshot` is inventory that says so in its own
//! limitations — "adapter detection is inventory only and never execution
//! qualification". This module is where a decision is finally made, so it has
//! to be explicit about how much of that decision is evidence and how much is
//! still outstanding.
//!
//! Two things are deliberately kept apart:
//!
//! * **Admissibility** — the card is an Nvidia part new enough to run the
//!   backends, established from NVML. Cheap, and knowable at launch.
//! * **Qualification** — a real execution test has loaded a model on this
//!   machine and produced a result. Expensive, and the only thing that
//!   justifies telling a user the GPU path works.
//!
//! [`GpuQualification::Admissible`] exists so the difference cannot be
//! collapsed by accident. Nothing here ever returns [`GpuQualification::Qualified`]:
//! that value is constructed by the caller that ran the execution test, and the
//! type is the record that one did.
//!
//! # Why not the registry
//!
//! [`crate::hardware`] already enumerates display adapters out of the Windows
//! registry, and reusing its `dedicated_memory_bytes` for a VRAM gate is the
//! obvious move. It is also wrong twice over, and both were measured on an
//! RTX 5090 (`gpu_inventory_report` below prints them):
//!
//! 1. `HardwareInformation.MemorySize` is a `REG_DWORD`, and `hardware.rs`
//!    reads it as `u64`. The type never matches, so `dedicated_memory_bytes`
//!    is already `None` on this card — the field looks like a VRAM source and
//!    supplies nothing.
//! 2. Fixing that type would not help. The stored DWORD is `0xFFF00000` —
//!    4293918720, a shade under 4 GB — because a 32-bit byte count cannot hold
//!    this card's 34190917632. WMI's `Win32_VideoController.AdapterRAM`
//!    surfaces the identical saturated figure. Every card past 4 GB looks the
//!    same, so a VRAM floor built on it rejects large cards *because* they are
//!    large.
//!
//! NVML reports 34190917632 for the same card. So VRAM comes from NVML, and
//! the registry scan stays what it already was: a name-only inventory.

use std::fmt::{self, Display, Formatter};

use crate::ExecutionProvider;

/// The oldest CUDA compute capability the GPU backends are admitted on.
///
/// 8.6 is Ampere, which is the 3000 series — the oldest generation this
/// project targets. Ada (4000) reports 8.9 and Blackwell (5000) reports 12.0,
/// so a single floor covers all three.
///
/// Only the 5000 series has actually been exercised here. This floor is
/// therefore a deliberate, recorded claim about two generations nobody has run:
/// admissibility is inferred from the capability, and the execution test is
/// what has to pass before any of them is called qualified. That is the whole
/// reason those are different states.
pub const MINIMUM_COMPUTE_CAPABILITY: ComputeCapability = ComputeCapability { major: 8, minor: 6 };

/// A CUDA compute capability, ordered as a version rather than a decimal.
///
/// Deriving `Ord` over `(major, minor)` is what makes 8.10 sort above 8.9;
/// reading these as decimals would put it below.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComputeCapability {
    pub major: u32,
    pub minor: u32,
}

impl Display for ComputeCapability {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// One CUDA-capable device, as NVML reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CudaDevice {
    pub name: String,
    pub compute_capability: ComputeCapability,
    /// Physically present VRAM.
    pub total_vram_bytes: u64,
    /// VRAM not currently allocated.
    ///
    /// Free rather than total is what a load has to fit into, and on a desktop
    /// it is a moving target — a browser and a compositor were holding 6.7 GB
    /// on the machine this was written on. Anything deciding whether a model
    /// fits has to read this at the moment it decides, not at launch.
    pub free_vram_bytes: u64,
}

/// What the probe could see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuSnapshot {
    /// The Nvidia driver version, when NVML answered.
    pub driver_version: Option<String>,
    /// Every CUDA device NVML enumerated, in NVML's order.
    pub devices: Vec<CudaDevice>,
    /// Why the probe saw nothing, when it saw nothing.
    pub unavailable: Option<GpuProbeFailure>,
}

impl GpuSnapshot {
    /// A snapshot recording that the probe could not run at all.
    pub const fn unavailable(failure: GpuProbeFailure) -> Self {
        Self {
            driver_version: None,
            devices: Vec::new(),
            unavailable: Some(failure),
        }
    }
}

/// Why NVML could not be consulted.
///
/// Distinguished from "no Nvidia card" on purpose: a machine with a healthy
/// card and a broken driver install needs different advice from a machine with
/// an AMD card, and collapsing the two produces a message that is wrong for
/// both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuProbeFailure {
    /// `nvml.dll` is not present. It ships with the Nvidia driver, so this
    /// usually means no Nvidia driver rather than a damaged one.
    LibraryMissing,
    /// NVML is present but refused to initialize.
    InitializationFailed,
    /// NVML initialized but would not answer a query.
    QueryFailed,
}

impl Display for GpuProbeFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LibraryMissing => "nvidia_management_library_missing",
            Self::InitializationFailed => "nvidia_management_library_init_failed",
            Self::QueryFailed => "nvidia_management_library_query_failed",
        })
    }
}

/// Why a machine is not admissible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuRejection {
    /// NVML could not be consulted.
    ProbeUnavailable(GpuProbeFailure),
    /// NVML answered and enumerated nothing.
    NoCudaDevice,
    /// Every device present is older than the floor.
    ComputeCapabilityTooLow {
        best: ComputeCapability,
        minimum: ComputeCapability,
    },
    /// The card qualifies but does not have room for this engine right now.
    ///
    /// Per-engine by nature, and the reason a single machine-wide GPU verdict
    /// cannot be correct: the streaming export and Granite's weights are
    /// different sizes, so one can fit while the other does not. Free rather
    /// than total VRAM, because free is what a load has to fit into — and it
    /// moves, so this is a reading at a moment, not a property of the machine.
    InsufficientFreeVram { free: u64, required: u64 },
}

impl Display for GpuRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProbeUnavailable(failure) => write!(formatter, "{failure}"),
            Self::NoCudaDevice => formatter.write_str("no_cuda_device"),
            Self::ComputeCapabilityTooLow { best, minimum } => write!(
                formatter,
                "compute_capability_{best}_below_minimum_{minimum}"
            ),
            // Bytes, not the device name: this reaches the diagnostic log, which
            // is a privacy surface.
            Self::InsufficientFreeVram { free, required } => write!(
                formatter,
                "insufficient_free_vram_{free}_below_required_{required}"
            ),
        }
    }
}

/// The state of the GPU decision for this machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuQualification {
    /// A device clears the capability floor. **Nothing has been executed on
    /// it.** This is permission to attempt the execution test, and no more —
    /// in particular it is not permission to tell the user the GPU works.
    Admissible { device: CudaDevice },
    /// An execution test loaded a model on this machine and it produced a
    /// result. Only a caller that ran one can build this.
    Qualified {
        device: CudaDevice,
        evidence: ExecutionEvidence,
    },
    /// This machine cannot run the GPU backends.
    Rejected(GpuRejection),
}

impl GpuQualification {
    /// Whether an execution test has actually passed here.
    ///
    /// Deliberately false for [`Self::Admissible`]. Every caller that wants to
    /// advertise the GPU path reads this, so it is the one place where
    /// conflating "should work" with "has worked" would leak out to a user.
    pub const fn is_qualified(&self) -> bool {
        matches!(self, Self::Qualified { .. })
    }

    /// The device this decision is about, when there is one.
    pub const fn device(&self) -> Option<&CudaDevice> {
        match self {
            Self::Admissible { device } | Self::Qualified { device, .. } => Some(device),
            Self::Rejected(_) => None,
        }
    }

    /// A stable code for logs and the UI. Never contains a device name: this
    /// travels into the diagnostic log, which is a privacy surface.
    pub fn code(&self) -> String {
        match self {
            Self::Admissible { .. } => "admissible_execution_untested".to_owned(),
            Self::Qualified { .. } => "qualified".to_owned(),
            Self::Rejected(rejection) => rejection.to_string(),
        }
    }
}

impl GpuQualification {
    /// Which execution provider this machine's packs should be selected for.
    ///
    /// The GPU-preferred, CPU-fallback rule, in one place so that it is one
    /// decision rather than a condition repeated at every call site.
    ///
    /// **`Admissible` is enough to prefer CUDA, and that is deliberate.** The
    /// alternative — requiring `Qualified` — cannot work: qualification comes
    /// from running a model, running a model needs the CUDA pack, and selecting
    /// the CUDA pack is what this decides. Demanding evidence here would mean
    /// no machine ever installs the pack that would produce the evidence.
    ///
    /// What that costs is bounded. Picking CUDA here is a claim about which
    /// *artifact* to fetch, not a claim to the user that the GPU works;
    /// [`Self::is_qualified`] remains the only thing that says that, and it
    /// stays false until something has actually run.
    ///
    /// This is also where a user override belongs when it arrives: forcing CPU
    /// is always allowed, forcing GPU only from `Admissible`.
    pub const fn preferred_provider(&self) -> ExecutionProvider {
        match self {
            Self::Admissible { .. } | Self::Qualified { .. } => ExecutionProvider::Cuda,
            Self::Rejected(_) => ExecutionProvider::Cpu,
        }
    }
}

/// Proof that a model ran on this GPU.
///
/// Mirrors [`crate::RuntimeEvidence`], which plays the same role for the CPU
/// runtime: a value that cannot be produced without having done the work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEvidence {
    /// Which backend proved it, e.g. `onnxruntime-cuda` or `llama-cpp-cuda`.
    pub provider: String,
    /// The artifact that was loaded.
    pub artifact_id: String,
    /// Samples pushed through. Zero would mean nothing was actually inferred,
    /// which is why the caller records it rather than asserting success.
    pub inference_sample_count: u64,
}

/// Reads the machine's Nvidia inventory.
///
/// A trait so the decision logic can be tested against machines this project
/// does not have: absent NVML, an AMD-only box, a 2000-series card below the
/// floor. Those branches decide whether a user is locked out of the app, and
/// they are the branches least likely to be exercised by hand on a developer
/// machine that has a working card in it.
pub trait GpuProbe {
    fn probe(&self) -> GpuSnapshot;
}

/// Decides admissibility from a snapshot.
///
/// Picks the best device rather than the first: NVML's ordering is by bus id,
/// so on a laptop with an integrated Nvidia part alongside a discrete one the
/// first device enumerated is not reliably the better one.
pub fn admit(snapshot: &GpuSnapshot) -> GpuQualification {
    if let Some(failure) = snapshot.unavailable {
        return GpuQualification::Rejected(GpuRejection::ProbeUnavailable(failure));
    }
    let Some(best) = snapshot
        .devices
        .iter()
        .max_by_key(|device| device.compute_capability)
    else {
        return GpuQualification::Rejected(GpuRejection::NoCudaDevice);
    };
    if best.compute_capability < MINIMUM_COMPUTE_CAPABILITY {
        return GpuQualification::Rejected(GpuRejection::ComputeCapabilityTooLow {
            best: best.compute_capability,
            minimum: MINIMUM_COMPUTE_CAPABILITY,
        });
    }
    GpuQualification::Admissible {
        device: best.clone(),
    }
}

/// Decides admissibility, given what the engine needs to fit.
///
/// This took a `GpuEngine` and had a sibling, `admit_engines`, that decided two
/// verdicts from one snapshot — because the streaming engine and Granite failed
/// differently and a machine could legitimately run one on the GPU and the
/// other on the CPU. There is one engine now, so a type whose entire purpose
/// was keeping two verdicts from being collapsed has nothing left to keep
/// apart.
///
/// `required_free_vram_bytes` is the **caller's** claim about this engine's
/// working set, taken from the pack it intends to install rather than guessed
/// here: this module knows about cards, not about model sizes, and a constant
/// invented in this file would be a number nobody measured drifting away from
/// artifacts that change with every manifest entry.
///
/// Everything else is shared with [`admit`] on purpose. In particular the
/// compute-capability floor is not the engine's: its own definition records it
/// as the oldest generation this *project* targets, not a limit ggml imposes —
/// it runs on considerably older cards.
pub fn admit_engine(snapshot: &GpuSnapshot, required_free_vram_bytes: u64) -> GpuQualification {
    match admit(snapshot) {
        GpuQualification::Admissible { device } => {
            if device.free_vram_bytes < required_free_vram_bytes {
                GpuQualification::Rejected(GpuRejection::InsufficientFreeVram {
                    free: device.free_vram_bytes,
                    required: required_free_vram_bytes,
                })
            } else {
                GpuQualification::Admissible { device }
            }
        }
        // A rejection for a machine-wide reason, or an already-proven
        // qualification, is the same answer for every engine.
        decided => decided,
    }
}

/// The real probe, over NVML.
///
/// NVML ships with the Nvidia driver and is loaded at runtime, so this builds
/// and runs on machines that have never seen CUDA — it reports
/// [`GpuProbeFailure::LibraryMissing`] there instead of failing to link. That
/// matters for a GPU-only product: the build must not require the hardware it
/// gates on, or no CI machine can compile it.
#[derive(Clone, Copy, Debug, Default)]
pub struct NvmlGpuProbe;

impl GpuProbe for NvmlGpuProbe {
    fn probe(&self) -> GpuSnapshot {
        nvml_snapshot()
    }
}

#[cfg(windows)]
fn nvml_snapshot() -> GpuSnapshot {
    use nvml_wrapper::Nvml;
    use nvml_wrapper::error::NvmlError;

    let nvml = match Nvml::init() {
        Ok(nvml) => nvml,
        // Every "we could not load it" shape collapses to LibraryMissing; the
        // rest mean it loaded and then refused, which is a different problem
        // for the user to fix.
        Err(NvmlError::LibloadingError(_) | NvmlError::NotFound) => {
            return GpuSnapshot::unavailable(GpuProbeFailure::LibraryMissing);
        }
        Err(_) => return GpuSnapshot::unavailable(GpuProbeFailure::InitializationFailed),
    };
    let driver_version = nvml.sys_driver_version().ok();
    let Ok(count) = nvml.device_count() else {
        return GpuSnapshot {
            driver_version,
            devices: Vec::new(),
            unavailable: Some(GpuProbeFailure::QueryFailed),
        };
    };
    let mut devices = Vec::new();
    for index in 0..count {
        // A device that will not answer is skipped rather than failing the
        // whole probe: one unhealthy card in a multi-card box should not hide a
        // healthy one.
        let Ok(device) = nvml.device_by_index(index) else {
            continue;
        };
        let (Ok(name), Ok(capability), Ok(memory)) = (
            device.name(),
            device.cuda_compute_capability(),
            device.memory_info(),
        ) else {
            continue;
        };
        let (Ok(major), Ok(minor)) = (
            u32::try_from(capability.major),
            u32::try_from(capability.minor),
        ) else {
            continue;
        };
        devices.push(CudaDevice {
            name,
            compute_capability: ComputeCapability { major, minor },
            total_vram_bytes: memory.total,
            free_vram_bytes: memory.free,
        });
    }
    GpuSnapshot {
        driver_version,
        devices,
        unavailable: None,
    }
}

#[cfg(not(windows))]
fn nvml_snapshot() -> GpuSnapshot {
    GpuSnapshot::unavailable(GpuProbeFailure::LibraryMissing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(major: u32, minor: u32) -> CudaDevice {
        CudaDevice {
            name: format!("Synthetic {major}.{minor}"),
            compute_capability: ComputeCapability { major, minor },
            total_vram_bytes: 8 * 1024 * 1024 * 1024,
            free_vram_bytes: 7 * 1024 * 1024 * 1024,
        }
    }

    fn present(devices: Vec<CudaDevice>) -> GpuSnapshot {
        GpuSnapshot {
            driver_version: Some("610.74".to_owned()),
            devices,
            unavailable: None,
        }
    }

    /// A card with a known amount free, so VRAM is the only thing under test.
    fn device_with_free_vram(free_bytes: u64) -> CudaDevice {
        CudaDevice {
            free_vram_bytes: free_bytes,
            ..device(8, 9)
        }
    }

    #[test]
    fn a_card_too_small_for_the_engine_is_rejected_by_size_not_by_capability() {
        // This was `one_engine_can_fit_while_the_other_does_not`, and it made
        // the case for two independent verdicts: same card, same instant, two
        // answers. There is one engine now, so what is left to pin is the part
        // that was never about the split — a card that passes every
        // machine-wide check and still cannot hold the model is rejected for
        // its free VRAM, and says so precisely.
        let four_gb = 4 * 1024 * 1024 * 1024;
        let snapshot = present(vec![device_with_free_vram(four_gb)]);

        assert!(matches!(
            admit_engine(&snapshot, 2 * 1024 * 1024 * 1024),
            GpuQualification::Admissible { .. }
        ));

        let rejected = admit_engine(&snapshot, 6 * 1024 * 1024 * 1024);
        assert_eq!(
            rejected,
            GpuQualification::Rejected(GpuRejection::InsufficientFreeVram {
                free: four_gb,
                required: 6 * 1024 * 1024 * 1024,
            })
        );
        assert_eq!(rejected.preferred_provider(), ExecutionProvider::Cpu);
    }

    #[test]
    fn fitting_in_vram_never_means_the_gpu_has_been_proven() {
        // The distinction the whole module exists to keep: passing a size check
        // is still only permission to attempt the execution test.
        let snapshot = present(vec![device_with_free_vram(8 * 1024 * 1024 * 1024)]);

        let qualification = admit_engine(&snapshot, 1024);

        assert!(!qualification.is_qualified());
        assert_eq!(qualification.code(), "admissible_execution_untested");
    }

    #[test]
    fn a_machine_wide_rejection_ignores_the_size_it_was_asked_about() {
        // No card at all is not a question about model size, and the answer
        // must not be dressed up as one — a "needs more VRAM" message on a
        // machine with no NVIDIA driver sends the user shopping for hardware
        // they already have.
        let snapshot = GpuSnapshot::unavailable(GpuProbeFailure::LibraryMissing);

        let huge = admit_engine(&snapshot, 64 * 1024 * 1024 * 1024);
        assert_eq!(huge, admit_engine(&snapshot, 0));
        assert_eq!(huge.code(), "nvidia_management_library_missing");
    }

    #[test]
    fn an_insufficient_vram_code_carries_no_device_name() {
        // This string reaches the diagnostic log, which is a privacy surface.
        let snapshot = present(vec![device_with_free_vram(1024)]);

        let code = admit_engine(&snapshot, 2048).code();

        assert_eq!(code, "insufficient_free_vram_1024_below_required_2048");
        assert!(!code.contains("Synthetic"));
    }

    #[test]
    fn a_machine_without_nvml_is_rejected_as_a_driver_problem_not_a_missing_card() {
        let rejected = admit(&GpuSnapshot::unavailable(GpuProbeFailure::LibraryMissing));

        assert_eq!(
            rejected,
            GpuQualification::Rejected(GpuRejection::ProbeUnavailable(
                GpuProbeFailure::LibraryMissing
            ))
        );
        assert!(!rejected.is_qualified());
        assert_eq!(rejected.code(), "nvidia_management_library_missing");
    }

    #[test]
    fn nvml_that_answers_with_no_devices_is_a_distinct_rejection() {
        // An AMD-only machine with the Nvidia driver uninstalled reports the
        // missing library; this is the odder case of NVML present and
        // enumerating nothing, and it must not be reported as a driver fault.
        let rejected = admit(&present(Vec::new()));

        assert_eq!(
            rejected,
            GpuQualification::Rejected(GpuRejection::NoCudaDevice)
        );
        assert_eq!(rejected.code(), "no_cuda_device");
    }

    #[test]
    fn an_admissible_card_selects_cuda_packs_without_claiming_the_gpu_works() {
        // The two halves of this are the whole point. A machine with a usable
        // card fetches CUDA artifacts, because otherwise it could never obtain
        // the pack that would let anything be qualified. It still reports
        // unqualified, because nothing has run.
        let admissible = admit(&present(vec![device(12, 0)]));

        assert_eq!(admissible.preferred_provider(), ExecutionProvider::Cuda);
        assert!(!admissible.is_qualified());
    }

    #[test]
    fn every_rejection_falls_back_to_cpu_rather_than_locking_the_user_out() {
        // The migration began as GPU-only and stopped being so once CPU int8
        // measured viable. No card, no driver, and too old a card must all land
        // on the CPU pack, not on a dead end.
        for rejected in [
            admit(&GpuSnapshot::unavailable(GpuProbeFailure::LibraryMissing)),
            admit(&present(Vec::new())),
            admit(&present(vec![device(7, 5)])),
        ] {
            assert_eq!(rejected.preferred_provider(), ExecutionProvider::Cpu);
        }
    }

    #[test]
    fn a_card_below_the_floor_is_rejected_and_the_code_names_both_capabilities() {
        // 7.5 is Turing — the 2000 series, one generation below the target.
        let rejected = admit(&present(vec![device(7, 5)]));

        assert_eq!(
            rejected,
            GpuQualification::Rejected(GpuRejection::ComputeCapabilityTooLow {
                best: ComputeCapability { major: 7, minor: 5 },
                minimum: MINIMUM_COMPUTE_CAPABILITY,
            })
        );
        assert_eq!(rejected.code(), "compute_capability_7.5_below_minimum_8.6");
    }

    #[test]
    fn each_targeted_generation_clears_the_floor() {
        // Ampere, Ada, Blackwell — the 3000, 4000 and 5000 series. Only the
        // last has been run; the other two are admitted on this floor alone,
        // which is exactly why admission is not qualification.
        for (major, minor) in [(8, 6), (8, 9), (12, 0)] {
            let admitted = admit(&present(vec![device(major, minor)]));

            assert!(
                matches!(admitted, GpuQualification::Admissible { .. }),
                "compute capability {major}.{minor} must be admissible"
            );
            assert!(!admitted.is_qualified());
        }
    }

    #[test]
    fn admission_is_never_qualification() {
        // The distinction this module exists to hold. Admissible means the
        // capability check passed and nothing has run.
        let admitted = admit(&present(vec![device(12, 0)]));

        assert_eq!(admitted.code(), "admissible_execution_untested");
        assert!(!admitted.is_qualified());
        assert!(admitted.device().is_some());
    }

    #[test]
    fn the_best_card_decides_not_the_first_enumerated() {
        // NVML orders by bus id. A box with an old card in the low slot and a
        // new one beside it must be admitted on the new one.
        let admitted = admit(&present(vec![device(7, 5), device(12, 0)]));

        assert_eq!(
            admitted.device().map(|device| device.compute_capability),
            Some(ComputeCapability {
                major: 12,
                minor: 0
            })
        );
    }

    #[test]
    fn compute_capability_orders_as_a_version_not_a_decimal() {
        assert!(
            ComputeCapability {
                major: 8,
                minor: 10
            } > ComputeCapability { major: 8, minor: 9 }
        );
        assert!(
            ComputeCapability {
                major: 12,
                minor: 0
            } > ComputeCapability { major: 8, minor: 9 }
        );
    }

    /// Prints what this machine actually reports, beside what the registry
    /// claims for the same card. Diagnostic, so it is `#[ignore]`d — it asserts
    /// nothing that holds on an arbitrary host.
    ///
    /// This is the evidence for reading VRAM from NVML rather than from
    /// [`crate::hardware`]: run it on any card above 4 GB and the two numbers
    /// disagree, with the registry pinned just under 4 GB.
    ///
    /// ```text
    /// cargo test -p speakeasy-models gpu_inventory -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "diagnostic: reports host hardware, asserts nothing"]
    fn gpu_inventory_report() {
        use crate::{HardwareProbe, SafeStandardHardwareProbe};

        let snapshot = NvmlGpuProbe.probe();
        println!("driver_version={:?}", snapshot.driver_version);
        println!("unavailable={:?}", snapshot.unavailable);
        for device in &snapshot.devices {
            println!(
                "nvml name={} capability={} total_vram={} free_vram={}",
                device.name,
                device.compute_capability,
                device.total_vram_bytes,
                device.free_vram_bytes
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

    #[test]
    fn the_real_probe_answers_without_panicking_on_any_machine() {
        // Runs in CI on machines with no Nvidia card. The assertion is only
        // that the probe reports rather than crashes; what it reports depends
        // on the host, and the decision logic is covered by the fakes above.
        let snapshot = NvmlGpuProbe.probe();

        if snapshot.unavailable.is_none() {
            assert!(
                snapshot
                    .devices
                    .iter()
                    .all(|device| device.total_vram_bytes >= device.free_vram_bytes)
            );
        }
    }
}
