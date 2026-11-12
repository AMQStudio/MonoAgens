"""The tool registry: what is present now, and the routing vocabulary of one act.

A capability descriptor keeps its frozen key `organ`; the value is the name of the adapter
or plugin that declares the capability (docs/core.md §8).
"""

from __future__ import annotations

from typing import Any

from monoagens.types import Action, Event

ROUTING = ("to", "via")


class ToolRegistry:
    """The adapters and plugins registered with the kernel and their capabilities."""

    def __init__(self) -> None:
        """Start with no tool present."""
        self.adapters: dict[str, Any] = {}
        self.capabilities: dict[str, dict[str, Any]] = {}

    def register(self, adapter: Any) -> None:
        """Register one adapter and every capability it declares."""
        name = getattr(adapter, "name", None)
        if not name:
            raise ValueError("An adapter must expose a name")
        self.adapters[name] = adapter
        for capability in getattr(adapter, "capabilities", lambda: ())():
            full = f"{name}.{capability['name']}"
            if full in self.capabilities:
                raise ValueError(f"Capability collision: {full}")
            self.capabilities[full] = {"organ": name, **capability}

    def add_capability(self, full: str, descriptor: dict[str, Any]) -> None:
        """Declare a capability the kernel dispatches for the middleware, like `self.share`."""
        if full in self.capabilities:
            raise ValueError(f"Capability collision: {full}")
        self.capabilities[full] = {"organ": full.partition(".")[0], **descriptor}

    def get(self, full: str) -> dict[str, Any] | None:
        """Return one capability's descriptor, or None when nothing declares it."""
        return self.capabilities.get(full)

    def descriptors(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors in registration order."""
        return tuple(self.capabilities.values())

    def routing_keys(self, capability: dict[str, Any]) -> tuple[str, ...]:
        """Return the arguments an act routes on, which the core strips before acting."""
        return ROUTING if capability.get("communicative") else ()

    async def act(self, event: Event, capability: str, payload: dict[str, Any], *,
                  event_id: int | None = None) -> Any:
        """Hand one action to the adapter or plugin that declared the capability."""
        declared_by, _, name = capability.partition(".")
        adapter = self.adapters.get(declared_by)
        if adapter is None:
            raise RuntimeError(f"No adapter registered for {declared_by!r}")
        action = Action(kind=name, place=event.place or "", payload=payload,
                        adapter=declared_by, event_id=event_id)
        return await adapter.act(action)
