use std::num::NonZeroU32;
use std::ops::{BitOr, BitOrAssign};

/// Schema version for audio metadata crossing adapter/worker boundaries.
pub const AUDIO_CONTRACT_SCHEMA_VERSION: u16 = 1;

/// Opaque identity supplied by the capture-session owner.
///
/// This type deliberately does not generate identifiers: the future coordinator
/// owns collision-resistant session identity and passes it into the audio adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CaptureStreamId([u8; 16]);

impl CaptureStreamId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Discontinuities that must remain visible after the real-time callback.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AudioDiscontinuity(u8);

impl AudioDiscontinuity {
    pub const NONE: Self = Self(0);
    pub const DEVICE_REPORTED: Self = Self(1 << 0);
    pub const SAMPLE_CLOCK_GAP: Self = Self(1 << 1);
    pub const CALLBACK_QUEUE_OVERFLOW: Self = Self(1 << 2);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & 0b111)
    }
}

impl BitOr for AudioDiscontinuity {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for AudioDiscontinuity {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Timestamp anchored in both the source sample clock and its monotonic clock.
///
/// Monotonic values from different producers/processes are never comparable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioTimestamp {
    pub sample_rate_hz: NonZeroU32,
    pub first_frame_index: u64,
    pub capture_monotonic_ns: u64,
}

/// Metadata attached to each bounded callback block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioChunkMetadata {
    pub schema_version: u16,
    pub stream_id: CaptureStreamId,
    pub source_sequence: u64,
    pub timestamp: AudioTimestamp,
    pub discontinuity: AudioDiscontinuity,
}

impl AudioChunkMetadata {
    pub const fn new(
        stream_id: CaptureStreamId,
        source_sequence: u64,
        timestamp: AudioTimestamp,
        discontinuity: AudioDiscontinuity,
    ) -> Self {
        Self {
            schema_version: AUDIO_CONTRACT_SCHEMA_VERSION,
            stream_id,
            source_sequence,
            timestamp,
            discontinuity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discontinuity_flags_compose_without_unknown_bits() {
        let flags = AudioDiscontinuity::DEVICE_REPORTED | AudioDiscontinuity::SAMPLE_CLOCK_GAP;
        assert!(flags.contains(AudioDiscontinuity::DEVICE_REPORTED));
        assert!(flags.contains(AudioDiscontinuity::SAMPLE_CLOCK_GAP));
        assert!(!flags.contains(AudioDiscontinuity::CALLBACK_QUEUE_OVERFLOW));
        assert_eq!(AudioDiscontinuity::from_bits_truncate(0xff).bits(), 0b111);
    }
}
