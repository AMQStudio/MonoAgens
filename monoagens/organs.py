"""Organs present now, the capabilities they declare, and action dispatch."""

from __future__ import annotations

from typing import Any

from monoagens.types import Action, Event


class OrganRegistry:
    """The bodies registered with the kernel and their capability descriptors."""

    def __init__(self) -> None:
        """Start with no organ present."""
        self.bodies: dict[str, Any] = {}
        self.capabilities: dict[str, dict[str, Any]] = {}

    def register(self, body: Any) -> None:
        """Register one body and every capability it declares."""
        name = getattr(body, "name", None)
        if not name:
            raise ValueError("A body must expose a name")
        self.bodies[name] = body
        for capability in getattr(body, "capabilities", lambda: ())():
            full = f"{name}.{capability['name']}"
            if full in self.capabilities:
                raise ValueError(f"Capability collision: {full}")
            self.capabilities[full] = {"organ": name, **capability}

    def descriptors(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors in registration order."""
        return tuple(self.capabilities.values())

    async def act(self, event: Event, capability: str, payload: dict[str, Any], *,
                  event_id: int | None = None) -> Any:
        """Hand one action to the organ that declared the capability."""
        organ, _, name = capability.partition(".")
        body = self.bodies.get(organ)
        if body is None:
            raise RuntimeError(f"No body registered for {organ!r}")
        action = Action(kind=name, place=event.place or "", payload=payload, body=organ,
                        event_id=event_id)
        return await body.act(action)
