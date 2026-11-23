use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use body_telegram::{
    APPROVAL_CALLBACK_PREFIX, BlobResolver, LiveTelegramTransport, MediaSource, MemoryBlobStager,
    OutboundTelegramEvent, PollKind, RecordedCall, RecordingTelegramTransport, SendPollPayload,
    TELEGRAM_TEXT_CAP, TelegramBody, TelegramPluginConfig, TelegramTransport, TgCallbackQuery,
    TgChat, TgChatAction, TgChatMember, TgDocument, TgMessage, TgMessageReactionUpdated,
    TgPhotoSize, TgReactionType, TgSticker, TgUpdate, TgUser, TgVoice, extract_receipt_pids,
};
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionId, ActorId, Audience, BodyDiscoverParams, BodyHandler, BodyId,
    CapabilityId, HAct, MAX_INLINE_ATTACHMENT_BYTES, OpaquePayload, PerceptEmitParams, PlaceId,
    ReceiptOutcome, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
    sort_capabilities,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct MapBlobResolver(HashMap<String, Vec<u8>>);

impl BlobResolver for MapBlobResolver {
    fn fetch_blob(&self, blob_ref: &str) -> Result<Vec<u8>, String> {
        self.0
            .get(blob_ref)
            .cloned()
            .ok_or_else(|| format!("missing blob '{blob_ref}'"))
    }
}

fn make_exec(
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
        audience: Audience::new([ActorId::new("tg:user:42")]),
        payload: OpaquePayload::from_bytes(payload),
        timeout_ms: Some(5_000),
        h_act: None,
    }
}

#[test]
fn discover_lists_sorted_native_capabilities_with_hashes() {
    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    );
    let res = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover succeeds");

    let ids: Vec<&str> = res.capabilities.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "approval.prompt",
            "chat.delete",
            "chat.edit",
            "chat.react",
            "chat.send_buttons",
            "chat.send_media",
            "chat.send_poll",
            "chat.send_sticker",
            "chat.speak",
            "chat.typing",
        ]
    );

    let mut expected_sorted = res.capabilities.clone();
    sort_capabilities(&mut expected_sorted);
    assert_eq!(res.capabilities, expected_sorted);
    assert!(
        res.capabilities
            .iter()
            .all(|c| c.capability_schema_hash.as_str().starts_with("sha256:"))
    );
}

#[test]
fn chat_speak_split_bubble_typing_buttons_and_echo_registration() {
    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    );

    let long_text = format!("{} {}", "A".repeat(3000), "B".repeat(2000));
    assert!(long_text.len() > TELEGRAM_TEXT_CAP);

    let payload = json!({
        "text": long_text,
        "parse_mode": "HTML",
        "reply_to_message_id": 55,
        "buttons": [[{ "text": "Details", "callback_data": "details:1" }]]
    });

    let res = body
        .execute(make_exec(
            "act-speak-1",
            "chat.speak",
            "tg:group:-100123:topic:77",
            payload.to_string().into_bytes(),
        ))
        .expect("execute succeeds");

    assert!(res.accepted);
    let receipt = res.receipt.expect("receipt present");
    assert_eq!(receipt.outcome, ReceiptOutcome::Done);
    assert_eq!(receipt.delivered_units, Some(2));
    let pids = extract_receipt_pids(&receipt);
    assert_eq!(pids.len(), 2);
    assert!(body.echo_registry().is_self_echo(&pids[0]));
    assert!(body.echo_registry().is_self_echo(&pids[1]));

    let calls = body.transport().snapshot_calls();
    assert!(matches!(
        calls[0],
        RecordedCall::SendChatAction {
            chat_id: -100123,
            thread_id: Some(77),
            action: TgChatAction::Typing
        }
    ));
    match &calls[1] {
        RecordedCall::SendMessage {
            chat_id,
            thread_id,
            parse_mode,
            reply_to_message_id,
            buttons,
            ..
        } => {
            assert_eq!(*chat_id, -100123);
            assert_eq!(*thread_id, Some(77));
            assert_eq!(parse_mode.as_deref(), Some("HTML"));
            assert_eq!(*reply_to_message_id, Some(55));
            assert!(buttons.is_none(), "buttons should attach only to final bubble");
        }
        other => panic!("expected SendMessage, got {other:?}"),
    }
    match &calls[2] {
        RecordedCall::SendMessage {
            reply_to_message_id,
            buttons,
            ..
        } => {
            assert!(reply_to_message_id.is_none());
            assert!(buttons.is_some(), "final bubble carries inline buttons");
        }
        other => panic!("expected SendMessage, got {other:?}"),
    }
}

