"""Scenario: "send our finished work to Budi" — read, share, approve, forward (4.6)."""

from __future__ import annotations

import asyncio
import pathlib
import shutil
from datetime import datetime, timezone
from typing import Any

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, read_state
from monoagens.middleware.loader import Loader
from monoagens.types import Action, Event
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
OWNER_DM = "space:tg:1001"
BUDI = "space:wa:628110001"


class FileTest:
    """A test adapter that can send a text and a file, as WhatsApp will (Task 5)."""

    name = "filetest"
    PERMISSIONS = {"say": "guest", "send": "guest"}
    FILES = {"per_message": 2, "max_bytes": 50_000_000, "mimes": ()}

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return ({"name": "say", "description": "Send a message.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": "filetest.say"},
                {"name": "send_file", "description": "Send a file with a note.",
                 "schema": {"type": "object",
                            "properties": {"text": {"type": "string"},
                                           "attachments": {"type": "array",
                                                           "items": {"type": "string"}}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": "filetest.send"})

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        return None


class Scripted:
    """A reasoning that plays one scripted list per decide call, then nothing."""

    def __init__(self, rounds: list[list[tuple[str, dict]]], replace=None) -> None:
        self.rounds = list(rounds)
        self.replace = replace
        self.contexts: list[Any] = []
        self.last_usage: dict[str, int] = {}

    async def decide(self, context: Any):
        """Play the next round only on the two wakes this scenario is about."""
        self.contexts.append(context)
        text = str((context.trigger.payload or {}).get("description") or "")
        if (context.results or not ("kirim" in text or "approved" in text
                                    or context.trigger.kind == "intent")):
            return []
        found = self.rounds.pop(0) if self.rounds else []
        if self.replace is not None:
            found = self.replace(found)
        return [ToolCall(name, dict(arguments)) for name, arguments in found]

    async def aclose(self):
        return None


async def test_the_work_is_read_shared_approved_and_sent(aconn, tmp_path):
    rounds: list[list[tuple[str, dict]]] = [
        [("act", {"capability": "workspace.read", "arguments": {"path": "laporan.md"}}),
         ("act", {"capability": "self.share",
                  "arguments": {"items": ["EV-FILE"], "to": "person:budi"}}),
         ("act", {"capability": "filetest.send_file",
                  "arguments": {"text": "ini laporannya",
                                "attachments": ["att-PLACEHOLDER"],
                                "to": "person:budi"}})],
        [("act", {"capability": "filetest.send_file",
                  "arguments": {"text": "ini laporannya",
                                "attachments": ["att-PLACEHOLDER"],
                                "to": "person:budi"}})],
        [("act", {"capability": "filetest.send_file",
                  "arguments": {"text": "ini laporannya",
                                "attachments": ["att-PLACEHOLDER"]}})],
    ]
    def resolve(batch):
        """Fill the attachment placeholder once the store knows the real ref."""
        found = []
        for name, arguments in batch:
            inner = dict(arguments.get("arguments") or {})
            if "attachments" in inner:
                inner["attachments"] = [REFS.get(ref, ref) for ref in inner["attachments"]]
            if "items" in inner:
                inner["items"] = [IDENTS.get(item, item) for item in inner["items"]]
            arguments = {**arguments, "arguments": inner}
            found.append((name, arguments))
        return found

    REFS: dict[str, str] = {}
    IDENTS: dict[str, int] = {}
    pfc = Scripted(rounds, resolve)
    root = tmp_path / "deployment"
    (root / "plugins").mkdir(parents=True)
    shutil.copytree(ROOT / "plugins" / "workspace", root / "plugins" / "workspace")
    values = settings(plugins={"workspace": {"enabled": True}})
    values["roles"] = {"fallback": "unverified", "unverified": {"base": "guest"},
                       "runner": {"base": "member", "approval": "yolo"}}
    values["people"] = {"person:owner": {"roles": ["runner"]},
                        "person:budi": {"roles": ["runner"]}}
    for place, actors in ((OWNER_DM, ["person:owner"]), (BUDI, ["person:budi"])):
        await append_event(aconn, ts=START, kind="place.audience", place=place,
                           payload={"place": place, "actors": actors})
    loader = Loader(root, env={})
    loaded = loader.load(values)
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=values)
    adapter = FileTest()
    kernel.register_adapter(adapter)
    await loader.install(kernel, loaded)
    store = kernel.attachments
    attachment = await store.put(b"# laporan\nisi pekerjaan kita", name="laporan.md")
    REFS["att-PLACEHOLDER"] = attachment["ref"]
    shared = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=OWNER_DM, body="telegram",
                                payload={"description": "ini laporannya", "place_kind": "direct",
                                         "data": {"attachments": [attachment]}})
    IDENTS["EV-FILE"] = int(shared.id)
    folder = root / "var" / "workspaces" / "person_owner"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "laporan.md").write_text("# laporan\nisi pekerjaan kita", encoding="utf-8")
    await append_event(aconn, ts=START, kind="percept", who="person:budi", place=BUDI,
                       body="filetest",
                       payload={"description": "halo", "place_kind": "direct"})

    ask = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                             place=OWNER_DM, body="telegram",
                             payload={"description": "kirim hasil kerjaan kita ke Budi",
                                      "place_kind": "direct", "addressed": True})
    put = asyncio.create_task(kernel.put(ask))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT kind, place, payload FROM events ORDER BY id")
    rows = [(kind, place, payload) for kind, place, payload in await cur.fetchall()]
    shared = [row for row in rows if row[0] == "share.granted"]
    asked = [row for row in rows if row[0] == "approval.requested"]
    intents = [row for row in rows if row[0] == "act.intent"]
    read = [row for row in rows if row[0] == "act.result"
            and "laporan" in str(row[2].get("result") or "")]

    assert read and "isi pekerjaan kita" in str(read[0][2]["result"])
    assert len(shared) == 1 and shared[0][2]["persons"] == ["person:budi"]
    assert len(asked) == 1 and asked[0][2]["reason"] == "forwarding a file"
    assert not intents, "nothing leaves before the owner says yes"

    answer = asyncio.create_task(kernel.put(Event(
        id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
        body="telegram", payload={"description": "ya", "place_kind": "direct"}, description="ya")))
    await asyncio.sleep(0)
    await kernel.step()
    await answer
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT kind, place, payload FROM events ORDER BY id")
    rows = [(kind, place, payload) for kind, place, payload in await cur.fetchall()]
    intents = [row for row in rows if row[0] == "act.intent"]
    sent = [action for action in adapter.actions if action.kind == "send_file"]
    assert intents and intents[0][2]["target"] == BUDI
    assert sent and sent[-1].place == BUDI
    assert attachment["ref"] in sent[-1].arguments["attachments"]
    assert await read_state(aconn, "alias:filetest") is None
