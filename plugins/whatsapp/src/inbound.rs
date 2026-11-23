//! Inbound WhatsApp event translation, history/pre-boot filtering, self-echo
//! suppression, group audience caching, poll vote decryption, and attachment staging.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use monoagens_body_kit::EchoRegistry;
use monoagens_body_sdk::{
    ActionId, ActorId, ApprovalResolveParams, AttachmentHandle, Audience, BlobPutResult, ChannelId,
    CorrelationId, HAct, ModalitySlot, OpaquePayload, OriginActor, PerceptEmitParams, Taint, Time,
    Waist2Headers, sha256_hex,
};
use serde_json::{Value, json};
use whatsapp_rust::wacore::poll::{
    PollVoteAddressing, PollVoteCiphertext, compute_option_hash, decrypt_poll_vote_with_fallback,
};
use whatsapp_rust::wacore::proto_helpers::MessageExt as _;
use whatsapp_rust::wacore::types::events::InboundMessage;
use whatsapp_rust::wacore_binary::JidExt as _;
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::transport::WhatsAppTransport;
use crate::types::{
    INLINE_ATTACHMENT_CAP_BYTES, PRE_BOOT_GRACE_MS, bare_jid_string, jid_to_actor, jid_to_place,
    parse_user_jid,
};

/// Stages large outbound-to-Gateway media payloads (`> 256 KiB`) via `blob/put`.
pub trait BlobStager {
    fn stage_blob(
        &mut self,
        bytes: &[u8],
        content_type: Option<&str>,
        scope_id: Option<&str>,
    ) -> Result<BlobPutResult, String>;
}

/// Default in-memory [`BlobStager`] used in unit tests and offline mode.
#[derive(Debug, Default)]
pub struct MemoryBlobStager {
    pub blobs: HashMap<String, Vec<u8>>,
}

impl BlobStager for MemoryBlobStager {
    fn stage_blob(
        &mut self,
        bytes: &[u8],
        _content_type: Option<&str>,
        _scope_id: Option<&str>,
    ) -> Result<BlobPutResult, String> {
        let digest = sha256_hex(bytes);
        let blob_ref = format!("blob://sha256/{digest}");
        self.blobs.insert(blob_ref.clone(), bytes.to_vec());
        Ok(BlobPutResult {
            blob_ref,
            sha256: digest,
            expires_at_ms: Time::from_millis(60_000),
        })
    }
}

/// Per-group participant cache ensuring non-empty `audience` on every group percept.
#[derive(Debug, Default, Clone)]
pub struct GroupAudienceCache {
    groups: HashMap<String, BTreeSet<ActorId>>,
}

impl GroupAudienceCache {
    /// Seeds or replaces cached participants for `group_jid`.
    pub fn seed(&mut self, group_jid: &Jid, participants: impl IntoIterator<Item = Jid>) {
        let key = bare_jid_string(group_jid);
        let entry = self.groups.entry(key).or_default();
        for p in participants {
            entry.insert(jid_to_actor(&p));
        }
    }

    /// Resolves non-empty [`Audience`] for `group_jid`, querying `transport` on cache miss
    /// and always including `sender_actor`.
    pub fn resolve(
        &mut self,
        group_jid: &Jid,
        sender_actor: &ActorId,
        transport: &mut dyn WhatsAppTransport,
    ) -> Audience {
        let key = bare_jid_string(group_jid);
        if !self.groups.contains_key(&key)
            && let Ok(participants) = transport.get_group_participants(group_jid)
        {
            let stable: Vec<Jid> = participants
                .into_iter()
                .map(|p| {
                    if p.is_lid() {
                        transport.phone_for_lid(&p).unwrap_or(p)
                    } else {
                        p
                    }
                })
                .collect();
            self.seed(group_jid, stable);
        }
        let set = self.groups.entry(key).or_default();
        set.insert(sender_actor.clone());
        Audience::new(set.iter().cloned())
    }
}

