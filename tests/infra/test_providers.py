"""Tests for model provider drivers in monoagens/infra/providers/."""

from __future__ import annotations

import json

import httpx
import pytest

from monoagens.infra.providers import (
    OPENCODE_GO_ENDPOINT,
    OpenCodeGoProvider,
    session_id_for_soul,
)


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
