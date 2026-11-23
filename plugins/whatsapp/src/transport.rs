//! Decoupled `WhatsAppTransport` abstraction with `LiveWhatsAppTransport` (over
//! `whatsapp_rust::Client`) and `OfflineWhatsAppTransport` for offline conformance & tests.

use std::collections::HashMap;
use std::sync::Arc;

use whatsapp_rust::media::{self, AudioOptions, DocumentOptions, ImageOptions, VideoOptions};
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::wacore::download::MediaType;
use whatsapp_rust::wacore::proto_helpers::{MessageBuilderExt, MessageExt};
use whatsapp_rust::wacore_binary::JidExt as _;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{ChatStateType, Client, Jid, RevokeType, buffa};

use crate::types::{MediaKind, bare_jid_string};

/// Prepared media item ready for upload + dispatch.
#[derive(Debug, Clone)]
pub struct PreparedMedia {
    pub kind: MediaKind,
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
    pub caption: Option<String>,
    pub ptt: bool,
    pub duration_seconds: Option<u32>,
    pub gif_playback: Option<bool>,
    pub context_info: Option<Box<wa::ContextInfo>>,
}

/// Prepared WebP sticker ready for upload + dispatch.
#[derive(Debug, Clone)]
pub struct PreparedSticker {
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub is_animated: bool,
    pub context_info: Option<Box<wa::ContextInfo>>,
}

/// Transport abstraction isolating `whatsapp-rust` live network I/O from Waist-2 protocol logic.
pub trait WhatsAppTransport {
    /// Own logged-in bare JID if available.
    fn own_jid(&self) -> Option<Jid>;

    /// Sends a text message (optionally with `ContextInfo` for quote-reply / mentions).
    fn send_text(
        &mut self,
        chat: &Jid,
        text: &str,
        context: Option<wa::ContextInfo>,
    ) -> Result<String, String>;

    /// Sends an emoji reaction (`""` removes an existing reaction).
    fn send_reaction(
        &mut self,
        chat: &Jid,
        target_key: wa::MessageKey,
        emoji: &str,
    ) -> Result<String, String>;

    /// Sends a typing / recording / paused chatstate indicator.
    fn send_chatstate(&mut self, chat: &Jid, state: ChatStateType) -> Result<(), String>;

    /// Uploads and sends one or more media items sequentially, returning delivered message IDs.
    fn send_media(&mut self, chat: &Jid, items: &[PreparedMedia]) -> Result<Vec<String>, String>;

    /// Uploads and sends a WebP sticker.
    fn send_sticker(&mut self, chat: &Jid, sticker: &PreparedSticker) -> Result<String, String>;

    /// Creates and sends a poll, returning `(message_id, message_secret)`.
    fn send_poll(
        &mut self,
        chat: &Jid,
        question: &str,
        options: &[String],
        selectable_count: u32,
    ) -> Result<(String, Vec<u8>), String>;

    /// Sends read receipts for `message_ids`.
    fn mark_read(
        &mut self,
        chat: &Jid,
        sender: Option<&Jid>,
        message_ids: &[String],
    ) -> Result<(), String>;

    /// Edits a previously sent message.
    fn edit_message(
        &mut self,
        chat: &Jid,
        original_id: &str,
        new_text: &str,
    ) -> Result<String, String>;

    /// Revokes/deletes a message for everyone.
    fn revoke_message(
        &mut self,
        chat: &Jid,
        message_id: &str,
        original_sender: Option<&Jid>,
    ) -> Result<(), String>;

    /// Queries group participants for accurate `VisibleScope` audience construction.
    fn get_group_participants(&mut self, group: &Jid) -> Result<Vec<Jid>, String>;

    /// Downloads decrypted media bytes from an inbound WhatsApp message if downloadable.
    fn download_media(&mut self, message: &wa::Message) -> Result<Option<Vec<u8>>, String>;

    /// Looks up the phone-number JID (`<digits>@s.whatsapp.net`) the client knows for a
    /// LID JID. `None` when no mapping is stored.
    fn phone_for_lid(&mut self, _lid: &Jid) -> Option<Jid> {
        None
    }
}

