"""Tests for the pairing code that makes the first sender the owner."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.middleware import identity
from monoagens.core.hooks import Continue
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.types import Action, Event
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
    await append_event(aconn, ts=START, kind="place.audience", place=GROUP,
                       payload={"place": GROUP,
                                "actors": ["person:tg:7001", "person:owner"]})
    await append_event(aconn, ts=START, kind="percept", who="person:tg:7001",
                       place=GROUP, body="telegram",
                       payload={"description": "halo"})
    body = _SayBody()
    kernel = build(aconn, clock=FakeClock(START), settings=settings())
    kernel.register_adapter(body)
    trigger = Event(id=None, ts=START, kind="percept", who="person:owner", place=GROUP,
                    body="telegram", payload={"description": "jawab"}, description="jawab",
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
        payload={"description": "halo", "place_kind": "shared", "addressed": False}, description="halo"))

    assert re.fullmatch(r"person:p_[0-9a-f]+", guest.who)
    assert await read_state(aconn, "alias:telegram:123") == {"who": guest.who}
    reg = await _events(aconn, identity.REGISTERED)
    assert len(reg) == 1

    [owner_msg] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:999",
        place="space:tg:999", body="telegram",
        payload={"description": code, "place_kind": "direct", "addressed": True}, description=code))
    assert owner_msg.who != "person:owner"
    [next_owner_msg] = await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:999",
        place="space:tg:999", body="telegram",
        payload={"description": "halo lagi", "place_kind": "direct", "addressed": True}, description="halo lagi"))
    assert next_owner_msg.who == "person:owner"


async def test_two_racing_percepts_of_one_account_get_one_person_id(aconn):
    kernel = build(aconn, clock=FakeClock(START), pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())
    first, second = await _put_idle(
        kernel,
        Event(id=None, ts=START, kind="percept", who="whatsapp:628110099",
              place="space:wa:g1", body="telegram",
              payload={"description": "satu", "place_kind": "shared", "addressed": False}, description="satu"),
        Event(id=None, ts=START, kind="percept", who="whatsapp:628110099",
              place="space:wa:g1", body="telegram",
              payload={"description": "dua", "place_kind": "shared", "addressed": False}, description="dua"),
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
                          "reply_to": {"message_id": 5, "who": "telegram:202"}}}, description="tolong cek"))

    assert stored.who.startswith("person:p_")
    mentions = stored.payload["data"]["mentions"]
    reply_who = stored.payload["data"]["reply_to"]["who"]
    assert len(mentions) == 2 and all(m.startswith("person:p_") for m in mentions)
    assert reply_who == mentions[0]
    row = await (await aconn.execute(
        "SELECT who, payload::text FROM events WHERE id = %s", (stored.id,))).fetchone()
    assert "telegram:" not in row[0] and "telegram:" not in row[1]
    assert fake.contexts
    assert all("telegram:" not in str(item.text)
               for ctx in fake.contexts
               for item in ctx.items)
    assert all("telegram:" not in str(ctx.trigger.who) for ctx in fake.contexts)


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
        payload={"description": "suka kopi hitam", "place_kind": "shared", "addressed": False}, description="suka kopi hitam"))
    clock.advance(timedelta(minutes=5))
    [second_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:222",
        place="space:dc:99", body="discord",
        payload={"description": "alergi kacang", "place_kind": "shared", "addressed": False}, description="alergi kacang"))
    older_id, younger_id = first_ev.who, second_ev.who
    assert older_id != younger_id
    await append_event(aconn, ts=clock.now(), kind="place.audience", place="space:tg:-10",
                       payload={"place": "space:tg:-10", "actors": ["telegram:111"]})
    await append_event(aconn, ts=clock.now(), kind="place.audience", place="space:dc:99",
                       payload={"place": "space:dc:99", "actors": ["discord:222"]})

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
    assert len(people_cards) == 1 and older_id in people_cards[0]
    assert younger_id not in people_cards[0]

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
        aconn, Action(kind="recall", place="space:dc:99", arguments={"query": older_id}),
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
        payload={"description": "halo", "place_kind": "direct", "addressed": True}, description="halo"))
    early_id = early.who
    await _put_idle(kernel, Event(
        id=None, ts=START, kind="percept", who="telegram:1",
        place="space:tg:1", body="telegram",
        payload={"description": code, "place_kind": "direct", "addressed": True}, description=code))

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
        payload={"description": "hi", "place_kind": "direct", "addressed": True}, description="hi"))
    [b_ev] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:502",
        place="space:dc:502", body="discord",
        payload={"description": "yo", "place_kind": "direct", "addressed": True}, description="yo"))

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
        payload={"description": "baru", "place_kind": "direct", "addressed": True}, description="baru"))
    assert after_unlink.who not in (a_ev.who, b_ev.who)
    assert after_unlink.who.startswith("person:p_")
    assert await read_state(aconn, f"merged:{b_ev.who}") == {"into": a_ev.who}
    assert len(await _events(aconn, identity.UNLINKED)) == 1


async def test_link_tries_are_counted_per_guesser_and_never_burn_other_codes(aconn):
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())

    [alice] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:601",
        place="space:tg:601", body="telegram",
        payload={"description": "alice", "place_kind": "direct", "addressed": True}, description="alice"))
    [bob] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:602",
        place="space:tg:602", body="telegram",
        payload={"description": "bob", "place_kind": "direct", "addressed": True}, description="bob"))
    [eve] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:603",
        place="space:dc:603", body="discord",
        payload={"description": "eve", "place_kind": "direct", "addressed": True}, description="eve"))

    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:601",
        place="space:tg:601", body="telegram", payload={"command": "link", "arguments": []}))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:602",
        place="space:tg:602", body="telegram", payload={"command": "link", "arguments": []}))
    alice_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{alice.who}"))["code"])
    bob_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{bob.who}"))["code"])

    # Eve burns her own 3 guesses; neither Alice's nor Bob's code is deleted,
    # and Eve's 4th attempt with the real code is blocked during the 10m window.
    for guess in ("000000", "111111", "222222", alice_code):
        await _put_idle(kernel, Event(
            id=None, ts=clock.now(), kind="command", who="discord:603",
            place="space:dc:603", body="discord", payload={"command": "link", "arguments": [guess]}))
    assert await identity.find_person(aconn, "discord:603") == eve.who
    assert await read_state(aconn, f"{identity.LINK_PREFIX}{alice.who}") is not None
    assert await read_state(aconn, f"{identity.LINK_PREFIX}{bob.who}") is not None

    # Legitimate second accounts can still redeem both codes immediately.
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:604",
        place="space:dc:604", body="discord",
        payload={"command": "link", "arguments": [alice_code]}))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:605",
        place="space:dc:605", body="discord",
        payload={"command": "link", "arguments": [bob_code]}))
    assert await identity.find_person(aconn, "discord:604") == alice.who
    assert await identity.find_person(aconn, "discord:605") == bob.who

    # After the 10-minute window passes, Eve may try again.
    clock.advance(timedelta(minutes=11))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:602",
        place="space:tg:602", body="telegram", payload={"command": "link", "arguments": []}))
    new_bob_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{bob.who}"))["code"])
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:603",
        place="space:dc:603", body="discord",
        payload={"command": "link", "arguments": [new_bob_code]}))
    assert await identity.find_person(aconn, "discord:603") == bob.who


async def test_owner_cannot_absorb_non_owner_via_link_code(aconn):
    clock = FakeClock(START)
    code = await identity.ensure(aconn, now=clock.now())
    kernel = build(aconn, clock=clock, pfc=_FakePFC(), settings=settings())
    kernel.register_adapter(_SayBody())

    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="telegram:700",
        place="space:tg:700", body="telegram",
        payload={"description": code, "place_kind": "direct", "addressed": True}, description=code))
    assert await identity.find_person(aconn, "telegram:700") == identity.OWNER

    [stranger] = await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="percept", who="discord:701",
        place="space:dc:701", body="discord",
        payload={"description": "hi", "place_kind": "direct", "addressed": True}, description="hi"))
    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="discord:701",
        place="space:dc:701", body="discord", payload={"command": "link", "arguments": []}))
    stranger_code = str((await read_state(aconn, f"{identity.LINK_PREFIX}{stranger.who}"))["code"])

    await _put_idle(kernel, Event(
        id=None, ts=clock.now(), kind="command", who="telegram:700",
        place="space:tg:700", body="telegram",
        payload={"command": "link", "arguments": [stranger_code]}))
    assert await identity.find_person(aconn, "discord:701") == stranger.who
    assert await read_state(aconn, f"{identity.LINK_PREFIX}{stranger.who}") is not None


async def test_pairing_code_limits_tries_per_account(aconn):
    code = await identity.ensure(aconn, now=START)
    for idx in range(3):
        wrong = await _percept(aconn, who="person:tg:999",
                               text=f"00000{idx}" if f"00000{idx}" != code else f"99999{idx}")
        assert await identity.claim_owner(aconn, wrong, now=START) is None

    # Locked out within the 10-minute window even with the real code
    locked = await _percept(aconn, who="person:tg:999", text=code)
    assert await identity.claim_owner(aconn, locked, now=START + timedelta(minutes=1)) is None
    assert await read_state(aconn, identity.OWNER_KEY) is None

    # A different account is not affected by tg:999's failed attempts
    other = await _percept(aconn, who="person:tg:100", text=code)
    assert await identity.claim_owner(aconn, other, now=START + timedelta(minutes=1)) is not None
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:100"}


async def test_pairing_code_lockout_clears_after_window(aconn):
    code = await identity.ensure(aconn, now=START)
    for idx in range(3):
        wrong = await _percept(aconn, who="person:tg:999",
                               text=f"00000{idx}" if f"00000{idx}" != code else f"99999{idx}")
        assert await identity.claim_owner(aconn, wrong, now=START) is None
    assert await identity.claim_owner(
        aconn, await _percept(aconn, who="person:tg:999", text=code),
        now=START + timedelta(minutes=11)) is not None
    assert await read_state(aconn, identity.OWNER_KEY) == {"who": "person:tg:999"}


async def test_direct_place_skips_place_that_became_shared(aconn):
    dm = "space:game:dm:owner"
    world = "space:game:world1"
    await append_event(aconn, ts=START, kind="place.audience", place=dm,
                       payload={"place": dm, "actors": [identity.OWNER]})
    await append_event(
        aconn, ts=START, kind="percept", who=identity.OWNER, place=dm, body="game",
        payload={"description": "hai"})
    await append_event(aconn, ts=START + timedelta(minutes=1), kind="place.audience",
                       place=world, payload={"place": world, "actors": [identity.OWNER]})
    await append_event(
        aconn, ts=START + timedelta(minutes=1), kind="percept", who=identity.OWNER,
        place=world, body="game", payload={"description": "sendiri dulu"})
    await append_event(aconn, ts=START + timedelta(minutes=2), kind="place.audience",
                       place=world,
                       payload={"place": world,
                                "actors": [identity.OWNER, "person:charlie"]})
    await append_event(
        aconn, ts=START + timedelta(minutes=2), kind="percept", who="person:charlie",
        place=world, body="game", payload={"description": "ada aku juga"})

    assert await identity.direct_place(aconn, identity.OWNER, via="game") == dm
    place, error = await identity.resolve_to(aconn, identity.OWNER)
    assert (place, error) == (dm, None)
