"""The loader: config.yaml, config.d/ and the plugin manifests, and what they install."""

from __future__ import annotations

import json
import pathlib
import shutil
from datetime import datetime, timezone
from typing import Any

import pytest
import yaml

from monoagens.boot import build
from monoagens.infra.clock import FakeClock
from monoagens.infra.prior import load_settings
from monoagens.middleware import loader as loader_module
from monoagens.middleware import roles
from monoagens.middleware.loader import BootReport, Loader
from monoagens.types import Event
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 11, 12, 8, 0, tzinfo=timezone.utc)
TOOL_CODE = '''
class Tool:
    """One tool with one capability."""

    name = "demo"

    def capabilities(self):
        return ({"name": "poke", "description": "Poke something.",
                 "schema": {"type": "object", "properties": {}},
                 "communicative": False, "long_running": False,
                 "identity_changing": False},)

    async def act(self, action):
        return {"ok": True}


def register(api):
    api.add_tool(Tool())
'''


def manifest(name: str = "demo", **extra: Any) -> dict[str, Any]:
    """Return a minimal valid manifest for one plugin."""
    base = {
        "manifest_version": 1, "name": name, "version": "0.1.0",
        "description": "A demo plugin.", "dependencies": [], "optional_dependencies": [],
        "state_namespace": name, "features": {"tools": ["poke"]},
        "tools": {"poke": {"facts": ["reads"], "permission": f"{name}.poke"}},
        "settings": {}, "permissions": {f"{name}.poke": {"label": "Poke", "base": "guest"}},
        "role_settings": {}, "extension_points": {"opens": [], "fills": []},
        "skills": [], "rules": [], "knob_offsets": {}, "requires": [],
    }
    return {**base, **extra}


def make_plugin(root: pathlib.Path, name: str = "demo", *,
                spec: dict[str, Any] | None = None, code: str = TOOL_CODE,
                skills: dict[str, str] | None = None) -> pathlib.Path:
    """Write plugins/<name>/ with a manifest, its code and optional skills."""
    folder = root / "plugins" / name
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "plugin.yaml").write_text(yaml.safe_dump(spec or manifest(name)), encoding="utf-8")
    (folder / "__init__.py").write_text(code, encoding="utf-8")
    for skill, text in (skills or {}).items():
        target = folder / "skills" / skill
        target.mkdir(parents=True, exist_ok=True)
        (target / "SKILL.md").write_text(text, encoding="utf-8")
    return folder


def deployment(root: pathlib.Path, *, enabled: bool = False) -> pathlib.Path:
    """Write a whole deployment root: config.yaml, soul.yaml, prior/ and the demo plugin."""
    root.mkdir(parents=True, exist_ok=True)
    shutil.copytree(ROOT / "prior", root / "prior", dirs_exist_ok=True)
    (root / "soul.yaml").write_text("version: 1\nname: Mona\nlanguage: id\n", encoding="utf-8")
    (root / "config.yaml").write_text(
        "version: 2\n"
        "timezone: Asia/Jakarta\n"
        "owner_settings: {daily_token_budget: 1000}\n"
        "runtime: {tick_seconds: 30}\n"
        "bodies: {}\n"
        f"plugins:\n  demo: {{enabled: {'true' if enabled else 'false'}}}\n"
        "interaction: {thinking: owner_direct, steps: long, approvals: owner}\n"
        "roles:\n  fallback: unverified\n  unverified: {base: guest}\n"
        "people: {}\n", encoding="utf-8")
    make_plugin(root)
    if enabled:
        (root / "config.d").mkdir(exist_ok=True)
        (root / "config.d" / "demo.yaml").write_text(
            "version: 1\npermissions: {demo.poke: guest}\nrole_settings: {}\n",
            encoding="utf-8")
    return root


def settings_for(root: pathlib.Path) -> dict[str, Any]:
    """Load the deployment settings of one root the way `__main__` does."""
    return load_settings(root / "config.yaml", root / "soul.yaml", root / "prior")


async def _installed(aconn, root: pathlib.Path, *, enabled: bool = True):
    values = settings_for(root) if (root / "config.yaml").is_file() else settings(
        plugins={"demo": {"enabled": enabled}})
    loaded = Loader(root).load(values)
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    loader_module.install_commands(Loader(root), kernel)
    await Loader(root).install(kernel, loaded)
    return kernel, loaded


def _event() -> Event:
    return Event(id=None, ts=START, kind="command", who="person:owner", place="space:cli",
                 body="cli", payload={"command": "plugin", "arguments": []})


