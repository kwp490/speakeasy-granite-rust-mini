use std::cmp::Ordering;
use std::num::{NonZeroU16, NonZeroU32};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSampleFormat {
    F32,
    I16,
    U16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeStreamConfig {
    sample_format: NativeSampleFormat,
    sample_rate_hz: NonZeroU32,
    channels: NonZeroU16,
}

impl NativeStreamConfig {
    pub const fn new(
        sample_format: NativeSampleFormat,
        sample_rate_hz: NonZeroU32,
        channels: NonZeroU16,
    ) -> Self {
        Self {
            sample_format,
            sample_rate_hz,
            channels,
        }
    }

    pub const fn sample_format(self) -> NativeSampleFormat {
        self.sample_format
    }

    pub const fn sample_rate_hz(self) -> NonZeroU32 {
        self.sample_rate_hz
    }

    pub const fn channels(self) -> NonZeroU16 {
        self.channels
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeStreamCandidate {
    pub config: NativeStreamConfig,
    pub is_device_default: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFormatPreference {
    PreferF32,
    PreferI16,
    PreferU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiationPreference {
    /// A device-native preference, not an engine-rate request.
    pub preferred_sample_rate_hz: Option<NonZeroU32>,
    /// Mono is preferred. If unavailable, the fewest-channel native format wins.
    pub prefer_mono: bool,
    pub sample_format: NativeFormatPreference,
}

impl Default for NegotiationPreference {
    fn default() -> Self {
        Self {
            preferred_sample_rate_hz: None,
            prefer_mono: true,
            sample_format: NativeFormatPreference::PreferF32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationError {
    NoSupportedNativeFormat,
}

/// Selects one concrete supported native stream format.
///
/// # Errors
///
/// Returns [`NegotiationError::NoSupportedNativeFormat`] when `candidates` is empty.
pub fn negotiate_native_format(
    candidates: &[NativeStreamCandidate],
    preference: NegotiationPreference,
) -> Result<NativeStreamConfig, NegotiationError> {
    candidates
        .iter()
        .min_by(|left, right| compare_candidates(left, right, preference))
        .map(|candidate| candidate.config)
        .ok_or(NegotiationError::NoSupportedNativeFormat)
}

fn compare_candidates(
    left: &NativeStreamCandidate,
    right: &NativeStreamCandidate,
    preference: NegotiationPreference,
) -> Ordering {
    candidate_rank(*left, preference).cmp(&candidate_rank(*right, preference))
}

fn candidate_rank(
    candidate: NativeStreamCandidate,
    preference: NegotiationPreference,
) -> (u16, u8, u32, u8, u32) {
    let config = candidate.config;
    let channels = config.channels().get();
    let channel_rank = if preference.prefer_mono && channels == 1 {
        0
    } else {
        channels
    };
    let default_rank = u8::from(!candidate.is_device_default);
    let rate_rank = preference.preferred_sample_rate_hz.map_or(0, |preferred| {
        config.sample_rate_hz().get().abs_diff(preferred.get())
    });
    let format_rank = match (preference.sample_format, config.sample_format()) {
        (NativeFormatPreference::PreferF32, NativeSampleFormat::F32)
        | (NativeFormatPreference::PreferI16, NativeSampleFormat::I16)
        | (NativeFormatPreference::PreferU16, NativeSampleFormat::U16) => 0,
        _ => 1,
    };
    (
        channel_rank,
        default_rank,
        rate_rank,
        format_rank,
        config.sample_rate_hz().get(),
    )
}

/// Downmixing is explicit. Microphone-array channels are never averaged by default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelPolicy {
    Select { channel_index: u16 },
    AverageAllExplicit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatError {
    ChannelOutOfRange { requested: u16, available: u16 },
    InterleavedLength { samples: usize, channels: u16 },
    OutputTooSmall { required: usize, available: usize },
}

impl ChannelPolicy {
    pub(crate) fn validate(self, channels: NonZeroU16) -> Result<(), FormatError> {
        if let Self::Select { channel_index } = self
            && channel_index >= channels.get()
        {
            return Err(FormatError::ChannelOutOfRange {
                requested: channel_index,
                available: channels.get(),
            });
        }
        Ok(())
    }
}

pub(crate) fn downmix_into(
    interleaved: &[f32],
    channels: NonZeroU16,
    policy: ChannelPolicy,
    output: &mut [f32],
) -> Result<usize, FormatError> {
    policy.validate(channels)?;
    let channel_count = usize::from(channels.get());
    if !interleaved.len().is_multiple_of(channel_count) {
        return Err(FormatError::InterleavedLength {
            samples: interleaved.len(),
            channels: channels.get(),
        });
    }
    let frames = interleaved.len() / channel_count;
    if output.len() < frames {
        return Err(FormatError::OutputTooSmall {
            required: frames,
            available: output.len(),
        });
    }

    match policy {
        ChannelPolicy::Select { channel_index } => {
            let channel_index = usize::from(channel_index);
            for (destination, frame) in output
                .iter_mut()
                .zip(interleaved.chunks_exact(channel_count))
            {
                *destination = frame[channel_index];
            }
        }
        ChannelPolicy::AverageAllExplicit => {
            let divisor = f32::from(channels.get());
            for (destination, frame) in output
                .iter_mut()
                .zip(interleaved.chunks_exact(channel_count))
            {
                *destination = frame.iter().copied().sum::<f32>() / divisor;
            }
        }
    }
    Ok(frames)
}