/// Cached poll metadata for decrypting incoming `poll_update_message` votes.
#[derive(Debug, Clone)]
pub struct CachedPoll {
    pub question: String,
    pub options: Vec<String>,
    pub creator_jid: Option<Jid>,
    pub message_secret: Vec<u8>,
}

/// Bounded map of `poll_msg_id -> CachedPoll`.
#[derive(Debug, Default, Clone)]
pub struct PollSecretCache {
    polls: HashMap<String, CachedPoll>,
}

impl PollSecretCache {
    /// Registers a poll's `message_secret` and option names for vote decryption.
    pub fn insert(&mut self, poll_msg_id: impl Into<String>, poll: CachedPoll) {
        self.polls.insert(poll_msg_id.into(), poll);
    }

    /// Looks up cached poll metadata by `poll_msg_id`.
    #[must_use]
    pub fn get(&self, poll_msg_id: &str) -> Option<&CachedPoll> {
        self.polls.get(poll_msg_id)
    }
}

/// Default maximum number of pending `approval.prompt` message bindings retained in memory.
pub const DEFAULT_MAX_PENDING_APPROVALS: usize = 1_024;

/// Bound approval metadata associated with a sent WhatsApp `approval.prompt` message ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApprovalBinding {
    pub action_id: String,
    pub h_act: String,
    pub state_epoch: u64,
    pub approve_emojis: Vec<String>,
    pub deny_emojis: Vec<String>,
    pub expires_at_ms: u64,
}

/// Bounded FIFO map from WhatsApp `prompt_message_id` to [`PendingApprovalBinding`].
///
/// Bindings are kept non-destructively across reactions so a non-owner reaction does not
/// consume the prompt before the owner reacts; Gateway/Valve enforces single settlement.
#[derive(Debug, Clone)]
pub struct PendingApprovalPromptMap {
    capacity: usize,
    order: VecDeque<String>,
    bindings: HashMap<String, PendingApprovalBinding>,
}

impl Default for PendingApprovalPromptMap {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PENDING_APPROVALS)
    }
}

impl PendingApprovalPromptMap {
    /// Creates a bounded [`PendingApprovalPromptMap`] retaining at most `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::new(),
            bindings: HashMap::new(),
        }
    }

    /// Binds `message_id` to `binding`, evicting the oldest entry if `capacity` is reached.
    pub fn insert(&mut self, message_id: impl Into<String>, binding: PendingApprovalBinding) {
        if self.capacity == 0 {
            return;
        }
        let message_id = message_id.into();
        if let Some(existing) = self.bindings.get_mut(&message_id) {
            *existing = binding;
            return;
        }
        while self.bindings.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.bindings.remove(&oldest);
            } else {
                break;
            }
        }
        self.order.push_back(message_id.clone());
        self.bindings.insert(message_id, binding);
    }

    /// Looks up the [`PendingApprovalBinding`] for a WhatsApp `message_id`.
    #[must_use]
    pub fn get(&self, message_id: &str) -> Option<&PendingApprovalBinding> {
        self.bindings.get(message_id)
    }

    /// Returns the number of tracked approval prompt bindings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns `true` when no approval prompts are currently tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Outbound Waist-2 frame produced from an inbound WhatsApp message.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundAction {
    PerceptEmit(Box<PerceptEmitParams>),
    ApprovalResolve(ApprovalResolveParams),
}

/// Translates live `whatsapp-rust` [`InboundMessage`]s into Waist-2 `percept/emit` or
/// `approval/resolve` payloads.
#[derive(Debug)]
pub struct InboundTranslator {
    boot_timestamp_ms: u64,
    grace_ms: u64,
    pub group_audience: GroupAudienceCache,
    pub poll_secrets: PollSecretCache,
    /// LIDs already reported once as having no known phone number.
    unmapped_lids_logged: HashSet<String>,
}

impl InboundTranslator {
    /// Creates an [`InboundTranslator`] anchored at `boot_timestamp_ms` with [`PRE_BOOT_GRACE_MS`].
    #[must_use]
    pub fn new(boot_timestamp_ms: u64) -> Self {
        Self {
            boot_timestamp_ms,
            grace_ms: PRE_BOOT_GRACE_MS,
            group_audience: GroupAudienceCache::default(),
            poll_secrets: PollSecretCache::default(),
            unmapped_lids_logged: HashSet::new(),
        }
    }

