//! Canonical Waist-2 constants, capability descriptors, JID/place/actor normalization,
//! and typed request/payload definitions for `body-whatsapp`.

use std::collections::BTreeSet;

use monoagens_body_kit::capability_schema_hash;
use monoagens_body_sdk::{
    ActorId, BodyId, CapabilityDescriptor, CapabilityFlags, CapabilityId, PlaceId, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use whatsapp_rust::wacore_binary::JidExt as _;
use whatsapp_rust::{ChatStateType, Jid, Server};

/// Default Waist-2 body identifier for `body-whatsapp`.
pub const DEFAULT_BODY_ID: &str = "body-whatsapp";
/// Default system place declared by `body-whatsapp` in `body/discover`.
pub const DEFAULT_SYSTEM_PLACE: &str = "wa:system";
/// Default WhatsApp text chunk cap `L_cap` for split-bubble delivery.
pub const DEFAULT_BUBBLE_CAP: usize = 4_096;
/// Payloads larger than 256 KiB are staged out-of-band via `blob/put`.
pub const INLINE_ATTACHMENT_CAP_BYTES: usize = 256 * 1024;
/// Clock-skew grace window before `boot_timestamp_ms` when filtering historical messages.
pub const PRE_BOOT_GRACE_MS: u64 = 15_000;

pub const CAP_APPROVAL_PROMPT: &str = "approval.prompt";
pub const CAP_CHAT_DELETE: &str = "chat.delete";
pub const CAP_CHAT_EDIT: &str = "chat.edit";
pub const CAP_CHAT_MARK_READ: &str = "chat.mark_read";
pub const CAP_CHAT_REACT: &str = "chat.react";
pub const CAP_CHAT_SEND_MEDIA: &str = "chat.send_media";
pub const CAP_CHAT_SEND_POLL: &str = "chat.send_poll";
pub const CAP_CHAT_SEND_STICKER: &str = "chat.send_sticker";
pub const CAP_CHAT_SPEAK: &str = "chat.speak";
pub const CAP_CHAT_TYPING: &str = "chat.typing";

/// Returns default `BodyId` (`body-whatsapp`).
#[must_use]
pub fn default_body_id() -> BodyId {
    BodyId::new(DEFAULT_BODY_ID)
}

/// Returns default system `PlaceId` (`wa:system`).
#[must_use]
pub fn default_system_place() -> PlaceId {
    PlaceId::new(DEFAULT_SYSTEM_PLACE)
}

/// Normalizes a JID into canonical bare form (`user@server`, stripping `:device`).
#[must_use]
pub fn bare_jid_string(jid: &Jid) -> String {
    jid.to_non_ad_string()
}

/// Converts a WhatsApp chat [`Jid`] into a normalized Waist-2 [`PlaceId`]:
/// - Group (`@g.us`): `wa:group:<bare_jid>`
/// - DM: `wa:dm:<bare_jid>`
#[must_use]
pub fn jid_to_place(jid: &Jid) -> PlaceId {
    let bare = bare_jid_string(jid);
    if jid.is_group() {
        PlaceId::new(format!("wa:group:{bare}"))
    } else {
        PlaceId::new(format!("wa:dm:{bare}"))
    }
}

/// Converts a WhatsApp sender [`Jid`] into a normalized Waist-2 [`ActorId`] (`wa:user:<bare_jid>`).
#[must_use]
pub fn jid_to_actor(jid: &Jid) -> ActorId {
    ActorId::new(format!("wa:user:{}", bare_jid_string(jid)))
}

/// Parses a `wa:user:<jid>` or raw `<jid>` string into a bare [`Jid`].
pub fn parse_user_jid(raw: &str) -> Result<Jid, String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix("wa:user:").unwrap_or(trimmed);
    if inner.is_empty() {
        return Err("empty WhatsApp user JID".to_owned());
    }
    let jid = inner
        .parse::<Jid>()
        .map_err(|e| format!("invalid WhatsApp user JID '{raw}': {e}"))?;
    Ok(jid.to_non_ad())
}

