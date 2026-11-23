//! Telegram Bot API wire types, Waist-2 payload schemas, and place/actor parsers.

use core::fmt;

use monoagens_body_sdk::{ActorId, PlaceId};
use serde::{Deserialize, Serialize};

/// Canonical Telegram text bubble cap (`L_cap = 4096` UTF-8 chars/bytes).
pub const TELEGRAM_TEXT_CAP: usize = 4_096;

/// Default virtual place advertised in `body/discover` for conformance/bootstrap.
pub const DEFAULT_TELEGRAM_SYSTEM_PLACE: &str = "tg:system";

/// Parsed Telegram place target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramTarget {
    /// Virtual system discovery/probe place (`tg:system`).
    System,
    /// Direct message chat (`tg:dm:<chat_id>`).
    Dm { chat_id: i64 },
    /// Group, supergroup, or forum topic (`tg:group:<chat_id>` or `tg:group:<chat_id>:topic:<thread_id>`).
    Group {
        chat_id: i64,
        thread_id: Option<i64>,
    },
}

impl TelegramTarget {
    /// Parses a Waist-2 [`PlaceId`] into a [`TelegramTarget`].
    #[must_use]
    pub fn parse(place: &PlaceId) -> Option<Self> {
        let raw = place.as_str();
        if raw == DEFAULT_TELEGRAM_SYSTEM_PLACE {
            return Some(Self::System);
        }
        if let Some(rest) = raw.strip_prefix("tg:dm:") {
            let chat_id = rest.parse::<i64>().ok()?;
            return Some(Self::Dm { chat_id });
        }
        if let Some(rest) = raw.strip_prefix("tg:group:") {
            if let Some((chat_part, topic_part)) = rest.split_once(":topic:") {
                let chat_id = chat_part.parse::<i64>().ok()?;
                let thread_id = topic_part.parse::<i64>().ok()?;
                return Some(Self::Group {
                    chat_id,
                    thread_id: Some(thread_id),
                });
            }
            let chat_id = rest.parse::<i64>().ok()?;
            return Some(Self::Group {
                chat_id,
                thread_id: None,
            });
        }
        None
    }

    /// Returns the Telegram `chat_id` if this is a concrete chat target.
    #[must_use]
    pub const fn chat_id(&self) -> Option<i64> {
        match self {
            Self::System => None,
            Self::Dm { chat_id } | Self::Group { chat_id, .. } => Some(*chat_id),
        }
    }

    /// Returns the optional Telegram forum `message_thread_id`.
    #[must_use]
    pub const fn thread_id(&self) -> Option<i64> {
        match self {
            Self::System | Self::Dm { .. } => None,
            Self::Group { thread_id, .. } => *thread_id,
        }
    }
}

/// Formats a Waist-2 [`PlaceId`] from a Telegram chat and optional forum topic thread ID.
#[must_use]
pub fn format_telegram_place(chat: &TgChat, message_thread_id: Option<i64>) -> PlaceId {
    if chat.chat_type == "private" {
        PlaceId::new(format!("tg:dm:{}", chat.id))
    } else if let Some(tid) = message_thread_id {
        PlaceId::new(format!("tg:group:{}:topic:{tid}", chat.id))
    } else {
        PlaceId::new(format!("tg:group:{}", chat.id))
    }
}

/// Formats a Waist-2 user [`ActorId`] from a Telegram user ID.
#[must_use]
pub fn format_user_actor(user_id: i64) -> ActorId {
    ActorId::new(format!("tg:user:{user_id}"))
}

/// Formats a Waist-2 bot [`ActorId`] from an optional bot user ID.
#[must_use]
pub fn format_bot_actor(bot_user_id: Option<i64>) -> ActorId {
    match bot_user_id {
        Some(id) => ActorId::new(format!("tg:bot:{id}")),
        None => ActorId::new("tg:bot:self"),
    }
}

/// Telegram `sendChatAction` values supported by `chat.typing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TgChatAction {
    Typing,
    UploadPhoto,
    UploadDocument,
    RecordVoice,
    UploadVideo,
    ChooseSticker,
}