    /// Resolves `jid` to its stable phone-number JID when WhatsApp addressed it by LID.
    ///
    /// Order: the message's alternate JID (when it is a phone-number JID), then the
    /// client's LID-to-phone mapping, else the LID itself with one info line per LID.
    fn stable_jid(
        &mut self,
        jid: &Jid,
        alt: Option<&Jid>,
        transport: &mut dyn WhatsAppTransport,
    ) -> Jid {
        if !jid.is_lid() {
            return jid.to_non_ad();
        }
        if let Some(alt) = alt
            && alt.is_pn()
        {
            return alt.to_non_ad();
        }
        if let Some(pn) = transport.phone_for_lid(jid)
            && pn.is_pn()
        {
            return pn.to_non_ad();
        }
        let lid = bare_jid_string(jid);
        if self.unmapped_lids_logged.insert(lid.clone()) {
            eprintln!(
                "whatsapp: no phone number known for {lid}; using the LID account (link it to the owner to restore identity)"
            );
        }
        jid.to_non_ad()
    }

    /// Overrides the clock-skew grace window (useful in deterministic tests).
    #[must_use]
    pub const fn with_grace_ms(mut self, grace_ms: u64) -> Self {
        self.grace_ms = grace_ms;
        self
    }

    /// Returns `true` if `msg_timestamp_ms` predates `boot_timestamp_ms - grace_ms`.
    #[must_use]
    pub fn is_pre_boot(&self, msg_timestamp_ms: u64) -> bool {
        let cutoff = self.boot_timestamp_ms.saturating_sub(self.grace_ms);
        msg_timestamp_ms < cutoff
    }

    /// Translates an [`InboundMessage`] to [`PerceptEmitParams`], returning `None` when the
    /// message must be dropped (pre-boot history, self-echo, or empty unsupported control).
    pub fn translate_inbound(
        &mut self,
        inbound: &InboundMessage,
        transport: &mut dyn WhatsAppTransport,
        echo_registry: &EchoRegistry,
        blob_stager: &mut dyn BlobStager,
        idempotency_key: impl Into<String>,
    ) -> Option<PerceptEmitParams> {
        let empty_approvals = PendingApprovalPromptMap::new(0);
        match self.translate_inbound_action(
            inbound,
            transport,
            echo_registry,
            &empty_approvals,
            blob_stager,
            idempotency_key,
        )? {
            InboundAction::PerceptEmit(percept) => Some(*percept),
            InboundAction::ApprovalResolve(_) => None,
        }
    }