/// Live WhatsApp Web E2EE transport backed by `Arc<whatsapp_rust::Client>`.
///
/// Must be called from a blocking context (e.g. `tokio::task::spawn_blocking` or
/// `tokio::task::block_in_place`) on a multi-threaded Tokio runtime.
pub struct LiveWhatsAppTransport {
    client: Arc<Client>,
    rt: tokio::runtime::Handle,
}

impl LiveWhatsAppTransport {
    /// Wraps an initialized `whatsapp_rust::Client` and Tokio runtime handle.
    #[must_use]
    pub fn new(client: Arc<Client>, rt: tokio::runtime::Handle) -> Self {
        Self { client, rt }
    }

    /// Borrows the underlying `whatsapp_rust::Client`.
    #[must_use]
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }
}

impl WhatsAppTransport for LiveWhatsAppTransport {
    fn own_jid(&self) -> Option<Jid> {
        self.client.pn().or_else(|| self.client.lid())
    }

    fn send_text(
        &mut self,
        chat: &Jid,
        text: &str,
        context: Option<wa::ContextInfo>,
    ) -> Result<String, String> {
        let msg = match context {
            Some(ctx) => wa::Message::text_with_context(text, ctx),
            None => wa::Message::text(text),
        };
        let res = self
            .rt
            .block_on(self.client.send_message(chat.clone(), msg))
            .map_err(|e| format!("send_message failed: {e}"))?;
        Ok(res.message_id)
    }

    fn send_reaction(
        &mut self,
        chat: &Jid,
        target_key: wa::MessageKey,
        emoji: &str,
    ) -> Result<String, String> {
        let res = self
            .rt
            .block_on(self.client.send_reaction(chat.clone(), target_key, emoji))
            .map_err(|e| format!("send_reaction failed: {e}"))?;
        Ok(res.message_id)
    }

    fn send_chatstate(&mut self, chat: &Jid, state: ChatStateType) -> Result<(), String> {
        self.rt
            .block_on(self.client.chatstate().send(chat, state))
            .map_err(|e| format!("chatstate send failed: {e}"))
    }