def test_a_plugin_loads_and_installs_its_tool(tmp_path):
    root = tmp_path
    make_plugin(root, spec=manifest(settings={"size": {"type": "int", "default": 5}}))
    loaded = Loader(root).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.plugins == ("demo",)
    assert loaded.permissions == {"demo.poke": "guest"}
    assert loaded.report.render().startswith("boot report:")


async def test_installing_adds_the_tool_to_the_registry(aconn, tmp_path):
    make_plugin(tmp_path)
    kernel, loaded = await _installed(aconn, tmp_path)

    assert kernel.tools.get("demo.poke") is not None
    assert kernel.tools.permission("demo.poke") == "demo.poke"
    assert kernel.boot_report is loaded.report
    assert kernel.loaded_plugins == ("demo",)


def test_a_disabled_plugin_is_off_and_need_not_exist(tmp_path):
    assert Loader(tmp_path).load(settings(plugins={"ghost": {"enabled": False}})).plugins == ()
    assert Loader(tmp_path).load(settings(plugins={"ghost": {}})).plugins == ()


def test_a_disabled_plugin_is_named_in_the_report(tmp_path):
    loaded = Loader(tmp_path).load(settings(plugins={"ghost": {"enabled": False}}))

    assert "plugin ghost: off" in loaded.report.lines


def test_an_enabled_plugin_without_a_folder_fails_at_boot(tmp_path):
    with pytest.raises(ValueError, match="plugin not installed: ghost"):
        Loader(tmp_path).load(settings(plugins={"ghost": {"enabled": True}}))


