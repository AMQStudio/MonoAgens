"""Entry point: wire the kernel, the clock, and the enabled bodies."""

from __future__ import annotations

import asyncio
import os

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.bodies.cli import CLIBody
from monoagens.bodies.telegram import TelegramBody
from monoagens.clock import Clock
from monoagens.config import load_llm_config, tick_seconds
from monoagens.kernel import Kernel
from monoagens import identity
from monoagens.embedding import FastEmbedder
from monoagens.migrate import migrate
from monoagens.pairing import ensure as ensure_pairing
from monoagens.pairing import inbound as pairing_inbound
from monoagens.pfc import PFC
from monoagens.prior import load_settings


async def main() -> None:
    """Run the kernel with the clock and the enabled bodies until stopped."""
    llm = load_llm_config()
    if not llm["base_url"] or not llm["model"]:
        raise RuntimeError(
            "PFC is not configured: set LLM_BASE_URL and LLM_MODEL (and LLM_API_KEY) in .env"
        )
    settings = load_settings()
    async with await psycopg.AsyncConnection.connect(os.environ["DATABASE_URL"]) as conn:
        await migrate(conn)
        await register_vector_async(conn)
        clock = Clock(tick_seconds=tick_seconds(settings))
        settings["identity"] = await identity.ensure(conn, settings["identity"],
                                                     now=clock.now())
        pfc = PFC(llm["base_url"], llm["api_key"], llm["model"], effort=llm["effort"],
                  settings=settings)
        kernel = Kernel(conn, clock=clock, pfc=pfc, settings=settings,
                        embedder=FastEmbedder())
        kernel.hooks.inbound.add("pairing", pairing_inbound)
        code = await ensure_pairing(conn, now=clock.now())
        if code:
            print(f"pairing code: {code} — send it from any organ to become the owner",
                  flush=True)
        bodies = []
        enabled = settings.get("bodies") or {}
        if (enabled.get("telegram") or {}).get("enabled"):
            bodies.append(TelegramBody(os.environ["TELEGRAM_BOT_TOKEN"],
                                       get_cursor=lambda: kernel.cursor("telegram")))
        if (enabled.get("cli") or {}).get("enabled"):
            bodies.append(CLIBody())
        for body in bodies:
            kernel.register_body(body)
        stop = asyncio.Event()
        tasks = [asyncio.create_task(coro) for coro in (
            kernel.run(stop),
            *(body.run(kernel.put) for body in bodies),
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
