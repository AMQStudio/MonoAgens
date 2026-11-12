"""Format versions (D58): older is upcast in memory and named, newer fails at boot.

A file without its version key is read as version 1 with a note; the file is never
rewritten. `config.d/*.yaml` files carry `version` too and read unchanged at version 1.
"""

from __future__ import annotations

import pytest

from monoagens.infra.config import (DEFAULT_CONFIG_PATH, DEFAULT_SOUL_PATH, load_config,
                                    load_soul, read_versioned)
from monoagens.infra.prior import load_settings


def _write(tmp_path, text: str, name: str = "config.yaml"):
    path = tmp_path / name
    path.write_text(text, encoding="utf-8")
    return path


def test_a_current_file_reads_unchanged_without_notes(tmp_path):
    path = _write(tmp_path, "version: 2\ntimezone: UTC\nplugins:\n  workspace: {enabled: true}\n")
    notes: list[str] = []

    config = load_config(path, notes=notes)

    assert config == {"version": 2, "timezone": "UTC",
                      "plugins": {"workspace": {"enabled": True}}}
    assert notes == []


def test_a_version_1_file_is_upcast_in_memory_and_named(tmp_path):
    path = _write(tmp_path, "timezone: UTC\nplugins: [workspace, web-search]\n")
    notes: list[str] = []

    config = load_config(path, notes=notes)

    assert config["plugins"] == {"workspace": {"enabled": True},
                                 "web-search": {"enabled": True}}
    assert notes == ["config.yaml config.yaml: no version: read as version 1",
                     "config.yaml config.yaml: upcast from version 1 to 2"]


def test_the_file_is_never_rewritten(tmp_path):
    path = _write(tmp_path, "timezone: UTC\nplugins: [workspace]\n")
    load_config(path, notes=[])
    assert path.read_text(encoding="utf-8") == "timezone: UTC\nplugins: [workspace]\n"


def test_a_newer_version_than_supported_fails_at_boot(tmp_path):
    path = _write(tmp_path, "version: 3\n")

    with pytest.raises(ValueError, match="config.yaml config.yaml: format version 3 is "
                                        "newer than supported 2"):
        load_config(path, notes=[])


def test_an_invalid_version_fails_at_boot(tmp_path):
    path = _write(tmp_path, "version: two\n")

    with pytest.raises(ValueError, match="config.yaml config.yaml: invalid version 'two'"):
        load_config(path, notes=[])


def test_a_missing_file_is_empty_and_silent(tmp_path):
    notes: list[str] = []
    assert load_config(tmp_path / "absent.yaml", notes=notes) == {}
    assert notes == []


def test_a_missing_version_warns_without_a_note_sink(tmp_path, caplog):
    path = _write(tmp_path, "timezone: UTC\n", "soul.yaml")

    with caplog.at_level("WARNING", logger="monoagens.infra.config"):
        certificate = load_soul(path)

    assert certificate["timezone"] == "UTC"
    assert "soul.yaml soul.yaml: no version: read as version 1" in caplog.text


def test_a_config_d_file_at_version_1_reads_unchanged(tmp_path):
    path = _write(tmp_path, "version: 1\noptions: {timeout_seconds: 300}\n",
                  "workspace.yaml")
    notes: list[str] = []

    values = read_versioned(path, key="version", supported=1, label="config.d",
                            upcasters={}, notes=notes)

    assert values == {"version": 1, "options": {"timeout_seconds": 300}}
    assert notes == []


def test_the_shipped_files_carry_the_current_version():
    notes: list[str] = []

    assert load_config(DEFAULT_CONFIG_PATH, notes=notes)["version"] == 2
    assert load_soul(DEFAULT_SOUL_PATH, notes=notes)["version"] == 1
    assert load_settings(notes=notes)
    assert notes == []
