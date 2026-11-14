"""Adapters through the same loader: `create(options, env)`, required and optional parts."""

from __future__ import annotations

import pathlib

import pytest

from monoagens.adapters import cli as cli_adapter
from monoagens.adapters import telegram as telegram_adapter
from monoagens.boot import build
from monoagens.infra.clock import FakeClock
from monoagens.middleware.loader import Loader
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent


def test_a_test_adapter_loads_from_config_alone():
    loaded = Loader(ROOT, env={"ECHO_GREETING": "hai"}).load(settings(
        bodies={"echo": {"enabled": True, "module": "tests.fixtures.echo_adapter",
                         "pitch": 2}}))

    adapter = loaded.adapters[0]
    assert adapter.name == "echo"
    assert adapter.options == {"enabled": True, "pitch": 2}
    assert adapter.greeting == "hai"
    assert "adapter echo: surfaces none" in loaded.report.lines


async def test_the_loaded_test_adapter_is_registered(aconn):
    loader = Loader(ROOT, env={"ECHO_GREETING": "hai"})
    loaded = loader.load(settings(bodies={"echo": {"enabled": True,
                                                  "module": "tests.fixtures.echo_adapter"}}))
    kernel = build(aconn, clock=FakeClock(), settings=settings())

    await loader.install(kernel, loaded)

    assert kernel.tools.get("echo.say")["organ"] == "echo"


async def test_a_loaded_adapter_gets_its_cursor_reader(aconn):
    loader = Loader(ROOT, env={"TELEGRAM_BOT_TOKEN": "t"})
    loaded = loader.load(settings(bodies={"telegram": {"enabled": True}}))
    kernel = build(aconn, clock=FakeClock(), settings=settings())

    await loader.install(kernel, loaded)

    assert await kernel.tools.adapters["telegram"].get_cursor() is None


def test_a_missing_required_variable_fails_at_boot():
    with pytest.raises(ValueError, match="echo: ECHO_GREETING is not set"):
        Loader(ROOT, env={}).load(settings(
            bodies={"echo": {"enabled": True, "module": "tests.fixtures.echo_adapter"}}))


def test_a_missing_required_part_fails_with_its_name():
    with pytest.raises(ValueError, match="adapter broken: missing required member 'run'"):
        Loader(ROOT).load(settings(
            bodies={"broken": {"enabled": True, "module": "tests.fixtures.broken_adapter"}}))


def test_a_disabled_adapter_is_off_and_need_not_exist():
    loaded = Loader(ROOT).load(settings(bodies={"ghost": {"enabled": False}}))

    assert loaded.adapters == ()
    assert "adapter ghost: off" in loaded.report.lines


def test_a_capability_without_a_permission_fails_at_boot():
    """Every entry declares its permission; the boot report says which one is wrong (§3)."""
    body = cli_adapter.create({}, {})
    body.capabilities = lambda: ({"name": "say", "description": "Say a line.",
                                  "schema": {"type": "object", "properties": {
                                      "text": {"type": "string"}}},
                                  "facts": ()},)

    with pytest.raises(ValueError, match="adapter cli: say: permission is required"):
        Loader._check_adapter("cli", body, __import__(
            "monoagens.middleware.loader", fromlist=["BootReport"]).BootReport())


def test_the_telegram_factory_reads_the_token_from_the_environment():
    adapter = telegram_adapter.create({"poll_timeout": 5}, {"TELEGRAM_BOT_TOKEN": "t"})

    assert isinstance(adapter, telegram_adapter.TelegramBody)
    assert adapter._poll_timeout == 5
    assert adapter.shows == telegram_adapter.SHOWS

    with pytest.raises(RuntimeError, match="telegram: TELEGRAM_BOT_TOKEN is not set"):
        telegram_adapter.create({}, {})


def test_the_cli_factory_needs_nothing(tmp_path):
    adapter = cli_adapter.create({"socket": str(tmp_path / "s.sock")}, {})

    assert isinstance(adapter, cli_adapter.CLIBody)
    assert adapter.path == str(tmp_path / "s.sock")
    assert adapter.shows == cli_adapter.SHOWS


def test_both_adapters_come_through_the_loader_without_the_environment():
    loaded = Loader(ROOT, env={}).load(settings(bodies={"cli": {"enabled": True}}))
    assert [adapter.name for adapter in loaded.adapters] == ["cli"]

    loaded = Loader(ROOT, env={"TELEGRAM_BOT_TOKEN": "t"}).load(
        settings(bodies={"telegram": {"enabled": True}, "cli": {"enabled": True}}))
    assert [adapter.name for adapter in loaded.adapters] == ["cli", "telegram"]


def test_main_imports_no_adapter():
    source = (ROOT / "monoagens" / "__main__.py").read_text(encoding="utf-8")

    assert "monoagens.adapters" not in source
    assert "Loader(" in source
