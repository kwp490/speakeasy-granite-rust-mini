use std::path::Path;

use sysinfo::{Disks, System};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectedAdapter {
    pub name: String,
    pub driver_version: Option<String>,
    pub dedicated_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardwareSnapshot {
    pub operating_system: String,
    pub operating_system_build: Option<String>,
    pub architecture: String,
    pub physical_cores: Option<usize>,
    pub logical_processors: usize,
    pub has_avx2: bool,
    pub total_memory_bytes: Option<u64>,
    pub available_disk_bytes: Option<u64>,
    pub detected_adapters: Vec<DetectedAdapter>,
    pub limitations: Vec<String>,
}

pub trait HardwareProbe {
    fn probe(&self, install_root: &Path) -> HardwareSnapshot;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SafeStandardHardwareProbe;

impl HardwareProbe for SafeStandardHardwareProbe {
    fn probe(&self, install_root: &Path) -> HardwareSnapshot {
        let system = System::new_all();
        let disks = Disks::new_with_refreshed_list();
        let absolute_root = if install_root.is_absolute() {
            install_root.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(
                |_| install_root.to_path_buf(),
                |current| current.join(install_root),
            )
        };
        let disk = disks
            .list()
            .iter()
            .filter(|disk| absolute_root.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().components().count());
        let (operating_system_build, detected_adapters) = platform_inventory();
        HardwareSnapshot {
            operating_system: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_else(|| std::env::consts::OS.to_owned()),
            operating_system_build,
            architecture: System::cpu_arch(),
            physical_cores: System::physical_core_count(),
            logical_processors: std::thread::available_parallelism().map_or(1, usize::from),
            has_avx2: has_avx2(),
            total_memory_bytes: Some(system.total_memory()),
            available_disk_bytes: disk.map(sysinfo::Disk::available_space),
            detected_adapters,
            limitations: vec![
                "adapter detection is inventory only and never execution qualification".to_owned(),
                "provider qualification requires an exact runtime execution test".to_owned(),
            ],
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn has_avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(windows)]
fn platform_inventory() -> (Option<String>, Vec<DetectedAdapter>) {
    use std::collections::HashSet;
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let build = local_machine
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()
        .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber").ok());
    let mut adapters = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(video) = local_machine.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Video") {
        for adapter_key in video.enum_keys().filter_map(Result::ok) {
            let Ok(adapter) = video.open_subkey(format!("{adapter_key}\\0000")) else {
                continue;
            };
            let Ok(name) = adapter.get_value::<String, _>("DriverDesc") else {
                continue;
            };
            let driver_version = adapter.get_value::<String, _>("DriverVersion").ok();
            if seen.insert((name.clone(), driver_version.clone())) {
                adapters.push(DetectedAdapter {
                    name,
                    driver_version,
                    dedicated_memory_bytes: adapter
                        .get_value::<u64, _>("HardwareInformation.MemorySize")
                        .ok(),
                });
            }
        }
    }
    (build, adapters)
}

#[cfg(not(windows))]
fn platform_inventory() -> (Option<String>, Vec<DetectedAdapter>) {
    (System::kernel_version(), Vec::new())
}

#[cfg(not(target_arch = "x86_64"))]
const fn has_avx2() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_probe_never_promotes_detected_hardware_to_qualification() {
        let snapshot = SafeStandardHardwareProbe.probe(Path::new("C:\\"));
        assert!(!snapshot.operating_system.is_empty());
        assert!(!snapshot.architecture.is_empty());
        assert!(snapshot.logical_processors > 0);
        assert!(snapshot.total_memory_bytes.is_some_and(|bytes| bytes > 0));
        assert!(
            snapshot
                .limitations
                .iter()
                .any(|item| item.contains("never execution qualification"))
        );
    }
}