/// Parses a Waist-2 [`PlaceId`] (`wa:dm:<jid>` or `wa:group:<jid>`) into a bare [`Jid`].
pub fn place_to_jid(place: &PlaceId) -> Result<Jid, String> {
    let raw = place.as_str();
    if let Some(rest) = raw.strip_prefix("wa:dm:") {
        let jid = parse_jid_with_server(rest, false)?;
        if jid.is_group() {
            return Err(format!("DM place '{raw}' cannot contain a group JID"));
        }
        return Ok(jid.to_non_ad());
    }
    if let Some(rest) = raw.strip_prefix("wa:group:") {
        let jid = parse_jid_with_server(rest, true)?;
        if !jid.is_group() {
            return Err(format!("group place '{raw}' must use a @g.us group JID"));
        }
        return Ok(jid.to_non_ad());
    }
    Err(format!(
        "unsupported WhatsApp place '{raw}' (expected wa:dm:<jid> or wa:group:<jid>)"
    ))
}

fn parse_jid_with_server(raw: &str, is_group: bool) -> Result<Jid, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty JID inside WhatsApp place".to_owned());
    }
    if trimmed.contains('@') {
        let jid = trimmed
            .parse::<Jid>()
            .map_err(|e| format!("invalid JID '{trimmed}': {e}"))?;
        if jid.user().is_empty() {
            return Err(format!("JID '{trimmed}' missing user part"));
        }
        Ok(jid)
    } else if trimmed.chars().all(|c| c.is_ascii_digit() || c == '-') {
        let server = if is_group { Server::Group } else { Server::Pn };
        Ok(Jid::new(trimmed, server))
    } else {
        Err(format!("invalid JID '{trimmed}'"))
    }
}

/// Returns `true` if `place` is owned by `body-whatsapp` (`wa:system`, `wa:dm:...`, or `wa:group:...`).
#[must_use]
pub fn is_owned_whatsapp_place(place: &PlaceId) -> bool {
    place.as_str() == DEFAULT_SYSTEM_PLACE || place_to_jid(place).is_ok()
}

/// Deduplicates and sorts `places`, always ensuring `wa:system` is included.
#[must_use]
pub fn normalize_declared_places(extra_places: impl IntoIterator<Item = PlaceId>) -> Vec<PlaceId> {
    let mut set = BTreeSet::new();
    set.insert(default_system_place());
    for p in extra_places {
        if is_owned_whatsapp_place(&p) {
            set.insert(p);
        }
    }
    set.into_iter().collect()
}

