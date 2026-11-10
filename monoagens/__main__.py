"""Entry point: wire the kernel, the clock, and the Telegram body.

Only this module knows both the kernel and a concrete body (Invariant 1).
"""

from __future__ import annotations

import asyncio
import os

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.bodies.telegram import TelegramBody
from monoagens.clock import Clock
from monoagens.config import load_clock_tick_seconds, load_llm_config, load_owner_ids
from monoagens.kernel import Kernel
from monoagens.pfc import PFC


async def main() -> None:
    llm = load_llm_config()
    if not llm["base_url"] or not llm["model"]:
        raise RuntimeError(
            "PFC is not configured: set LLM_BASE_URL and LLM_MODEL (and LLM_API_KEY) in .env"
        )
    pfc = PFC(llm["base_url"], llm["api_key"], llm["model"])

    async with await psycopg.AsyncConnection.connect(os.environ["DATABASE_URL"]) as conn:
        await register_vector_async(conn)
        clock = Clock(tick_seconds=load_clock_tick_seconds())
        kernel = Kernel(conn, clock=clock, pfc=pfc)
        body = TelegramBody(
            os.environ["TELEGRAM_BOT_TOKEN"],
            owner_ids=load_owner_ids(),
            get_cursor=lambda: kernel.cursor("telegram"),
        )

        stop = asyncio.Event()
        tasks = [
            asyncio.create_task(kernel.run(stop)),
            asyncio.create_task(body.run(kernel.put)),
            asyncio.create_task(clock.run(kernel.put, stop)),
        ]
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
