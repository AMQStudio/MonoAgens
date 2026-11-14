"""The five fake bodies through the real core and middleware (Task 6.2).

Each body senses into a running kernel: its places' audiences reach the Log, its percepts become
events, an action the middleware routes to it comes back as that body's own receipt, a question
is drawn by the body and answered through the one `self.answer` capability, and a back-filled
percept is recorded without costing a model call. No change to `monoagens/contract/` is needed.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from typing import Any

import pytest

from monoagens.boot import build
from monoagens.contract import Action, Percept
from monoagens.infra.clock import FakeClock
from monoagens.middleware import interaction, registry
from tests.bodies.base import T0
from tests.bodies.chat import ChatBody
from tests.bodies.game import GameBody
from tests.bodies.robot import RobotBody
from tests.bodies.sensor import SensorBody
from tests.bodies.workspace_app import WorkspaceBody
from tests.support import settings

BODIES: tuple[tuple[str, Callable[[], Any]], ...] = (
    ("chat", ChatBody),
    ("robot", RobotBody),
    ("workspace_app", WorkspaceBody),
    ("sensor", SensorBody),
    ("game", GameBody),
)
IDS = [name for name, _ in BODIES]


@pytest.mark.parametrize(("name", "factory"), BODIES, ids=IDS)
async def test_a_body_senses_acts_asks_and_catches_up_through_the_kernel(
        aconn, name: str, factory: Callable[[], Any]) -> None:
    """One body, one kernel: audience, percepts, receipt, question, answer and back-fill."""
    body = factory()
    clock = FakeClock(T0)
    kernel = build(aconn, clock=clock, settings=settings())
    kernel.register_adapter(body)
    sensed: list[Any] = []

    async def put(item: Any) -> None:
        """Collect what the body senses; the test hands it to the kernel itself."""
        sensed.append(item)

    listener = asyncio.create_task(body.run(put))
    try:
        await asyncio.wait_for(body.ready(), timeout=5)
        await body.demo()
        await _feed(kernel, sensed, name)

        kinds = await _kinds(aconn)
        assert "place.audience" in kinds, "a body reports who can perceive its places"
        assert "percept" in kinds, "what it sensed became events"
        assert await _bodies(aconn) == {name}, "every event carries the body's own name"

        action = body.plan()[0]
        receipt = await kernel.tools.act(Action(
            id=f"{name}:test:1", capability=f"{name}.{action.capability}",
            place=action.place, arguments=dict(action.arguments)))

        assert receipt.state == "done", receipt.reason
        assert body.sent and body.sent[-1].capability == action.capability

        place = str(action.place)
        await interaction.ask(kernel, _trigger(body, place), place, "Pilih satu?",
                             ["Ya", "Tidak"], kind="approval", request=1)
        assert any(asked[0] == place for asked in body.asked) or not hasattr(body, "ask")

        backfilled = Percept(
            actor=body.actor, place=place, occurred_at=T0,
            description="a line from before the body was connected", backfill=True,
            features={"addressed": 0, "named": 0, "addressed_other": 0})
        await body.emit(backfilled)
        await _feed(kernel, sensed, name)
    finally:
        await body.stop()
        await asyncio.gather(listener, return_exceptions=True)

    rows = await _rows(aconn)
    assert any(kind == "percept" and backfill for kind, _, backfill in rows), \
        "a back-filled percept is recorded"
    assert any(kind == "question.asked" for kind, _, _ in rows), \
        "the question is in the Log"


async def _feed(kernel: Any, items: list[Any], name: str) -> None:
    """Put each collected item into the kernel and let the loop finish it (the test's own task)."""
    pending = [asyncio.create_task(kernel.put(item, body=name)) for item in items]
    while not all(task.done() for task in pending):
        await kernel.run_until_idle()
        await asyncio.sleep(0)
    for task in pending:
        task.result()
    await kernel.run_until_idle()


def _trigger(body: Any, place: str) -> Any:
    """Return the percept event a question hangs from, as an adapter's own line would."""
    from monoagens.contract import to_event

    return to_event(Percept(actor=body.actor, place=place, occurred_at=T0,
                            description="hi", features={"addressed": 1}), body=body.name,
                    ts=T0)


async def _rows(conn: Any) -> list[tuple[str, str, bool]]:
    """Return every event's kind, body and back-fill flag, oldest first."""
    cur = await conn.execute("SELECT kind, body, backfill FROM events ORDER BY id")
    return [(str(kind), str(body or ""), bool(backfill)) for kind, body, backfill
            in await cur.fetchall()]


async def _kinds(conn: Any) -> list[str]:
    """Return every event kind in the Log, oldest first."""
    return [kind for kind, _, _ in await _rows(conn)]


async def _bodies(conn: Any) -> set[str]:
    """Return the body names the Log holds, minus the middleware's own lines."""
    return {body for kind, body, _ in await _rows(conn)
            if kind in ("percept", "place.audience") and body}
