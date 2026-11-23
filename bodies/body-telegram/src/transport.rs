//! Telegram Bot API transport abstraction plus live, offline, and recording implementations.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::blocking::{Client, multipart};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::types::{
    InlineButton, MediaSource, OutboundMediaItem, PollKind, SendPollPayload, TgApiResponse,
    TgChatAction, TgChatMember, TgFile, TgMessage, TgUpdate, TgUser,
};

/// Default Telegram Bot API base URL.
pub const DEFAULT_TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// Transport interface covering all Telegram Bot API endpoints used by `body-telegram`.
pub trait TelegramTransport: Send {
    /// Returns `true` for offline/synthetic conformance mode (no network calls).
    fn is_offline(&self) -> bool {
        false
    }

    fn get_me(&mut self) -> Result<TgUser, String>;

    fn get_updates(&mut self, offset: Option<i64>, timeout_secs: u64) -> Result<Vec<TgUpdate>, String>;

    fn send_chat_action(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        action: TgChatAction,
    ) -> Result<(), String>;

    fn send_message(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
        parse_mode: Option<&str>,
        reply_to_message_id: Option<i64>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<i64, String>;

    fn send_single_media(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        item: &OutboundMediaItem,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String>;

    fn send_media_group(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        items: &[OutboundMediaItem],
        reply_to_message_id: Option<i64>,
    ) -> Result<Vec<i64>, String>;

    fn send_sticker(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        source: &MediaSource,
        emoji: Option<&str>,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String>;

    fn send_poll(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        poll: &SendPollPayload,
    ) -> Result<i64, String>;

    fn set_message_reaction(
        &mut self,
        chat_id: i64,
        message_id: i64,
        emoji: Option<&str>,
        is_big: bool,
    ) -> Result<(), String>;

    fn edit_message_text(
        &mut self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<(), String>;

    fn delete_message(&mut self, chat_id: i64, message_id: i64) -> Result<(), String>;

    fn answer_callback_query(
        &mut self,
        callback_query_id: &str,
        text: Option<&str>,
        show_alert: bool,
    ) -> Result<(), String>;

    fn download_file(&mut self, file_id: &str) -> Result<Vec<u8>, String>;

    fn get_chat_administrators(&mut self, chat_id: i64) -> Result<Vec<TgChatMember>, String>;
}

/// Offline no-op transport returning deterministic synthetic Telegram IDs without network I/O.
#[derive(Debug, Default)]
pub struct OfflineTelegramTransport {
    next_msg_id: i64,
    bot_user: Option<TgUser>,
}

impl OfflineTelegramTransport {
    /// Creates an [`OfflineTelegramTransport`] starting at synthetic `message_id = 100`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_msg_id: 100,
            bot_user: Some(TgUser {
                id: 999_000,
                is_bot: true,
                first_name: "OfflineBot".to_owned(),
                last_name: None,
                username: Some("offline_monoagens_bot".to_owned()),
            }),
        }
    }

    fn alloc_id(&mut self) -> i64 {
        self.next_msg_id = self.next_msg_id.saturating_add(1);
        self.next_msg_id
    }
}

impl TelegramTransport for OfflineTelegramTransport {
    fn is_offline(&self) -> bool {
        true
    }

    fn get_me(&mut self) -> Result<TgUser, String> {
        self.bot_user
            .clone()
            .ok_or_else(|| "offline bot user unavailable".to_owned())
    }

    fn get_updates(
        &mut self,
        _offset: Option<i64>,
        _timeout_secs: u64,
    ) -> Result<Vec<TgUpdate>, String> {
        Ok(Vec::new())
    }

    fn send_chat_action(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _action: TgChatAction,
    ) -> Result<(), String> {
        Ok(())
    }

    fn send_message(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _text: &str,
        _parse_mode: Option<&str>,
        _reply_to_message_id: Option<i64>,
        _buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<i64, String> {
        Ok(self.alloc_id())
    }

    fn send_single_media(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _item: &OutboundMediaItem,
        _reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        Ok(self.alloc_id())
    }

    fn send_media_group(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        items: &[OutboundMediaItem],
        _reply_to_message_id: Option<i64>,
    ) -> Result<Vec<i64>, String> {
        Ok((0..items.len()).map(|_| self.alloc_id()).collect())
    }

    fn send_sticker(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _source: &MediaSource,
        _emoji: Option<&str>,
        _reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        Ok(self.alloc_id())
    }

    fn send_poll(
        &mut self,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _poll: &SendPollPayload,
    ) -> Result<i64, String> {
        Ok(self.alloc_id())
    }

    fn set_message_reaction(
        &mut self,
        _chat_id: i64,
        _message_id: i64,
        _emoji: Option<&str>,
        _is_big: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    fn edit_message_text(
        &mut self,
        _chat_id: i64,
        _message_id: i64,
        _text: &str,
        _parse_mode: Option<&str>,
        _buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn delete_message(&mut self, _chat_id: i64, _message_id: i64) -> Result<(), String> {
        Ok(())
    }

    fn answer_callback_query(
        &mut self,
        _callback_query_id: &str,
        _text: Option<&str>,
        _show_alert: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    fn download_file(&mut self, _file_id: &str) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn get_chat_administrators(&mut self, _chat_id: i64) -> Result<Vec<TgChatMember>, String> {
        Ok(Vec::new())
    }
}

/// Recorded outbound Telegram API call captured by [`RecordingTelegramTransport`].
#[derive(Debug, Clone, PartialEq)]
pub enum RecordedCall {
    SendChatAction {
        chat_id: i64,
        thread_id: Option<i64>,
        action: TgChatAction,
    },
    SendMessage {
        chat_id: i64,
        thread_id: Option<i64>,
        text: String,
        parse_mode: Option<String>,
        reply_to_message_id: Option<i64>,
        buttons: Option<Vec<Vec<InlineButton>>>,
    },
    SendSingleMedia {
        chat_id: i64,
        thread_id: Option<i64>,
        item: OutboundMediaItem,
        reply_to_message_id: Option<i64>,
    },
    SendMediaGroup {
        chat_id: i64,
        thread_id: Option<i64>,
        items: Vec<OutboundMediaItem>,
        reply_to_message_id: Option<i64>,
    },
    SendSticker {
        chat_id: i64,
        thread_id: Option<i64>,
        source: MediaSource,
        emoji: Option<String>,
        reply_to_message_id: Option<i64>,
    },
    SendPoll {
        chat_id: i64,
        thread_id: Option<i64>,
        poll: SendPollPayload,
    },
    SetMessageReaction {
        chat_id: i64,
        message_id: i64,
        emoji: Option<String>,
        is_big: bool,
    },
    EditMessageText {
        chat_id: i64,
        message_id: i64,
        text: String,
        parse_mode: Option<String>,
        buttons: Option<Vec<Vec<InlineButton>>>,
    },
    DeleteMessage {
        chat_id: i64,
        message_id: i64,
    },
    AnswerCallbackQuery {
        callback_query_id: String,
        text: Option<String>,
        show_alert: bool,
    },
}

/// In-memory recording transport for deterministic unit and end-to-end tests.
#[derive(Debug, Clone)]
pub struct RecordingTelegramTransport {
    pub calls: Arc<Mutex<Vec<RecordedCall>>>,
    pub files: BTreeMap<String, Vec<u8>>,
    pub admins: BTreeMap<i64, Vec<TgChatMember>>,
    pub bot_user: TgUser,
    next_msg_id: i64,
    pub fail_next_send_on_chunk: Option<usize>,
    send_counter: usize,
}

impl Default for RecordingTelegramTransport {
    fn default() -> Self {
        Self::new(777_000, "test_monoagens_bot")
    }
}

impl RecordingTelegramTransport {
    /// Creates a [`RecordingTelegramTransport`] with explicit bot ID and username.
    #[must_use]
    pub fn new(bot_id: i64, username: &str) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            files: BTreeMap::new(),
            admins: BTreeMap::new(),
            bot_user: TgUser {
                id: bot_id,
                is_bot: true,
                first_name: "TestBot".to_owned(),
                last_name: None,
                username: Some(username.to_owned()),
            },
            next_msg_id: 1_000,
            fail_next_send_on_chunk: None,
            send_counter: 0,
        }
    }

    /// Returns a snapshot of recorded Telegram API calls.
    #[must_use]
    pub fn snapshot_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn alloc_id(&mut self) -> i64 {
        self.next_msg_id = self.next_msg_id.saturating_add(1);
        self.next_msg_id
    }

    fn record(&self, call: RecordedCall) {
        self.calls.lock().expect("calls lock").push(call);
    }
}

impl TelegramTransport for RecordingTelegramTransport {
    fn get_me(&mut self) -> Result<TgUser, String> {
        Ok(self.bot_user.clone())
    }

    fn get_updates(
        &mut self,
        _offset: Option<i64>,
        _timeout_secs: u64,
    ) -> Result<Vec<TgUpdate>, String> {
        Ok(Vec::new())
    }

    fn send_chat_action(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        action: TgChatAction,
    ) -> Result<(), String> {
        self.record(RecordedCall::SendChatAction {
            chat_id,
            thread_id,
            action,
        });
        Ok(())
    }

    fn send_message(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
        parse_mode: Option<&str>,
        reply_to_message_id: Option<i64>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<i64, String> {
        let idx = self.send_counter;
        self.send_counter += 1;
        if self.fail_next_send_on_chunk == Some(idx) {
            return Err("simulated sendMessage failure".to_owned());
        }
        self.record(RecordedCall::SendMessage {
            chat_id,
            thread_id,
            text: text.to_owned(),
            parse_mode: parse_mode.map(ToOwned::to_owned),
            reply_to_message_id,
            buttons: buttons.map(<[Vec<InlineButton>]>::to_vec),
        });
        Ok(self.alloc_id())
    }

    fn send_single_media(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        item: &OutboundMediaItem,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        self.record(RecordedCall::SendSingleMedia {
            chat_id,
            thread_id,
            item: item.clone(),
            reply_to_message_id,
        });
        Ok(self.alloc_id())
    }

    fn send_media_group(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        items: &[OutboundMediaItem],
        reply_to_message_id: Option<i64>,
    ) -> Result<Vec<i64>, String> {
        self.record(RecordedCall::SendMediaGroup {
            chat_id,
            thread_id,
            items: items.to_vec(),
            reply_to_message_id,
        });
        Ok((0..items.len()).map(|_| self.alloc_id()).collect())
    }

    fn send_sticker(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        source: &MediaSource,
        emoji: Option<&str>,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        self.record(RecordedCall::SendSticker {
            chat_id,
            thread_id,
            source: source.clone(),
            emoji: emoji.map(ToOwned::to_owned),
            reply_to_message_id,
        });
        Ok(self.alloc_id())
    }

    fn send_poll(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        poll: &SendPollPayload,
    ) -> Result<i64, String> {
        self.record(RecordedCall::SendPoll {
            chat_id,
            thread_id,
            poll: poll.clone(),
        });
        Ok(self.alloc_id())
    }

    fn set_message_reaction(
        &mut self,
        chat_id: i64,
        message_id: i64,
        emoji: Option<&str>,
        is_big: bool,
    ) -> Result<(), String> {
        self.record(RecordedCall::SetMessageReaction {
            chat_id,
            message_id,
            emoji: emoji.map(ToOwned::to_owned),
            is_big,
        });
        Ok(())
    }

    fn edit_message_text(
        &mut self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<(), String> {
        self.record(RecordedCall::EditMessageText {
            chat_id,
            message_id,
            text: text.to_owned(),
            parse_mode: parse_mode.map(ToOwned::to_owned),
            buttons: buttons.map(<[Vec<InlineButton>]>::to_vec),
        });
        Ok(())
    }

    fn delete_message(&mut self, chat_id: i64, message_id: i64) -> Result<(), String> {
        self.record(RecordedCall::DeleteMessage {
            chat_id,
            message_id,
        });
        Ok(())
    }

    fn answer_callback_query(
        &mut self,
        callback_query_id: &str,
        text: Option<&str>,
        show_alert: bool,
    ) -> Result<(), String> {
        self.record(RecordedCall::AnswerCallbackQuery {
            callback_query_id: callback_query_id.to_owned(),
            text: text.map(ToOwned::to_owned),
            show_alert,
        });
        Ok(())
    }

    fn download_file(&mut self, file_id: &str) -> Result<Vec<u8>, String> {
        self.files
            .get(file_id)
            .cloned()
            .ok_or_else(|| format!("mock file '{file_id}' not found"))
    }

    fn get_chat_administrators(&mut self, chat_id: i64) -> Result<Vec<TgChatMember>, String> {
        Ok(self.admins.get(&chat_id).cloned().unwrap_or_default())
    }
}

/// Live Telegram Bot API HTTP transport powered by `reqwest::blocking::Client`.
#[derive(Clone)]
pub struct LiveTelegramTransport {
    client: Client,
    api_base: String,
    token: String,
}

impl LiveTelegramTransport {
    /// Creates a [`LiveTelegramTransport`] targeting `api_base` (`https://api.telegram.org` by default).
    pub fn new(api_base: impl Into<String>, token: impl Into<String>) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(40))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        let base = api_base.into().trim_end_matches('/').to_owned();
        Ok(Self {
            client,
            api_base: base,
            token: token.into(),
        })
    }

    fn method_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.token, method)
    }

    fn file_url(&self, file_path: &str) -> String {
        let clean = file_path.trim_start_matches('/');
        format!("{}/file/bot{}/{}", self.api_base, self.token, clean)
    }

    fn redact(&self, msg: impl ToString) -> String {
        let raw = msg.to_string();
        if self.token.is_empty() {
            raw
        } else {
            raw.replace(&self.token, "<REDACTED>")
        }
    }

    fn post_json<T: DeserializeOwned>(&self, method: &str, body: &Value) -> Result<T, String> {
        let resp = self
            .client
            .post(self.method_url(method))
            .json(body)
            .send()
            .map_err(|e| format!("telegram {method} request error: {}", self.redact(e)))?;
        self.parse_api_response(method, resp)
    }

    fn post_multipart<T: DeserializeOwned>(
        &self,
        method: &str,
        form: multipart::Form,
    ) -> Result<T, String> {
        let resp = self
            .client
            .post(self.method_url(method))
            .multipart(form)
            .send()
            .map_err(|e| format!("telegram {method} multipart error: {}", self.redact(e)))?;
        self.parse_api_response(method, resp)
    }

    fn parse_api_response<T: DeserializeOwned>(
        &self,
        method: &str,
        resp: reqwest::blocking::Response,
    ) -> Result<T, String> {
        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("telegram {method} read body error: {}", self.redact(e)))?;
        let parsed: TgApiResponse<T> = serde_json::from_str(&text).map_err(|e| {
            format!("telegram {method} invalid JSON (HTTP {status}): {e}")
        })?;
        if !parsed.ok {
            let code = parsed.error_code.unwrap_or(status.as_u16().into());
            let desc = parsed
                .description
                .unwrap_or_else(|| "unknown error".to_owned());
            return Err(format!("telegram {method} error ({code}): {desc}"));
        }
        parsed
            .result
            .ok_or_else(|| format!("telegram {method} ok=true but missing result"))
    }

    fn build_part(
        data: Vec<u8>,
        filename: String,
        mime_type: Option<&str>,
    ) -> Result<multipart::Part, String> {
        let mut part = multipart::Part::bytes(data).file_name(filename);
        if let Some(mime) = mime_type {
            part = part
                .mime_str(mime)
                .map_err(|e| format!("invalid mime_type '{mime}': {e}"))?;
        }
        Ok(part)
    }
}

fn inline_keyboard_json(buttons: &[Vec<InlineButton>]) -> Value {
    let rows: Vec<Value> = buttons
        .iter()
        .map(|row| {
            Value::Array(
                row.iter()
                    .map(|btn| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("text".to_owned(), json!(btn.text));
                        if let Some(ref cb) = btn.callback_data {
                            obj.insert("callback_data".to_owned(), json!(cb));
                        }
                        if let Some(ref url) = btn.url {
                            obj.insert("url".to_owned(), json!(url));
                        }
                        Value::Object(obj)
                    })
                    .collect(),
            )
        })
        .collect();
    json!({ "inline_keyboard": rows })
}

