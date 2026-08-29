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

/// What one finished capture produced: the audio, and anything worth saying
/// about how it was captured.
///
/// The two annotations are deliberately not errors. Before 2026-08-25 both
/// arrived as `Err` and cost the user the recording; see the note at the end of
/// [`capture`] for what that did to long dictations.
struct CapturedUtterance {
    audio: UtteranceAudio,
    /// A `capture_*` code describing an imperfection in audio that still
    /// exists -- a dropped block, a processing overrun, a buffer limit. `None`
    /// when the capture was clean.
    quality_note: Option<&'static str>,
    /// Whether the safety ceiling ended this capture rather than the user.
    ///
    /// Its own fact rather than a `quality_note`, because it is the one the
    /// user has to be *told*: the recording is complete up to the limit and
    /// everything after it was never heard, which is a thing to know before
    /// starting the next one.
    reached_ceiling: bool,
}

/// The capture's whole state, as the frontend sees it.
///
/// The four booleans are an IPC contract, not a modelling choice: each is read
/// by a different control in a different window, and the shape is pinned by
/// `src/ipc/phase9.schema.json` and by the settings and dock components.
/// Collapsing them into a sub-struct or an enum would change the wire format
/// for a lint about readability.
#[expect(
    clippy::struct_excessive_bools,
    reason = "an IPC DTO whose shape is pinned by the schema and two frontends"
)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaptureWizardView {
    pub state: String,
    pub device_name: Option<String>,
    pub captured_samples: Option<usize>,
    pub nonzero_samples: Option<usize>,
    pub peak_magnitude: Option<i16>,
    pub error_code: Option<String>,
    /// An imperfection in audio that was still delivered. Never a failure --
    /// `error_code` is for those, and the two are never both set.
    pub quality_note: Option<String>,
    /// Whether the safety ceiling ended the capture rather than the user.
    pub reached_ceiling: bool,
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
    /// The last answer to "is there a supported microphone", and when it was
    /// taken. See [`Self::has_supported_microphone`].
    microphone_presence: Arc<Mutex<Option<(Instant, bool)>>>,
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
            microphone_presence: Arc::new(Mutex::new(None)),
        }
    }
}

/// How long an enumerated microphone-presence answer is reused for.
///
/// The HUD asks the question ten times a second and a microphone does not
/// appear or vanish ten times a second, so this is what stands between a
/// permanent dock and ten WASAPI device walks per second. Two seconds is short
/// enough that plugging a headset in is noticed about as fast as a person can
/// look at the dock, and long enough that an idle dock enumerates once every
/// twenty ticks instead of on all of them.
const MICROPHONE_PRESENCE_TTL: Duration = Duration::from_secs(2);

