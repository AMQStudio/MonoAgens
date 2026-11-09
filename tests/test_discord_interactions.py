"""ADR-0018 P0: Discord interactions are acknowledged and carry a real payload."""

import asyncio
import contextlib
from types import SimpleNamespace

from monoagens.gateways.discord import ingress
from monoagens.gateways.discord.transport import (
    DiscordPyTransport,
    Handlers,
    dispatch_interaction,
    interaction_payload,
)


class FakeResponse:
    """`interaction.response`: records the defer kwargs in the shared journal."""

    def __init__(self, journal, fail=False):
        self.journal = journal
        self.fail = fail

    async def defer(self, **kwargs):
        self.journal.append(("defer", kwargs))
        if self.fail:
            raise RuntimeError("already answered")


class FakeInteraction:
    """The discord.py attributes the transport reads, and nothing else."""

    def __init__(
        self,
        *,
        kind=3,
        interaction_id="i1",
        channel_id="77",
        parent_id=None,
        guild_id="12",
        user_id="222",
        user_name="Budi",
        data=None,
        message_id="500",
        journal=None,
        fail_defer=False,
    ):
        self.journal = journal if journal is not None else []
        self.type = kind
        self.id = interaction_id
        self.data = data if data is not None else {"custom_id": "cmd:/approve #42", "values": []}
        self.user = SimpleNamespace(id=user_id, name=user_name)
        self.channel = SimpleNamespace(id=channel_id, parent_id=parent_id)
        self.guild = SimpleNamespace(id=guild_id) if guild_id is not None else None
        self.message = SimpleNamespace(id=message_id)
        self.response = FakeResponse(self.journal, fail=fail_defer)

    async def delete_original_response(self):
        self.journal.append(("delete", {}))


def handlers_for(journal, *, raises=False):
    seen = []

    async def ready():
        journal.append(("ready", {}))

    async def message(payload):
        journal.append(("message", dict(payload)))

    async def interaction(payload):
        journal.append(("handler", dict(payload)))
        seen.append(ingress.interaction_percept(payload))
        if raises:
            raise RuntimeError("engine failed")

    return Handlers(ready=ready, message=message, interaction=interaction), seen


def slash(callback, interaction, argument=""):
    asyncio.run(callback(interaction, argument))


def test_a_slash_command_is_acknowledged_before_the_handler_and_cleaned_up(tmp_path) -> None:
    journal = []
    handlers, seen = handlers_for(journal)
    interaction = FakeInteraction(kind=2, data={"name": "status", "options": []}, journal=journal)
    callback = DiscordPyTransport("token")._slash_callback("status", handlers)

    slash(callback, interaction, "now")

    assert [step for step, _ in journal] == ["defer", "handler", "delete"]
    assert journal[0][1] == {"ephemeral": True}
    found = seen[0]
    assert found is not None
    percept, interaction_id, message_id = found
    assert percept.payload.text == "/status now"
    assert percept.actor == "222"
    assert percept.hints["group"] is True
    assert percept.thread == "77"
    assert interaction_id == "i1"
    assert message_id == "i1"


def test_the_slash_placeholder_is_removed_even_when_the_handler_raises() -> None:
    journal = []
    handlers, _ = handlers_for(journal, raises=True)
    interaction = FakeInteraction(kind=2, data={"name": "status", "options": []}, journal=journal)
    callback = DiscordPyTransport("token")._slash_callback("status", handlers)

    with contextlib.suppress(RuntimeError):
        slash(callback, interaction, "now")

    assert [step for step, _ in journal] == ["defer", "handler", "delete"]


def test_a_slash_command_in_a_dm_is_marked_private() -> None:
    journal = []
    handlers, seen = handlers_for(journal)
    interaction = FakeInteraction(
        kind=2,
        guild_id=None,
        channel_id="44",
        data={"name": "help", "options": []},
        journal=journal,
    )
    callback = DiscordPyTransport("token")._slash_callback("help", handlers)

    slash(callback, interaction)

    percept = seen[0][0]
    assert percept.payload.text == "/help"
    assert percept.hints["dm"] is True
    assert percept.hints["group"] is False


def test_a_component_click_defers_without_ephemeral_then_handles() -> None:
    journal = []
    handlers, seen = handlers_for(journal)
    interaction = FakeInteraction(kind=3, journal=journal)

    asyncio.run(dispatch_interaction(interaction, handlers))

    assert [step for step, _ in journal] == ["defer", "handler"]
    assert journal[0][1] == {}
    assert seen[0] is not None
    assert seen[0][0].payload.text == "/approve #42"


def test_unsupported_interactions_are_ignored() -> None:
    journal = []
    handlers, _ = handlers_for(journal)

    for kind in (1, 5):
        interaction = FakeInteraction(kind=kind, journal=journal)
        assert interaction_payload(interaction) is None
        asyncio.run(dispatch_interaction(interaction, handlers))

    assert journal == []


def test_a_failing_defer_never_blocks_the_handler() -> None:
    journal = []
    handlers, seen = handlers_for(journal)
    interaction = FakeInteraction(kind=3, journal=journal, fail_defer=True)

    asyncio.run(dispatch_interaction(interaction, handlers))

    assert [step for step, _ in journal] == ["defer", "handler"]
    assert seen[0] is not None
    assert seen[0][0].payload.text == "/approve #42"


def test_the_event_path_leaves_slash_commands_to_the_slash_callback() -> None:
    """discord.py fires `on_interaction` for slash commands too; only the callback may ack them."""
    journal = []
    handlers, _ = handlers_for(journal)
    interaction = FakeInteraction(kind=2, journal=journal)

    asyncio.run(dispatch_interaction(interaction, handlers))

    assert journal == []