impl TelegramTransport for LiveTelegramTransport {
    fn get_me(&mut self) -> Result<TgUser, String> {
        self.post_json("getMe", &json!({}))
    }

    fn get_updates(
        &mut self,
        offset: Option<i64>,
        timeout_secs: u64,
    ) -> Result<Vec<TgUpdate>, String> {
        let mut body = json!({
            "timeout": timeout_secs,
            "allowed_updates": ["message", "edited_message", "callback_query", "message_reaction"]
        });
        if let Some(off) = offset {
            body["offset"] = json!(off);
        }
        self.post_json("getUpdates", &body)
    }

    fn send_chat_action(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        action: TgChatAction,
    ) -> Result<(), String> {
        let mut body = json!({
            "chat_id": chat_id,
            "action": action.as_str(),
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = json!(tid);
        }
        let _: Value = self.post_json("sendChatAction", &body)?;
        Ok(())
    }

    fn send_message(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
        parse_mode: Option<&str>,
        reply_to_message_id: Option<i64>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<i64, String> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = json!(tid);
        }
        if let Some(pm) = parse_mode {
            body["parse_mode"] = json!(pm);
        }
        if let Some(rid) = reply_to_message_id {
            body["reply_parameters"] = json!({ "message_id": rid });
        }
        if let Some(rows) = buttons {
            body["reply_markup"] = inline_keyboard_json(rows);
        }
        let msg: TgMessage = self.post_json("sendMessage", &body)?;
        Ok(msg.message_id)
    }

