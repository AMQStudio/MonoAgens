"""OpenAI-compatible chat completions provider driver."""

from __future__ import annotations

import json
from typing import Any

import httpx

from monoagens.core.reasoning import ToolCall


def normalize_usage(raw: dict[str, Any]) -> dict[str, int]:
    """Normalize one response's token usage into the five tracked counters."""
    details = raw.get("prompt_tokens_details") or {}
    prompt = int(raw.get("prompt_tokens") or 0)
    reasoning = raw.get("completion_tokens_details") or {}
    completion = int(raw.get("completion_tokens") or 0)
    return {
        "prompt_tokens": prompt,
        "cached_tokens": int(details.get("cached_tokens") or 0),
        "completion_tokens": completion,
        "reasoning_tokens": int(reasoning.get("reasoning_tokens") or 0),
        "total_tokens": int(raw.get("total_tokens") or prompt + completion),
    }


class OpenAICompatibleProvider:
    """Chat completions client over an OpenAI-compatible HTTP endpoint."""

    name = "openai-compatible"

    def __init__(
        self,
        base_url: str,
        api_key: str,
        model: str,
        *,
        effort: str = "",
        retries: int = 1,
        transport: httpx.AsyncBaseTransport | None = None,
        client: httpx.AsyncClient | None = None,
        timeout: float = 30.0,
        parallel_tool_calls: bool = True,
        extra_headers: dict[str, str] | None = None,
    ) -> None:
        """Store endpoint, credentials, model, and transport configuration."""
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.model = model
        self.effort = effort.strip().lower()
        self.retries = max(0, int(retries))
        self.parallel_tool_calls = parallel_tool_calls
        self.extra_headers = dict(extra_headers or {})
        self._transport = transport
        self._client = client
        self._timeout = timeout

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
            self._client = httpx.AsyncClient(transport=self._transport, timeout=self._timeout)
        return self._client

    async def aclose(self) -> None:
        """Close the underlying HTTP client."""
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    def _endpoint(self) -> str:
        if self.base_url.endswith("/chat/completions"):
            return self.base_url
        return f"{self.base_url}/chat/completions"

    def _headers(self) -> dict[str, str]:
        headers = dict(self.extra_headers)
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        return headers

    def _body(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        tool_choice: str,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": tool_choice,
        }
        if self.parallel_tool_calls:
            body["parallel_tool_calls"] = True
        if self.effort:
            body["reasoning_effort"] = self.effort
        return body

    async def _post(self, body: dict[str, Any], headers: dict[str, str]) -> httpx.Response:
        """POST one completion, retrying a transport failure up to `self.retries` times."""
        url = self._endpoint()
        for attempt in range(self.retries + 1):
            try:
                return await self._http().post(url, json=body, headers=headers)
            except httpx.TransportError:
                if attempt == self.retries:
                    raise
        raise RuntimeError("unreachable: the retry loop always returns or raises")

    async def decide(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        tool_choice: str = "required",
    ) -> tuple[list[ToolCall], dict[str, Any]]:
        """Run one chat completion call and return its tool calls and token usage."""
        response = await self._post(self._body(messages, tools, tool_choice), self._headers())
        response.raise_for_status()
        try:
            payload = response.json()
            raw_calls = payload["choices"][0]["message"].get("tool_calls") or []
        except (KeyError, IndexError, TypeError) as exc:
            raise ValueError(f"Malformed chat completion response: {exc}") from exc
        calls = [
            ToolCall(call["function"]["name"], json.loads(call["function"]["arguments"] or "{}"))
            for call in raw_calls
        ]
        return calls, normalize_usage(payload.get("usage") or {})
