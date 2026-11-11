"""The scenario contract of Part 10: every scenario names its brain function and capability."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
from zoneinfo import ZoneInfo

import pytest

ROOT = pathlib.Path(__file__).resolve().parent.parent
_SPEC = importlib.util.spec_from_file_location("replay_script", ROOT / "scripts" / "replay.py")
replay = importlib.util.module_from_spec(_SPEC)
assert _SPEC.loader is not None
sys.modules[_SPEC.name] = replay
_SPEC.loader.exec_module(replay)


def test_every_scenario_declares_its_function_and_capability() -> None:
    paths = sorted((ROOT / "scenarios").rglob("*.yaml"))
    assert paths
    for path in paths:
        scenario = replay.load_scenario(path, ZoneInfo("UTC"))
        assert scenario.function == path.parent.name, path
        assert scenario.capability and "\n" not in scenario.capability, path


def test_a_scenario_without_a_capability_is_rejected(tmp_path: pathlib.Path) -> None:
    folder = tmp_path / "memory"
    folder.mkdir()
    path = folder / "99_missing.yaml"
    path.write_text("name: 99_missing\nfunction: memory\nsteps:\n  - text: hi\n",
                    encoding="utf-8")
    with pytest.raises(ValueError, match="capability"):
        replay.load_scenario(path, ZoneInfo("UTC"))


def test_a_scenario_whose_function_does_not_match_its_folder_is_rejected(
        tmp_path: pathlib.Path) -> None:
    folder = tmp_path / "memory"
    folder.mkdir()
    path = folder / "99_mismatch.yaml"
    path.write_text("name: 99_mismatch\nfunction: silence\ncapability: x\n"
                    "steps:\n  - text: hi\n", encoding="utf-8")
    with pytest.raises(ValueError, match="folder"):
        replay.load_scenario(path, ZoneInfo("UTC"))
