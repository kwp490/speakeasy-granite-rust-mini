use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use speakeasy_audio::{
    AudioPipelineConfig, CaptureIdentity, CaptureStreamId, ChannelPolicy, CpalCaptureRequest,
    CpalCaptureSession, InputDeviceDescriptor, NativeStreamConfig, UtteranceIssues,
    build_audio_pipeline, enumerate_input_devices,
};
use speakeasy_domain::{CorrelationId, ProducerId, SessionId, UtteranceAudio};

const TARGET_RATE_HZ: u32 = 16_000;
/// Product endpoint policy and safety ceiling for one capture (UI-GUIDE
/// "Main window and focus", the dictation lifecycle).
///
/// A user press is the normal endpoint; automatic voice activity detection
/// (VAD) endpointing is deliberately not used. Hitting this two-minute ceiling
/// is *not* a failure: capture stops exactly as a user-initiated stop does, and
/// the utterance is transcribed and delivered. Discarding speech to enforce a
/// limit would be the worse outcome.
///
/// **Why two minutes and not the thirty originally specified.** That figure
/// costed retained audio at ~3.8 MB/min, which counts only the `f32` samples. The
/// pipeline also retains a `ProcessedSampleMetadata` *per sample* — three
/// `u64`s and a flag, 32 bytes — so the real cost is 36 bytes per sample, or
/// ~34.6 MB per minute. A thirty-minute ceiling is therefore ~1 GB, and
/// `TimedUtteranceBuffer::new` allocates the whole thing up front on every
/// dictation: the sample array gets zeroed pages cheaply, but the metadata
/// array is element-wise written, so that cost lands inside start latency.
///
/// Two minutes is ~69 MB, which is affordable per dictation and is the shipped
/// maximum duration for one dictation.
///
/// Raising this to thirty minutes needs `speakeasy-audio` to stop retaining
/// per-sample metadata for the utterance first. Nothing in the product reads
/// `AudioWorker::utterance_metadata` — only a test inside that crate — so the
/// array is pure overhead here, but removing it is a change to the real-time
/// capture path and belongs in its own increment.
pub const MAX_CAPTURE_SECONDS: u32 = 2 * 60;
/// How often the input level is republished. Matches the HUD's 100 ms poll, so
/// the meter redraws at 10 Hz — inside the 20-30 Hz budget without raising IPC.
const LEVEL_BUCKET: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaptureDeviceView {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub supported: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaptureWizardView {
    pub state: String,
    pub device_name: Option<String>,
    pub captured_samples: Option<usize>,
    pub nonzero_samples: Option<usize>,
    pub peak_magnitude: Option<i16>,
    pub error_code: Option<String>,
    pub can_stop: bool,
    pub can_transcribe: bool,
    pub can_retry: bool,
}

/// Receives live audio as it is captured, for the real-time streaming HUD.
///
/// The tap is display-only: it never influences the retained utterance that
/// delivery is built from, and it is deliberately infallible from the capture
/// loop's point of view. A tap that fails must record that failure itself and
/// go quiet — a broken HUD must never cost the user their dictation.
pub trait CaptureTap: Send {
    /// One block of freshly processed 16 kHz mono audio, quantized to i16.
    ///
    /// Called on the capture loop, so this must not block: stalling here backs
    /// the callback queue up and can overflow it, which *fails the dictation*.
    fn push(&mut self, samples: &[i16], monotonic_ns: u64);
    /// Capture has stopped and no further blocks will arrive. Takes ownership
    /// because the capture loop hands the tap to its own thread to finish on —
    /// finalization can take seconds and the capture thread must not wait for
    /// it (the retained utterance is gated behind the capture thread finishing).
    fn finish(self: Box<Self>, monotonic_ns: u64);
}

struct ActiveCapture {
    stop: Arc<AtomicBool>,
}

pub struct CaptureWizardCoordinator {
    status: Arc<Mutex<CaptureWizardView>>,
    active: Arc<Mutex<Option<ActiveCapture>>>,
    retained: Arc<Mutex<Option<UtteranceAudio>>>,
    next_identity: AtomicU64,
    /// How long the current (or most recent) capture has been running.
    ///
    /// Written by the capture loop and read by the HUD poll, so the elapsed
    /// clock the user sees is the capture's own, not a second timer in the UI
    /// that could drift away from it. It stops advancing when capture ends and
    /// keeps its final value, which is what the "N seconds captured" line
    /// during transcription reports.
    elapsed_ms: Arc<AtomicU64>,
    /// Input amplitude 0..1 for the level meter, as `f32` bits.
    ///
    /// Measured in the capture loop rather than in the streaming tap, because
    /// the meter must work when streaming is unavailable — no model installed,
    /// no worker — and "the microphone is picking me up" is exactly the
    /// question a user asks when transcription is *not* working.
    level: Arc<AtomicU32>,
    /// Callback queue overflows observed by the most recent capture. Kept
    /// separately from the user-facing error code so diagnostics can report
    /// the actual count instead of manufacturing a zero.
    audio_overflow_count: Arc<AtomicU64>,
    /// Set when the user stops. Distinguishes a user stop from the safety
    /// ceiling firing, so the ceiling can be finished the same way without
    /// the two racing to transcribe the same utterance twice.
    stop_requested: Arc<AtomicBool>,
    /// Set when the user abandons the dictation.
    ///
    /// Read by the capture thread once the audio has finished arriving, because
    /// that thread is the only place that knows when there is anything to throw
    /// away — the user presses Cancel while samples are still being written.
    cancelled: Arc<AtomicBool>,
}

impl Default for CaptureWizardCoordinator {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(CaptureWizardView::idle())),
            active: Arc::new(Mutex::new(None)),
            retained: Arc::new(Mutex::new(None)),
            next_identity: AtomicU64::new(1),
            elapsed_ms: Arc::new(AtomicU64::new(0)),
            level: Arc::new(AtomicU32::new(0)),
            audio_overflow_count: Arc::new(AtomicU64::new(0)),
            stop_requested: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl CaptureWizardCoordinator {
    pub fn devices() -> Result<Vec<CaptureDeviceView>, &'static str> {
        enumerate_input_devices()
            .map_err(|_| "capture_device_enumeration_failed")
            .map(|devices| devices.iter().map(CaptureDeviceView::from).collect())
    }

    pub fn start_for_session(
        &self,
        device_id: &str,
        maximum_seconds: u32,
        session_id: SessionId,
        tap: Option<Box<dyn CaptureTap>>,
        acquire_operation: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let identity_number = self.next_identity.fetch_add(1, Ordering::Relaxed);
        self.start_with_identity(
            device_id,
            maximum_seconds,
            capture_identity(identity_number, session_id),
            tap,
            acquire_operation,
        )
    }

    fn start_with_identity(
        &self,
        device_id: &str,
        maximum_seconds: u32,
        identity: CaptureIdentity,
        tap: Option<Box<dyn CaptureTap>>,
        acquire_operation: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if !(1..=MAX_CAPTURE_SECONDS).contains(&maximum_seconds) {
            return Err("capture_duration_out_of_range");
        }
        let devices = enumerate_input_devices().map_err(|_| "capture_device_enumeration_failed")?;
        let descriptor = devices
            .into_iter()
            .find(|device| device.stable_id == device_id)
            .ok_or("capture_device_unavailable")?;
        let native = descriptor
            .default_config
            .ok_or("capture_device_format_unsupported")?;
        let mut active = self
            .active
            .lock()
            .map_err(|_| "capture_state_unavailable")?;
        if active.is_some() {
            return Err("capture_already_active");
        }
        acquire_operation()?;
        let stop = Arc::new(AtomicBool::new(false));
        *active = Some(ActiveCapture {
            stop: Arc::clone(&stop),
        });
        drop(active);
        *self
            .retained
            .lock()
            .map_err(|_| "capture_state_unavailable")? = None;
        set_status(
            &self.status,
            CaptureWizardView::progress("arming", &descriptor.display_name, false),
        );

        let status = Arc::clone(&self.status);
        let active = Arc::clone(&self.active);
        let retained = Arc::clone(&self.retained);
        let elapsed_ms = Arc::clone(&self.elapsed_ms);
        let level = Arc::clone(&self.level);
        let audio_overflow_count = Arc::clone(&self.audio_overflow_count);
        let cancelled = Arc::clone(&self.cancelled);
        elapsed_ms.store(0, Ordering::Release);
        level.store(0, Ordering::Release);
        self.stop_requested.store(false, Ordering::Release);
        cancelled.store(false, Ordering::Release);
        thread::spawn(move || {
            let outcome = capture(
                &descriptor,
                native,
                identity,
                maximum_seconds,
                &stop,
                &status,
                tap,
                &elapsed_ms,
                &level,
                &audio_overflow_count,
            );
            // The meter must not freeze at the last speech peak once the
            // microphone is closed.
            level.store(0, Ordering::Release);
            // A cancelled dictation is discarded here rather than in `cancel`,
            // which cannot know when the last sample has landed.
            if cancelled.load(Ordering::Acquire) {
                drop(outcome);
                discard_cancelled_capture(&status, &active, &retained, &elapsed_ms);
                return;
            }
            match outcome {
                Ok(audio) => {
                    let sample_count = audio.samples.len();
                    let nonzero_samples =
                        audio.samples.iter().filter(|sample| **sample != 0).count();
                    let peak_magnitude = audio
                        .samples
                        .iter()
                        .map(|sample| sample.saturating_abs())
                        .max()
                        .unwrap_or_default();
                    if let Ok(mut slot) = retained.lock() {
                        *slot = Some(audio);
                    }
                    set_status(
                        &status,
                        CaptureWizardView {
                            state: "captured".to_owned(),
                            device_name: Some(descriptor.display_name),
                            captured_samples: Some(sample_count),
                            nonzero_samples: Some(nonzero_samples),
                            peak_magnitude: Some(peak_magnitude),
                            error_code: None,
                            can_stop: false,
                            can_transcribe: true,
                            can_retry: true,
                        },
                    );
                }
                Err(code) => set_status(&status, CaptureWizardView::failed(code)),
            }
            if let Ok(mut slot) = active.lock() {
                *slot = None;
            }
        });
        Ok(())
    }

    /// Whether the current capture was stopped by the user rather than by the
    /// safety ceiling. Lets the caller tell "the user finished speaking" from
    /// "we ran out of ceiling", which end the same way but are reached
    /// differently.
    pub fn stop_was_requested(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
    }

    pub fn stop(&self) -> Result<(), &'static str> {
        let active = self
            .active
            .lock()
            .map_err(|_| "capture_state_unavailable")?;
        let active = active.as_ref().ok_or("capture_not_active")?;
        self.stop_requested.store(true, Ordering::Release);
        active.stop.store(true, Ordering::Release);
        if let Ok(current) = self.status.lock() {
            let device_name = current.device_name.clone();
            drop(current);
            set_status(
                &self.status,
                CaptureWizardView {
                    state: "draining".to_owned(),
                    device_name,
                    captured_samples: None,
                    nonzero_samples: None,
                    peak_magnitude: None,
                    error_code: None,
                    can_stop: false,
                    can_transcribe: false,
                    can_retry: false,
                },
            );
        }
        Ok(())
    }

    /// Abandons the dictation in progress: the microphone closes and the audio
    /// is thrown away instead of transcribed.
    ///
    /// Reports whether a running capture was signalled to stop. Succeeds either
    /// way — Cancel is pressed from a UI that can be a poll behind the truth, so
    /// cancelling a capture that has already ended is a no-op, not an error, and
    /// pressing it twice must not fail the second time.
    ///
    /// Note what this deliberately does *not* do: move the status. Reporting idle
    /// from here would claim the recording had ended while the microphone was
    /// still draining. The capture thread flips it once that is actually true.
    pub fn cancel(&self) -> Result<bool, &'static str> {
        self.cancelled.store(true, Ordering::Release);
        let running = self
            .active
            .lock()
            .map_err(|_| "capture_state_unavailable")?
            .as_ref()
            .map(|active| Arc::clone(&active.stop));
        if let Some(stop) = running {
            self.stop_requested.store(true, Ordering::Release);
            stop.store(true, Ordering::Release);
            return Ok(true);
        }
        // Capture is already over, so there is no thread left to do the
        // discarding — including the case where it ended with audio retained and
        // ready to transcribe. That audio is what Cancel exists to throw away.
        *self
            .retained
            .lock()
            .map_err(|_| "capture_state_unavailable")? = None;
        self.elapsed_ms.store(0, Ordering::Release);
        self.level.store(0, Ordering::Release);
        set_status(&self.status, CaptureWizardView::idle());
        Ok(false)
    }

    /// Milliseconds captured so far, or the final duration once capture ended.
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms.load(Ordering::Acquire)
    }

    /// Current input amplitude, 0..1. Zero whenever no capture is running.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Acquire)).clamp(0.0, 1.0)
    }

    /// Number of callback queue overflows observed by the most recent
    /// capture. Zero means no overflow was observed, not "not measured".
    pub fn audio_overflow_count(&self) -> u64 {
        self.audio_overflow_count.load(Ordering::Acquire)
    }

    pub fn view(&self) -> Result<CaptureWizardView, &'static str> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "capture_state_unavailable")
    }

    pub fn retained_audio(&self) -> Result<UtteranceAudio, &'static str> {
        self.retained
            .lock()
            .map_err(|_| "capture_state_unavailable")?
            .clone()
            .ok_or("capture_audio_unavailable")
    }

    pub fn has_retained_audio(&self) -> bool {
        self.retained
            .lock()
            .is_ok_and(|audio| audio.as_ref().is_some())
    }

    pub fn mark_finalizing(&self) {
        if let Ok(current) = self.status.lock() {
            let device_name = current.device_name.clone();
            let captured_samples = current.captured_samples;
            let nonzero_samples = current.nonzero_samples;
            let peak_magnitude = current.peak_magnitude;
            drop(current);
            set_status(
                &self.status,
                CaptureWizardView {
                    state: "finalizing".to_owned(),
                    device_name,
                    captured_samples,
                    nonzero_samples,
                    peak_magnitude,
                    error_code: None,
                    can_stop: false,
                    can_transcribe: false,
                    can_retry: false,
                },
            );
        }
    }

    pub fn mark_transcription_finished(&self, error_code: Option<&'static str>) {
        if let Ok(current) = self.status.lock() {
            let device_name = current.device_name.clone();
            let captured_samples = current.captured_samples;
            let nonzero_samples = current.nonzero_samples;
            let peak_magnitude = current.peak_magnitude;
            drop(current);
            set_status(
                &self.status,
                CaptureWizardView {
                    state: if error_code.is_some() {
                        "failed"
                    } else {
                        "complete"
                    }
                    .to_owned(),
                    device_name,
                    captured_samples,
                    nonzero_samples,
                    peak_magnitude,
                    error_code: error_code.map(str::to_owned),
                    can_stop: false,
                    can_transcribe: false,
                    can_retry: true,
                },
            );
        }
    }
}