impl CaptureWizardCoordinator {
    pub fn devices() -> Result<Vec<CaptureDeviceView>, &'static str> {
        enumerate_input_devices()
            .map_err(|_| "capture_device_enumeration_failed")
            .map(|devices| devices.iter().map(CaptureDeviceView::from).collect())
    }

    /// Whether this machine has a capture device the app can actually use.
    ///
    /// Answered from a short-lived cache, because its one caller is
    /// `setup_requirement` and `setup_requirement`'s one caller is the HUD poll
    /// at 10 Hz. `enumerate_input_devices` is a full WASAPI walk that asks every
    /// device for its identifier, its description *and* its default input
    /// config, and `capture_hud_status` a few lines above the call carries a
    /// comment saying device enumeration is "far too expensive to do at 10 Hz"
    /// -- about a different field, while this one did exactly that on every
    /// tick from the fork until 2026-08-28.
    ///
    /// A stale answer is bounded by [`MICROPHONE_PRESENCE_TTL`] and is only ever
    /// used to decide whether the dock says "Setup needed". Starting a capture
    /// enumerates for real and fails on its own terms, so nothing is *gated* on
    /// this cache -- a microphone that appeared one second ago still records.
    pub fn has_supported_microphone(&self) -> bool {
        self.cached_microphone_presence(Instant::now(), || {
            Self::devices().is_ok_and(|devices| devices.iter().any(|device| device.supported))
        })
    }

    /// The cache itself, with `now` and the enumeration both passed in.
    ///
    /// The clock is a parameter rather than an `Instant::now()` inside so a test
    /// can advance it: a test that calls this thirty times in a row proves only
    /// that a burst is coalesced, which is the easy half. Expiry is the half
    /// that decides whether a plugged-in headset is ever noticed, and it cannot
    /// be observed without moving time or sleeping for the whole TTL.
    ///
    /// A poisoned lock answers by probing rather than by refusing: this decides
    /// whether the dock offers to start a dictation, and a mutex that failed
    /// somewhere else must not be able to say "you have no microphone".
    fn cached_microphone_presence(&self, now: Instant, probe: impl FnOnce() -> bool) -> bool {
        let Ok(mut cached) = self.microphone_presence.lock() else {
            return probe();
        };
        if let Some((taken, present)) = *cached
            && now.saturating_duration_since(taken) < MICROPHONE_PRESENCE_TTL
        {
            return present;
        }
        let present = probe();
        *cached = Some((now, present));
        present
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
                Ok(captured) => {
                    let view = retain_captured(&retained, &descriptor.display_name, captured);
                    set_status(&status, view);
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
                    quality_note: None,
                    // A user stop, by definition: this method is what sets
                    // `stop_requested`. The capture thread overwrites both of
                    // these when it finishes anyway.
                    reached_ceiling: false,
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
            // Carried forward rather than cleared. Delivery reads them after
            // the state has moved on, and a note that disappeared the moment
            // transcription started would be a note nothing could report.
            let quality_note = current.quality_note.clone();
            let reached_ceiling = current.reached_ceiling;
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
                    quality_note,
                    reached_ceiling,
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
            let quality_note = current.quality_note.clone();
            let reached_ceiling = current.reached_ceiling;
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
                    quality_note,
                    reached_ceiling,
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
            quality_note: None,
            reached_ceiling: false,
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
            quality_note: None,
            reached_ceiling: false,
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
            // Never both. A failure has no transcript to annotate.
            quality_note: None,
            reached_ceiling: false,
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
) -> Result<CapturedUtterance, &'static str> {
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
    // Which of the two loop conditions ended it, read before anything else can
    // change. `stop` is the user; anything else is the ceiling. This is the
    // reliable ceiling signal and `UtteranceIssues::DURATION_LIMIT` is not --
    // that one is only raised if the utterance buffer actually *rejects*
    // samples, and the buffer holds one second more than the ceiling allows, so
    // a capture can run the full two minutes without ever setting it.
    let reached_ceiling = !stop.load(Ordering::Acquire);
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
    let quality_note = judge_completion(completion.frames_buffered, completion.issues)?;
    Ok(CapturedUtterance {
        audio: UtteranceAudio {
            session_id: identity.session_id,
            sample_rate_hz: TARGET_RATE_HZ,
            samples: worker
                .utterance_samples()
                .iter()
                .copied()
                .map(quantize_sample)
                .collect(),
        },
        quality_note,
        reached_ceiling,
    })
}

