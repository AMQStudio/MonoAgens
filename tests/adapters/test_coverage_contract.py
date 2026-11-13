"""D67 contract test across all 4 adapters: CLI, Telegram, WhatsApp, Discord."""

from __future__ import annotations

from datetime import datetime, timezone
from types import SimpleNamespace

import discord
import httpx
import pytest

from monoagens.adapters import cli, discord as dc_mod, telegram as tg_mod, whatsapp as wa_mod
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.middleware import contract, registry
from monoagens.middleware.contract import AttachmentDoor
from monoagens.middleware.loader import BootReport, Loader
from monoagens.types import Action


class _DummyTelegramHTTP:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict]] = []
        self._msg_id = 100

    def handler(self, request: httpx.Request) -> httpx.Response:
        import json

        method = request.url.path.rsplit("/", 1)[-1]
        data = {}
        if request.content and request.headers.get("content-type", "").startswith("application/json"):
            data = json.loads(request.content)
        self.calls.append((method, data))
        self._msg_id += 1
        if method == "sendPoll":
            return httpx.Response(200, json={
                "ok": True,
                "result": {"message_id": self._msg_id, "poll": {"id": f"poll_{self._msg_id}"}},
            })
        if method == "createForumTopic":
            return httpx.Response(200, json={"ok": True, "result": {"message_thread_id": 77}})
        if method == "getFile":
            return httpx.Response(200, json={"ok": True, "result": {"file_path": "f.ogg"}})
        if "/file/bot" in request.url.path:
            return httpx.Response(200, content=b"OGGDATA")
        return httpx.Response(200, json={"ok": True, "result": {"message_id": self._msg_id}})


class _FakeDiscordChannel:
    def __init__(self) -> None:
        self.id = 2001
        self.name = "general"
        self.type = discord.ChannelType.text
        self.guild = SimpleNamespace(id=1001, name="Server", member_count=5, me=None)
        self.sent: list[dict] = []
        self.typing_count = 0

    async def trigger_typing(self) -> None:
        self.typing_count += 1

    async def send(self, content: str = "", **kwargs) -> SimpleNamespace:
        self.sent.append({"content": content, **kwargs})
        msg = SimpleNamespace(id=5000 + len(self.sent), content=content,
                              edit=self._noop, add_reaction=self._noop,
                              delete=self._noop, create_thread=self._make_thread)
        return msg

    async def _noop(self, *a, **kw) -> None:
        return None

    async def _make_thread(self, *, name: str) -> SimpleNamespace:
        return SimpleNamespace(id=6001, name=name)

    async def create_thread(self, *, name: str, **kwargs) -> SimpleNamespace:
        return SimpleNamespace(id=6002, name=name)

    async def fetch_message(self, _mid: int) -> SimpleNamespace:
        return SimpleNamespace(id=5001, edit=self._noop, add_reaction=self._noop,
                               delete=self._noop, pin=self._noop)


class _FakeDiscordClient:
    def __init__(self, channel: _FakeDiscordChannel) -> None:
        self.user = SimpleNamespace(id=999, name="Soul")
        self._ch = channel

    def get_channel(self, _cid: int):
        return self._ch


class _FakeWAClient:
    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple]] = []
        self.me = SimpleNamespace(JID=SimpleNamespace(User="628999", Server="s.whatsapp.net"))

    async def build_reaction(self, *a, **kw):
        return SimpleNamespace()

    async def send_chat_presence(self, *a, **kw):
        self.calls.append(("presence", a))

    async def mark_read(self, *a, **kw):
        self.calls.append(("mark_read", a))

    async def edit_message(self, *a, **kw):
        self.calls.append(("edit", a))
        return SimpleNamespace(ID="wa-e1")

    async def revoke_message(self, *a, **kw):
        self.calls.append(("revoke", a))

    async def send_reaction(self, *a, **kw):
        self.calls.append(("react", a))

    async def send_message(self, *a, **kw):
        self.calls.append(("send_message", a))
        return SimpleNamespace(ID=f"wa-{len(self.calls)}")

    async def build_poll_vote_creation(self, *a, **kw):
        return SimpleNamespace()

    async def send_sticker(self, *a, **kw):
        self.calls.append(("sticker", a))
        return SimpleNamespace(ID="wa-st1")