impl CaptureWizardView {
    fn idle() -> Self {
        Self {
            state: "idle".to_owned(),
            device_name: None,
            captured_samples: None,
            nonzero_samples: None,
            peak_magnitude: None,
            error_code: None,
            can_stop: false,
            can_transcribe: false,
            can_retry: false,
        }
    }

    fn progress(state: &str, device_name: &str, can_stop: bool) -> Self {
        Self {
            state: state.to_owned(),
            device_name: Some(device_name.to_owned()),
            captured_samples: None,
            nonzero_samples: None,
            peak_magnitude: None,
            error_code: None,
            can_stop,
            can_transcribe: false,
            can_retry: false,
        }
    }

    fn failed(code: &'static str) -> Self {
        Self {
            state: "failed".to_owned(),
            device_name: None,
            captured_samples: None,
            nonzero_samples: None,
            peak_magnitude: None,
            error_code: Some(code.to_owned()),
            can_stop: false,
            can_transcribe: false,
            can_retry: false,
        }
    }
}

impl From<&InputDeviceDescriptor> for CaptureDeviceView {
    fn from(device: &InputDeviceDescriptor) -> Self {
        Self {
            id: device.stable_id.clone(),
            name: device.display_name.clone(),
            is_default: device.is_default,
            supported: device.default_config.is_some(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capture(
    descriptor: &InputDeviceDescriptor,
    native: NativeStreamConfig,
    identity: CaptureIdentity,
    maximum_seconds: u32,
    stop: &AtomicBool,
    status: &Mutex<CaptureWizardView>,
    mut tap: Option<Box<dyn CaptureTap>>,
    elapsed_ms: &AtomicU64,
    level: &AtomicU32,
    audio_overflow_count: &AtomicU64,
) -> Result<UtteranceAudio, &'static str> {
    audio_overflow_count.store(0, Ordering::Release);
    let (callback, mut worker) =
        build_audio_pipeline(pipeline_config(native, identity, maximum_seconds)?)
            .map_err(|_| "capture_pipeline_rejected")?;
    worker
        .begin_utterance()
        .map_err(|_| "capture_utterance_rejected")?;
    let mut session = CpalCaptureSession::start(
        &CpalCaptureRequest {
            identity,
            device_stable_id: descriptor.stable_id.clone(),
        },
        callback,
    )
    .map_err(|_| "capture_start_failed")?;
    set_status(
        status,
        CaptureWizardView::progress("capturing", &descriptor.display_name, true),
    );
    let started = Instant::now();
    // Reused across blocks so the live tap costs no per-block allocation on the
    // capture loop.
    let mut quantized: Vec<i16> = Vec::new();
    // The level meter is published once per bucket rather than once per block:
    // blocks are far shorter than the HUD's 100 ms poll, so a per-block value
    // would be an arbitrary sample of the waveform rather than a peak.
    let mut bucket_peak = 0.0_f32;
    let mut bucket_started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(maximum_seconds));
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        elapsed_ms.store(
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
        while let Some(block) = worker.process_next() {
            bucket_peak = bucket_peak.max(block_peak(block.samples));
            if let Some(tap) = tap.as_mut() {
                quantized.clear();
                quantized.extend(block.samples.iter().copied().map(quantize_sample));
                tap.push(&quantized, elapsed_ns(started));
            }
        }
        if bucket_started.elapsed() >= LEVEL_BUCKET {
            level.store(bucket_peak.to_bits(), Ordering::Release);
            bucket_peak = 0.0;
            bucket_started = Instant::now();
        }
        if session.poll_fault().is_some() {
            finish_tap_detached(tap, elapsed_ns(started));
            return Err("capture_device_fault");
        }
        thread::yield_now();
    }
    session.stop();
    elapsed_ms.store(
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        Ordering::Release,
    );
    // Finalization is handed off before `finish_utterance` so the retained
    // utterance — the only thing delivery depends on — is never waiting behind
    // the streaming engine.
    finish_tap_detached(tap, elapsed_ns(started));
    let completion = worker
        .finish_utterance()
        .map_err(|_| "capture_finish_failed")?;
    audio_overflow_count.store(
        worker.callback_counters().queue_overflows,
        Ordering::Release,
    );
    if completion.frames_buffered == 0 {
        return Err("capture_empty");
    }
    if let Some(code) = issue_code(completion.issues) {
        return Err(code);
    }
    Ok(UtteranceAudio {
        session_id: identity.session_id,
        sample_rate_hz: TARGET_RATE_HZ,
        samples: worker
            .utterance_samples()
            .iter()
            .copied()
            .map(quantize_sample)
            .collect(),
    })
}

fn pipeline_config(
    native: NativeStreamConfig,
    identity: CaptureIdentity,
    maximum_seconds: u32,
) -> Result<AudioPipelineConfig, &'static str> {
    let target_frames = usize::try_from(TARGET_RATE_HZ)
        .map_err(|_| "capture_capacity_overflow")?
        .checked_mul(
            usize::try_from(maximum_seconds)
                .map_err(|_| "capture_capacity_overflow")?
                .saturating_add(1),
        )
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
            .ok_or("capture_capacity_overflow")?,
        max_utterance_ms: NonZeroU32::new(maximum_seconds.saturating_add(1).saturating_mul(1_000))
            .ok_or("capture_duration_out_of_range")?,
        max_buffered_bytes: NonZeroUsize::new(64 * 1_024 * 1_024)
            .expect("byte capacity is non-zero"),
    })
}

