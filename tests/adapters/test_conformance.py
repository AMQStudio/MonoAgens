"""Every adapter on the one conformance suite, and fakes taken from the real clients (4.5, 4.6).

The five fake bodies already prove the contract surface; this file runs the same checks over the
four adapters this repository ships, and builds the two gateway fakes from the real libraries
(`create_autospec`), so a method that the libraries do not have cannot be invented by a test.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any
from unittest.mock import create_autospec

import discord
import pytest

from monoagens.adapters import support
from monoagens.adapters.cli import CLIBody
from monoagens.adapters.discord import DiscordBody
from monoagens.adapters.telegram import TelegramBody
from monoagens.adapters.whatsapp import WhatsAppBody
from monoagens.contract import (OPTIONAL_SURFACES, Action, descriptor_errors,
                                missing_members, surfaces)
from monoagens.infra.clock import Clock, FakeClock
from tests.bodies.base import T0

ADAPTERS: tuple[tuple[str, Callable[[], Any], tuple[str, ...]], ...] = (
    ("cli", lambda: CLIBody("/tmp/monoagens-conformance.sock", clock=FakeClock(T0)),
     ("render", "ask")),
    ("telegram", lambda: TelegramBody("token", clock=FakeClock(T0)), ("render", "ask")),
    ("discord", lambda: DiscordBody("token", clock=FakeClock(T0)), ("history", "render", "ask")),
    ("whatsapp", lambda: WhatsAppBody(clock=FakeClock(T0)), ("render", "ask")),
)
IDS = [name for name, _, _ in ADAPTERS]


@pytest.mark.parametrize(("name", "factory", "expected"), ADAPTERS, ids=IDS)
async def test_every_adapter_answers_the_contract_surface(
        name: str, factory: Callable[[], Any], expected: tuple[str, ...]) -> None:
    """The required members exist, the surfaces are the ones promised, each capability validates."""
    body = factory()
    assert body.name == name
    assert missing_members(body) == ()
    assert surfaces(body) == expected
    assert set(surfaces(body)) <= set(OPTIONAL_SURFACES)
    assert isinstance(body.self_actor(), str) and body.self_actor()
    assert 0 < len(body.prompt_section()) <= 1200
    for capability in body.capabilities():
        errors = descriptor_errors(name, capability, require_facts=True)
        assert not errors, errors[0]
        assert capability["name"] and capability["permission"].startswith(name + ".")


@pytest.mark.parametrize(("name", "factory", "expected"), ADAPTERS, ids=IDS)
async def test_one_action_id_gets_one_receipt_and_never_a_second_send(
        name: str, factory: Callable[[], Any], expected: tuple[str, ...]) -> None:
    """A repeated id is answered from the body's own memory, never carried out twice."""
    del expected
    body = factory()
    action = Action(id=f"{name}:conf:1", capability="no-such-capability", place="space:x",
                    arguments={})

    first = await body.act(action)
    again = await body.act(action)

    assert first.state == "failed"
    assert again is first or again == first


def test_the_discord_fake_is_the_library_itself() -> None:
    """A fake client has only the members discord.py really has, as an autospec declares them."""
    client = create_autospec(discord.Client, instance=True)

    assert hasattr(client, "get_channel") and hasattr(client, "fetch_channel")
    assert not hasattr(client, "send_everything")


def test_the_whatsapp_fake_is_the_library_itself() -> None:
    """The neonize client's own methods (send_message, get_joined_groups) are what a body calls."""
    from neonize.client import NewClient

    client = create_autospec(NewClient, instance=True)

    assert hasattr(client, "send_message")
    assert not hasattr(client, "say")


def test_an_adapter_records_what_it_would_send_in_shadow_mode(tmp_path, monkeypatch) -> None:
    """`dry_run` answers `done` and records the action, and `record` writes the line (§4.7)."""
    monkeypatch.chdir(tmp_path)
    body = CLIBody("/tmp/monoagens-conformance.sock", clock=Clock())
    support.options(body, {"record": True, "dry_run": True})

    receipt = _run(body.act(Action(id="cli:dry:1", capability="say", place="space:cli",
                                   arguments={"text": "halo"})))

    assert receipt.state == "done" and receipt.result == {"dry_run": True}
    lines = (tmp_path / "var/recordings/cli.jsonl").read_text(encoding="utf-8").splitlines()
    assert any('"dry_run"' in line and "cli:dry:1" in line for line in lines)


def _run(coroutine: Any) -> Any:
    """Run one coroutine to completion (these bodies answer without a loop of their own)."""
    import asyncio

    return asyncio.run(coroutine)
