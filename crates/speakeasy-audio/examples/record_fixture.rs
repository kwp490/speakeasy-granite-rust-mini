//! Records a speech fixture from the microphone, through this app's own
//! capture pipeline.
//!
//! # Why this exists beside `scripts/New-SmokeFixture.ps1`
//!
//! That script *synthesises* a clip, and says plainly what it gives up:
//! synthesised speech has no room, no breath and no clipping, so it can prove
//! the model reads audio and cannot answer anything about a real voice. Some
//! questions need a real voice — whether the recogniser hears `LogicMonitor` as
//! one word is one of them, and no synthesiser can be trusted to have said it
//! the way a person would.
//!
//! This repository has lost a recorded fixture twice, both times because the
//! only copy lived in gitignored `.tools/` on the machine that made it and
//! there was no way to make another. The lesson recorded then was that a
//! hardware test whose input cannot be obtained is not a test that is hard to
//! run, it is a test that is gone.
//!
//! The answer to that is **not** to commit the recording. A recording made here
//! is a person's voice, this is a public repository, and a voice cannot be
//! un-published. So the *recorder* is committed and its output never is
//! (owner decision, 2026-08-27; `.gitignore` carries the rule and the reason).
//! What was missing both times was the capability, not the file — with this
//! here, a lost clip costs one reading rather than the whole proof.
//!
//! # Why it goes through the real pipeline
//!
//! It builds `build_audio_pipeline` against the device's own native format and
//! reads the retained utterance back out, which is exactly what
//! `capture_wizard.rs` does for a dictation. So the bytes this writes are the
//! bytes Granite would have been handed — same channel selection, same
//! resampler, same 16 kHz mono target. A fixture recorded some other way would
//! measure that other way's resampler as much as it measures the model.
//!
//! ```text
//! cargo run -p speakeasy-audio --example record_fixture -- <destination.wav>
//! ```
//!
//! Press Enter to begin, and Enter again to stop. Nothing is written if no
//! audio arrived, because an empty WAV that a later test reads as "the model
//! transcribed silence" is the failure this whole file exists to avoid.

use std::io::{Read, Write};
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use speakeasy_audio::{
    AudioPipelineConfig, CaptureIdentity, ChannelPolicy, CpalCaptureRequest, CpalCaptureSession,
    UtteranceIssues, build_audio_pipeline, enumerate_input_devices,
};
use speakeasy_domain::{CaptureStreamId, CorrelationId, ProducerId, SessionId};

/// What Granite's encoder consumes, and what `capture_wizard.rs` targets.
const TARGET_RATE_HZ: u32 = 16_000;

/// Generous, because this is a person reading a page rather than dictating a
/// sentence. Well past the 120 s the product's own ceiling allows — a fixture
/// longer than that simply cannot be replayed through the worker, which is a
/// thing to discover here rather than at transcription time.
const MAX_SECONDS: u32 = 300;