fn capture_identity(value: u64, session_id: SessionId) -> CaptureIdentity {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&value.to_le_bytes());
    CaptureIdentity {
        correlation_id: CorrelationId::from_bytes(bytes),
        session_id,
        producer_id: ProducerId::from_bytes(bytes.map(|byte| byte ^ 0xa2)),
        stream_id: CaptureStreamId::from_bytes(bytes.map(|byte| byte ^ 0xf3)),
        generation: value,
    }
}

/// Finalizes the live tap on its own thread.
///
/// Streaming finalization is a blocking round trip to the inference worker and
/// can take seconds. The capture thread must not wait for it: the caller only
/// marks the utterance ready to transcribe once `capture` returns, and the
/// hotkey flow gives up on that after `CAPTURE_DRAIN_TIMEOUT`. Blocking here
/// would cost the user their dictation to save a HUD update.
fn finish_tap_detached(tap: Option<Box<dyn CaptureTap>>, monotonic_ns: u64) {
    if let Some(tap) = tap {
        thread::spawn(move || tap.finish(monotonic_ns));
    }
}

/// Nanoseconds since capture started, saturating rather than wrapping. Used as
/// the streaming frames' producer timestamp.
fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Peak absolute amplitude of one processed block, clamped to 0..1.
fn block_peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        .clamp(0.0, 1.0)
}