def _make_adapter(name: str, *, tools: tuple[str, ...] = ()):
    env = {"TELEGRAM_BOT_TOKEN": "tok", "DISCORD_BOT_TOKEN": "tok",
           "WHATSAPP_SESSION_DB": "/tmp/unused.db"}
    opts: dict = {"enabled": True}
    if tools:
        opts["tools"] = list(tools)
    if name == "cli":
        return cli.create(opts, env), "space:cli"
    if name == "telegram":
        tg_http = _DummyTelegramHTTP()
        body = tg_mod.TelegramBody("tok", transport=httpx.MockTransport(tg_http.handler),
                                   enabled_tools=tools)
        body._dummy = tg_http  # type: ignore[attr-defined]
        return body, "space:tg:-1001"
    if name == "whatsapp":
        body = wa_mod.WhatsAppBody("/tmp/unused.db", client=_FakeWAClient(), enabled_tools=tools)
        return body, "space:wa:628111@s.whatsapp.net"
    if name == "discord":
        ch = _FakeDiscordChannel()
        body = dc_mod.DiscordBody("tok", client=_FakeDiscordClient(ch), enabled_tools=tools)
        return body, "space:dc:2001"
    raise AssertionError(name)


@pytest.mark.parametrize("name", ("cli", "telegram", "whatsapp", "discord"))
async def test_all_adapters_declare_valid_coverage_and_draw_shows_and_say_fields(name: str, tmp_path) -> None:
    adapter, place = _make_adapter(name)
    report = BootReport()
    Loader(tmp_path)._check_adapter(name, adapter, report)
    joined = "\n".join(report.lines)
    assert f"adapter {name} coverage: reads" in joined
    assert f"adapter {name} coverage: shows" in joined
    assert contract.check_coverage(name, adapter)

    # Every declared SHOWS item executes cleanly without ValueError.
    for show_kind in adapter.shows:
        await adapter.show(
            place, show_kind,
            text="Step update", title="Topic", emoji="👍",
            message_id="123" if name != "telegram" else 123,
            buttons=[("Yes", "approval:yes:1"), ("No", "approval:no:1")],
            options=["Alpha", "Beta"], question="Pick?",
            location={"lat": -6.2, "lon": 106.8, "name": "Jakarta"},
        )

    # Undeclared SHOWS entry fails honestly with ValueError.
    for absent in contract.SHOWS:
        if absent not in adapter.shows:
            assert absent in adapter.gaps
            with pytest.raises(ValueError):
                await adapter.show(place, absent, text="x")

    # Neutral `say` fields work across every adapter via native or fallback.
    res = await adapter.act(Action(kind="say", place=place, payload={
        "text": "Choose one:",
        "choices": ["A", "B"],
        "thread": "Design thread",
        "location": {"lat": -6.2, "lon": 106.8, "name": "Office"},
    }))
    assert res is not None or name == "whatsapp"

    # Every fallback in the boot report uses a capability in adapter.shows or plain text.
    shows = set(adapter.shows)
    for feature, chain in contract.FALLBACKS.items():
        cap, chosen = contract.fallback_choice(feature, shows)
        assert cap is None or cap in shows
        expected_line = (f"adapter {name} fallback {feature}: {chosen}"
                         if chosen == chain[0][1]
                         else f"adapter {name} fallback {feature}: {chosen} (fallback from {chain[0][1]})")
        assert expected_line in report.lines
    if name == "cli":
        assert "adapter cli fallback approval: numbered list (fallback from buttons)" in joined
        assert "adapter cli fallback choices: numbered list (fallback from buttons)" in joined
        assert "adapter cli fallback location: text (fallback from native)" in joined