    fn send_single_media(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        item: &OutboundMediaItem,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        let (method, field) = match item.kind {
            crate::types::MediaKind::Photo => ("sendPhoto", "photo"),
            crate::types::MediaKind::Document => ("sendDocument", "document"),
            crate::types::MediaKind::Video => ("sendVideo", "video"),
            crate::types::MediaKind::Audio => ("sendAudio", "audio"),
            crate::types::MediaKind::Voice => ("sendVoice", "voice"),
        };

        let msg: TgMessage = match &item.source {
            MediaSource::FileId(s) | MediaSource::Url(s) => {
                let mut body = json!({
                    "chat_id": chat_id,
                });
                body[field] = json!(s);
                if let Some(tid) = thread_id {
                    body["message_thread_id"] = json!(tid);
                }
                if let Some(ref cap) = item.caption {
                    body["caption"] = json!(cap);
                }
                if let Some(ref pm) = item.parse_mode {
                    body["parse_mode"] = json!(pm);
                }
                if let Some(rid) = reply_to_message_id {
                    body["reply_parameters"] = json!({ "message_id": rid });
                }
                self.post_json(method, &body)?
            }
            MediaSource::Bytes {
                data,
                filename,
                mime_type,
            } => {
                let mut form = multipart::Form::new().text("chat_id", chat_id.to_string());
                if let Some(tid) = thread_id {
                    form = form.text("message_thread_id", tid.to_string());
                }
                if let Some(ref cap) = item.caption {
                    form = form.text("caption", cap.clone());
                }
                if let Some(ref pm) = item.parse_mode {
                    form = form.text("parse_mode", pm.clone());
                }
                if let Some(rid) = reply_to_message_id {
                    form = form.text(
                        "reply_parameters",
                        json!({ "message_id": rid }).to_string(),
                    );
                }
                let part = Self::build_part(
                    data.clone(),
                    filename.clone(),
                    mime_type.as_deref(),
                )?;
                form = form.part(field, part);
                self.post_multipart(method, form)?
            }
        };
        Ok(msg.message_id)
    }

