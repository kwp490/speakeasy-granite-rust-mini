use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};

use super::*;

fn nz16(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).expect("test values are non-zero")
}

fn nz32(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test values are non-zero")
}

fn nz_usize(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("test values are non-zero")
}

fn config(
    format: NativeSampleFormat,
    source_rate: u32,
    target_rate: u32,
    channels: u16,
    frames_per_block: usize,
) -> AudioPipelineConfig {
    AudioPipelineConfig {
        stream_id: CaptureStreamId::from_bytes([7; 16]),
        native: NativeStreamConfig::new(format, nz32(source_rate), nz16(channels)),
        target_sample_rate_hz: nz32(target_rate),
        channel_policy: ChannelPolicy::Select { channel_index: 0 },
        callback_block_count: nz_usize(8),
        max_native_frames_per_block: nz_usize(frames_per_block),
        pre_roll_ms: 250,
        utterance_capacity_frames: nz_usize(target_rate as usize),
        max_utterance_ms: nz32(1_000),
        max_buffered_bytes: nz_usize(64 * 1024 * 1024),
    }
}

fn stamp(first_frame_index: u64) -> CallbackStamp {
    CallbackStamp {
        first_frame_index,
        capture_monotonic_ns: 1_000_000_000
            + first_frame_index.saturating_mul(1_000_000_000) / 16_000,
        discontinuity: AudioDiscontinuity::NONE,
    }
}

#[test]
fn negotiation_prefers_mono_without_requesting_engine_rate() {
    let candidates = [
        NativeStreamCandidate {
            config: NativeStreamConfig::new(NativeSampleFormat::F32, nz32(48_000), nz16(2)),
            is_device_default: true,
        },
        NativeStreamCandidate {
            config: NativeStreamConfig::new(NativeSampleFormat::I16, nz32(44_100), nz16(1)),
            is_device_default: false,
        },
    ];
    let selected = negotiate_native_format(&candidates, NegotiationPreference::default()).unwrap();
    assert_eq!(selected.channels().get(), 1);
    assert_eq!(selected.sample_rate_hz().get(), 44_100);
    assert_eq!(selected.sample_format(), NativeSampleFormat::I16);
}

#[test]
fn all_native_formats_convert_with_defined_saturation() {
    let cases = [
        (NativeSampleFormat::F32, vec![-1.0, 0.0, 1.0]),
        (
            NativeSampleFormat::I16,
            vec![-1.0, 0.0, 32_767.0 / 32_768.0],
        ),
        (
            NativeSampleFormat::U16,
            vec![-1.0, 0.0, 32_767.0 / 32_768.0],
        ),
    ];
    for (format, expected) in cases {
        let (mut callback, mut worker) =
            build_audio_pipeline(config(format, 16_000, 16_000, 1, 8)).unwrap();
        match format {
            NativeSampleFormat::F32 => {
                callback.write_f32(&[-2.0, 0.0, 2.0], stamp(0));
            }
            NativeSampleFormat::I16 => {
                callback.write_i16(&[i16::MIN, 0, i16::MAX], stamp(0));
            }
            NativeSampleFormat::U16 => {
                callback.write_u16(&[u16::MIN, 32_768, u16::MAX], stamp(0));
            }
        }
        let block = worker.process_next().unwrap();
        assert_eq!(block.samples, expected);
        if format == NativeSampleFormat::F32 {
            assert_eq!(worker.callback_counters().clipped_samples, 2);
        }
    }
}

#[test]
fn callback_rejects_wrong_format_and_malformed_interleaving() {
    let mut pipeline_config = config(NativeSampleFormat::I16, 16_000, 16_000, 2, 8);
    pipeline_config.channel_policy = ChannelPolicy::Select { channel_index: 1 };
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();

    let wrong_format = callback.write_f32(&[0.0, 0.0], stamp(0));
    assert_eq!(
        wrong_format.status,
        CallbackWriteStatus::UnexpectedSampleFormat
    );
    let malformed = callback.write_i16(&[0, 1, 2], stamp(0));
    assert_eq!(malformed.status, CallbackWriteStatus::InvalidInterleaving);
    assert!(worker.process_next().is_none());
    let counters = callback.counters();
    assert_eq!(counters.sample_format_mismatches, 1);
    assert_eq!(counters.malformed_callbacks, 1);
}

