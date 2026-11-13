"""WhatsApp CLI helpers: QR pairing and throwaway-device history backfill."""

from __future__ import annotations

import asyncio
import contextlib
import pathlib
import sys
from typing import Any

from monoagens.adapters.whatsapp.history_sync import history_props


def _secure_db(path: pathlib.Path) -> None:
    if path.is_file():
        with contextlib.suppress(OSError):
            path.chmod(0o600)


async def pair(session_db: str | pathlib.Path = "var/whatsapp.db", *,
               timeout: float = 300.0) -> int:
    """Pair the WhatsApp session in the foreground, printing a compact terminal QR."""
    import segno
    from neonize.aioze.client import NewAClient
    from neonize.aioze.events import ConnectedEv

    session = pathlib.Path(session_db)
    session.parent.mkdir(parents=True, exist_ok=True)
    done = asyncio.Event()
    client = NewAClient(str(session), props=history_props(False))

    async def on_qr(_accepted: Any, data: bytes) -> None:
        print("\nScan this QR in WhatsApp > Linked devices:\n", flush=True)
        rendered = segno.make_qr(data).terminal(compact=True)
        if rendered:
            print(rendered, flush=True)

    @client.event(ConnectedEv)
    async def on_connected(_accepted: Any, _ev: Any) -> None:
        _secure_db(session)
        print(f"\nWhatsApp paired: {session}\n", flush=True)
        done.set()

    client.event.qr(on_qr)
    try:
        await client.connect()
        await asyncio.wait_for(done.wait(), timeout)
    except TimeoutError:
        print(f"\nWhatsApp pairing timed out after {timeout:.0f} seconds.\n", file=sys.stderr)
        return 1
    finally:
        with contextlib.suppress(Exception):
            await client.stop()
    return 0


async def main(argv: list[str] | None = None) -> int:
    """CLI entry point: `python -m monoagens.adapters.whatsapp.cli pair [session_db]`."""
    args = list(sys.argv[1:] if argv is None else argv)
    cmd = args[0] if args else "pair"
    if cmd == "pair":
        target = args[1] if len(args) > 1 else "var/whatsapp.db"
        return await pair(target)
    print(f"unknown command: {cmd}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
