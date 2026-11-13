"""Tests for model provider drivers in monoagens/infra/providers/."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
import json

import httpx
import pytest

from monoagens.boot import build
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import FakeClock
from monoagens.infra.config import load_provider_config
from monoagens.infra.providers import (
    OPENCODE_GO_ENDPOINT,
    FallbackProvider,
    OpenAICompatibleProvider,
    OpenCodeGoProvider,
    build_provider,
    session_id_for_soul,
)
from monoagens.types import Event
from tests.support import settings


def _ok_response(*, cached_tokens: int = 12) -> httpx.Response:
    return httpx.Response(
        200,
        json={
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "act",
                                    "arguments": json.dumps(
                                        {"capability": "telegram.say", "arguments": {"text": "hi"}}
                                    ),
                                },
                            }
                        ],
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 20,
                "prompt_tokens_details": {"cached_tokens": cached_tokens},
                "completion_tokens": 5,
                "completion_tokens_details": {"reasoning_tokens": 2},
                "total_tokens": 25,
            },
        },
    )


async def test_opencode_go_sends_endpoint_session_header_and_reads_cached_tokens():
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return _ok_response(cached_tokens=16)

    provider = OpenCodeGoProvider(
        "oc-key",
        "deepseek-v4.1-flash",
        soul_id="Mona",
        effort="high",
        transport=httpx.MockTransport(handler),
    )
    calls, usage = await provider.decide(
        [{"role": "user", "content": "halo"}],
        [{"type": "function", "function": {"name": "act"}}],
    )
    await provider.aclose()

    assert len(seen) == 1
    assert str(seen[0].url) == OPENCODE_GO_ENDPOINT
    assert seen[0].headers["authorization"] == "Bearer oc-key"
    assert seen[0].headers["x-opencode-session"] == session_id_for_soul("Mona")
    assert session_id_for_soul("Mona") == session_id_for_soul("Mona")
    assert session_id_for_soul("Mona") != session_id_for_soul("Other")
    body = json.loads(seen[0].content)
    assert body["model"] == "deepseek-v4.1-flash"
    assert body["reasoning_effort"] == "high"
    assert calls[0].name == "act"
    assert usage == {
        "prompt_tokens": 20,
        "cached_tokens": 16,
        "completion_tokens": 5,
        "reasoning_tokens": 2,
        "total_tokens": 25,
    }


async def test_opencode_go_maps_medium_effort_to_high_with_boot_note():
    seen_bodies: list[dict] = []
    notes: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen_bodies.append(json.loads(request.content))
        return _ok_response()

    provider = OpenCodeGoProvider(
        "oc-key",
        effort="medium",
        notes=notes,
        transport=httpx.MockTransport(handler),
    )
    await provider.decide([{"role": "user", "content": "hi"}], [])
    await provider.aclose()

    assert seen_bodies[0]["reasoning_effort"] == "high"
    assert notes == ["opencode-go: mapped reasoning_effort 'medium' to 'high'"]


async def test_opencode_go_allows_low_and_max_and_rejects_invalid_effort():
    for effort in ("low", "max"):
        bodies: list[dict] = []
        provider = OpenCodeGoProvider(
            "oc-key",
            effort=effort,
            transport=httpx.MockTransport(
                lambda req, b=bodies: (b.append(json.loads(req.content)), _ok_response())[1]
            ),
        )
        await provider.decide([{"role": "user", "content": "hi"}], [])
        await provider.aclose()
        assert bodies[0]["reasoning_effort"] == effort

    with pytest.raises(ValueError):
        OpenCodeGoProvider("oc-key", effort="extreme")


async def test_fallback_moves_on_429_503_and_transport_and_records_in_usage():
    for status_or_exc, expected_reason in (
        (429, "http_429"),
        (402, "http_402"),
        (503, "http_503"),
        ("transport", "ConnectError"),
    ):
        primary_calls = 0
        fallback_calls = 0

        def primary_handler(req: httpx.Request, kind=status_or_exc) -> httpx.Response:
            nonlocal primary_calls
            primary_calls += 1
            if kind == "transport":
                raise httpx.ConnectError("down", request=req)
            return httpx.Response(int(kind), json={"error": {"message": "unavailable"}})

        def fallback_handler(req: httpx.Request) -> httpx.Response:
            nonlocal fallback_calls
            fallback_calls += 1
            return _ok_response(cached_tokens=8)

        primary = OpenCodeGoProvider(
            "oc-key", "deepseek-v4.1-flash", retries=0,
            transport=httpx.MockTransport(primary_handler),
        )
        secondary = OpenAICompatibleProvider(
            "https://backup.test/v1", "bk-key", "backup-model", retries=0,
            transport=httpx.MockTransport(fallback_handler),
        )
        chain = FallbackProvider([primary, secondary])
        calls, usage = await chain.decide([{"role": "user", "content": "halo"}], [])
        await chain.aclose()

        assert primary_calls == 1
        assert fallback_calls == 1
        assert calls[0].name == "act"
        assert chain.model == "backup-model"
        assert usage["provider"] == "openai-compatible"
        assert usage["fallbacks"] == [
            {"from": "opencode-go", "to": "openai-compatible", "reason": expected_reason}
        ]


async def test_fallback_does_not_move_on_400_bad_argument():
    fallback_called = False

    def primary_handler(req: httpx.Request) -> httpx.Response:
        return httpx.Response(400, json={"error": {"message": "bad argument"}})

    def fallback_handler(req: httpx.Request) -> httpx.Response:
        nonlocal fallback_called
        fallback_called = True
        return _ok_response()

    chain = FallbackProvider([
        OpenCodeGoProvider("oc-key", retries=0, transport=httpx.MockTransport(primary_handler)),
        OpenAICompatibleProvider("https://backup.test/v1", "bk-key", "m2", retries=0,
                                 transport=httpx.MockTransport(fallback_handler)),
    ])
    with pytest.raises(httpx.HTTPStatusError):
        await chain.decide([{"role": "user", "content": "halo"}], [])
    await chain.aclose()
    assert fallback_called is False


async def test_fallback_moves_land_as_additive_fields_on_model_call_event(aconn):
    def primary_handler(req: httpx.Request) -> httpx.Response:
        return httpx.Response(429, json={"error": {"message": "rate limit"}})

    def fallback_handler(req: httpx.Request) -> httpx.Response:
        return _ok_response(cached_tokens=4)

    chain = FallbackProvider([
        OpenCodeGoProvider("oc-key", retries=0, transport=httpx.MockTransport(primary_handler)),
        OpenAICompatibleProvider("https://backup.test/v1", "bk-key", "backup-model", retries=0,
                                 transport=httpx.MockTransport(fallback_handler)),
    ])
    cfg = settings()
    pfc = Reasoning(chain, settings=cfg)
    start = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
    kernel = build(aconn, clock=FakeClock(start), pfc=pfc, settings=cfg)
    put_task = asyncio.create_task(kernel.put(Event(
        id=None, ts=start, kind="percept", who="person:owner", place="space:tg:1",
        body="telegram", payload={"description": "halo", "addressed": True, "place_kind": "direct"},
    )))
    await asyncio.sleep(0)
    await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put_task, 1.0)

    row = await (await aconn.execute("SELECT payload FROM events WHERE kind = 'model.call'")).fetchone()
    payload = row[0]
    assert payload["model"] == "backup-model"
    assert payload["provider"] == "openai-compatible"
    assert payload["fallbacks"] == [
        {"from": "opencode-go", "to": "openai-compatible", "reason": "http_429"}
    ]


def test_load_provider_config_and_build_provider_for_fallbacks(tmp_path, monkeypatch):
    for key in ("LLM_PROVIDER", "LLM_MODEL", "LLM_FALLBACKS", "LLM_BASE_URL", "LLM_API_KEY",
                "OPENCODE_API_KEY", "OPENCODE_MODEL",
                "OPENAI_COMPATIBLE_API_KEY", "OPENAI_COMPATIBLE_MODEL", "OPENAI_COMPATIBLE_BASE_URL"):
        monkeypatch.delenv(key, raising=False)
    env_file = tmp_path / ".env"
    env_file.write_text(
        "LLM_PROVIDER=opencode-go\n"
        "LLM_MODEL=deepseek-v4.1-flash\n"
        "LLM_FALLBACKS=openai-compatible\n"
        "OPENCODE_API_KEY=oc-secret\n"
        "OPENAI_COMPATIBLE_BASE_URL=https://alt.test/v1\n"
        "OPENAI_COMPATIBLE_API_KEY=alt-secret\n"
        "OPENAI_COMPATIBLE_MODEL=alt-model\n",
        encoding="utf-8",
    )
    cfg = load_provider_config(env_file)
    assert cfg["provider"] == "opencode-go"
    assert cfg["fallbacks"] == ["openai-compatible"]
    from monoagens.infra.config import load_env_file
    chain = build_provider(cfg, load_env_file(env_file))
    assert chain.name == "opencode-go"
    assert chain.model == "deepseek-v4.1-flash"
    assert len(chain._providers) == 2
    assert chain._providers[1].model == "alt-model"
