"""A test adapter that loads from config.yaml alone (1.5), with its own required variable."""

from __future__ import annotations

from typing import Any

REQUIRES = ("ECHO_GREETING",)


class EchoAdapter:
    """A test organ: it greets with its option and echoes what it is asked to say."""

    name = "echo"
    shows = ("typing",)

    def __init__(self, options: dict[str, Any], env: dict[str, str]) -> None:
        """Store the option and the required variable the loader resolved."""
        self.options = dict(options)
        self.greeting = str(env.get("ECHO_GREETING") or "")
        self.said: list[str] = []

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare one talking capability, as an adapter's say is."""
        return ({"name": "say", "description": "Echo a line.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},)

    async def run(self, put) -> None:
        """A test adapter is driven by the test, never by the network."""
        return None

    async def act(self, action) -> dict[str, Any]:
        """Record one line and report it."""
        self.said.append(str(action.arguments.get("text") or ""))
        return {"echo": self.said[-1], "greeting": self.greeting}


def create(options: dict[str, Any], env: dict[str, str]) -> EchoAdapter:
    """Build the adapter from config.yaml's options and the environment."""
    return EchoAdapter(options, env)
