"""Tests for the pairing code that makes the first sender the owner."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.middleware import identity
from monoagens.core.hooks import Continue
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 1, 1, tzinfo=timezone.utc)


async def _percept(conn, who="person:tg:42", text="", payload=None):
    return await append_event(conn, ts=START, kind="percept", who=who,
                              place=f"space:{who}", body="telegram",
                              payload=payload or {"description": text})


async def _events(conn, kind):
    cur = await conn.execute("SELECT id, caused_by FROM events WHERE kind = %s ORDER BY id",
                             (kind,))
    return await cur.fetchall()


async def test_first_start_creates_and_shows_a_pairing_code(aconn, caplog):
    with caplog.at_level("INFO", logger="monoagens.middleware.identity"):
        code = await identity.ensure(aconn, now=START)
    assert code is not None and len(code) == 7 and code[3] == "-"
    assert code.replace("-", "").isdigit()
    assert await read_state(aconn, identity.CODE_KEY) == {"code": code}
    assert await read_state(aconn, identity.OWNER_KEY) is None
    assert len(await _events(aconn, identity.CREATED)) == 1
    assert code in caplog.text


async def test_ensure_is_idempotent_and_stops_once_an_owner_exists(aconn):
    first = await identity.ensure(aconn, now=START)
    assert await identity.ensure(aconn, now=START) == first
    assert len(await _events(aconn, identity.CREATED)) == 1
    created = await append_event(aconn, ts=START, kind="pair.claimed", who="person:1")
    await write_state(aconn, identity.OWNER_KEY, {"who": "person:1"}, created.id)
    assert await identity.ensure(aconn, now=START) is None


async def test_claim_makes_the_sender_owner_and_removes_the_code(aconn):
    code = await identity.ensure(aconn, now=START)
    event = await _percept(aconn, text=f"hello, my code is {code}")
    claimed = await identity.claim_owner(aconn, event, now=START)
    assert claimed is not None
    assert claimed.kind == identity.CLAIMED
    assert claimed.caused_by == event.id
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:42"}
    assert await read_state(aconn, identity.CODE_KEY) is None
    assert len(await _events(aconn, identity.CLAIMED)) == 1


async def test_claim_reads_the_text_field_and_ignores_wrong_codes(aconn):
    code = await identity.ensure(aconn, now=START)
    wrong = await _percept(aconn, text="123-456" if code != "123-456" else "654-321")
    assert await identity.claim_owner(aconn, wrong, now=START) is None
    event = await _percept(aconn, text=code, payload={"text": f"pair {code}"})
    assert await identity.claim_owner(aconn, event, now=START) is not None
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:42"}


async def test_an_existing_owner_cannot_be_replaced_by_the_pairing_code(aconn):
    code = await identity.ensure(aconn, now=START)
    created = await append_event(aconn, ts=START, kind="pair.claimed", who="person:tg:1")
    await write_state(aconn, identity.OWNER_KEY, {"who": "person:tg:1"}, created.id)
    event = await _percept(aconn, who="person:tg:99", text=code)
    assert await identity.claim_owner(aconn, event, now=START) is None
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:1"}
    assert await read_state(aconn, identity.CODE_KEY) == {"code": code}


async def test_only_percepts_can_claim(aconn):
    code = await identity.ensure(aconn, now=START)
    event = await append_event(aconn, ts=START, kind="act.out", who="person:tg:42",
                               payload={"description": code})
    assert await identity.claim_owner(aconn, event, now=START) is None
    assert await read_state(aconn, identity.OWNER_KEY) is None


async def test_inbound_hook_claims_from_the_context(aconn):
    code = await identity.ensure(aconn, now=START)
    event = await _percept(aconn, text=code)
    outcome = await identity.claim(aconn)(event, {"conn": aconn, "now": START})
    assert isinstance(outcome, Continue)
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:42"}


async def test_the_paired_owner_is_rewritten_at_intake(aconn):
    code = await identity.ensure(aconn, now=START)
    event = await _percept(aconn, text=code)
    await identity.claim_owner(aconn, event, now=START)

    rewritten = await identity.as_owner(aconn, event)
    assert rewritten.who == "person:owner"
    other = await identity.as_owner(aconn, await _percept(aconn, who="person:tg:99"))
    assert other.who == "person:tg:99"


GROUP = "space:tg:-500"


class _SayBody:
    """A body that answers `say` and records what it was asked to send."""

    name = "telegram"

    def __init__(self) -> None:
        self.actions = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False,
                 "identity_changing": False, "permission": None},)

    async def act(self, action):
        self.actions.append(action)


async def test_a_person_present_here_is_answered_here(aconn):
    """1.0b: a person with no direct place who is present here is answered here."""
    await _percept(aconn, who="person:tg:7001", text="halo")
    await append_event(aconn, ts=START, kind="percept", who="person:tg:7001",
                       place=GROUP, body="telegram",
                       payload={"description": "halo", "place_kind": "shared"})
    body = _SayBody()
    kernel = build(aconn, clock=FakeClock(START), settings=settings())
    kernel.register_adapter(body)
    trigger = Event(id=None, ts=START, kind="percept", who="person:owner", place=GROUP,
                    body="telegram", payload={"description": "jawab", "place_kind": "shared"},
                    origin_actor="person:owner")

    await kernel._apply(trigger, ToolCall("act", {
        "capability": "telegram.say",
        "arguments": {"text": "halo juga", "to": "person:tg:7001"}}))

    assert [action.place for action in body.actions] == [GROUP]
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.out'")).fetchone()
    assert row[0]["to"] == "person:tg:7001"
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'act.result' AND payload::text LIKE "
        "'%no way to reach%'")).fetchone() == (0,)


class _FakePFC:
    def __init__(self) -> None:
        self.contexts = []

    async def decide(self, context):
        self.contexts.append(context)
        return []


async def _put_idle(kernel, *events):
    import asyncio
    tasks = [asyncio.create_task(kernel.put(ev)) for ev in events]
    while not all(t.done() for t in tasks):
        await kernel.run_until_idle()
        await asyncio.sleep(0)
    await kernel.run_until_idle()
    return [t.result() for t in tasks]


async def test_unknown_account_registers_one_person_id_and_owner_stays_owner(aconn):
    import re

    kernel = build(aconn, clock=FakeClock(START), pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())
    code = await identity.ensure(aconn, now=START)

    [guest] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:123",
        place="space:tg:-500", body="telegram",
        payload={"description": "halo", "place_kind": "shared", "addressed": False}))

    assert re.fullmatch(r"person:p_[0-9a-f]+", guest.who)
    assert await read_state(aconn, "alias:telegram:123") == {"who": guest.who}
    reg = await _events(aconn, identity.REGISTERED)
    assert len(reg) == 1

    [owner_msg] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:999",
        place="space:tg:999", body="telegram",
        payload={"description": code, "place_kind": "direct", "addressed": True}))
    assert owner_msg.who != "person:owner"
    [next_owner_msg] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:999",
        place="space:tg:999", body="telegram",
        payload={"description": "halo lagi", "place_kind": "direct", "addressed": True}))
    assert next_owner_msg.who == "person:owner"


async def test_two_racing_percepts_of_one_account_get_one_person_id(aconn):
    kernel = build(aconn, clock=FakeClock(START), pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())
    first, second = await _put_idle(
        kernel,
        Event(id=None, ts=START, kind="percept", who="whatsapp:628110099",
              place="space:wa:g1", body="telegram",
              payload={"description": "satu", "place_kind": "shared", "addressed": False}),
        Event(id=None, ts=START, kind="percept", who="whatsapp:628110099",
              place="space:wa:g1", body="telegram",
              payload={"description": "dua", "place_kind": "shared", "addressed": False}),
    )

    assert first.who == second.who
    assert first.who.startswith("person:p_")
    assert len(await _events(aconn, identity.REGISTERED)) == 1


async def test_mentions_and_reply_to_are_mapped_before_core_sees_them(aconn):
    fake = _FakePFC()
    kernel = build(aconn, clock=FakeClock(START), pfc=fake, settings=settings())
    kernel.register_adapter(_SayBody())

    [stored] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:101",
        place="space:tg:-500", body="telegram",
        payload={"description": "tolong cek", "place_kind": "shared", "addressed": True,
                 "data": {"message_id": 10, "mentions": ["telegram:202", "telegram:303"],
                          "reply_to": {"message_id": 5, "who": "telegram:202"}}}))

    assert stored.who.startswith("person:p_")
    mentions = stored.payload["data"]["mentions"]
    reply_who = stored.payload["data"]["reply_to"]["who"]
    assert len(mentions) == 2 and all(m.startswith("person:p_") for m in mentions)
    assert reply_who == mentions[0]
    row = await (await aconn.execute(
        "SELECT who, payload::text FROM events WHERE id = %s", (stored.id,))).fetchone()
    assert "telegram:" not in row[0] and "telegram:" not in row[1]
    assert fake.contexts
    assert all("telegram:" not in str(line)
               for ctx in fake.contexts
               for line in (*ctx.people, *ctx.memories, ctx.trigger.who,
                            str(ctx.trigger.payload)))
