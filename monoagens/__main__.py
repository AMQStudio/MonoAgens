"""Entry point: wire the kernel, the clock, and the enabled adapters."""

from __future__ import annotations

import asyncio
import os

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.adapters.cli import CLIBody
from monoagens.adapters.telegram import TelegramBody
from monoagens.boot import build
from monoagens.core import identity
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import Clock
from monoagens.infra.config import load_llm_config, tick_seconds
from monoagens.infra.embedding import FastEmbedder
from monoagens.infra.migrate import migrate
from monoagens.infra.prior import load_settings
from monoagens.middleware.identity import ensure as ensure_pairing


async def main() -> None:
    """Run the kernel with the clock and the enabled adapters until stopped."""
    llm = load_llm_config()
    if not llm["base_url"] or not llm["model"]:
        raise RuntimeError(
            "PFC is not configured: set LLM_BASE_URL and LLM_MODEL (and LLM_API_KEY) in .env"
        )
    settings = load_settings()
    async with await psycopg.AsyncConnection.connect(os.environ["DATABASE_URL"],
                                                     autocommit=True) as conn:
        await migrate(conn)
        await register_vector_async(conn)
        clock = Clock(tick_seconds=tick_seconds(settings))
        settings["identity"] = await identity.ensure(conn, settings["identity"],
                                                     now=clock.now())
        pfc = Reasoning(llm["base_url"], llm["api_key"], llm["model"], effort=llm["effort"],
                  settings=settings)
        kernel = build(conn, clock=clock, pfc=pfc, settings=settings,
                       embedder=FastEmbedder())
        code = await ensure_pairing(conn, now=clock.now())
        if code:
            print(f"pairing code: {code} — send it from any adapter to become the owner",
                  flush=True)
        adapters = []
        enabled = settings.get("bodies") or {}
        if (enabled.get("telegram") or {}).get("enabled"):
            adapters.append(TelegramBody(os.environ["TELEGRAM_BOT_TOKEN"],
                                       get_cursor=lambda: kernel.cursor("telegram")))
        if (enabled.get("cli") or {}).get("enabled"):
            adapters.append(CLIBody())
        for adapter in adapters:
            kernel.register_adapter(adapter)
        stop = asyncio.Event()
        tasks = [asyncio.create_task(coro) for coro in (
            kernel.run(stop),
            *(adapter.run(kernel.put) for adapter in adapters),
            kernel.clock.run(kernel.put, stop))]
        try:
            await asyncio.gather(*tasks)
        finally:
            stop.set()
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            await pfc.aclose()


if __name__ == "__main__":
    asyncio.run(main())