#[test]
fn callback_splits_without_growing_and_marks_overflow_on_recovery() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 16_000, 16_000, 1, 2);
    pipeline_config.callback_block_count = nz_usize(1);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();

    let first = callback.write_f32(&[0.1, 0.2, 0.3, 0.4], stamp(0));
    assert_eq!(first.accepted_frames, 2);
    assert_eq!(first.dropped_frames, 2);
    assert_eq!(first.status, CallbackWriteStatus::QueueFull);
    assert_eq!(callback.counters().queue_depth_blocks, 1);

    {
        let first_block = worker.process_next().unwrap();
        assert!(first_block.source.discontinuity.is_empty());
    }
    let recovered = callback.write_f32(&[0.5, 0.6], stamp(4));
    assert_eq!(recovered.status, CallbackWriteStatus::Complete);
    let recovered_block = worker.process_next().unwrap();
    assert!(
        recovered_block
            .source
            .discontinuity
            .contains(AudioDiscontinuity::CALLBACK_QUEUE_OVERFLOW)
    );
    let counters = callback.counters();
    assert_eq!(counters.native_frames_dropped, 2);
    assert_eq!(counters.queue_overflows, 1);
}

#[test]
fn callback_allocates_nothing_and_does_not_wait_for_queue_space() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 48_000, 16_000, 1, 4);
    pipeline_config.callback_block_count = nz_usize(1);
    pipeline_config.max_native_frames_per_block = nz_usize(4);
    let (mut callback, _worker) = build_audio_pipeline(pipeline_config).unwrap();

    let allocation_info = allocation_counter::measure(|| {
        let first = callback.write_f32(&[0.1; 4], stamp(0));
        assert_eq!(first.status, CallbackWriteStatus::Complete);
        let full = callback.write_f32(&[0.2; 4], stamp(4));
        assert_eq!(full.status, CallbackWriteStatus::QueueFull);
        assert_eq!(full.dropped_frames, 4);
    });

    assert_eq!(allocation_info.count_total, 0);
    assert_eq!(allocation_info.bytes_total, 0);
}

#[test]
fn callback_block_split_propagates_sample_and_monotonic_offsets() {
    let (mut callback, mut worker) =
        build_audio_pipeline(config(NativeSampleFormat::F32, 8_000, 8_000, 1, 2)).unwrap();
    callback.write_f32(
        &[0.0, 0.1, 0.2, 0.3],
        CallbackStamp {
            first_frame_index: 10,
            capture_monotonic_ns: 2_000_000_000,
            discontinuity: AudioDiscontinuity::DEVICE_REPORTED,
        },
    );
    {
        let first = worker.process_next().unwrap();
        assert_eq!(first.source.timestamp.first_frame_index, 10);
        assert_eq!(first.source.timestamp.capture_monotonic_ns, 2_000_000_000);
        assert!(
            first
                .source
                .discontinuity
                .contains(AudioDiscontinuity::DEVICE_REPORTED)
        );
    }
    let second = worker.process_next().unwrap();
    assert_eq!(second.source.timestamp.first_frame_index, 12);
    assert_eq!(second.source.timestamp.capture_monotonic_ns, 2_000_250_000);
    assert!(second.source.discontinuity.is_empty());
}

#[test]
fn channel_selection_avoids_blind_phase_cancellation() {
    let mut selected_config = config(NativeSampleFormat::F32, 16_000, 16_000, 2, 4);
    selected_config.channel_policy = ChannelPolicy::Select { channel_index: 0 };
    let (mut callback, mut worker) = build_audio_pipeline(selected_config).unwrap();
    callback.write_f32(&[0.5, -0.5, 0.25, -0.25], stamp(0));
    assert_eq!(worker.process_next().unwrap().samples, &[0.5, 0.25]);

    let mut averaged_config = selected_config;
    averaged_config.channel_policy = ChannelPolicy::AverageAllExplicit;
    let (mut callback, mut worker) = build_audio_pipeline(averaged_config).unwrap();
    callback.write_f32(&[0.5, -0.5, 0.25, -0.25], stamp(0));
    assert_eq!(worker.process_next().unwrap().samples, &[0.0, 0.0]);
}

#[test]
fn configured_channel_selection_handles_multichannel_input() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 16_000, 16_000, 4, 4);
    pipeline_config.channel_policy = ChannelPolicy::Select { channel_index: 2 };
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();
    callback.write_f32(&[0.1, 0.2, 0.3, 0.4, -0.1, -0.2, -0.3, -0.4], stamp(0));
    assert_eq!(worker.process_next().unwrap().samples, &[0.3, -0.3]);
}

