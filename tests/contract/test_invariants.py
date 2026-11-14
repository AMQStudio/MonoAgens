"""Stateful invariants over the contract (Task 6.3).

Two machines, each checking one promise the rest of the suite can only sample:

* an audience machine — however places, people and audiences interleave, a context item is
  visible only where the audience admits the item's own audience;
* a receipt machine — whatever the retries and repeats, one action id gets exactly one receipt
  and the body carries it out at most once.

Both run the real code (`middleware.privacy`, `adapters.support`). The audience machine owns one
event loop and one connection, so every rule talks to the same database session.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from functools import partial
from typing import Any

from hypothesis import HealthCheck, settings
from hypothesis import strategies as st
from hypothesis.stateful import RuleBasedStateMachine, invariant, precondition, rule
from hypothesis.stateful import run_state_machine_as_test

from monoagens.adapters import support
from monoagens.contract import Action, Audience, Percept, Receipt, audience_event
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware import privacy

START = datetime(2026, 11, 14, 9, 0, tzinfo=timezone.utc)
PEOPLE = ("person:owner", "person:budi", "person:ani", "person:citra")
PLACES = ("space:cli", "space:wa:g1@g.us", "space:wa:g2@g.us", "space:wa:dm:budi")

MACHINE = settings(max_examples=20, stateful_step_count=8, deadline=None,
                   suppress_health_check=[HealthCheck.function_scoped_fixture])


class AudienceMachine(RuleBasedStateMachine):
    """Places report audiences, percepts land in them, and the one privacy rule is checked."""

    def __init__(self, loop: asyncio.AbstractEventLoop, conn: Any) -> None:
        super().__init__()
        self.loop = loop
        self.conn = conn
        # Every example starts from an empty Log: the model below is the Log's own history, so a
        # row left by the previous example would make the model lie.
        loop.run_until_complete(conn.execute(
            "TRUNCATE TABLE events, state, expectations, memories CASCADE"))
        self.clock = FakeClock(START)
        self.history: dict[str, list[tuple[int, frozenset[str] | None]]] = {}
        self.sources: list[tuple[int, str]] = []

    @rule(place=st.sampled_from(PLACES),
          audience=st.sets(st.sampled_from(PEOPLE), max_size=4))
    def report_audience(self, place: str, audience: set[str]) -> None:
        """Report a place's audience, as an adapter does when it first sees it or it changes."""
        event = audience_event(place, Audience(frozenset(audience)), ts=self.clock.now(),
                               body="chat")
        stored = self.loop.run_until_complete(append_event(
            self.conn, ts=event.ts, kind=event.kind, place=place, body="chat",
            payload=dict(event.payload)))
        # An empty report is the contract's "nobody could be listed": unknown, not private.
        self.history.setdefault(place, []).append(
            (int(stored.id), frozenset(audience) or None))

    @rule(place=st.sampled_from(PLACES), actor=st.sampled_from(PEOPLE))
    @precondition(lambda self: bool(self.history))
    def perceive(self, place: str, actor: str) -> None:
        """One percept in a place: it is a source, and what a context item would quote."""
        percept = Percept(actor=actor, place=place, occurred_at=self.clock.now(),
                          description="hello there")
        stored = self.loop.run_until_complete(append_event(
            self.conn, ts=self.clock.now(), kind="percept", who=percept.actor,
            place=percept.place, body="chat", payload={"data": {}},
            description=percept.description))
        self.sources.append((int(stored.id), place))

    def audience_at(self, place: str, event_id: int | None,
                    ) -> frozenset[str] | None:
        """Return who perceived a place at one point in the Log, or None when nobody said."""
        known = None
        for reported_at, members in self.history.get(place, ()):
            if event_id is None or reported_at <= event_id:
                known = members
        return known

    @invariant()
    def no_item_leaves_its_audience(self) -> None:
        """A source is visible from a context place only when that audience admits it.

        The item's audience is the one its place reported before the item itself; the context's
        is the one its place reports now. That is the rule the middleware applies, so the model
        here is the Log's own history, not a guess about it.
        """
        for source_id, source_place in self.sources:
            for context in PLACES:
                target = self.audience_at(context, None)
                said = self.audience_at(source_place, source_id)
                allowed = self.loop.run_until_complete(privacy.visible(
                    self.conn, context=context, source=source_place, before=source_id))
                if allowed != privacy.admits(target, said):
                    detail = self.loop.run_until_complete(_detail(
                        self.conn, context, source_place, source_id))
                    raise AssertionError(
                        (source_place, context, said, target, allowed, detail))


class _CountingBody:
    """A body that counts how many times each action id is really carried out."""

    name = "counting"

    def __init__(self) -> None:
        self.calls: list[str] = []
        self.fail_next = 0

    async def act(self, action: Action) -> Receipt:
        """Carry the action out once per id, through the one shared wrapper."""

        async def function() -> dict[str, str]:
            if self.fail_next:
                self.fail_next -= 1
                raise RuntimeError("the transport blinked")
            self.calls.append(str(action.id))
            return {"sent": str(action.id)}

        return await support.answer(self, action, capability="counting.say",
                                    function=function)


class ReceiptMachine(RuleBasedStateMachine):
    """One action id: one receipt, and the body carries it out at most once."""

    def __init__(self) -> None:
        super().__init__()
        self.body = _CountingBody()
        self.receipts: dict[str, Receipt] = {}
        self.loop = asyncio.new_event_loop()

    @rule(action_id=st.integers(min_value=1, max_value=3), fail=st.booleans())
    def send(self, action_id: int, fail: bool) -> None:
        """Send one action, sometimes with a transport that fails its first attempt."""
        self.body.fail_next = 1 if fail else 0
        action = Action(id=f"act:{action_id}", capability="say", place="space:cli",
                        arguments={"text": "hi"})
        receipt = self.loop.run_until_complete(self.body.act(action))
        first = self.receipts.get(str(action.id))
        if first is None:
            self.receipts[str(action.id)] = receipt
        else:
            assert receipt == first, "a repeated action id answered a different receipt"

    def teardown(self) -> None:
        """Close the loop this example opened."""
        self.loop.close()

    @invariant()
    def one_carry_per_id(self) -> None:
        """The body carried each id at most once, whatever the repeats and the failures."""
        carried = self.body.calls
        assert len(carried) == len(set(carried)), carried


async def _detail(conn, context: str, source: str, source_id: int) -> tuple:
    """Return what the middleware itself sees, for a failing example."""
    return (await privacy.audience(conn, context), await privacy.audience(conn, source),
            await privacy.source_audience(conn, source_id))


def test_no_context_item_leaves_its_audience(database_url: str) -> None:
    """The audience rule holds for every interleaving of places, people and audiences."""
    import psycopg

    loop = asyncio.new_event_loop()
    conn = loop.run_until_complete(psycopg.AsyncConnection.connect(database_url, autocommit=True))
    try:
        run_state_machine_as_test(partial(AudienceMachine, loop, conn), settings=MACHINE)
    finally:
        loop.run_until_complete(conn.close())
        loop.close()


def test_one_receipt_per_action_id() -> None:
    """A repeated id never re-sends and never answers a second, different receipt."""
    run_state_machine_as_test(ReceiptMachine, settings=MACHINE)
