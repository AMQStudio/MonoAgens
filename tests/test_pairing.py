"""Tests for the pairing code that makes the first sender the owner."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from monoagens import pairing
from monoagens.middleware import Continue
from monoagens.store import append_event, read_state, write_state

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
    with caplog.at_level("INFO", logger="monoagens.pairing"):
        code = await pairing.ensure(aconn, now=START)
    assert code is not None and len(code) == 7 and code[3] == "-"
    assert code.replace("-", "").isdigit()
    assert await read_state(aconn, pairing.CODE_KEY) == {"code": code}
    assert await read_state(aconn, pairing.OWNER_KEY) is None
    assert len(await _events(aconn, pairing.CREATED)) == 1
    assert code in caplog.text


async def test_ensure_is_idempotent_and_stops_once_an_owner_exists(aconn):
    first = await pairing.ensure(aconn, now=START)
    assert await pairing.ensure(aconn, now=START) == first
    assert len(await _events(aconn, pairing.CREATED)) == 1
    created = await append_event(aconn, ts=START, kind="pair.claimed", who="person:1")
    await write_state(aconn, pairing.OWNER_KEY, {"who": "person:1"}, created.id)
    assert await pairing.ensure(aconn, now=START) is None


async def test_claim_makes_the_sender_owner_and_removes_the_code(aconn):
    code = await pairing.ensure(aconn, now=START)
    event = await _percept(aconn, text=f"hello, my code is {code}")
    claimed = await pairing.claim(aconn, event, now=START)
    assert claimed is not None
    assert claimed.kind == pairing.CLAIMED
    assert claimed.caused_by == event.id
    assert await read_state(aconn, pairing.OWNER_KEY) == {"who": "person:tg:42"}
    assert await read_state(aconn, pairing.CODE_KEY) is None
    assert len(await _events(aconn, pairing.CLAIMED)) == 1


async def test_claim_reads_the_text_field_and_ignores_wrong_codes(aconn):
    code = await pairing.ensure(aconn, now=START)
    wrong = await _percept(aconn, text="123-456" if code != "123-456" else "654-321")
    assert await pairing.claim(aconn, wrong, now=START) is None
    event = await _percept(aconn, text=code, payload={"text": f"pair {code}"})
    assert await pairing.claim(aconn, event, now=START) is not None
    assert await read_state(aconn, pairing.OWNER_KEY) == {"who": "person:tg:42"}


async def test_an_existing_owner_cannot_be_replaced_by_the_pairing_code(aconn):
    code = await pairing.ensure(aconn, now=START)
    created = await append_event(aconn, ts=START, kind="pair.claimed", who="person:tg:1")
    await write_state(aconn, pairing.OWNER_KEY, {"who": "person:tg:1"}, created.id)
    event = await _percept(aconn, who="person:tg:99", text=code)
    assert await pairing.claim(aconn, event, now=START) is None
    assert await read_state(aconn, pairing.OWNER_KEY) == {"who": "person:tg:1"}
    assert await read_state(aconn, pairing.CODE_KEY) == {"code": code}


async def test_only_percepts_can_claim(aconn):
    code = await pairing.ensure(aconn, now=START)
    event = await append_event(aconn, ts=START, kind="act.out", who="person:tg:42",
                               payload={"description": code})
    assert await pairing.claim(aconn, event, now=START) is None
    assert await read_state(aconn, pairing.OWNER_KEY) is None


async def test_inbound_hook_claims_from_the_context(aconn):
    code = await pairing.ensure(aconn, now=START)
    event = await _percept(aconn, text=code)
    outcome = await pairing.inbound(event, {"conn": aconn, "now": START})
    assert isinstance(outcome, Continue)
    assert await read_state(aconn, pairing.OWNER_KEY) == {"who": "person:tg:42"}
