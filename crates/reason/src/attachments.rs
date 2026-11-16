//! Blind [`AttachmentHandle`] resolution strictly after [`VisibleScope`] and context-door checks.
//!
//! Raw media bytes never enter Core (`crates/core`) and never leave the gateway/provider
//! path in a plain serializable struct. A denied handle fails closed with a typed
//! [`AttachmentError`] without touching the underlying blob store.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use monoagens_core::DoorDecision;
use monoagens_protocol::{AttachmentHandle, Audience, EventId, ModalitySlot, Time, VisibleScope};

/// Typed errors produced during blind [`AttachmentHandle`] resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentError {
    Expired {
        handle_id: String,
        expires_at_ms: Time,
        now: Time,
    },
    ScopeDenied {
        handle_id: String,
    },
    DoorDenied {
        handle_id: String,
        verdict: &'static str,
        rule: String,
        reason: String,
    },
    NotFound {
        handle_id: String,
    },
    ByteLengthMismatch {
        handle_id: String,
        expected: u64,
        actual: u64,
    },
    StoreBackend {
        handle_id: String,
        detail: String,
    },
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired {
                handle_id,
                expires_at_ms,
                now,
            } => write!(
                f,
                "attachment '{handle_id}' expired at {expires_at_ms} (now {now})"
            ),
            Self::ScopeDenied { handle_id } => {
                write!(f, "attachment '{handle_id}' denied by VisibleScope")
            }
            Self::DoorDenied {
                handle_id,
                verdict,
                rule,
                reason,
            } => write!(
                f,
                "attachment '{handle_id}' denied by context door ({verdict}, rule '{rule}'): {reason}"
            ),
            Self::NotFound { handle_id } => write!(f, "attachment '{handle_id}' not found"),
            Self::ByteLengthMismatch {
                handle_id,
                expected,
                actual,
            } => write!(
                f,
                "attachment '{handle_id}' byte length mismatch: expected {expected}, got {actual}"
            ),
            Self::StoreBackend { handle_id, detail } => {
                write!(f, "attachment '{handle_id}' store failure: {detail}")
            }
        }
    }
}

impl std::error::Error for AttachmentError {}

/// Synchronous blob store port queried ONLY after `VisibleScope` and door authorization pass.
pub trait AttachmentBlobStore {
    /// Fetches raw bytes for an already-authorized [`AttachmentHandle`].
    fn fetch_bytes(&self, handle: &AttachmentHandle) -> Result<Vec<u8>, AttachmentError>;
}

/// Deterministic in-memory [`AttachmentBlobStore`] for offline tests and Lab harnesses.
#[derive(Debug, Default)]
pub struct InMemoryAttachmentStore {
    blobs: BTreeMap<String, Vec<u8>>,
    fetch_attempts: AtomicUsize,
}

impl InMemoryAttachmentStore {
    /// Creates an empty in-memory attachment store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts raw bytes for `handle_id`.
    pub fn insert(&mut self, handle_id: impl Into<String>, bytes: Vec<u8>) {
        self.blobs.insert(handle_id.into(), bytes);
    }

    /// Returns the number of times [`AttachmentBlobStore::fetch_bytes`] was actually invoked.
    #[must_use]
    pub fn fetch_attempts(&self) -> usize {
        self.fetch_attempts.load(Ordering::Relaxed)
    }
}

impl AttachmentBlobStore for InMemoryAttachmentStore {
    fn fetch_bytes(&self, handle: &AttachmentHandle) -> Result<Vec<u8>, AttachmentError> {
        self.fetch_attempts.fetch_add(1, Ordering::Relaxed);
        self.blobs
            .get(&handle.handle_id)
            .cloned()
            .ok_or_else(|| AttachmentError::NotFound {
                handle_id: handle.handle_id.clone(),
            })
    }
}

/// Encapsulated resolved media bytes authorized for a single provider request.
///
/// Intentionally does not implement `Serialize`, `Deserialize`, or expose raw bytes
/// as public fields; `Debug` redacts payload bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedAttachment {
    handle_id: String,
    modality_slot: ModalitySlot,
    mime_type: String,
    bytes: Vec<u8>,
}

