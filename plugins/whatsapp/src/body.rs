//! Waist-2 [`BodyHandler`] implementation (`WhatsAppBody`) translating all 10 native
//! WhatsApp capabilities into `WhatsAppTransport` operations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use monoagens_body_kit::{
    BargeInToken, BubbleSender, EchoRegistry, HonestCueThresholds, make_refused_receipt,
    make_terminal_receipt, deliver_split_bubbles,
};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, BodyShutdownParams, BodyShutdownResult, CURRENT_PROTOCOL_VERSION,
    CapabilityId, MetaValue, PerceptEmitParams, PerceptEmitResult, PlaceId, QuestionAnswerParams,
    QuestionAnswerResult, ReceiptOutcome, TerminationReason, Time, Waist2Error,
};
use serde_json::json;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{ChatStateType, Jid};

use crate::config::WhatsAppPluginConfig;
use crate::inbound::{
    CachedPoll, PendingApprovalBinding, PendingApprovalPromptMap, PollSecretCache,
};
use crate::transport::{OfflineWhatsAppTransport, PreparedMedia, PreparedSticker, WhatsAppTransport};
use crate::types::{
    ApprovalPromptRequest, CAP_APPROVAL_PROMPT, CAP_CHAT_DELETE, CAP_CHAT_EDIT,
    CAP_CHAT_MARK_READ, CAP_CHAT_REACT, CAP_CHAT_SEND_MEDIA, CAP_CHAT_SEND_POLL,
    CAP_CHAT_SEND_STICKER, CAP_CHAT_SPEAK, CAP_CHAT_TYPING, DEFAULT_BUBBLE_CAP,
    DEFAULT_SYSTEM_PLACE, DeleteRequest, EditRequest, MarkReadRequest, MediaAttachmentInput,
    ReactRequest, SendMediaRequest, SendPollRequest, SendStickerRequest, SpeakRequest,
    TypingRequest, bare_jid_string, build_whatsapp_capabilities, is_owned_whatsapp_place,
    normalize_declared_places, parse_user_jid, place_to_jid,
};

/// Resolves `blob_ref` handles into raw bytes for outbound `chat.send_media` / `chat.send_sticker`.
pub trait BlobLookup {
    fn lookup_blob(&self, blob_ref: &str) -> Result<Vec<u8>, String>;
}

/// Thread-safe in-memory blob lookup map (also usable with `PersistentBodyRuntime` blobs).
#[derive(Debug, Default, Clone)]
pub struct MemoryBlobLookup {
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemoryBlobLookup {
    /// Inserts raw bytes for `blob_ref`.
    pub fn insert(&self, blob_ref: impl Into<String>, bytes: Vec<u8>) {
        if let Ok(mut guard) = self.blobs.lock() {
            guard.insert(blob_ref.into(), bytes);
        }
    }
}

impl BlobLookup for MemoryBlobLookup {
    fn lookup_blob(&self, blob_ref: &str) -> Result<Vec<u8>, String> {
        let guard = self
            .blobs
            .lock()
            .map_err(|_| "blob store lock poisoned".to_owned())?;
        guard
            .get(blob_ref)
            .cloned()
            .ok_or_else(|| format!("blob '{blob_ref}' not found"))
    }
}

/// Waist-2 `body-whatsapp` handler wrapping a [`WhatsAppTransport`].
pub struct WhatsAppBody<T: WhatsAppTransport = OfflineWhatsAppTransport> {
    body_id: BodyId,
    declared_places: BTreeSet<PlaceId>,
    bubble_cap: usize,
    thresholds: HonestCueThresholds,
    config: WhatsAppPluginConfig,
    transport: T,
    echo_registry: EchoRegistry,
    poll_secrets: PollSecretCache,
    pending_approvals: PendingApprovalPromptMap,
    barge_in: BargeInToken,
    held_actions: HashSet<ActionId>,
    blob_lookup: Box<dyn BlobLookup + Send>,
    clock_now_ms: u64,
}

impl WhatsAppBody<OfflineWhatsAppTransport> {
    /// Creates an offline [`WhatsAppBody`] suitable for conformance checks and unit tests.
    #[must_use]
    pub fn offline(body_id: BodyId) -> Self {
        Self::new(body_id, OfflineWhatsAppTransport::new())
    }
}

impl<T: WhatsAppTransport> WhatsAppBody<T> {
    /// Creates a new [`WhatsAppBody`] over `transport`.
    #[must_use]
    pub fn new(body_id: BodyId, transport: T) -> Self {
        Self {
            body_id,
            declared_places: normalize_declared_places(std::iter::empty())
                .into_iter()
                .collect(),
            bubble_cap: DEFAULT_BUBBLE_CAP,
            thresholds: HonestCueThresholds::default(),
            config: WhatsAppPluginConfig::default(),
            transport,
            echo_registry: EchoRegistry::default(),
            poll_secrets: PollSecretCache::default(),
            pending_approvals: PendingApprovalPromptMap::default(),
            barge_in: BargeInToken::new(),
            held_actions: HashSet::new(),
            blob_lookup: Box::new(MemoryBlobLookup::default()),
            clock_now_ms: 0,
        }
    }

