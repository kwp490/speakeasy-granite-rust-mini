use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use speakeasy_domain::CancelToken;
use speakeasy_models::{
    DownloadError, DownloadPolicy, DownloadRequest, HardwareProbe, InstallError, InstallManager,
    InstallSpec, SafeStandardHardwareProbe, bundled_manifest, download_to_file,
};

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/phase2-current-host");
    let archive_path = root.join("downloads/nemotron-3.5-streaming-en-cpu.tar.bz2");
    let models_root = root.join("models");
    fs::create_dir_all(archive_path.parent().expect("download parent"))?;

    let manifest = bundled_manifest()?;
    let pack = manifest
        .packs()
        .iter()
        .find(|pack| pack.id() == "nemotron-3.5-streaming-en-cpu")
        .ok_or("admitted CPU streaming pack is absent")?;
    if !pack.is_install_eligible() {
        return Err("current-host proof requires an install-admitted pack".into());
    }
    let spec = InstallSpec::from(pack);
    let manager = InstallManager::new(&models_root);
    let hardware = SafeStandardHardwareProbe.probe(&models_root);
    let required_disk = manager.preflight_disk(&spec)?;
    println!(
        "inventory: os={} build={:?} arch={} physical={:?} logical={} ram={:?} disk={:?} adapters={}",
        hardware.operating_system,
        hardware.operating_system_build,
        hardware.architecture,
        hardware.physical_cores,
        hardware.logical_processors,
        hardware.total_memory_bytes,
        hardware.available_disk_bytes,
        hardware.detected_adapters.len()
    );
    println!("disk preflight requirement: {required_disk} bytes");

    if !archive_path.exists() {
        let archive = pack
            .archive()
            .expect("this proof needs an archive-based pack");
        let request = DownloadRequest {
            url: archive
                .url()
                .expect("this proof needs a downloadable pack")
                .to_owned(),
            destination: archive_path.clone(),
            expected_bytes: archive.bytes(),
            expected_sha256: archive.sha256().to_owned(),
        };
        let policy = DownloadPolicy {
            redirect_hosts: vec![
                "github.com".to_owned(),
                "release-assets.githubusercontent.com".to_owned(),
            ],
            connect_deadline_ms: 30_000,
            read_deadline_ms: 120_000,
            overall_deadline_ms: 1_800_000,
            maximum_retries: 3,
            proxy_aware: true,
        };
        let interrupted = CancelToken::default();
        let cancellation_signal = interrupted.clone();
        let thread = thread::spawn(move || {
            thread::sleep(Duration::from_secs(1));
            cancellation_signal.cancel();
        });
        match download_to_file(&request, &policy, &interrupted) {
            Err(DownloadError::Cancelled) => {
                println!("interruption proof: cancelled with partial state");
            }
            Ok(_) => println!("interruption proof: transfer completed before cancellation"),
            Err(error) => return Err(error.into()),
        }
        thread.join().map_err(|_| "interruption thread panicked")?;
        if !archive_path.exists() {
            let result = download_to_file(&request, &policy, &CancelToken::default())?;
            println!(
                "download verified: bytes={} resumed={}",
                result.bytes, result.resumed
            );
        }
    }

    let installed = manager.install_archive(&spec, &archive_path, &CancelToken::default())?;
    manager.reverify(&spec)?;
    manager.install_archive(&spec, &archive_path, &CancelToken::default())?;
    manager.reverify(&spec)?;
    let lease = manager.lease(&spec)?;
    if !matches!(manager.delete(&spec), Err(InstallError::InUse)) {
        return Err("active lease did not block deletion".into());
    }
    drop(lease);
    println!("verified on disk: {}", installed.display());
    println!("replacement rollback path and lease-blocked deletion: verified");
    Ok(())
}
