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
from monoagens.config import load_clock_tick_seconds, load_owner_ids
from monoagens.kernel import Kernel


async def main() -> None:
    async with await psycopg.AsyncConnection.connect(os.environ["DATABASE_URL"]) as conn:
        await register_vector_async(conn)
        clock = Clock(tick_seconds=load_clock_tick_seconds())
        kernel = Kernel(conn, clock=clock)
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


if __name__ == "__main__":
    asyncio.run(main())
