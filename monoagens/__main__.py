"""Entry point: wire the kernel, the clock, and the adapters the configuration enables."""

from __future__ import annotations

import asyncio

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.boot import build, warm
from monoagens.core import identity
from monoagens.core.reasoning import Reasoning
from monoagens.infra import config
from monoagens.infra.clock import Clock
from monoagens.infra.embedding import FastEmbedder
from monoagens.infra.migrate import migrate
from monoagens.infra.prior import load_settings
from monoagens.middleware.identity import ensure as ensure_pairing
from monoagens.middleware.loader import Loader


async def main() -> None:
    """Run the kernel with the clock and the enabled adapters until stopped."""
    root, env = config.DEFAULT_ROOT, config.load_env()
    llm = config.load_llm_config()
    if not llm["base_url"] or not llm["model"]:
        raise RuntimeError(
            "PFC is not configured: set LLM_BASE_URL and LLM_MODEL (and LLM_API_KEY) in .env"
        )
    notes: list[str] = []
    settings = load_settings(notes=notes)
    database_url = env.get("DATABASE_URL")
    if not database_url:
        raise RuntimeError("DATABASE_URL is not set")
    async with await psycopg.AsyncConnection.connect(database_url, autocommit=True) as conn:
        await migrate(conn)
        await register_vector_async(conn)
        clock = Clock(tick_seconds=config.tick_seconds(settings))
        settings["identity"] = await identity.ensure(conn, settings["identity"],
                                                    now=clock.now())
        pfc = Reasoning(llm["base_url"], llm["api_key"], llm["model"], effort=llm["effort"],
                        settings=settings)
        kernel = build(conn, clock=clock, pfc=pfc, settings=settings, embedder=FastEmbedder())
        await warm(kernel)
        code = await ensure_pairing(conn, now=clock.now())
        if code:
            print(f"pairing code: {code} — send it from any adapter to become the owner",
                  flush=True)
        loader = Loader(root, env=env)
        loaded = loader.load(settings, notes=notes)
        await loader.install(kernel, loaded)
        print(loaded.report.render(), flush=True)
        stop = asyncio.Event()
        tasks = [asyncio.create_task(coro) for coro in (
            kernel.run(stop),
            *(adapter.run(kernel.put) for adapter in loaded.adapters),
            kernel.clock.run(kernel.put, stop))]
        try:
            await asyncio.gather(*tasks)
        finally:
            stop.set()
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            for adapter in loaded.adapters:
                closing = getattr(adapter, "aclose", None)
                if callable(closing):
                    await closing()
            await pfc.aclose()


if __name__ == "__main__":
    asyncio.run(main())