    /// Translates an [`InboundMessage`] to an [`InboundAction`] (`percept/emit` or
    /// `approval/resolve` when a `ReactionMessage` matches a pending `approval.prompt`).
    pub fn translate_inbound_action(
        &mut self,
        inbound: &InboundMessage,
        transport: &mut dyn WhatsAppTransport,
        echo_registry: &EchoRegistry,
        pending_approvals: &PendingApprovalPromptMap,
        blob_stager: &mut dyn BlobStager,
        idempotency_key: impl Into<String>,
    ) -> Option<InboundAction> {
        let ts_ms_i64 = inbound.info.timestamp.timestamp_millis();
        let msg_timestamp_ms = u64::try_from(ts_ms_i64).unwrap_or(0);

        if self.is_pre_boot(msg_timestamp_ms) {
            return None;
        }
        if inbound.info.source.is_from_me {
            return None;
        }
        if echo_registry.is_self_echo(&inbound.info.id) {
            return None;
        }

        let source = &inbound.info.source;
        let is_group = source.is_group || source.chat.is_group();
        let sender_jid = &self.stable_jid(&source.sender, source.sender_alt.as_ref(), transport);
        // A DM chat addressed by the sender's own LID shares that LID's alternate JID.
        let chat_alt = if !is_group && source.chat.user == source.sender.user {
            source.sender_alt.as_ref()
        } else {
            None
        };
        let chat_jid = &if is_group {
            source.chat.clone()
        } else {
            self.stable_jid(&source.chat, chat_alt, transport)
        };
        let base = inbound.message.get_base_message();

        if let Some(react) = base.reaction_message.as_option()
            && let Some(emoji) = react.text.as_deref().map(str::trim).filter(|s| !s.is_empty())
            && let Some(target_id) = react.key.as_option().and_then(|k| k.id.as_deref())
            && let Some(binding) = pending_approvals.get(target_id)
        {
            let approved = if binding.approve_emojis.iter().any(|e| e == emoji) {
                Some(true)
            } else if binding.deny_emojis.iter().any(|e| e == emoji) {
                Some(false)
            } else {
                None
            };
            if let Some(approved) = approved {
                return Some(InboundAction::ApprovalResolve(ApprovalResolveParams {
                    headers: Waist2Headers::new(idempotency_key),
                    action_id: ActionId::new(binding.action_id.clone()),
                    h_act: HAct::new(binding.h_act.clone()),
                    state_epoch: binding.state_epoch,
                    approved,
                    resolver: jid_to_actor(sender_jid),
                    resolved_at_ms: Time::from_millis(msg_timestamp_ms),
                    signature: None,
                    out_of_band_channel: false,
                }));
            }
        }

        let place = jid_to_place(chat_jid);
        let actor = jid_to_actor(sender_jid);

        let audience = if is_group {
            self.group_audience.resolve(chat_jid, &actor, transport)
        } else {
            let mut set = BTreeSet::new();
            set.insert(actor.clone());
            if let Some(own) = transport.own_jid() {
                set.insert(jid_to_actor(&own));
            }
            Audience::new(set)
        };

        // Cache poll creation secrets if an incoming poll carries messageSecret.
        if let Some(poll_creation) = base
            .poll_creation_message
            .as_option()
            .or_else(|| base.poll_creation_message_v2.as_option())
            .or_else(|| base.poll_creation_message_v3.as_option())
            && let Some(secret) = inbound
                .message
                .message_context_info
                .as_option()
                .and_then(|m| m.message_secret.as_ref())
        {
            let options = poll_creation
                .options
                .iter()
                .filter_map(|o| o.option_name.clone())
                .collect();
            self.poll_secrets.insert(
                inbound.info.id.clone(),
                CachedPoll {
                    question: poll_creation.name.clone().unwrap_or_default(),
                    options,
                    // Poll vote crypto binds to the raw wire address, not the stable account.
                    creator_jid: Some(source.sender.to_non_ad()),
                    message_secret: secret.clone(),
                },
            );
        }

        let mut attachments = Vec::new();
        let (payload_json, named_jids, reply_to_pid) =
            self.extract_payload(inbound, base, &place, transport, blob_stager, &mut attachments)?;

        let payload_bytes = serde_json::to_vec(&payload_json).ok()?;
        let length = payload_bytes.len() as u64;

        let named: Vec<ActorId> = named_jids
            .into_iter()
            .filter_map(|raw| parse_user_jid(&raw).ok())
            .map(|j| {
                let j = self.stable_jid(&j, None, transport);
                jid_to_actor(&j)
            })
            .collect();

        let correlation_id = reply_to_pid
            .as_deref()
            .and_then(|pid| echo_registry.resolve_reply_to(pid))
            .map(|rec| CorrelationId::new(rec.action_id.as_str()))
            .or_else(|| Some(CorrelationId::new(inbound.info.id.clone())));

        Some(InboundAction::PerceptEmit(Box::new(PerceptEmitParams {
            headers: Waist2Headers::new(idempotency_key),
            actor: actor.clone(),
            origin_actor: Some(OriginActor::new(actor)),
            place: place.clone(),
            audience: Some(audience),
            timestamp_ms: Time::from_millis(msg_timestamp_ms),
            payload: OpaquePayload::from_bytes(payload_bytes),
            taint: Taint::Untrusted,
            addressees: Vec::new(),
            named,
            orient: Vec::new(),
            reply_to: None,
            correlation_id,
            channel: Some(ChannelId::new(place.as_str())),
            delta_obs: None,
            channel_prediction: None,
            length: Some(length),
            negative_flag: None,
            repair: None,
            continued_attention: None,
            f_modal: None,
            embeddings: Vec::new(),
            attachments,
            dialogue_act_gap: None,
            peer_expertise_margin: None,
        })))
    }