#[allow(clippy::cast_possible_truncation)]
fn quantize_sample(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * f32::from(i16::MAX);
    scaled.round() as i16
}

fn issue_code(issues: UtteranceIssues) -> Option<&'static str> {
    if issues.contains(UtteranceIssues::CALLBACK_QUEUE_OVERFLOW) {
        Some("capture_queue_overflow")
    } else if issues.contains(UtteranceIssues::DISCONTINUITY) {
        Some("capture_discontinuity")
    } else if issues.contains(UtteranceIssues::DURATION_LIMIT) {
        Some("capture_duration_limit")
    } else if issues.contains(UtteranceIssues::BYTE_LIMIT) {
        Some("capture_byte_limit")
    } else if issues.contains(UtteranceIssues::BUFFER_CAPACITY) {
        Some("capture_buffer_limit")
    } else {
        None
    }
}

/// Returns a cancelled capture to rest, keeping none of its audio.
///
/// `active` is released *before* the status flips to idle, and that order is the
/// point: the user may press Start the instant they see Ready, and a capture
/// still holding `active` would refuse it with `capture_already_active`.
fn discard_cancelled_capture(
    status: &Mutex<CaptureWizardView>,
    active: &Mutex<Option<ActiveCapture>>,
    retained: &Mutex<Option<UtteranceAudio>>,
    elapsed_ms: &AtomicU64,
) {
    if let Ok(mut slot) = retained.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = active.lock() {
        *slot = None;
    }
    elapsed_ms.store(0, Ordering::Release);
    set_status(status, CaptureWizardView::idle());
}

