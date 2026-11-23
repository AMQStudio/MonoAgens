//! Canonical `body-whatsapp` Waist-2 body (`monoagens.body.v1`) powered by `whatsapp-rust 0.7`.

#![forbid(unsafe_code)]

pub mod body;
pub mod inbound;
pub mod transport;
pub mod types;

pub use body::{BlobLookup, MemoryBlobLookup, WhatsAppBody};
pub use inbound::{
    BlobStager, CachedPoll, GroupAudienceCache, InboundTranslator, MemoryBlobStager, PollSecretCache,
};
pub use transport::{
    LiveWhatsAppTransport, OfflineWhatsAppTransport, PreparedMedia, PreparedSticker,
    RecordedOutboundAction, WhatsAppTransport,
};
pub use types::{
    CAP_CHAT_DELETE, CAP_CHAT_EDIT, CAP_CHAT_MARK_READ, CAP_CHAT_REACT, CAP_CHAT_SEND_MEDIA,
    CAP_CHAT_SEND_POLL, CAP_CHAT_SEND_STICKER, CAP_CHAT_SPEAK, CAP_CHAT_TYPING, DEFAULT_BODY_ID,
    DEFAULT_BUBBLE_CAP, DEFAULT_SYSTEM_PLACE, DeleteRequest, EditRequest,
    INLINE_ATTACHMENT_CAP_BYTES, MarkReadRequest, MediaAttachmentInput, MediaKind,
    PRE_BOOT_GRACE_MS, ReactRequest, SendMediaRequest, SendPollRequest, SendStickerRequest,
    SpeakRequest, TypingRequest, bare_jid_string, build_whatsapp_capabilities, default_body_id,
    default_system_place, is_owned_whatsapp_place, jid_to_actor, jid_to_place,
    normalize_declared_places, parse_user_jid, place_to_jid,
};
