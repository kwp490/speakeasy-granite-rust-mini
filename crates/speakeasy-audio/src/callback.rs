use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use speakeasy_domain::{AudioChunkMetadata, AudioDiscontinuity, AudioTimestamp, CaptureStreamId};

use crate::{NativeSampleFormat, NativeStreamConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallbackStamp {
    pub first_frame_index: u64,
    pub capture_monotonic_ns: u64,
    pub discontinuity: AudioDiscontinuity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallbackWriteStatus {
    Complete,
    QueueFull,
    InvalidInterleaving,
    UnexpectedSampleFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallbackWrite {
    pub accepted_frames: usize,
    pub dropped_frames: usize,
    pub status: CallbackWriteStatus,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CallbackCountersSnapshot {
    pub callbacks: u64,
    pub native_frames_received: u64,
    pub blocks_enqueued: u64,
    pub native_frames_enqueued: u64,
    pub blocks_dropped: u64,
    pub native_frames_dropped: u64,
    pub malformed_callbacks: u64,
    pub sample_format_mismatches: u64,
    pub clipped_samples: u64,
    pub non_finite_samples: u64,
    pub sample_clock_gaps: u64,
    pub queue_overflows: u64,
    pub queue_depth_blocks: usize,
}

#[derive(Default)]
struct CallbackCounters {
    callbacks: AtomicU64,
    native_frames_received: AtomicU64,
    blocks_enqueued: AtomicU64,
    native_frames_enqueued: AtomicU64,
    blocks_dropped: AtomicU64,
    native_frames_dropped: AtomicU64,
    malformed_callbacks: AtomicU64,
    sample_format_mismatches: AtomicU64,
    clipped_samples: AtomicU64,
    non_finite_samples: AtomicU64,
    sample_clock_gaps: AtomicU64,
    queue_overflows: AtomicU64,
}

struct Slot {
    samples: Box<[AtomicU32]>,
    frames: AtomicUsize,
    source_sequence: AtomicU64,
    first_frame_index: AtomicU64,
    capture_monotonic_ns: AtomicU64,
    discontinuity: AtomicU32,
}

impl Slot {
    fn new(sample_capacity: usize) -> Self {
        Self {
            samples: (0..sample_capacity)
                .map(|_| AtomicU32::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            frames: AtomicUsize::new(0),
            source_sequence: AtomicU64::new(0),
            first_frame_index: AtomicU64::new(0),
            capture_monotonic_ns: AtomicU64::new(0),
            discontinuity: AtomicU32::new(0),
        }
    }
}

pub(crate) struct Ring {
    slots: Box<[Slot]>,
    published_blocks: AtomicUsize,
    consumed_blocks: AtomicUsize,
    counters: CallbackCounters,
    stream_id: CaptureStreamId,
    native: NativeStreamConfig,
    frames_per_block: usize,
}

impl Ring {
    pub(crate) fn new(
        stream_id: CaptureStreamId,
        native: NativeStreamConfig,
        block_count: NonZeroUsize,
        frames_per_block: NonZeroUsize,
    ) -> Arc<Self> {
        let samples_per_block = frames_per_block
            .get()
            .checked_mul(usize::from(native.channels().get()))
            .expect("pipeline capacity validated before ring creation");
        Arc::new(Self {
            slots: (0..block_count.get())
                .map(|_| Slot::new(samples_per_block))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            published_blocks: AtomicUsize::new(0),
            consumed_blocks: AtomicUsize::new(0),
            counters: CallbackCounters::default(),
            stream_id,
            native,
            frames_per_block: frames_per_block.get(),
        })
    }

    fn snapshot(&self) -> CallbackCountersSnapshot {
        let consumed = self.consumed_blocks.load(Ordering::Acquire);
        let published = self.published_blocks.load(Ordering::Acquire);
        CallbackCountersSnapshot {
            callbacks: self.counters.callbacks.load(Ordering::Relaxed),
            native_frames_received: self.counters.native_frames_received.load(Ordering::Relaxed),
            blocks_enqueued: self.counters.blocks_enqueued.load(Ordering::Relaxed),
            native_frames_enqueued: self.counters.native_frames_enqueued.load(Ordering::Relaxed),
            blocks_dropped: self.counters.blocks_dropped.load(Ordering::Relaxed),
            native_frames_dropped: self.counters.native_frames_dropped.load(Ordering::Relaxed),
            malformed_callbacks: self.counters.malformed_callbacks.load(Ordering::Relaxed),
            sample_format_mismatches: self
                .counters
                .sample_format_mismatches
                .load(Ordering::Relaxed),
            clipped_samples: self.counters.clipped_samples.load(Ordering::Relaxed),
            non_finite_samples: self.counters.non_finite_samples.load(Ordering::Relaxed),
            sample_clock_gaps: self.counters.sample_clock_gaps.load(Ordering::Relaxed),
            queue_overflows: self.counters.queue_overflows.load(Ordering::Relaxed),
            queue_depth_blocks: published.wrapping_sub(consumed),
        }
    }
}

/// Single-producer handle intended to be owned by the native capture callback.
///
/// Its write methods perform no allocation, blocking, locking, logging, model,
/// VAD, or UI work. The type is intentionally not cloneable.
pub struct CaptureCallback {
    ring: Arc<Ring>,
    write_cursor: usize,
    source_sequence: u64,
    expected_first_frame: Option<u64>,
    pending_discontinuity: AudioDiscontinuity,
}

impl CaptureCallback {
    pub(crate) fn new(ring: Arc<Ring>) -> Self {
        Self {
            ring,
            write_cursor: 0,
            source_sequence: 0,
            expected_first_frame: None,
            pending_discontinuity: AudioDiscontinuity::NONE,
        }
    }

    pub fn write_f32(&mut self, samples: &[f32], stamp: CallbackStamp) -> CallbackWrite {
        self.write(samples, stamp)
    }

    pub fn write_i16(&mut self, samples: &[i16], stamp: CallbackStamp) -> CallbackWrite {
        self.write(samples, stamp)
    }

    pub fn write_u16(&mut self, samples: &[u16], stamp: CallbackStamp) -> CallbackWrite {
        self.write(samples, stamp)
    }

    pub fn counters(&self) -> CallbackCountersSnapshot {
        self.ring.snapshot()
    }

    // Keeping the hot path linear makes its no-allocation/no-blocking audit explicit.
    #[allow(clippy::too_many_lines)]
    fn write<T: NativeSample>(&mut self, samples: &[T], stamp: CallbackStamp) -> CallbackWrite {
        let channels = usize::from(self.ring.native.channels().get());
        self.ring.counters.callbacks.fetch_add(1, Ordering::Relaxed);
        if T::FORMAT != self.ring.native.sample_format() {
            self.ring
                .counters
                .sample_format_mismatches
                .fetch_add(1, Ordering::Relaxed);
            return CallbackWrite {
                accepted_frames: 0,
                dropped_frames: 0,
                status: CallbackWriteStatus::UnexpectedSampleFormat,
            };
        }
        if !samples.len().is_multiple_of(channels) {
            self.ring
                .counters
                .malformed_callbacks
                .fetch_add(1, Ordering::Relaxed);
            return CallbackWrite {
                accepted_frames: 0,
                dropped_frames: 0,
                status: CallbackWriteStatus::InvalidInterleaving,
            };
        }

        let frame_count = samples.len() / channels;
        self.ring
            .counters
            .native_frames_received
            .fetch_add(frame_count as u64, Ordering::Relaxed);

        let mut discontinuity = stamp.discontinuity | self.pending_discontinuity;
        self.pending_discontinuity = AudioDiscontinuity::NONE;
        if self
            .expected_first_frame
            .is_some_and(|expected| expected != stamp.first_frame_index)
        {
            discontinuity |= AudioDiscontinuity::SAMPLE_CLOCK_GAP;
            self.ring
                .counters
                .sample_clock_gaps
                .fetch_add(1, Ordering::Relaxed);
        }
        self.expected_first_frame =
            Some(stamp.first_frame_index.saturating_add(frame_count as u64));

        if frame_count == 0 {
            self.pending_discontinuity |= discontinuity;
            return CallbackWrite {
                accepted_frames: 0,
                dropped_frames: 0,
                status: CallbackWriteStatus::Complete,
            };
        }

        let mut accepted_frames = 0usize;
        while accepted_frames < frame_count {
            let consumed = self.ring.consumed_blocks.load(Ordering::Acquire);
            if self.write_cursor.wrapping_sub(consumed) >= self.ring.slots.len() {
                let dropped_frames = frame_count - accepted_frames;
                self.ring
                    .counters
                    .blocks_dropped
                    .fetch_add(1, Ordering::Relaxed);
                self.ring
                    .counters
                    .native_frames_dropped
                    .fetch_add(dropped_frames as u64, Ordering::Relaxed);
                self.ring
                    .counters
                    .queue_overflows
                    .fetch_add(1, Ordering::Relaxed);
                self.pending_discontinuity |= AudioDiscontinuity::CALLBACK_QUEUE_OVERFLOW;
                if accepted_frames == 0 {
                    self.pending_discontinuity |= discontinuity;
                }
                return CallbackWrite {
                    accepted_frames,
                    dropped_frames,
                    status: CallbackWriteStatus::QueueFull,
                };
            }

            let frames = (frame_count - accepted_frames).min(self.ring.frames_per_block);
            let sample_start = accepted_frames * channels;
            let sample_end = sample_start + frames * channels;
            let slot = &self.ring.slots[self.write_cursor % self.ring.slots.len()];
            let mut clipped = 0u64;
            let mut non_finite = 0u64;
            for (destination, source) in slot.samples.iter().zip(&samples[sample_start..sample_end])
            {
                let converted = source.to_normalized_f32();
                clipped += u64::from(converted.clipped);
                non_finite += u64::from(converted.non_finite);
                destination.store(converted.value.to_bits(), Ordering::Relaxed);
            }

            let frame_offset = accepted_frames as u64;
            let time_offset_ns = frame_offset.saturating_mul(1_000_000_000)
                / u64::from(self.ring.native.sample_rate_hz().get());
            slot.frames.store(frames, Ordering::Relaxed);
            slot.source_sequence
                .store(self.source_sequence, Ordering::Relaxed);
            slot.first_frame_index.store(
                stamp.first_frame_index.saturating_add(frame_offset),
                Ordering::Relaxed,
            );
            slot.capture_monotonic_ns.store(
                stamp.capture_monotonic_ns.saturating_add(time_offset_ns),
                Ordering::Relaxed,
            );
            slot.discontinuity
                .store(u32::from(discontinuity.bits()), Ordering::Relaxed);

            self.ring
                .counters
                .clipped_samples
                .fetch_add(clipped, Ordering::Relaxed);
            self.ring
                .counters
                .non_finite_samples
                .fetch_add(non_finite, Ordering::Relaxed);
            self.ring
                .counters
                .blocks_enqueued
                .fetch_add(1, Ordering::Relaxed);
            self.ring
                .counters
                .native_frames_enqueued
                .fetch_add(frames as u64, Ordering::Relaxed);

            self.write_cursor = self.write_cursor.wrapping_add(1);
            self.source_sequence = self.source_sequence.wrapping_add(1);
            self.ring
                .published_blocks
                .store(self.write_cursor, Ordering::Release);
            accepted_frames += frames;
            discontinuity = AudioDiscontinuity::NONE;
        }

        CallbackWrite {
            accepted_frames,
            dropped_frames: 0,
            status: CallbackWriteStatus::Complete,
        }
    }
}

struct ConvertedSample {
    value: f32,
    clipped: bool,
    non_finite: bool,
}

trait NativeSample: Copy {
    const FORMAT: NativeSampleFormat;

    fn to_normalized_f32(self) -> ConvertedSample;
}

impl NativeSample for f32 {
    const FORMAT: NativeSampleFormat = NativeSampleFormat::F32;

    fn to_normalized_f32(self) -> ConvertedSample {
        if !self.is_finite() {
            return ConvertedSample {
                value: 0.0,
                clipped: false,
                non_finite: true,
            };
        }
        ConvertedSample {
            value: self.clamp(-1.0, 1.0),
            clipped: !(-1.0..=1.0).contains(&self),
            non_finite: false,
        }
    }
}

impl NativeSample for i16 {
    const FORMAT: NativeSampleFormat = NativeSampleFormat::I16;

    fn to_normalized_f32(self) -> ConvertedSample {
        ConvertedSample {
            value: f32::from(self) / 32_768.0,
            clipped: false,
            non_finite: false,
        }
    }
}

impl NativeSample for u16 {
    const FORMAT: NativeSampleFormat = NativeSampleFormat::U16;

    fn to_normalized_f32(self) -> ConvertedSample {
        ConvertedSample {
            value: (f32::from(self) - 32_768.0) / 32_768.0,
            clipped: false,
            non_finite: false,
        }
    }
}

pub(crate) struct CapturedNativeBlock {
    samples: Box<[f32]>,
    sample_len: usize,
    pub(crate) frames: usize,
    pub(crate) metadata: Option<AudioChunkMetadata>,
}

impl CapturedNativeBlock {
    pub(crate) fn new(sample_capacity: usize) -> Self {
        Self {
            samples: vec![0.0; sample_capacity].into_boxed_slice(),
            sample_len: 0,
            frames: 0,
            metadata: None,
        }
    }

    pub(crate) fn samples(&self) -> &[f32] {
        &self.samples[..self.sample_len]
    }
}

pub(crate) struct RingConsumer {
    ring: Arc<Ring>,
    read_cursor: usize,
}

impl RingConsumer {
    pub(crate) fn new(ring: Arc<Ring>) -> Self {
        Self {
            ring,
            read_cursor: 0,
        }
    }

    pub(crate) fn try_pop(&mut self, destination: &mut CapturedNativeBlock) -> bool {
        let published = self.ring.published_blocks.load(Ordering::Acquire);
        if self.read_cursor == published {
            return false;
        }
        let slot = &self.ring.slots[self.read_cursor % self.ring.slots.len()];
        let frames = slot.frames.load(Ordering::Relaxed);
        let sample_len = frames * usize::from(self.ring.native.channels().get());
        for (output, input) in destination.samples[..sample_len]
            .iter_mut()
            .zip(slot.samples.iter())
        {
            *output = f32::from_bits(input.load(Ordering::Relaxed));
        }
        destination.sample_len = sample_len;
        destination.frames = frames;
        destination.metadata = Some(AudioChunkMetadata::new(
            self.ring.stream_id,
            slot.source_sequence.load(Ordering::Relaxed),
            AudioTimestamp {
                sample_rate_hz: self.ring.native.sample_rate_hz(),
                first_frame_index: slot.first_frame_index.load(Ordering::Relaxed),
                capture_monotonic_ns: slot.capture_monotonic_ns.load(Ordering::Relaxed),
            },
            AudioDiscontinuity::from_bits_truncate(
                u8::try_from(slot.discontinuity.load(Ordering::Relaxed))
                    .expect("callback stores discontinuity as u8"),
            ),
        ));
        self.read_cursor = self.read_cursor.wrapping_add(1);
        self.ring
            .consumed_blocks
            .store(self.read_cursor, Ordering::Release);
        true
    }

    pub(crate) fn callback_counters(&self) -> CallbackCountersSnapshot {
        self.ring.snapshot()
    }
}
