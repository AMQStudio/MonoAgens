"""The action ledger: one row per action, one receipt per row, and a resume that keeps ids."""

from __future__ import annotations

from datetime import datetime, timezone

from monoagens import contract
from monoagens.boot import build
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, unresolved_actions, write_action
from monoagens.types import Action
from tests.support import settings

START = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
PLACE = "space:tg:1"


class SendingAdapter:
    """A tool that records every action id it was handed and the line it carried."""

    name = "telegram"
    PERMISSIONS = {"react": "guest", "say": "guest"}

    def __init__(self) -> None:
        """Start with nothing sent."""
        self.sent: list[str] = []
        self.ids: list[str] = []

    def capabilities(self) -> tuple[dict, ...]:
        """Declare one communicative and one non-communicative capability."""
        return ({"name": "say", "description": "Send a line.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "permission": "telegram.say"},
                {"name": "react", "description": "React with an emoji.",
                 "schema": {"type": "object", "properties": {"emoji": {"type": "string"}},
                            "required": ["emoji"]},
                 "communicative": False, "long_running": False, "identity_changing": False,
                 "permission": "telegram.react"})

    async def act(self, action: Action) -> dict:
        """Record the id and the line, and answer with what the body did."""
        self.ids.append(action.id)
        line = str(action.arguments.get("text") or action.arguments.get("emoji") or "")
        self.sent.append(line)
        return {"ok": True, "text": line}


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [row[0] for row in await cur.fetchall()]


async def _trigger(conn):
    return await append_event(conn, ts=START, kind="percept", who="person:owner", place=PLACE,
                              body="telegram", payload={"description": "halo"},
                              description="halo")


async def _unclosed(conn, *, capability: str, arguments: dict, communicative: bool,
                    action_id: str = "job:7:12:0") -> int:
    """Record one act whose action never got a receipt, the way a crash leaves it."""
    trigger = await _trigger(conn)
    out = await append_event(conn, ts=START, kind="act.out", who="person:owner", place=PLACE,
                             body="telegram", caused_by=trigger.id,
                             payload={"capability": capability, "arguments": dict(arguments),
                                      "communicative": communicative})
    await write_action(conn, contract.Action(id=action_id, capability=capability, place=PLACE,
                                             arguments=dict(arguments),
                                             caused_by=trigger.id),
                       out.id, communicative=communicative)
    return out.id


async def test_a_normal_act_is_recorded_and_gets_exactly_one_receipt(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    trigger = await _trigger(aconn)

    await kernel.act_via_hooks(trigger, "telegram.say", {"text": "hai"})

    kinds = await _kinds(aconn)
    assert kinds.count("act.out") == 1
    assert kinds.count("act.result") == 1
    assert adapter.sent == ["hai"]
    assert await unresolved_actions(aconn) == []
    receipt = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert (receipt[0]["state"], receipt[0]["result"]) == ("done", {"ok": True, "text": "hai"})


async def test_a_non_communicative_act_gets_its_receipt_too(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    trigger = await _trigger(aconn)

    await kernel.act_via_hooks(trigger, "telegram.react", {"emoji": "👍"})

    kinds = await _kinds(aconn)
    assert (kinds.count("act.out"), kinds.count("act.result")) == (1, 1)
    assert await unresolved_actions(aconn) == []


async def test_two_acts_of_one_event_get_two_different_ids(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    trigger = await _trigger(aconn)

    await kernel.act_via_hooks(trigger, "telegram.react", {"emoji": "👍"})
    await kernel.act_via_hooks(trigger, "telegram.react", {"emoji": "👎"})

    assert len(set(adapter.ids)) == 2
    assert adapter.ids[0].rsplit(":", 1)[0] == adapter.ids[1].rsplit(":", 1)[0]


async def test_an_unclosed_action_goes_back_with_the_same_id_once(aconn):
    adapter = SendingAdapter()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(adapter)
    await _unclosed(aconn, capability="telegram.say", arguments={"text": "halo"},
                    communicative=True)
    assert [row["id"] for row in await unresolved_actions(aconn)] == ["job:7:12:0"]

    await kernel.recover()

    assert adapter.ids == ["job:7:12:0"]  # the same id, not a new one
    assert adapter.sent == ["halo"]
    assert await unresolved_actions(aconn) == []
    await kernel.recover()
    assert adapter.sent == ["halo"]  # a closed action is never carried twice
    assert (await _kinds(aconn)).count("act.result") == 1


async def test_an_unclosed_action_whose_tool_is_gone_gets_a_failed_receipt(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _unclosed(aconn, capability="gone.say", arguments={"text": "halo"},
                    communicative=True)

    await kernel.recover()

    receipt = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert receipt[0]["state"] == "failed"
    assert "gone.say" in receipt[0]["reason"]
    assert await unresolved_actions(aconn) == []