impl TgChatAction {
    /// Returns the Telegram Bot API `action` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typing => "typing",
            Self::UploadPhoto => "upload_photo",
            Self::UploadDocument => "upload_document",
            Self::RecordVoice => "record_voice",
            Self::UploadVideo => "upload_video",
            Self::ChooseSticker => "choose_sticker",
        }
    }
}

impl fmt::Display for TgChatAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Inline keyboard button descriptor for `chat.speak`, `chat.send_buttons`, and `chat.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineButton {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Payload for `chat.speak` (JSON object or plain UTF-8 text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakPayload {
    pub text: String,
    #[serde(default)]
    pub parse_mode: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
    #[serde(default)]
    pub buttons: Option<Vec<Vec<InlineButton>>>,
}

/// Payload for `chat.send_buttons`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendButtonsPayload {
    pub text: String,
    pub buttons: Vec<Vec<InlineButton>>,
    #[serde(default)]
    pub parse_mode: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
}

/// Payload for `chat.react`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactPayload {
    pub message_id: i64,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub is_big: bool,
}

/// Payload for `chat.typing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypingPayload {
    #[serde(default = "default_typing_action")]
    pub action: TgChatAction,
}

const fn default_typing_action() -> TgChatAction {
    TgChatAction::Typing
}

/// Supported Telegram media attachment kinds for `chat.send_media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Document,
    Video,
    Audio,
    Voice,
}

impl MediaKind {
    /// Returns the Telegram `sendChatAction` hint corresponding to this media kind.
    #[must_use]
    pub const fn chat_action(self) -> TgChatAction {
        match self {
            Self::Photo => TgChatAction::UploadPhoto,
            Self::Document => TgChatAction::UploadDocument,
            Self::Video => TgChatAction::UploadVideo,
            Self::Audio | Self::Voice => TgChatAction::RecordVoice,
        }
    }

    /// Returns the Telegram `InputMedia` type tag (`photo`, `document`, `video`, `audio`).
    #[must_use]
    pub const fn input_media_type(self) -> Option<&'static str> {
        match self {
            Self::Photo => Some("photo"),
            Self::Document => Some("document"),
            Self::Video => Some("video"),
            Self::Audio => Some("audio"),
            Self::Voice => None,
        }
    }
}

/// One media item input for `chat.send_media`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAttachmentInput {
    pub kind: MediaKind,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub blob_ref: Option<String>,
    #[serde(default)]
    pub bytes_base64: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

/// Payload for `chat.send_media`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMediaPayload {
    #[serde(default)]
    pub attachments: Vec<MediaAttachmentInput>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub parse_mode: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
}

/// Payload for `chat.send_sticker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendStickerPayload {
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub blob_ref: Option<String>,
    #[serde(default)]
    pub bytes_base64: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
}

/// Poll variant (`regular` or `quiz`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollKind {
    #[default]
    Regular,
    Quiz,
}

impl PollKind {
    /// Returns the Telegram Bot API `type` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Quiz => "quiz",
        }
    }
}

/// Payload for `chat.send_poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPollPayload {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub is_anonymous: Option<bool>,
    #[serde(default)]
    pub kind: PollKind,
    #[serde(default)]
    pub allows_multiple_answers: Option<bool>,
    #[serde(default)]
    pub correct_option_id: Option<i64>,
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
}

/// Payload for `chat.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditPayload {
    pub message_id: i64,
    pub text: String,
    #[serde(default)]
    pub parse_mode: Option<String>,
    #[serde(default)]
    pub buttons: Option<Vec<Vec<InlineButton>>>,
}

/// Payload for `chat.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletePayload {
    pub message_id: i64,
}

/// Payload for `approval.prompt` rendering a generic held action with native inline buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPromptPayload {
    pub action_id: String,
    pub h_act: String,
    pub state_epoch: u64,
    pub capability_id: String,
    #[serde(default)]
    pub target_places: Vec<String>,
    pub summary: String,
    pub expires_at_ms: u64,
    #[serde(default)]
    pub approve_label: Option<String>,
    #[serde(default)]
    pub deny_label: Option<String>,
}

/// Resolved media source ready for transport dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSource {
    FileId(String),
    Url(String),
    Bytes {
        data: Vec<u8>,
        filename: String,
        mime_type: Option<String>,
    },
}

