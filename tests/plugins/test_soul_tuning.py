"""Soul Tuning: the mode block, the knob offsets, /mode, and uninstalling without a trace."""

from __future__ import annotations

import hashlib
import pathlib
import shutil
from datetime import datetime, timezone

import pytest
import yaml

from monoagens.boot import build
from monoagens.core.context import layout
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware import policy
from monoagens.middleware.loader import Loader
from monoagens.types import Event
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
PLUGIN = ROOT / "plugins" / "soul-tuning"
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
PLACE = "space:cli"


def _root(tmp_path: pathlib.Path, *names: str) -> pathlib.Path:
    """Write a deployment root holding the named plugins (copied from the repository)."""
    (tmp_path / "plugins").mkdir(exist_ok=True)
    for name in names:
        shutil.copytree(ROOT / "plugins" / name, tmp_path / "plugins" / name)
    return tmp_path


async def _kernel(conn, root, *names: str):
    values = settings(plugins={name: {"enabled": True} for name in names})
    loader = Loader(root)
    loaded = loader.load(values)
    kernel = build(conn, clock=FakeClock(START), settings=values)
    await loader.install(kernel, loaded)
    return kernel, loaded


async def _trigger(conn) -> Event:
    return await append_event(conn, ts=START, kind="percept", who="person:owner", place=PLACE,
                              body="cli", payload={"description": "halo", "place_kind": "direct"})


async def _block_text(conn, kernel) -> str:
    trigger = await _trigger(conn)
    context = await layout.build_context(conn, trigger, PLACE, "test", now=START,
                                         settings=kernel.settings, memories=(),
                                         capabilities=kernel.tools.descriptors())
    ctx = {"conn": conn, "event": trigger, "now": START}
    filtered = await kernel._run_hooks("context", context, ctx, trigger)
    return "\n".join(item.text for item in filtered.items if item.section == "notes")


async def _mode(kernel, arguments: list[str], who: str = "person:owner") -> str:
    entry = kernel.tools.command("mode")
    event = Event(id=None, ts=START, kind="command", who=who, place=PLACE, body="cli",
                  payload={"command": "mode", "arguments": arguments})
    return await entry.handler(kernel, event, arguments)


async def test_installing_adds_the_mode_block_with_its_rule_and_examples(aconn, tmp_path):
    kernel, loaded = await _kernel(aconn, _root(tmp_path, "soul-tuning"), "soul-tuning")

    assert loaded.plugins == ("soul-tuning",)
    text = await _block_text(aconn, kernel)
    assert text.startswith("<soul_tuning>\nmode: daily\nSpeak casually")
    assert "Examples:" in text
    assert "the style, not the language" in text
    assert text.index("morning!") < text.index("not really")
    assert "person:" in text and "you:" in text
    assert "professional" not in text


async def test_the_owner_may_bring_examples_through_the_option(aconn, tmp_path):
    root = _root(tmp_path, "soul-tuning")
    (root / "config.d").mkdir(exist_ok=True)
    (root / "config.d" / "soul-tuning.yaml").write_text(
        "version: 1\noptions:\n  examples:\n    daily:\n"
        "      - [person, \"hey\"]\n      - [you, \"hey, what's up?\"]\n",
        encoding="utf-8")
    kernel, _loaded = await _kernel(aconn, root, "soul-tuning")

    text = await _block_text(aconn, kernel)
    assert "person: hey" in text and "you: hey, what's up?" in text
    assert "morning!" not in text
    assert "person: good morning" not in text  # the other mode keeps the file's examples

    import sys
    modes = sys.modules["monoagens_plugins_soul_tuning"].modes
    assert modes.from_options({"examples": {"daily": [["person", "a"], ["you", "b"]]}})[
        "daily"] == [("person", "a"), ("you", "b")]
    with pytest.raises(ValueError, match="unknown mode"):
        modes.from_options({"examples": {"rapping": [["person", "a"]]}})


