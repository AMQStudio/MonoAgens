"""Entry point: wire the kernel, the clock, and the adapters the configuration enables."""

from __future__ import annotations

import asyncio
import contextlib
from collections.abc import Mapping
from typing import Any

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens.boot import build, warm
from monoagens.core import identity, jobs
from monoagens.core.reasoning import Reasoning
from monoagens.infra import config
from monoagens.infra.clock import Clock
from monoagens.infra.embedding import FastEmbedder
from monoagens.infra.migrate import migrate
from monoagens.infra.prior import load_settings
from monoagens.infra.providers import build_provider
from monoagens.middleware.identity import ensure as ensure_pairing
from monoagens.middleware.loader import Loader
from monoagens.types import Event


async def supervise_adapter(
    kernel: Any,
    adapter: Any,
    stop: asyncio.Event,
    *,
    retry: Mapping[str, float] | None = None,
) -> None:
    """Supervise one adapter's `run()`: record exceptions as `error` events and restart alone."""
    cfg = dict(retry) if retry is not None else config.adapter_retry(kernel.settings)
    initial = float(cfg["initial_seconds"])
    maximum = float(cfg["max_seconds"])
    factor = float(cfg["factor"])
    name = str(getattr(adapter, "name", "") or type(adapter).__name__)
    backoff = initial
    while not stop.is_set():
        started = asyncio.get_running_loop().time()
        try:
            await adapter.run(kernel.put)
            backoff = initial
            if stop.is_set():
                return
            delay = backoff
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            detail = jobs.describe_failure(exc)
            kernel.enqueue(Event(
                id=None, ts=kernel.clock.now(), kind="error", who=name, place="", body=name,
                payload={"adapter": name, "error": f"{name}: {detail}"},
            ))
            elapsed = asyncio.get_running_loop().time() - started
            if elapsed >= initial:
                backoff = initial
            delay = min(backoff, maximum)
            backoff = min(backoff * factor, maximum)
        with contextlib.suppress(TimeoutError):
            await asyncio.wait_for(stop.wait(), timeout=delay)


async def main() -> None:
    """Run the kernel with the clock and the enabled adapters until stopped."""
    root, env = config.DEFAULT_ROOT, config.load_env()
    llm = config.load_provider_config()
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
        retries = int(settings["constants"]["model_call_retries"])
        soul_id = str(settings["identity"].get("name") or "monoagens")
        port = build_provider(llm, env, retries=retries, soul_id=soul_id, notes=notes)
        pfc = Reasoning(port, settings=settings)
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
        retry = config.adapter_retry(settings)
        tasks = [asyncio.create_task(coro) for coro in (
            kernel.run(stop),
            *(supervise_adapter(kernel, adapter, stop, retry=retry) for adapter in loaded.adapters),
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