    fn send_media(&mut self, chat: &Jid, items: &[PreparedMedia]) -> Result<Vec<String>, String> {
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            let media_type = match item.kind {
                MediaKind::Image => MediaType::Image,
                MediaKind::Video => MediaType::Video,
                MediaKind::Document => MediaType::Document,
                MediaKind::Audio | MediaKind::Voice => MediaType::Audio,
            };
            let upload = self
                .rt
                .block_on(self.client.upload(
                    item.bytes.clone(),
                    media_type,
                    UploadOptions::default(),
                ))
                .map_err(|e| format!("media upload failed: {e}"))?;

            let msg = match item.kind {
                MediaKind::Image => media::image_message(
                    upload,
                    ImageOptions {
                        caption: item.caption.clone(),
                        mimetype: item.mime_type.clone(),
                        context_info: item.context_info.clone(),
                        ..Default::default()
                    },
                ),
                MediaKind::Video => media::video_message(
                    upload,
                    VideoOptions {
                        caption: item.caption.clone(),
                        mimetype: item.mime_type.clone(),
                        duration_seconds: item.duration_seconds,
                        gif_playback: item.gif_playback,
                        context_info: item.context_info.clone(),
                        ..Default::default()
                    },
                ),
                MediaKind::Document => media::document_message(
                    upload,
                    DocumentOptions {
                        mimetype: item.mime_type.clone(),
                        file_name: item.filename.clone(),
                        caption: item.caption.clone(),
                        context_info: item.context_info.clone(),
                        ..Default::default()
                    },
                ),
                MediaKind::Audio | MediaKind::Voice => media::audio_message(
                    upload,
                    AudioOptions {
                        mimetype: item.mime_type.clone(),
                        duration_seconds: item.duration_seconds,
                        ptt: Some(item.ptt || item.kind == MediaKind::Voice),
                        context_info: item.context_info.clone(),
                        ..Default::default()
                    },
                ),
            };
            let res = self
                .rt
                .block_on(self.client.send_message(chat.clone(), msg))
                .map_err(|e| format!("media send_message failed: {e}"))?;
            ids.push(res.message_id);
        }
        Ok(ids)
    }

    fn send_sticker(&mut self, chat: &Jid, sticker: &PreparedSticker) -> Result<String, String> {
        let upload = self
            .rt
            .block_on(self.client.upload(
                sticker.bytes.clone(),
                MediaType::Sticker,
                UploadOptions::default(),
            ))
            .map_err(|e| format!("sticker upload failed: {e}"))?;
        let msg = wa::Message {
            sticker_message: buffa::MessageField::some(wa::message::StickerMessage {
                url: Some(upload.url),
                direct_path: Some(upload.direct_path),
                media_key: Some(upload.media_key.to_vec()),
                file_sha256: Some(upload.file_sha256.to_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                file_length: Some(upload.file_length),
                media_key_timestamp: Some(upload.media_key_timestamp),
                mimetype: Some(
                    sticker
                        .mime_type
                        .clone()
                        .unwrap_or_else(|| "image/webp".to_owned()),
                ),
                is_animated: Some(sticker.is_animated),
                context_info: sticker
                    .context_info
                    .clone()
                    .map(|ci| buffa::MessageField::some(*ci))
                    .unwrap_or_default(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let res = self
            .rt
            .block_on(self.client.send_message(chat.clone(), msg))
            .map_err(|e| format!("sticker send_message failed: {e}"))?;
        Ok(res.message_id)
    }

    fn send_poll(
        &mut self,
        chat: &Jid,
        question: &str,
        options: &[String],
        selectable_count: u32,
    ) -> Result<(String, Vec<u8>), String> {
        let (res, secret) = self
            .rt
            .block_on(
                self.client
                    .polls()
                    .create(chat.clone(), question, options, selectable_count),
            )
            .map_err(|e| format!("poll create failed: {e}"))?;
        Ok((res.message_id, secret))
    }

    fn mark_read(
        &mut self,
        chat: &Jid,
        sender: Option<&Jid>,
        message_ids: &[String],
    ) -> Result<(), String> {
        let id_refs: Vec<&str> = message_ids.iter().map(String::as_str).collect();
        self.rt
            .block_on(self.client.mark_as_read(chat, sender, &id_refs))
            .map_err(|e| format!("mark_as_read failed: {e}"))
    }

    fn edit_message(
        &mut self,
        chat: &Jid,
        original_id: &str,
        new_text: &str,
    ) -> Result<String, String> {
        self.rt
            .block_on(self.client.edit_message(
                chat.clone(),
                original_id,
                wa::Message::text(new_text),
            ))
            .map_err(|e| format!("edit_message failed: {e}"))
    }

    fn revoke_message(
        &mut self,
        chat: &Jid,
        message_id: &str,
        original_sender: Option<&Jid>,
    ) -> Result<(), String> {
        let revoke_type = match original_sender {
            Some(sender) if chat.is_group() => RevokeType::Admin {
                original_sender: sender.clone(),
            },
            _ => RevokeType::Sender,
        };
        self.rt
            .block_on(self.client.revoke_message(chat.clone(), message_id, revoke_type))
            .map_err(|e| format!("revoke_message failed: {e}"))
    }

    fn get_group_participants(&mut self, group: &Jid) -> Result<Vec<Jid>, String> {
        let meta = self
            .rt
            .block_on(self.client.groups().get_metadata(group))
            .map_err(|e| format!("get_metadata failed: {e}"))?;
        Ok(meta
            .participants
            .into_iter()
            .map(|p| p.phone_number.unwrap_or(p.jid).to_non_ad())
            .collect())
    }

    fn phone_for_lid(&mut self, lid: &Jid) -> Option<Jid> {
        if !lid.is_lid() {
            return None;
        }
        let entry = self
            .rt
            .block_on(self.client.get_lid_pn_entry(lid))
            .ok()
            .flatten()?;
        Some(Jid::pn(entry.phone_number.to_string()))
    }

    fn download_media(&mut self, message: &wa::Message) -> Result<Option<Vec<u8>>, String> {
        let base = message.get_base_message();
        if let Some(img) = base.image_message.as_option() {
            return self
                .rt
                .block_on(self.client.download(img))
                .map(Some)
                .map_err(|e| format!("image download failed: {e}"));
        }
        if let Some(vid) = base.video_message.as_option() {
            return self
                .rt
                .block_on(self.client.download(vid))
                .map(Some)
                .map_err(|e| format!("video download failed: {e}"));
        }
        if let Some(doc) = base.document_message.as_option() {
            return self
                .rt
                .block_on(self.client.download(doc))
                .map(Some)
                .map_err(|e| format!("document download failed: {e}"));
        }
        if let Some(aud) = base.audio_message.as_option() {
            return self
                .rt
                .block_on(self.client.download(aud))
                .map(Some)
                .map_err(|e| format!("audio download failed: {e}"));
        }
        if let Some(stk) = base.sticker_message.as_option() {
            return self
                .rt
                .block_on(self.client.download(stk))
                .map(Some)
                .map_err(|e| format!("sticker download failed: {e}"));
        }
        Ok(None)
    }
}

/// Recorded outbound action captured by [`OfflineWhatsAppTransport`] for deterministic testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedOutboundAction {
    Text {
        chat: String,
        text: String,
        reply_to_message_id: Option<String>,
        mentions: Vec<String>,
    },
    Reaction {
        chat: String,
        target_id: String,
        emoji: String,
        from_me: Option<bool>,
        participant: Option<String>,
    },
    Chatstate {
        chat: String,
        state: String,
    },
    Media {
        chat: String,
        items: Vec<(MediaKind, usize, Option<String>, bool)>,
    },
    Sticker {
        chat: String,
        byte_len: usize,
        is_animated: bool,
    },
    Poll {
        chat: String,
        question: String,
        options: Vec<String>,
        selectable_count: u32,
    },
    MarkRead {
        chat: String,
        sender: Option<String>,
        message_ids: Vec<String>,
    },
    Edit {
        chat: String,
        original_id: String,
        new_text: String,
    },
    Revoke {
        chat: String,
        message_id: String,
        original_sender: Option<String>,
    },
}

/// Offline & test transport recording all outbound operations without network access.
#[derive(Debug, Default)]
pub struct OfflineWhatsAppTransport {
    pub own_jid: Option<Jid>,
    pub next_seq: u64,
    pub recorded: Vec<RecordedOutboundAction>,
    pub group_participants: HashMap<String, Vec<Jid>>,
    /// LID user part -> phone-number user part (offline stand-in for the client mapping).
    pub lid_to_phone: HashMap<String, String>,
    pub mock_media_bytes: Option<Vec<u8>>,
    pub fail_next_send: Option<String>,
}

impl OfflineWhatsAppTransport {
    /// Creates an empty [`OfflineWhatsAppTransport`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn next_msg_id(&mut self) -> String {
        self.next_seq = self.next_seq.saturating_add(1);
        format!("wa-offline-msg-{}", self.next_seq)
    }

    fn check_failure(&mut self) -> Result<(), String> {
        if let Some(err) = self.fail_next_send.take() {
            return Err(err);
        }
        Ok(())
    }
}

impl WhatsAppTransport for OfflineWhatsAppTransport {
    fn own_jid(&self) -> Option<Jid> {
        self.own_jid.clone()
    }

    fn send_text(
        &mut self,
        chat: &Jid,
        text: &str,
        context: Option<wa::ContextInfo>,
    ) -> Result<String, String> {
        self.check_failure()?;
        let pid = self.next_msg_id();
        self.recorded.push(RecordedOutboundAction::Text {
            chat: bare_jid_string(chat),
            text: text.to_owned(),
            reply_to_message_id: context.as_ref().and_then(|c| c.stanza_id.clone()),
            mentions: context
                .as_ref()
                .map(|c| c.mentioned_jid.clone())
                .unwrap_or_default(),
        });
        Ok(pid)
    }

    fn send_reaction(
        &mut self,
        chat: &Jid,
        target_key: wa::MessageKey,
        emoji: &str,
    ) -> Result<String, String> {
        self.check_failure()?;
        let pid = self.next_msg_id();
        self.recorded.push(RecordedOutboundAction::Reaction {
            chat: bare_jid_string(chat),
            target_id: target_key.id.unwrap_or_default(),
            emoji: emoji.to_owned(),
            from_me: target_key.from_me,
            participant: target_key.participant,
        });
        Ok(pid)
    }

    fn send_chatstate(&mut self, chat: &Jid, state: ChatStateType) -> Result<(), String> {
        self.recorded.push(RecordedOutboundAction::Chatstate {
            chat: bare_jid_string(chat),
            state: state.as_str().to_owned(),
        });
        Ok(())
    }

    fn send_media(&mut self, chat: &Jid, items: &[PreparedMedia]) -> Result<Vec<String>, String> {
        self.check_failure()?;
        let mut pids = Vec::with_capacity(items.len());
        let mut summary = Vec::with_capacity(items.len());
        for item in items {
            pids.push(self.next_msg_id());
            summary.push((
                item.kind,
                item.bytes.len(),
                item.caption.clone(),
                item.ptt || item.kind == MediaKind::Voice,
            ));
        }
        self.recorded.push(RecordedOutboundAction::Media {
            chat: bare_jid_string(chat),
            items: summary,
        });
        Ok(pids)
    }

    fn send_sticker(&mut self, chat: &Jid, sticker: &PreparedSticker) -> Result<String, String> {
        self.check_failure()?;
        let pid = self.next_msg_id();
        self.recorded.push(RecordedOutboundAction::Sticker {
            chat: bare_jid_string(chat),
            byte_len: sticker.bytes.len(),
            is_animated: sticker.is_animated,
        });
        Ok(pid)
    }

    fn send_poll(
        &mut self,
        chat: &Jid,
        question: &str,
        options: &[String],
        selectable_count: u32,
    ) -> Result<(String, Vec<u8>), String> {
        self.check_failure()?;
        let pid = self.next_msg_id();
        self.recorded.push(RecordedOutboundAction::Poll {
            chat: bare_jid_string(chat),
            question: question.to_owned(),
            options: options.to_vec(),
            selectable_count,
        });
        Ok((pid, vec![0x42u8; 32]))
    }

    fn mark_read(
        &mut self,
        chat: &Jid,
        sender: Option<&Jid>,
        message_ids: &[String],
    ) -> Result<(), String> {
        self.check_failure()?;
        self.recorded.push(RecordedOutboundAction::MarkRead {
            chat: bare_jid_string(chat),
            sender: sender.map(bare_jid_string),
            message_ids: message_ids.to_vec(),
        });
        Ok(())
    }

    fn edit_message(
        &mut self,
        chat: &Jid,
        original_id: &str,
        new_text: &str,
    ) -> Result<String, String> {
        self.check_failure()?;
        let pid = self.next_msg_id();
        self.recorded.push(RecordedOutboundAction::Edit {
            chat: bare_jid_string(chat),
            original_id: original_id.to_owned(),
            new_text: new_text.to_owned(),
        });
        Ok(pid)
    }

    fn revoke_message(
        &mut self,
        chat: &Jid,
        message_id: &str,
        original_sender: Option<&Jid>,
    ) -> Result<(), String> {
        self.check_failure()?;
        self.recorded.push(RecordedOutboundAction::Revoke {
            chat: bare_jid_string(chat),
            message_id: message_id.to_owned(),
            original_sender: original_sender.map(bare_jid_string),
        });
        Ok(())
    }

    fn get_group_participants(&mut self, group: &Jid) -> Result<Vec<Jid>, String> {
        Ok(self
            .group_participants
            .get(&bare_jid_string(group))
            .cloned()
            .unwrap_or_default())
    }

    fn download_media(&mut self, _message: &wa::Message) -> Result<Option<Vec<u8>>, String> {
        Ok(self.mock_media_bytes.clone())
    }

    fn phone_for_lid(&mut self, lid: &Jid) -> Option<Jid> {
        if !lid.is_lid() {
            return None;
        }
        self.lid_to_phone
            .get(&lid.user.to_string())
            .map(|p| Jid::pn(p.clone()))
    }
}
