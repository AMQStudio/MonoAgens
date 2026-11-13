"""Discord adapter tests: hierarchy, roles, components & <3s defer, multi-attachments, out."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from types import SimpleNamespace

import discord

from monoagens.adapters.discord import DiscordBody, FILES, REQUIRES, create
from monoagens.adapters.discord.identity import guild_place_id, place_id
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware.contract import AttachmentDoor
from monoagens.middleware.loader import BootReport, Loader
from monoagens.types import Action


class _Role:
    def __init__(self, rid: int, name: str, *, default: bool = False) -> None:
        self.id = rid
        self.name = name
        self._default = default

    def is_default(self) -> bool:
        return self._default


class _Attachment:
    def __init__(self, aid: int, filename: str, data: bytes, *,
                 content_type: str = "image/png", url: str = "") -> None:
        self.id = aid
        self.filename = filename
        self._data = data
        self.size = len(data)
        self.content_type = content_type
        self.url = url or f"https://cdn.discordapp.com/attachments/1/{aid}/{filename}"

    async def read(self) -> bytes:
        return self._data


class _Sticker:
    def __init__(self, sid: int, name: str, data: bytes = b"PNG") -> None:
        self.id = sid
        self.name = name
        self.format = "png"
        self.url = f"https://cdn.discordapp.com/stickers/{sid}.png"
        self._data = data

    async def read(self) -> bytes:
        return self._data


class _SentMessage:
    def __init__(self, mid: int, content: str, *, kwargs: dict, channel: "_Channel") -> None:
        self.id = mid
        self.content = content
        self.kwargs = kwargs
        self.channel = channel
        self.reactions: list[str] = []
        self.edits: list[str] = []
        self.edited_views: list[object] = []

    async def add_reaction(self, emoji: str) -> None:
        self.reactions.append(emoji)

    async def edit(self, **kwargs) -> None:
        if "content" in kwargs:
            self.edits.append(str(kwargs["content"]))
            self.content = str(kwargs["content"])
        if "view" in kwargs:
            self.edited_views.append(kwargs["view"])

    async def create_thread(self, *, name: str) -> "_Channel":
        return _Channel(888000, name, discord.ChannelType.public_thread,
                        guild=self.channel.guild, parent=self.channel)


class _Channel:
    def __init__(self, cid: int, name: str, ctype: discord.ChannelType, *,
                 guild=None, parent=None, can_send: bool = True,
                 can_send_thread: bool = True) -> None:
        self.id = cid
        self.name = name
        self.type = ctype
        self.guild = guild
        self.parent = parent
        self.parent_id = getattr(parent, "id", None)
        self._can_send = can_send
        self._can_send_thread = can_send_thread
        self.sent: list[_SentMessage] = []
        self.typing_count = 0
        self._messages: dict[int, _SentMessage] = {}
        self._next_id = 9000

    def permissions_for(self, _member) -> SimpleNamespace:
        return SimpleNamespace(
            send_messages=self._can_send,
            send_messages_in_threads=self._can_send_thread,
        )

    async def send(self, content: str = "", **kwargs) -> _SentMessage:
        self._next_id += 1
        msg = _SentMessage(self._next_id, content, kwargs=kwargs, channel=self)
        self.sent.append(msg)
        self._messages[msg.id] = msg
        return msg

    async def create_thread(self, *, name: str, content: str = "", **kwargs) -> _SentMessage:
        self._next_id += 1
        thr = _Channel(self._next_id + 1000, name, discord.ChannelType.public_thread,
                       guild=self.guild, parent=self)
        msg = _SentMessage(self._next_id, content, kwargs=kwargs, channel=thr)
        self.sent.append(msg)
        return SimpleNamespace(message=msg, thread=thr)

    async def fetch_message(self, mid: int) -> _SentMessage:
        return self._messages[mid]

    async def trigger_typing(self) -> None:
        self.typing_count += 1


class _Client:
    def __init__(self, bot_id: int, channels: dict[int, _Channel]) -> None:
        self.user = SimpleNamespace(id=bot_id, name="MonoSoul")
        self._channels = channels
        self.closed = False

    def get_channel(self, cid: int):
        return self._channels.get(cid)

    def get_user(self, uid: int):
        return SimpleNamespace(id=uid, name="User")

    async def close(self) -> None:
        self.closed = True


async def test_discord_hierarchy_and_role_ids_on_percepts(aconn, tmp_path) -> None:
    clock = FakeClock(datetime(2026, 11, 13, 12, 0, tzinfo=timezone.utc))
    bot = SimpleNamespace(id=999, name="MonoSoul")
    guild = SimpleNamespace(id=1001, name="Home Guild", member_count=42, me=bot)
    general = _Channel(2001, "general", discord.ChannelType.text, guild=guild)
    news = _Channel(2002, "announcements", discord.ChannelType.news, guild=guild, can_send=False)
    thread = _Channel(3001, "bug-fix", discord.ChannelType.public_thread,
                      guild=guild, parent=general)
    dm = _Channel(4001, "dm-alice", discord.ChannelType.private)

    client = _Client(999, {c.id: c for c in (general, news, thread, dm)})
    store = AttachmentStore(tmp_path / "att")
    body = DiscordBody("tok", clock=clock, client=client)
    body.bot_id = "999"
    body.files = AttachmentDoor(store, aconn)

    seen = []

    async def put(ev):
        seen.append(ev)

    body._put = put

    everyone = _Role(1001, "@everyone", default=True)
    mod_role = _Role(555, "Moderator")
    author = SimpleNamespace(id=77, name="budi", display_name="Budi",
                             guild=guild, roles=[everyone, mod_role])

    msg_thread = SimpleNamespace(
        id=801, channel=thread, author=author,
        content="hey <@999> check thread", mentions=[bot],
        reference=None, attachments=[], stickers=[], poll=None, embeds=[],
        type=discord.MessageType.default, created_at=clock.now(),
    )
    ev1 = await body.handle_message(msg_thread)
    assert ev1 is not None
    assert ev1.place == place_id(thread)
    assert ev1.payload["addressed"] is True
    assert ev1.payload["place_kind"] == "shared"
    assert ev1.payload["data"]["platform_roles"] == {
        "space": guild_place_id(guild), "roles": ["555"],
    }

    # Full parent chain: thread -> channel -> guild server
    t_info = await body.place_info(place_id(thread))
    c_info = await body.place_info(t_info["parent"])
    g_info = await body.place_info(c_info["parent"])
    assert t_info["form"] == "thread"
    assert c_info["form"] == "group"
    assert g_info["form"] == "guild"
    assert c_info["parent"] == guild_place_id(guild)
    assert t_info["parents"][place_id(thread)] == place_id(general)
    assert t_info["parents"][place_id(general)] == guild_place_id(guild)
    assert (await body.place_info(place_id(news)))["can_post"] is False

    # Role removed on next percept -> cleared; DM -> no roles, direct place_kind
    author_demoted = SimpleNamespace(id=77, name="budi", display_name="Budi",
                                     guild=guild, roles=[everyone])
    msg_ch = SimpleNamespace(
        id=802, channel=general, author=author_demoted, content="hello all",
        mentions=[], reference=None, attachments=[], stickers=[], poll=None, embeds=[],
        type=discord.MessageType.default, created_at=clock.now(),
    )
    ev2 = await body.handle_message(msg_ch)
    assert ev2 is not None
    assert ev2.payload["addressed"] is False
    assert ev2.payload["data"]["platform_roles"] == {
        "space": guild_place_id(guild), "roles": [],
    }

    dm_author = SimpleNamespace(id=77, name="budi", display_name="Budi")
    msg_dm = SimpleNamespace(
        id=803, channel=dm, author=dm_author, content="hi privately",
        mentions=[], reference=None, attachments=[], stickers=[], poll=None, embeds=[],
        type=discord.MessageType.default, created_at=clock.now(),
    )
    ev3 = await body.handle_message(msg_dm)
    assert ev3 is not None
    assert ev3.payload["addressed"] is True
    assert ev3.payload["place_kind"] == "direct"
    assert "platform_roles" not in ev3.payload["data"]
    assert len(seen) == 3


async def test_discord_interactions_defer_before_put_and_render_buttons_menus(aconn, tmp_path) -> None:
    clock = FakeClock(datetime(2026, 11, 13, 12, 5, tzinfo=timezone.utc))
    bot = SimpleNamespace(id=999, name="MonoSoul")
    guild = SimpleNamespace(id=1001, name="Home Guild", member_count=10, me=bot)
    general = _Channel(2001, "general", discord.ChannelType.text, guild=guild)
    client = _Client(999, {2001: general})
    store = AttachmentStore(tmp_path / "att")
    body = DiscordBody("tok", clock=clock, client=client)
    body.bot_id = "999"
    body.files = AttachmentDoor(store, aconn)

    order: list[str] = []

    async def slow_put(ev):
        order.append("put_start")
        await asyncio.sleep(0.01)
        order.append(f"put_end:{ev.payload.get('data', {}).get('choice')}")

    body._put = slow_put

    # Say with <=5 choices draws Buttons; with 6 choices draws a Select menu.
    r_btn = await body.act(Action(kind="say", place=place_id(general), payload={
        "text": "Pick color", "choices": ["Red", "Blue"],
    }))
    r_menu = await body.act(Action(kind="say", place=place_id(general), payload={
        "text": "Pick number", "choices": ["One", "Two", "Three", "Four", "Five", "Six"],
    }))
    assert any(isinstance(c, discord.ui.Button) for c in general.sent[0].kwargs["view"].children)
    assert any(isinstance(c, discord.ui.Select) for c in general.sent[1].kwargs["view"].children)

    class _Resp:
        def __init__(self) -> None:
            self.done = False
            self.deferred = False

        def is_done(self) -> bool:
            return self.done

        async def defer(self, *, thinking: bool = False) -> None:
            order.append(f"defer:{thinking}")
            self.done = True
            self.deferred = True

    resp = _Resp()
    user = SimpleNamespace(id=77, name="budi", guild=guild, roles=[_Role(555, "Mod")])
    interaction = SimpleNamespace(
        type=discord.InteractionType.component,
        user=user,
        channel=general,
        response=resp,
        message=SimpleNamespace(id=int(r_btn["message_id"])),
        data={"custom_id": "choice:2:Blue"},
    )
    ev_btn = await body.handle_interaction(interaction)
    assert resp.deferred is True
    assert order[0] == "defer:False"
    assert ev_btn is not None
    assert ev_btn.payload["data"]["choice"] == "Blue"
    assert ev_btn.payload["data"]["answer"] == "choice:2:Blue"
    assert ev_btn.payload["data"]["reply_to"] == {"message_id": r_btn["message_id"]}
    assert ev_btn.payload["data"]["platform_roles"] == {
        "space": guild_place_id(guild), "roles": ["555"],
    }
    assert r_btn["choices"] == [
        {"label": "Red", "answer": "choice:1:Red"},
        {"label": "Blue", "answer": "choice:2:Blue"},
    ]

    resp2 = _Resp()
    interaction_menu = SimpleNamespace(
        type=discord.InteractionType.component,
        user=user,
        channel=general,
        response=resp2,
        message=SimpleNamespace(id=int(r_menu["message_id"])),
        data={"custom_id": "choice:menu", "values": ["Five"]},
    )
    ev_menu = await body.handle_interaction(interaction_menu)
    assert ev_menu is not None
    assert ev_menu.payload["data"]["choice"] == "Five"
    assert ev_menu.payload["data"]["answer"] == "Five"
    assert ev_menu.payload["data"]["reply_to"] == {"message_id": r_menu["message_id"]}


async def test_discord_multi_attachments_edits_deletes_reactions_and_out(aconn, tmp_path) -> None:
    clock = FakeClock(datetime(2026, 11, 13, 12, 10, tzinfo=timezone.utc))
    bot = SimpleNamespace(id=999, name="MonoSoul")
    guild = SimpleNamespace(id=1001, name="Home Guild", member_count=5, me=bot)
    general = _Channel(2001, "general", discord.ChannelType.text, guild=guild)
    client = _Client(999, {2001: general})
    store = AttachmentStore(tmp_path / "att")
    body = DiscordBody("tok", clock=clock, client=client)
    body.bot_id = "999"
    door = AttachmentDoor(store, aconn)
    body.files = door

    author = SimpleNamespace(id=88, name="siti", display_name="Siti", guild=guild, roles=[])
    msg = SimpleNamespace(
        id=910, channel=general, author=author, content="two photos and a sticker",
        mentions=[], reference=None,
        attachments=[
            _Attachment(1, "a.png", b"PNG_ONE"),
            _Attachment(2, "b.pdf", b"%PDF-TWO", content_type="application/pdf"),
        ],
        stickers=[_Sticker(30, "wave")],
        poll=None, embeds=[], type=discord.MessageType.default, created_at=clock.now(),
    )
    ev = await body.handle_message(msg)
    assert ev is not None
    await append_event(aconn, ts=ev.ts, kind=ev.kind, who=ev.who,
                       place=ev.place, body=ev.body, payload=ev.payload)
    atts = ev.payload["data"]["attachments"]
    assert len(atts) == 2
    assert ev.payload["data"]["sticker"]["name"] == "wave"
    raw0, _ = await door.open(atts[0]["ref"], context=ev.place)
    raw1, _ = await door.open(atts[1]["ref"], context=ev.place)
    assert raw0 == b"PNG_ONE"
    assert raw1 == b"%PDF-TWO"

    # Edit, delete, reaction on soul's own message
    ev_edit = await body.handle_message_edit(
        SimpleNamespace(content="old"),
        SimpleNamespace(id=910, channel=general, author=author, content="new"),
    )
    assert ev_edit is not None and ev_edit.payload["data"]["edits"] == "910"
    ev_del = await body.handle_message_delete(msg)
    assert ev_del is not None and ev_del.payload["data"]["deletes"] == "910"

    sent = await body.act(Action(kind="say", place=place_id(general), payload={
        "text": "look", "attachments": [atts[0]["ref"]], "location": {"lat": -6.2, "lon": 106.8, "name": "Jakarta"},
    }))
    await body.act(Action(kind="edit", place=place_id(general), payload={
        "message_id": sent["message_id"], "text": "edited text",
    }))
    await body.act(Action(kind="react", place=place_id(general), payload={
        "message_id": sent["message_id"], "emoji": "✅",
    }))
    assert general._messages[int(sent["message_id"])].edits == ["edited text"]
    assert general._messages[int(sent["message_id"])].reactions == ["✅"]

    ev_react = await body.handle_reaction_add(
        SimpleNamespace(message=SimpleNamespace(id=int(sent["message_id"]), channel=general,
                                              author=bot), emoji="👍"),
        author,
    )
    assert ev_react is not None and ev_react.payload["addressed"] is True

    # Show typing + Loader contract check
    await body.show(place_id(general), "typing")
    assert general.typing_count == 1
    assert REQUIRES == ("DISCORD_BOT_TOKEN",)
    assert FILES["per_message"] == 10
    loaded = Loader(tmp_path, env={"DISCORD_BOT_TOKEN": "tok"}).adapters(
        {"bodies": {"discord": {"enabled": True}}}, BootReport()
    )
    assert loaded[0].name == "discord"
    assert isinstance(create({}, {"DISCORD_BOT_TOKEN": "t"}), DiscordBody)