impl ResolvedAttachment {
    /// Returns the opaque handle id.
    #[must_use]
    pub fn handle_id(&self) -> &str {
        &self.handle_id
    }

    /// Returns the modality slot.
    #[must_use]
    pub const fn modality_slot(&self) -> ModalitySlot {
        self.modality_slot
    }

    /// Returns the MIME type inferred or supplied for the attachment.
    #[must_use]
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// Returns the byte length without exposing payload bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Encodes payload as base64 solely for outbound provider transport.
    pub(crate) fn encode_base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }

    /// Encodes payload as a `data:<mime>;base64,...` URI solely for outbound multimodal provider transport.
    pub(crate) fn to_data_uri(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.encode_base64())
    }
}

impl fmt::Debug for ResolvedAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedAttachment")
            .field("handle_id", &self.handle_id)
            .field("modality_slot", &self.modality_slot)
            .field("mime_type", &self.mime_type)
            .field("byte_len", &self.bytes.len())
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

/// Authorization context required before resolving any [`AttachmentHandle`].
#[derive(Debug, Clone, Copy)]
pub struct AttachmentAccessContext<'a> {
    pub visible_scope: &'a VisibleScope,
    pub door_decision: &'a DoorDecision,
    pub item_audience: &'a Audience,
    pub item_event_id: Option<EventId>,
    pub now: Time,
}

/// Default MIME type associated with a [`ModalitySlot`].
#[must_use]
pub const fn default_mime_for_modality(slot: ModalitySlot) -> &'static str {
    match slot {
        ModalitySlot::Image => "image/png",
        ModalitySlot::AudioVoice => "audio/wav",
        ModalitySlot::Video => "video/mp4",
        ModalitySlot::SensorBlob => "application/octet-stream",
    }
}

/// Verifies TTL, [`VisibleScope`], and context-door authorization BEFORE reading blob bytes.
pub fn resolve_authorized_attachment(
    handle: &AttachmentHandle,
    access: AttachmentAccessContext<'_>,
    store: &dyn AttachmentBlobStore,
    mime_override: Option<&str>,
) -> Result<ResolvedAttachment, AttachmentError> {
    if access.now > handle.expires_at_ms {
        return Err(AttachmentError::Expired {
            handle_id: handle.handle_id.clone(),
            expires_at_ms: handle.expires_at_ms,
            now: access.now,
        });
    }

    if !access
        .visible_scope
        .allows(access.item_audience, access.item_event_id, access.now)
    {
        return Err(AttachmentError::ScopeDenied {
            handle_id: handle.handle_id.clone(),
        });
    }

    match access.door_decision {
        DoorDecision::Allow => {}
        DoorDecision::Strip { removed_ids, rule } => {
            let stripped = match access.item_event_id {
                Some(ev) => removed_ids.contains(&ev),
                None => !removed_ids.is_empty(),
            };
            if stripped {
                return Err(AttachmentError::DoorDenied {
                    handle_id: handle.handle_id.clone(),
                    verdict: "strip",
                    rule: rule.clone(),
                    reason: "item stripped by context door".to_owned(),
                });
            }
        }
        DoorDecision::Hold { rule, reason } => {
            return Err(AttachmentError::DoorDenied {
                handle_id: handle.handle_id.clone(),
                verdict: "hold",
                rule: rule.clone(),
                reason: reason.clone(),
            });
        }
        DoorDecision::Refuse { rule, reason } => {
            return Err(AttachmentError::DoorDenied {
                handle_id: handle.handle_id.clone(),
                verdict: "refuse",
                rule: rule.clone(),
                reason: reason.clone(),
            });
        }
    }

    let bytes = store.fetch_bytes(handle)?;
    let actual = bytes.len() as u64;
    if actual != handle.byte_len {
        return Err(AttachmentError::ByteLengthMismatch {
            handle_id: handle.handle_id.clone(),
            expected: handle.byte_len,
            actual,
        });
    }

    let mime_type = mime_override
        .unwrap_or_else(|| default_mime_for_modality(handle.modality_slot))
        .to_owned();

    Ok(ResolvedAttachment {
        handle_id: handle.handle_id.clone(),
        modality_slot: handle.modality_slot,
        mime_type,
        bytes,
    })
}
