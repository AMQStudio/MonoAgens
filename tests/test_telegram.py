"""Tests for Step 3: the Telegram body over httpx.MockTransport."""

from __future__ import annotations

import asyncio
import json

import httpx
import pytest

from monoagens.bodies.telegram import TelegramBody
from monoagens.types import Action, Event

OWNER_ID = 777
CHAT_PRIVATE = 777
CHAT_GROUP = -100123
BOT_ID = 424242
BOT_USERNAME = "mona_bot"


def _message(
    *,
    message_id: int = 1,
    chat_id: int = CHAT_PRIVATE,
    chat_type: str = "private",
    text: str = "hello",
    sender_id: int = OWNER_ID,
    entities: list[dict] | None = None,
    reply_to: dict | None = None,
) -> dict:
    message: dict = {
        "message_id": message_id,
        "date": 1762800000,
        "chat": {"id": chat_id, "type": chat_type},
        "from": {"id": sender_id, "is_bot": False, "first_name": "Muqsith"},
        "text": text,
    }
    if entities is not None:
        message["entities"] = entities
    if reply_to is not None:
        message["reply_to_message"] = reply_to
    return message


class FakeTelegram:
    """Records API calls and serves scripted updates like Telegram would."""

    def __init__(self, updates: list[dict] | None = None) -> None:
        self.updates = updates or []
        self.me = {
            "id": BOT_ID,
            "is_bot": True,
            "username": BOT_USERNAME,
            "first_name": "Mona",
        }
        self.calls: list[tuple[str, dict]] = []
        self.polled = asyncio.Event()

    def handler(self, request: httpx.Request) -> httpx.Response:
        method = request.url.path.rsplit("/", 1)[-1]
        params = json.loads(request.content or b"{}")
        self.calls.append((method, params))
        if method == "getMe":
            return httpx.Response(200, json={"ok": True, "result": self.me})
        if method == "getUpdates":
            self.polled.set()
            offset = params.get("offset", 0)
            pending = [u for u in self.updates if u["update_id"] >= offset]
            return httpx.Response(200, json={"ok": True, "result": pending})
        return httpx.Response(200, json={"ok": True, "result": True})

    def methods(self) -> list[str]:
        return [method for method, _ in self.calls]

    def calls_of(self, method: str) -> list[dict]:
        return [params for name, params in self.calls if name == method]


def _body(fake: FakeTelegram, **kwargs) -> TelegramBody:
    return TelegramBody(
        "test-token",
        transport=httpx.MockTransport(fake.handler),
        **kwargs,
    )


async def _drive(body: TelegramBody, fake: FakeTelegram, expected: int, polls: int) -> list[Event]:
    """Run the long-poll loop until `expected` events or `polls` getUpdates calls."""
    events: list[Event] = []

    async def put(event: Event) -> None:
        events.append(event)

    task = asyncio.create_task(body.run(put))
    try:
        for _ in range(2000):
            if len(events) >= expected and len(fake.calls_of("getUpdates")) >= polls:
                break
            await asyncio.sleep(0.001)
        else:  # pragma: no cover - defensive
            raise AssertionError("Telegram body did not reach the expected state")
    finally:
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
    return events


async def test_get_me_call():
    fake = FakeTelegram()
    body = _body(fake)
    await _drive(body, fake, expected=0, polls=1)

    assert fake.methods()[0] == "getMe"
    assert body.bot_id == BOT_ID
    assert body.bot_username == BOT_USERNAME


async def test_private_message_from_owner():
    update = {"update_id": 500, "message": _message(message_id=11, text="halo")}
    fake = FakeTelegram([update])
    body = _body(fake)

    events = await _drive(body, fake, expected=1, polls=1)

    assert len(events) == 1
    event = events[0]
    assert event.kind == "percept"
    assert event.body == "telegram"
    assert event.who == f"person:tg:{OWNER_ID}"
    assert event.place == f"space:tg:{CHAT_PRIVATE}"
    assert event.payload["cursor"] == 500
    assert event.payload["data"]["message_id"] == 11
    assert event.payload["description"] == "halo"
    assert event.payload["addressed"] is True
    assert event.payload["place_kind"] == "direct"