@pytest.mark.parametrize("name,opt_tool", [
    ("cli", "pin"),
    ("telegram", "pin"),
    ("whatsapp", "reject_call"),
    ("discord", "manage_role"),
])
def test_platform_only_tools_are_off_by_default_and_excluded_from_prompt(name: str, opt_tool: str) -> None:
    default_adapter, _ = _make_adapter(name)
    default_names = {c["name"] for c in default_adapter.capabilities()}
    assert opt_tool not in default_names

    role_book = SimpleNamespace(can=lambda _who, _perm: True)
    reg = registry.ToolRegistry()
    reg.register(default_adapter)
    assert f"{name}.{opt_tool}" not in reg.capabilities
    assert f"{name}.{opt_tool}" not in reg.allowed("person:owner", roles=role_book)

    enabled_adapter, _ = _make_adapter(name, tools=(opt_tool,))
    enabled_names = {c["name"] for c in enabled_adapter.capabilities()}
    assert opt_tool in enabled_names
    reg2 = registry.ToolRegistry()
    reg2.register(enabled_adapter)
    assert f"{name}.{opt_tool}" in reg2.capabilities
    assert f"{name}.{opt_tool}" in reg2.allowed("person:owner", roles=role_book)


async def test_telegram_native_bot_api_features_and_cli_numbered_choices(aconn, tmp_path) -> None:
    clock = FakeClock(datetime(2026, 11, 13, 14, 0, tzinfo=timezone.utc))
    store = AttachmentStore(tmp_path / "att")
    door = AttachmentDoor(store, aconn)

    # CLI numbered choices answer carries data.choice and data.reply_to
    cli_body = cli.CLIBody("/tmp/unused.sock", clock=clock)
    sent_cli = await cli_body.act(Action(kind="say", place="space:cli", payload={
        "text": "Pick mode:", "choices": ["Fast", "Careful"],
    }))
    ev_cli = await cli_body._event("2")
    assert ev_cli.payload["data"]["choice"] == "Careful"
    assert ev_cli.payload["data"]["reply_to"] == {"message_id": sent_cli["message_id"]}

    # Telegram: edit, voice note stored as attachment (not transcribed), inline button reply, sync_commands
    tg_http = _DummyTelegramHTTP()
    tg_body = tg_mod.TelegramBody("tok", transport=httpx.MockTransport(tg_http.handler), clock=clock)
    tg_body.bot_id = 900
    tg_body.files = door
    await tg_body.sync_commands([{"command": "help", "description": "Show help"}])
    assert any(m == "setMyCommands" for m, _ in tg_http.calls)

    await tg_body.act(Action(kind="say", place="space:tg:-1001", payload={
        "text": "Pick:", "choices": ["Red", "Blue"],
    }))
    await tg_body.act(Action(kind="edit", place="space:tg:-1001", payload={
        "message_id": 102, "text": "Updated pick:",
    }))
    assert any(m == "editMessageText" for m, _ in tg_http.calls)

    ev_cb = tg_body._callback({
        "id": "cb1",
        "from": {"id": 42, "first_name": "Budi"},
        "message": {"message_id": 102, "chat": {"id": -1001, "type": "supergroup"}},
        "data": "choice:2:Blue",
    })
    assert ev_cb.payload["addressed"] is True
    assert ev_cb.payload["data"]["choice"] == "Blue"
    assert ev_cb.payload["data"]["reply_to"] == {"message_id": "102"}

    ev_voice = await tg_body._message_event(88, {
        "message_id": 50,
        "chat": {"id": 42, "type": "private", "first_name": "Budi"},
        "from": {"id": 42, "first_name": "Budi"},
        "voice": {"file_id": "voice_f1", "duration": 4},
    })
    assert len(ev_voice.payload["data"]["attachments"]) == 1
    assert "transcript" not in ev_voice.payload["data"]