/// Builds the 10 deterministically sorted native WhatsApp Waist-2 [`CapabilityDescriptor`]s.
#[must_use]
pub fn build_whatsapp_capabilities(places: &[PlaceId]) -> Vec<CapabilityDescriptor> {
    let place_names: Vec<&str> = places.iter().map(PlaceId::as_str).collect();
    let communicative = CapabilityFlags {
        communicative: true,
        identity_changing: false,
        irreversible: false,
    };
    let irreversible_comm = CapabilityFlags {
        communicative: true,
        identity_changing: false,
        irreversible: true,
    };
    let non_comm = CapabilityFlags {
        communicative: false,
        identity_changing: false,
        irreversible: false,
    };

    let mut caps = vec![
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_APPROVAL_PROMPT),
            description:
                "Render a generic pending-approval prompt in WhatsApp with emoji reaction instructions"
                    .to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 150,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_APPROVAL_PROMPT,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": [
                        "action_id",
                        "h_act",
                        "state_epoch",
                        "capability_id",
                        "summary",
                        "expires_at_ms"
                    ],
                    "properties": {
                        "action_id": { "type": "string" },
                        "h_act": { "type": "string" },
                        "state_epoch": { "type": "integer", "minimum": 0 },
                        "capability_id": { "type": "string" },
                        "target_places": { "type": "array", "items": { "type": "string" } },
                        "summary": { "type": "string" },
                        "expires_at_ms": { "type": "integer", "minimum": 0 },
                        "approve_emojis": { "type": "array", "items": { "type": "string" } },
                        "deny_emojis": { "type": "array", "items": { "type": "string" } }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_SPEAK),
            description:
                "Send WhatsApp text message (or split-bubble chunks <= 4096 bytes) with optional quote-reply, @mentions, and automatic composing/paused chatstate"
                    .to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 150,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_SPEAK,
                "places": &place_names,
                "input": {
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "required": ["text"],
                            "properties": {
                                "text": { "type": "string" },
                                "reply_to_message_id": { "type": "string" },
                                "reply_to_participant": { "type": "string" },
                                "mentions": { "type": "array", "items": { "type": "string" } }
                            }
                        }
                    ]
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_REACT),
            description:
                "React to a WhatsApp message with an emoji (pass empty emoji \"\" to remove an existing reaction)"
                    .to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 120,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_REACT,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": ["message_id", "emoji"],
                    "properties": {
                        "message_id": { "type": "string" },
                        "emoji": { "type": "string" },
                        "from_me": { "type": "boolean" },
                        "participant": { "type": "string" }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_TYPING),
            description: "Send WhatsApp chatstate indicator (composing, recording, paused)".to_owned(),
            places: places.to_vec(),
            flags: non_comm,
            typical_latency_ms: 80,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_TYPING,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": ["state"],
                    "properties": {
                        "state": { "type": "string", "enum": ["composing", "recording", "paused", "typing"] }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_SEND_MEDIA),
            description:
                "Upload and send single or multiple WhatsApp media attachments (image, video, document, audio, voice/PTT)"
                    .to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 450,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_SEND_MEDIA,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "properties": {
                        "attachments": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["kind"],
                                "properties": {
                                    "kind": { "type": "string", "enum": ["image", "video", "document", "audio", "voice"] },
                                    "path": { "type": "string" },
                                    "blob_ref": { "type": "string" },
                                    "bytes_base64": { "type": "string" },
                                    "mime_type": { "type": "string" },
                                    "filename": { "type": "string" },
                                    "caption": { "type": "string" },
                                    "ptt": { "type": "boolean" },
                                    "duration_seconds": { "type": "integer" },
                                    "gif_playback": { "type": "boolean" }
                                }
                            }
                        }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_SEND_STICKER),
            description: "Upload and send a native WebP WhatsApp sticker".to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 300,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_SEND_STICKER,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "blob_ref": { "type": "string" },
                        "bytes_base64": { "type": "string" },
                        "mime_type": { "type": "string" },
                        "is_animated": { "type": "boolean" }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_SEND_POLL),
            description: "Create and send a native WhatsApp PollCreationMessage".to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 150,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_SEND_POLL,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": ["question", "options"],
                    "properties": {
                        "question": { "type": "string" },
                        "options": { "type": "array", "items": { "type": "string" }, "minItems": 2 },
                        "selectable_count": { "type": "integer", "minimum": 1 }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_MARK_READ),
            description: "Send WhatsApp blue-tick read receipts for one or more message IDs".to_owned(),
            places: places.to_vec(),
            flags: non_comm,
            typical_latency_ms: 80,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_MARK_READ,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "properties": {
                        "message_ids": { "type": "array", "items": { "type": "string" } },
                        "message_id": { "type": "string" },
                        "sender": { "type": "string" }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_EDIT),
            description: "Edit a previously sent WhatsApp text message via ProtocolMessage".to_owned(),
            places: places.to_vec(),
            flags: communicative,
            typical_latency_ms: 150,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_EDIT,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": ["message_id", "text"],
                    "properties": {
                        "message_id": { "type": "string" },
                        "text": { "type": "string" }
                    }
                }
            })),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAP_CHAT_DELETE),
            description: "Revoke/delete a WhatsApp message for everyone via ProtocolMessage".to_owned(),
            places: places.to_vec(),
            flags: irreversible_comm,
            typical_latency_ms: 150,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": CAP_CHAT_DELETE,
                "places": &place_names,
                "input": {
                    "type": "object",
                    "required": ["message_id"],
                    "properties": {
                        "message_id": { "type": "string" },
                        "sender": { "type": "string" }
                    }
                }
            })),
            sensed_effect: None,
        },
    ];
    sort_capabilities(&mut caps);
    caps
}

/// Parsed request payload for `chat.speak`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakRequest {
    pub text: String,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub reply_to_participant: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

impl SpeakRequest {
    /// Parses `payload` either as structured JSON `{ text, reply_to_message_id?, ... }`
    /// or as raw UTF-8 text.
    pub fn parse_payload(bytes: &[u8]) -> Result<Self, String> {
        let raw = String::from_utf8_lossy(bytes);
        let trimmed = raw.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
            && val.get("text").is_some_and(serde_json::Value::is_string)
        {
            return serde_json::from_value::<Self>(val)
                .map_err(|e| format!("invalid chat.speak JSON payload: {e}"));
        }
        Ok(Self {
            text: raw.into_owned(),
            reply_to_message_id: None,
            reply_to_participant: None,
            mentions: Vec::new(),
        })
    }
}

