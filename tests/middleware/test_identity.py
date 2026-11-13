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


async def test_link_merges_two_ids_into_one_card_memory_and_recall(aconn):
    from datetime import timedelta
    from pgvector import Vector
    from monoagens.core.context import parts
    from monoagens.core.context.layout import build_context
    from monoagens.core.memory import cards, recall
    from monoagens.core.reasoning import Reasoning
    from monoagens.middleware import privacy

    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())

    [first_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:111",
        place="space:tg:-10", body="telegram",
        payload={"description": "suka kopi hitam", "place_kind": "shared", "addressed": False}))
    clock.advance(timedelta(minutes=5))
    [second_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:222",
        place="space:dc:99", body="discord",
        payload={"description": "alergi kacang", "place_kind": "shared", "addressed": False}))
    older_id, younger_id = first_ev.who, second_ev.who
    assert older_id != younger_id

    await write_state(aconn, f"person:{older_id}", {
        "name": "Budi", "closeness": 0.4, "reliability": {"promises": {"kept": 2}}}, first_ev.id)
    await write_state(aconn, f"person:{younger_id}", {
        "closeness": 0.8, "reliability": {"promises": {"kept": 3, "broken": 1}}}, second_ev.id)
    await write_state(aconn, f"habit:space:dc:99:{younger_id}", {"h": 2.5, "ts": clock.now().isoformat()}, second_ev.id)
    await aconn.execute(
        "INSERT INTO memories (who, place, text, embedding, importance, valid_from, "
        "last_accessed_at, derived_from) VALUES (%s, %s, %s, %s, %s, %s, %s, %s)",
        (younger_id, "space:dc:99", "Budi alergi kacang", Vector([0.1] * 384), 0.9,
         clock.now(), clock.now(), [second_ev.id]))

    # Person links via 6-digit /link code across accounts
    [req_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:111",
        place="space:tg:111", body="telegram", payload={"command": "link", "arguments": []}))
    code_row = await read_state(aconn, f"{identity.LINK_PREFIX}{older_id}")
    code = str(code_row["code"])
    assert len(code) == 6
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:222",
        place="space:dc:222", body="discord", payload={"command": "link", "arguments": [code]}))

    # Merged into the older id; cards, memories, habit, last_seen follow without rewriting Log
    assert await cards.canonical(aconn, younger_id) == older_id
    assert await read_state(aconn, f"merged:{younger_id}") == {"into": older_id}
    merged_card = await read_state(aconn, f"person:{older_id}")
    assert merged_card["closeness"] == pytest.approx(0.8)
    assert merged_card["reliability"] == {"promises": {"kept": 5, "broken": 1}}
    assert await read_state(aconn, f"person:{younger_id}") is None
    assert await read_state(aconn, f"habit:space:dc:99:{older_id}") == {
        "h": 2.5, "ts": clock.now().isoformat()}
    assert await identity.accounts_of(aconn, older_id) == ("discord:222", "telegram:111")
    people_cards = await identity.person_cards(aconn, "space:tg:-10", first_ev)
    assert len(people_cards) == 1 and older_id in people_cards[0] and younger_id not in people_cards[0]

    # Log is untouched, yet audience labels, working context and recall follow the merge
    raw_row = await (await aconn.execute("SELECT who FROM events WHERE id = %s",
                                         (second_ev.id,))).fetchone()
    assert raw_row == (younger_id,)
    assert await privacy.audience(aconn, "space:dc:99") == frozenset({older_id})
    ctx = await build_context(aconn, second_ev, "space:dc:99", "check", now=clock.now(),
                              settings=settings(), memories=(), capabilities=())
    rendered = str(parts.messages(Reasoning(settings=settings()), ctx))
    assert f"<event>{older_id}: alergi kacang</event>" in rendered
    assert f"{younger_id}:" not in rendered
    res = await recall.search(
        aconn, Event(id=None, ts=clock.now(), kind="act.out", place="space:dc:99",
                     payload={"query": older_id}),
        visible=privacy.visible, memory_visible=privacy.memory_visible)
    found_texts = [item["text"] for item in res["found"]]
    assert "Budi alergi kacang" in found_texts
    assert any("alergi kacang" in text for text in found_texts)