def test_an_unknown_manifest_key_fails_at_boot(tmp_path):
    make_plugin(tmp_path, spec={**manifest(), "weird": 1})

    with pytest.raises(ValueError, match="plugin demo: unknown manifest key 'weird'"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_missing_required_key_fails_at_boot(tmp_path):
    broken = manifest()
    broken.pop("description")
    make_plugin(tmp_path, spec=broken)

    with pytest.raises(ValueError, match="plugin demo: manifest needs description"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_state_namespace_must_match_the_plugin_name(tmp_path):
    make_plugin(tmp_path, spec=manifest(state_namespace="other"))

    with pytest.raises(ValueError, match="state_namespace must be 'demo'"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_wrong_option_type_fails_at_boot(tmp_path):
    make_plugin(tmp_path, spec=manifest(settings={"size": {"type": "int", "default": 5}}))
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "demo.yaml").write_text(
        "version: 1\noptions: {size: big}\n", encoding="utf-8")

    with pytest.raises(ValueError, match="config.d/demo.yaml: options.size: expected int"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_an_unknown_option_fails_at_boot(tmp_path):
    make_plugin(tmp_path)
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "demo.yaml").write_text(
        "version: 1\noptions: {nope: 1}\n", encoding="utf-8")

    with pytest.raises(ValueError, match="config.d/demo.yaml: options.nope: not in the manifest"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_config_d_file_for_an_absent_plugin_is_reported_and_ignored(tmp_path):
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "gone.yaml").write_text("version: 1\noptions: {}\n",
                                                     encoding="utf-8")

    loaded = Loader(tmp_path).load(settings(plugins={}))

    assert "config.d/gone.yaml: plugin gone is not installed; ignored" in loaded.report.lines


def test_dependencies_load_first(tmp_path):
    make_plugin(tmp_path, "base", code="def register(api):\n    pass\n",
                spec=manifest("base", features={}, tools={}, permissions={}))
    make_plugin(tmp_path, "demo", spec=manifest(dependencies=["base"]))

    loaded = Loader(tmp_path).load(
        settings(plugins={"demo": {"enabled": True}, "base": {"enabled": True}}))

    assert loaded.plugins == ("base", "demo")


def test_a_missing_dependency_fails_naming_both(tmp_path):
    make_plugin(tmp_path, spec=manifest(dependencies=["base"]))

    with pytest.raises(ValueError, match="plugin demo: dependency base is not installed"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_dependency_cycle_fails(tmp_path):
    make_plugin(tmp_path, "a", spec=manifest("a", dependencies=["b"]))
    make_plugin(tmp_path, "b", spec=manifest("b", dependencies=["a"]))

    with pytest.raises(ValueError, match="plugin cycle: a -> b -> a"):
        Loader(tmp_path).load(settings(plugins={"a": {"enabled": True},
                                                "b": {"enabled": True}}))


def test_a_feature_switched_off_is_removed_from_the_registry(tmp_path):
    make_plugin(tmp_path)
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "demo.yaml").write_text(
        "version: 1\nfeatures: {tools: {poke: false}}\n", encoding="utf-8")

    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.apis[0].features["tools"] == []


def test_an_empty_list_switches_every_feature_off(tmp_path):
    make_plugin(tmp_path)
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "demo.yaml").write_text("version: 1\nfeatures: {tools: []}\n",
                                                     encoding="utf-8")

    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.apis[0].features["tools"] == []


def test_a_list_replaces_the_default_with_a_note(tmp_path):
    make_plugin(tmp_path)
    (tmp_path / "config.d").mkdir()
    (tmp_path / "config.d" / "demo.yaml").write_text(
        "version: 1\nfeatures: {tools: [other]}\n", encoding="utf-8")

    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.apis[0].features["tools"] == ["other"]
    assert "config.d/demo.yaml: tools list replaces the manifest default" in loaded.report.lines


def test_a_tool_without_facts_fails_at_boot(tmp_path):
    make_plugin(tmp_path, spec=manifest(tools={"poke": {"permission": "demo.poke"}}))

    with pytest.raises(ValueError, match="demo.poke: a tool without facts"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_tool_without_a_declared_permission_fails_at_boot(tmp_path):
    make_plugin(tmp_path, spec=manifest(tools={"poke": {"facts": ["reads"]}},
                                        permissions={}))

    with pytest.raises(ValueError, match="demo.poke: an entry without a declared permission"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


def test_a_missing_required_module_marks_the_plugin_unavailable(tmp_path):
    make_plugin(tmp_path, spec=manifest(requires=["no_such_module_xyz"]))

    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.plugins == ()
    assert any("plugin demo: needs no_such_module_xyz" in line for line in loaded.report.lines)


def test_a_missing_required_module_can_be_stubbed(tmp_path, monkeypatch, sys_modules=None):
    import sys
    import types

    monkeypatch.setitem(sys.modules, "stub_module_xyz", types.ModuleType("stub_module_xyz"))
    make_plugin(tmp_path, spec=manifest(requires=["stub_module_xyz"]))

    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    assert loaded.plugins == ("demo",)


async def test_two_plugins_claiming_one_command_collide(tmp_path, aconn):
    code = '''
def register(api):
    permission = next(iter(api.manifest["permissions"]))

    async def handler(kernel, event, arguments):
        return "hi"

    api.add_command("greet", "Say hi.", handler, permission=permission)
'''
    make_plugin(tmp_path, "one", code=code, spec=manifest("one"))
    make_plugin(tmp_path, "two", code=code, spec=manifest("two"))
    loaded = Loader(tmp_path).load(settings(plugins={"one": {"enabled": True},
                                                     "two": {"enabled": True}}))
    kernel = build(None, clock=FakeClock(START), settings=settings())

    with pytest.raises(ValueError, match="collision: greet"):
        await Loader(tmp_path).install(kernel, loaded)


def test_skills_are_read_and_registered(tmp_path):
    make_plugin(tmp_path, skills={"brewing": "---\nname: brewing\n"
                                            "description: How to brew coffee.\nversion: 1\n---\n"
                                            "Steps here.\n"},
                spec=manifest(skills=["brewing"]))
    loaded = Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))

    skill = loaded.apis[0].skills[0]
    assert (skill.name, skill.description, skill.body) == (
        "brewing", "How to brew coffee.", "Steps here.")


def test_an_extension_point_opens_once_and_must_exist_to_fill(tmp_path):
    code = '''
def register(api):
    api.open_point("reader")
'''
    make_plugin(tmp_path, "one", code=code, spec=manifest("one"))
    make_plugin(tmp_path, "two", code=code, spec=manifest("two"))

    with pytest.raises(ValueError, match="extension point reader is already open"):
        Loader(tmp_path).load(settings(plugins={"one": {"enabled": True},
                                                "two": {"enabled": True}}))


def test_filling_an_unopened_point_fails(tmp_path):
    code = '''
def register(api):
    api.fill_point("reader", ".pdf", lambda path: "pdf")
'''
    make_plugin(tmp_path, code=code)

    with pytest.raises(ValueError, match="plugin demo: no such extension point reader"):
        Loader(tmp_path).load(settings(plugins={"demo": {"enabled": True}}))


async def test_plugin_enable_writes_only_its_own_lines(aconn, tmp_path):
    root = deployment(tmp_path)
    before = (root / "config.yaml").read_text(encoding="utf-8")
    kernel, _ = await _installed(aconn, root)

    reply = await kernel.tools.command("plugin").handler(kernel, _event(),
                                                         ["enable", "demo"])

    after = (root / "config.yaml").read_text(encoding="utf-8")
    assert reply.startswith("/plugin enable demo: reload ok:")
    assert after.splitlines()[6] == "  demo: {enabled: true}"
    assert [line for i, line in enumerate(after.splitlines())
            if i != 6] == [line for i, line in enumerate(before.splitlines()) if i != 6]
    assert list(root.glob("config.yaml.bak-*"))
    assert (root / "config.d" / "demo.yaml").is_file()
    assert kernel.tools.get("demo.poke") is not None


async def test_plugin_disable_flips_one_line(aconn, tmp_path):
    root = deployment(tmp_path, enabled=True)
    kernel, _ = await _installed(aconn, root)

    await kernel.tools.command("plugin").handler(kernel, _event(), ["disable", "demo"])

    text = (root / "config.yaml").read_text(encoding="utf-8")
    assert "  demo: {enabled: false}" in text
    assert kernel.tools.get("demo.poke") is None


async def test_reload_refuses_an_invalid_file_and_keeps_the_running_state(aconn, tmp_path):
    root = deployment(tmp_path, enabled=True)
    kernel, _ = await _installed(aconn, root)
    (root / "config.d" / "demo.yaml").write_text("version: 1\nunknown: 1\n", encoding="utf-8")

    reply = await kernel.tools.command("reload").handler(kernel, _event(), [])

    assert reply.startswith("reload refused: config.d/demo.yaml: unknown key 'unknown'")
    assert kernel.tools.get("demo.poke") is not None


async def test_uninstalling_removes_the_plugin_and_keeps_its_history(aconn, tmp_path):
    root = deployment(tmp_path, enabled=True)
    kernel, _ = await _installed(aconn, root)
    await kernel.tools.command("plugin").handler(kernel, _event(), ["disable", "demo"])
    await kernel.tools.command("plugin").handler(kernel, _event(), ["enable", "demo"])
    shutil.rmtree(root / "plugins" / "demo")
    text = (root / "config.yaml").read_text(encoding="utf-8").replace(
        "  demo: {enabled: true}\n", "")  # the owner also removes the entry by hand
    (root / "config.yaml").write_text(text, encoding="utf-8")

    reply = await kernel.tools.command("reload").handler(kernel, _event(), [])

    assert reply.startswith("reload ok: 0 plugins")
    assert kernel.tools.get("demo.poke") is None
    assert any("config.d/demo.yaml: plugin demo is not installed; ignored"
               in line for line in kernel.boot_report.lines)
    cur = await aconn.execute("SELECT count(*) FROM events "
                              "WHERE kind = 'plugin.config_changed'")
    assert (await cur.fetchone())[0] >= 2


def test_set_plugin_enabled_creates_the_block_when_absent():
    text = "version: 2\ntimezone: UTC\n"

    assert loader_module.set_plugin_enabled(text, "demo", True) == (
        "version: 2\ntimezone: UTC\nplugins:\n  demo: {enabled: true}\n")


def test_set_plugin_enabled_adds_a_missing_entry_inside_the_block():
    text = "plugins:\n  one: {enabled: true}\nroles: {}\n"

    assert loader_module.set_plugin_enabled(text, "demo", True) == (
        "plugins:\n  one: {enabled: true}\n  demo: {enabled: true}\nroles: {}\n")


def test_a_report_with_lines_renders_them_and_a_clean_one_says_so():
    report = BootReport()
    assert report.render() == "boot report: clean"
    report.add("plugin demo: off")
    assert report.render() == "boot report:\nplugin demo: off"


async def test_a_plugin_middleware_is_replaced_in_place_on_reload(aconn, tmp_path):
    code = '''
from monoagens.core.hooks import Continue


def register(api):
    async def rule(item, ctx):
        ctx.setdefault("seen", []).append("v1")
        return Continue()

    api.add_middleware("inbound", "mark", rule)
'''
    root = tmp_path
    make_plugin(root, code=code, spec=manifest(features={}, tools={}, permissions={}))
    kernel, _ = await _installed(aconn, root)
    before = [entry.name for entry in kernel.hooks.inbound._registrations]

    await kernel.tools.command("reload").handler(kernel, _event(), [])

    after = [entry.name for entry in kernel.hooks.inbound._registrations]
    assert after == before
    assert "demo-mark" in after
