use std::error::Error;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::{Duration, Instant};

use speakeasy_audio::{
    AudioPipelineConfig, CaptureIdentity, CaptureStreamId, ChannelPolicy, CpalCaptureRequest,
    CpalCaptureSession, NativeStreamConfig, build_audio_pipeline, enumerate_input_devices,
};
use speakeasy_domain::{CorrelationId, ProducerId, SessionId};

const TARGET_RATE_HZ: u32 = 16_000;

struct Options {
    device_id: String,
    duration_seconds: u64,
    cycle_count: u32,
    cycle_ms: u64,
}

fn parse_options() -> Result<Options, &'static str> {
    let mut device_id = None;
    let mut duration_seconds = 0;
    let mut cycle_count = 0;
    let mut cycle_ms = 100;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--device-id" => device_id = Some(arguments.next().ok_or("device_id_required")?),
            "--duration-seconds" => {
                duration_seconds = arguments
                    .next()
                    .ok_or("duration_required")?
                    .parse()
                    .map_err(|_| "duration_invalid")?;
            }
            "--cycle-count" => {
                cycle_count = arguments
                    .next()
                    .ok_or("cycle_count_required")?
                    .parse()
                    .map_err(|_| "cycle_count_invalid")?;
            }
            "--cycle-ms" => {
                cycle_ms = arguments
                    .next()
                    .ok_or("cycle_ms_required")?
                    .parse()
                    .map_err(|_| "cycle_ms_invalid")?;
            }
            _ => return Err("unknown_argument"),
        }
    }
    if duration_seconds == 0 && cycle_count == 0 {
        return Err("duration_or_cycles_required");
    }
    if duration_seconds > 1_800 || cycle_count > 200 || !(20..=5_000).contains(&cycle_ms) {
        return Err("probe_bounds_invalid");
    }
    Ok(Options {
        device_id: device_id.ok_or("device_id_required")?,
        duration_seconds,
        cycle_count,
        cycle_ms,
    })
}

fn identity(generation: u64) -> CaptureIdentity {
    CaptureIdentity {
        correlation_id: CorrelationId::from_bytes([0x41; 16]),
        session_id: SessionId::from_bytes([0x42; 16]),
        producer_id: ProducerId::from_bytes([0x43; 16]),
        stream_id: CaptureStreamId::from_bytes([0x44; 16]),
        generation,
    }
}

fn native_config(device_id: &str) -> Result<Option<NativeStreamConfig>, Box<dyn Error>> {
    Ok(enumerate_input_devices()?
        .into_iter()
        .find(|device| device.stable_id == device_id)
        .and_then(|device| device.default_config))
}

fn pipeline_config(native: NativeStreamConfig, generation: u64) -> AudioPipelineConfig {
    AudioPipelineConfig {
        stream_id: identity(generation).stream_id,
        native,
        target_sample_rate_hz: NonZeroU32::new(TARGET_RATE_HZ).expect("target rate is non-zero"),
        channel_policy: ChannelPolicy::Select { channel_index: 0 },
        callback_block_count: NonZeroUsize::new(128).expect("callback capacity is non-zero"),
        max_native_frames_per_block: NonZeroUsize::new(4_096).expect("block capacity is non-zero"),
        pre_roll_ms: 250,
        utterance_capacity_frames: NonZeroUsize::new(TARGET_RATE_HZ as usize)
            .expect("utterance capacity is non-zero"),
        max_utterance_ms: NonZeroU32::new(1_000).expect("duration is non-zero"),
        max_buffered_bytes: NonZeroUsize::new(64 * 1_024 * 1_024)
            .expect("byte capacity is non-zero"),
    }
}

fn start(
    device_id: &str,
    generation: u64,
) -> Result<Option<(CpalCaptureSession, speakeasy_audio::AudioWorker)>, Box<dyn Error>> {
    let Some(native) = native_config(device_id)? else {
        return Ok(None);
    };
    let (callback, worker) = build_audio_pipeline(pipeline_config(native, generation))
        .map_err(|error| format!("pipeline_rejected:{error:?}"))?;
    let session = CpalCaptureSession::start(
        &CpalCaptureRequest {
            identity: identity(generation),
            device_stable_id: device_id.to_owned(),
        },
        callback,
    )?;
    Ok(Some((session, worker)))
}

fn repeated_cycles(options: &Options) -> Result<(), Box<dyn Error>> {
    for cycle in 1..=options.cycle_count {
        let (mut session, mut worker) =
            start(&options.device_id, u64::from(cycle))?.ok_or("fixed_device_unavailable")?;
        let deadline = Instant::now() + Duration::from_millis(options.cycle_ms);
        while Instant::now() < deadline {
            while worker.process_next().is_some() {}
            if session.poll_fault().is_some() {
                return Err(format!("stream_fault_cycle:{cycle}").into());
            }
            std::thread::yield_now();
        }
        session.stop();
    }
    println!("probe=cycles_complete cycles={}", options.cycle_count);
    Ok(())
}

fn recovery_observation(options: &Options) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(options.duration_seconds);
    let mut generation = 1_u64;
    let initial =
        start(&options.device_id, generation)?.ok_or("fixed_device_unavailable_at_start")?;
    let mut active = Some(initial);
    let mut starts = 1_usize;
    let mut faults = 0_u32;
    let mut disappearances = 0_u32;
    println!("probe=stream_active generation={generation}");
    while Instant::now() < deadline {
        if let Some((session, worker)) = active.as_mut() {
            while worker.process_next().is_some() {}
            let faulted = session.poll_fault().is_some();
            let disappeared = native_config(&options.device_id)?.is_none();
            if faulted || disappeared {
                faults = faults.saturating_add(u32::from(faulted));
                disappearances = disappearances.saturating_add(u32::from(disappeared));
                session.stop();
                active = None;
                println!(
                    "probe=device_lost generation={generation} callback_fault={faulted} stable_id_absent={disappeared}"
                );
            }
        } else if native_config(&options.device_id)?.is_some() {
            generation = generation.saturating_add(1);
            active = start(&options.device_id, generation)?;
            if active.is_some() {
                starts = starts.saturating_add(1);
                println!("probe=device_recovered generation={generation}");
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some((session, worker)) = active.as_mut() {
        while worker.process_next().is_some() {}
        session.stop();
    }
    println!(
        "probe=observation_complete starts={starts} faults={faults} disappearances={disappearances} final_generation={generation}"
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    if options.cycle_count > 0 {
        repeated_cycles(&options)?;
    }
    if options.duration_seconds > 0 {
        recovery_observation(&options)?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("phase3_device_probe_failed={error}");
        std::process::exit(1);
    }
}
