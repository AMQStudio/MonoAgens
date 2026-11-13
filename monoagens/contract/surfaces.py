"""What a body must provide and what it may provide (contract.md §4.1)."""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from typing import Any, Protocol

REQUIRED_MEMBERS = ("name", "run", "stop", "capabilities", "act", "prompt_section",
                    "self_actor")
OPTIONAL_SURFACES = ("history", "fetch", "render", "ask")

_MISSING = object()
_DATA_MEMBER = "name"


class Body(Protocol):
    """One body: an adapter or a plugin, translated into the contract."""

    name: str

    async def run(self, put: Callable[[Any], Any]) -> None:
        """Sense the world and put percepts into the queue until `stop`."""
        ...

    async def stop(self) -> None:
        """Stop sensing and release every resource."""
        ...

    def capabilities(self) -> Sequence[Mapping[str, Any]]:
        """Return this body's own capabilities, each with its descriptor."""
        ...

    async def act(self, action: Any) -> Any:
        """Carry out one action and answer with one receipt."""
        ...

    def prompt_section(self) -> str:
        """Return what reasoning should know about this body, at most 1,200 characters."""
        ...

    def self_actor(self) -> str:
        """Return the actor id this body acts as."""
        ...


class History(Protocol):
    """Older events of a place, for back-fill and import."""

    async def history(self, place: str, before: str, limit: int) -> Sequence[Any]:
        """Return up to `limit` percepts older than `before` in `place`, newest first."""
        ...


class Fetch(Protocol):
    """The bytes of a deferred attachment."""

    async def fetch(self, locator: str) -> bytes:
        """Return the bytes one locator names."""
        ...


class Render(Protocol):
    """A lifecycle event, drawn the body's own way."""

    async def render(self, event: Any) -> None:
        """Show one lifecycle event, or nothing."""
        ...


class Ask(Protocol):
    """A question with choices, asked the body's own way."""

    async def ask(self, place: str, question: str, choices: Sequence[str]) -> str:
        """Ask and return the body's ref for the question."""
        ...


def missing_members(body: object) -> tuple[str, ...]:
    """Return every member of §4.1 this body lacks or cannot be called."""
    found = []
    for member in REQUIRED_MEMBERS:
        value = getattr(body, member, _MISSING)
        if value is _MISSING:
            found.append(member)
        elif member != _DATA_MEMBER and not callable(value):
            found.append(member)
    return tuple(found)


def surfaces(body: object) -> tuple[str, ...]:
    """Return the optional surfaces this body provides, in contract order."""
    return tuple(name for name in OPTIONAL_SURFACES
                 if callable(getattr(body, name, None)))
