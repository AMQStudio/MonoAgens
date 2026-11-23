#![forbid(unsafe_code)]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use body_whatsapp::{
    CAP_APPROVAL_PROMPT, CAP_CHAT_DELETE, CAP_CHAT_EDIT, CAP_CHAT_MARK_READ, CAP_CHAT_REACT,
    CAP_CHAT_SEND_MEDIA, CAP_CHAT_SEND_POLL, CAP_CHAT_SEND_STICKER, CAP_CHAT_SPEAK,
    CAP_CHAT_TYPING, DEFAULT_BUBBLE_CAP, DEFAULT_SYSTEM_PLACE, INLINE_ATTACHMENT_CAP_BYTES,
    InboundAction, InboundTranslator, MediaKind, MemoryBlobLookup, MemoryBlobStager,
    OfflineWhatsAppTransport, PendingApprovalBinding, PendingApprovalPromptMap,
    RecordedOutboundAction, WhatsAppBody, WhatsAppPluginConfig, default_body_id,
    default_system_place, jid_to_actor, jid_to_place,
};
use monoagens_body_kit::{EchoRegistry, PersistentBodyRuntime};
use monoagens_body_sdk::{
    ActionCancelParams, ActionExecuteParams, ActionExecuteResult, ActionId, ActorId,
    ApprovalResolveParams, Audience, BlobPutParams, BodyDiscoverParams, BodyHandler,
    CURRENT_PROTOCOL_VERSION, CapabilityId, HAct, MetaValue, ModalitySlot, OpaquePayload,
    PerceptEmitParams, PlaceId, ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, Taint, Time, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response, sha256_hex, sort_capabilities,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use whatsapp_rust::chrono::{TimeZone as _, Utc};
use whatsapp_rust::wacore::poll::{compute_option_hash, encrypt_poll_vote_with_secret};
use whatsapp_rust::wacore::types::events::InboundMessage;
use whatsapp_rust::wacore::types::message::{MessageInfo, MessageSource};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Jid, buffa};

fn exec_params(
    action_id: &str,
    cap: &str,
    place: &str,
    payload: Vec<u8>,
) -> ActionExecuteParams {
    ActionExecuteParams {
        headers: Waist2Headers::new(format!("idem-{action_id}")),
        action_id: ActionId::new(action_id),
        capability_id: CapabilityId::new(cap),
        places: vec![PlaceId::new(place)],
        audience: Audience::new([ActorId::new("wa:user:6281111111@s.whatsapp.net")]),
        payload: OpaquePayload::from_bytes(payload),
        timeout_ms: Some(5_000),
        h_act: None,
    }
}

fn inbound_msg(
    id: &str,
    chat: &str,
    sender: &str,
    is_group: bool,
    is_from_me: bool,
    ts_ms: i64,
    message: wa::Message,
) -> InboundMessage {
    let mut info = MessageInfo {
        id: id.to_owned(),
        timestamp: Utc.timestamp_millis_opt(ts_ms).unwrap(),
        ..Default::default()
    };
    info.source = MessageSource {
        chat: chat.parse().unwrap(),
        sender: sender.parse().unwrap(),
        is_group,
        is_from_me,
        ..Default::default()
    };
    InboundMessage::builder()
        .message(Arc::new(message))
        .info(Arc::new(info))
        .build()
}

#[test]
fn discover_returns_all_10_sorted_capabilities_with_hashes() {
    let mut body = WhatsAppBody::offline(default_body_id());
    let d1 = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("disc-1"),
            body_id: None,
        })
        .unwrap();
    let d2 = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("disc-2"),
            body_id: None,
        })
        .unwrap();

    assert_eq!(d1, d2);
    assert_eq!(d1.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(d1.places, vec![default_system_place()]);
    assert_eq!(d1.capabilities.len(), 10);

    let mut sorted = d1.capabilities.clone();
    sort_capabilities(&mut sorted);
    assert_eq!(d1.capabilities, sorted);

    let ids: Vec<&str> = d1.capabilities.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            CAP_APPROVAL_PROMPT,
            CAP_CHAT_DELETE,
            CAP_CHAT_EDIT,
            CAP_CHAT_MARK_READ,
            CAP_CHAT_REACT,
            CAP_CHAT_SEND_MEDIA,
            CAP_CHAT_SEND_POLL,
            CAP_CHAT_SEND_STICKER,
            CAP_CHAT_SPEAK,
            CAP_CHAT_TYPING,
        ]
    );
    for cap in &d1.capabilities {
        assert!(!cap.capability_schema_hash.as_str().is_empty());
    }
}

#[test]
fn chat_speak_emits_composing_paused_quote_mentions_and_splits_bubbles() {
    let mut body = WhatsAppBody::offline(default_body_id());
    body.set_now_ms(1_700_000_000_000);

    // 1. Structured JSON with quote + mentions
    let payload = json!({
        "text": "Hello @628999!",
        "reply_to_message_id": "orig-msg-1",
        "reply_to_participant": "wa:user:628999:3@s.whatsapp.net",
        "mentions": ["wa:user:628999:3@s.whatsapp.net"]
    });
    let res = body
        .execute(exec_params(
            "act-speak-1",
            CAP_CHAT_SPEAK,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&payload).unwrap(),
        ))
        .unwrap();
    assert!(res.accepted);
    let receipt = res.receipt.unwrap();
    assert_eq!(receipt.outcome, ReceiptOutcome::Done);
    assert_eq!(receipt.delivered_units, Some(1));

    assert_eq!(
        body.transport().recorded,
        vec![
            RecordedOutboundAction::Chatstate {
                chat: "1203630123456@g.us".to_owned(),
                state: "composing".to_owned(),
            },
            RecordedOutboundAction::Text {
                chat: "1203630123456@g.us".to_owned(),
                text: "Hello @628999!".to_owned(),
                reply_to_message_id: Some("orig-msg-1".to_owned()),
                mentions: vec!["628999@s.whatsapp.net".to_owned()],
            },
            RecordedOutboundAction::Chatstate {
                chat: "1203630123456@g.us".to_owned(),
                state: "paused".to_owned(),
            },
        ]
    );
    assert!(body.echo_registry().is_self_echo("wa-offline-msg-1"));

    // 2. Long text > 4096 splits into multiple bubbles
    let long_text = "word ".repeat(1_000); // 5000 bytes
    assert!(long_text.len() > DEFAULT_BUBBLE_CAP);
    let res2 = body
        .execute(exec_params(
            "act-speak-2",
            CAP_CHAT_SPEAK,
            "wa:dm:6281111111@s.whatsapp.net",
            long_text.into_bytes(),
        ))
        .unwrap();
    let r2 = res2.receipt.unwrap();
    assert_eq!(r2.outcome, ReceiptOutcome::Done);
    assert_eq!(r2.delivered_units, Some(2));
    assert!(body.echo_registry().is_self_echo("wa-offline-msg-2"));
    assert!(body.echo_registry().is_self_echo("wa-offline-msg-3"));
}