    /// Configures plugin settings loaded from `whatsapp.toml`.
    #[must_use]
    pub fn with_config(mut self, config: WhatsAppPluginConfig) -> Self {
        self.config = config;
        self
    }

    /// Configures a custom [`BlobLookup`] for resolving `blob_ref` attachments.
    #[must_use]
    pub fn with_blob_lookup(mut self, lookup: impl BlobLookup + Send + 'static) -> Self {
        self.blob_lookup = Box::new(lookup);
        self
    }

    /// Borrows the active [`WhatsAppPluginConfig`].
    #[must_use]
    pub fn config(&self) -> &WhatsAppPluginConfig {
        &self.config
    }

    /// Registers an owned WhatsApp place so `body/discover` lists it deterministically.
    pub fn register_place(&mut self, place: PlaceId) {
        if is_owned_whatsapp_place(&place) {
            self.declared_places.insert(place);
        }
    }

    /// Borrows the underlying transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutably borrows the underlying transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Borrows the [`EchoRegistry`] for `R-09` self-echo and reply-to correlation.
    #[must_use]
    pub fn echo_registry(&self) -> &EchoRegistry {
        &self.echo_registry
    }

    /// Borrows the [`PollSecretCache`] populated by `chat.send_poll`.
    #[must_use]
    pub fn poll_secrets(&self) -> &PollSecretCache {
        &self.poll_secrets
    }

    /// Borrows the [`PendingApprovalPromptMap`] tracking `approval.prompt` message bindings.
    #[must_use]
    pub fn pending_approvals(&self) -> &PendingApprovalPromptMap {
        &self.pending_approvals
    }

    /// Mutably borrows the [`PendingApprovalPromptMap`].
    pub fn pending_approvals_mut(&mut self) -> &mut PendingApprovalPromptMap {
        &mut self.pending_approvals
    }

    /// Splits mutable transport access from read-only echo & pending-approval state for
    /// inbound event translation without cloning maps.
    pub fn inbound_context_mut(
        &mut self,
    ) -> (&mut T, &EchoRegistry, &PendingApprovalPromptMap) {
        (&mut self.transport, &self.echo_registry, &self.pending_approvals)
    }

    /// Sets the local clock timestamp used in emitted receipts.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    /// Triggers mid-stream barge-in cancellation on any in-progress split-bubble send.
    pub fn trigger_barge_in(&self) {
        self.barge_in.trigger_external();
    }

    fn known_places(&self) -> Vec<PlaceId> {
        self.declared_places.iter().cloned().collect()
    }

    fn execute_speak(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req = match SpeakRequest::parse_payload(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
            }
        };
        let context = build_outbound_context(
            req.reply_to_message_id.as_deref(),
            req.reply_to_participant.as_deref(),
            &req.mentions,
        );

        if self.config.auto_typing_on_speak {
            let _ = self.transport.send_chatstate(chat, ChatStateType::Composing);
        }