#[test]
fn spsc_endpoints_transfer_blocks_across_threads() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 16_000, 16_000, 1, 4);
    pipeline_config.callback_block_count = nz_usize(128);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();
    let producer = std::thread::spawn(move || {
        for block_index in 0..100u64 {
            let value = if block_index.is_multiple_of(2) {
                0.25
            } else {
                -0.25
            };
            let write = callback.write_f32(&[value; 4], stamp(block_index * 4));
            assert_eq!(write.status, CallbackWriteStatus::Complete);
        }
        callback.counters()
    });

    let mut blocks = 0usize;
    while blocks < 100 {
        if let Some(block) = worker.process_next() {
            assert_eq!(block.samples.len(), 4);
            blocks += 1;
        } else {
            std::thread::yield_now();
        }
    }
    let counters = producer.join().unwrap();
    assert_eq!(counters.blocks_enqueued, 100);
    assert_eq!(worker.counters().blocks_processed, 100);
}

#[test]
fn streaming_resampling_is_chunk_invariant_for_required_rates() {
    for source_rate in [8_000, 16_000, 44_100, 48_000] {
        let frames = source_rate as usize / 100;
        let input: Vec<f32> = (0..frames)
            .map(|index| match index % 4 {
                0 => -0.75,
                1 => -0.25,
                2 => 0.25,
                _ => 0.75,
            })
            .collect();

        let mut one_config = config(NativeSampleFormat::F32, source_rate, 16_000, 1, frames);
        one_config.pre_roll_ms = 500;
        let (mut one_callback, mut one_worker) = build_audio_pipeline(one_config).unwrap();
        one_callback.write_f32(&input, stamp(0));
        let one = one_worker.process_next().unwrap().samples.to_vec();

        let split_at = frames / 2;
        let mut split_config = one_config;
        split_config.max_native_frames_per_block = nz_usize(split_at.max(1));
        let (mut split_callback, mut split_worker) = build_audio_pipeline(split_config).unwrap();
        split_callback.write_f32(&input, stamp(0));
        let mut split = Vec::new();
        while let Some(block) = split_worker.process_next() {
            split.extend_from_slice(block.samples);
        }
        assert_eq!(split, one, "source rate {source_rate}");
    }
}

#[test]
fn discontinuity_resets_resampling_and_remains_in_processed_metadata() {
    let (mut callback, mut worker) =
        build_audio_pipeline(config(NativeSampleFormat::F32, 48_000, 16_000, 1, 8)).unwrap();
    callback.write_f32(&[0.25; 4], stamp(0));
    callback.write_f32(
        &[0.75; 4],
        CallbackStamp {
            first_frame_index: 10,
            capture_monotonic_ns: 2_000_000_000,
            discontinuity: AudioDiscontinuity::DEVICE_REPORTED,
        },
    );
    worker.process_next().unwrap();
    {
        let second = worker.process_next().unwrap();
        assert!(
            second
                .source
                .discontinuity
                .contains(AudioDiscontinuity::DEVICE_REPORTED)
        );
        assert!(
            second
                .source
                .discontinuity
                .contains(AudioDiscontinuity::SAMPLE_CLOCK_GAP)
        );
        assert_eq!(second.samples.first(), Some(&0.75));
    }
    assert_eq!(worker.counters().resampler_resets, 1);
}

#[test]
fn pre_roll_is_chronological_and_utterance_appends_live_audio() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 1_000, 1_000, 1, 200);
    pipeline_config.utterance_capacity_frames = nz_usize(600);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();

    callback.write_f32(
        &(0..300)
            .map(|value| f32::from(u16::try_from(value).unwrap()) / 1_000.0)
            .collect::<Vec<_>>(),
        stamp(0),
    );
    while worker.process_next().is_some() {}
    assert_eq!(worker.pre_roll_frames(), 250);
    worker.begin_utterance().unwrap();
    assert_eq!(worker.utterance_samples().first(), Some(&0.05));
    assert_eq!(worker.utterance_samples().last(), Some(&0.299));

    callback.write_f32(&[0.3, 0.301], stamp(300));
    worker.process_next().unwrap();
    assert_eq!(
        &worker.utterance_samples()[248..],
        &[0.298, 0.299, 0.3, 0.301]
    );
    assert_eq!(
        worker.utterance_metadata().len(),
        worker.utterance_samples().len()
    );
}

