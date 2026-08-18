use std::error::Error;
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use speakeasy_audio::{
    AudioPipelineConfig, CaptureIdentity, CaptureStreamId, ChannelPolicy, CpalCaptureRequest,
    CpalCaptureSession, InputDeviceDescriptor, NativeStreamConfig, build_audio_pipeline,
    enumerate_input_devices,
};
use speakeasy_domain::{CorrelationId, ProducerId, SessionId};
use speakeasy_windows::{DiagnosticWavConsent, DiagnosticWavPolicy, save_diagnostic_wav};

const TARGET_RATE_HZ: u32 = 16_000;

#[derive(Debug, Default)]
struct Options {
    device_id: Option<String>,
    capture_seconds: Option<u32>,
    output: Option<PathBuf>,
    consent: bool,
}

fn parse_options() -> Result<Options, &'static str> {
    let mut options = Options::default();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--device-id" => {
                options.device_id = Some(arguments.next().ok_or("device_id_required")?);
            }
            "--capture-seconds" => {
                let seconds = arguments
                    .next()
                    .ok_or("capture_seconds_required")?
                    .parse::<u32>()
                    .map_err(|_| "capture_seconds_invalid")?;
                if !(1..=30).contains(&seconds) {
                    return Err("capture_seconds_out_of_range");
                }
                options.capture_seconds = Some(seconds);
            }
            "--output" => {
                options.output = Some(PathBuf::from(arguments.next().ok_or("output_required")?));
            }
            "--consent-to-diagnostic-wav" => options.consent = true,
            _ => return Err("unknown_argument"),
        }
    }
    let capture_fields = [
        options.device_id.is_some(),
        options.capture_seconds.is_some(),
        options.output.is_some(),
        options.consent,
    ];
    if capture_fields.iter().any(|value| *value) && !capture_fields.iter().all(|value| *value) {
        return Err("capture_requires_device_duration_output_and_consent");
    }
    Ok(options)
}

fn print_devices(devices: &[InputDeviceDescriptor]) {
    for device in devices {
        println!(
            "device_id={}\tdefault={}\tname={}\tconfig={:?}",
            device.stable_id, device.is_default, device.display_name, device.default_config
        );
    }
}

fn capture(options: &Options, devices: &[InputDeviceDescriptor]) -> Result<(), Box<dyn Error>> {
    let device_id = options.device_id.as_deref().ok_or("device_id_required")?;
    let descriptor = devices
        .iter()
        .find(|device| device.stable_id == device_id)
        .ok_or("selected_device_unavailable")?;
    let native = descriptor
        .default_config
        .ok_or("selected_device_format_unsupported")?;
    let seconds = options.capture_seconds.ok_or("capture_seconds_required")?;
    let identity = CaptureIdentity {
        correlation_id: CorrelationId::from_bytes([0x31; 16]),
        session_id: SessionId::from_bytes([0x32; 16]),
        producer_id: ProducerId::from_bytes([0x33; 16]),
        stream_id: CaptureStreamId::from_bytes([0x34; 16]),
        generation: 1,
    };
    let (callback, mut worker) = build_audio_pipeline(pipeline_config(native, identity, seconds)?)
        .map_err(|error| format!("pipeline_rejected:{error:?}"))?;
    worker
        .begin_utterance()
        .map_err(|_| "utterance_begin_rejected")?;
    let mut session = CpalCaptureSession::start(
        &CpalCaptureRequest {
            identity,
            device_stable_id: device_id.to_owned(),
        },
        callback,
    )?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(seconds));
    while Instant::now() < deadline {
        while worker.process_next().is_some() {}
        if session.poll_fault().is_some() {
            return Err("capture_device_fault".into());
        }
        std::thread::yield_now();
    }
    session.stop();
    let completion = worker
        .finish_utterance()
        .map_err(|_| "utterance_finish_rejected")?;
    let samples = worker.utterance_samples();
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let sum_squares = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>();
    let sample_count = u32::try_from(samples.len()).map_err(|_| "sample_count_unrepresentable")?;
    let rms = if samples.is_empty() {
        0.0
    } else {
        (sum_squares / f64::from(sample_count)).sqrt()
    };
    let output = options.output.as_ref().ok_or("output_required")?;
    let consent = DiagnosticWavConsent::after_disclosure(output, options.consent)?;
    let saved = save_diagnostic_wav(
        DiagnosticWavPolicy { enabled: true },
        consent,
        NonZeroU32::new(TARGET_RATE_HZ).expect("target rate is non-zero"),
        samples,
    )?;
    println!(
        "capture=complete device_name={} native={:?} target_rate_hz={} frames={} drained_blocks={} issues={:?} peak={peak:.6} rms={rms:.6} output_name={}",
        descriptor.display_name,
        session.native_config(),
        TARGET_RATE_HZ,
        completion.frames_buffered,
        completion.drained_blocks,
        completion.issues,
        saved
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("private.wav")
    );
    Ok(())
}

fn pipeline_config(
    native: NativeStreamConfig,
    identity: CaptureIdentity,
    seconds: u32,
) -> Result<AudioPipelineConfig, &'static str> {
    let target_frames = usize::try_from(TARGET_RATE_HZ)
        .map_err(|_| "target_rate_unrepresentable")?
        .checked_mul(usize::try_from(seconds).map_err(|_| "duration_unrepresentable")? + 1)
        .ok_or("capture_capacity_overflow")?;
    Ok(AudioPipelineConfig {
        stream_id: identity.stream_id,
        native,
        target_sample_rate_hz: NonZeroU32::new(TARGET_RATE_HZ).expect("target rate is non-zero"),
        channel_policy: ChannelPolicy::Select { channel_index: 0 },
        callback_block_count: NonZeroUsize::new(128).expect("callback capacity is non-zero"),
        max_native_frames_per_block: NonZeroUsize::new(4_096).expect("block capacity is non-zero"),
        pre_roll_ms: 250,
        utterance_capacity_frames: NonZeroUsize::new(target_frames)
            .ok_or("utterance_capacity_zero")?,
        max_utterance_ms: NonZeroU32::new(seconds.saturating_add(1).saturating_mul(1_000))
            .ok_or("duration_zero")?,
        max_buffered_bytes: NonZeroUsize::new(64 * 1_024 * 1_024)
            .expect("byte capacity is non-zero"),
    })
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let devices = enumerate_input_devices()?;
    print_devices(&devices);
    if options.capture_seconds.is_some() {
        capture(&options, &devices)?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("phase3_capture_failed={error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_requires_every_explicit_consent_field() {
        let options = Options {
            device_id: Some("fixture".to_owned()),
            capture_seconds: Some(5),
            output: None,
            consent: true,
        };
        let fields = [
            options.device_id.is_some(),
            options.capture_seconds.is_some(),
            options.output.is_some(),
            options.consent,
        ];
        assert!(fields.iter().any(|value| *value));
        assert!(!fields.iter().all(|value| *value));
    }
}