        let mut sink = WhatsAppBubbleSink {
            transport: &mut self.transport,
            chat,
            context,
        };
        let mut outcome = deliver_split_bubbles(
            params.action_id.clone(),
            params.capability_id.clone(),
            &req.text,
            self.bubble_cap,
            &mut sink,
            &self.barge_in,
            &mut self.echo_registry,
            params.h_act.clone(),
            Time::from_millis(self.clock_now_ms),
        )
        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        if self.config.auto_typing_on_speak {
            let _ = self.transport.send_chatstate(chat, ChatStateType::Paused);
        }

        if self.barge_in.is_triggered() {
            self.barge_in = BargeInToken::new();
        }
        if let Some(first_pid) = outcome.delivered_pids.first() {
            outcome.receipt.detail = Some(
                json!({
                    "pids": outcome.delivered_pids,
                    "primary_pid": first_pid,
                    "place": params.places[0].as_str(),
                })
                .to_string(),
            );
        }
        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(outcome.receipt),
        })
    }

    fn execute_react(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: ReactRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.react payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        if req.message_id.trim().is_empty() {
            return Ok(refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "chat.react requires non-empty message_id",
                self.clock_now_ms,
            ));
        }
        let participant = match req.participant.as_deref() {
            Some(p) if !p.trim().is_empty() => match parse_user_jid(p) {
                Ok(j) => Some(j.to_string()),
                Err(err) => {
                    return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
                }
            },
            _ => None,
        };
        let target_key = wa::MessageKey {
            remote_jid: Some(chat.to_string()),
            from_me: req.from_me,
            id: Some(req.message_id.clone()),
            participant,
        };
        match self.transport.send_reaction(chat, target_key, &req.emoji) {
            Ok(pid) => {
                let ts = Time::from_millis(self.clock_now_ms);
                self.echo_registry
                    .register_echo(pid.clone(), params.action_id.clone(), 0, ts);
                Ok(done_result(
                    params,
                    1,
                    json!({
                        "pids": [pid],
                        "target_message_id": req.message_id,
                        "emoji": req.emoji,
                    })
                    .to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(params, "reaction_send_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_typing(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: TypingRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.typing payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        let state = match req.chat_state_type() {
            Ok(s) => s,
            Err(err) => {
                return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
            }
        };
        match self.transport.send_chatstate(chat, state) {
            Ok(()) => Ok(done_result(
                params,
                1,
                json!({ "state": state.as_str() }).to_string(),
                Time::from_millis(self.clock_now_ms),
            )),
            Err(err) => Ok(failed_result(params, "chatstate_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_send_media(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req = match SendMediaRequest::parse_payload(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
            }
        };

        let mut prepared = Vec::with_capacity(req.attachments.len());
        for item in &req.attachments {
            let bytes = match load_attachment_bytes(
                item.bytes_base64.as_deref(),
                item.blob_ref.as_deref(),
                item.path.as_deref(),
                self.blob_lookup.as_ref(),
            ) {
                Ok(b) => b,
                Err(err) => {
                    return Ok(refused_result(params, "ERR_INVALID_MEDIA", err, self.clock_now_ms));
                }
            };
            let ctx = build_outbound_context(
                item.reply_to_message_id.as_deref(),
                None,
                &item.mentions,
            )
            .map(Box::new);
            prepared.push(PreparedMedia {
                kind: item.kind,
                bytes,
                mime_type: item.mime_type.clone(),
                filename: item.filename.clone().or_else(|| default_filename(item)),
                caption: item.caption.clone(),
                ptt: item.ptt.unwrap_or(false),
                duration_seconds: item.duration_seconds,
                gif_playback: item.gif_playback,
                context_info: ctx,
            });
        }

        match self.transport.send_media(chat, &prepared) {
            Ok(pids) => {
                let ts = Time::from_millis(self.clock_now_ms);
                for (idx, pid) in pids.iter().enumerate() {
                    self.echo_registry
                        .register_echo(pid.clone(), params.action_id.clone(), idx, ts);
                }
                let units = pids.len() as u64;
                Ok(done_result(
                    params,
                    units,
                    json!({ "pids": pids, "count": units }).to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(params, "media_send_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_send_sticker(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: SendStickerRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.send_sticker payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        let bytes = match load_attachment_bytes(
            req.bytes_base64.as_deref(),
            req.blob_ref.as_deref(),
            req.path.as_deref(),
            self.blob_lookup.as_ref(),
        ) {
            Ok(b) => b,
            Err(err) => {
                return Ok(refused_result(params, "ERR_INVALID_MEDIA", err, self.clock_now_ms));
            }
        };
        let sticker = PreparedSticker {
            bytes,
            mime_type: req.mime_type,
            is_animated: req.is_animated,
            context_info: build_outbound_context(req.reply_to_message_id.as_deref(), None, &[])
                .map(Box::new),
        };
        match self.transport.send_sticker(chat, &sticker) {
            Ok(pid) => {
                let ts = Time::from_millis(self.clock_now_ms);
                self.echo_registry
                    .register_echo(pid.clone(), params.action_id.clone(), 0, ts);
                Ok(done_result(
                    params,
                    1,
                    json!({ "pids": [pid], "is_animated": sticker.is_animated }).to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(params, "sticker_send_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_send_poll(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: SendPollRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.send_poll payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        if req.question.trim().is_empty() || req.options.len() < 2 {
            return Ok(refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "chat.send_poll requires non-empty question and at least 2 options",
                self.clock_now_ms,
            ));
        }
        match self
            .transport
            .send_poll(chat, &req.question, &req.options, req.selectable_count)
        {
            Ok((pid, secret)) => {
                let ts = Time::from_millis(self.clock_now_ms);
                self.echo_registry
                    .register_echo(pid.clone(), params.action_id.clone(), 0, ts);
                self.poll_secrets.insert(
                    pid.clone(),
                    CachedPoll {
                        question: req.question.clone(),
                        options: req.options.clone(),
                        creator_jid: self.transport.own_jid().map(|j| j.to_non_ad()),
                        message_secret: secret,
                    },
                );
                Ok(done_result(
                    params,
                    1,
                    json!({ "pids": [pid], "question": req.question }).to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(params, "poll_send_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_mark_read(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req = match MarkReadRequest::parse_payload(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
            }
        };
        let sender = match req.sender.as_deref() {
            Some(s) if !s.trim().is_empty() => match parse_user_jid(s) {
                Ok(j) => Some(j),
                Err(err) => {
                    return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
                }
            },
            _ => None,
        };
        match self
            .transport
            .mark_read(chat, sender.as_ref(), &req.message_ids)
        {
            Ok(()) => Ok(done_result(
                params,
                req.message_ids.len() as u64,
                json!({ "marked_ids": req.message_ids }).to_string(),
                Time::from_millis(self.clock_now_ms),
            )),
            Err(err) => Ok(failed_result(params, "mark_read_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_edit(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: EditRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.edit payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        if req.message_id.trim().is_empty() {
            return Ok(refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "chat.edit requires non-empty message_id",
                self.clock_now_ms,
            ));
        }
        match self.transport.edit_message(chat, &req.message_id, &req.text) {
            Ok(pid) => {
                let ts = Time::from_millis(self.clock_now_ms);
                self.echo_registry
                    .register_echo(pid.clone(), params.action_id.clone(), 0, ts);
                Ok(done_result(
                    params,
                    1,
                    json!({ "pids": [pid], "edited_message_id": req.message_id }).to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(params, "edit_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_delete(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: DeleteRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid chat.delete payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        if req.message_id.trim().is_empty() {
            return Ok(refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "chat.delete requires non-empty message_id",
                self.clock_now_ms,
            ));
        }
        let sender = match req.sender.as_deref() {
            Some(s) if !s.trim().is_empty() => match parse_user_jid(s) {
                Ok(j) => Some(j),
                Err(err) => {
                    return Ok(refused_result(params, "ERR_INVALID_PAYLOAD", err, self.clock_now_ms));
                }
            },
            _ => None,
        };
        match self
            .transport
            .revoke_message(chat, &req.message_id, sender.as_ref())
        {
            Ok(()) => Ok(done_result(
                params,
                1,
                json!({ "revoked_message_id": req.message_id }).to_string(),
                Time::from_millis(self.clock_now_ms),
            )),
            Err(err) => Ok(failed_result(params, "delete_failed", err, 0, self.clock_now_ms)),
        }
    }

    fn execute_approval_prompt(
        &mut self,
        params: &ActionExecuteParams,
        chat: &Jid,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        let req: ApprovalPromptRequest = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(req) => req,
            Err(err) => {
                return Ok(refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid approval.prompt payload: {err}"),
                    self.clock_now_ms,
                ));
            }
        };
        if req.action_id.trim().is_empty()
            || req.h_act.trim().is_empty()
            || req.capability_id.trim().is_empty()
            || req.summary.trim().is_empty()
        {
            return Ok(refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "approval.prompt requires non-empty action_id, h_act, capability_id, and summary",
                self.clock_now_ms,
            ));
        }

        let approve_emojis = self
            .config
            .effective_approve_emojis(req.approve_emojis.as_deref());
        let deny_emojis = self
            .config
            .effective_deny_emojis(req.deny_emojis.as_deref());
        let prompt_text = render_approval_prompt_text(&req, &approve_emojis, &deny_emojis);

        match self.transport.send_text(chat, &prompt_text, None) {
            Ok(pid) => {
                let ts = Time::from_millis(self.clock_now_ms);
                self.echo_registry
                    .register_echo(pid.clone(), params.action_id.clone(), 0, ts);
                self.pending_approvals.insert(
                    pid.clone(),
                    PendingApprovalBinding {
                        action_id: req.action_id.clone(),
                        h_act: req.h_act.clone(),
                        state_epoch: req.state_epoch,
                        approve_emojis: approve_emojis.clone(),
                        deny_emojis: deny_emojis.clone(),
                        expires_at_ms: req.expires_at_ms,
                    },
                );
                Ok(done_result(
                    params,
                    1,
                    json!({
                        "pids": [pid],
                        "prompt_message_id": pid,
                        "action_id": req.action_id,
                        "h_act": req.h_act,
                        "state_epoch": req.state_epoch,
                        "approve_emojis": approve_emojis,
                        "deny_emojis": deny_emojis,
                    })
                    .to_string(),
                    ts,
                ))
            }
            Err(err) => Ok(failed_result(
                params,
                "approval_prompt_failed",
                err,
                0,
                self.clock_now_ms,
            )),
        }
    }
}

fn render_approval_prompt_text(
    req: &ApprovalPromptRequest,
    approve_emojis: &[String],
    deny_emojis: &[String],
) -> String {
    let mut lines = vec![
        "Approval required".to_owned(),
        format!("Action: {}", req.action_id),
        format!("Capability: {}", req.capability_id),
        format!("Summary: {}", req.summary),
    ];
    if !req.target_places.is_empty() {
        lines.push(format!("Places: {}", req.target_places.join(", ")));
    }
    lines.push(format!(
        "React {} to approve or {} to deny.",
        approve_emojis.join(" / "),
        deny_emojis.join(" / "),
    ));
    lines.join("\n")
}

struct WhatsAppBubbleSink<'a, T: WhatsAppTransport> {
    transport: &'a mut T,
    chat: &'a Jid,
    context: Option<wa::ContextInfo>,
}

impl<T: WhatsAppTransport> BubbleSender for WhatsAppBubbleSink<'_, T> {
    fn send_bubble(&mut self, chunk_index: usize, chunk_text: &str) -> Result<String, String> {
        let ctx = if chunk_index == 0 {
            self.context.take()
        } else {
            None
        };
        self.transport.send_text(self.chat, chunk_text, ctx)
    }
}

impl<T: WhatsAppTransport> BodyHandler for WhatsAppBody<T> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let places = self.known_places();
        let capabilities = build_whatsapp_capabilities(&places);
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities,
            places,
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        if !is_known_capability(params.capability_id.as_str()) {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(is_owned_whatsapp_place);
        if !place_ok {
            return Ok(refused_result(
                &params,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by body-whatsapp",
                self.clock_now_ms,
            ));
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting owner approval before WhatsApp dispatch".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let is_delivery_probe = params.payload.as_bytes() == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));

        let target_place = &params.places[0];
        if target_place.as_str() == DEFAULT_SYSTEM_PLACE {
            if is_delivery_probe {
                return Ok(done_result(
                    &params,
                    1,
                    json!({
                        "d_art_ms": self.thresholds.d_art_ms,
                        "working_cue_ms": self.thresholds.working_cue_ms,
                        "progress_note_ms": self.thresholds.progress_note_ms,
                        "place": DEFAULT_SYSTEM_PLACE,
                    })
                    .to_string(),
                    Time::from_millis(self.clock_now_ms),
                ));
            }
            return Ok(done_result(
                &params,
                1,
                json!({
                    "place": DEFAULT_SYSTEM_PLACE,
                    "capability": params.capability_id.as_str(),
                    "status": "system_probe_ok",
                })
                .to_string(),
                Time::from_millis(self.clock_now_ms),
            ));
        }

        let chat = match place_to_jid(target_place) {
            Ok(jid) => jid,
            Err(err) => {
                return Ok(refused_result(
                    &params,
                    "ERR_UNKNOWN_PLACE",
                    err,
                    self.clock_now_ms,
                ));
            }
        };

        let result = match params.capability_id.as_str() {
            CAP_APPROVAL_PROMPT => self.execute_approval_prompt(&params, &chat),
            CAP_CHAT_SPEAK => self.execute_speak(&params, &chat),
            CAP_CHAT_REACT => self.execute_react(&params, &chat),
            CAP_CHAT_TYPING => self.execute_typing(&params, &chat),
            CAP_CHAT_SEND_MEDIA => self.execute_send_media(&params, &chat),
            CAP_CHAT_SEND_STICKER => self.execute_send_sticker(&params, &chat),
            CAP_CHAT_SEND_POLL => self.execute_send_poll(&params, &chat),
            CAP_CHAT_MARK_READ => self.execute_mark_read(&params, &chat),
            CAP_CHAT_EDIT => self.execute_edit(&params, &chat),
            CAP_CHAT_DELETE => self.execute_delete(&params, &chat),
            _ => Err(Waist2Error::unknown_capability(&params.capability_id)),
        }?;

        if is_delivery_probe
            && let Some(mut receipt) = result.receipt
        {
            receipt.detail = Some(
                json!({
                    "d_art_ms": self.thresholds.d_art_ms,
                    "working_cue_ms": self.thresholds.working_cue_ms,
                    "progress_note_ms": self.thresholds.progress_note_ms,
                    "place": target_place.as_str(),
                })
                .to_string(),
            );
            return Ok(ActionExecuteResult {
                accepted: result.accepted,
                receipt: Some(receipt),
            });
        }

        Ok(result)
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.barge_in.trigger_core_cancel();
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new(CAP_CHAT_SPEAK),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        if self.held_actions.remove(&params.action_id) {
            return Ok(ApprovalResolveResult { applied: true });
        }
        Ok(ApprovalResolveResult { applied: false })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if !is_owned_whatsapp_place(&params.place) {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by body-whatsapp",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }

    fn on_question(&mut self, params: QuestionAnswerParams) -> Result<QuestionAnswerResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        Ok(QuestionAnswerResult { accepted: true })
    }

    fn on_shutdown(&mut self, _params: BodyShutdownParams) -> Result<BodyShutdownResult, Waist2Error> {
        Ok(BodyShutdownResult { accepted: true })
    }
}

fn is_known_capability(id: &str) -> bool {
    matches!(
        id,
        CAP_APPROVAL_PROMPT
            | CAP_CHAT_SPEAK
            | CAP_CHAT_REACT
            | CAP_CHAT_TYPING
            | CAP_CHAT_SEND_MEDIA
            | CAP_CHAT_SEND_STICKER
            | CAP_CHAT_SEND_POLL
            | CAP_CHAT_MARK_READ
            | CAP_CHAT_EDIT
            | CAP_CHAT_DELETE
    )
}

fn build_outbound_context(
    reply_to_message_id: Option<&str>,
    reply_to_participant: Option<&str>,
    mentions: &[String],
) -> Option<wa::ContextInfo> {
    let has_reply = reply_to_message_id.is_some_and(|s| !s.trim().is_empty());
    let normalized_mentions: Vec<String> = mentions
        .iter()
        .filter(|m| !m.trim().is_empty())
        .map(|m| parse_user_jid(m).map(|j| bare_jid_string(&j)).unwrap_or_else(|_| m.clone()))
        .collect();
    if !has_reply && normalized_mentions.is_empty() {
        return None;
    }
    Some(wa::ContextInfo {
        stanza_id: reply_to_message_id
            .filter(|s| !s.trim().is_empty())
            .map(ToOwned::to_owned),
        participant: reply_to_participant
            .filter(|s| !s.trim().is_empty())
            .map(|p| parse_user_jid(p).map(|j| bare_jid_string(&j)).unwrap_or_else(|_| p.to_owned())),
        mentioned_jid: normalized_mentions,
        ..Default::default()
    })
}

fn load_attachment_bytes(
    bytes_base64: Option<&str>,
    blob_ref: Option<&str>,
    path: Option<&str>,
    blob_lookup: &dyn BlobLookup,
) -> Result<Vec<u8>, String> {
    if let Some(b64) = bytes_base64 {
        return BASE64_STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("invalid bytes_base64: {e}"));
    }
    if let Some(bref) = blob_ref {
        return blob_lookup.lookup_blob(bref);
    }
    if let Some(file_path) = path {
        return fs::read(file_path)
            .map_err(|e| format!("failed to read attachment '{file_path}': {e}"));
    }
    Err("attachment requires one of 'bytes_base64', 'blob_ref', or 'path'".to_owned())
}

fn default_filename(item: &MediaAttachmentInput) -> Option<String> {
    item.path.as_deref().and_then(|p| {
        std::path::Path::new(p)
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToOwned::to_owned)
    })
}

fn refused_result(
    params: &ActionExecuteParams,
    code: &str,
    detail: impl Into<String>,
    now_ms: u64,
) -> ActionExecuteResult {
    let receipt = make_refused_receipt(
        params.action_id.clone(),
        params.capability_id.clone(),
        code,
        detail,
        params.h_act.clone(),
        Time::from_millis(now_ms),
    );
    ActionExecuteResult {
        accepted: false,
        receipt: Some(receipt),
    }
}

fn done_result(
    params: &ActionExecuteParams,
    delivered_units: u64,
    detail: String,
    timestamp_ms: Time,
) -> ActionExecuteResult {
    let receipt = make_terminal_receipt(
        params.action_id.clone(),
        params.capability_id.clone(),
        ReceiptOutcome::Done,
        None,
        Some(detail),
        Some(TerminationReason::Normal),
        Some(1.0),
        Some(delivered_units),
        params.h_act.clone(),
        timestamp_ms,
    );
    ActionExecuteResult {
        accepted: true,
        receipt: Some(receipt),
    }
}

fn failed_result(
    params: &ActionExecuteParams,
    reason_code: &str,
    detail: String,
    delivered_units: u64,
    now_ms: u64,
) -> ActionExecuteResult {
    let receipt = make_terminal_receipt(
        params.action_id.clone(),
        params.capability_id.clone(),
        ReceiptOutcome::Failed,
        Some(reason_code.to_owned()),
        Some(detail),
        Some(TerminationReason::Normal),
        Some(0.0),
        Some(delivered_units),
        params.h_act.clone(),
        Time::from_millis(now_ms),
    );
    ActionExecuteResult {
        accepted: true,
        receipt: Some(receipt),
    }
}