async def test_link_owner_always_wins_and_soul_has_no_link_tool(aconn):
    kernel = build(aconn, clock=FakeClock(START), pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())
    code = await identity.ensure(aconn, now=START)

    [early] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="discord:700",
        place="space:dc:700", body="discord",
        payload={"description": "halo", "place_kind": "direct", "addressed": True}))
    early_id = early.who
    await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:1",
        place="space:tg:1", body="telegram",
        payload={"description": code, "place_kind": "direct", "addressed": True}))

    await _put_idle(kernel, Event(
        id=None, ts=START, kind="command", who="telegram:1",
        place="space:tg:1", body="telegram",
        payload={"command": "link", "arguments": ["person:owner", "discord:700"]}))

    assert await identity.find_person(aconn, "discord:700") == "person:owner"
    assert await read_state(aconn, f"merged:{early_id}") == {"into": "person:owner"}
    assert not any("link" in name for name in kernel.tools.entries)


async def test_wrong_and_expired_link_codes_and_unlink(aconn):
    from datetime import timedelta

    clock = FakeClock(START)
    body = _SayBody()
    kernel = build(aconn, clock=clock, pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(body)

    [a_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:501",
        place="space:tg:501", body="telegram",
        payload={"description": "hi", "place_kind": "direct", "addressed": True}))
    [b_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"description": "yo", "place_kind": "direct", "addressed": True}))

    # Wrong code 3 times locks out and links nothing even on try 4 with the real code
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:501",
        place="space:tg:501", body="telegram", payload={"command": "link", "arguments": []}))
    real_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{a_ev.who}"))["code"])
    for wrong in ("000000", "111111", "222222", real_code):
        await _put_idle(kernel, Event(
            id=None, ts=clock.now(), kind="command", who="discord:502",
            place="space:dc:502", body="discord",
            payload={"command": "link", "arguments": [wrong]}))
    assert await identity.find_person(aconn, "discord:502") == b_ev.who

    # Expired code links nothing
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:501",
        place="space:tg:501", body="telegram", payload={"command": "link", "arguments": []}))
    fresh_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{a_ev.who}"))["code"])
    clock.advance(timedelta(minutes=11))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"command": "link", "arguments": [fresh_code]}))
    assert await identity.find_person(aconn, "discord:502") == b_ev.who

    # Valid link then /unlink gives a new id from then on and splits nothing
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:501",
        place="space:tg:501", body="telegram", payload={"command": "link", "arguments": []}))
    valid_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{a_ev.who}"))["code"])
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"command": "link", "arguments": [valid_code]}))
    assert await identity.find_person(aconn, "discord:502") == a_ev.who

    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"command": "unlink", "arguments": ["discord:502"]}))
    [after_unlink] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"description": "baru", "place_kind": "direct", "addressed": True}))
    assert after_unlink.who not in (a_ev.who, b_ev.who)
    assert after_unlink.who.startswith("person:p_")
    assert await read_state(aconn, f"merged:{b_ev.who}") == {"into": a_ev.who}
    assert len(await _events(aconn, identity.UNLINKED)) == 1


async def test_place_persisted_and_readonly_place_refuses_before_act(aconn):
    body = _SayBody()
    kernel = build(aconn, clock=FakeClock(START), pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(body)

    [ro] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:10",
        place="space:tg:-100555:9", body="telegram",
        payload={"description": "pengumuman", "place_kind": "shared", "addressed": False,
                 "place_info": {"form": "thread", "parent": "space:tg:-100555",
                                "can_post": False, "name": "Topik"}}))
    saved = await read_state(aconn, "place:space:tg:-100555:9")
    assert saved["form"] == "thread"
    assert saved["parent"] == "space:tg:-100555"
    assert saved["can_post"] is False
    assert saved["name"] == "Topik"

    await kernel._apply(ro, ToolCall("act", {
        "capability": "telegram.say",
        "arguments": {"text": "coba balas"}}))
    await kernel._apply(ro, ToolCall("act", {
        "capability": "telegram.say",
        "arguments": {"text": "ke sana", "to": "space:tg:-100555:9"}}))

    assert body.actions == []
    results = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id")).fetchall()
    assert len(results) == 2
    assert all("read-only" in str(r[0]["result"]["error"]) for r in results)