    fn send_media_group(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        items: &[OutboundMediaItem],
        reply_to_message_id: Option<i64>,
    ) -> Result<Vec<i64>, String> {
        let mut media_entries = Vec::with_capacity(items.len());
        let mut upload_parts: Vec<(String, multipart::Part)> = Vec::new();

        for (idx, item) in items.iter().enumerate() {
            let media_type = item.kind.input_media_type().ok_or_else(|| {
                "voice notes cannot be included in Telegram sendMediaGroup".to_owned()
            })?;
            let media_ref = match &item.source {
                MediaSource::FileId(s) | MediaSource::Url(s) => s.clone(),
                MediaSource::Bytes {
                    data,
                    filename,
                    mime_type,
                } => {
                    let attach_key = format!("file_{idx}");
                    let part = Self::build_part(
                        data.clone(),
                        filename.clone(),
                        mime_type.as_deref(),
                    )?;
                    upload_parts.push((attach_key.clone(), part));
                    format!("attach://{attach_key}")
                }
            };
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_owned(), json!(media_type));
            obj.insert("media".to_owned(), json!(media_ref));
            if let Some(ref cap) = item.caption {
                obj.insert("caption".to_owned(), json!(cap));
            }
            if let Some(ref pm) = item.parse_mode {
                obj.insert("parse_mode".to_owned(), json!(pm));
            }
            media_entries.push(Value::Object(obj));
        }

