"""Item 1.7 Part 1 integration check across WhatsApp, Discord, Telegram and CLI."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone
import json
from types import SimpleNamespace
from typing import Any

import discord
import httpx
import pytest

from monoagens.adapters import cli, discord as dc_mod, telegram as tg_mod, whatsapp as wa_mod
from monoagens.boot import build
from monoagens.core.context import parts
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, read_state
from monoagens.middleware import identity, privacy
from monoagens.middleware.contract import AttachmentDoor
from monoagens.middleware.loader import BootReport, Loader
from monoagens.types import Event
from tests.support import settings

ROOT = __import__("pathlib").Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 11, 13, 9, 0, tzinfo=timezone.utc)


class _ScriptedPFC:
    """Record every context seen and execute queued tool calls per `decide` turn."""

    def __init__(self, handler) -> None:
        self.handler = handler
        self.contexts: list[Any] = []
        self.rendered_prompts: list[str] = []
        self._reasoning = Reasoning(settings=settings())

    async def decide(self, context):
        self.contexts.append(context)
        self.rendered_prompts.append(str(parts.messages(self._reasoning, context)))
        return self.handler(context, len(self.contexts))


class _FakeWAClient:
    """Fake WhatsApp neonize client recording sends and serving deferred downloads."""

    def __init__(self, payload_bytes: bytes = b"PDF-WA-BYTES") -> None:
        self.sent_messages: list[tuple[Any, Any]] = []
        self.sent_documents: list[tuple[Any, bytes, str, str]] = []
        self.sent_stickers: list[tuple[Any, bytes]] = []
        self.download_calls: int = 0
        self._payload_bytes = payload_bytes
        self._seq = 0

    async def send_message(self, jid, content, **_kw):
        self._seq += 1
        self.sent_messages.append((jid, content))
        return SimpleNamespace(ID=f"wa_sent_{self._seq}")

    async def send_document(self, jid, doc_bytes: bytes, *, filename: str = "", mimetype: str = "", **_kw):
        self._seq += 1
        self.sent_documents.append((jid, doc_bytes, filename, mimetype))
        return SimpleNamespace(ID=f"wa_doc_{self._seq}")

    async def send_sticker(self, jid, sticker_bytes: bytes, **_kw):
        self._seq += 1
        self.sent_stickers.append((jid, sticker_bytes))
        return SimpleNamespace(ID=f"wa_stk_{self._seq}")

    async def download_any(self, _message):
        self.download_calls += 1
        return self._payload_bytes

    async def mark_read(self, *_a, **_kw):
        return None

    async def send_chat_presence(self, *_a, **_kw):
        return None

    async def edit_message(self, *_a, **_kw):
        return None


class _FakeTGTransport:
    """HTTPX MockTransport for Telegram Bot API calls and file/sticker transfers."""

    def __init__(self, sticker_bytes: bytes = b"WEBP-STICKER-BYTES") -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.uploads: list[tuple[str, bytes]] = []
        self._sticker_bytes = sticker_bytes
        self._seq = 200

    def handler(self, request: httpx.Request) -> httpx.Response:
        path = request.url.path
        if "/file/bot" in path:
            return httpx.Response(200, content=self._sticker_bytes)
        method = path.rsplit("/", 1)[-1]
        ctype = request.headers.get("content-type", "")
        if ctype.startswith("application/json") and request.content:
            data = json.loads(request.content)
        else:
            data = {"multipart": True, "size": len(request.content or b"")}
            if request.content:
                self.uploads.append((method, bytes(request.content)))
        self.calls.append((method, data))
        self._seq += 1
        if method == "getFile":
            return httpx.Response(200, json={"ok": True, "result": {"file_path": "stickers/s1.webp"}})
        return httpx.Response(200, json={"ok": True, "result": {"message_id": self._seq}})


class _FakeDiscordChannel:
    """Fake Discord DM or server channel recording outbound messages & attachments."""

    def __init__(self, cid: int, *, dm: bool = True) -> None:
        self.id = cid
        self.type = discord.ChannelType.private if dm else discord.ChannelType.text
        self.guild = None if dm else SimpleNamespace(id=1001, name="Guild", member_count=4, me=None)
        self.sent: list[dict[str, Any]] = []

    async def trigger_typing(self) -> None:
        return None

    async def send(self, content: str = "", **kwargs) -> SimpleNamespace:
        self.sent.append({"content": content, **kwargs})
        return SimpleNamespace(id=7000 + len(self.sent), content=content)


class _FakeDiscordClient:
    """Minimal Discord client resolving registered channels."""

    def __init__(self, channels: dict[int, _FakeDiscordChannel]) -> None:
        self.user = SimpleNamespace(id=900, name="Mona")
        self._channels = channels

    def get_channel(self, cid: int):
        return self._channels.get(cid)


async def _audiences(conn, places: dict[str, list[str]]) -> None:
    """Publish the one fact a body writes when it first sees a place (contract.md §4.3)."""
    for place, actors in places.items():
        await append_event(conn, ts=START, kind="place.audience", place=place,
                           payload={"actors": list(actors), "unknown": False})


async def _put_idle(kernel, *events):
    tasks = [asyncio.create_task(kernel.put(ev)) for ev in events]
    while not all(t.done() for t in tasks):
        await kernel.run_until_idle()
        await asyncio.sleep(0)
    await kernel.run_until_idle()
    return [t.result() for t in tasks]


def test_the_boot_report_lists_every_body_and_the_surfaces_it_offers(tmp_path):
    report = BootReport()
    loader = Loader(ROOT, env={
        "TELEGRAM_BOT_TOKEN": "tg-tok",
        "DISCORD_BOT_TOKEN": "dc-tok",
        "WHATSAPP_SESSION_DB": str(tmp_path / "wa.db"),
    })
    adapters = {
        "cli": cli.create({}, {}),
        "telegram": tg_mod.create({}, {"TELEGRAM_BOT_TOKEN": "tg-tok"}),
        "whatsapp": wa_mod.create({"session_db": str(tmp_path / "wa.db")}, {}),
        "discord": dc_mod.create({}, {"DISCORD_BOT_TOKEN": "dc-tok"}),
    }
    for name, adapter in adapters.items():
        loader._check_adapter(name, adapter, report)

    joined = "\n".join(report.lines)
    assert "adapter cli: surfaces none" in joined
    assert "adapter telegram: surfaces none" in joined
    assert "adapter whatsapp: surfaces none" in joined
    assert "adapter discord: surfaces history" in joined
    for name in ("cli", "telegram", "whatsapp", "discord"):
        assert f"adapter {name}: files " in joined


async def test_conversation_and_private_context_continue_across_wa_dc_tg_after_verified_link(
    aconn, tmp_path
):
    clock = FakeClock(START)
    store = AttachmentStore(tmp_path / "att")
    door = AttachmentDoor(store, aconn)
    wa_client = _FakeWAClient()
    tg_http = _FakeTGTransport()
    dc_channel = _FakeDiscordChannel(3301, dm=True)

    wa_body = wa_mod.WhatsAppBody(tmp_path / "wa.db", clock=clock, client=wa_client)
    tg_body = tg_mod.TelegramBody("tg-tok", transport=httpx.MockTransport(tg_http.handler), clock=clock)
    tg_body.bot_id = 900
    dc_body = dc_mod.DiscordBody("dc-tok", clock=clock, client=_FakeDiscordClient({3301: dc_channel}))

    def _reply_handler(ctx, _turn):
        body = ctx.trigger.body
        return [ToolCall("act", {
            "capability": f"{body}.say",
            "arguments": {"text": f"noted on {body}: {ctx.trigger.payload.get('description', '')}"},
        })]

    pfc = _ScriptedPFC(_reply_handler)
    kernel = build(aconn, clock=clock, pfc=pfc, settings=settings())
    kernel.attachments = store
    for adapter in (wa_body, tg_body, dc_body):
        adapter.files = door
        kernel.register_adapter(adapter)

    wa_place = "space:wa:62811000@s.whatsapp.net"
    dc_place = "space:dc:3301"
    tg_place = "space:tg:7701"
    await _audiences(aconn, {wa_place: ["whatsapp:62811000"],
                             dc_place: ["discord:alice99"],
                             tg_place: ["telegram:7701"]})

    # 1. Alice shares a private secret in her WhatsApp DM.
    [wa_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="whatsapp:62811000",
        place=wa_place, body="whatsapp",
        payload={"description": "My launch secret is ALPHA-742", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "m_wa_1"}}, description="My launch secret is ALPHA-742"))
    assert len(wa_client.sent_messages) == 1

    # 2. Before linking: Alice on Discord DM is a different person and CANNOT see WhatsApp DM.
    clock.advance(timedelta(minutes=2))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:alice99",
        place=dc_place, body="discord",
        payload={"description": "What was my launch secret?", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "m_dc_1"}}, description="What was my launch secret?"))
    assert not await privacy.visible(aconn, context=dc_place, source=wa_place, before=wa_ev.id + 1)
    assert "ALPHA-742" not in pfc.rendered_prompts[-1]

    # 3. Verified 6-digit /link connects WhatsApp -> Discord -> Telegram into one person.
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="whatsapp:62811000",
        place=wa_place, body="whatsapp", payload={"command": "link", "arguments": []}))
    code_dc = str((await read_state(aconn, f"{identity.LINK_PREFIX}{wa_ev.who}"))["code"])
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:alice99",
        place=dc_place, body="discord", payload={"command": "link", "arguments": [code_dc]}))

    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="whatsapp:62811000",
        place=wa_place, body="whatsapp", payload={"command": "link", "arguments": []}))
    code_tg = str((await read_state(aconn, f"{identity.LINK_PREFIX}{wa_ev.who}"))["code"])
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:7701",
        place=tg_place, body="telegram",
        payload={"command": "link", "arguments": [code_tg], "place_kind": "direct",
                 "place_info": {"members": ["telegram:7701"]}}))

    # 4. After verified link: private context & conversation follow across Discord & Telegram.
    assert await privacy.visible(aconn, context=dc_place, source=wa_place, before=10**9)
    assert await privacy.visible(aconn, context=tg_place, source=wa_place, before=10**9)

    clock.advance(timedelta(minutes=2))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:alice99",
        place=dc_place, body="discord",
        payload={"description": "Continue launch plan from WA", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "m_dc_2"}}, description="Continue launch plan from WA"))
    assert "ALPHA-742" in pfc.rendered_prompts[-1]
    assert any("noted on discord:" in m["content"] for m in dc_channel.sent)

    clock.advance(timedelta(minutes=2))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:7701",
        place=tg_place, body="telegram",
        payload={"description": "Final step here on Telegram", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "m_tg_1"}}, description="Final step here on Telegram"))
    assert "ALPHA-742" in pfc.rendered_prompts[-1]
    assert "Continue launch plan from WA" in pfc.rendered_prompts[-1]
    assert any(m == "sendMessage" for m, _ in tg_http.calls)


async def test_whatsapp_deferred_file_opens_and_forwards_and_sticker_sends_back(aconn, tmp_path):
    clock = FakeClock(START)
    store = AttachmentStore(tmp_path / "att")
    door = AttachmentDoor(store, aconn)
    wa_client = _FakeWAClient(payload_bytes=b"WA-REPORT-BYTES")
    tg_http = _FakeTGTransport(sticker_bytes=b"WEBP-STICKER-BYTES")

    wa_body = wa_mod.WhatsAppBody(tmp_path / "wa.db", clock=clock, client=wa_client)
    tg_body = tg_mod.TelegramBody("tg-tok", transport=httpx.MockTransport(tg_http.handler), clock=clock)
    tg_body.bot_id = 900

    wa_place = "space:wa:62811000@s.whatsapp.net"
    tg_place = "space:tg:7701"
    stranger_place = "space:tg:9999"
    await _audiences(aconn, {wa_place: ["whatsapp:62811000"],
                             tg_place: ["telegram:7701"],
                             stranger_place: ["telegram:9999"]})

    captured_refs: dict[str, str] = {}

    def _scripted(ctx, _turn):
        atts = (ctx.trigger.payload.get("data") or {}).get("attachments") or []
        if atts:
            ref = str(atts[0]["ref"])
            if ctx.trigger.body == "whatsapp":
                captured_refs["wa_file"] = ref
                return [ToolCall("act", {"capability": "whatsapp.say",
                                         "arguments": {"text": "got file"}})]
            if ctx.trigger.body == "telegram":
                captured_refs["tg_sticker"] = ref
                return [ToolCall("act", {"capability": "telegram.say",
                                         "arguments": {"text": "", "sticker": ref}})]
        if (ctx.trigger.body == "telegram"
                and "forward please" in str(ctx.trigger.payload.get("description"))):
            return [ToolCall("act", {"capability": "telegram.say",
                                     "arguments": {"text": "Here is your WA file:",
                                                   "attachments": [captured_refs["wa_file"]]}})]
        return []

    pfc = _ScriptedPFC(_scripted)
    kernel = build(aconn, clock=clock, pfc=pfc, settings=settings())
    kernel.attachments = store
    for adapter in (wa_body, tg_body):
        adapter.files = door
        kernel.register_adapter(adapter)

    # 1. WhatsApp receives a deferred document without downloading bytes yet.
    fake_wa_ev = SimpleNamespace(
        Info=SimpleNamespace(
            ID="wa_doc_msg_1",
            Timestamp=int(clock.now().timestamp()),
            Pushname="Alice",
            MessageSource=SimpleNamespace(
                IsFromMe=False, IsGroup=False,
                Chat=SimpleNamespace(User="62811000", Server="s.whatsapp.net"),
                Sender=SimpleNamespace(User="62811000", Server="s.whatsapp.net"),
                SenderAlt=None,
            ),
        ),
        Message=SimpleNamespace(
            conversation="",
            documentMessage=SimpleNamespace(
                fileName="report.pdf", mimetype="application/pdf", fileLength=15, caption="read this"),
        ),
    )
    wa_raw_ev = await wa_body.handle_message(fake_wa_ev, lambda e: asyncio.sleep(0))
    assert wa_raw_ev is not None
    assert wa_client.download_calls == 0  # still deferred!

    [wa_stored] = await _put_idle(kernel, wa_raw_ev)
    wa_ref = captured_refs["wa_file"]

    # Stranger on Telegram cannot open WhatsApp DM file.
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:9999",
        place=stranger_place, body="telegram",
        payload={"description": "hello", "place_kind": "direct", "addressed": False}, description="hello"))
    with pytest.raises(PermissionError):
        await door.open(wa_ref, context=stranger_place)
    assert wa_client.download_calls == 0

    # Link Alice's WhatsApp + Telegram via /link, then open & forward the deferred WA file to TG.
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="whatsapp:62811000",
        place=wa_place, body="whatsapp", payload={"command": "link", "arguments": []}))
    link_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{wa_stored.who}"))["code"])
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:7701",
        place=tg_place, body="telegram",
        payload={"command": "link", "arguments": [link_code], "place_kind": "direct",
                 "place_info": {"members": ["telegram:7701"]}}))

    # Materialize & read in Telegram context succeeds lazily once linked.
    opened_bytes, opened_info = await door.open(wa_ref, context=tg_place)
    assert opened_bytes == b"WA-REPORT-BYTES"
    assert opened_info["name"] == "report.pdf"
    assert wa_client.download_calls == 1

    # Soul forwards that same WhatsApp ref via telegram.say -> uploaded with sendDocument!
    clock.advance(timedelta(minutes=1))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:7701",
        place=tg_place, body="telegram",
        payload={"description": "forward please", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "tg_req_doc"}}, description="forward please"))
    assert any(method == "sendDocument" and b"WA-REPORT-BYTES" in body
               for method, body in tg_http.uploads)

    # 2. Receive a sticker on Telegram and send that same sticker ref back natively via sendSticker!
    clock.advance(timedelta(minutes=1))
    tg_stk_ev = await tg_body._message_event(501, {
        "message_id": 60,
        "chat": {"id": 7701, "type": "private", "first_name": "Alice"},
        "from": {"id": 7701, "first_name": "Alice"},
        "sticker": {"file_id": "stk_in_1", "emoji": "😄", "file_size": 19},
    })
    await _put_idle(kernel, tg_stk_ev)
    assert captured_refs.get("tg_sticker")
    assert any(method == "sendSticker" and b"WEBP-STICKER-BYTES" in body
               for method, body in tg_http.uploads)
