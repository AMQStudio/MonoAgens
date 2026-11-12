"""The send ledger: every outgoing act is confirmed in the Log, and a restart resends once."""

from __future__ import annotations

from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.core import sends
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.types import Action
from tests.support import settings

START = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
PLACE = "space:tg:1"


class SendingAdapter:
    """A tool that records every line it was asked to send and returns nothing."""

    name = "telegram"

    def __init__(self) -> None:
        self.sent: list[str] = []

    def capabilities(self) -> tuple[dict, ...]:
        """Declare one communicative and one non-communicative capability."""
        return ({"name": "say", "description": "Send a line.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False},
                {"name": "react", "description": "React with an emoji.",
                 "schema": {"type": "object", "properties": {"emoji": {"type": "string"}},
                            "required": ["emoji"]},
                 "communicative": False, "long_running": False, "identity_changing": False})

    async def act(self, action: Action) -> None:
        self.sent.append(str(action.payload.get("text") or action.payload.get("emoji")))


async def _out(conn, payload: dict) -> int:
    event = await append_event(conn, ts=START, kind="act.out", who="person:owner", place=PLACE,
                               body="telegram", payload=payload)
    return event.id


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [row[0] for row in await cur.fetchall()]


async def test_a_pending_act_is_resent_once_by_recover(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    act_id = await _out(aconn, payload={
        "capability": "telegram.say", "arguments": {"text": "halo"}, "communicative": True})
    assert [event.id for event in await sends.pending(aconn)] == [act_id]

    assert await sends.resume(kernel) == 1
    assert adapter.sent == ["halo"]
    confirmed = await (await aconn.execute(
        "SELECT payload, caused_by FROM events WHERE kind = 'act.confirmed'")).fetchall()
    assert confirmed == [({"capability": "telegram.say"}, act_id)]

    assert await sends.resume(kernel) == 0  # never twice
    assert adapter.sent == ["halo"]
    assert (await _kinds(aconn)).count("act.confirmed") == 1


async def test_a_normal_act_is_confirmed_after_it_is_recorded(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                 place=PLACE, body="telegram",
                                 payload={"description": "halo", "place_kind": "direct"})

    call = ToolCall("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})
    await kernel._apply(trigger, call)

    kinds = await _kinds(aconn)
    assert kinds.index("act.out") < kinds.index("act.confirmed")
    assert "act.result" not in kinds  # the tool returned nothing to record
    assert adapter.sent == ["hai"]


async def test_a_pending_act_whose_tool_is_gone_is_closed_with_one_error(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _out(aconn, payload={
        "capability": "gone.say", "arguments": {"text": "halo"}, "communicative": True})

    assert await sends.resume(kernel) == 0

    errors = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()
    assert len(errors) == 1 and "No adapter registered for 'gone'" in errors[0][0]["error"]
    assert await sends.pending(aconn) == []
    assert await sends.resume(kernel) == 0  # a closed row is never retried


async def test_a_non_communicative_act_is_never_pending(aconn):
    await _out(aconn, payload={
        "capability": "telegram.react", "arguments": {"emoji": "👍"}, "communicative": False})

    assert await sends.pending(aconn) == []

