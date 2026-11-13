"""Provider selection and ordered fallback over ModelPort drivers."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

import httpx

from monoagens.core.reasoning import ToolCall
from monoagens.infra.providers.openai_compatible import OpenAICompatibleProvider
from monoagens.infra.providers.opencode_go import (
    DEFAULT_OPENCODE_GO_MODEL,
    OPENCODE_GO_ENDPOINT,
    OpenCodeGoProvider,
)

FALLBACK_STATUSES = frozenset({402, 429})
_PROVIDER_PREFIX = {
    "opencode-go": "OPENCODE",
    "openai-compatible": "OPENAI_COMPATIBLE",
}


def is_fallback_error(exc: Exception) -> str | None:
    """Return a short reason if exc warrants moving to the next provider, else None."""
    if isinstance(exc, httpx.TransportError):
        return type(exc).__name__
    if isinstance(exc, httpx.HTTPStatusError):
        code = exc.response.status_code
        if code in FALLBACK_STATUSES or 500 <= code < 600:
            return f"http_{code}"
        return None
    return None


class FallbackProvider:
    """Ordered ModelPort chain that records provider used and fallback moves in usage."""

    def __init__(self, providers: Sequence[Any]) -> None:
        """Wrap one or more provider drivers in preference order."""
        if not providers:
            raise ValueError("FallbackProvider requires at least one provider")
        self._providers = list(providers)
        self.model = str(getattr(self._providers[0], "model", "") or "")

    @property
    def name(self) -> str:
        """Return the primary provider name."""
        return str(getattr(self._providers[0], "name", "") or "")

    async def aclose(self) -> None:
        """Close every wrapped provider."""
        for provider in self._providers:
            await provider.aclose()

    async def decide(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        tool_choice: str = "required",
    ) -> tuple[list[ToolCall], dict[str, Any]]:
        """Try providers in order; move only on quota, rate-limit, transport, or 5xx errors."""
        moves: list[dict[str, str]] = []
        last = len(self._providers) - 1
        for index, provider in enumerate(self._providers):
            try:
                calls, usage = await provider.decide(messages, tools, tool_choice=tool_choice)
                self.model = str(getattr(provider, "model", "") or "")
                enriched = dict(usage)
                enriched["provider"] = str(getattr(provider, "name", "") or "")
                if moves:
                    enriched["fallbacks"] = list(moves)
                return calls, enriched
            except Exception as exc:
                reason = is_fallback_error(exc)
                if reason is None or index == last:
                    raise
                next_provider = self._providers[index + 1]
                moves.append(
                    {
                        "from": str(getattr(provider, "name", "") or ""),
                        "to": str(getattr(next_provider, "name", "") or ""),
                        "reason": reason,
                    }
                )
        raise RuntimeError("unreachable: provider loop always returns or raises")


def _fallback_env(env: Mapping[str, str], provider_name: str, key: str) -> str:
    prefix = _PROVIDER_PREFIX.get(provider_name) or provider_name.upper().replace("-", "_")
    alt_prefix = provider_name.upper().replace("-", "_")
    return (
        env.get(f"{prefix}_{key}")
        or env.get(f"{alt_prefix}_{key}")
        or ""
    ).strip()


def _make_single(
    provider_name: str,
    *,
    api_key: str,
    model: str,
    base_url: str,
    effort: str,
    parallel_tool_calls: bool,
    retries: int,
    soul_id: str,
    transport: httpx.AsyncBaseTransport | None = None,
    client: httpx.AsyncClient | None = None,
    timeout: float = 30.0,
    notes: list[str] | None = None,
) -> Any:
    name = provider_name.strip().lower() or "openai-compatible"
    if name == "opencode-go":
        return OpenCodeGoProvider(
            api_key,
            model or DEFAULT_OPENCODE_GO_MODEL,
            base_url=base_url or OPENCODE_GO_ENDPOINT,
            soul_id=soul_id,
            effort=effort,
            retries=retries,
            transport=transport,
            client=client,
            timeout=timeout,
            parallel_tool_calls=parallel_tool_calls,
            notes=notes,
        )
    if name == "openai-compatible":
        if not base_url or not model:
            raise RuntimeError(
                "openai-compatible requires base_url and model "
                "(LLM_BASE_URL and LLM_MODEL, or OPENAI_COMPATIBLE_* for fallback)"
            )
        return OpenAICompatibleProvider(
            base_url,
            api_key,
            model,
            effort=effort,
            retries=retries,
            transport=transport,
            client=client,
            timeout=timeout,
            parallel_tool_calls=parallel_tool_calls,
        )
    raise ValueError(f"Unknown LLM_PROVIDER: {provider_name!r}")


def build_provider(
    llm: Mapping[str, Any],
    env: Mapping[str, str],
    *,
    retries: int = 1,
    soul_id: str = "monoagens",
    transport: httpx.AsyncBaseTransport | None = None,
    client: httpx.AsyncClient | None = None,
    timeout: float = 30.0,
    notes: list[str] | None = None,
) -> FallbackProvider:
    """Build primary and fallback providers from config and environment."""
    primary_name = str(llm.get("provider") or "openai-compatible").strip().lower()
    primary_key = str(llm.get("api_key") or "")
    if not primary_key and primary_name == "opencode-go":
        primary_key = _fallback_env(env, primary_name, "API_KEY")
    primary_model = str(llm.get("model") or "")
    if not primary_model and primary_name == "opencode-go":
        primary_model = DEFAULT_OPENCODE_GO_MODEL
    primary = _make_single(
        primary_name,
        api_key=primary_key,
        model=primary_model,
        base_url=str(llm.get("base_url") or ""),
        effort=str(llm.get("effort") or ""),
        parallel_tool_calls=bool(llm.get("parallel_tool_calls", True)),
        retries=retries,
        soul_id=soul_id,
        transport=transport,
        client=client,
        timeout=timeout,
        notes=notes,
    )
    chain = [primary]
    for fb_name in llm.get("fallbacks") or ():
        clean = str(fb_name).strip().lower()
        if not clean:
            continue
        fb_key = _fallback_env(env, clean, "API_KEY") or str(llm.get("api_key") or "")
        fb_model = _fallback_env(env, clean, "MODEL")
        fb_base = _fallback_env(env, clean, "BASE_URL")
        if clean == "openai-compatible":
            fb_key = fb_key or env.get("LLM_API_KEY", "")
            fb_base = fb_base or env.get("LLM_BASE_URL", "")
            fb_model = fb_model or env.get("LLM_MODEL", "")
        chain.append(
            _make_single(
                clean,
                api_key=fb_key,
                model=fb_model,
                base_url=fb_base,
                effort=str(llm.get("effort") or ""),
                parallel_tool_calls=bool(llm.get("parallel_tool_calls", True)),
                retries=retries,
                soul_id=soul_id,
                transport=transport,
                client=client,
                timeout=timeout,
                notes=notes,
            )
        )
    return FallbackProvider(chain)
