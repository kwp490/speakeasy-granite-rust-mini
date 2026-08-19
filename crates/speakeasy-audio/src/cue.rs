//! The audible start and stop cues.
//!
//! These were two Windows system sounds — `MessageBeep(ASTERISK)` to start and
//! `MessageBeep(OK)` to stop. Both are notification chimes the OS uses for
//! unrelated things, neither says which of the two transitions just happened,
//! and the user cannot tell a dictation starting from a mail arriving.
//!
//! What replaces them is a **rising perfect fifth to start and the same fifth
//! falling to stop** (D4/A4). The interval is the point: after the octave it is
//! the most consonant in twelve-tone tuning, so it is pleasant at the volume of
//! something you hear thirty times a day, and its *direction* is the message.
//! Rising opens, falling resolves — the same convention a doorbell, a phone
//! unlocking, and every UI in the world already uses, so nothing has to be
//! learned. Neither is a voice and neither is a word.
//!
//! The first version of this was reported as jarring, and the interval was not
//! why. It sat a fifth higher (440/659) and reached full amplitude in 15ms,
//! which is a *struck* tone — and the ear hears a struck tone as an alert
//! however consonant it is. Same notes, same 245ms: lower register, a ~48ms
//! swell instead of a strike, a gentler decay and less level (see `ATTACK`).
//!
//! Synthesised rather than shipped as `.wav` assets: pure sine tones with a
//! soft envelope are a few lines of arithmetic, the response is unit-testable
//! in a way a binary blob is not, and there is no file to lose or to fail to
//! install. It goes through `cpal`, which this crate already owns for capture,
//! because `unsafe_code = "forbid"` rules out calling `PlaySound` directly and
//! `winsafe` — the app's Windows wrapper — does not bind `winmm` at all.
//!
//! A cue never blocks or fails the transition it describes. Playback happens on
//! its own thread and every error is dropped: visual state stays authoritative,
//! and a machine with no output device still records.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Recording transition that receives redundant audible feedback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingFeedback {
    Started,
    Stopped,
}

/// D4 and the A4 a perfect fifth above it, in hertz.
///
/// A fifth lower than the 440/659 this started at, which was reported as
/// jarring. The interval is unchanged — its direction is the whole message —
/// but the register is not: at 660 Hz a sine reads as an alert whatever
/// envelope is on it, and the same gesture an octave down reads as warm.
///
/// Not lower still. Below roughly 250 Hz a laptop speaker reproduces the
/// envelope but not the pitch, which turns a tone into a thud, and 293 Hz is
/// comfortably clear of that on hardware that cannot be assumed.
const LOW_HZ: f32 = 293.665;
const HIGH_HZ: f32 = 440.0;

/// How long each of the two notes sounds, and how far the second overlaps the
/// first.
///
/// The overlap is what makes it one gesture rather than two beeps: the second
/// note starts while the first is still decaying, so the interval is briefly
/// heard as a chord and the movement between them is audible.
const NOTE: Duration = Duration::from_millis(150);
const OVERLAP: Duration = Duration::from_millis(55);

/// Peak amplitude of a single note, well under full scale.
///
/// Two overlapping notes sum, so the ceiling this has to respect is `2 *
/// AMPLITUDE`, not `AMPLITUDE`. At 0.16 that is 0.32 — quiet, and with enough
/// headroom that no phase alignment can clip.
const AMPLITUDE: f32 = 0.16;

/// Attack and release of one note's envelope, as a fraction of `NOTE`.
///
/// Neither is zero and that is the whole job. A sine that starts or stops at a
/// non-zero sample is a step discontinuity, and a step is a click — which is
/// the one artefact that would make this less pleasant than the system sound
/// it replaces, at any volume.
///
/// `ATTACK` was 0.10 — 15ms — and that is what made the first version jarring.
/// A tone that reaches full amplitude in 15ms is *struck*, and the ear reads a
/// struck tone as an alert no matter how consonant it is. At 0.32 the note
/// swells over ~48ms instead, which is the difference between a ping and a
/// breath. The cue's total length is unchanged; only its shape is.
const ATTACK: f32 = 0.32;
const RELEASE: f32 = 0.70;

/// How fast a note decays under its own envelope.
///
/// Lower is gentler. At 2.0 the note had dropped to an eighth of its peak by
/// the end, which reads as a pluck; at 1.1 it settles rather than snaps.
const DECAY: f32 = 1.1;

/// Plays the cue for a recording transition, on its own thread.
///
/// Returns immediately. Nothing about capture waits on audio output, and a
/// failure to play is never reported: the sound is redundant feedback and the
/// visual state is authoritative.
pub fn play_recording_feedback(feedback: RecordingFeedback) {
    // Detached deliberately. Joining would put device enumeration — which can
    // take tens of milliseconds, and longer on a machine waking a USB
    // interface — in front of the recording it is announcing.
    drop(std::thread::spawn(move || {
        play_blocking(feedback);
    }));
}