async def test_group_chat_mention_and_non_mention():
    text = f"@{BOT_USERNAME} halo"
    mention = {"type": "mention", "offset": 0, "length": len(f"@{BOT_USERNAME}")}
    updates = [
        {
            "update_id": 600,
            "message": _message(
                message_id=20, chat_id=CHAT_GROUP, chat_type="supergroup", sender_id=999
            ),
        },
        {
            "update_id": 601,
            "message": _message(
                message_id=21,
                chat_id=CHAT_GROUP,
                chat_type="supergroup",
                text=text,
                sender_id=999,
                entities=[mention],
            ),
        },
    ]
    fake = FakeTelegram(updates)
    body = _body(fake)

    events = await _drive(body, fake, expected=2, polls=1)

    plain, mentioned = events
    assert plain.payload["addressed"] is False
    assert plain.payload["place_kind"] == "shared"
    assert plain.who == "person:tg:999"
    assert mentioned.payload["addressed"] is True
    assert mentioned.payload["cursor"] == 601
    assert mentioned.place == f"space:tg:{CHAT_GROUP}"


async def test_reply_to_bot_is_addressed():
    reply_to = {"message_id": 30, "from": {"id": BOT_ID, "is_bot": True, "first_name": "Mona"}}
    update = {
        "update_id": 700,
        "message": _message(
            message_id=31,
            chat_id=CHAT_GROUP,
            chat_type="supergroup",
            sender_id=999,
            text="ya",
            reply_to=reply_to,
        ),
    }
    fake = FakeTelegram([update])
    body = _body(fake)

    events = await _drive(body, fake, expected=1, polls=1)

    assert events[0].payload["addressed"] is True


async def test_restart_from_cursor_does_not_duplicate():
    updates = [
        {"update_id": 100, "message": _message(message_id=40, text="old")},
        {"update_id": 101, "message": _message(message_id=41, text="new")},
    ]
    fake = FakeTelegram(updates)

    async def get_cursor() -> int:
        return 100

    body = _body(fake, get_cursor=get_cursor)
    events = await _drive(body, fake, expected=1, polls=1)

    # Offset = cursor + 1: update 100 is never re-requested.
    assert fake.calls_of("getUpdates")[0]["offset"] == 101
    assert [e.payload["cursor"] for e in events] == [101]


async def test_non_text_update_is_skipped_but_advances_offset():
    updates = [{"update_id": 5, "edited_message": {"message_id": 1}}]
    fake = FakeTelegram(updates)
    body = _body(fake)

    events = await _drive(body, fake, expected=0, polls=2)

    assert events == []
    polls = fake.calls_of("getUpdates")
    assert "offset" not in polls[0]
    assert polls[1]["offset"] == 6


async def test_get_updates_retries_with_backoff():
    """A network error on a poll is retried; the body never crashes."""
    update = {"update_id": 800, "message": _message(message_id=50, text="retry")}
    fake = FakeTelegram([update])
    respond = fake.handler
    failures = {"left": 1}

    def flaky(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/getUpdates") and failures["left"]:
            failures["left"] -= 1
            raise httpx.ConnectError("network down", request=request)
        return respond(request)

    body = TelegramBody(
        "test-token",
        transport=httpx.MockTransport(flaky),
        retry_base=0.001,
    )
    events = await _drive(body, fake, expected=1, polls=1)

    assert [e.payload["description"] for e in events] == ["retry"]
    assert failures["left"] == 0


async def test_act_say_sends_message():
    fake = FakeTelegram()
    body = _body(fake)

    await body.act(Action(kind="say", place="space:tg:-100123", payload={"text": "halo"}))

    assert fake.calls == [("sendMessage", {"chat_id": "-100123", "text": "halo"})]


async def test_act_react_sets_reaction():
    fake = FakeTelegram()
    body = _body(fake)

    await body.act(
        Action(kind="react", place="space:tg:-100123", payload={"message_id": 7, "emoji": "👍"})
    )

    assert fake.calls == [
        (
            "setMessageReaction",
            {
                "chat_id": "-100123",
                "message_id": 7,
                "reaction": [{"type": "emoji", "emoji": "👍"}],
            },
        )
    ]


def test_capability_descriptors():
    """say counts as speaking; react does not; react accepts a message id."""
    caps = {cap["name"]: cap for cap in TelegramBody("token").capabilities()}
    assert set(caps) == {"say", "react"}
    assert caps["say"]["communicative"] is True
    assert caps["react"]["communicative"] is False
    assert caps["react"]["schema"]["properties"]["message_id"]["type"] == "integer"
    for cap in caps.values():
        assert cap["long_running"] is False
        assert cap["identity_changing"] is False
