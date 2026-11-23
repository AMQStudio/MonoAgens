use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use body_telegram::{
    BlobResolver, LiveTelegramTransport, MediaSource, MemoryBlobStager, PollKind, RecordedCall,
    RecordingTelegramTransport, SendPollPayload, TELEGRAM_TEXT_CAP, TelegramBody,
    TelegramTransport, TgCallbackQuery, TgChat, TgChatAction, TgChatMember, TgDocument, TgMessage,
    TgMessageReactionUpdated, TgPhotoSize, TgReactionType, TgSticker, TgUpdate, TgUser, TgVoice,
    extract_receipt_pids,
};
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionId, ActorId, Audience, BodyDiscoverParams, BodyHandler, BodyId,
    CapabilityId, MAX_INLINE_ATTACHMENT_BYTES, OpaquePayload, PerceptEmitParams, PlaceId,
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