/// The two notes of a cue, in the order they sound.
const fn notes(feedback: RecordingFeedback) -> [f32; 2] {
    match feedback {
        RecordingFeedback::Started => [LOW_HZ, HIGH_HZ],
        RecordingFeedback::Stopped => [HIGH_HZ, LOW_HZ],
    }
}

/// How far into the cue the second note begins.
///
/// `saturating_sub` rather than `-`: the two constants above are the only
/// inputs and `OVERLAP` is the smaller of them, but a subtraction that can
/// panic has no business being reachable from a sound cue.
fn stride() -> Duration {
    NOTE.saturating_sub(OVERLAP)
}

/// How long a whole cue lasts.
fn cue_duration() -> Duration {
    stride().saturating_add(NOTE)
}

/// Renders a cue as mono `f32` samples at `sample_rate`.
///
/// Separate from playback so the shape of the sound is testable without an
/// audio device — see this crate's tests, which assert the direction of the
/// interval, the absence of a step at either end, and the headroom.
#[must_use]
pub fn render_cue(feedback: RecordingFeedback, sample_rate: u32) -> Vec<f32> {
    let rate = f64::from(sample_rate.max(1));
    let total = duration_samples(cue_duration(), rate);
    let note_samples = duration_samples(NOTE, rate);
    let stride = duration_samples(stride(), rate);
    let [first, second] = notes(feedback);

    (0..total)
        .map(|index| {
            voice(index, 0, note_samples, first, rate)
                + voice(index, stride, note_samples, second, rate)
        })
        .collect()
}

/// One note's contribution at `index`, silent outside its own window.
///
/// The phase is accumulated in `f64` and narrowed once at the end: at 48 kHz a
/// note is only a few thousand samples, but `f32` seconds lose enough precision
/// over a long buffer to detune the tone audibly, and this is cheap.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn voice(index: usize, start: usize, length: usize, hz: f32, rate: f64) -> f32 {
    if index < start || index >= start + length || length == 0 {
        return 0.0;
    }
    let position = index - start;
    let progress = position as f32 / length as f32;
    let seconds = (position as f64 / rate) as f32;
    AMPLITUDE * envelope(progress) * (TAU * hz * seconds).sin()
}

/// The amplitude envelope over one note, as a function of its 0–1 progress.
///
/// A raised cosine at both ends rather than a linear ramp: linear ramps leave a
/// discontinuity in the *slope*, which is audible as a faint tick at the volume
/// a cue is played at. Between them the note decays exponentially, gently
/// enough (see `DECAY`) that it settles rather than snaps, which is what keeps it
/// from sounding either like a pluck or like a test tone.
fn envelope(progress: f32) -> f32 {
    let attack = if progress < ATTACK {
        raised_cosine(progress / ATTACK)
    } else {
        1.0
    };
    let release = if progress > 1.0 - RELEASE {
        raised_cosine((1.0 - progress) / RELEASE)
    } else {
        1.0
    };
    attack * release * (-DECAY * progress).exp()
}