#[test]
fn chat_react_typing_mark_read_edit_delete_and_poll() {
    let mut body = WhatsAppBody::offline(default_body_id());

    // React add & remove
    let r1 = body
        .execute(exec_params(
            "act-react-add",
            CAP_CHAT_REACT,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({
                "message_id": "msg-target-1",
                "emoji": "🔥",
                "from_me": false
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(r1.receipt.unwrap().outcome, ReceiptOutcome::Done);

    let r2 = body
        .execute(exec_params(
            "act-react-rm",
            CAP_CHAT_REACT,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "message_id": "msg-target-2",
                "emoji": "",
                "participant": "wa:user:6282222222:1@s.whatsapp.net"
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(r2.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // Typing
    let t1 = body
        .execute(exec_params(
            "act-typing",
            CAP_CHAT_TYPING,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({ "state": "recording" })).unwrap(),
        ))
        .unwrap();
    assert_eq!(t1.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // Poll
    let p1 = body
        .execute(exec_params(
            "act-poll",
            CAP_CHAT_SEND_POLL,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "question": "Lunch?",
                "options": ["Sushi", "Ramen"],
                "selectable_count": 1
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(p1.receipt.unwrap().outcome, ReceiptOutcome::Done);
    assert!(body.poll_secrets().get("wa-offline-msg-3").is_some());

    // Mark read
    let mr = body
        .execute(exec_params(
            "act-read",
            CAP_CHAT_MARK_READ,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "message_ids": ["m1", "m2"],
                "sender": "wa:user:6282222222@s.whatsapp.net"
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(mr.receipt.unwrap().delivered_units, Some(2));

    // Edit
    let ed = body
        .execute(exec_params(
            "act-edit",
            CAP_CHAT_EDIT,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({
                "message_id": "wa-offline-msg-1",
                "text": "Edited hello!"
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(ed.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // Delete (admin revoke in group)
    let del = body
        .execute(exec_params(
            "act-del",
            CAP_CHAT_DELETE,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "message_id": "bad-msg-9",
                "sender": "wa:user:6282222222@s.whatsapp.net"
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(del.receipt.unwrap().outcome, ReceiptOutcome::Done);

    assert_eq!(
        body.transport().recorded,
        vec![
            RecordedOutboundAction::Reaction {
                chat: "6281111111@s.whatsapp.net".to_owned(),
                target_id: "msg-target-1".to_owned(),
                emoji: "🔥".to_owned(),
                from_me: Some(false),
                participant: None,
            },
            RecordedOutboundAction::Reaction {
                chat: "1203630123456@g.us".to_owned(),
                target_id: "msg-target-2".to_owned(),
                emoji: String::new(),
                from_me: None,
                participant: Some("6282222222@s.whatsapp.net".to_owned()),
            },
            RecordedOutboundAction::Chatstate {
                chat: "6281111111@s.whatsapp.net".to_owned(),
                state: "recording".to_owned(),
            },
            RecordedOutboundAction::Poll {
                chat: "1203630123456@g.us".to_owned(),
                question: "Lunch?".to_owned(),
                options: vec!["Sushi".to_owned(), "Ramen".to_owned()],
                selectable_count: 1,
            },
            RecordedOutboundAction::MarkRead {
                chat: "1203630123456@g.us".to_owned(),
                sender: Some("6282222222@s.whatsapp.net".to_owned()),
                message_ids: vec!["m1".to_owned(), "m2".to_owned()],
            },
            RecordedOutboundAction::Edit {
                chat: "6281111111@s.whatsapp.net".to_owned(),
                original_id: "wa-offline-msg-1".to_owned(),
                new_text: "Edited hello!".to_owned(),
            },
            RecordedOutboundAction::Revoke {
                chat: "1203630123456@g.us".to_owned(),
                message_id: "bad-msg-9".to_owned(),
                original_sender: Some("6282222222@s.whatsapp.net".to_owned()),
            },
        ]
    );
}

#[test]
fn chat_send_media_single_multi_and_sticker() {
    let lookup = MemoryBlobLookup::default();
    lookup.insert("blob://sha256/abc", b"blob-doc-bytes".to_vec());

    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.write_all(b"disk-voice-bytes").unwrap();
    tmp.flush().unwrap();

    let mut body = WhatsAppBody::offline(default_body_id()).with_blob_lookup(lookup);

    // 1. Single flat media attachment
    let single_res = body
        .execute(exec_params(
            "act-media-1",
            CAP_CHAT_SEND_MEDIA,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({
                "kind": "image",
                "bytes_base64": BASE64_STANDARD.encode(b"img-1"),
                "caption": "pic"
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(single_res.receipt.unwrap().delivered_units, Some(1));

    // 2. Multi-attachment batch mixing base64, blob_ref, and path
    let multi_res = body
        .execute(exec_params(
            "act-media-multi",
            CAP_CHAT_SEND_MEDIA,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "attachments": [
                    {
                        "kind": "video",
                        "bytes_base64": BASE64_STANDARD.encode(b"vid-bytes"),
                        "caption": "clip"
                    },
                    {
                        "kind": "document",
                        "blob_ref": "blob://sha256/abc",
                        "filename": "report.pdf",
                        "mime_type": "application/pdf"
                    },
                    {
                        "kind": "voice",
                        "path": tmp.path().to_str().unwrap(),
                        "duration_seconds": 4
                    }
                ]
            }))
            .unwrap(),
        ))
        .unwrap();
    let multi_rcpt = multi_res.receipt.unwrap();
    assert_eq!(multi_rcpt.outcome, ReceiptOutcome::Done);
    assert_eq!(multi_rcpt.delivered_units, Some(3));

    // 3. Sticker
    let stk_res = body
        .execute(exec_params(
            "act-sticker",
            CAP_CHAT_SEND_STICKER,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({
                "bytes_base64": BASE64_STANDARD.encode(b"RIFFWEBP"),
                "is_animated": true
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(stk_res.receipt.unwrap().delivered_units, Some(1));

    assert_eq!(
        body.transport().recorded,
        vec![
            RecordedOutboundAction::Media {
                chat: "6281111111@s.whatsapp.net".to_owned(),
                items: vec![(MediaKind::Image, 5, Some("pic".to_owned()), false)],
            },
            RecordedOutboundAction::Media {
                chat: "1203630123456@g.us".to_owned(),
                items: vec![
                    (MediaKind::Video, 9, Some("clip".to_owned()), false),
                    (MediaKind::Document, 14, None, false),
                    (MediaKind::Voice, 16, None, true),
                ],
            },
            RecordedOutboundAction::Sticker {
                chat: "6281111111@s.whatsapp.net".to_owned(),
                byte_len: 8,
                is_animated: true,
            },
        ]
    );
}

#[test]
fn inbound_filters_preboot_and_self_echo_and_normalizes_places_and_audiences() {
    let boot_ms = 1_700_000_100_000u64;
    let mut translator = InboundTranslator::new(boot_ms); // default 15s grace
    let mut transport = OfflineWhatsAppTransport::new();
    transport.group_participants.insert(
        "1203630123456@g.us".to_owned(),
        vec![
            "6281111111@s.whatsapp.net".parse().unwrap(),
            "6282222222@s.whatsapp.net".parse().unwrap(),
        ],
    );
    let mut echo = EchoRegistry::default();
    echo.register_echo(
        "echo-msg-1",
        ActionId::new("act-origin-99"),
        0,
        Time::from_millis(boot_ms),
    );
    let mut stager = MemoryBlobStager::default();

    // 1. Pre-boot historical message (boot - 30s) -> dropped
    let stale = inbound_msg(
        "old-1",
        "6281111111:12@s.whatsapp.net",
        "6281111111:12@s.whatsapp.net",
        false,
        false,
        (boot_ms - 30_000) as i64,
        wa::Message {
            conversation: Some("old history".to_owned()),
            ..Default::default()
        },
    );
    assert!(
        translator
            .translate_inbound(&stale, &mut transport, &echo, &mut stager, "k1")
            .is_none()
    );

    // 2. Self-sent (is_from_me) -> dropped
    let from_me = inbound_msg(
        "me-1",
        "6281111111@s.whatsapp.net",
        "6289999999@s.whatsapp.net",
        false,
        true,
        (boot_ms + 1_000) as i64,
        wa::Message {
            conversation: Some("my own echo".to_owned()),
            ..Default::default()
        },
    );
    assert!(
        translator
            .translate_inbound(&from_me, &mut transport, &echo, &mut stager, "k2")
            .is_none()
    );

    // 3. EchoRegistry hit -> dropped
    let echo_hit = inbound_msg(
        "echo-msg-1",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_000) as i64,
        wa::Message {
            conversation: Some("bot echo".to_owned()),
            ..Default::default()
        },
    );
    assert!(
        translator
            .translate_inbound(&echo_hit, &mut transport, &echo, &mut stager, "k3")
            .is_none()
    );

    // 4. Live DM with device suffix + quote-reply to our echo-msg-1 + LID mention
    let reply_msg = inbound_msg(
        "in-dm-1",
        "6281111111:12@s.whatsapp.net",
        "6281111111:12@s.whatsapp.net",
        false,
        false,
        (boot_ms + 2_000) as i64,
        wa::Message {
            extended_text_message: buffa::MessageField::some(wa::message::ExtendedTextMessage {
                text: Some("replying to you".to_owned()),
                context_info: buffa::MessageField::some(wa::ContextInfo {
                    stanza_id: Some("echo-msg-1".to_owned()),
                    mentioned_jid: vec!["99887766:2@lid".to_owned()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_dm = translator
        .translate_inbound(&reply_msg, &mut transport, &echo, &mut stager, "k4")
        .expect("live DM should translate");
    assert_eq!(p_dm.place.as_str(), "wa:dm:6281111111@s.whatsapp.net");
    assert_eq!(p_dm.actor.as_str(), "wa:user:6281111111@s.whatsapp.net");
    assert_eq!(p_dm.taint, Taint::Untrusted);
    assert_eq!(
        p_dm.correlation_id.as_ref().map(|c| c.as_str()),
        Some("act-origin-99")
    );
    assert_eq!(p_dm.named, vec![ActorId::new("wa:user:99887766@lid")]);
    assert!(!p_dm.audience.unwrap().is_empty());

    // 5. Group message populates non-empty audience from metadata cache + sender
    let grp_msg = inbound_msg(
        "in-grp-1",
        "1203630123456@g.us",
        "6283333333:4@s.whatsapp.net",
        true,
        false,
        (boot_ms + 3_000) as i64,
        wa::Message {
            conversation: Some("hi group".to_owned()),
            ..Default::default()
        },
    );
    let p_grp = translator
        .translate_inbound(&grp_msg, &mut transport, &echo, &mut stager, "k5")
        .expect("group msg should translate");
    assert_eq!(p_grp.place.as_str(), "wa:group:1203630123456@g.us");
    let aud = p_grp.audience.unwrap();
    assert!(aud.len() >= 3);
    assert!(aud.contains(&ActorId::new("wa:user:6281111111@s.whatsapp.net")));
    assert!(aud.contains(&ActorId::new("wa:user:6282222222@s.whatsapp.net")));
    assert!(aud.contains(&ActorId::new("wa:user:6283333333@s.whatsapp.net")));
}

#[test]
fn inbound_translates_media_staging_reactions_polls_locations_and_contacts() {
    let boot_ms = 1_700_000_200_000u64;
    let mut translator = InboundTranslator::new(boot_ms);
    let mut transport = OfflineWhatsAppTransport::new();
    let echo = EchoRegistry::default();
    let mut stager = MemoryBlobStager::default();

    // 1. Large image (> 256 KiB) stages out-of-band via blob/put AttachmentHandle
    let big_bytes = vec![0xABu8; INLINE_ATTACHMENT_CAP_BYTES + 1024];
    transport.mock_media_bytes = Some(big_bytes.clone());
    let big_img = inbound_msg(
        "img-big",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_000) as i64,
        wa::Message {
            image_message: buffa::MessageField::some(wa::message::ImageMessage {
                caption: Some("big photo".to_owned()),
                mimetype: Some("image/jpeg".to_owned()),
                file_length: Some(big_bytes.len() as u64),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_img = translator
        .translate_inbound(&big_img, &mut transport, &echo, &mut stager, "m1")
        .unwrap();
    assert_eq!(p_img.attachments.len(), 1);
    assert_eq!(p_img.attachments[0].modality_slot, ModalitySlot::Image);
    assert_eq!(p_img.attachments[0].byte_len, big_bytes.len() as u64);
    let img_json: Value = serde_json::from_slice(p_img.payload.as_bytes()).unwrap();
    assert_eq!(img_json["media"]["custody"], "blob_ref");
    assert!(img_json["media"].get("bytes_base64").is_none());

    // 2. Small animated sticker (<= 256 KiB) travels inline
    transport.mock_media_bytes = Some(b"webp-small".to_vec());
    let stk = inbound_msg(
        "stk-1",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_100) as i64,
        wa::Message {
            sticker_message: buffa::MessageField::some(wa::message::StickerMessage {
                mimetype: Some("image/webp".to_owned()),
                is_animated: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_stk = translator
        .translate_inbound(&stk, &mut transport, &echo, &mut stager, "m2")
        .unwrap();
    assert!(p_stk.attachments.is_empty());
    let stk_json: Value = serde_json::from_slice(p_stk.payload.as_bytes()).unwrap();
    assert_eq!(stk_json["type"], "sticker");
    assert_eq!(stk_json["is_animated"], true);
    assert_eq!(stk_json["media"]["custody"], "inline_base64");

    // 3. Reaction
    let react = inbound_msg(
        "rx-1",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_200) as i64,
        wa::Message {
            reaction_message: buffa::MessageField::some(wa::message::ReactionMessage {
                key: buffa::MessageField::some(wa::MessageKey {
                    id: Some("target-99".to_owned()),
                    ..Default::default()
                }),
                text: Some("❤️".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_rx = translator
        .translate_inbound(&react, &mut transport, &echo, &mut stager, "m3")
        .unwrap();
    let rx_json: Value = serde_json::from_slice(p_rx.payload.as_bytes()).unwrap();
    assert_eq!(rx_json["type"], "reaction");
    assert_eq!(rx_json["emoji"], "❤️");
    assert_eq!(rx_json["removed"], false);

    // 4. Poll creation + encrypted poll vote decryption
    let secret = vec![0x77u8; 32];
    let poll_create = inbound_msg(
        "poll-1",
        "1203630123456@g.us",
        "6285555555@s.whatsapp.net",
        true,
        false,
        (boot_ms + 1_300) as i64,
        wa::Message {
            poll_creation_message_v3: buffa::MessageField::some(wa::message::PollCreationMessage {
                name: Some("Color?".to_owned()),
                options: vec![
                    wa::message::poll_creation_message::Option {
                        option_name: Some("Red".to_owned()),
                        ..Default::default()
                    },
                    wa::message::poll_creation_message::Option {
                        option_name: Some("Blue".to_owned()),
                        ..Default::default()
                    },
                ],
                selectable_options_count: Some(1),
                ..Default::default()
            }),
            message_context_info: buffa::MessageField::some(wa::MessageContextInfo {
                message_secret: Some(secret.clone()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let _ = translator
        .translate_inbound(&poll_create, &mut transport, &echo, &mut stager, "m4")
        .unwrap();

    let selected_hash = compute_option_hash("Blue").to_vec();
    let (enc_payload, iv) = encrypt_poll_vote_with_secret(
        &[selected_hash],
        &secret,
        "poll-1",
        "6285555555@s.whatsapp.net",
        "6284444444@s.whatsapp.net",
    )
    .unwrap();
    let poll_vote = inbound_msg(
        "vote-1",
        "1203630123456@g.us",
        "6284444444@s.whatsapp.net",
        true,
        false,
        (boot_ms + 1_400) as i64,
        wa::Message {
            poll_update_message: buffa::MessageField::some(wa::message::PollUpdateMessage {
                poll_creation_message_key: buffa::MessageField::some(wa::MessageKey {
                    remote_jid: Some("1203630123456@g.us".to_owned()),
                    id: Some("poll-1".to_owned()),
                    participant: Some("6285555555@s.whatsapp.net".to_owned()),
                    ..Default::default()
                }),
                vote: buffa::MessageField::some(wa::message::PollEncValue {
                    enc_payload: Some(enc_payload),
                    enc_iv: Some(iv.to_vec()),
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_vote = translator
        .translate_inbound(&poll_vote, &mut transport, &echo, &mut stager, "m5")
        .unwrap();
    let vote_json: Value = serde_json::from_slice(p_vote.payload.as_bytes()).unwrap();
    assert_eq!(vote_json["type"], "poll_vote");
    assert_eq!(vote_json["selected_options"], json!(["Blue"]));

    // 5. Location & Contact
    let loc = inbound_msg(
        "loc-1",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_500) as i64,
        wa::Message {
            location_message: buffa::MessageField::some(wa::message::LocationMessage {
                degrees_latitude: Some(-6.2),
                degrees_longitude: Some(106.8),
                name: Some("Jakarta".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_loc = translator
        .translate_inbound(&loc, &mut transport, &echo, &mut stager, "m6")
        .unwrap();
    let loc_json: Value = serde_json::from_slice(p_loc.payload.as_bytes()).unwrap();
    assert_eq!(loc_json["type"], "location");
    assert_eq!(loc_json["name"], "Jakarta");

    let contact = inbound_msg(
        "cnt-1",
        "6281111111@s.whatsapp.net",
        "6281111111@s.whatsapp.net",
        false,
        false,
        (boot_ms + 1_600) as i64,
        wa::Message {
            contact_message: buffa::MessageField::some(wa::message::ContactMessage {
                display_name: Some("Alice".to_owned()),
                vcard: Some("BEGIN:VCARD\nFN:Alice\nEND:VCARD".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p_cnt = translator
        .translate_inbound(&contact, &mut transport, &echo, &mut stager, "m7")
        .unwrap();
    let cnt_json: Value = serde_json::from_slice(p_cnt.payload.as_bytes()).unwrap();
    assert_eq!(cnt_json["type"], "contact");
    assert_eq!(cnt_json["display_name"], "Alice");
}

#[test]
fn runtime_enforces_idempotency_crash_journal_isolation_errors_and_blobs() {
    let dir = TempDir::new().unwrap();
    let mut rt = PersistentBodyRuntime::new(Some(dir.path().to_path_buf()));
    let mut body = WhatsAppBody::offline(default_body_id());

    // 1. 3x idempotency_key replay -> single execution
    let req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        exec_params(
            "act-idem-1",
            CAP_CHAT_SPEAK,
            "wa:dm:6281111111@s.whatsapp.net",
            b"once".to_vec(),
        ),
    );
    let wire = serde_json::to_string(&req).unwrap();
    let r1 = rt.dispatch_line(&wire, &mut body).unwrap();
    let r2 = rt.dispatch_line(&wire, &mut body).unwrap();
    let r3 = rt.dispatch_line(&wire, &mut body).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
    assert_eq!(
        body.transport()
            .recorded
            .iter()
            .filter(|a| matches!(a, RecordedOutboundAction::Text { .. }))
            .count(),
        1
    );

    // 2. Restart recovers journal for state/query
    drop(rt);
    let mut rt2 = PersistentBodyRuntime::new(Some(dir.path().to_path_buf()));
    let mut body2 = WhatsAppBody::offline(default_body_id());
    let sq_req = Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("sq-1"),
            action_id: ActionId::new("act-idem-1"),
        },
    );
    let sq_resp: Waist2Response<StateQueryResult> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&sq_req).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        sq_resp.result.unwrap().status,
        StateQueryStatus::Terminal { .. }
    ));

    // 3. Unknown capability -> -32001 ERR_UNKNOWN_CAPABILITY
    let unk_req = Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::ActionExecute,
        exec_params("act-unk", "wa.nope", DEFAULT_SYSTEM_PLACE, b"x".to_vec()),
    );
    let unk_resp: Waist2Response<ActionExecuteResult> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&unk_req).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        unk_resp.error.unwrap().code,
        Waist2ErrorCode::UnknownCapability.code()
    );

    // 4. Empty audience -> -32006 ERR_UNDECLARED_AUDIENCE
    let mut empty_aud = exec_params("act-aud", CAP_CHAT_SPEAK, DEFAULT_SYSTEM_PLACE, b"x".to_vec());
    empty_aud.audience = Audience::empty();
    let aud_req = Waist2Request::new(Some(RpcId::Int(4)), Waist2Method::ActionExecute, empty_aud);
    let aud_resp: Waist2Response<ActionExecuteResult> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&aud_req).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        aud_resp.error.unwrap().code,
        Waist2ErrorCode::UndeclaredAudience.code()
    );

    // 5. Foreign place -> refused
    let foreign_req = Waist2Request::new(
        Some(RpcId::Int(5)),
        Waist2Method::ActionExecute,
        exec_params(
            "act-foreign",
            CAP_CHAT_SPEAK,
            "tck:foreign-place-999",
            b"x".to_vec(),
        ),
    );
    let foreign_resp: Waist2Response<ActionExecuteResult> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&foreign_req).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    let f_res = foreign_resp.result.unwrap();
    assert!(!f_res.accepted);
    assert_eq!(f_res.receipt.unwrap().outcome, ReceiptOutcome::Refused);

    // 6. Held action + approval/resolve
    let mut held_params = exec_params("act-held", CAP_CHAT_DELETE, DEFAULT_SYSTEM_PLACE, b"hold".to_vec());
    held_params.h_act = Some(HAct::new("blake3:hold-01"));
    let held_req = Waist2Request::new(Some(RpcId::Int(6)), Waist2Method::ActionExecute, held_params);
    let held_resp: Waist2Response<ActionExecuteResult> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&held_req).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        held_resp.result.unwrap().receipt.unwrap().outcome,
        ReceiptOutcome::Held
    );

    let approve_req = Waist2Request::new(
        Some(RpcId::Int(7)),
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("app-1"),
            action_id: ActionId::new("act-held"),
            h_act: HAct::new("blake3:hold-01"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: Time::from_millis(2_000),
            signature: None,
            out_of_band_channel: false,
        },
    );
    let app_wire = serde_json::to_string(&approve_req).unwrap();
    let app_resp = rt2.dispatch_line(&app_wire, &mut body2).unwrap();
    assert!(app_resp.contains("\"applied\":true"));

    // 7. Corrupt blob/put -> -32005 ERR_HASH_MISMATCH
    let bad_blob = Waist2Request::new(
        Some(RpcId::Int(8)),
        Waist2Method::BlobPut,
        BlobPutParams {
            headers: Waist2Headers::new("blob-bad"),
            sha256: sha256_hex(b"expected"),
            byte_len: 3,
            inline_bytes: Some(OpaquePayload::from_bytes(b"bad".as_slice())),
        },
    );
    let blob_resp: Waist2Response<Value> = serde_json::from_str(
        &rt2.dispatch_line(&serde_json::to_string(&bad_blob).unwrap(), &mut body2)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        blob_resp.error.unwrap().code,
        Waist2ErrorCode::HashMismatch.code()
    );

    // 8. Delivery timing probe + cancel + jid helpers
    let mut probe = exec_params(
        "act-probe",
        CAP_CHAT_DELETE,
        DEFAULT_SYSTEM_PLACE,
        b"delivery-timing-probe".to_vec(),
    );
    probe
        .headers
        .meta
        .insert("org.monoagens.delivery_probe".to_owned(), MetaValue::Bool(true));
    let probe_res = body2.execute(probe).unwrap();
    let detail: Value = serde_json::from_str(probe_res.receipt.unwrap().detail.as_deref().unwrap()).unwrap();
    assert_eq!(detail["d_art_ms"], 0);
    assert_eq!(detail["working_cue_ms"], 1_000);
    assert_eq!(detail["progress_note_ms"], 10_000);

    let cancel_res = body2
        .cancel(ActionCancelParams {
            headers: Waist2Headers::new("cancel-1"),
            action_id: ActionId::new("act-speak-cancel"),
            reason: Some("stop".to_owned()),
        })
        .unwrap();
    assert!(cancel_res.cancelled);

    let lid_jid: Jid = "123456:4@lid".parse().unwrap();
    assert_eq!(jid_to_place(&lid_jid).as_str(), "wa:dm:123456@lid");
    assert_eq!(jid_to_actor(&lid_jid).as_str(), "wa:user:123456@lid");

    // 9. Foreign percept refused
    let foreign_percept = PerceptEmitParams {
        headers: Waist2Headers::new("p-foreign"),
        actor: ActorId::new("wa:user:1@s.whatsapp.net"),
        origin_actor: None,
        place: PlaceId::new("tck:foreign"),
        audience: Some(Audience::new([ActorId::new("wa:user:1@s.whatsapp.net")])),
        timestamp_ms: Time::from_millis(1),
        payload: OpaquePayload::from_bytes(b"hi".as_slice()),
        taint: Taint::Untrusted,
        addressees: Vec::new(),
        named: Vec::new(),
        orient: Vec::new(),
        reply_to: None,
        correlation_id: None,
        channel: None,
        delta_obs: None,
        channel_prediction: None,
        length: None,
        negative_flag: None,
        repair: None,
        continued_attention: None,
        f_modal: None,
        embeddings: Vec::new(),
        attachments: Vec::new(),
        dialogue_act_gap: None,
        peer_expertise_margin: None,
    };
    assert!(body2.on_percept(foreign_percept).is_err());
}

#[test]
fn plugin_config_auto_generates_and_reloads_customized_emojis_and_state_dir() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("plugins").join("whatsapp.toml");
    let state_dir = dir.path().join("state");

    // 1. First run via load_or_init_at writes commented default whatsapp.toml
    assert!(!cfg_path.exists());
    let cfg1 = WhatsAppPluginConfig::load_or_init_at(&cfg_path).unwrap();
    assert!(cfg_path.exists());
    assert_eq!(cfg1, WhatsAppPluginConfig::default());
    assert_eq!(cfg1.version, 1);
    assert!(cfg1.auto_typing_on_speak);
    assert!(!cfg1.auto_mark_read);
    assert_eq!(cfg1.history_grace_secs, 15);
    assert_eq!(cfg1.approve_emojis, vec!["👍", "✅"]);
    assert_eq!(cfg1.deny_emojis, vec!["👎", "❌"]);

    // Generated TOML parses back cleanly as default
    let raw_generated = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(raw_generated.contains("approve_emojis"));
    assert_eq!(
        toml::from_str::<WhatsAppPluginConfig>(&raw_generated).unwrap(),
        WhatsAppPluginConfig::default()
    );

    // 2. Customize whatsapp.toml and reload on second run
    let customized = r#"
version = 1
auto_typing_on_speak = false
auto_mark_read = true
history_grace_secs = 30
approve_emojis = ["🚀", "🆗"]
deny_emojis = ["🛑", "🚫"]
"#;
    std::fs::write(&cfg_path, customized).unwrap();
    let cfg2 = WhatsAppPluginConfig::load_or_init_at(&cfg_path).unwrap();
    assert!(!cfg2.auto_typing_on_speak);
    assert!(cfg2.auto_mark_read);
    assert_eq!(cfg2.history_grace_secs, 30);
    assert_eq!(cfg2.approve_emojis, vec!["🚀", "🆗"]);
    assert_eq!(cfg2.deny_emojis, vec!["🛑", "🚫"]);

    // 3. End-to-end binary launch with MONOAGENS_PLUGIN_CONFIG & MONOAGENS_PLUGIN_STATE_DIR:
    //    - auto-generates config file on first run
    //    - honors MONOAGENS_PLUGIN_STATE_DIR for PersistentBodyRuntime state file
    //    - stdout remains strictly NDJSON Waist-2 frames only
    let bin_cfg_path = dir.path().join("env-run").join("whatsapp.toml");
    let discover_req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("env-disc-1"),
            body_id: None,
        },
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_body-whatsapp"))
        .env("MONOAGENS_PLUGIN_CONFIG", &bin_cfg_path)
        .env("MONOAGENS_PLUGIN_STATE_DIR", &state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{}", serde_json::to_string(&discover_req).unwrap()).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(bin_cfg_path.exists());
    assert!(state_dir.join("waist2_body_state.json").exists());

    let stdout_str = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout_str.lines().collect();
    assert_eq!(lines.len(), 1);
    let resp: Waist2Response<Value> = serde_json::from_str(lines[0]).unwrap();
    assert!(resp.error.is_none());
    assert_eq!(
        resp.result.unwrap()["capabilities"].as_array().unwrap().len(),
        10
    );
}

#[test]
fn approval_prompt_renders_instructions_and_binds_message_id() {
    let custom_cfg = WhatsAppPluginConfig {
        approve_emojis: vec!["🚀".to_owned(), "👌".to_owned()],
        deny_emojis: vec!["🛑".to_owned()],
        ..WhatsAppPluginConfig::default()
    };
    let mut body = WhatsAppBody::offline(default_body_id()).with_config(custom_cfg);
    body.set_now_ms(1_700_000_500_000);

    // 1. Default configured emojis (customized via WhatsAppPluginConfig)
    let payload = json!({
        "action_id": "act-approve-1",
        "h_act": "blake3:act-hash-1",
        "state_epoch": 7,
        "capability_id": "fs.write",
        "target_places": ["fs:/tmp/demo.txt"],
        "summary": "Write 42 bytes to /tmp/demo.txt",
        "expires_at_ms": 1_700_000_560_000u64
    });
    let res = body
        .execute(exec_params(
            "act-prompt-1",
            CAP_APPROVAL_PROMPT,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&payload).unwrap(),
        ))
        .unwrap();
    assert!(res.accepted);
    let receipt = res.receipt.unwrap();
    assert_eq!(receipt.outcome, ReceiptOutcome::Done);
    let detail: Value = serde_json::from_str(receipt.detail.as_deref().unwrap()).unwrap();
    let prompt_msg_id = detail["prompt_message_id"].as_str().unwrap().to_owned();
    assert_eq!(prompt_msg_id, "wa-offline-msg-1");
    assert!(body.echo_registry().is_self_echo(&prompt_msg_id));

    match &body.transport().recorded[0] {
        RecordedOutboundAction::Text { chat, text, .. } => {
            assert_eq!(chat, "1203630123456@g.us");
            assert!(text.contains("act-approve-1"));
            assert!(text.contains("fs.write"));
            assert!(text.contains("Write 42 bytes to /tmp/demo.txt"));
            assert!(text.contains("fs:/tmp/demo.txt"));
            assert!(text.contains("React 🚀 / 👌 to approve or 🛑 to deny."));
        }
        other => panic!("expected Text outbound action, got {other:?}"),
    }

    let binding = body
        .pending_approvals()
        .get(&prompt_msg_id)
        .expect("prompt message_id must be bound");
    assert_eq!(
        binding,
        &PendingApprovalBinding {
            action_id: "act-approve-1".to_owned(),
            h_act: "blake3:act-hash-1".to_owned(),
            state_epoch: 7,
            approve_emojis: vec!["🚀".to_owned(), "👌".to_owned()],
            deny_emojis: vec!["🛑".to_owned()],
            expires_at_ms: 1_700_000_560_000,
        }
    );

    // 2. Per-request emoji override
    let override_payload = json!({
        "action_id": "act-approve-2",
        "h_act": "blake3:act-hash-2",
        "state_epoch": 8,
        "capability_id": "shell.exec",
        "summary": "Run diagnostic script",
        "expires_at_ms": 1_700_000_590_000u64,
        "approve_emojis": ["👍", "✅"],
        "deny_emojis": ["👎", "❌"]
    });
    let res2 = body
        .execute(exec_params(
            "act-prompt-2",
            CAP_APPROVAL_PROMPT,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&override_payload).unwrap(),
        ))
        .unwrap();
    assert_eq!(res2.receipt.unwrap().outcome, ReceiptOutcome::Done);
    match &body.transport().recorded[1] {
        RecordedOutboundAction::Text { text, .. } => {
            assert!(text.contains("React 👍 / ✅ to approve or 👎 / ❌ to deny."));
        }
        other => panic!("expected Text outbound action, got {other:?}"),
    }

    // 3. Invalid payload refused
    let bad_res = body
        .execute(exec_params(
            "act-prompt-bad",
            CAP_APPROVAL_PROMPT,
            "wa:dm:6281111111@s.whatsapp.net",
            serde_json::to_vec(&json!({
                "action_id": "",
                "h_act": "blake3:x",
                "state_epoch": 1,
                "capability_id": "chat.speak",
                "summary": "empty action_id",
                "expires_at_ms": 100
            }))
            .unwrap(),
        ))
        .unwrap();
    assert!(!bad_res.accepted);
    assert_eq!(bad_res.receipt.unwrap().outcome, ReceiptOutcome::Refused);

    // 4. Bounded FIFO eviction on PendingApprovalPromptMap
    let mut bounded = PendingApprovalPromptMap::new(2);
    for idx in 1..=3 {
        bounded.insert(
            format!("msg-{idx}"),
            PendingApprovalBinding {
                action_id: format!("act-{idx}"),
                h_act: format!("h-{idx}"),
                state_epoch: idx,
                approve_emojis: vec!["👍".to_owned()],
                deny_emojis: vec!["👎".to_owned()],
                expires_at_ms: 999,
            },
        );
    }
    assert_eq!(bounded.len(), 2);
    assert!(bounded.get("msg-1").is_none());
    assert!(bounded.get("msg-2").is_some());
    assert!(bounded.get("msg-3").is_some());
}

#[test]
fn inbound_reaction_translates_bound_approval_prompts_and_preserves_normal_reactions() {
    let boot_ms = 1_700_000_600_000u64;
    let mut body = WhatsAppBody::offline(default_body_id());
    body.set_now_ms(boot_ms + 100);

    // Dispatch approval.prompt with default emojis (👍/✅ approve, 👎/❌ deny)
    let prompt_res = body
        .execute(exec_params(
            "act-prompt-rx",
            CAP_APPROVAL_PROMPT,
            "wa:group:1203630123456@g.us",
            serde_json::to_vec(&json!({
                "action_id": "act-held-42",
                "h_act": "blake3:held-42",
                "state_epoch": 12,
                "capability_id": "net.fetch",
                "summary": "Fetch external webhook",
                "expires_at_ms": boot_ms + 60_000
            }))
            .unwrap(),
        ))
        .unwrap();
    let detail: Value =
        serde_json::from_str(prompt_res.receipt.unwrap().detail.as_deref().unwrap()).unwrap();
    let prompt_msg_id = detail["prompt_message_id"].as_str().unwrap().to_owned();

    let mut translator = InboundTranslator::new(boot_ms);
    let mut stager = MemoryBlobStager::default();

    let make_reaction = |id: &str, sender: &str, target_id: &str, emoji: &str, ts_offset: u64| {
        inbound_msg(
            id,
            "1203630123456@g.us",
            sender,
            true,
            false,
            (boot_ms + ts_offset) as i64,
            wa::Message {
                reaction_message: buffa::MessageField::some(wa::message::ReactionMessage {
                    key: buffa::MessageField::some(wa::MessageKey {
                        id: Some(target_id.to_owned()),
                        ..Default::default()
                    }),
                    text: Some(emoji.to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
    };

    // 1. Approve with 👍 and ✅ (with device suffix normalized to bare JID)
    for (idx, emoji) in ["👍", "✅"].iter().enumerate() {
        let rx = make_reaction(
            &format!("rx-app-{idx}"),
            "6281111111:5@s.whatsapp.net",
            &prompt_msg_id,
            emoji,
            1_000 + idx as u64,
        );
        let (transport, echo_reg, pending) = body.inbound_context_mut();
        let action = translator
            .translate_inbound_action(
                &rx,
                transport,
                echo_reg,
                pending,
                &mut stager,
                format!("k-app-{idx}"),
            )
            .expect("approve reaction should translate");
        match action {
            InboundAction::ApprovalResolve(resolve) => {
                assert!(resolve.approved);
                assert_eq!(resolve.action_id, ActionId::new("act-held-42"));
                assert_eq!(resolve.h_act, HAct::new("blake3:held-42"));
                assert_eq!(resolve.state_epoch, 12);
                assert_eq!(
                    resolve.resolver,
                    ActorId::new("wa:user:6281111111@s.whatsapp.net")
                );
                assert_eq!(
                    resolve.resolved_at_ms,
                    Time::from_millis(boot_ms + 1_000 + idx as u64)
                );
            }
            other => panic!("expected ApprovalResolve, got {other:?}"),
        }
    }

    // 2. Deny with 👎 and ❌
    for (idx, emoji) in ["👎", "❌"].iter().enumerate() {
        let rx = make_reaction(
            &format!("rx-deny-{idx}"),
            "6282222222:2@s.whatsapp.net",
            &prompt_msg_id,
            emoji,
            2_000 + idx as u64,
        );
        let (transport, echo_reg, pending) = body.inbound_context_mut();
        let action = translator
            .translate_inbound_action(
                &rx,
                transport,
                echo_reg,
                pending,
                &mut stager,
                format!("k-deny-{idx}"),
            )
            .expect("deny reaction should translate");
        match action {
            InboundAction::ApprovalResolve(resolve) => {
                assert!(!resolve.approved);
                assert_eq!(resolve.action_id, ActionId::new("act-held-42"));
                assert_eq!(resolve.h_act, HAct::new("blake3:held-42"));
                assert_eq!(resolve.state_epoch, 12);
                assert_eq!(
                    resolve.resolver,
                    ActorId::new("wa:user:6282222222@s.whatsapp.net")
                );
            }
            other => panic!("expected ApprovalResolve, got {other:?}"),
        }
    }

    // 3. Unrelated emoji on approval prompt -> normal percept/emit
    let unrelated = make_reaction(
        "rx-other",
        "6281111111@s.whatsapp.net",
        &prompt_msg_id,
        "😂",
        3_000,
    );
    let (transport, echo_reg, pending) = body.inbound_context_mut();
    match translator
        .translate_inbound_action(
            &unrelated,
            transport,
            echo_reg,
            pending,
            &mut stager,
            "k-other",
        )
        .unwrap()
    {
        InboundAction::PerceptEmit(percept) => {
            let p_json: Value = serde_json::from_slice(percept.payload.as_bytes()).unwrap();
            assert_eq!(p_json["type"], "reaction");
            assert_eq!(p_json["emoji"], "😂");
        }
        other => panic!("expected PerceptEmit, got {other:?}"),
    }

    // 4. Empty emoji (reaction removal) on approval prompt -> normal percept/emit
    let removed = make_reaction(
        "rx-rm",
        "6281111111@s.whatsapp.net",
        &prompt_msg_id,
        "",
        3_100,
    );
    let (transport, echo_reg, pending) = body.inbound_context_mut();
    match translator
        .translate_inbound_action(
            &removed,
            transport,
            echo_reg,
            pending,
            &mut stager,
            "k-rm",
        )
        .unwrap()
    {
        InboundAction::PerceptEmit(percept) => {
            let p_json: Value = serde_json::from_slice(percept.payload.as_bytes()).unwrap();
            assert_eq!(p_json["type"], "reaction");
            assert_eq!(p_json["removed"], true);
        }
        other => panic!("expected PerceptEmit, got {other:?}"),
    }

    // 5. Reaction with 👍 on a non-approval message -> normal percept/emit
    let normal = make_reaction(
        "rx-normal",
        "6281111111@s.whatsapp.net",
        "some-other-message-id",
        "👍",
        3_200,
    );
    let (transport, echo_reg, pending) = body.inbound_context_mut();
    match translator
        .translate_inbound_action(
            &normal,
            transport,
            echo_reg,
            pending,
            &mut stager,
            "k-normal",
        )
        .unwrap()
    {
        InboundAction::PerceptEmit(percept) => {
            let p_json: Value = serde_json::from_slice(percept.payload.as_bytes()).unwrap();
            assert_eq!(p_json["type"], "reaction");
            assert_eq!(p_json["emoji"], "👍");
            assert_eq!(p_json["target_message_id"], "some-other-message-id");
        }
        other => panic!("expected PerceptEmit, got {other:?}"),
    }
}

fn lid_inbound(chat: &str, sender: &str, alt: Option<&str>, is_group: bool, ts_ms: i64) -> InboundMessage {
    let mut info = MessageInfo {
        id: format!("lid-{ts_ms}"),
        timestamp: Utc.timestamp_millis_opt(ts_ms).unwrap(),
        ..Default::default()
    };
    info.source = MessageSource {
        chat: chat.parse().unwrap(),
        sender: sender.parse().unwrap(),
        sender_alt: alt.map(|a| a.parse().unwrap()),
        is_group,
        ..Default::default()
    };
    InboundMessage::builder()
        .message(Arc::new(wa::Message {
            conversation: Some("hello".to_owned()),
            ..Default::default()
        }))
        .info(Arc::new(info))
        .build()
}

#[test]
fn lid_dm_with_alternate_sender_uses_phone_account_and_reply_reaches_chat() {
    let boot_ms = 1_700_000_100_000u64;
    let mut translator = InboundTranslator::new(boot_ms);
    let mut transport = OfflineWhatsAppTransport::new();
    let echo = EchoRegistry::default();
    let mut stager = MemoryBlobStager::default();
    let msg = lid_inbound(
        "130799689031758@lid",
        "130799689031758@lid",
        Some("6285111242960@s.whatsapp.net"),
        false,
        (boot_ms + 1_000) as i64,
    );
    let p = translator
        .translate_inbound(&msg, &mut transport, &echo, &mut stager, "k")
        .expect("translates");
    assert_eq!(p.actor.as_str(), "wa:user:6285111242960@s.whatsapp.net");
    assert_eq!(p.place.as_str(), "wa:dm:6285111242960@s.whatsapp.net");
    assert!(
        p.audience
            .unwrap()
            .contains(&ActorId::new("wa:user:6285111242960@s.whatsapp.net"))
    );

    // Outbound chat.speak to that place goes to the phone-number chat.
    let mut body = WhatsAppBody::offline(default_body_id());
    body.set_now_ms(1_700_000_000_000);
    let res = body
        .execute(exec_params(
            "act-lid-reply",
            CAP_CHAT_SPEAK,
            p.place.as_str(),
            b"pong".to_vec(),
        ))
        .unwrap();
    assert!(res.accepted);
    assert!(body.transport().recorded.iter().any(|a| matches!(
        a,
        RecordedOutboundAction::Text { chat, text, .. }
            if chat == "6285111242960@s.whatsapp.net" && text == "pong"
    )));
}

#[test]
fn lid_dm_without_alternate_uses_stored_mapping_else_lid() {
    let boot_ms = 1_700_000_100_000u64;
    let mut translator = InboundTranslator::new(boot_ms);
    let mut transport = OfflineWhatsAppTransport::new();
    transport
        .lid_to_phone
        .insert("130799689031758".to_owned(), "6285111242960".to_owned());
    let echo = EchoRegistry::default();
    let mut stager = MemoryBlobStager::default();
    let mapped = lid_inbound(
        "130799689031758@lid",
        "130799689031758@lid",
        None,
        false,
        (boot_ms + 1_000) as i64,
    );
    let p = translator
        .translate_inbound(&mapped, &mut transport, &echo, &mut stager, "k1")
        .unwrap();
    assert_eq!(p.actor.as_str(), "wa:user:6285111242960@s.whatsapp.net");
    assert_eq!(p.place.as_str(), "wa:dm:6285111242960@s.whatsapp.net");

    let unknown = lid_inbound(
        "999000111222@lid",
        "999000111222@lid",
        None,
        false,
        (boot_ms + 2_000) as i64,
    );
    let p = translator
        .translate_inbound(&unknown, &mut transport, &echo, &mut stager, "k2")
        .unwrap();
    assert_eq!(p.actor.as_str(), "wa:user:999000111222@lid");
    assert_eq!(p.place.as_str(), "wa:dm:999000111222@lid");
}

#[test]
fn lid_group_sender_and_participants_use_phone_accounts() {
    let boot_ms = 1_700_000_100_000u64;
    let mut translator = InboundTranslator::new(boot_ms);
    let mut transport = OfflineWhatsAppTransport::new();
    transport.group_participants.insert(
        "1203630123456@g.us".to_owned(),
        vec!["777888999@lid".parse().unwrap()],
    );
    transport
        .lid_to_phone
        .insert("777888999".to_owned(), "6282222222".to_owned());
    let echo = EchoRegistry::default();
    let mut stager = MemoryBlobStager::default();
    let msg = lid_inbound(
        "1203630123456@g.us",
        "130799689031758@lid",
        Some("6285111242960@s.whatsapp.net"),
        true,
        (boot_ms + 1_000) as i64,
    );
    let p = translator
        .translate_inbound(&msg, &mut transport, &echo, &mut stager, "k")
        .unwrap();
    assert_eq!(p.place.as_str(), "wa:group:1203630123456@g.us");
    assert_eq!(p.actor.as_str(), "wa:user:6285111242960@s.whatsapp.net");
    let aud = p.audience.unwrap();
    assert!(aud.contains(&ActorId::new("wa:user:6285111242960@s.whatsapp.net")));
    assert!(aud.contains(&ActorId::new("wa:user:6282222222@s.whatsapp.net")));
    assert!(!aud.contains(&ActorId::new("wa:user:130799689031758@lid")));
}

#[test]
fn status_json_reports_needs_pairing_without_a_session_and_opens_no_network() {
    let dir = TempDir::new().unwrap();
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_body-whatsapp"))
        .env("MONOAGENS_PLUGIN_STATE_DIR", &state_dir)
        .env_remove("MONOAGENS_PLUGIN_CONFIG")
        .args(["status", "--json"])
        .output()
        .expect("run status");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    assert_eq!(v["ready"], json!(false));
    assert_eq!(v["needs"], json!(["pairing"]));
    // The command must not have created a session database.
    assert!(!state_dir.join("session.db").exists());
}
