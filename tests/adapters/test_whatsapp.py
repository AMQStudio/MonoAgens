"""Tests for Task 5 Item 1.3: WhatsApp adapter, deferred refs, D66 shared account, and D69 import."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace

from neonize.proto import Neonize_pb2 as nb
from neonize.proto.waCommon.WACommon_pb2 import MessageKey
from neonize.proto.waE2E import WAWebProtobufsE2E_pb2 as e2e
from neonize.proto.waHistorySync.WAWebProtobufsHistorySync_pb2 import (
    Conversation,
    HistorySync,
    HistorySyncMsg,
    PhoneNumberToLIDMapping,
)
from neonize.proto.waWeb.WAWebProtobufsWeb_pb2 import WebMessageInfo
from neonize.utils.jid import build_jid

from monoagens.adapters.whatsapp import WhatsAppBody
from monoagens.adapters.whatsapp.history_sync import history_props
from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.middleware import contract, identity
from monoagens.types import Action, Event

START = datetime(2026, 11, 13, 12, 0, tzinfo=timezone.utc)
OWN_PN = "6281100001"
OWN_LID = "111222333444"
BUDI_PN = "6281100002"
BUDI_LID = "999888777666"
GROUP_JID = "120363000111@g.us"
COMM_JID = "120363000999@g.us"
CHAN_JID = "120363000777@newsletter"


class FakeLLM:
    """Counts calls without hitting any real model endpoint."""

    def __init__(self, calls: list[list[ToolCall]] | None = None) -> None:
        self.calls: list[list[dict]] = []
        self.scripted = list(calls or ())
        self.usage = {"prompt_tokens": 10, "completion_tokens": 5}

    async def call(self, messages, tools, **_kw):
        self.calls.append(list(messages))
        return self.scripted.pop(0) if self.scripted else []


class FakeWAClient:
    """Records neonize calls and serves scripted media downloads and re-upload retries."""

    def __init__(self) -> None:
        self.me = SimpleNamespace(JID=build_jid(OWN_PN, "s.whatsapp.net"),
                                  LID=build_jid(OWN_LID, "lid"))
        self.sent: list[tuple[str, tuple]] = []
        self.downloads: list[str] = []
        self.reuploads: list[tuple[str, str]] = []
        self.media_bytes: bytes = b"\xff\xd8\xff\xe0FAKEJPEGDATA"
        self.fail_once_expired = False
        self.always_expired = False
        self.group_infos: dict[str, nb.GroupInfo] = {}
        self.newsletters: dict[str, nb.NewsletterMetadata] = {}
        self._id_seq = 0

    def _next_id(self) -> str:
        self._id_seq += 1
        return f"WA_OUT_{self._id_seq}"

    async def get_me(self):
        return self.me

    async def send_message(self, to, message, **_kw):
        self.sent.append(("send_message", (to, message)))
        return SimpleNamespace(ID=self._next_id())

    async def send_image(self, to, file, **_kw):
        self.sent.append(("send_image", (to, bytes(file) if isinstance(file, (bytes, bytearray)) else file)))
        return SimpleNamespace(ID=self._next_id())

    async def send_audio(self, to, file, **kw):
        self.sent.append(("send_audio", (to, kw.get("ptt"))))
        return SimpleNamespace(ID=self._next_id())

    async def send_document(self, to, file, **kw):
        self.sent.append(("send_document", (to, kw.get("filename"))))
        return SimpleNamespace(ID=self._next_id())

    async def send_sticker(self, to, file, **_kw):
        self.sent.append(("send_sticker", (to, bytes(file) if isinstance(file, (bytes, bytearray)) else file)))
        return SimpleNamespace(ID=self._next_id())

    async def edit_message(self, chat, message_id, new_message):
        self.sent.append(("edit_message", (chat, message_id, new_message.conversation)))
        return SimpleNamespace(ID=message_id)

    async def build_reaction(self, chat, sender, message_id, reaction):
        return ("REACTION", chat, message_id, reaction)

    async def build_poll_vote_creation(self, name, options, selectable_count):
        return ("POLL", name, list(options), selectable_count)

    async def send_chat_presence(self, jid, state, media):
        self.sent.append(("typing", (jid, state)))

    async def get_group_info(self, jid):
        key = f"{jid.User}@{jid.Server}"
        return self.group_infos.get(key)

    async def get_newsletter_info(self, jid):
        key = f"{jid.User}@{jid.Server}"
        return self.newsletters.get(key)

    async def download_any(self, proto):
        if self.always_expired:
            raise RuntimeError("410 Gone: media expired")
        if self.fail_once_expired:
            self.fail_once_expired = False
            raise RuntimeError("410 Gone: media expired")
        self.downloads.append("download")
        return self.media_bytes

    async def request_media_reupload(self, chat, msg_id, _proto):
        self.reuploads.append((chat, msg_id))


def _make_ev(*, msg_id: str = "M1", chat_user: str = BUDI_PN, chat_server: str = "s.whatsapp.net",
             sender_user: str = BUDI_PN, sender_server: str = "s.whatsapp.net",
             alt_user: str = "", alt_server: str = "", from_me: bool = False,
             pushname: str = "Budi", ts: int = 1762800000,
             message: e2e.Message | None = None) -> nb.Message:
    source = nb.MessageSource(
        Chat=build_jid(chat_user, chat_server),
        Sender=build_jid(sender_user, sender_server),
        SenderAlt=build_jid(alt_user, alt_server) if alt_user else nb.JID(),
        IsFromMe=from_me,
        IsGroup=(chat_server == "g.us"),
    )
    info = nb.MessageInfo(ID=msg_id, MessageSource=source, Pushname=pushname, Timestamp=ts)
    return nb.Message(Info=info, Message=message or e2e.Message(conversation="halo"))


def _door(aconn, tmp_path) -> contract.AttachmentDoor:
    return contract.AttachmentDoor(AttachmentStore(tmp_path / "att"), aconn)


async def _drain(kernel, put_coro) -> object:
    task = asyncio.create_task(put_coro)
    while not task.done():
        await kernel.run_until_idle()
        await asyncio.sleep(0)
    return await task


async def test_lid_and_pn_resolve_to_one_account_and_one_dm_place(aconn, tmp_path):
    body = WhatsAppBody(clock=FakeClock(START), self_account=f"whatsapp:{OWN_PN}")
    body.files = _door(aconn, tmp_path)
    seen: list[Event] = []

    async def put(ev: Event) -> None:
        seen.append(ev)

    # Message arrives from @lid with SenderAlt revealing PN
    ev1 = _make_ev(msg_id="M1", chat_user=BUDI_LID, chat_server="lid",
                   sender_user=BUDI_LID, sender_server="lid",
                   alt_user=BUDI_PN, alt_server="s.whatsapp.net",
                   message=e2e.Message(conversation="from lid"))
    await body.handle_message(ev1, put)
    # Second message arrives directly from PN without alt
    ev2 = _make_ev(msg_id="M2", chat_user=BUDI_PN, chat_server="s.whatsapp.net",
                   sender_user=BUDI_PN, sender_server="s.whatsapp.net",
                   message=e2e.Message(conversation="from pn"))
    await body.handle_message(ev2, put)

    assert len(seen) == 2
    assert seen[0].who == f"whatsapp:{BUDI_PN}"
    assert seen[1].who == f"whatsapp:{BUDI_PN}"
    assert seen[0].place == f"space:wa:{BUDI_PN}@s.whatsapp.net"
    assert seen[1].place == seen[0].place

    # Middleware intake gives both the exact same person id
    kernel = build(aconn, clock=FakeClock(START))
    r1 = await _drain(kernel, kernel.put(seen[0]))
    r2 = await _drain(kernel, kernel.put(seen[1]))
    assert r1.who == r2.who and r1.who.startswith("person:")


async def test_rich_inbound_percepts_and_album_grouping(aconn, tmp_path):
    client = FakeWAClient()
    body = WhatsAppBody(clock=FakeClock(START), client=client, self_account=f"whatsapp:{OWN_PN}",
                        album_window=0.05)
    body.files = _door(aconn, tmp_path)
    body.remember_sent("BOT_MSG_1")
    seen: list[Event] = []

    async def put(ev: Event) -> None:
        seen.append(ev)

    # Reply + mention in group
    ctx = e2e.ContextInfo(stanzaID="Q1", participant=f"{OWN_PN}@s.whatsapp.net",
                          mentionedJID=[f"{OWN_PN}@s.whatsapp.net", f"{BUDI_LID}@lid"])
    body.directory.learn_lid(f"{BUDI_LID}@lid", f"{BUDI_PN}@s.whatsapp.net")
    grp_msg = e2e.Message(extendedTextMessage=e2e.ExtendedTextMessage(text="cek @own", contextInfo=ctx))
    await body.handle_message(_make_ev(msg_id="G1", chat_user="120363000111", chat_server="g.us",
                                       sender_user=BUDI_PN, message=grp_msg), put)
    # Edit + revoke + reaction to own msg + location + contact + poll + view-once sticker
    edit_proto = e2e.ProtocolMessage(type=e2e.ProtocolMessage.MESSAGE_EDIT,
                                     key=MessageKey(ID="G1"),
                                     editedMessage=e2e.Message(conversation="revised"))
    await body.handle_message(_make_ev(msg_id="E1", message=e2e.Message(protocolMessage=edit_proto)), put)
    rev_proto = e2e.ProtocolMessage(type=e2e.ProtocolMessage.REVOKE, key=MessageKey(ID="G1"))
    await body.handle_message(_make_ev(msg_id="D1", message=e2e.Message(protocolMessage=rev_proto)), put)
    react_msg = e2e.ReactionMessage(key=MessageKey(ID="BOT_MSG_1"), text="🔥")
    await body.handle_message(_make_ev(msg_id="R1", chat_user="120363000111", chat_server="g.us",
                                       message=e2e.Message(reactionMessage=react_msg)), put)
    loc_msg = e2e.LocationMessage(degreesLatitude=-6.2, degreesLongitude=106.8, name="Monas")
    await body.handle_message(_make_ev(msg_id="L1", message=e2e.Message(locationMessage=loc_msg)), put)
    contact_msg = e2e.ContactMessage(displayName="Siti", vcard="BEGIN:VCARD")
    await body.handle_message(_make_ev(msg_id="C1", message=e2e.Message(contactMessage=contact_msg)), put)
    poll_msg = e2e.PollCreationMessage(
        name="Lunch?",
        options=[e2e.PollCreationMessage.Option(optionName="Soto"),
                 e2e.PollCreationMessage.Option(optionName="Bakso")],
        selectableOptionsCount=1,
    )
    await body.handle_message(_make_ev(msg_id="P1", message=e2e.Message(pollCreationMessage=poll_msg)), put)
    vo_sticker = e2e.Message(viewOnceMessage=e2e.FutureProofMessage(
        message=e2e.Message(stickerMessage=e2e.StickerMessage(mimetype="image/webp", fileLength=123))))
    await body.handle_message(_make_ev(msg_id="S1", message=vo_sticker), put)

    # Album of 3 photos -> MUST become ONE percept with 3 deferred attachments
    await body.handle_message(_make_ev(
        msg_id="ALBUM_1", message=e2e.Message(albumMessage=e2e.AlbumMessage(expectedImageCount=3))), put)
    for idx in range(1, 4):
        child_msg = e2e.Message(
            imageMessage=e2e.ImageMessage(mimetype="image/jpeg", fileLength=100 + idx,
                                          caption="holiday" if idx == 1 else ""),
            messageContextInfo=e2e.MessageContextInfo(
                messageAssociation=e2e.MessageAssociation(
                    associationType=e2e.MessageAssociation.MEDIA_ALBUM,
                    parentMessageKey=MessageKey(ID="ALBUM_1"),
                )
            ),
        )
        await body.handle_message(_make_ev(msg_id=f"PH_{idx}", message=child_msg), put)

    # Calls (incoming, missed, ended)
    meta = nb.BasicCallMeta(**{"from": build_jid(BUDI_PN, "s.whatsapp.net"), "callID": "CALL1"})
    await body.handle_call("incoming", meta, put)
    await body.handle_call("missed", meta, put, reason="timeout")
    await body.handle_call("ended", meta, put, reason="terminated")

    assert seen[0].payload["addressed"] is True
    assert seen[0].payload["data"]["reply_to"]["message_id"] == "Q1"
    assert seen[0].payload["data"]["mentions"] == [f"whatsapp:{BUDI_PN}"]
    assert seen[1].payload["data"]["edits"] == "G1"
    assert seen[2].payload["data"]["deletes"] == "G1"
    assert seen[3].payload["data"]["reaction"] == {"emoji": "🔥", "message_id": "BOT_MSG_1"}
    assert seen[3].payload["addressed"] is True  # reaction on soul's own message
    assert seen[4].payload["data"]["location"]["name"] == "Monas"
    assert seen[5].payload["data"]["contacts"][0]["name"] == "Siti"
    assert seen[6].payload["data"]["poll"]["question"] == "Lunch?"
    assert seen[7].payload["data"]["view_once"] is True
    assert "sticker" in seen[7].payload["data"]
    album_ev = seen[8]
    assert len(album_ev.payload["data"]["attachments"]) == 3
    assert album_ev.payload["data"]["message_id"] == "ALBUM_1"
    assert [ev.payload["data"]["call"]["status"] for ev in seen[9:12]] == ["incoming", "missed", "ended"]


async def test_shared_account_d66_distinguishes_soul_vs_phone_and_adds_context_note(aconn, tmp_path):
    client = FakeWAClient()
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=FakeLLM())
    body = WhatsAppBody(clock=clock, client=client, self_account=f"whatsapp:{OWN_PN}")
    body.files = _door(aconn, tmp_path)
    kernel.register_adapter(body)

    # 1. Pair owner from their WA account:
    code = await identity.ensure(aconn, now=START)
    await identity.claim_owner(aconn, Event(
        id=None, ts=START, kind="percept", who=f"whatsapp:{OWN_PN}",
        place=f"space:wa:{OWN_PN}@s.whatsapp.net", body="whatsapp",
        payload={"description": code, "self_account": f"whatsapp:{OWN_PN}"}), now=START)
    # Record a message sent by the soul via act()
    res = await body.act(Action(kind="say", place=f"space:wa:{BUDI_PN}@s.whatsapp.net",
                                payload={"text": "dari soul"}))
    soul_msg_id = res["message_id"]
    # Simulate Log act.result persisting across a restart (new WhatsAppBody with empty _sent)
    await kernel.conn.execute(
        "INSERT INTO events (ts, kind, who, place, body, payload, schema_version) "
        "VALUES (%s, 'act.result', 'person:owner', %s, 'whatsapp', %s::jsonb, 1)",
        (START, f"space:wa:{BUDI_PN}@s.whatsapp.net",
         f'{{"capability": "whatsapp.say", "result": {{"message_id": "{soul_msg_id}"}}}}'),
    )

    fresh_body = WhatsAppBody(clock=clock, client=client, self_account=f"whatsapp:{OWN_PN}")
    fresh_body.files = _door(aconn, tmp_path)
    captured: list[Event] = []

    async def put(ev: Event) -> None:
        captured.append(ev)

    # Echo of the soul's own message (IsFromMe=True, id in Log) -> NOT emitted
    await fresh_body.handle_message(_make_ev(
        msg_id=soul_msg_id, from_me=True, chat_user=BUDI_PN, sender_user=OWN_PN,
        message=e2e.Message(conversation="dari soul")), put)
    assert captured == []

    # Message typed on the owner's phone (IsFromMe=True, id NOT in Log) in Budi's chat -> emitted as owner, addressed=False
    await fresh_body.handle_message(_make_ev(
        msg_id="PHONE_MANUAL_1", from_me=True, chat_user=BUDI_PN, sender_user=OWN_PN,
        message=e2e.Message(conversation="typed on phone")), put)
    assert len(captured) == 1
    assert captured[0].who == f"whatsapp:{OWN_PN}"
    assert captured[0].payload["addressed"] is False

    # Message typed on owner's phone in "Message yourself" chat -> addressed=True
    await fresh_body.handle_message(_make_ev(
        msg_id="PHONE_SELF_1", from_me=True, chat_user=OWN_PN, sender_user=OWN_PN,
        message=e2e.Message(conversation="note to self")), put)
    assert len(captured) == 2
    assert captured[1].payload["addressed"] is True

    # Context hook adds the neutral shared-account note when self_account maps to owner
    await _drain(kernel, kernel.put(captured[0]))
    notes = await identity.shared_account_blocks(aconn)
    assert any("through the owner's account" in note.text for note in notes)


async def test_import_2000_messages_across_40_chats_makes_zero_model_calls(aconn, tmp_path):
    clock = FakeClock(START)
    llm = FakeLLM()
    kernel = build(aconn, clock=clock, pfc=llm)
    body = WhatsAppBody(clock=clock, self_account=f"whatsapp:{OWN_PN}")
    body.files = _door(aconn, tmp_path)

    # Ensure history_props patches field 15 (supportGroupHistory)
    assert len(history_props(True).SerializeToString()) > 20

    convs = []
    for c in range(40):
        jid = f"6289900{c:04d}@s.whatsapp.net"
        msgs = [
            HistorySyncMsg(message=WebMessageInfo(
                key=MessageKey(remoteJID=jid, ID=f"H_{c}_{m}", fromMe=False),
                message=e2e.Message(conversation=f"hist chat {c} msg {m}"),
                messageTimestamp=1762000000 + m,
            ))
            for m in range(50)
        ]
        convs.append(Conversation(ID=jid, name=f"Contact {c}", messages=msgs))
    sync = HistorySync(syncType=HistorySync.FULL, progress=100, conversations=convs)

    emitted: list[Event] = []

    async def put_into_kernel(ev: Event) -> None:
        emitted.append(ev)
        ev = await kernel.intake(ev)
        await kernel.queue.put(
            __import__("monoagens.core.kernel", fromlist=["_Queued"])._Queued(ev, None)
        )

    events = await body.handle_history_sync(sync, put_into_kernel)
    await kernel.run_until_idle()

    assert len(events) == 2000
    assert len(llm.calls) == 0
    row = await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'percept' AND (payload->>'imported')::boolean"
    )).fetchone()
    assert row[0] == 2000


async def test_deferred_photo_opens_once_fetched_and_requests_reupload_when_expired(aconn, tmp_path):
    client = FakeWAClient()
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock)
    door = _door(aconn, tmp_path)
    kernel.attachments = door.store
    body = WhatsAppBody(clock=clock, client=client, self_account=f"whatsapp:{OWN_PN}")
    body.files = door

    img = e2e.ImageMessage(mimetype="image/jpeg", fileLength=18, caption="lihat ini")
    ev = _make_ev(msg_id="PHOTO_DEF_1", message=e2e.Message(imageMessage=img))
    captured: list[Event] = []

    async def put(item: Event) -> None:
        captured.append(item)

    await body.handle_message(ev, put)
    stored_ev = await _drain(kernel, kernel.put(captured[0]))
    att = stored_ev.payload["data"]["attachments"][0]
    ref = att["ref"]
    assert att["deferred"] is True
    # Bytes are NOT downloaded on receive
    assert client.downloads == []
    assert door.store.find(ref) is None

    # Simulate an expired link that succeeds after re-upload request
    client.fail_once_expired = True
    data1, info1 = await door.open(ref, context=stored_ev.place)
    assert data1 == b"\xff\xd8\xff\xe0FAKEJPEGDATA"
    assert info1["ref"] == ref
    assert len(client.reuploads) == 1
    assert len(client.downloads) == 1
    assert door.store.find(ref) is not None

    # Second open reads from disk without calling client again
    data2, _ = await door.open(ref, context=stored_ev.place)
    assert data2 == data1
    assert len(client.downloads) == 1


async def test_channels_broadcast_and_communities_parent_and_say_outputs(aconn, tmp_path):
    client = FakeWAClient()
    client.newsletters[CHAN_JID] = nb.NewsletterMetadata(
        ID=build_jid("120363000777", "newsletter"),
        ThreadMeta=nb.NewsletterThreadMetadata(Name=nb.NewsletterText(Text="Mona News")),
        ViewerMeta=nb.NewsletterViewerMetadata(Role=nb.NewsletterRole.SUBSCRIBER),
    )
    client.group_infos[GROUP_JID] = nb.GroupInfo(
        JID=build_jid("120363000111", "g.us"),
        GroupName=nb.GroupName(Name="Subgroup A"),
        GroupLinkedParent=nb.GroupLinkedParent(LinkedParentJID=build_jid("120363000999", "g.us")),
        GroupAnnounce=nb.GroupAnnounce(IsAnnounce=False),
        Participants=[nb.GroupParticipant(PhoneNumber=build_jid(OWN_PN, "s.whatsapp.net"), IsAdmin=True)],
    )
    body = WhatsAppBody(clock=FakeClock(START), client=client, self_account=f"whatsapp:{OWN_PN}")
    body.files = _door(aconn, tmp_path)

    ch_info = await body.place_info(f"space:wa:{CHAN_JID}")
    assert ch_info["form"] == "broadcast" and ch_info["can_post"] is False and ch_info["name"] == "Mona News"

    grp_info = await body.place_info(f"space:wa:{GROUP_JID}")
    assert grp_info["form"] == "group" and grp_info["parent"] == f"space:wa:{COMM_JID}" and grp_info["can_post"] is True

    # Out: say (mentions, reply_to, location, choices, thread, voice) + react + edit
    voice_att = await body.files.put(b"OggS\x00\x02FAKEOPUS", name="voice.ogg", mime="audio/ogg")
    await body._files._conn.execute(
        "INSERT INTO events (ts, kind, who, place, body, payload, schema_version) "
        "VALUES (%s, 'percept', 'person:owner', %s, 'whatsapp', %s::jsonb, 1)",
        (START, f"space:wa:{BUDI_PN}@s.whatsapp.net",
         __import__("json").dumps({"data": {"attachments": [voice_att]}})),
    )
    await body.act(Action(kind="say", place=f"space:wa:{BUDI_PN}@s.whatsapp.net", payload={
        "text": "halo", "reply_to": "Q9", "mentions": [f"whatsapp:{BUDI_PN}"],
        "thread": "Topic", "location": {"lat": -6.2, "lon": 106.8, "name": "Office"},
        "choices": ["A", "B"], "attachments": [voice_att["ref"]], "voice": True,
    }))
    await body.act(Action(kind="react", place=f"space:wa:{BUDI_PN}@s.whatsapp.net",
                          payload={"emoji": "👍", "message_id": "M9"}))
    await body.act(Action(kind="edit", place=f"space:wa:{BUDI_PN}@s.whatsapp.net",
                          payload={"message_id": "M9", "text": "fixed"}))
    kinds = [kind for kind, _ in client.sent]
    assert "send_message" in kinds and "send_audio" in kinds and "edit_message" in kinds
    audio_call = next(args for kind, args in client.sent if kind == "send_audio")
    assert audio_call[1] is True  # ptt=True voice note
    assert any("120363000" in str(x) or "fixed" in str(x) for _, x in client.sent)

    # Unrelated extra sanity: props contains field 15
    assert b"MonoAgens" in history_props(False).SerializeToString()