/// A 0–1 ramp with zero slope at both ends.
fn raised_cosine(fraction: f32) -> f32 {
    let clamped = fraction.clamp(0.0, 1.0);
    0.5 - 0.5 * (std::f32::consts::PI * clamped).cos()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn duration_samples(duration: Duration, rate: f64) -> usize {
    (duration.as_secs_f64() * rate).round().max(0.0) as usize
}

/// How long to wait past the device taking the last sample before closing the
/// stream, and the ceiling on waiting for it to take them at all.
///
/// The drain is not optional. The callback handing over the final sample means
/// the *buffer* has it, not the speaker; closing immediately after cuts the
/// tail off.
const DRAIN: Duration = Duration::from_millis(250);
const PLAYBACK_CEILING: Duration = Duration::from_secs(3);

/// The cue's samples, handed to the device one frame at a time.
///
/// A struct rather than a closure because the two sample-format branches below
/// each need their own `move`, and a closure cannot be written once and moved
/// into both.
struct CuePlayback {
    samples: Vec<f32>,
    cursor: usize,
    drained: Arc<AtomicBool>,
}

impl CuePlayback {
    /// The next sample, or silence once the cue has been handed over in full.
    ///
    /// Flipping `drained` here — when the device asks for a sample past the end
    /// — is what tells the playing thread the cue actually left, as opposed to
    /// a wall-clock guess about when it should have.
    fn next_sample(&mut self) -> f32 {
        let Some(sample) = self.samples.get(self.cursor) else {
            self.drained.store(true, Ordering::Release);
            return 0.0;
        };
        self.cursor += 1;
        *sample
    }
}

/// Opens the default output device, plays the cue and closes it again.
///
/// Every failure path is a silent return. The catalogue of things that can go
/// wrong here — no host, no default device, a config the device will not
/// accept, a sample format this does not render — are all conditions under
/// which the *recording* is still fine, and none of them is worth a dialog.
fn play_blocking(feedback: RecordingFeedback) {
    let Some(device) = cpal::default_host().default_output_device() else {
        return;
    };
    let Ok(supported) = device.default_output_config() else {
        return;
    };
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    if channels == 0 {
        return;
    }

    let samples = render_cue(feedback, config.sample_rate);
    if samples.is_empty() {
        return;
    }

    let drained = Arc::new(AtomicBool::new(false));
    // Mono rendered once and fanned out to every channel: a cue has no stereo
    // information to carry, and writing it to one channel only would put it in
    // one ear — or, on the 8-channel device this was found on, into a surround
    // channel nothing is listening to.
    let mut playback = CuePlayback {
        samples,
        cursor: 0,
        drained: Arc::clone(&drained),
    };

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in output.chunks_mut(channels) {
                    let sample = playback.next_sample();
                    for slot in frame {
                        *slot = sample;
                    }
                }
            },
            |_| {},
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |output: &mut [i16], _: &cpal::OutputCallbackInfo| {
                for frame in output.chunks_mut(channels) {
                    let sample = to_i16(playback.next_sample());
                    for slot in frame {
                        *slot = sample;
                    }
                }
            },
            |_| {},
            None,
        ),
        // Every other format the host might name. Silence rather than a
        // half-rendered cue: this is redundant feedback, and the transition it
        // describes has already happened correctly.
        _ => return,
    };
    let Ok(stream) = stream else { return };
    if stream.play().is_err() {
        return;
    }

    // Wait for the device to actually take the samples, rather than sleeping
    // for how long the cue *would* last and assuming it did.
    //
    // That assumption is what made this silent. The first version slept the
    // cue's own duration plus 120ms and then dropped the stream. Measured on
    // the machine it was reported broken on — an 8-channel 96 kHz virtual
    // device — the stream had consumed 142ms of a 245ms cue after 600ms of wall
    // clock, because a WASAPI stream does not begin pulling the instant `play`
    // returns. So the stream was destroyed around a third of the way in, every
    // time, on every dictation. Nothing errored: the callback simply stopped
    // being called, which is indistinguishable from a cue that finished.
    let deadline = Instant::now() + PLAYBACK_CEILING;
    while !drained.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(DRAIN);
    drop(stream);
}