fn set_status(status: &Mutex<CaptureWizardView>, next: CaptureWizardView) {
    if let Ok(mut status) = status.lock() {
        *status = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speakeasy_audio::{NativeSampleFormat, NativeStreamConfig, ProcessedSampleMetadata};
    use std::num::{NonZeroU16, NonZeroU32};

    #[test]
    fn safety_ceiling_is_two_minutes() {
        assert_eq!(MAX_CAPTURE_SECONDS, 2 * 60);
    }

    #[test]
    fn capture_limits_are_bounded_without_opening_a_device() {
        let identity = capture_identity(7, SessionId::from_bytes([7; 16]));
        let native = NativeStreamConfig::new(
            NativeSampleFormat::F32,
            NonZeroU32::new(48_000).unwrap(),
            NonZeroU16::new(2).unwrap(),
        );
        let config = pipeline_config(native, identity, MAX_CAPTURE_SECONDS).unwrap();
        assert_eq!(config.target_sample_rate_hz.get(), TARGET_RATE_HZ);
        // One second of headroom past the ceiling, at the target rate.
        assert_eq!(
            config.utterance_capacity_frames.get(),
            (MAX_CAPTURE_SECONDS as usize + 1) * TARGET_RATE_HZ as usize
        );
        assert_eq!(config.max_buffered_bytes.get(), 64 * 1_024 * 1_024);
    }

    #[test]
    fn the_ceiling_stays_inside_the_pipeline_byte_limit() {
        // The retained utterance costs 36 bytes per sample, not 4: alongside
        // each `f32` the pipeline keeps a 32-byte `ProcessedSampleMetadata`.
        // A ceiling that outgrows `max_buffered_bytes` would silently truncate
        // the recording instead of capturing it, so the relationship is pinned
        // here rather than left to be rediscovered.
        let samples = (MAX_CAPTURE_SECONDS as usize + 1) * TARGET_RATE_HZ as usize;
        let retained_bytes = samples * (size_of::<f32>() + size_of::<ProcessedSampleMetadata>());
        assert!(
            retained_bytes < 128 * 1_024 * 1_024,
            "a {MAX_CAPTURE_SECONDS}s ceiling retains {retained_bytes} bytes per dictation, \
             which is allocated up front on every start"
        );
    }

    #[test]
    fn sample_quantization_clamps_before_conversion() {
        assert_eq!(quantize_sample(-2.0), -32_767);
        assert_eq!(quantize_sample(0.0), 0);
        assert_eq!(quantize_sample(2.0), 32_767);
    }

    fn is_close(value: f32, expected: f32) -> bool {
        (value - expected).abs() < 1e-6
    }

    #[test]
    fn level_reports_peak_magnitude_and_never_leaves_the_meter_range() {
        assert!(is_close(block_peak(&[]), 0.0));
        assert!(is_close(block_peak(&[0.0, 0.0]), 0.0));
        // A negative trough is as loud as a positive crest.
        assert!(is_close(block_peak(&[0.1, -0.7, 0.3]), 0.7));
        // A `<meter min=0 max=1>` must never be handed an out-of-range value,
        // even if the pipeline hands us one.
        assert!(is_close(block_peak(&[4.0]), 1.0));
        assert!(is_close(block_peak(&[-4.0]), 1.0));
    }

    #[test]
    fn level_is_zero_before_any_capture_has_run() {
        let coordinator = CaptureWizardCoordinator::default();
        assert!(is_close(coordinator.level(), 0.0));
        assert_eq!(coordinator.elapsed_ms(), 0);
    }

    /// Cancel is pressed from a transcriber that polls at 10 Hz, so it routinely
    /// arrives after the capture it meant to stop has already ended with audio
    /// retained and ready to transcribe. Discarding that audio is the whole job:
    /// leaving it behind is what let a cancelled dictation still be delivered.
    #[test]
    fn cancelling_after_capture_ended_discards_the_retained_audio() {
        let coordinator = CaptureWizardCoordinator::default();
        *coordinator.retained.lock().unwrap() = Some(UtteranceAudio {
            session_id: SessionId::from_bytes([3; 16]),
            sample_rate_hz: TARGET_RATE_HZ,
            samples: vec![1, -1, 1],
        });
        set_status(
            &coordinator.status,
            CaptureWizardView {
                state: "captured".to_owned(),
                device_name: Some("Test microphone".to_owned()),
                captured_samples: Some(3),
                nonzero_samples: Some(3),
                peak_magnitude: Some(1),
                error_code: None,
                can_stop: false,
                can_transcribe: true,
                can_retry: true,
            },
        );
        coordinator.elapsed_ms.store(4_200, Ordering::Release);

        // False: nothing was running, which is not a failure.
        assert_eq!(coordinator.cancel(), Ok(false));
        assert!(!coordinator.has_retained_audio());
        assert!(coordinator.retained_audio().is_err());
        let view = coordinator.view().unwrap();
        assert_eq!(view.state, "idle");
        assert!(
            !view.can_transcribe,
            "a cancelled dictation must not still offer to transcribe"
        );
        assert_eq!(coordinator.elapsed_ms(), 0);

        // Pressing Cancel twice is ordinary — the button stays up until the next
        // poll lands — so the second press must not fail.
        assert_eq!(coordinator.cancel(), Ok(false));
    }

    /// The discard runs on the capture thread, and the order it releases things
    /// in is load-bearing: a Start pressed the moment the transcriber says Ready
    /// would hit `capture_already_active` if `active` outlived the idle status.
    #[test]
    fn a_discarded_capture_releases_its_slot_before_it_reports_idle() {
        let status = Mutex::new(CaptureWizardView::progress("capturing", "Test", true));
        let active = Mutex::new(Some(ActiveCapture {
            stop: Arc::new(AtomicBool::new(true)),
        }));
        let retained = Mutex::new(Some(UtteranceAudio {
            session_id: SessionId::from_bytes([9; 16]),
            sample_rate_hz: TARGET_RATE_HZ,
            samples: vec![7],
        }));
        let elapsed_ms = AtomicU64::new(9_000);

        discard_cancelled_capture(&status, &active, &retained, &elapsed_ms);

        assert!(active.lock().unwrap().is_none());
        assert!(retained.lock().unwrap().is_none());
        assert_eq!(status.lock().unwrap().state, "idle");
        assert_eq!(elapsed_ms.load(Ordering::Acquire), 0);
    }
}
