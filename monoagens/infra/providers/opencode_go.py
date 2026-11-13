"""OpenCode Go provider driver (OpenAI-compatible endpoint with session routing)."""

from __future__ import annotations

import uuid
from typing import Any

import httpx

from monoagens.infra.providers.openai_compatible import OpenAICompatibleProvider

OPENCODE_GO_ENDPOINT = "https://opencode.ai/inference/go/openai/v1/chat/completions"
DEFAULT_OPENCODE_GO_MODEL = "deepseek-v4.1-flash"
_EFFORT_MAP = {"low": "low", "medium": "high", "high": "high", "max": "max"}


def session_id_for_soul(soul_id: str) -> str:
    """Derive a deterministic UUIDv5 session id from a stable soul identifier."""
    clean = (soul_id or "monoagens").strip() or "monoagens"
    return str(uuid.uuid5(uuid.NAMESPACE_DNS, f"monoagens:soul:{clean}"))


def map_opencode_effort(effort: str) -> tuple[str, str | None]:
    """Map generic effort to OpenCode Go's (low, high, max) and return an optional note."""
    clean = effort.strip().lower()
    if not clean:
        return "", None
    mapped = _EFFORT_MAP.get(clean)
    if mapped is None:
        raise ValueError(f"Unsupported OpenCode Go reasoning effort: {effort!r}")
    if clean == "medium":
        return mapped, "opencode-go: mapped reasoning_effort 'medium' to 'high'"
    return mapped, None


class OpenCodeGoProvider(OpenAICompatibleProvider):
    """OpenCode Go chat completions driver with deterministic x-opencode-session header."""

    name = "opencode-go"

    def __init__(
        self,
        api_key: str,
        model: str = DEFAULT_OPENCODE_GO_MODEL,
        *,
        base_url: str = OPENCODE_GO_ENDPOINT,
        soul_id: str = "monoagens",
        session_id: str | None = None,
        effort: str = "",
        retries: int = 1,
        transport: httpx.AsyncBaseTransport | None = None,
        client: httpx.AsyncClient | None = None,
        timeout: float = 30.0,
        parallel_tool_calls: bool = True,
        notes: list[str] | None = None,
    ) -> None:
        """Configure the OpenCode Go endpoint, session header, and effort mapping."""
        mapped_effort, note = map_opencode_effort(effort)
        if note and notes is not None:
            notes.append(note)
        resolved_session = session_id or session_id_for_soul(soul_id)
        self.session_id = resolved_session
        self.effort_note = note
        super().__init__(
            base_url or OPENCODE_GO_ENDPOINT,
            api_key,
            model or DEFAULT_OPENCODE_GO_MODEL,
            effort=mapped_effort,
            retries=retries,
            transport=transport,
            client=client,
            timeout=timeout,
            parallel_tool_calls=parallel_tool_calls,
            extra_headers={"x-opencode-session": resolved_session},
        )