async def test_the_stable_prefix_does_not_change(aconn, tmp_path):
    with_plugin, _ = await _kernel(aconn, _root(tmp_path, "soul-tuning"), "soul-tuning")
    without = build(aconn, clock=FakeClock(START), settings=settings())
    trigger = await _trigger(aconn)
    soul = Reasoning(settings=with_plugin.settings)

    async def first_message(kernel) -> str:
        context = await layout.build_context(
            aconn, trigger, PLACE, "test", now=START, settings=kernel.settings, memories=(),
            capabilities=kernel.tools.descriptors())
        filtered = await kernel._run_hooks("context", context,
                                           {"conn": aconn, "event": trigger, "now": START},
                                           trigger)
        return soul.build_messages(filtered)[0]["content"]

    before = hashlib.sha256((await first_message(without)).encode()).hexdigest()
    after = hashlib.sha256((await first_message(with_plugin)).encode()).hexdigest()
    await _mode(with_plugin, ["professional"])
    switched = hashlib.sha256((await first_message(with_plugin)).encode()).hexdigest()

    assert before == after == switched


async def test_mode_moves_the_knob_by_its_offset_and_back(aconn, tmp_path):
    kernel, _ = await _kernel(aconn, _root(tmp_path, "soul-tuning"), "soul-tuning")
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.45)

    assert await _mode(kernel, ["professional"]) == "mode: professional"
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.40)
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'disposition.updated'")
    payload = (await cur.fetchone())[0]
    assert payload["source"] == "soul-tuning" and payload["knob"] == "talkativeness"
    assert payload["offset"] == pytest.approx(-0.05)

    assert await _mode(kernel, ["professional"]) == "mode: professional"
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.40)
    assert await _mode(kernel, ["daily"]) == "mode: daily"
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.50)
    assert await _mode(kernel, []) == "mode: daily (daily|professional)"
    assert await _mode(kernel, ["loud"]) == "mode must be daily or professional"


async def test_a_guest_cannot_switch_the_mode(aconn, tmp_path):
    kernel, _ = await _kernel(aconn, _root(tmp_path, "soul-tuning"), "soul-tuning")

    entry = kernel.tools.command("mode")
    assert not kernel.roles.can("person:tg:guest", entry.permission)
    assert "professional" not in kernel.tools.allowed("person:tg:guest", kernel.roles)


async def test_two_plugins_offsets_sum_and_stay_inside_their_bounds(aconn, tmp_path):
    root = _root(tmp_path, "soul-tuning")
    other = root / "plugins" / "other-tuning"
    other.mkdir()
    (other / "plugin.yaml").write_text(yaml.safe_dump({
        "manifest_version": 1, "name": "other-tuning", "version": "0.1.0",
        "description": "A second tuner.", "features": {}, "tools": {}, "settings": {},
        "permissions": {}, "knob_offsets": {"talkativeness": [-0.2, 0.2]}}), encoding="utf-8")
    (other / "__init__.py").write_text("def register(api):\n    pass\n", encoding="utf-8")
    kernel, _ = await _kernel(aconn, root, "soul-tuning", "other-tuning")
    await _mode(kernel, ["professional"])

    for offset in (0.1, 0.5):
        await append_event(aconn, ts=START, kind="disposition.updated",
                           payload={"source": "other-tuning", "knob": "talkativeness",
                                    "offset": offset})
        await policy.apply_offsets(kernel)
        assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(
            max(0.0, min(1.0, 0.45 - 0.05 + min(0.2, offset))))

    kernel.knob_bounds.pop("other-tuning")
    await policy.apply_offsets(kernel)
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.40)


async def test_uninstalling_returns_the_knobs_to_the_prior(aconn, tmp_path):
    root = _root(tmp_path, "soul-tuning")
    kernel, _ = await _kernel(aconn, root, "soul-tuning")
    await _mode(kernel, ["professional"])
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.40)

    shutil.rmtree(root / "plugins" / "soul-tuning")
    values = settings(plugins={})
    loader = Loader(root)
    loaded = loader.load(values)
    await loader.install(kernel, loaded)
    trigger = await _trigger(aconn)
    context = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                         settings=kernel.settings, memories=(),
                                         capabilities=kernel.tools.descriptors())

    filtered = await kernel._run_hooks("context", context,
                                       {"conn": aconn, "event": trigger, "now": START},
                                       trigger)
    assert kernel.settings["knobs"]["talkativeness"] == pytest.approx(0.45)
    assert not any(item.kind == "soul-tuning" for item in filtered.items)
    assert kernel.tools.command("mode") is None