/// Prepared attachment item passed to [`crate::transport::TelegramTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMediaItem {
    pub kind: MediaKind,
    pub source: MediaSource,
    pub caption: Option<String>,
    pub parse_mode: Option<String>,
}

// ---------------------------------------------------------------------------
// Telegram Bot API wire types
// ---------------------------------------------------------------------------

fn default_none<T>() -> Option<T> {
    None
}

/// Generic Telegram Bot API response envelope.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TgApiResponse<T> {
    pub ok: bool,
    #[serde(default = "default_none")]
    pub result: Option<T>,
    #[serde(default)]
    pub error_code: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Telegram `User` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgUser {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// Telegram `Chat` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub is_forum: Option<bool>,
}

/// Telegram `PhotoSize` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgPhotoSize {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Document` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgDocument {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Video` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgVideo {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Audio` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgAudio {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub performer: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Voice` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgVoice {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Sticker` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgSticker {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub is_animated: bool,
    #[serde(default)]
    pub is_video: bool,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub set_name: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Telegram `Message` object (subset needed for Waist-2 translation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TgMessage {
    pub message_id: i64,
    #[serde(default)]
    pub message_thread_id: Option<i64>,
    #[serde(default)]
    pub from: Option<TgUser>,
    #[serde(default)]
    pub sender_chat: Option<TgChat>,
    pub date: i64,
    pub chat: TgChat,
    #[serde(default)]
    pub reply_to_message: Option<Box<TgMessage>>,
    #[serde(default)]
    pub edit_date: Option<i64>,
    #[serde(default)]
    pub media_group_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub photo: Option<Vec<TgPhotoSize>>,
    #[serde(default)]
    pub document: Option<TgDocument>,
    #[serde(default)]
    pub video: Option<TgVideo>,
    #[serde(default)]
    pub audio: Option<TgAudio>,
    #[serde(default)]
    pub voice: Option<TgVoice>,
    #[serde(default)]
    pub sticker: Option<TgSticker>,
    #[serde(default)]
    pub new_chat_members: Option<Vec<TgUser>>,
    #[serde(default)]
    pub left_chat_member: Option<TgUser>,
}

/// Telegram `CallbackQuery` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TgCallbackQuery {
    pub id: String,
    pub from: TgUser,
    #[serde(default)]
    pub message: Option<TgMessage>,
    #[serde(default)]
    pub inline_message_id: Option<String>,
    #[serde(default)]
    pub chat_instance: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
}

/// Telegram `ReactionType` union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TgReactionType {
    Emoji {
        emoji: String,
    },
    CustomEmoji {
        custom_emoji_id: String,
    },
    Paid,
    #[serde(other)]
    Unknown,
}

impl TgReactionType {
    /// Returns a canonical string key for the reaction (`emoji` or `custom:<id>`).
    #[must_use]
    pub fn as_label(&self) -> String {
        match self {
            Self::Emoji { emoji } => emoji.clone(),
            Self::CustomEmoji { custom_emoji_id } => format!("custom:{custom_emoji_id}"),
            Self::Paid => "paid".to_owned(),
            Self::Unknown => "unknown".to_owned(),
        }
    }
}

/// Telegram `MessageReactionUpdated` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgMessageReactionUpdated {
    pub chat: TgChat,
    pub message_id: i64,
    #[serde(default)]
    pub user: Option<TgUser>,
    #[serde(default)]
    pub actor_chat: Option<TgChat>,
    pub date: i64,
    #[serde(default)]
    pub old_reaction: Vec<TgReactionType>,
    #[serde(default)]
    pub new_reaction: Vec<TgReactionType>,
}

/// Telegram `Update` object from `getUpdates`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TgUpdate {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<TgMessage>,
    #[serde(default)]
    pub edited_message: Option<TgMessage>,
    #[serde(default)]
    pub callback_query: Option<TgCallbackQuery>,
    #[serde(default)]
    pub message_reaction: Option<TgMessageReactionUpdated>,
}

/// Telegram `File` object returned by `getFile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgFile {
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub file_path: Option<String>,
}

/// Telegram `ChatMember` object returned by `getChatAdministrators`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TgChatMember {
    pub status: String,
    pub user: TgUser,
}
