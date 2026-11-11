"""Entry point: wire the kernel, the clock, and the bodies."""

from __future__ import annotations

import asyncio
import os

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.bodies.cli import CLIBody
from monoagens.bodies.telegram import TelegramBody
from monoagens.clock import Clock
from monoagens.config import load_clock_tick_seconds, load_llm_config, load_owner_ids
from monoagens.kernel import Kernel
from monoagens.memory import FastEmbedder
from monoagens.migrate import migrate
from monoagens.pairing import ensure as ensure_pairing
from monoagens.pairing import inbound as pairing_inbound
from monoagens.pfc import PFC


async def main() -> None:
    """Run the kernel with the clock and both bodies until stopped."""
    llm = load_llm_config()
    if not llm["base_url"] or not llm["model"]:
        raise RuntimeError(
            "PFC is not configured: set LLM_BASE_URL and LLM_MODEL (and LLM_API_KEY) in .env"
        )
    pfc = PFC(llm["base_url"], llm["api_key"], llm["model"])
    async with await psycopg.AsyncConnection.connect(os.environ["DATABASE_URL"]) as conn:
        await migrate(conn)
        await register_vector_async(conn)
        clock = Clock(tick_seconds=load_clock_tick_seconds())
        kernel = Kernel(conn, clock=clock, pfc=pfc, embedder=FastEmbedder())
        kernel.hooks.inbound.add("pairing", pairing_inbound)
        code = await ensure_pairing(conn, now=clock.now())
        if code:
            print(f"pairing code: {code} — send it from any organ to become the owner",
                  flush=True)
        telegram = TelegramBody(os.environ["TELEGRAM_BOT_TOKEN"], owner_ids=load_owner_ids(),
                                get_cursor=lambda: kernel.cursor("telegram"))
        cli = CLIBody()
        for body in (telegram, cli):
            kernel.register_body(body)
        stop = asyncio.Event()
        tasks = [asyncio.create_task(coro) for coro in (
            kernel.run(stop), telegram.run(kernel.put), cli.run(kernel.put),
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
