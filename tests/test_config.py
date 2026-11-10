"""Tests for the .env loader and LLM configuration resolution (Step 4)."""

from __future__ import annotations

import os

import pytest

from monoagens.config import load_env_file, load_llm_config

KEYS = ("LLM_BASE_URL", "LLM_API_KEY", "LLM_MODEL")
LEGACY_KEYS = ("PFC_BASE_URL", "PFC_API_KEY", "PFC_MODEL")


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    for key in KEYS + LEGACY_KEYS:
        monkeypatch.delenv(key, raising=False)


def test_load_env_file_parses_and_strips(tmp_path):
    path = tmp_path / ".env"
    path.write_text("# comment\n\nA=1\nB = 'two' \nC=\"three\"\nNOPE\n", encoding="utf-8")

    assert load_env_file(path) == {"A": "1", "B": "two", "C": "three"}


def test_load_env_file_missing_path_is_empty(tmp_path):
    assert load_env_file(tmp_path / "absent") == {}


def test_llm_config_from_file(tmp_path):
    path = tmp_path / ".env"
    path.write_text(
        "LLM_BASE_URL=https://file.test/v1\nLLM_API_KEY=file-key\nLLM_MODEL=file-model\n",
        encoding="utf-8",
    )

    assert load_llm_config(path) == {
        "base_url": "https://file.test/v1",
        "api_key": "file-key",
        "model": "file-model",
    }


def test_pfc_variables_are_ignored(tmp_path, monkeypatch):
    path = tmp_path / ".env"
    path.write_text(
        "PFC_BASE_URL=https://legacy.test/v1\nPFC_API_KEY=legacy-key\nPFC_MODEL=legacy-model\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("PFC_BASE_URL", "https://env-legacy.test/v1")
    monkeypatch.setenv("PFC_API_KEY", "env-legacy-key")
    monkeypatch.setenv("PFC_MODEL", "env-legacy-model")

    assert load_llm_config(path) == {"base_url": "", "api_key": "", "model": ""}


def test_environment_wins_over_file(tmp_path, monkeypatch):
    path = tmp_path / ".env"
    path.write_text("LLM_BASE_URL=https://file.test/v1\nLLM_MODEL=file-model\n", encoding="utf-8")
    monkeypatch.setenv("LLM_BASE_URL", "https://env.test/v1")

    config = load_llm_config(path)
    assert config["base_url"] == "https://env.test/v1"
    assert config["model"] == "file-model"
    assert config["api_key"] == ""


def test_missing_file_and_env_yield_empty_strings(tmp_path):
    assert os.environ.get("LLM_MODEL") is None
    assert load_llm_config(tmp_path / "absent") == {"base_url": "", "api_key": "", "model": ""}