/// Parsed request payload for `chat.react`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactRequest {
    pub message_id: String,
    pub emoji: String,
    #[serde(default)]
    pub from_me: Option<bool>,
    #[serde(default)]
    pub participant: Option<String>,
}

/// Parsed request payload for `chat.typing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypingRequest {
    pub state: String,
}

impl TypingRequest {
    /// Maps string state (`"composing"`, `"typing"`, `"recording"`, `"paused"`) to [`ChatStateType`].
    pub fn chat_state_type(&self) -> Result<ChatStateType, String> {
        match self.state.trim().to_ascii_lowercase().as_str() {
            "composing" | "typing" => Ok(ChatStateType::Composing),
            "recording" | "record_audio" | "voice" => Ok(ChatStateType::Recording),
            "paused" | "stop" => Ok(ChatStateType::Paused),
            other => Err(format!(
                "unsupported chat.typing state '{other}' (expected composing, recording, or paused)"
            )),
        }
    }
}

/// Supported attachment kinds for `chat.send_media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Document,
    Audio,
    Voice,
}

/// Single media attachment descriptor in `chat.send_media`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAttachmentInput {
    pub kind: MediaKind,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub blob_ref: Option<String>,
    #[serde(default)]
    pub bytes_base64: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub ptt: Option<bool>,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub gif_playback: Option<bool>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

/// Request payload for `chat.send_media`, supporting either `{ attachments: [...] }`
/// or a single flat attachment object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMediaRequest {
    pub attachments: Vec<MediaAttachmentInput>,
}

impl SendMediaRequest {
    /// Parses `payload` bytes into a normalized non-empty attachment list.
    pub fn parse_payload(bytes: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid chat.send_media JSON payload: {e}"))?;
        if val.get("attachments").is_some() {
            #[derive(Deserialize)]
            struct Wrapper {
                attachments: Vec<MediaAttachmentInput>,
            }
            let w: Wrapper = serde_json::from_value(val)
                .map_err(|e| format!("invalid chat.send_media attachments array: {e}"))?;
            if w.attachments.is_empty() {
                return Err("chat.send_media attachments array must not be empty".to_owned());
            }
            Ok(Self {
                attachments: w.attachments,
            })
        } else {
            let single: MediaAttachmentInput = serde_json::from_value(val)
                .map_err(|e| format!("invalid chat.send_media attachment payload: {e}"))?;
            Ok(Self {
                attachments: vec![single],
            })
        }
    }
}

/// Request payload for `chat.send_sticker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendStickerRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub blob_ref: Option<String>,
    #[serde(default)]
    pub bytes_base64: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub is_animated: bool,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
}

/// Request payload for `chat.send_poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPollRequest {
    #[serde(alias = "name")]
    pub question: String,
    pub options: Vec<String>,
    #[serde(default = "default_selectable_count", alias = "selectable_options_count")]
    pub selectable_count: u32,
}

const fn default_selectable_count() -> u32 {
    1
}

/// Request payload for `chat.mark_read`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkReadRequest {
    pub message_ids: Vec<String>,
    pub sender: Option<String>,
}

impl MarkReadRequest {
    /// Parses `{ message_ids: [...] }` or `{ message_id: "..." }`.
    pub fn parse_payload(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            message_ids: Vec<String>,
            #[serde(default)]
            message_id: Option<String>,
            #[serde(default)]
            sender: Option<String>,
        }
        let raw: Raw = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid chat.mark_read JSON payload: {e}"))?;
        let mut ids = raw.message_ids;
        if let Some(single) = raw.message_id
            && !single.is_empty()
        {
            ids.push(single);
        }
        if ids.is_empty() {
            return Err("chat.mark_read requires at least one message_id".to_owned());
        }
        Ok(Self {
            message_ids: ids,
            sender: raw.sender,
        })
    }
}

/// Request payload for `chat.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRequest {
    pub message_id: String,
    pub text: String,
}

/// Request payload for `chat.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub message_id: String,
    #[serde(default)]
    pub sender: Option<String>,
}

/// Request payload for `approval.prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPromptRequest {
    pub action_id: String,
    pub h_act: String,
    pub state_epoch: u64,
    pub capability_id: String,
    #[serde(default)]
    pub target_places: Vec<String>,
    pub summary: String,
    pub expires_at_ms: u64,
    #[serde(default)]
    pub approve_emojis: Option<Vec<String>>,
    #[serde(default)]
    pub deny_emojis: Option<Vec<String>>,
}
