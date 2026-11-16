//! Protocol versioning and additive compatibility checks.
//!
//! Compatibility rule: `(M_msg == M_gw) && (m_msg <= m_gw)`.
//! Older minor versions are accepted; newer or mismatched major versions fail
//! loudly with an error naming both versions.

use core::fmt;
use serde::{Deserialize, Serialize};

/// Current Waist 1 / Waist 2 protocol version supported by this build.
pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

/// Semantic major/minor protocol version carried by every message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(transform = crate::schema::transform_protocol_version)
)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    /// Constructs a `ProtocolVersion`.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Returns `true` when `self` (message version) is readable by `supported` (reader version):
    /// `self.major == supported.major && self.minor <= supported.minor`.
    #[must_use]
    pub const fn is_compatible_with(self, supported: Self) -> bool {
        self.major == supported.major && self.minor <= supported.minor
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        CURRENT_PROTOCOL_VERSION
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Error returned when a message declares an unsupported protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionError {
    pub received: ProtocolVersion,
    pub supported: ProtocolVersion,
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported protocol version {} (supported {})",
            self.received, self.supported
        )
    }
}

impl std::error::Error for VersionError {}

/// Validates that `received` is compatible with `supported`.
pub const fn check_version(
    received: ProtocolVersion,
    supported: ProtocolVersion,
) -> Result<(), VersionError> {
    if received.is_compatible_with(supported) {
        Ok(())
    } else {
        Err(VersionError {
            received,
            supported,
        })
    }
}
