"""The soul's own memory of a person: what it learned about a source, under that id."""

from __future__ import annotations

from monoagens.infra.store import read_state


async def closeness(conn, who: str | None) -> float:
    """Return the closeness the soul learned towards one person, clamped to [0, 1].

    The key `state: person:<id>` is frozen data (docs/core.md §3): the name belongs to the
    Log, not to any layer, and the value is the soul's own memory of a source, so it stays in
    the core while the middleware only adds the card that shows it.
    """
    if not who:
        return 0.0
    state = await read_state(conn, f"person:{who}") or {}
    return max(0.0, min(1.0, float(state.get("closeness") or 0.0)))
