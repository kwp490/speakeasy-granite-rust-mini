macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn into_bytes(self) -> [u8; 16] {
                self.0
            }
        }
    };
}

opaque_id!(CorrelationId);
opaque_id!(ProducerId);
opaque_id!(SessionId);

/// Monotonically increasing identity assigned after an utterance has been
/// finalized and is ready for inference.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtteranceId(u64);

impl UtteranceId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}