/// Full-scale `f32` to `i16`, clamped.
#[allow(clippy::cast_possible_truncation)]
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::{
        AMPLITUDE, Arc, AtomicBool, CuePlayback, DRAIN, Duration, HIGH_HZ, LOW_HZ, Ordering,
        PLAYBACK_CEILING, RecordingFeedback, cue_duration, render_cue, to_i16,
    };

    const RATE: u32 = 48_000;

    /// Dominant frequency of a slice, by picking the strongest of the two
    /// candidate pitches with a Goertzel-style correlation. Enough to tell
    /// which of two notes is sounding, which is all these tests ask.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn correlation(samples: &[f32], hz: f32) -> f32 {
        let rate = f64::from(RATE);
        let (mut real, mut imaginary) = (0.0_f64, 0.0_f64);
        for (index, sample) in samples.iter().enumerate() {
            let angle = std::f64::consts::TAU * f64::from(hz) * (index as f64 / rate);
            real += f64::from(*sample) * angle.cos();
            imaginary += f64::from(*sample) * angle.sin();
        }
        (real.hypot(imaginary) / samples.len() as f64) as f32
    }

    #[test]
    fn the_start_cue_rises_and_the_stop_cue_falls() {
        // The whole message is the direction. If these two ever render the same
        // way round, the app says "started" when it has stopped.
        for (feedback, opening, closing) in [
            (RecordingFeedback::Started, LOW_HZ, HIGH_HZ),
            (RecordingFeedback::Stopped, HIGH_HZ, LOW_HZ),
        ] {
            let samples = render_cue(feedback, RATE);
            let head = &samples[..samples.len() / 4];
            let tail = &samples[samples.len() * 3 / 4..];
            assert!(
                correlation(head, opening) > correlation(head, closing),
                "{feedback:?} does not open on {opening} Hz",
            );
            assert!(
                correlation(tail, closing) > correlation(tail, opening),
                "{feedback:?} does not close on {closing} Hz",
            );
        }
    }

    #[test]
    fn the_two_cues_are_not_the_same_sound() {
        let started = render_cue(RecordingFeedback::Started, RATE);
        let stopped = render_cue(RecordingFeedback::Stopped, RATE);
        assert_eq!(started.len(), stopped.len());
        assert_ne!(started, stopped);
    }

    #[test]
    fn neither_cue_starts_or_ends_on_a_step() {
        // A non-zero first or last sample is a discontinuity against the
        // silence either side of it, and a discontinuity is a click — the one
        // artefact that would make this worse than the system beep it replaces.
        for feedback in [RecordingFeedback::Started, RecordingFeedback::Stopped] {
            let samples = render_cue(feedback, RATE);
            assert!(samples.first().unwrap().abs() < 1e-4);
            assert!(samples.last().unwrap().abs() < 1e-3);
            // And no step anywhere in between: at 48 kHz a 660 Hz sine at this
            // amplitude moves well under 0.02 per sample, so anything above
            // that is an envelope seam rather than the waveform.
            for pair in samples.windows(2) {
                assert!(
                    (pair[1] - pair[0]).abs() < 0.02,
                    "step of {} between samples",
                    pair[1] - pair[0],
                );
            }
        }
    }

    #[test]
    fn overlapping_notes_cannot_clip() {
        // Two notes sound together across the overlap, so the ceiling that
        // matters is their sum. Full scale is what a clamp would catch and a
        // listener would hear as a crackle.
        for feedback in [RecordingFeedback::Started, RecordingFeedback::Stopped] {
            let peak = render_cue(feedback, RATE)
                .into_iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            assert!(peak <= 2.0 * AMPLITUDE, "peak {peak} exceeded two notes");
            assert!(peak < 1.0, "peak {peak} clips");
            // And it is actually audible, rather than silence that passes every
            // other assertion here.
            assert!(peak > AMPLITUDE / 2.0, "peak {peak} is inaudibly quiet");
        }
    }

    #[test]
    fn a_cue_is_short_enough_to_stay_out_of_the_way() {
        // It plays at the moment a dictation starts, over a live microphone.
        // Long enough to be heard, short enough not to be spoken over.
        assert!(cue_duration().as_millis() <= 300);
        let samples = render_cue(RecordingFeedback::Started, RATE);
        assert_eq!(samples.len(), (RATE as usize * 245) / 1000);
    }

    #[test]
    fn an_absurd_sample_rate_still_renders_something_finite() {
        // `sample_rate` comes from whatever device the OS hands back, not from
        // here, so it is not this code's business to trust it.
        assert!(render_cue(RecordingFeedback::Started, 0).is_empty());
        assert!(
            render_cue(RecordingFeedback::Started, 8_000)
                .iter()
                .all(|sample| sample.is_finite())
        );
    }

    #[test]
    fn the_cue_reports_drained_only_after_the_device_has_taken_every_sample() {
        // The regression this exists for. Playback used to sleep for the cue's
        // own duration plus a margin and then drop the stream, on the
        // assumption that a device starts consuming when `play()` returns.
        // Measured on an 8-channel 96 kHz WASAPI device, it took *zero* samples
        // for the first 400ms and the whole cue by 653ms — so a 365ms sleep
        // destroyed the stream before one sample had left, on every dictation,
        // without erroring. `drained` is what replaced the clock, so what has
        // to hold is that it never says "done" early.
        let drained = Arc::new(AtomicBool::new(false));
        let samples = render_cue(RecordingFeedback::Started, 8_000);
        let total = samples.len();
        let mut playback = CuePlayback {
            samples: samples.clone(),
            cursor: 0,
            drained: Arc::clone(&drained),
        };

        for (index, expected) in samples.iter().enumerate() {
            let handed = playback.next_sample();
            assert!(
                (handed - expected).abs() < f32::EPSILON,
                "sample {index} was reordered or dropped",
            );
            assert!(
                !drained.load(Ordering::Acquire),
                "reported drained at sample {index} of {total}, before the device had it all",
            );
        }

        // Only the pull *past* the end means the device has it all.
        assert!(playback.next_sample().abs() < f32::EPSILON);
        assert!(drained.load(Ordering::Acquire));
        // And it keeps feeding silence rather than looping the cue, for however
        // long the stream stays open while the tail drains.
        assert!(playback.next_sample().abs() < f32::EPSILON);
    }

    #[test]
    fn the_drain_margin_outlasts_a_device_that_starts_late() {
        // Both halves of the wait have a job. The ceiling has to be generous
        // enough for a device that took 653ms to take a 245ms cue, and the
        // drain has to outlive the buffer the device is still holding when it
        // takes the last sample.
        assert!(PLAYBACK_CEILING > cue_duration() * 4);
        assert!(DRAIN >= Duration::from_millis(200));
    }

    #[test]
    fn conversion_to_integer_samples_clamps_rather_than_wrapping() {
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(2.0), i16::MAX);
        assert_eq!(to_i16(-2.0), -i16::MAX);
    }
}
