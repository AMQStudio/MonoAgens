"""One host-file writer (ADR-0002 §4.1): a single-file bind mount refuses the rename.

`compose.yaml` mounts `config.yaml`, `roles.yaml`, and `soul.yaml` as single files. `rename(2)`
onto a mount point answers `EBUSY`, so the atomic path falls back to rewriting the same inode.
Before this contract, `plugins enable` and the `settings` tool failed only on a compose
deployment; these tests pin the fallback and the mode it must keep.
"""


import errno
import os
import stat
from pathlib import Path

import pytest
import yaml
from support import build_config

from monoagens.engine import plugins
from monoagens.engine.config import PluginSettings
from monoagens.engine.config.write import write_text

ROOT = Path(__file__).resolve().parents[1]
PLUGIN_TOOL = '''"""Fixture tool."""

from monoagens.engine.registry import Handler


async def _run(args, ctx):
    return "ok"


TOOL = Handler(
    name="echo",
    family="read",
    description="Fixture tool.",
    fn=_run,
    parameters={"type": "object", "properties": {}},
)
'''
PLUGIN_ENTRY = '''"""Fixture entry."""

from monoagens.engine.registry import Handler

from .tools.echo import TOOL as ECHO


def register() -> dict[str, list[Handler]]:
    return {"tools": [ECHO], "commands": [], "gateways": []}
'''


def busy(src, dst):
    raise OSError(errno.EBUSY, "Device or resource busy")


def test_the_write_falls_back_to_the_same_inode_on_ebusy(tmp_path, monkeypatch) -> None:
    path = tmp_path / "config.yaml"
    path.write_text("a: 1\n", encoding="utf-8")
    before = path.stat().st_ino
    monkeypatch.setattr("monoagens.engine.config.write.os.replace", busy)

    write_text(path, "a: 2\n")

    assert path.read_text(encoding="utf-8") == "a: 2\n"
    assert path.stat().st_ino == before  # the mount sees the same file
    assert not list(tmp_path.glob(".*.tmp"))


def test_the_write_replaces_the_file_and_keeps_its_mode(tmp_path) -> None:
    path = tmp_path / "config.yaml"
    path.write_text("a: 1\n", encoding="utf-8")
    os.chmod(path, 0o600)

    write_text(path, "a: 2\n")

    assert path.read_text(encoding="utf-8") == "a: 2\n"
    assert stat.S_IMODE(path.stat().st_mode) == 0o600


def test_a_write_that_is_not_ebusy_still_raises(tmp_path, monkeypatch) -> None:
    path = tmp_path / "config.yaml"
    path.write_text("a: 1\n", encoding="utf-8")

    def broken(src, dst):
        raise OSError(errno.ENOSPC, "No space left on device")

    monkeypatch.setattr("monoagens.engine.config.write.os.replace", broken)

    with pytest.raises(OSError, match="No space left"):
        write_text(path, "a: 2\n")
    assert path.read_text(encoding="utf-8") == "a: 1\n"  # the old file survives


def plugin_tree(root: Path) -> Path:
    """One minimal local plugin whose manifest carries the real tree digest."""
    folder = root / "demo"
    (folder / "tools").mkdir(parents=True)
    (folder / "tools" / "__init__.py").write_text("", encoding="utf-8")
    (folder / "tools" / "echo.py").write_text(PLUGIN_TOOL, encoding="utf-8")
    (folder / "plugin.py").write_text(PLUGIN_ENTRY, encoding="utf-8")
    manifest = {
        "name": "demo",
        "version": "1.0.0",
        "engine": ">=0.0.0,<1.0",
        "entry": "plugin.py:register",
        "digest": plugins.tree_digest(folder),
        "provides": {"tools": ["echo"], "commands": [], "gateways": [], "prompt": []},
        "requires": [],
        "secrets": [],
        "credits": [
            {
                "license": "Apache-2.0",
                "author": "Fixture Authors",
                "source": "https://example.test/fixture",
                "use": "fixture dependency",
            }
        ],
    }
    (folder / "plugin.yaml").write_text(yaml.safe_dump(manifest), encoding="utf-8")
    return folder


def test_enable_writes_the_config_of_a_bind_mounted_file(tmp_path, monkeypatch) -> None:
    """The operator's actual path: `plugins enable` on a compose deployment."""
    root = tmp_path / "plugins"
    plugin_tree(root)
    config = tmp_path / "config.yaml"
    config.write_text(yaml.safe_dump({"plugins": {"demo": {"enabled": False}}}), encoding="utf-8")
    settings = plugins.PluginLoader(
        root=root,
        fetch=False,
        plugins={"demo": PluginSettings(enabled=False)},
        config_path=config,
    )
    monkeypatch.setattr("monoagens.engine.config.write.os.replace", busy)

    item = plugins.enable(settings, "demo")

    assert item.state == "enabled"
    body = yaml.safe_load(config.read_text(encoding="utf-8"))
    assert body["plugins"]["demo"]["enabled"] is True
    assert not list(tmp_path.glob(".*.tmp"))


def test_a_missing_source_is_named_before_the_pin(tmp_path) -> None:
    """The first error must name the blocker the operator can act on: `plugin.source`."""
    settings = plugins.PluginLoader(root=tmp_path / "plugins", fetch=True, source="", plugins={})

    with pytest.raises(plugins.PluginError, match=r"plugin\.source is not set"):
        plugins.install(settings, "demo")

    pinned = plugins.PluginLoader(
        root=tmp_path / "plugins",
        fetch=True,
        source="",
        plugins={"demo": PluginSettings(enabled=False, digest="sha256:" + "0" * 64)},
    )
    with pytest.raises(plugins.PluginError, match=r"plugin\.source is not set"):
        plugins.install(pinned, "demo")


def test_the_loader_still_reads_what_the_writer_wrote(tmp_path, monkeypatch) -> None:
    """The fallback keeps the document valid, so a reload parses it (no half-written file)."""
    from monoagens.engine.config import load_host

    config = build_config(tmp_path).paths.config
    config.write_text("plugin:\n  source: ''\n", encoding="utf-8")
    monkeypatch.setattr("monoagens.engine.config.write.os.replace", busy)

    write_text(config, "plugin:\n  source: /srv/plugins\n")

    host, _paths = load_host(config)
    assert host.plugin.source == "/srv/plugins"
