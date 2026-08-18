//! Reports every step of the cue playback path, loudly.
//!
//! `play_recording_feedback` drops all of its errors on purpose — a machine
//! with no output device must still record — which means a cue that never
//! sounds is indistinguishable from one that played. This walks the same
//! sequence and prints what happened at each step.
//!
//! ```text
//! cargo run -p speakeasy-audio --example cue_diagnostics
//! ```

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn main() {
    let host = cpal::default_host();
    println!("host: {:?}", host.id());

    match host.output_devices() {
        Ok(devices) => {
            for device in devices {
                println!("  output device: {device}");
            }
        }
        Err(error) => println!("  output_devices failed: {error}"),
    }

    let Some(device) = host.default_output_device() else {
        println!("FAIL: no default output device — the cue can never sound");
        return;
    };
    println!("default output device: {device}");

    let supported = match device.default_output_config() {
        Ok(supported) => supported,
        Err(error) => {
            println!("FAIL: default_output_config: {error}");
            return;
        }
    };
    println!(
        "default config: {:?} {} Hz, {} channel(s)",
        supported.sample_format(),
        supported.sample_rate(),
        supported.channels(),
    );
    // The shipping code renders only F32 and I16 and returns silently for
    // anything else. If this line says otherwise, that is the bug.
    println!(
        "rendered by the shipping code: {}",
        matches!(
            supported.sample_format(),
            cpal::SampleFormat::F32 | cpal::SampleFormat::I16
        )
    );

    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let samples = speakeasy_audio::render_cue(speakeasy_audio::RecordingFeedback::Started, rate);
    println!("rendered {} samples at {rate} Hz", samples.len());
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    println!("peak amplitude: {peak:.3}");

    let total = samples.len();
    let taken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&taken);
    let mut cursor = 0usize;
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in output.chunks_mut(channels) {
                    let sample = samples.get(cursor).copied().unwrap_or(0.0);
                    if cursor < samples.len() {
                        cursor += 1;
                        counter.store(cursor, std::sync::atomic::Ordering::Release);
                    }
                    for slot in frame {
                        *slot = sample;
                    }
                }
            },
            |error| println!("STREAM ERROR: {error}"),
            None,
        ),
        other => {
            println!("(diagnostic only renders F32; device wants {other:?})");
            return;
        }
    };

    let stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            println!("FAIL: build_output_stream: {error}");
            return;
        }
    };
    println!("stream built");
    let started = std::time::Instant::now();
    if let Err(error) = stream.play() {
        println!("FAIL: play: {error}");
        return;
    }
    println!("play() ok — the device callback should now be pulling samples");

    // The measurement that mattered. The first shipped version slept the cue's
    // own duration plus a small margin and then dropped the stream; on this
    // machine the device had taken well under half the cue by then, so it was
    // cut off mid-note every time without erroring.
    let mut milestone = Duration::from_millis(0);
    for step in 1..=40 {
        std::thread::sleep(Duration::from_millis(50));
        let progress = taken.load(std::sync::atomic::Ordering::Acquire);
        if step % 4 == 0 || progress == total {
            println!(
                "  +{:>4}ms  {progress}/{total} samples taken",
                started.elapsed().as_millis()
            );
        }
        if progress == total {
            milestone = started.elapsed();
            break;
        }
    }
    drop(stream);

    let progress = taken.load(std::sync::atomic::Ordering::Acquire);
    if progress == total {
        println!(
            "PASS: device took the whole cue after {}ms",
            milestone.as_millis()
        );
        println!(
            "      a fixed sleep of {}ms (the cue's own length) would have cut it off",
            (total as f64 / f64::from(rate) * 1000.0).round(),
        );
    } else {
        println!("FAIL: device took only {progress} of {total} samples");
    }
}
