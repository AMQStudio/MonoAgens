//! Opaque protocol identifiers and timestamp newtypes.
//!
//! Identifiers are never parsed for platform structure; equality and hashing
//! operate solely on the opaque value.

use core::fmt;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates a new opaque identifier from any string-like value.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the raw identifier string slice without interpreting it.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the wrapper and returns the inner `String`.
            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Opaque identifier of an actor (person, soul, or external participant).
    ActorId
);
string_id!(
    /// Opaque identifier of a place where percepts originate or actions land.
    PlaceId
);
string_id!(
    /// Opaque identifier of a declared body capability.
    CapabilityId
);
string_id!(
    /// Opaque identifier of a dispatched action.
    ActionId
);
string_id!(
    /// Opaque correlation identifier linking an action and its outcome/reply.
    CorrelationId
);
string_id!(
    /// Opaque identifier of a sensed effect channel.
    ChannelId
);
string_id!(
    /// Opaque identifier of a connected body adapter.
    BodyId
);
string_id!(
    /// Opaque identifier of a belief/expectation domain.
    DomainId
);
string_id!(
    /// Deterministic hash of a capability's input/output schema (`R-38`).
    CapabilitySchemaHash
);

/// Opaque monotonic event sequence identifier in the core log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct EventId(pub u64);

impl EventId {
    /// Creates a new `EventId`.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the numeric identifier.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for EventId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Millisecond timestamp from an injected clock (`u64` ms since epoch).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Time(pub u64);

impl Time {
    /// Creates a `Time` from milliseconds.
    #[must_use]
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    /// Returns milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Saturating addition in milliseconds.
    #[must_use]
    pub const fn saturating_add_ms(self, delta_ms: u64) -> Self {
        Self(self.0.saturating_add(delta_ms))
    }

    /// Saturating subtraction returning elapsed milliseconds.
    #[must_use]
    pub const fn saturating_sub_ms(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl From<u64> for Time {
    fn from(ms: u64) -> Self {
        Self(ms)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}