        let msgs: Vec<TgMessage> = if upload_parts.is_empty() {
            let mut body = json!({
                "chat_id": chat_id,
                "media": media_entries,
            });
            if let Some(tid) = thread_id {
                body["message_thread_id"] = json!(tid);
            }
            if let Some(rid) = reply_to_message_id {
                body["reply_parameters"] = json!({ "message_id": rid });
            }
            self.post_json("sendMediaGroup", &body)?
        } else {
            let mut form = multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .text("media", Value::Array(media_entries).to_string());
            if let Some(tid) = thread_id {
                form = form.text("message_thread_id", tid.to_string());
            }
            if let Some(rid) = reply_to_message_id {
                form = form.text(
                    "reply_parameters",
                    json!({ "message_id": rid }).to_string(),
                );
            }
            for (key, part) in upload_parts {
                form = form.part(key, part);
            }
            self.post_multipart("sendMediaGroup", form)?
        };
        Ok(msgs.into_iter().map(|m| m.message_id).collect())
    }

    fn send_sticker(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        source: &MediaSource,
        emoji: Option<&str>,
        reply_to_message_id: Option<i64>,
    ) -> Result<i64, String> {
        let msg: TgMessage = match source {
            MediaSource::FileId(s) | MediaSource::Url(s) => {
                let mut body = json!({
                    "chat_id": chat_id,
                    "sticker": s,
                });
                if let Some(tid) = thread_id {
                    body["message_thread_id"] = json!(tid);
                }
                if let Some(em) = emoji {
                    body["emoji"] = json!(em);
                }
                if let Some(rid) = reply_to_message_id {
                    body["reply_parameters"] = json!({ "message_id": rid });
                }
                self.post_json("sendSticker", &body)?
            }
            MediaSource::Bytes {
                data,
                filename,
                mime_type,
            } => {
                let mut form = multipart::Form::new().text("chat_id", chat_id.to_string());
                if let Some(tid) = thread_id {
                    form = form.text("message_thread_id", tid.to_string());
                }
                if let Some(em) = emoji {
                    form = form.text("emoji", em.to_owned());
                }
                if let Some(rid) = reply_to_message_id {
                    form = form.text(
                        "reply_parameters",
                        json!({ "message_id": rid }).to_string(),
                    );
                }
                let part = Self::build_part(
                    data.clone(),
                    filename.clone(),
                    mime_type.as_deref(),
                )?;
                form = form.part("sticker", part);
                self.post_multipart("sendSticker", form)?
            }
        };
        Ok(msg.message_id)
    }

    fn send_poll(
        &mut self,
        chat_id: i64,
        thread_id: Option<i64>,
        poll: &SendPollPayload,
    ) -> Result<i64, String> {
        let options: Vec<Value> = poll
            .options
            .iter()
            .map(|opt| json!({ "text": opt }))
            .collect();
        let mut body = json!({
            "chat_id": chat_id,
            "question": poll.question,
            "options": options,
            "type": poll.kind.as_str(),
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = json!(tid);
        }
        if let Some(anon) = poll.is_anonymous {
            body["is_anonymous"] = json!(anon);
        }
        if let Some(multi) = poll.allows_multiple_answers {
            body["allows_multiple_answers"] = json!(multi);
        }
        if poll.kind == PollKind::Quiz
            && let Some(correct_id) = poll.correct_option_id
        {
            body["correct_option_id"] = json!(correct_id);
        }
        if let Some(rid) = poll.reply_to_message_id {
            body["reply_parameters"] = json!({ "message_id": rid });
        }
        let msg: TgMessage = self.post_json("sendPoll", &body)?;
        Ok(msg.message_id)
    }

    fn set_message_reaction(
        &mut self,
        chat_id: i64,
        message_id: i64,
        emoji: Option<&str>,
        is_big: bool,
    ) -> Result<(), String> {
        let reaction = match emoji.filter(|e| !e.trim().is_empty()) {
            Some(em) => json!([{ "type": "emoji", "emoji": em }]),
            None => json!([]),
        };
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": reaction,
            "is_big": is_big,
        });
        let _: Value = self.post_json("setMessageReaction", &body)?;
        Ok(())
    }

    fn edit_message_text(
        &mut self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        buttons: Option<&[Vec<InlineButton>]>,
    ) -> Result<(), String> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
        });
        if let Some(pm) = parse_mode {
            body["parse_mode"] = json!(pm);
        }
        if let Some(rows) = buttons {
            body["reply_markup"] = inline_keyboard_json(rows);
        }
        let _: Value = self.post_json("editMessageText", &body)?;
        Ok(())
    }

    fn delete_message(&mut self, chat_id: i64, message_id: i64) -> Result<(), String> {
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });
        let _: Value = self.post_json("deleteMessage", &body)?;
        Ok(())
    }

    fn answer_callback_query(
        &mut self,
        callback_query_id: &str,
        text: Option<&str>,
        show_alert: bool,
    ) -> Result<(), String> {
        let mut body = json!({
            "callback_query_id": callback_query_id,
            "show_alert": show_alert,
        });
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        let _: Value = self.post_json("answerCallbackQuery", &body)?;
        Ok(())
    }

    fn download_file(&mut self, file_id: &str) -> Result<Vec<u8>, String> {
        let file: TgFile = self.post_json("getFile", &json!({ "file_id": file_id }))?;
        let file_path = file
            .file_path
            .ok_or_else(|| format!("telegram getFile returned no file_path for '{file_id}'"))?;
        let resp = self
            .client
            .get(self.file_url(&file_path))
            .send()
            .map_err(|e| format!("telegram file download error: {}", self.redact(e)))?;
        if !resp.status().is_success() {
            return Err(format!(
                "telegram file download HTTP {}",
                resp.status().as_u16()
            ));
        }
        let bytes = resp
            .bytes()
            .map_err(|e| format!("telegram file read error: {}", self.redact(e)))?;
        Ok(bytes.to_vec())
    }

    fn get_chat_administrators(&mut self, chat_id: i64) -> Result<Vec<TgChatMember>, String> {
        self.post_json("getChatAdministrators", &json!({ "chat_id": chat_id }))
    }
}