/// Whether a finished capture is a failure, and what to say about it if not.
///
/// `Err` means there is nothing to transcribe. `Ok(Some(code))` means there is,
/// and something about how it was captured is worth reporting alongside it.
///
/// # This was the two-minute bug
///
/// Until 2026-08-25 every one of `issue_code`'s five conditions was an `Err`
/// here, and returning `Err` discards the recording. Only one of the six
/// conditions actually means the audio is unusable -- `frames_buffered == 0`.
/// The other five annotate audio that exists and would transcribe.
///
/// **What that actually cost**, measured rather than reasoned about: the byte
/// limit bound at 116.5 s against a 120 s ceiling (see `pipeline_config`), so
/// **every** maximum-length dictation filled its buffer, raised `BYTE_LIMIT`,
/// and was discarded. Not intermittently -- deterministically, which is exactly
/// why a long recording failed every time while a short one always worked. The
/// user was shown "The operation stopped safely", because four of the five
/// codes had no catalog entry either, and the log said only
/// `dictation_ceiling_stop result=no_audio`.
///
/// The byte limit is fixed now, so that particular annotation should not recur.
/// The other four remain reachable and stay annotations: two of them
/// (`DISCONTINUITY`, `CALLBACK_QUEUE_OVERFLOW`) are single events that latch
/// for the whole utterance, so a long recording is exposed to them far more
/// often than a short one -- which is a reason to report them, not to throw the
/// recording away.
///
/// A pure function taking the two facts, rather than a branch inside `capture`,
/// so the decision is reachable from a test on a machine with no microphone.
fn judge_completion(
    frames_buffered: usize,
    issues: UtteranceIssues,
) -> Result<Option<&'static str>, &'static str> {
    if frames_buffered == 0 {
        return Err("capture_empty");
    }
    Ok(issue_code(issues))
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
        // 128 MiB, and the figure is load-bearing. It was 64 MiB, which is
        // **less than the ceiling needs**: the retained utterance costs 36
        // bytes per frame -- an `f32` plus a 32-byte `ProcessedSampleMetadata`
        // -- so 64 MiB is 1,864,135 frames, or 116.5 s at 16 kHz, against a
        // 121 s capacity and a 120 s ceiling. The byte limit therefore bound
        // 3.5 s *before* the ceiling: every maximum-length dictation filled the
        // buffer, began rejecting samples, and raised `BYTE_LIMIT`.
        //
        // Until 2026-08-25 that was returned as an `Err` and the whole
        // recording was discarded, which is what made a two-minute dictation
        // fail every time while a short one worked. Measured on this machine as
        // `dictation_ceiling_stop ... quality=capture_byte_limit`. 121 s needs
        // 66.5 MiB, so this is comfortable rather than exact, and nothing
        // allocates it -- `utterance_capacity_frames` is what is reserved.
        max_buffered_bytes: NonZeroUsize::new(128 * 1_024 * 1_024)
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

/// Stores the finished utterance and describes it for the frontend.
///
/// Split out of `start_with_identity` because that function crossed the
/// hundred-line lint when the capture began carrying annotations. The grouping
/// is the right one anyway: everything here is about turning one finished
/// capture into the two things the rest of the app reads from it -- the
/// retained audio, and the view that says what is in it.
fn retain_captured(
    retained: &Mutex<Option<UtteranceAudio>>,
    device_name: &str,
    captured: CapturedUtterance,
) -> CaptureWizardView {
    let CapturedUtterance {
        audio,
        quality_note,
        reached_ceiling,
    } = captured;
    let sample_count = audio.samples.len();
    let nonzero_samples = audio.samples.iter().filter(|sample| **sample != 0).count();
    let peak_magnitude = audio
        .samples
        .iter()
        .map(|sample| sample.saturating_abs())
        .max()
        .unwrap_or_default();
    if let Ok(mut slot) = retained.lock() {
        *slot = Some(audio);
    }
    CaptureWizardView {
        state: "captured".to_owned(),
        device_name: Some(device_name.to_owned()),
        captured_samples: Some(sample_count),
        nonzero_samples: Some(nonzero_samples),
        peak_magnitude: Some(peak_magnitude),
        error_code: None,
        quality_note: quality_note.map(str::to_owned),
        reached_ceiling,
        can_stop: false,
        can_transcribe: true,
        can_retry: true,
    }
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
        // 128 MiB since 2026-08-25. At 64 MiB the byte limit bound at 116.5 s
        // -- inside the 120 s ceiling -- so the tail of every maximum-length
        // dictation was rejected and the recording discarded.
        assert_eq!(config.max_buffered_bytes.get(), 128 * 1_024 * 1_024);
    }

    #[test]
    fn the_ceiling_stays_inside_the_pipeline_byte_limit() {
        // The retained utterance costs 36 bytes per sample, not 4: alongside
        // each `f32` the pipeline keeps a 32-byte `ProcessedSampleMetadata`.
        // A ceiling that outgrows `max_buffered_bytes` truncates the recording
        // instead of capturing it, so the relationship is pinned here.
        //
        // **Compared against the configured value, not a written-down copy of
        // it.** This assertion used a hardcoded 128 MiB while `pipeline_config`
        // was built with 64 MiB, so it passed at 66.5 MiB with the real limit
        // already exceeded -- and the last 3.5 s of every maximum-length
        // dictation was being rejected, which then discarded the whole
        // recording. An instrument holding its own copy of a constant cannot
        // see that constant change.
        let native = NativeStreamConfig::new(
            NativeSampleFormat::F32,
            NonZeroU32::new(48_000).expect("native rate is non-zero"),
            NonZeroU16::new(1).expect("channel count is non-zero"),
        );
        let identity = capture_identity(11, SessionId::from_bytes([11; 16]));
        let config =
            pipeline_config(native, identity, MAX_CAPTURE_SECONDS).expect("config must build");
        let bytes_per_frame = size_of::<f32>() + size_of::<ProcessedSampleMetadata>();
        let byte_limit_frames = config.max_buffered_bytes.get() / bytes_per_frame;
        let capacity_frames = config.utterance_capacity_frames.get();
        assert!(
            byte_limit_frames >= capacity_frames,
            "the byte limit binds at {} s, before the {MAX_CAPTURE_SECONDS} s ceiling:              the tail of every long dictation is rejected",
            byte_limit_frames / TARGET_RATE_HZ as usize
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
    /// A capture with nothing in it is the only capture that fails.
    ///
    /// The regression test for the defect that cost a user two minutes of
    /// speech on 2026-08-25: every one of these five conditions used to return
    /// `Err` from `capture`, and an `Err` there discards the recording. They
    /// annotate audio that exists. Only an empty buffer means there is nothing
    /// to transcribe.
    #[test]
    fn an_imperfect_capture_is_annotated_and_an_empty_one_fails() {
        // `capture_byte_limit` is the one that actually bit: it bound at
        // 116.5 s against a 120 s ceiling, so every maximum-length dictation
        // raised it and was discarded. The limit is raised now, but the arm has
        // to stay an annotation -- the next thing to bind will be something
        // else, and a recording is not worth less because its buffer filled.
        for (issue, expected) in [
            (
                UtteranceIssues::CALLBACK_QUEUE_OVERFLOW,
                "capture_queue_overflow",
            ),
            (UtteranceIssues::DISCONTINUITY, "capture_discontinuity"),
            (UtteranceIssues::DURATION_LIMIT, "capture_duration_limit"),
            (UtteranceIssues::BYTE_LIMIT, "capture_byte_limit"),
            (UtteranceIssues::BUFFER_CAPACITY, "capture_buffer_limit"),
        ] {
            assert_eq!(
                judge_completion(16_000, issue),
                Ok(Some(expected)),
                "{expected} must annotate the capture, not discard it"
            );
        }

        // A clean capture says nothing at all.
        assert_eq!(judge_completion(16_000, UtteranceIssues::NONE), Ok(None));

        // The one real failure, and it is checked before the annotations: a
        // buffer with no frames in it has nothing for Granite to read.
        assert_eq!(
            judge_completion(0, UtteranceIssues::NONE),
            Err("capture_empty")
        );
        assert_eq!(
            judge_completion(0, UtteranceIssues::DISCONTINUITY),
            Err("capture_empty")
        );
    }

    /// Every code this can produce has copy a user can act on.
    ///
    /// Four of the five had none until 2026-08-25 and fell through to
    /// `errorUnknown` -- "The operation stopped safely" -- which is what the
    /// user was shown after losing a two-minute dictation. Asserted against the
    /// catalog source because the rule is a product rule: a reason code with no
    /// instruction is not a reason.
    #[test]
    fn every_capture_annotation_has_catalog_copy() {
        let catalog = include_str!("../../src/catalog.ts");
        for issue in [
            UtteranceIssues::CALLBACK_QUEUE_OVERFLOW,
            UtteranceIssues::DISCONTINUITY,
            UtteranceIssues::DURATION_LIMIT,
            UtteranceIssues::BYTE_LIMIT,
            UtteranceIssues::BUFFER_CAPACITY,
        ] {
            let code = issue_code(issue).expect("each issue maps to a code");
            assert!(
                catalog.contains(&format!("{code}:")),
                "{code} needs catalog copy or it renders as errorUnknown"
            );
        }
    }

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
                quality_note: None,
                reached_ceiling: false,
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

    /// The HUD poll does not enumerate audio hardware, and does notice a
    /// microphone that appears.
    ///
    /// `setup_requirement` runs on every one of the dock's ten ticks a second
    /// and used to call `CaptureWizardCoordinator::devices` -- a full WASAPI
    /// walk asking each device for its identifier, description and default
    /// input config -- directly, four lines below a comment saying enumeration
    /// is far too expensive at 10 Hz.
    ///
    /// Counted rather than timed: a timing assertion on a machine with one
    /// microphone passes with the enumeration still in place. Both halves are
    /// asserted, because a cache that never expires would satisfy the first.
    #[test]
    fn the_hud_poll_coalesces_device_enumeration_but_still_expires_it() {
        let capture = CaptureWizardCoordinator::default();
        let probes = std::cell::Cell::new(0_u32);
        let mut answer = true;
        let probe = |present: bool| {
            probes.set(probes.get() + 1);
            present
        };
        let start = Instant::now();
        // The last instant that must still be a cache hit.
        let just_inside = MICROPHONE_PRESENCE_TTL
            .checked_sub(Duration::from_millis(1))
            .expect("the TTL is longer than a millisecond");

        // A burst inside the window enumerates once.
        for tick in 0..30 {
            let now = start + Duration::from_millis(tick * 10);
            assert!(capture.cached_microphone_presence(now, || probe(answer)));
        }
        assert_eq!(
            probes.get(),
            1,
            "a burst inside the TTL must enumerate once"
        );

        // The last tick before expiry still reuses it.
        assert!(capture.cached_microphone_presence(start + just_inside, || probe(answer)));
        assert_eq!(probes.get(), 1, "the TTL must not expire early");

        // At expiry it asks again -- and takes the new answer, which is the
        // whole point: a cache that never refreshed would leave the dock
        // saying "Setup needed" forever after a microphone was unplugged once.
        answer = false;
        assert!(
            !capture.cached_microphone_presence(start + MICROPHONE_PRESENCE_TTL, || probe(answer))
        );
        assert_eq!(probes.get(), 2, "the TTL must expire");

        // And the new answer is what the next burst reuses.
        assert!(!capture.cached_microphone_presence(
            start + MICROPHONE_PRESENCE_TTL + Duration::from_millis(10),
            || probe(true)
        ));
        assert_eq!(probes.get(), 2);
    }
}
