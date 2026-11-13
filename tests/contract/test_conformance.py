"""The one conformance suite every body passes (contract.md §7, §8).

It runs over the five fake bodies here and, from Part 4 on, over every adapter: the surface
of §4.1 exists, every percept matches §4.2, every action gets exactly one receipt, and a
repeated action id is answered from the body's own memory instead of being carried out twice.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from datetime import datetime
from typing import Any

import pytest

from monoagens.contract import (OPTIONAL_SURFACES, RECEIPT_STATES, REQUIRED_MEMBERS, Percept,
                                composing, descriptor_errors, missing_members, surfaces)
from tests.bodies.chat import ChatBody
from tests.bodies.game import GameBody
from tests.bodies.robot import RobotBody
from tests.bodies.sensor import SensorBody
from tests.bodies.workspace_app import WorkspaceBody

BODIES: tuple[tuple[str, Callable[[], Any], tuple[str, ...]], ...] = (
    ("chat", ChatBody, ("render", "ask")),
    ("robot", RobotBody, ()),
    ("workspace_app", WorkspaceBody, ()),
    ("sensor", SensorBody, ()),
    ("game", GameBody, ()),
)
IDS = [name for name, _, _ in BODIES]


async def _start(factory: Callable[[], Any]) -> tuple[Any, list[Any], asyncio.Task[None]]:
    """Run one body with a sink for what it puts, and return it, the sink and the task."""
    body = factory()
    put: list[Any] = []

    async def sink(item: Any) -> None:
        put.append(item)

    task = asyncio.create_task(body.run(sink))
    await asyncio.wait_for(body.ready(), timeout=5)
    await body.demo()
    return body, put, task


async def _stop(body: Any, task: asyncio.Task[None]) -> None:
    """Stop one body and wait for it to finish sensing."""
    await body.stop()
    await asyncio.wait_for(task, timeout=5)


@pytest.mark.parametrize(("name", "factory", "optional"), BODIES, ids=IDS)
async def test_the_body_surface_is_complete(name: str, factory: Callable[[], Any],
                                            optional: tuple[str, ...]) -> None:
    """Every member of §4.1 exists, the descriptor validates, the surfaces are as declared."""
    body = factory()
    assert body.name == name
    assert missing_members(body) == ()
    assert all(hasattr(body, member) for member in REQUIRED_MEMBERS)
    assert surfaces(body) == optional
    assert optional == tuple(item for item in OPTIONAL_SURFACES if item in optional)
    assert 0 < len(body.prompt_section()) <= 1200
    assert body.self_actor()
    assert body.capabilities()
    for capability in body.capabilities():
        assert descriptor_errors(body.name, capability) == ()


@pytest.mark.parametrize(("name", "factory", "optional"), BODIES, ids=IDS)
async def test_every_percept_matches_the_contract(name: str, factory: Callable[[], Any],
                                                  optional: tuple[str, ...]) -> None:
    """What a body puts is a percept of §4.2, and it reports who can see its places."""
    body, put, task = await _start(factory)
    try:
        percepts = [item for item in put if isinstance(item, Percept)]
        assert percepts, f"{name} senses nothing"
        for percept in percepts:
            assert isinstance(percept.occurred_at, datetime)
            assert percept.actor
            assert percept.place
            assert percept.description
            for value in percept.features.values():
                assert isinstance(value, (int, float)) and not isinstance(value, bool)
        audiences = [item for item in put if not isinstance(item, Percept)]
        assert audiences, f"{name} never says who can see its places"
        assert body.members
    finally:
        await _stop(body, task)


@pytest.mark.parametrize(("name", "factory", "optional"), BODIES, ids=IDS)
async def test_every_action_gets_exactly_one_receipt(name: str, factory: Callable[[], Any],
                                                     optional: tuple[str, ...]) -> None:
    """One receipt per action, of a defined state, and never a second send for one id."""
    body, _, task = await _start(factory)
    try:
        plan = body.plan()
        assert plan, f"{name} offers no action to try"
        for action in plan:
            receipt = await body.act(action)
            assert receipt.state in RECEIPT_STATES
        assert len(body.receipts) == len(plan)
        assert len(body.sent) == len(plan)
        again = await body.act(plan[0])
        assert again == body.receipts[plan[0].id]
        assert len(body.sent) == len(plan)
    finally:
        await _stop(body, task)


@pytest.mark.parametrize(("name", "factory", "optional"), BODIES, ids=IDS)
async def test_the_optional_surfaces_work_where_present(
        name: str, factory: Callable[[], Any], optional: tuple[str, ...]) -> None:
    """A body with `ask` asks and returns a ref; one with `render` shows the event."""
    body, _, task = await _start(factory)
    try:
        place = next(iter(body.members))
        if "ask" in optional:
            ref = await body.ask(place, "which one?", ("a", "b"))
            assert ref
            assert body.asked[-1][0] == place
        else:
            assert not hasattr(body, "ask")
        if "render" in optional:
            await body.render(composing(place))
            assert body.rendered[-1].place == place
    finally:
        await _stop(body, task)
