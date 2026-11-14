"""Send one test message through the soul's own WhatsApp session and print what WhatsApp says.

This answers the one question the Log cannot: when `act.result` says `done` and the person sees
nothing, does the *session* fail to send, or does the message go somewhere else? The script uses
the same client and the same `send_say` the adapter does, so a difference is the session's, not
ours.

Stop the running soul first — neonize holds the session file, and two clients on one session fight
over it. Then:

    uv run python scripts/whatsapp_send_test.py 6285111242960 "tes koneksi"

The number and text come from the command line only; the session file holds the keys and is never
printed.
"""

from __future__ import annotations

import asyncio
import pathlib
import sys

SESSION = pathlib.Path("var/whatsapp.db")


async def main(argv: list[str]) -> int:
    """Connect with the stored session, send one line, and print the whole server answer."""
    if len(argv) < 3:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2
    recipient, text = argv[1].lstrip("+"), argv[2]
    if not SESSION.is_file():
        print(f"no session at {SESSION}", file=sys.stderr)
        return 1

    from neonize.aioze.client import NewAClient
    from neonize.aioze.events import ConnectedEv
    from neonize.utils.jid import build_jid

    from monoagens.adapters.whatsapp.identity import WhatsAppDirectory
    from monoagens.adapters.whatsapp.send import send_say

    client = NewAClient(str(SESSION))
    connected = asyncio.Event()
    me: dict[str, str] = {}

    @client.event(ConnectedEv)
    async def _on_connected(_client, event):  # noqa: ARG001 - the library's signature
        build = getattr(event, "Build", None)
        me["jid"] = str(getattr(build, "JID", "") or "")
        me["push_name"] = str(getattr(build, "PushName", "") or "")
        print("connected as:", me["jid"] or "?", "/", me["push_name"])
        connected.set()

    task = asyncio.create_task(client.connect())
    try:
        try:
            await asyncio.wait_for(connected.wait(), timeout=30)
        except asyncio.TimeoutError:
            print("never connected within 30 s", file=sys.stderr)
            return 1

        jid = build_jid(recipient, "s.whatsapp.net")
        print("direct client send to:", jid)
        direct = await client.send_message(jid, text)
        print("direct response:", direct)

        print("adapter path (send_say) ...")
        directory = WhatsAppDirectory()
        result = await send_say(client, directory, _NoFiles(), f"space:wa:{recipient}@s.whatsapp.net",
                                {"text": f"{text} (lewat jalur adapter)"})
        print("adapter response:", result)
        await asyncio.sleep(5)
        return 0
    finally:
        task.cancel()
        try:
            await client.disconnect()
        except Exception as exc:  # the point is the send, not the goodbye
            print("disconnect:", type(exc).__name__)


class _NoFiles:
    """An attachment door with nothing to open; this test sends text only."""

    async def open(self, ref: str, *, context: str = "") -> tuple[bytes, dict[str, str]]:
        raise FileNotFoundError(ref)


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main(sys.argv)))
