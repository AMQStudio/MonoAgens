"""Shared mechanics for the five fake bodies of contract.md §8.

A fake body senses only when a test feeds it, answers one receipt per action id, and never
looks at the clock: every perceived time is passed in, so a test is reproducible.
"""

from __future__ import annotations

import asyncio
from collections.abc import Iterable, Mapping, Sequence
from datetime import datetime
from typing import Any

from monoagens.contract import Action, Audience, Percept, Receipt, audience_event

T0 = datetime(2026, 11, 14, 9, 0, 0)
"""The one perceived time the fake bodies use unless a test passes another."""


class FakeBody:
    """One body on the contract surface, with the extra door a test drives it through."""

    name = ""
    section = ""
    actor = ""
    declared: tuple[Mapping[str, Any], ...] = ()

    def __init__(self) -> None:
        """Start stopped, with nothing perceived and nothing sent."""
        self.percepts: list[Percept] = []
        self.audiences: list[Any] = []
        self.sent: list[Action] = []
        self.receipts: dict[str, Receipt] = {}
        self.members: dict[str, set[str]] = {}
        self.rendered: list[Any] = []
        self.asked: list[tuple[str, str, tuple[str, ...]]] = []
        self._put: Any = None
        self._ready = asyncio.Event()
        self._stopped = asyncio.Event()

    # ----------------------------------------------------------- the body surface

    async def run(self, put: Any) -> None:
        """Keep sensing until `stop`; a test feeds the percepts through `emit`."""
        self._put = put
        self._ready.set()
        await self._stopped.wait()

    async def stop(self) -> None:
        """Stop sensing."""
        self._stopped.set()

    def capabilities(self) -> Sequence[Mapping[str, Any]]:
        """Return this body's own capabilities."""
        return self.declared

    def prompt_section(self) -> str:
        """Return what reasoning should know about this body."""
        return self.section

    def self_actor(self) -> str:
        """Return the actor this body acts as."""
        return self.actor

    async def act(self, action: Action) -> Receipt:
        """Carry out one action, once per id, and answer its receipt."""
        found = self.receipts.get(action.id)
        if found is not None:
            return found
        receipt = self.carry(action)
        self.receipts[action.id] = receipt
        self.sent.append(action)
        return receipt

    def carry(self, action: Action) -> Receipt:
        """Carry out one action for real; each body decides how."""
        raise NotImplementedError

    # ----------------------------------------------------------- the test's door

    async def ready(self) -> None:
        """Wait until `run` has been entered and can take a percept."""
        await self._ready.wait()

    async def emit(self, percept: Percept) -> None:
        """Put one percept into the queue, as the body's own sensing would."""
        self.percepts.append(percept)
        await self._put(percept)

    async def announce(self, place: str, actors: Iterable[str], *,
                       unknown: bool = False, at: datetime = T0) -> None:
        """Report who can perceive a place, as a body does when it first sees it."""
        self.members[place] = set(actors)
        event = audience_event(place, Audience(frozenset(self.members[place]), unknown),
                               ts=at, body=self.name)
        self.audiences.append(event)
        await self._put(event)

    def plan(self) -> Sequence[Action]:
        """Return the actions a test may hand this body, including one that is refused."""
        return ()
