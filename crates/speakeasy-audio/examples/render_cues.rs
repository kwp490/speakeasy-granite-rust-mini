//! Writes the start and stop cues to `.wav` files so they can be listened to.
//!
//! The cues are synthesised (`cue.rs`), so there is no asset in the repository
//! to open and check. This renders exactly what the app plays — the same
//! `render_cue` the audio thread calls — rather than a re-implementation that
//! could drift from it, which is the only reason this is an example and not a
//! script somewhere.
//!
//! ```text
//! cargo run -p speakeasy-audio --example render_cues -- <directory> [--play]
//! ```
//!
//! `--play` additionally sends both cues to the default output device through
//! the same `play_recording_feedback` the app calls. That path fails silently
//! by design — a machine with no output device must still record — so this is
//! the only way to find out whether it actually reaches a speaker.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use speakeasy_audio::{RecordingFeedback, play_recording_feedback, render_cue};

const RATE: u32 = 48_000;

fn main() -> std::io::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let play = arguments.iter().any(|argument| argument == "--play");
    let directory = arguments
        .iter()
        .find(|argument| !argument.starts_with("--"))
        .map_or_else(std::env::temp_dir, PathBuf::from);

    for (feedback, name) in [
        (RecordingFeedback::Started, "cue-start.wav"),
        (RecordingFeedback::Stopped, "cue-stop.wav"),
    ] {
        let path = directory.join(name);
        let samples = render_cue(feedback, RATE);
        std::fs::write(&path, wav(&samples, RATE))?;
        println!("{} ({} samples)", path.display(), samples.len());
    }

    if play {
        for (feedback, label) in [
            (RecordingFeedback::Started, "start"),
            (RecordingFeedback::Stopped, "stop"),
        ] {
            println!("playing {label}…");
            play_recording_feedback(feedback);
            // Playback is on a detached thread, so this has to outlive it or
            // the process exits mid-cue and nothing is heard. Generous on
            // purpose: a WASAPI stream can take half a second to start pulling
            // at all — see `cue_diagnostics`, which is where that was measured
            // and why the cue waits for the device rather than for a clock.
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    Ok(())
}

/// A 16-bit mono PCM WAV around `samples`.
fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|sample| {
            let clamped = sample.clamp(-1.0, 1.0) * f32::from(i16::MAX);
            #[allow(clippy::cast_possible_truncation)]
            (clamped as i16).to_le_bytes()
        })
        .collect();

    let mut out = Vec::with_capacity(44 + data.len());
    let write = |out: &mut Vec<u8>, bytes: &[u8]| out.write_all(bytes).expect("vec write");
    let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);

    write(&mut out, b"RIFF");
    write(&mut out, &(36 + data_len).to_le_bytes());
    write(&mut out, b"WAVEfmt ");
    write(&mut out, &16u32.to_le_bytes()); // PCM chunk size
    write(&mut out, &1u16.to_le_bytes()); // PCM
    write(&mut out, &1u16.to_le_bytes()); // mono
    write(&mut out, &rate.to_le_bytes());
    write(&mut out, &(rate * 2).to_le_bytes()); // byte rate
    write(&mut out, &2u16.to_le_bytes()); // block align
    write(&mut out, &16u16.to_le_bytes()); // bits per sample
    write(&mut out, b"data");
    write(&mut out, &data_len.to_le_bytes());
    write(&mut out, &data);
    out
}
