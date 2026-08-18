use std::num::{NonZeroU32, NonZeroUsize};

use speakeasy_domain::{AudioChunkMetadata, AudioDiscontinuity, CaptureStreamId};

use crate::callback::{CapturedNativeBlock, Ring, RingConsumer};
use crate::format::{FormatError, downmix_into};
use crate::{CallbackCountersSnapshot, CaptureCallback, ChannelPolicy, NativeStreamConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioPipelineConfig {
    pub stream_id: CaptureStreamId,
    pub native: NativeStreamConfig,
    pub target_sample_rate_hz: NonZeroU32,
    pub channel_policy: ChannelPolicy,
    pub callback_block_count: NonZeroUsize,
    pub max_native_frames_per_block: NonZeroUsize,
    /// Must remain within the product contract's post-activation 250-500 ms range.
    pub pre_roll_ms: u16,
    /// Structural storage capacity; product duration/overflow policy is a later item.
    pub utterance_capacity_frames: NonZeroUsize,
    pub max_utterance_ms: NonZeroU32,
    pub max_buffered_bytes: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineBuildError {
    InvalidChannelPolicy(FormatError),
    PreRollOutsideContract { milliseconds: u16 },
    CapacityOverflow,
    UtteranceCannotHoldPreRoll { required: usize, available: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UtteranceStateError {
    AlreadyActive,
    NotActive,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UtteranceIssues(u8);

impl UtteranceIssues {
    pub const NONE: Self = Self(0);
    pub const DURATION_LIMIT: Self = Self(1 << 0);
    pub const BYTE_LIMIT: Self = Self(1 << 1);
    pub const BUFFER_CAPACITY: Self = Self(1 << 2);
    pub const CALLBACK_QUEUE_OVERFLOW: Self = Self(1 << 3);
    pub const DISCONTINUITY: Self = Self(1 << 4);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResamplerTailPolicy {
    DiscardUnobservedInterpolation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtteranceCompletion {
    pub frames_buffered: usize,
    pub drained_blocks: u64,
    pub issues: UtteranceIssues,
    pub tail_policy: ResamplerTailPolicy,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerCountersSnapshot {
    pub blocks_processed: u64,
    pub native_frames_processed: u64,
    pub resampled_frames_produced: u64,
    pub discontinuous_blocks: u64,
    pub resampler_resets: u64,
    pub pre_roll_overwritten_frames: u64,
    pub utterances_started: u64,
    pub utterance_frames_buffered: u64,
    pub utterance_frames_rejected: u64,
    pub utterance_limit_events: u64,
    pub final_drain_blocks: u64,
}

#[derive(Default)]
struct WorkerCounters {
    blocks_processed: u64,
    native_frames_processed: u64,
    resampled_frames_produced: u64,
    discontinuous_blocks: u64,
    resampler_resets: u64,
    pre_roll_overwritten_frames: u64,
    utterances_started: u64,
    utterance_frames_buffered: u64,
    utterance_frames_rejected: u64,
    utterance_limit_events: u64,
    final_drain_blocks: u64,
}

impl From<&WorkerCounters> for WorkerCountersSnapshot {
    fn from(value: &WorkerCounters) -> Self {
        Self {
            blocks_processed: value.blocks_processed,
            native_frames_processed: value.native_frames_processed,
            resampled_frames_produced: value.resampled_frames_produced,
            discontinuous_blocks: value.discontinuous_blocks,
            resampler_resets: value.resampler_resets,
            pre_roll_overwritten_frames: value.pre_roll_overwritten_frames,
            utterances_started: value.utterances_started,
            utterance_frames_buffered: value.utterance_frames_buffered,
            utterance_frames_rejected: value.utterance_frames_rejected,
            utterance_limit_events: value.utterance_limit_events,
            final_drain_blocks: value.final_drain_blocks,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessedSampleMetadata {
    pub output_sample_index: u64,
    pub source_first_frame_index: u64,
    pub estimated_monotonic_ns: u64,
    pub discontinuity: AudioDiscontinuity,
}

const EMPTY_SAMPLE_METADATA: ProcessedSampleMetadata = ProcessedSampleMetadata {
    output_sample_index: 0,
    source_first_frame_index: 0,
    estimated_monotonic_ns: 0,
    discontinuity: AudioDiscontinuity::NONE,
};

#[derive(Debug)]
pub struct ProcessedAudioBlock<'a> {
    pub source: AudioChunkMetadata,
    pub first_output_sample_index: u64,
    pub sample_rate_hz: NonZeroU32,
    pub samples: &'a [f32],
}

/// Builds the preallocated SPSC callback/worker pair.
///
/// # Errors
///
/// Returns [`PipelineBuildError`] when the channel plan, pre-roll contract, or
/// preallocated capacities are invalid or cannot be represented.
pub fn build_audio_pipeline(
    config: AudioPipelineConfig,
) -> Result<(CaptureCallback, AudioWorker), PipelineBuildError> {
    config
        .channel_policy
        .validate(config.native.channels())
        .map_err(PipelineBuildError::InvalidChannelPolicy)?;
    if !(250..=500).contains(&config.pre_roll_ms) {
        return Err(PipelineBuildError::PreRollOutsideContract {
            milliseconds: config.pre_roll_ms,
        });
    }
    let native_sample_capacity = config
        .max_native_frames_per_block
        .get()
        .checked_mul(usize::from(config.native.channels().get()))
        .ok_or(PipelineBuildError::CapacityOverflow)?;
    let pre_roll_frames_u64 = u64::from(config.target_sample_rate_hz.get())
        .checked_mul(u64::from(config.pre_roll_ms))
        .ok_or(PipelineBuildError::CapacityOverflow)?
        .div_ceil(1_000);
    let pre_roll_frames =
        usize::try_from(pre_roll_frames_u64).map_err(|_| PipelineBuildError::CapacityOverflow)?;
    if config.utterance_capacity_frames.get() < pre_roll_frames {
        return Err(PipelineBuildError::UtteranceCannotHoldPreRoll {
            required: pre_roll_frames,
            available: config.utterance_capacity_frames.get(),
        });
    }
    let resampled_capacity = max_resampled_frames(
        config.max_native_frames_per_block.get(),
        config.native.sample_rate_hz(),
        config.target_sample_rate_hz,
    )?;
    let duration_limit_frames_u64 = u64::from(config.target_sample_rate_hz.get())
        .checked_mul(u64::from(config.max_utterance_ms.get()))
        .ok_or(PipelineBuildError::CapacityOverflow)?
        / 1_000;
    let duration_limit_frames = usize::try_from(duration_limit_frames_u64)
        .map_err(|_| PipelineBuildError::CapacityOverflow)?;
    let bytes_per_frame = size_of::<f32>()
        .checked_add(size_of::<ProcessedSampleMetadata>())
        .ok_or(PipelineBuildError::CapacityOverflow)?;
    let byte_limit_frames = config.max_buffered_bytes.get() / bytes_per_frame;
    let utterance_limit_frames = config
        .utterance_capacity_frames
        .get()
        .min(duration_limit_frames)
        .min(byte_limit_frames);
    if utterance_limit_frames < pre_roll_frames {
        return Err(PipelineBuildError::UtteranceCannotHoldPreRoll {
            required: pre_roll_frames,
            available: utterance_limit_frames,
        });
    }
    let mut limit_issues = UtteranceIssues::NONE;
    if utterance_limit_frames == duration_limit_frames {
        limit_issues.insert(UtteranceIssues::DURATION_LIMIT);
    }
    if utterance_limit_frames == byte_limit_frames {
        limit_issues.insert(UtteranceIssues::BYTE_LIMIT);
    }
    if utterance_limit_frames == config.utterance_capacity_frames.get() {
        limit_issues.insert(UtteranceIssues::BUFFER_CAPACITY);
    }

    let ring = Ring::new(
        config.stream_id,
        config.native,
        config.callback_block_count,
        config.max_native_frames_per_block,
    );
    let callback = CaptureCallback::new(ring.clone());
    let worker = AudioWorker {
        consumer: RingConsumer::new(ring),
        native_block: CapturedNativeBlock::new(native_sample_capacity),
        mono_scratch: vec![0.0; config.max_native_frames_per_block.get()].into_boxed_slice(),
        resampled_scratch: vec![0.0; resampled_capacity].into_boxed_slice(),
        resampled_len: 0,
        config,
        resampler: StreamingLinearResampler::new(
            config.native.sample_rate_hz(),
            config.target_sample_rate_hz,
        ),
        next_output_sample_index: 0,
        pre_roll: TimedRingBuffer::new(pre_roll_frames),
        utterance: TimedUtteranceBuffer::new(utterance_limit_frames),
        utterance_active: false,
        utterance_issues: UtteranceIssues::NONE,
        utterance_limit_issues: limit_issues,
        callback_overflows_at_start: 0,
        counters: WorkerCounters::default(),
    };
    Ok((callback, worker))
}

fn max_resampled_frames(
    input_frames: usize,
    source_rate: NonZeroU32,
    target_rate: NonZeroU32,
) -> Result<usize, PipelineBuildError> {
    let numerator = (input_frames as u128)
        .checked_mul(u128::from(target_rate.get()))
        .ok_or(PipelineBuildError::CapacityOverflow)?;
    let frames = numerator.div_ceil(u128::from(source_rate.get())) + 2;
    usize::try_from(frames).map_err(|_| PipelineBuildError::CapacityOverflow)
}

pub struct AudioWorker {
    consumer: RingConsumer,
    native_block: CapturedNativeBlock,
    mono_scratch: Box<[f32]>,
    resampled_scratch: Box<[f32]>,
    resampled_len: usize,
    config: AudioPipelineConfig,
    resampler: StreamingLinearResampler,
    next_output_sample_index: u64,
    pre_roll: TimedRingBuffer,
    utterance: TimedUtteranceBuffer,
    utterance_active: bool,
    utterance_issues: UtteranceIssues,
    utterance_limit_issues: UtteranceIssues,
    callback_overflows_at_start: u64,
    counters: WorkerCounters,
}

impl AudioWorker {
    /// Processes one queued native block. No model or VAD work occurs here.
    ///
    /// # Panics
    ///
    /// Panics only if an internal preallocation invariant established by
    /// [`build_audio_pipeline`] is violated.
    pub fn process_next(&mut self) -> Option<ProcessedAudioBlock<'_>> {
        if !self.consumer.try_pop(&mut self.native_block) {
            return None;
        }
        let source = self
            .native_block
            .metadata
            .expect("consumer always sets metadata");
        if !source.discontinuity.is_empty() {
            self.resampler.reset();
            self.counters.discontinuous_blocks += 1;
            self.counters.resampler_resets += 1;
            if self.utterance_active {
                self.utterance_issues.insert(UtteranceIssues::DISCONTINUITY);
            }
        }
        let mono_frames = downmix_into(
            self.native_block.samples(),
            self.config.native.channels(),
            self.config.channel_policy,
            &mut self.mono_scratch,
        )
        .expect("pipeline validates channel policy and scratch capacity");
        self.resampled_len = self
            .resampler
            .process_into(
                &self.mono_scratch[..mono_frames],
                &mut self.resampled_scratch,
            )
            .expect("pipeline precomputes resampler scratch capacity");

        let first_output_sample_index = self.next_output_sample_index;
        let output = &self.resampled_scratch[..self.resampled_len];
        let overwritten = self.pre_roll.push_block(
            output,
            source,
            first_output_sample_index,
            self.config.target_sample_rate_hz,
        );
        self.counters.pre_roll_overwritten_frames += overwritten as u64;
        if self.utterance_active {
            let (accepted, rejected) = self.utterance.append_block(
                output,
                source,
                first_output_sample_index,
                self.config.target_sample_rate_hz,
            );
            self.counters.utterance_frames_buffered += accepted as u64;
            self.counters.utterance_frames_rejected += rejected as u64;
            if rejected > 0 {
                if self.utterance_issues.is_empty() {
                    self.counters.utterance_limit_events += 1;
                }
                self.utterance_issues.insert(self.utterance_limit_issues);
            }
        }
        self.next_output_sample_index = self
            .next_output_sample_index
            .saturating_add(self.resampled_len as u64);
        self.counters.blocks_processed += 1;
        self.counters.native_frames_processed += self.native_block.frames as u64;
        self.counters.resampled_frames_produced += self.resampled_len as u64;

        Some(ProcessedAudioBlock {
            source,
            first_output_sample_index,
            sample_rate_hz: self.config.target_sample_rate_hz,
            samples: output,
        })
    }

    /// Starts an utterance from the currently retained post-activation pre-roll.
    /// Future processed samples append until `end_utterance` is called.
    ///
    /// # Errors
    ///
    /// Returns [`UtteranceStateError::AlreadyActive`] if an utterance is active.
    pub fn begin_utterance(&mut self) -> Result<(), UtteranceStateError> {
        if self.utterance_active {
            return Err(UtteranceStateError::AlreadyActive);
        }
        self.utterance.clear();
        self.utterance_issues = UtteranceIssues::NONE;
        self.callback_overflows_at_start = self.callback_counters().queue_overflows;
        let copied = self.pre_roll.copy_into(&mut self.utterance);
        self.counters.utterances_started += 1;
        self.counters.utterance_frames_buffered += copied as u64;
        self.utterance_active = true;
        Ok(())
    }

    pub fn end_utterance(&mut self) {
        self.utterance_active = false;
    }

    /// Drains all callback blocks published before this call and seals the utterance.
    /// Linear interpolation beyond the final observed native sample is deliberately
    /// discarded rather than synthesizing audio that the device did not capture.
    ///
    /// # Errors
    ///
    /// Returns [`UtteranceStateError::NotActive`] when no utterance is active.
    pub fn finish_utterance(&mut self) -> Result<UtteranceCompletion, UtteranceStateError> {
        if !self.utterance_active {
            return Err(UtteranceStateError::NotActive);
        }
        let blocks_before = self.counters.blocks_processed;
        while self.process_next().is_some() {}
        let drained_blocks = self.counters.blocks_processed.saturating_sub(blocks_before);
        self.counters.final_drain_blocks = self
            .counters
            .final_drain_blocks
            .saturating_add(drained_blocks);
        if self.callback_counters().queue_overflows > self.callback_overflows_at_start {
            self.utterance_issues
                .insert(UtteranceIssues::CALLBACK_QUEUE_OVERFLOW);
        }
        self.utterance_active = false;
        Ok(UtteranceCompletion {
            frames_buffered: self.utterance.samples().len(),
            drained_blocks,
            issues: self.utterance_issues,
            tail_policy: ResamplerTailPolicy::DiscardUnobservedInterpolation,
        })
    }

    pub fn clear_utterance(&mut self) {
        self.utterance_active = false;
        self.utterance.clear();
        self.utterance_issues = UtteranceIssues::NONE;
    }

    pub fn utterance_samples(&self) -> &[f32] {
        self.utterance.samples()
    }

    pub fn utterance_metadata(&self) -> &[ProcessedSampleMetadata] {
        self.utterance.metadata()
    }

    pub fn pre_roll_frames(&self) -> usize {
        self.pre_roll.len
    }

    pub fn callback_counters(&self) -> CallbackCountersSnapshot {
        self.consumer.callback_counters()
    }

    pub fn counters(&self) -> WorkerCountersSnapshot {
        WorkerCountersSnapshot::from(&self.counters)
    }
}

struct StreamingLinearResampler {
    source_rate: u64,
    target_rate: u64,
    total_input_frames: u64,
    next_output_numerator: u128,
    previous: Option<f32>,
}

impl StreamingLinearResampler {
    fn new(source_rate: NonZeroU32, target_rate: NonZeroU32) -> Self {
        Self {
            source_rate: u64::from(source_rate.get()),
            target_rate: u64::from(target_rate.get()),
            total_input_frames: 0,
            next_output_numerator: 0,
            previous: None,
        }
    }

    fn reset(&mut self) {
        self.total_input_frames = 0;
        self.next_output_numerator = 0;
        self.previous = None;
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn process_into(&mut self, input: &[f32], output: &mut [f32]) -> Result<usize, ()> {
        let mut output_len = 0usize;
        for &current in input {
            let current_index = self.total_input_frames;
            if let Some(previous) = self.previous {
                let segment_start =
                    u128::from(current_index.saturating_sub(1)) * u128::from(self.target_rate);
                let segment_end = u128::from(current_index) * u128::from(self.target_rate);
                while self.next_output_numerator <= segment_end {
                    if output_len == output.len() {
                        return Err(());
                    }
                    let fraction_numerator = self.next_output_numerator - segment_start;
                    let fraction = fraction_numerator as f64 / self.target_rate as f64;
                    output[output_len] = previous + (current - previous) * fraction as f32;
                    output_len += 1;
                    self.next_output_numerator += u128::from(self.source_rate);
                }
            } else if self.next_output_numerator == 0 {
                if output.is_empty() {
                    return Err(());
                }
                output[0] = current;
                output_len = 1;
                self.next_output_numerator = u128::from(self.source_rate);
            }
            self.previous = Some(current);
            self.total_input_frames = self.total_input_frames.saturating_add(1);
        }
        Ok(output_len)
    }
}

struct TimedRingBuffer {
    samples: Box<[f32]>,
    metadata: Box<[ProcessedSampleMetadata]>,
    start: usize,
    len: usize,
}

impl TimedRingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0.0; capacity].into_boxed_slice(),
            metadata: vec![EMPTY_SAMPLE_METADATA; capacity].into_boxed_slice(),
            start: 0,
            len: 0,
        }
    }

    fn push_block(
        &mut self,
        samples: &[f32],
        source: AudioChunkMetadata,
        first_output_sample_index: u64,
        target_rate: NonZeroU32,
    ) -> usize {
        let mut overwritten = 0;
        for (offset, &sample) in samples.iter().enumerate() {
            let metadata = sample_metadata(source, first_output_sample_index, offset, target_rate);
            if self.len < self.samples.len() {
                let index = (self.start + self.len) % self.samples.len();
                self.samples[index] = sample;
                self.metadata[index] = metadata;
                self.len += 1;
            } else {
                self.samples[self.start] = sample;
                self.metadata[self.start] = metadata;
                self.start = (self.start + 1) % self.samples.len();
                overwritten += 1;
            }
        }
        overwritten
    }

    fn copy_into(&self, destination: &mut TimedUtteranceBuffer) -> usize {
        for offset in 0..self.len {
            let index = (self.start + offset) % self.samples.len();
            let accepted = destination.push(self.samples[index], self.metadata[index]);
            debug_assert!(accepted, "pipeline requires utterance capacity >= pre-roll");
        }
        self.len
    }
}

struct TimedUtteranceBuffer {
    samples: Box<[f32]>,
    metadata: Box<[ProcessedSampleMetadata]>,
    len: usize,
}

impl TimedUtteranceBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0.0; capacity].into_boxed_slice(),
            metadata: vec![EMPTY_SAMPLE_METADATA; capacity].into_boxed_slice(),
            len: 0,
        }
    }

    fn push(&mut self, sample: f32, metadata: ProcessedSampleMetadata) -> bool {
        if self.len == self.samples.len() {
            return false;
        }
        self.samples[self.len] = sample;
        self.metadata[self.len] = metadata;
        self.len += 1;
        true
    }

    fn append_block(
        &mut self,
        samples: &[f32],
        source: AudioChunkMetadata,
        first_output_sample_index: u64,
        target_rate: NonZeroU32,
    ) -> (usize, usize) {
        let available = self.samples.len() - self.len;
        let accepted = available.min(samples.len());
        for (offset, &sample) in samples[..accepted].iter().enumerate() {
            let metadata = sample_metadata(source, first_output_sample_index, offset, target_rate);
            let pushed = self.push(sample, metadata);
            debug_assert!(pushed);
        }
        (accepted, samples.len() - accepted)
    }

    fn clear(&mut self) {
        self.samples[..self.len].fill(0.0);
        self.metadata[..self.len].fill(EMPTY_SAMPLE_METADATA);
        self.len = 0;
    }

    fn samples(&self) -> &[f32] {
        &self.samples[..self.len]
    }

    fn metadata(&self) -> &[ProcessedSampleMetadata] {
        &self.metadata[..self.len]
    }
}

fn sample_metadata(
    source: AudioChunkMetadata,
    first_output_sample_index: u64,
    offset: usize,
    target_rate: NonZeroU32,
) -> ProcessedSampleMetadata {
    let offset_u64 = offset as u64;
    ProcessedSampleMetadata {
        output_sample_index: first_output_sample_index.saturating_add(offset_u64),
        source_first_frame_index: source.timestamp.first_frame_index,
        estimated_monotonic_ns: source.timestamp.capture_monotonic_ns.saturating_add(
            offset_u64.saturating_mul(1_000_000_000) / u64::from(target_rate.get()),
        ),
        discontinuity: if offset == 0 {
            source.discontinuity
        } else {
            AudioDiscontinuity::NONE
        },
    }
}