    fn extract_payload(
        &self,
        inbound: &InboundMessage,
        base: &wa::Message,
        place_str: &monoagens_body_sdk::PlaceId,
        transport: &mut dyn WhatsAppTransport,
        blob_stager: &mut dyn BlobStager,
        attachments: &mut Vec<AttachmentHandle>,
    ) -> Option<(Value, Vec<String>, Option<String>)> {
        let msg_id = &inbound.info.id;

        if let Some(text) = base.conversation.as_deref() {
            return Some((
                json!({
                    "type": "text",
                    "message_id": msg_id,
                    "text": text,
                }),
                Vec::new(),
                None,
            ));
        }

        if let Some(ext) = base.extended_text_message.as_option() {
            let ctx = ext.context_info.as_option();
            let (reply_to, quoted_text, mentions) = extract_context_fields(ctx);
            return Some((
                json!({
                    "type": "text",
                    "message_id": msg_id,
                    "text": ext.text.as_deref().unwrap_or(""),
                    "reply_to_message_id": reply_to,
                    "quoted_text": quoted_text,
                    "mentions": mentions,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(react) = base.reaction_message.as_option() {
            let emoji = react.text.as_deref().unwrap_or("");
            let target_id = react.key.as_option().and_then(|k| k.id.as_deref());
            return Some((
                json!({
                    "type": "reaction",
                    "message_id": msg_id,
                    "target_message_id": target_id,
                    "emoji": emoji,
                    "removed": emoji.is_empty(),
                }),
                Vec::new(),
                target_id.map(ToOwned::to_owned),
            ));
        }

        if let Some(img) = base.image_message.as_option() {
            let ctx = img.context_info.as_option();
            let (reply_to, _, mentions) = extract_context_fields(ctx);
            let media_meta = stage_or_inline_media(
                inbound.message.as_ref(),
                ModalitySlot::Image,
                img.mimetype.as_deref(),
                place_str.as_str(),
                transport,
                blob_stager,
                attachments,
            );
            return Some((
                json!({
                    "type": "media",
                    "kind": "image",
                    "message_id": msg_id,
                    "caption": img.caption,
                    "mime_type": img.mimetype,
                    "file_length": img.file_length,
                    "media": media_meta,
                    "reply_to_message_id": reply_to,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(vid) = base.video_message.as_option() {
            let ctx = vid.context_info.as_option();
            let (reply_to, _, mentions) = extract_context_fields(ctx);
            let media_meta = stage_or_inline_media(
                inbound.message.as_ref(),
                ModalitySlot::Video,
                vid.mimetype.as_deref(),
                place_str.as_str(),
                transport,
                blob_stager,
                attachments,
            );
            return Some((
                json!({
                    "type": "media",
                    "kind": "video",
                    "message_id": msg_id,
                    "caption": vid.caption,
                    "mime_type": vid.mimetype,
                    "seconds": vid.seconds,
                    "gif_playback": vid.gif_playback,
                    "file_length": vid.file_length,
                    "media": media_meta,
                    "reply_to_message_id": reply_to,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(doc) = base.document_message.as_option() {
            let ctx = doc.context_info.as_option();
            let (reply_to, _, mentions) = extract_context_fields(ctx);
            let media_meta = stage_or_inline_media(
                inbound.message.as_ref(),
                ModalitySlot::SensorBlob,
                doc.mimetype.as_deref(),
                place_str.as_str(),
                transport,
                blob_stager,
                attachments,
            );
            return Some((
                json!({
                    "type": "media",
                    "kind": "document",
                    "message_id": msg_id,
                    "filename": doc.file_name,
                    "title": doc.title,
                    "caption": doc.caption,
                    "mime_type": doc.mimetype,
                    "file_length": doc.file_length,
                    "media": media_meta,
                    "reply_to_message_id": reply_to,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(aud) = base.audio_message.as_option() {
            let ctx = aud.context_info.as_option();
            let (reply_to, _, mentions) = extract_context_fields(ctx);
            let ptt = aud.ptt.unwrap_or(false);
            let media_meta = stage_or_inline_media(
                inbound.message.as_ref(),
                ModalitySlot::AudioVoice,
                aud.mimetype.as_deref(),
                place_str.as_str(),
                transport,
                blob_stager,
                attachments,
            );
            return Some((
                json!({
                    "type": "media",
                    "kind": if ptt { "voice" } else { "audio" },
                    "ptt": ptt,
                    "message_id": msg_id,
                    "mime_type": aud.mimetype,
                    "seconds": aud.seconds,
                    "file_length": aud.file_length,
                    "media": media_meta,
                    "reply_to_message_id": reply_to,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(stk) = base.sticker_message.as_option() {
            let ctx = stk.context_info.as_option();
            let (reply_to, _, mentions) = extract_context_fields(ctx);
            let media_meta = stage_or_inline_media(
                inbound.message.as_ref(),
                ModalitySlot::Image,
                stk.mimetype.as_deref(),
                place_str.as_str(),
                transport,
                blob_stager,
                attachments,
            );
            return Some((
                json!({
                    "type": "sticker",
                    "message_id": msg_id,
                    "mime_type": stk.mimetype,
                    "is_animated": stk.is_animated.unwrap_or(false),
                    "file_length": stk.file_length,
                    "media": media_meta,
                    "reply_to_message_id": reply_to,
                }),
                mentions,
                reply_to,
            ));
        }

        if let Some(poll_upd) = base.poll_update_message.as_option() {
            let poll_msg_id = poll_upd
                .poll_creation_message_key
                .as_option()
                .and_then(|k| k.id.as_deref())
                .unwrap_or("");
            let selected_options = self.try_decrypt_poll_vote(inbound, poll_upd, poll_msg_id);
            return Some((
                json!({
                    "type": "poll_vote",
                    "message_id": msg_id,
                    "poll_message_id": poll_msg_id,
                    "selected_options": selected_options,
                }),
                Vec::new(),
                (!poll_msg_id.is_empty()).then(|| poll_msg_id.to_owned()),
            ));
        }

        if let Some(poll_create) = base
            .poll_creation_message
            .as_option()
            .or_else(|| base.poll_creation_message_v2.as_option())
            .or_else(|| base.poll_creation_message_v3.as_option())
        {
            let opts: Vec<String> = poll_create
                .options
                .iter()
                .filter_map(|o| o.option_name.clone())
                .collect();
            return Some((
                json!({
                    "type": "poll_create",
                    "message_id": msg_id,
                    "question": poll_create.name,
                    "options": opts,
                    "selectable_count": poll_create.selectable_options_count,
                }),
                Vec::new(),
                None,
            ));
        }

        if let Some(loc) = base.location_message.as_option() {
            return Some((
                json!({
                    "type": "location",
                    "message_id": msg_id,
                    "latitude": loc.degrees_latitude,
                    "longitude": loc.degrees_longitude,
                    "name": loc.name,
                    "address": loc.address,
                }),
                Vec::new(),
                None,
            ));
        }

        if let Some(contact) = base.contact_message.as_option() {
            return Some((
                json!({
                    "type": "contact",
                    "message_id": msg_id,
                    "display_name": contact.display_name,
                    "vcard": contact.vcard,
                }),
                Vec::new(),
                None,
            ));
        }

        if let Some(pm) = base.protocol_message.as_option() {
            use wa::message::protocol_message::Type as PmType;
            let target_id = pm.key.as_option().and_then(|k| k.id.as_deref());
            match pm.r#type {
                Some(PmType::MessageEdit) => {
                    let edited_text = pm
                        .edited_message
                        .as_option()
                        .and_then(|m| m.text_content())
                        .unwrap_or("");
                    return Some((
                        json!({
                            "type": "edit",
                            "message_id": msg_id,
                            "target_message_id": target_id,
                            "text": edited_text,
                        }),
                        Vec::new(),
                        target_id.map(ToOwned::to_owned),
                    ));
                }
                Some(PmType::Revoke) => {
                    return Some((
                        json!({
                            "type": "revoke",
                            "message_id": msg_id,
                            "target_message_id": target_id,
                        }),
                        Vec::new(),
                        target_id.map(ToOwned::to_owned),
                    ));
                }
                _ => return None,
            }
        }

        None
    }

    fn try_decrypt_poll_vote(
        &self,
        inbound: &InboundMessage,
        poll_upd: &wa::message::PollUpdateMessage,
        poll_msg_id: &str,
    ) -> Option<Vec<String>> {
        let cached = self.poll_secrets.get(poll_msg_id)?;
        let vote = poll_upd.vote.as_option()?;
        let enc_payload = vote.enc_payload.as_deref()?;
        let enc_iv = vote.enc_iv.as_deref()?;
        let creator_jid = cached
            .creator_jid
            .as_ref()
            .map(bare_jid_string)
            .or_else(|| {
                poll_upd
                    .poll_creation_message_key
                    .as_option()
                    .and_then(|k| k.participant.clone())
            })?;
        let voter_jid = bare_jid_string(&inbound.info.source.sender);

        let hashes = decrypt_poll_vote_with_fallback(
            PollVoteCiphertext {
                enc_payload,
                enc_iv,
            },
            &cached.message_secret,
            poll_msg_id,
            PollVoteAddressing {
                poll_creator_jid: &creator_jid,
                voter_jid: &voter_jid,
            },
            None,
        )
        .ok()?;

        let selected: Vec<String> = cached
            .options
            .iter()
            .filter(|opt| {
                let h = compute_option_hash(opt);
                hashes.iter().any(|sel| sel.as_slice() == h)
            })
            .cloned()
            .collect();
        Some(selected)
    }
}

fn extract_context_fields(
    ctx: Option<&wa::ContextInfo>,
) -> (Option<String>, Option<String>, Vec<String>) {
    let Some(ctx) = ctx else {
        return (None, None, Vec::new());
    };
    let reply_to = ctx.stanza_id.clone();
    let quoted_text = ctx
        .quoted_message
        .as_option()
        .and_then(|q| q.text_content())
        .map(ToOwned::to_owned);
    let mentions = ctx.mentioned_jid.clone();
    (reply_to, quoted_text, mentions)
}

fn stage_or_inline_media(
    msg: &wa::Message,
    modality_slot: ModalitySlot,
    content_type: Option<&str>,
    scope_id: &str,
    transport: &mut dyn WhatsAppTransport,
    blob_stager: &mut dyn BlobStager,
    attachments: &mut Vec<AttachmentHandle>,
) -> Value {
    let Ok(Some(bytes)) = transport.download_media(msg) else {
        return json!({ "custody": "metadata_only" });
    };
    let byte_len = bytes.len() as u64;
    let digest = sha256_hex(&bytes);
    if bytes.len() > INLINE_ATTACHMENT_CAP_BYTES {
        match blob_stager.stage_blob(&bytes, content_type, Some(scope_id)) {
            Ok(put) => {
                attachments.push(AttachmentHandle {
                    handle_id: put.blob_ref.clone(),
                    modality_slot,
                    byte_len,
                    expires_at_ms: put.expires_at_ms,
                    embed_model_ver: "raw-media-v1".to_owned(),
                });
                json!({
                    "custody": "blob_ref",
                    "blob_ref": put.blob_ref,
                    "sha256": put.sha256,
                    "byte_len": byte_len,
                })
            }
            Err(err) => {
                json!({
                    "custody": "stage_failed",
                    "error": err,
                    "byte_len": byte_len,
                })
            }
        }
    } else {
        json!({
            "custody": "inline_base64",
            "sha256": digest,
            "byte_len": byte_len,
            "bytes_base64": BASE64_STANDARD.encode(&bytes),
        })
    }
}
