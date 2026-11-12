"""A test adapter that forgets one required part, so the loader has to refuse it (1.5)."""

from __future__ import annotations

from typing import Any


class BrokenAdapter:
    """An adapter with capabilities and act, but no run: it could never deliver a percept."""

    name = "broken"

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare nothing."""
        return ()

    async def act(self, action) -> None:
        """Do nothing."""
        return None


def create(options: dict[str, Any], env: dict[str, str]) -> BrokenAdapter:
    """Build the broken adapter."""
    return BrokenAdapter()
