"""Tests for the .env loader and LLM configuration resolution (Step 4)."""

from __future__ import annotations

import os

import pytest

from monoagens.infra.config import load_env_file, load_llm_config

KEYS = ("LLM_BASE_URL", "LLM_API_KEY", "LLM_MODEL", "LLM_REASONING_EFFORT")
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
        "effort": "",
        "parallel_tool_calls": True,
    }


def test_parallel_tool_calls_can_be_switched_off(tmp_path, monkeypatch):
    path = tmp_path / ".env"
    path.write_text("LLM_PARALLEL_TOOL_CALLS=false\n", encoding="utf-8")
    monkeypatch.delenv("LLM_PARALLEL_TOOL_CALLS", raising=False)

    assert load_llm_config(path)["parallel_tool_calls"] is False


def test_pfc_variables_are_ignored(tmp_path, monkeypatch):
    path = tmp_path / ".env"
    path.write_text(
        "PFC_BASE_URL=https://legacy.test/v1\nPFC_API_KEY=legacy-key\nPFC_MODEL=legacy-model\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("PFC_BASE_URL", "https://env-legacy.test/v1")
    monkeypatch.setenv("PFC_API_KEY", "env-legacy-key")
    monkeypatch.setenv("PFC_MODEL", "env-legacy-model")

    assert load_llm_config(path) == {"base_url": "", "api_key": "", "model": "",
                                     "effort": "", "parallel_tool_calls": True}


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
    assert load_llm_config(tmp_path / "absent") == {"base_url": "", "api_key": "",
                                                    "model": "", "effort": "",
                                                    "parallel_tool_calls": True}


def test_config_yaml_holds_only_deployment_settings():
    from monoagens.infra.config import DEFAULT_CONFIG_PATH, load_config

    config = load_config(DEFAULT_CONFIG_PATH)

    assert set(config) == {"version", "timezone", "owner_settings", "runtime", "bodies",
                          "plugins", "interaction", "roles", "people"}
    assert config["version"] == 2
    assert config["timezone"] == "Asia/Jakarta"
    assert config["owner_settings"]["daily_token_budget"] == 300000
    assert config["runtime"]["tick_seconds"] == 30
    assert config["bodies"]["cli"]["enabled"] is True


def test_soul_yaml_is_only_the_birth_certificate():
    from monoagens.infra.config import DEFAULT_SOUL_PATH, load_soul

    certificate = load_soul(DEFAULT_SOUL_PATH)

    assert set(certificate) == {"version", "name", "language", "persona", "principles"}
    assert certificate["name"] == "Mona"
    assert "Never lie and never invent facts." in certificate["principles"]


def test_the_prior_carries_the_behavioral_numbers():
    from monoagens.infra.prior import load_prior, signal_weights, load_settings

    prior = load_prior()
    assert set(prior["constants"]) >= {"seed", "retrieve_k", "decay", "default_expires_days",
                                       "max_active", "job_timeout_seconds"}
    assert set(prior["knobs"]) and set(prior["calibration"])
    assert set(signal_weights(load_settings())) == {
        "own_name", "info_gap", "feeling_of_knowing", "relevance", "social_cost"}


def test_settings_join_config_certificate_and_prior():
    from monoagens.infra.prior import load_settings

    settings = load_settings()

    assert settings["identity"]["name"] == "Mona"
    assert settings["timezone"] == "Asia/Jakarta"
    assert settings["owner_settings"]["daily_token_budget"] == 300000
    assert settings["runtime"]["sleep"]["hour"] == 2
    assert "cognition" not in settings and "memory" not in settings


def test_reasoning_effort_is_read_and_checked(tmp_path, monkeypatch):
    path = tmp_path / ".env"
    path.write_text("LLM_REASONING_EFFORT=high\n", encoding="utf-8")
    assert load_llm_config(path)["effort"] == "high"

    monkeypatch.setenv("LLM_REASONING_EFFORT", "medium")
    assert load_llm_config(path)["effort"] == "medium"

    path.write_text("LLM_REASONING_EFFORT=ultra\n", encoding="utf-8")
    monkeypatch.delenv("LLM_REASONING_EFFORT")
    with pytest.raises(ValueError, match="LLM_REASONING_EFFORT"):
        load_llm_config(path)