#[test]
fn invalid_configuration_fails_before_callback_creation() {
    let mut bad_pre_roll = config(NativeSampleFormat::F32, 16_000, 16_000, 1, 8);
    bad_pre_roll.pre_roll_ms = 249;
    assert!(matches!(
        build_audio_pipeline(bad_pre_roll),
        Err(PipelineBuildError::PreRollOutsideContract { .. })
    ));

    let mut bad_channel = config(NativeSampleFormat::F32, 16_000, 16_000, 2, 8);
    bad_channel.channel_policy = ChannelPolicy::Select { channel_index: 2 };
    assert!(matches!(
        build_audio_pipeline(bad_channel),
        Err(PipelineBuildError::InvalidChannelPolicy(
            FormatError::ChannelOutOfRange { .. }
        ))
    ));
}

#[test]
fn final_drain_preserves_queued_audio_and_reports_tail_policy() {
    let (mut callback, mut worker) =
        build_audio_pipeline(config(NativeSampleFormat::F32, 16_000, 16_000, 1, 4)).unwrap();
    worker.begin_utterance().unwrap();
    callback.write_f32(&[0.1, 0.2, 0.3, 0.4], stamp(0));

    let completion = worker.finish_utterance().unwrap();

    assert_eq!(completion.frames_buffered, 4);
    assert_eq!(completion.drained_blocks, 1);
    assert!(completion.issues.is_empty());
    assert_eq!(
        completion.tail_policy,
        ResamplerTailPolicy::DiscardUnobservedInterpolation
    );
    assert_eq!(worker.utterance_samples(), &[0.1, 0.2, 0.3, 0.4]);
}

#[test]
fn utterance_limits_preserve_bounded_audio_and_return_recoverable_issues() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 1_000, 1_000, 1, 400);
    pipeline_config.utterance_capacity_frames = nz_usize(600);
    pipeline_config.max_utterance_ms = nz32(300);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();
    worker.begin_utterance().unwrap();
    callback.write_f32(&vec![0.25; 400], stamp(0));

    let completion = worker.finish_utterance().unwrap();

    assert_eq!(completion.frames_buffered, 300);
    assert!(completion.issues.contains(UtteranceIssues::DURATION_LIMIT));
    assert_eq!(worker.counters().utterance_frames_rejected, 100);
    assert_eq!(worker.counters().utterance_limit_events, 1);
}

#[test]
fn callback_overflow_is_reported_even_without_a_recovery_block() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 1_000, 1_000, 1, 2);
    pipeline_config.callback_block_count = nz_usize(1);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();
    worker.begin_utterance().unwrap();
    let write = callback.write_f32(&[0.1, 0.2, 0.3, 0.4], stamp(0));
    assert_eq!(write.status, CallbackWriteStatus::QueueFull);

    let completion = worker.finish_utterance().unwrap();

    assert_eq!(completion.frames_buffered, 2);
    assert!(
        completion
            .issues
            .contains(UtteranceIssues::CALLBACK_QUEUE_OVERFLOW)
    );
}

#[test]
fn two_hundred_fixture_activations_preserve_first_word_and_bounds() {
    let mut pipeline_config = config(NativeSampleFormat::F32, 1_000, 1_000, 1, 300);
    pipeline_config.callback_block_count = nz_usize(4);
    pipeline_config.utterance_capacity_frames = nz_usize(600);
    let (mut callback, mut worker) = build_audio_pipeline(pipeline_config).unwrap();
    let mut first_frame_index = 0u64;

    for activation in 0u16..200 {
        let marker = 0.1 + f32::from(activation) / 1_000.0;
        let pre_vad = vec![marker; 300];
        callback.write_f32(&pre_vad, stamp(first_frame_index));
        first_frame_index += pre_vad.len() as u64;
        while worker.process_next().is_some() {}

        worker.begin_utterance().unwrap();
        assert_eq!(worker.utterance_samples().first(), Some(&marker));
        callback.write_f32(&[marker; 20], stamp(first_frame_index));
        first_frame_index += 20;
        let completion = worker.finish_utterance().unwrap();

        assert_eq!(completion.frames_buffered, 270);
        assert!(completion.issues.is_empty());
        assert_eq!(worker.callback_counters().queue_depth_blocks, 0);
        worker.clear_utterance();
    }

    assert_eq!(worker.counters().utterances_started, 200);
    assert_eq!(worker.callback_counters().queue_overflows, 0);
}