#[allow(clippy::too_many_lines)]
fn main() {
    let Some(destination) = std::env::args().nth(1) else {
        eprintln!("usage: record_fixture <destination.wav>");
        std::process::exit(2);
    };
    let destination = Path::new(&destination);

    let devices = match enumerate_input_devices() {
        Ok(devices) => devices,
        Err(error) => {
            eprintln!("FAIL: could not enumerate input devices: {error:?}");
            std::process::exit(1);
        }
    };
    // The default device, and it is named rather than assumed: this machine's
    // default input has been a virtual device before now, and a fixture
    // recorded off the wrong one is indistinguishable from a quiet room.
    let Some(device) = devices
        .iter()
        .find(|device| device.is_default && device.default_config.is_some())
    else {
        eprintln!("FAIL: no default input device offers a usable configuration");
        std::process::exit(1);
    };
    let Some(native) = device.default_config else {
        eprintln!("FAIL: the default device reports no native configuration");
        std::process::exit(1);
    };
    println!("device : {}", device.display_name);
    println!("native : {native:?}");

    let identity = identity();
    let config = match pipeline_config(native, identity) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("FAIL: {error}");
            std::process::exit(1);
        }
    };
    let (callback, mut worker) = match build_audio_pipeline(config) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("FAIL: the pipeline was rejected: {error:?}");
            std::process::exit(1);
        }
    };

    println!();
    println!("Press Enter to start recording.");
    wait_for_enter();

    if let Err(error) = worker.begin_utterance() {
        eprintln!("FAIL: the utterance was rejected: {error:?}");
        std::process::exit(1);
    }
    let mut session = match CpalCaptureSession::start(
        &CpalCaptureRequest {
            identity,
            device_stable_id: device.stable_id.clone(),
        },
        callback,
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("FAIL: the capture stream would not start: {error:?}");
            std::process::exit(1);
        }
    };

    println!("RECORDING — read the script, then press Enter to stop.");
    let stop = Arc::new(AtomicBool::new(false));
    spawn_enter_watcher(&stop);

    let started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(MAX_SECONDS));
    let mut last_report = Instant::now();
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        // Drained every pass, not at the end. The callback writes into a
        // bounded ring; a consumer that waits until the user stops talking
        // overflows it and the recording loses its middle, silently, with only
        // `CALLBACK_QUEUE_OVERFLOW` to say so afterwards.
        while worker.process_next().is_some() {}
        if session.poll_fault().is_some() {
            eprintln!("FAIL: the capture device faulted mid-recording");
            std::process::exit(1);
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            print!("  {} s\r", started.elapsed().as_secs());
            let _ = std::io::stdout().flush();
            last_report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    session.stop();

    let completion = match worker.finish_utterance() {
        Ok(completion) => completion,
        Err(error) => {
            eprintln!("FAIL: the utterance would not finish: {error:?}");
            std::process::exit(1);
        }
    };

    let samples = worker.utterance_samples();
    println!();
    println!("frames : {}", completion.frames_buffered);
    report_issues(completion.issues);

    // `frames_buffered == 0` is the one condition that means there is nothing
    // to transcribe. Everything else annotates audio that exists.
    if samples.is_empty() {
        eprintln!("FAIL: no audio was captured — nothing written");
        std::process::exit(1);
    }
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    println!("peak   : {peak:.3}");
    if peak < 0.01 {
        eprintln!("FAIL: the recording is effectively silent — nothing written");
        std::process::exit(1);
    }

    match write_wave(destination, samples) {
        Ok(bytes) => {
            let seconds = seconds_of(samples.len());
            println!(
                "wrote  : {} ({bytes} bytes, {seconds:.2} s)",
                destination.display()
            );
            if seconds > 120.0 {
                println!(
                    "NOTE   : longer than the product's own 120 s ceiling, so this clip \
                     cannot be replayed through granite-worker."
                );
            }
        }
        Err(error) => {
            eprintln!("FAIL: could not write {}: {error}", destination.display());
            std::process::exit(1);
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn seconds_of(frames: usize) -> f64 {
    frames as f64 / f64::from(TARGET_RATE_HZ)
}

/// Names every annotation the pipeline raised rather than reducing them to one
/// code. A recording made for measurement is worth keeping even when it is
/// imperfect — but only if whoever measures it knows what it is.
fn report_issues(issues: UtteranceIssues) {
    for (flag, label) in [
        (UtteranceIssues::DURATION_LIMIT, "duration limit reached"),
        (UtteranceIssues::BYTE_LIMIT, "byte limit reached"),
        (UtteranceIssues::BUFFER_CAPACITY, "buffer capacity reached"),
        (
            UtteranceIssues::CALLBACK_QUEUE_OVERFLOW,
            "callback queue overflowed — audio was dropped",
        ),
        (UtteranceIssues::DISCONTINUITY, "a discontinuity was seen"),
    ] {
        if issues.contains(flag) {
            println!("ISSUE  : {label}");
        }
    }
}

fn wait_for_enter() {
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

/// Watches stdin on its own thread so the drain loop above never blocks.
///
/// Reads one byte rather than a line: the drain loop has to keep running while
/// this waits, and a blocking `read_line` on the recording thread would stall
/// the consumer and overflow the callback ring.
fn spawn_enter_watcher(stop: &Arc<AtomicBool>) {
    let stop = Arc::clone(stop);
    std::thread::spawn(move || {
        let mut byte = [0_u8; 1];
        while std::io::stdin().read_exact(&mut byte).is_ok() {
            if byte[0] == b'\n' {
                break;
            }
        }
        stop.store(true, Ordering::Release);
    });
}

/// Fixed rather than random. This clip is not a dictation and nothing
/// correlates it with anything; a generated identity would only suggest it did.
fn identity() -> CaptureIdentity {
    let bytes = [0x7a_u8; 16];
    CaptureIdentity {
        correlation_id: CorrelationId::from_bytes(bytes),
        session_id: SessionId::from_bytes(bytes),
        producer_id: ProducerId::from_bytes(bytes.map(|byte| byte ^ 0xa2)),
        stream_id: CaptureStreamId::from_bytes(bytes.map(|byte| byte ^ 0xf3)),
        generation: 1,
    }
}

/// Mirrors `capture_wizard.rs`'s `pipeline_config`, which is the point: a
/// fixture recorded under different settings would not be the bytes a dictation
/// produces. Only the ceiling differs, and it is stated as its own constant.
fn pipeline_config(
    native: speakeasy_audio::NativeStreamConfig,
    identity: CaptureIdentity,
) -> Result<AudioPipelineConfig, &'static str> {
    let target_frames = usize::try_from(TARGET_RATE_HZ)
        .map_err(|_| "capacity overflow")?
        .checked_mul(usize::try_from(MAX_SECONDS).map_err(|_| "capacity overflow")? + 1)
        .ok_or("capacity overflow")?;
    Ok(AudioPipelineConfig {
        stream_id: identity.stream_id,
        native,
        target_sample_rate_hz: NonZeroU32::new(TARGET_RATE_HZ).ok_or("target rate is zero")?,
        channel_policy: ChannelPolicy::Select { channel_index: 0 },
        callback_block_count: NonZeroUsize::new(128).ok_or("callback capacity is zero")?,
        max_native_frames_per_block: NonZeroUsize::new(4_096).ok_or("block capacity is zero")?,
        pre_roll_ms: 250,
        utterance_capacity_frames: NonZeroUsize::new(target_frames).ok_or("capacity overflow")?,
        max_utterance_ms: NonZeroU32::new((MAX_SECONDS + 1) * 1_000).ok_or("duration is zero")?,
        // 36 bytes per retained frame, so 300 s needs ~165 MiB. Sized from that
        // arithmetic rather than copied from the app's 128 MiB, which is
        // correct for a 120 s ceiling and would silently truncate this one --
        // the exact defect that discarded whole two-minute dictations here.
        max_buffered_bytes: NonZeroUsize::new(320 * 1_024 * 1_024)
            .ok_or("byte capacity is zero")?,
    })
}

/// A 16 kHz mono PCM16 RIFF/WAVE file: the one shape every fixture reader in
/// this repository understands.
fn write_wave(path: &Path, samples: &[f32]) -> std::io::Result<usize> {
    let data_bytes = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_bytes);
    let riff_size = u32::try_from(36 + data_bytes).unwrap_or(u32::MAX);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // mono
    out.extend_from_slice(&TARGET_RATE_HZ.to_le_bytes());
    out.extend_from_slice(&(TARGET_RATE_HZ * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2_u16.to_le_bytes()); // block align
    out.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data_bytes).unwrap_or(u32::MAX).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&quantize(*sample).to_le_bytes());
    }
    std::fs::write(path, &out)?;
    Ok(out.len())
}

/// The same conversion `capture_wizard.rs` uses for its live tap, so a fixture
/// and a dictation quantize identically.
#[allow(clippy::cast_possible_truncation)]
fn quantize(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}