#[test]
fn chat_send_buttons_react_typing_edit_delete_poll_sticker() {
    let mut resolver_map = HashMap::new();
    resolver_map.insert("blob://sha256/sticker123".to_owned(), b"WEBP_BYTES".to_vec());
    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    )
    .with_blob_resolver(Arc::new(MapBlobResolver(resolver_map)));

    // 1. chat.send_buttons
    let btn_res = body
        .execute(make_exec(
            "act-btn",
            "chat.send_buttons",
            "tg:dm:42",
            json!({
                "text": "Choose:",
                "buttons": [[
                    { "text": "Approve", "callback_data": "approve:1" },
                    { "text": "Docs", "url": "https://example.org" }
                ]],
                "reply_to_message_id": 10
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    assert_eq!(btn_res.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 2. chat.react (set + clear)
    let react_set = body
        .execute(make_exec(
            "act-react-1",
            "chat.react",
            "tg:dm:42",
            json!({ "message_id": 10, "emoji": "🔥", "is_big": true })
                .to_string()
                .into_bytes(),
        ))
        .unwrap();
    assert_eq!(react_set.receipt.unwrap().outcome, ReceiptOutcome::Done);

    let react_clear = body
        .execute(make_exec(
            "act-react-2",
            "chat.react",
            "tg:dm:42",
            json!({ "message_id": 10, "emoji": "" })
                .to_string()
                .into_bytes(),
        ))
        .unwrap();
    assert_eq!(react_clear.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 3. chat.typing
    let typing = body
        .execute(make_exec(
            "act-typing",
            "chat.typing",
            "tg:dm:42",
            json!({ "action": "record_voice" }).to_string().into_bytes(),
        ))
        .unwrap();
    assert_eq!(typing.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 4. chat.send_sticker via blob_ref
    let sticker = body
        .execute(make_exec(
            "act-sticker",
            "chat.send_sticker",
            "tg:dm:42",
            json!({ "blob_ref": "blob://sha256/sticker123", "emoji": "😎" })
                .to_string()
                .into_bytes(),
        ))
        .unwrap();
    assert_eq!(sticker.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 5. chat.send_poll (quiz)
    let poll = body
        .execute(make_exec(
            "act-poll",
            "chat.send_poll",
            "tg:group:-500",
            json!({
                "question": "2+2?",
                "options": ["3", "4", "5"],
                "kind": "quiz",
                "correct_option_id": 1,
                "is_anonymous": false
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    assert_eq!(poll.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 6. chat.edit
    let edit = body
        .execute(make_exec(
            "act-edit",
            "chat.edit",
            "tg:dm:42",
            json!({
                "message_id": 1001,
                "text": "Updated!",
                "parse_mode": "MarkdownV2"
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    assert_eq!(edit.receipt.unwrap().outcome, ReceiptOutcome::Done);

    // 7. chat.delete
    let del = body
        .execute(make_exec(
            "act-del",
            "chat.delete",
            "tg:dm:42",
            json!({ "message_id": 1001 }).to_string().into_bytes(),
        ))
        .unwrap();
    assert_eq!(del.receipt.unwrap().outcome, ReceiptOutcome::Done);

    let calls = body.transport().snapshot_calls();
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SetMessageReaction {
            message_id: 10,
            emoji: Some(e),
            is_big: true,
            ..
        } if e == "🔥"
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SetMessageReaction {
            message_id: 10,
            emoji: None,
            ..
        }
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SendChatAction {
            action: TgChatAction::RecordVoice,
            ..
        }
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SendSticker {
            source: MediaSource::Bytes { data, .. },
            emoji: Some(em),
            ..
        } if data == b"WEBP_BYTES" && em == "😎"
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SendPoll {
            chat_id: -500,
            poll: SendPollPayload {
                kind: PollKind::Quiz,
                correct_option_id: Some(1),
                ..
            },
            ..
        }
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::EditMessageText {
            message_id: 1001,
            text,
            ..
        } if text == "Updated!"
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::DeleteMessage {
            chat_id: 42,
            message_id: 1001,
        }
    )));
}

#[test]
fn chat_send_media_single_and_multi_media_group() {
    let mut resolver_map = HashMap::new();
    resolver_map.insert("blob://sha256/pic1".to_owned(), b"PNGDATA".to_vec());

    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    )
    .with_blob_resolver(Arc::new(MapBlobResolver(resolver_map)));

    // Single voice note -> send_single_media
    let single = body
        .execute(make_exec(
            "act-media-1",
            "chat.send_media",
            "tg:dm:42",
            json!({
                "attachments": [{
                    "kind": "voice",
                    "bytes_base64": BASE64_STANDARD.encode(b"OGGVOICE"),
                    "filename": "note.ogg"
                }]
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    let r1 = single.receipt.unwrap();
    assert_eq!(r1.outcome, ReceiptOutcome::Done);
    assert_eq!(r1.delivered_units, Some(1));

    // Multi-attachment (3 items) -> send_media_group
    let album = body
        .execute(make_exec(
            "act-media-group",
            "chat.send_media",
            "tg:group:-777",
            json!({
                "caption": "Album caption",
                "attachments": [
                    { "kind": "photo", "blob_ref": "blob://sha256/pic1" },
                    { "kind": "photo", "url": "https://example.org/p2.jpg" },
                    { "kind": "video", "file_id": "VID_FILE_ID_3" }
                ]
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    let r2 = album.receipt.unwrap();
    assert_eq!(r2.outcome, ReceiptOutcome::Done);
    assert_eq!(r2.delivered_units, Some(3));
    let detail: Value = serde_json::from_str(r2.detail.as_deref().unwrap()).unwrap();
    assert_eq!(detail["media_group"], json!(true));
    assert_eq!(extract_receipt_pids(&r2).len(), 3);

    let calls = body.transport().snapshot_calls();
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SendSingleMedia { .. }
    )));
    assert!(calls.iter().any(|c| matches!(
        c,
        RecordedCall::SendMediaGroup {
            chat_id: -777,
            items,
            ..
        } if items.len() == 3 && items[0].caption.as_deref() == Some("Album caption")
    )));
}

#[test]
fn errors_unknown_capability_empty_audience_and_unowned_place() {
    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    );

    // -32001 unknown capability
    let err = body
        .execute(make_exec("a1", "chat.nonexistent", "tg:dm:42", b"{}".to_vec()))
        .unwrap_err();
    assert_eq!(err.code, Waist2ErrorCode::UnknownCapability.code());

    // -32006 empty audience
    let mut empty_aud = make_exec("a2", "chat.speak", "tg:dm:42", b"hi".to_vec());
    empty_aud.audience = Audience::default();
    let err_aud = body.execute(empty_aud).unwrap_err();
    assert_eq!(err_aud.code, Waist2ErrorCode::UndeclaredAudience.code());

    // Unowned place -> Refused receipt
    let refused = body
        .execute(make_exec(
            "a3",
            "chat.speak",
            "wa:dm:12345",
            b"hi".to_vec(),
        ))
        .unwrap();
    assert!(!refused.accepted);
    assert_eq!(refused.receipt.unwrap().outcome, ReceiptOutcome::Refused);

    // on_percept rejects empty audience & foreign place
    let foreign_percept = PerceptEmitParams {
        headers: Waist2Headers::new("p1"),
        actor: ActorId::new("tg:user:1"),
        origin_actor: None,
        place: PlaceId::new("cli:terminal"),
        audience: Some(Audience::new([ActorId::new("tg:user:1")])),
        timestamp_ms: monoagens_body_sdk::Time::from_millis(100),
        payload: OpaquePayload::from_bytes(b"hi".to_vec()),
        taint: monoagens_body_sdk::Taint::Untrusted,
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
    assert!(body.on_percept(foreign_percept).is_err());
}

#[test]
fn inbound_updates_translate_text_sticker_media_callback_reaction_and_echo_filter() {
    let mut transport = RecordingTelegramTransport::new(777_000, "mono_tg_bot");
    transport.files.insert("small_photo".to_owned(), b"JPEG_SMALL".to_vec());
    transport.files.insert(
        "big_doc".to_owned(),
        vec![0xAB; MAX_INLINE_ATTACHMENT_BYTES + 1024],
    );
    transport.admins.insert(
        -100999,
        vec![TgChatMember {
            status: "administrator".to_owned(),
            user: TgUser {
                id: 88,
                is_bot: false,
                first_name: "AdminAlice".to_owned(),
                last_name: None,
                username: Some("alice".to_owned()),
            },
        }],
    );

    let mut body = TelegramBody::new(BodyId::new("body-telegram"), transport);
    body.init_bot_identity().unwrap();
    let mut stager = MemoryBlobStager::default();

    // Send a message so EchoRegistry knows message_id 1001 came from act-bot-1
    let sent = body
        .execute(make_exec(
            "act-bot-1",
            "chat.send_buttons",
            "tg:group:-100999:topic:12",
            json!({
                "text": "Tap me",
                "buttons": [[{ "text": "Go", "callback_data": "go:42" }]]
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    let bot_msg_id: i64 = extract_receipt_pids(&sent.receipt.unwrap())[0]
        .parse()
        .unwrap();

    // 1. Self-echo from bot's own user_id -> suppressed
    let self_upd = TgUpdate {
        update_id: 1,
        message: Some(TgMessage {
            message_id: 5000,
            message_thread_id: None,
            from: Some(TgUser {
                id: 777_000,
                is_bot: true,
                first_name: "TestBot".to_owned(),
                last_name: None,
                username: Some("mono_tg_bot".to_owned()),
            }),
            sender_chat: None,
            date: 1_700_000_000,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_owned(),
                title: None,
                username: None,
                is_forum: None,
            },
            reply_to_message: None,
            edit_date: None,
            media_group_id: None,
            text: Some("echo".to_owned()),
            caption: None,
            photo: None,
            document: None,
            video: None,
            audio: None,
            voice: None,
            sticker: None,
            new_chat_members: None,
            left_chat_member: None,
        }),
        edited_message: None,
        callback_query: None,
        message_reaction: None,
    };
    assert!(body.translate_update(&self_upd, Some(&mut stager)).unwrap().is_none());

    // 2. Group forum topic message with big document (>256 KiB -> blob/put) + small photo + sticker + @mention
    let rich_upd = TgUpdate {
        update_id: 2,
        message: Some(TgMessage {
            message_id: 2002,
            message_thread_id: Some(12),
            from: Some(TgUser {
                id: 42,
                is_bot: false,
                first_name: "Bob".to_owned(),
                last_name: None,
                username: Some("bob".to_owned()),
            }),
            sender_chat: None,
            date: 1_700_000_010,
            chat: TgChat {
                id: -100999,
                chat_type: "supergroup".to_owned(),
                title: Some("Devs".to_owned()),
                username: None,
                is_forum: Some(true),
            },
            reply_to_message: None,
            edit_date: None,
            media_group_id: Some("mg-99".to_owned()),
            text: Some("Hey @mono_tg_bot look".to_owned()),
            caption: None,
            photo: Some(vec![TgPhotoSize {
                file_id: "small_photo".to_owned(),
                file_unique_id: "u1".to_owned(),
                width: 100,
                height: 100,
                file_size: Some(10),
            }]),
            document: Some(TgDocument {
                file_id: "big_doc".to_owned(),
                file_unique_id: "u2".to_owned(),
                file_name: Some("large.bin".to_owned()),
                mime_type: Some("application/octet-stream".to_owned()),
                file_size: Some((MAX_INLINE_ATTACHMENT_BYTES + 1024) as u64),
            }),
            video: None,
            audio: None,
            voice: Some(TgVoice {
                file_id: "small_photo".to_owned(),
                file_unique_id: "u3".to_owned(),
                duration: 3,
                mime_type: Some("audio/ogg".to_owned()),
                file_size: Some(10),
            }),
            sticker: Some(TgSticker {
                file_id: "small_photo".to_owned(),
                file_unique_id: "u4".to_owned(),
                width: 512,
                height: 512,
                is_animated: false,
                is_video: false,
                emoji: Some("🚀".to_owned()),
                set_name: Some("SpacePack".to_owned()),
                file_size: Some(10),
            }),
            new_chat_members: None,
            left_chat_member: None,
        }),
        edited_message: None,
        callback_query: None,
        message_reaction: None,
    };
    let percept = body
        .translate_update(&rich_upd, Some(&mut stager))
        .unwrap()
        .expect("percept emitted");
    assert_eq!(percept.place.as_str(), "tg:group:-100999:topic:12");
    assert_eq!(percept.actor.as_str(), "tg:user:42");
    let aud = percept.audience.as_ref().unwrap();
    assert!(aud.members.contains(&ActorId::new("tg:user:42")));
    assert!(aud.members.contains(&ActorId::new("tg:user:88")));
    assert!(aud.members.contains(&ActorId::new("tg:bot:777000")));
    assert!(!percept.addressees.is_empty());
    assert_eq!(stager.staged.len(), 1);
    assert_eq!(percept.attachments.len(), 1);
    assert!(percept.attachments[0].handle_id.starts_with("blob://staged/"));
    let payload_json: Value = serde_json::from_slice(percept.payload.as_bytes()).unwrap();
    assert_eq!(payload_json["media_group_id"], json!("mg-99"));
    assert_eq!(payload_json["sticker"]["emoji"], json!("🚀"));
    assert_eq!(payload_json["sticker"]["set_name"], json!("SpacePack"));

    // 3. callback_query auto-answers and correlates back to act-bot-1
    let cb_upd = TgUpdate {
        update_id: 3,
        message: None,
        edited_message: None,
        callback_query: Some(TgCallbackQuery {
            id: "cbq-77".to_owned(),
            from: TgUser {
                id: 42,
                is_bot: false,
                first_name: "Bob".to_owned(),
                last_name: None,
                username: None,
            },
            message: Some(TgMessage {
                message_id: bot_msg_id,
                message_thread_id: Some(12),
                from: None,
                sender_chat: None,
                date: 1_700_000_020,
                chat: TgChat {
                    id: -100999,
                    chat_type: "supergroup".to_owned(),
                    title: None,
                    username: None,
                    is_forum: Some(true),
                },
                reply_to_message: None,
                edit_date: None,
                media_group_id: None,
                text: None,
                caption: None,
                photo: None,
                document: None,
                video: None,
                audio: None,
                voice: None,
                sticker: None,
                new_chat_members: None,
                left_chat_member: None,
            }),
            inline_message_id: None,
            chat_instance: Some("inst-1".to_owned()),
            data: Some("go:42".to_owned()),
        }),
        message_reaction: None,
    };
    let cb_percept = body
        .translate_update(&cb_upd, None)
        .unwrap()
        .expect("callback percept");
    assert_eq!(
        cb_percept.correlation_id.as_ref().map(|c| c.as_str()),
        Some("act-bot-1")
    );
    let cb_json: Value = serde_json::from_slice(cb_percept.payload.as_bytes()).unwrap();
    assert_eq!(cb_json["callback_data"], json!("go:42"));
    assert!(body.transport().snapshot_calls().iter().any(|c| matches!(
        c,
        RecordedCall::AnswerCallbackQuery {
            callback_query_id,
            ..
        } if callback_query_id == "cbq-77"
    )));

    // 4. message_reaction
    let react_upd = TgUpdate {
        update_id: 4,
        message: None,
        edited_message: None,
        callback_query: None,
        message_reaction: Some(TgMessageReactionUpdated {
            chat: TgChat {
                id: 42,
                chat_type: "private".to_owned(),
                title: None,
                username: None,
                is_forum: None,
            },
            message_id: bot_msg_id,
            user: Some(TgUser {
                id: 42,
                is_bot: false,
                first_name: "Bob".to_owned(),
                last_name: None,
                username: None,
            }),
            actor_chat: None,
            date: 1_700_000_030,
            old_reaction: vec![TgReactionType::Emoji {
                emoji: "👀".to_owned(),
            }],
            new_reaction: vec![TgReactionType::Emoji {
                emoji: "🎉".to_owned(),
            }],
        }),
    };
    let react_percept = body
        .translate_update(&react_upd, None)
        .unwrap()
        .expect("reaction percept");
    assert_eq!(react_percept.place.as_str(), "tg:dm:42");
    let react_json: Value = serde_json::from_slice(react_percept.payload.as_bytes()).unwrap();
    assert_eq!(react_json["added"], json!(["🎉"]));
    assert_eq!(react_json["removed"], json!(["👀"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_transport_http_mock_server_exercises_all_endpoints() {
    #[derive(Clone, Debug)]
    struct CapturedReq {
        path: String,
        body_text: String,
    }

    let captured: Arc<Mutex<Vec<CapturedReq>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured_srv = Arc::clone(&captured);

    let server_task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let cap = Arc::clone(&captured_srv);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    let n = match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none()
                        && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                    {
                        header_end = Some(pos + 4);
                        let headers = String::from_utf8_lossy(&buf[..pos]);
                        for line in headers.lines() {
                            if let Some((k, v)) = line.split_once(':')
                                && k.trim().eq_ignore_ascii_case("content-length")
                            {
                                content_len = v.trim().parse::<usize>().unwrap_or(0);
                            }
                        }
                    }
                    if let Some(hend) = header_end
                        && buf.len() >= hend + content_len
                    {
                        break;
                    }
                }

                let hend = header_end.unwrap();
                let head = String::from_utf8_lossy(&buf[..hend]);
                let first_line = head.lines().next().unwrap_or("");
                let path = first_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_owned();
                let body_bytes = &buf[hend..hend + content_len];
                let body_text = String::from_utf8_lossy(body_bytes).into_owned();
                cap.lock().unwrap().push(CapturedReq {
                    path: path.clone(),
                    body_text,
                });

                let (status, content_type, resp_body): (&str, &str, Vec<u8>) =
                    if path.ends_with("/getMe") {
                        (
                            "200 OK",
                            "application/json",
                            json!({
                                "ok": true,
                                "result": { "id": 555, "is_bot": true, "first_name": "MockBot", "username": "mock_bot" }
                            })
                            .to_string()
                            .into_bytes(),
                        )
                    } else if path.ends_with("/getUpdates") {
                        (
                            "200 OK",
                            "application/json",
                            json!({
                                "ok": true,
                                "result": [{
                                    "update_id": 9001,
                                    "message": {
                                        "message_id": 11,
                                        "date": 1700000000,
                                        "chat": { "id": 42, "type": "private" },
                                        "from": { "id": 42, "is_bot": false, "first_name": "Alice" },
                                        "text": "hello live mock"
                                    }
                                }]
                            })
                            .to_string()
                            .into_bytes(),
                        )
                    } else if path.ends_with("/sendMediaGroup") {
                        (
                            "200 OK",
                            "application/json",
                            json!({
                                "ok": true,
                                "result": [
                                    { "message_id": 201, "date": 1700000001, "chat": { "id": 42, "type": "private" } },
                                    { "message_id": 202, "date": 1700000001, "chat": { "id": 42, "type": "private" } }
                                ]
                            })
                            .to_string()
                            .into_bytes(),
                        )
                    } else if path.ends_with("/sendMessage")
                        || path.ends_with("/sendPhoto")
                        || path.ends_with("/sendSticker")
                        || path.ends_with("/sendPoll")
                    {
                        (
                            "200 OK",
                            "application/json",
                            json!({
                                "ok": true,
                                "result": { "message_id": 300, "date": 1700000002, "chat": { "id": 42, "type": "private" } }
                            })
                            .to_string()
                            .into_bytes(),
                        )
                    } else if path.ends_with("/getFile") {
                        (
                            "200 OK",
                            "application/json",
                            json!({
                                "ok": true,
                                "result": { "file_id": "f1", "file_unique_id": "uf1", "file_path": "docs/a.txt" }
                            })
                            .to_string()
                            .into_bytes(),
                        )
                    } else if path.ends_with("/docs/a.txt") {
                        ("200 OK", "application/octet-stream", b"FILE_CONTENT".to_vec())
                    } else if path.ends_with("/getChatAdministrators") {
                        (
                            "200 OK",
                            "application/json",
                            json!({ "ok": true, "result": [] }).to_string().into_bytes(),
                        )
                    } else {
                        (
                            "200 OK",
                            "application/json",
                            json!({ "ok": true, "result": true }).to_string().into_bytes(),
                        )
                    };

                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    resp_body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.write_all(&resp_body).await;
            });
        }
    });

    let api_base = format!("http://{addr}");
    tokio::task::spawn_blocking(move || {
        let mut transport = LiveTelegramTransport::new(api_base, "test-token-123").unwrap();
        let me = transport.get_me().unwrap();
        assert_eq!(me.id, 555);

        let updates = transport.get_updates(None, 0).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 9001);

        let file_bytes = transport.download_file("f1").unwrap();
        assert_eq!(file_bytes, b"FILE_CONTENT");

        // Exercise full TelegramBody + PersistentBodyRuntime dispatch against live mock HTTP
        let mut runtime = PersistentBodyRuntime::new(None);
        let mut body = TelegramBody::new(BodyId::new("body-telegram"), transport);
        body.init_bot_identity().unwrap();

        let speak_req = Waist2Request::new(
            Some(monoagens_body_sdk::RpcId::Int(1)),
            Waist2Method::ActionExecute,
            make_exec(
                "live-speak",
                "chat.speak",
                "tg:dm:42",
                json!({
                    "text": "Hello live!",
                    "buttons": [[{ "text": "OK", "callback_data": "ok" }]]
                })
                .to_string()
                .into_bytes(),
            ),
        );
        let line = serde_json::to_string(&speak_req).unwrap();
        let reply_line = runtime.dispatch_line(&line, &mut body).unwrap();
        let resp: Waist2Response<monoagens_body_sdk::ActionExecuteResult> =
            serde_json::from_str(&reply_line).unwrap();
        assert!(resp.result.unwrap().accepted);

        // Multipart sendMediaGroup with raw bytes + URL
        let media_req = Waist2Request::new(
            Some(monoagens_body_sdk::RpcId::Int(2)),
            Waist2Method::ActionExecute,
            make_exec(
                "live-album",
                "chat.send_media",
                "tg:dm:42",
                json!({
                    "attachments": [
                        {
                            "kind": "photo",
                            "bytes_base64": BASE64_STANDARD.encode(b"IMG1"),
                            "filename": "p1.jpg"
                        },
                        {
                            "kind": "photo",
                            "url": "https://example.org/p2.jpg"
                        }
                    ]
                })
                .to_string()
                .into_bytes(),
            ),
        );
        let media_reply = runtime
            .dispatch_line(&serde_json::to_string(&media_req).unwrap(), &mut body)
            .unwrap();
        let media_resp: Waist2Response<monoagens_body_sdk::ActionExecuteResult> =
            serde_json::from_str(&media_reply).unwrap();
        assert_eq!(media_resp.result.unwrap().receipt.unwrap().delivered_units, Some(2));

        // Additional native endpoints
        let t = body.transport_mut();
        t.set_message_reaction(42, 300, Some("👍"), false).unwrap();
        t.send_sticker(42, None, &MediaSource::FileId("stk1".to_owned()), None, None)
            .unwrap();
        t.send_poll(
            42,
            None,
            &SendPollPayload {
                question: "Pick?".to_owned(),
                options: vec!["A".to_owned(), "B".to_owned()],
                is_anonymous: Some(true),
                kind: PollKind::Regular,
                allows_multiple_answers: Some(false),
                correct_option_id: None,
                reply_to_message_id: None,
            },
        )
        .unwrap();
        t.answer_callback_query("cb-1", Some("Ack"), false).unwrap();
        t.edit_message_text(42, 300, "Edited", None, None).unwrap();
        t.delete_message(42, 300).unwrap();
    })
    .await
    .unwrap();

    server_task.abort();

    let reqs = captured.lock().unwrap();
    let paths: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
    for expected in [
        "/bottest-token-123/getMe",
        "/bottest-token-123/getUpdates",
        "/bottest-token-123/sendChatAction",
        "/bottest-token-123/sendMessage",
        "/bottest-token-123/sendMediaGroup",
        "/bottest-token-123/setMessageReaction",
        "/bottest-token-123/sendSticker",
        "/bottest-token-123/sendPoll",
        "/bottest-token-123/answerCallbackQuery",
        "/bottest-token-123/editMessageText",
        "/bottest-token-123/deleteMessage",
        "/bottest-token-123/getFile",
        "/file/bottest-token-123/docs/a.txt",
    ] {
        assert!(
            paths.contains(&expected),
            "missing expected HTTP call {expected}; got {paths:?}"
        );
    }

    let send_msg_req = reqs
        .iter()
        .find(|r| r.path.ends_with("/sendMessage"))
        .unwrap();
    assert!(send_msg_req.body_text.contains("inline_keyboard"));

    let album_req = reqs
        .iter()
        .find(|r| r.path.ends_with("/sendMediaGroup"))
        .unwrap();
    assert!(album_req.body_text.contains("attach://file_0"));
}

#[test]
fn plugin_config_auto_generates_and_reloads_custom_labels() {
    let temp = tempfile::tempdir().unwrap();
    let cfg_path = temp.path().join("plugins").join("telegram.toml");
    assert!(!cfg_path.exists());

    // First run: creates parent directories and writes commented default telegram.toml
    let first = TelegramPluginConfig::load_or_init_from_path(&cfg_path).unwrap();
    assert_eq!(first, TelegramPluginConfig::default());
    assert!(cfg_path.exists());
    let written = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(written.contains("version = 1"));
    assert!(written.contains("approve_button_label = \"✅ Approve\""));
    assert!(written.contains("deny_button_label = \"❌ Deny\""));

    // Customize telegram.toml and reload on second run
    let custom_toml = r#"
# Custom Telegram plugin configuration
version = 1
default_parse_mode = "HTML"
poll_timeout_secs = 30
auto_typing_on_speak = false
approve_button_label = "Ship It"
deny_button_label = "Abort Action"
"#;
    std::fs::write(&cfg_path, custom_toml).unwrap();

    let second = TelegramPluginConfig::load_or_init_from_path(&cfg_path).unwrap();
    assert_eq!(second.version, 1);
    assert_eq!(second.default_parse_mode.as_deref(), Some("HTML"));
    assert_eq!(second.poll_timeout_secs, 30);
    assert!(!second.auto_typing_on_speak);
    assert_eq!(second.approve_button_label, "Ship It");
    assert_eq!(second.deny_button_label, "Abort Action");

    // Verify configured defaults take effect in TelegramBody (default_parse_mode + auto_typing_on_speak)
    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    )
    .with_config(second);

    let speak = body
        .execute(make_exec(
            "act-cfg-speak",
            "chat.speak",
            "tg:dm:42",
            json!({ "text": "<b>Configured</b>" }).to_string().into_bytes(),
        ))
        .unwrap();
    assert_eq!(speak.receipt.unwrap().outcome, ReceiptOutcome::Done);

    let calls = body.transport().snapshot_calls();
    assert_eq!(calls.len(), 1, "auto_typing_on_speak=false skips sendChatAction");
    assert!(matches!(
        &calls[0],
        RecordedCall::SendMessage { parse_mode: Some(pm), .. } if pm == "HTML"
    ));
}

#[test]
fn binary_auto_initializes_plugin_config_and_state_dir_from_env() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let temp = tempfile::tempdir().unwrap();
    let cfg_path = temp.path().join("cfg").join("plugins").join("telegram.toml");
    let state_dir = temp.path().join("state").join("telegram");

    let discover_req = serde_json::to_string(&Waist2Request::new(
        Some(monoagens_body_sdk::RpcId::Int(1)),
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("d-env"),
            body_id: None,
        },
    ))
    .unwrap();
    let shutdown_req = serde_json::to_string(&Waist2Request::new(
        Some(monoagens_body_sdk::RpcId::Int(2)),
        Waist2Method::BodyShutdown,
        monoagens_body_sdk::BodyShutdownParams {
            headers: Waist2Headers::new("s-env"),
            body_id: BodyId::new("body-telegram"),
            reason: None,
        },
    ))
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_body-telegram"))
        .arg("--offline")
        .env("MONOAGENS_PLUGIN_CONFIG", &cfg_path)
        .env("MONOAGENS_PLUGIN_STATE_DIR", &state_dir)
        .env_remove("MONOAGENS_BODY_STATE_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{discover_req}").unwrap();
        writeln!(stdin, "{shutdown_req}").unwrap();
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(cfg_path.exists(), "MONOAGENS_PLUGIN_CONFIG should be auto-generated");
    assert!(
        state_dir.join("waist2_body_state.json").exists(),
        "MONOAGENS_PLUGIN_STATE_DIR should store PersistentBodyRuntime state"
    );

    // Every line on stdout must be valid JSON-RPC (no logs/config notices on stdout)
    let stdout_str = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout_str.lines().collect();
    assert_eq!(lines.len(), 2);
    let discover_resp: Waist2Response<monoagens_body_sdk::BodyDiscoverResult> =
        serde_json::from_str(lines[0]).unwrap();
    assert!(
        discover_resp
            .result
            .unwrap()
            .capabilities
            .iter()
            .any(|c| c.id.as_str() == "approval.prompt")
    );
}

#[test]
fn approval_prompt_renders_inline_buttons_and_resolves_via_callback_query() {
    let cfg = TelegramPluginConfig {
        approve_button_label: "Yes, Proceed".to_owned(),
        deny_button_label: "No, Stop".to_owned(),
        ..Default::default()
    };

    let mut body = TelegramBody::new(
        BodyId::new("body-telegram"),
        RecordingTelegramTransport::default(),
    )
    .with_config(cfg);
    body.set_now_ms(1_700_000_500_000);

    // 1. Render generic approval.prompt for an arbitrary held action
    let prompt_res = body
        .execute(make_exec(
            "act-prompt-1",
            "approval.prompt",
            "tg:dm:42",
            json!({
                "action_id": "held-act-99",
                "h_act": "sha256:deadbeef",
                "state_epoch": 7,
                "capability_id": "fs.write",
                "target_places": ["fs:/tmp/demo.txt"],
                "summary": "Write 12 bytes to /tmp/demo.txt",
                "expires_at_ms": 1_700_000_999_000u64
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    assert!(prompt_res.accepted);
    let prompt_receipt = prompt_res.receipt.unwrap();
    assert_eq!(prompt_receipt.outcome, ReceiptOutcome::Done);
    let prompt_msg_id: i64 = extract_receipt_pids(&prompt_receipt)[0].parse().unwrap();

    let binding = body
        .pending_approval("held-act-99")
        .expect("binding recorded");
    assert_eq!(binding.h_act, "sha256:deadbeef");
    assert_eq!(binding.state_epoch, 7);
    assert_eq!(binding.chat_id, 42);
    assert_eq!(binding.message_id, prompt_msg_id);

    let calls = body.transport().snapshot_calls();
    let sent_prompt = calls
        .iter()
        .find_map(|c| match c {
            RecordedCall::SendMessage {
                chat_id,
                text,
                buttons,
                ..
            } => Some((*chat_id, text.clone(), buttons.clone())),
            _ => None,
        })
        .expect("sendMessage called for approval.prompt");
    assert_eq!(sent_prompt.0, 42);
    assert!(sent_prompt.1.contains("held-act-99"));
    assert!(sent_prompt.1.contains("fs.write"));
    let kb = sent_prompt.2.expect("inline_keyboard present");
    assert_eq!(kb.len(), 1);
    assert_eq!(kb[0].len(), 2);
    assert_eq!(kb[0][0].text, "Yes, Proceed");
    assert_eq!(
        kb[0][0].callback_data.as_deref(),
        Some("monoagens:approval:held-act-99:approve")
    );
    assert_eq!(kb[0][1].text, "No, Stop");
    assert_eq!(
        kb[0][1].callback_data.as_deref(),
        Some("monoagens:approval:held-act-99:deny")
    );

    // 2. Tapping Approve translates to OutboundTelegramEvent::ApprovalResolve (NOT percept/emit)
    let approve_upd = TgUpdate {
        update_id: 501,
        message: None,
        edited_message: None,
        callback_query: Some(TgCallbackQuery {
            id: "cbq-approve-1".to_owned(),
            from: TgUser {
                id: 9001,
                is_bot: false,
                first_name: "Owner".to_owned(),
                last_name: None,
                username: Some("owner".to_owned()),
            },
            message: Some(TgMessage {
                message_id: prompt_msg_id,
                message_thread_id: None,
                from: None,
                sender_chat: None,
                date: 1_700_000_500,
                chat: TgChat {
                    id: 42,
                    chat_type: "private".to_owned(),
                    title: None,
                    username: None,
                    is_forum: None,
                },
                reply_to_message: None,
                edit_date: None,
                media_group_id: None,
                text: None,
                caption: None,
                photo: None,
                document: None,
                video: None,
                audio: None,
                voice: None,
                sticker: None,
                new_chat_members: None,
                left_chat_member: None,
            }),
            inline_message_id: None,
            chat_instance: Some("inst-approve".to_owned()),
            data: Some(format!("{APPROVAL_CALLBACK_PREFIX}held-act-99:approve")),
        }),
        message_reaction: None,
    };

    let event = body
        .translate_update_event(&approve_upd, None)
        .unwrap()
        .expect("approval event emitted");
    let outbound = match event {
        OutboundTelegramEvent::ApprovalResolve(out) => out,
        other => panic!("expected ApprovalResolve, got {other:?}"),
    };
    assert_eq!(outbound.params.action_id, ActionId::new("held-act-99"));
    assert_eq!(outbound.params.h_act, HAct::new("sha256:deadbeef"));
    assert_eq!(outbound.params.state_epoch, 7);
    assert!(outbound.params.approved);
    assert_eq!(outbound.params.resolver, ActorId::new("tg:user:9001"));
    assert_eq!(outbound.params.resolved_at_ms.as_millis(), 1_700_000_500_000);
    assert!(outbound.params.out_of_band_channel);

    assert!(body.transport().snapshot_calls().iter().any(|c| matches!(
        c,
        RecordedCall::AnswerCallbackQuery {
            callback_query_id,
            text: Some(txt),
            ..
        } if callback_query_id == "cbq-approve-1" && txt == "Approval submitted"
    )));

    // 3. Mark resolved -> clears buttons via editMessageText and prevents duplicate resolution
    body.mark_approval_resolved("held-act-99", true, &outbound.params.resolver);
    assert!(body.pending_approval("held-act-99").is_none());
    assert!(body.transport().snapshot_calls().iter().any(|c| matches!(
        c,
        RecordedCall::EditMessageText {
            chat_id: 42,
            message_id,
            text,
            buttons: Some(btns),
            ..
        } if *message_id == prompt_msg_id
            && text.contains("Approved by tg:user:9001")
            && btns.is_empty()
    )));

    // Re-tapping after resolution answers "Approval expired or unknown" and emits nothing
    let stale_res = body.translate_update_event(&approve_upd, None).unwrap();
    assert!(stale_res.is_none());
    assert!(body.transport().snapshot_calls().iter().any(|c| matches!(
        c,
        RecordedCall::AnswerCallbackQuery {
            callback_query_id,
            text: Some(txt),
            ..
        } if callback_query_id == "cbq-approve-1" && txt == "Approval expired or unknown"
    )));

    // 4. Deny button path with per-action custom labels
    let deny_prompt = body
        .execute(make_exec(
            "act-prompt-2",
            "approval.prompt",
            "tg:group:-100555",
            json!({
                "action_id": "held-act-deny",
                "h_act": "sha256:cafebabe",
                "state_epoch": 12,
                "capability_id": "ros2.move",
                "summary": "Move arm",
                "expires_at_ms": 1_700_000_999_000u64,
                "approve_label": "Allow",
                "deny_label": "Reject"
            })
            .to_string()
            .into_bytes(),
        ))
        .unwrap();
    assert_eq!(deny_prompt.receipt.unwrap().outcome, ReceiptOutcome::Done);

    let deny_upd = TgUpdate {
        update_id: 502,
        message: None,
        edited_message: None,
        callback_query: Some(TgCallbackQuery {
            id: "cbq-deny-1".to_owned(),
            from: TgUser {
                id: 9002,
                is_bot: false,
                first_name: "Reviewer".to_owned(),
                last_name: None,
                username: None,
            },
            message: None,
            inline_message_id: None,
            chat_instance: None,
            data: Some(format!("{APPROVAL_CALLBACK_PREFIX}held-act-deny:deny")),
        }),
        message_reaction: None,
    };
    let deny_event = body
        .translate_update_event(&deny_upd, None)
        .unwrap()
        .expect("deny event emitted");
    match deny_event {
        OutboundTelegramEvent::ApprovalResolve(out) => {
            assert_eq!(out.params.action_id, ActionId::new("held-act-deny"));
            assert_eq!(out.params.h_act, HAct::new("sha256:cafebabe"));
            assert_eq!(out.params.state_epoch, 12);
            assert!(!out.params.approved);
            assert_eq!(out.params.resolver, ActorId::new("tg:user:9002"));
        }
        other => panic!("expected ApprovalResolve, got {other:?}"),
    }

    // 5. Normal non-approval callback_query continues emitting percept/emit
    let normal_cb = TgUpdate {
        update_id: 503,
        message: None,
        edited_message: None,
        callback_query: Some(TgCallbackQuery {
            id: "cbq-normal-1".to_owned(),
            from: TgUser {
                id: 42,
                is_bot: false,
                first_name: "Bob".to_owned(),
                last_name: None,
                username: None,
            },
            message: None,
            inline_message_id: None,
            chat_instance: None,
            data: Some("menu:settings".to_owned()),
        }),
        message_reaction: None,
    };
    match body.translate_update_event(&normal_cb, None).unwrap() {
        Some(OutboundTelegramEvent::Percept(p)) => {
            let v: Value = serde_json::from_slice(p.payload.as_bytes()).unwrap();
            assert_eq!(v["kind"], json!("callback_query"));
            assert_eq!(v["callback_data"], json!("menu:settings"));
        }
        other => panic!("expected normal Percept, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_mock_http_approval_prompt_and_callback_resolve() {
    #[derive(Clone, Debug)]
    struct HttpCall {
        path: String,
        body: String,
    }

    let captured: Arc<Mutex<Vec<HttpCall>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cap_srv = Arc::clone(&captured);

    let srv = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let cap = Arc::clone(&cap_srv);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    let n = match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none()
                        && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                    {
                        header_end = Some(pos + 4);
                        let headers = String::from_utf8_lossy(&buf[..pos]);
                        for line in headers.lines() {
                            if let Some((k, v)) = line.split_once(':')
                                && k.trim().eq_ignore_ascii_case("content-length")
                            {
                                content_len = v.trim().parse::<usize>().unwrap_or(0);
                            }
                        }
                    }
                    if let Some(hend) = header_end
                        && buf.len() >= hend + content_len
                    {
                        break;
                    }
                }
                let hend = header_end.unwrap();
                let head = String::from_utf8_lossy(&buf[..hend]);
                let path = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_owned();
                let body = String::from_utf8_lossy(&buf[hend..hend + content_len]).into_owned();
                cap.lock().unwrap().push(HttpCall {
                    path: path.clone(),
                    body,
                });

                let resp_json = if path.ends_with("/sendMessage") {
                    json!({
                        "ok": true,
                        "result": { "message_id": 888, "date": 1700000500, "chat": { "id": 42, "type": "private" } }
                    })
                } else {
                    json!({ "ok": true, "result": true })
                };
                let bytes = resp_json.to_string().into_bytes();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.write_all(&bytes).await;
            });
        }
    });

    let api_base = format!("http://{addr}");
    tokio::task::spawn_blocking(move || {
        let transport = LiveTelegramTransport::new(api_base, "tok-approval").unwrap();
        let mut body = TelegramBody::new(BodyId::new("body-telegram"), transport);
        body.set_now_ms(1_700_000_600_000);

        let res = body
            .execute(make_exec(
                "act-http-ap",
                "approval.prompt",
                "tg:dm:42",
                json!({
                    "action_id": "http-held-1",
                    "h_act": "sha256:112233",
                    "state_epoch": 3,
                    "capability_id": "shell.exec",
                    "summary": "Run echo hello",
                    "expires_at_ms": 1_700_000_900_000u64
                })
                .to_string()
                .into_bytes(),
            ))
            .unwrap();
        assert_eq!(res.receipt.unwrap().outcome, ReceiptOutcome::Done);

        let upd = TgUpdate {
            update_id: 700,
            message: None,
            edited_message: None,
            callback_query: Some(TgCallbackQuery {
                id: "cbq-http-1".to_owned(),
                from: TgUser {
                    id: 777,
                    is_bot: false,
                    first_name: "Alice".to_owned(),
                    last_name: None,
                    username: None,
                },
                message: None,
                inline_message_id: None,
                chat_instance: None,
                data: Some("monoagens:approval:http-held-1:approve".to_owned()),
            }),
            message_reaction: None,
        };
        let ev = body.translate_update_event(&upd, None).unwrap().unwrap();
        match ev {
            OutboundTelegramEvent::ApprovalResolve(out) => {
                assert_eq!(out.params.action_id, ActionId::new("http-held-1"));
                assert_eq!(out.params.h_act, HAct::new("sha256:112233"));
                assert_eq!(out.params.state_epoch, 3);
                assert!(out.params.approved);
                assert_eq!(out.params.resolver, ActorId::new("tg:user:777"));
                body.mark_approval_resolved("http-held-1", true, &out.params.resolver);
            }
            other => panic!("expected ApprovalResolve, got {other:?}"),
        }
    })
    .await
    .unwrap();

    srv.abort();

    let calls = captured.lock().unwrap();
    let send_msg = calls
        .iter()
        .find(|c| c.path.ends_with("/sendMessage"))
        .expect("sendMessage called");
    let send_val: Value = serde_json::from_str(&send_msg.body).unwrap();
    assert_eq!(
        send_val["reply_markup"]["inline_keyboard"][0][0]["callback_data"],
        json!("monoagens:approval:http-held-1:approve")
    );
    assert_eq!(
        send_val["reply_markup"]["inline_keyboard"][0][1]["callback_data"],
        json!("monoagens:approval:http-held-1:deny")
    );

    let ack = calls
        .iter()
        .find(|c| c.path.ends_with("/answerCallbackQuery"))
        .expect("answerCallbackQuery called");
    let ack_val: Value = serde_json::from_str(&ack.body).unwrap();
    assert_eq!(ack_val["text"], json!("Approval submitted"));

    assert!(calls.iter().any(|c| c.path.ends_with("/editMessageText")));
}
