"""Interaction: thinking, steps and approvals — what is shown, to whom, and its fallback."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.core.hooks import Continue
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, write_state
from monoagens.middleware import interaction
from monoagens.middleware.loader import Loader
from monoagens.contract import Action
from monoagens.types import Event
from tests.support import settings

ROOT = __import__("pathlib").Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
OWNER_DM = "space:tg:1001"
GROUP = "space:tg:-500"


class Screen:
    """A body that records what it showed and what it sent."""

    PERMISSIONS = {"say": "guest", "run": "guest"}

    def __init__(self, name: str = "telegram", shows: tuple[str, ...] = ("typing", "edit")) -> None:
        self.name, self.shows = name, shows
        self.actions: list[Action] = []
        self.shown: list[tuple[str, str, dict[str, Any]]] = []
        self.rendered: list[Any] = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": "telegram.say"},
                {"name": "run", "description": "Work.",
                 "schema": {"type": "object", "properties": {}},
                 "communicative": False, "long_running": True, "identity_changing": False,
                 "facts": (), "permission": "telegram.run"},)

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        return {"text": "hasil" if action.capability == "run" else None}

    async def render(self, event: Any) -> None:
        """Render one lifecycle event, recording it as a body does (§2.4.5)."""
        self.rendered.append(event)

    async def show(self, place: str, kind: str, **data: Any) -> None:
        self.shown.append((place, kind, dict(data)))


async def _kernel(aconn, *, body=None, interaction_settings=None):
    values = settings()
    if interaction_settings is not None:
        values["interaction"] = interaction_settings
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    kernel.register_adapter(body or Screen())
    return kernel


async def _percept(aconn, place=OWNER_DM, who="person:owner", text="halo"):
    """Record one percept and report who can perceive its place (the one privacy fact)."""
    actors = [who, "person:owner"] if place == OWNER_DM else [who, "person:owner",
                                                              "person:other"]
    await append_event(aconn, ts=START, kind="place.audience", place=place,
                       payload={"place": place, "actors": actors})
    return await append_event(aconn, ts=START, kind="percept", who=who, place=place,
                              body="telegram", payload={"description": text})


async def test_thinking_shows_in_the_owners_chat_and_never_in_a_group(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "aku ragu"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=OWNER_DM, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})

    rule = interaction.narrate(kernel)
    await rule(later, {"conn": aconn, "now": START})

    rendered = kernel.tools.adapters["telegram"].rendered
    assert any(item.name == "thought" and "(berpikir) aku ragu" in str(item.data.get("text"))
               for item in rendered)

    group = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=GROUP, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    await rule(group, {"conn": aconn, "now": START})
    assert not any(item.place == GROUP
                   for item in kernel.tools.adapters["telegram"].rendered)


async def test_a_thought_is_shown_once(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    note = await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "satu"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=OWNER_DM, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    rule = interaction.narrate(kernel)

    await rule(later, {"conn": aconn, "now": START})
    await rule(later, {"conn": aconn, "now": START})

    rendered = [str(item.data.get("text")) for item in kernel.tools.adapters["telegram"].rendered
                if item.name == "thought"]
    assert rendered.count("(berpikir) satu") == 1
    assert note.id is not None


async def test_a_long_act_shows_one_step_and_a_short_one_nothing(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    long_result = Event(id=None, ts=START, kind="act.result", who="person:owner",
                        place=OWNER_DM, body="telegram",
                        payload={"capability": "telegram.run", "result": {"text": "x" * 300}})
    short_result = Event(id=None, ts=START, kind="act.result", who="person:owner",
                         place=OWNER_DM, body="telegram",
                         payload={"capability": "telegram.say", "result": {"text": "hai"}})
    rule = interaction.narrate(kernel)

    await rule(long_result, {"conn": aconn, "now": START})
    await rule(short_result, {"conn": aconn, "now": START})

    rendered = [str(item.data.get("text")) for item in kernel.tools.adapters["telegram"].rendered
                if item.name == "progress"]
    steps = [text for text in rendered if text.startswith("(langkah)")]
    assert len(steps) == 1 and "telegram.run" in steps[0]
    assert len(steps[0]) < 260
    assert trigger.id is not None


async def test_a_thought_the_place_may_not_see_is_dropped(aconn):
    kernel = await _kernel(aconn, body=Screen())
    private = await append_event(aconn, ts=START, kind="percept", who="person:other",
                                 place="space:wa:other", body="whatsapp",
                                 payload={"description": "rahasia", "place_kind": "direct"})
    trigger = await _percept(aconn, place=GROUP, who="person:budi", text="halo")
    await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "rahasia"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:budi",
                               place=GROUP, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    kernel.roles = kernel.roles.with_person("person:owner", ())

    async def also_owner(item, ctx):
        return Continue()

    kernel.hooks.inbound.add("always", also_owner)
    rule = interaction.narrate(kernel)
    await rule(later, {"conn": aconn, "now": START})

    assert not kernel.tools.adapters["telegram"].shown
    assert private.id is not None


async def test_approval_and_brake_send_neutral_choices_in_configured_language(aconn):
    kernel = await _kernel(aconn, body=_RichScreen(shows=("typing",)))
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram",
                       payload={"description": "halo", "place_kind": "direct"})
    trigger = await _percept(aconn)

    place = await interaction.request(kernel, trigger, 7, "workspace.run", {"command": "ls"},
                                      "perlu konfirmasi")

    assert place == OWNER_DM
    sent = kernel.tools.adapters["telegram"].actions[-1].arguments
    assert sent["text"].startswith("Perlu izin: workspace.run")
    assert "person:owner" in sent["text"]
    assert sent["choices"] == ["Ya", "Tidak"]
    assert not kernel.tools.adapters["telegram"].shown

    # Switching soul language to English switches catalog labels and prompts cleanly.
    kernel.settings["identity"]["language"] = "en"
    await interaction.notify_brake(
        kernel, trigger, root_id=int(trigger.id), place=OWNER_DM, work="task-1",
        reason="max_wakes_per_hour", limit="max_wakes_per_hour", value=3, cap=3)
    brake_sent = kernel.tools.adapters["telegram"].actions[-1].arguments
    assert "Stopped work (task-1)" in brake_sent["text"]
    assert brake_sent["choices"] == ["Continue", "Stop"]


async def test_a_button_callback_answers_the_approval(aconn):
    kernel = await _kernel(aconn, body=Screen(shows=("buttons",)))
    trigger = await _percept(aconn)
    asked = await kernel._emit(trigger, interaction.REQUEST, {
        "capability": "telegram.run", "arguments": {"command": "ls"},
        "who": "person:owner", "place": OWNER_DM, "reason": "the call runs code"})

    rule = interaction.answer(kernel)
    callback = Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                     body="telegram",
                     payload={"approval": asked.id, "answer": True, "description": "yes"}, description="yes")
    await rule(callback, {"conn": aconn, "now": START})

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'approval.answered'")
    assert (await cur.fetchone())[0] == {"request": asked.id, "allowed": True,
                                        "who": "person:owner", "by": "person:owner"}
    queued = kernel.queue.get_nowait().event
    assert queued.kind == "intent" and queued.payload["approved"] == asked.id


async def test_the_cli_renders_typing_and_the_approval_text(aconn):
    from monoagens.adapters import cli as cli_adapter

    body = cli_adapter.CLIBody("/tmp/monoagens-test-unused.sock", clock=FakeClock(START))
    kernel = await _kernel(aconn, body=body)
    trigger = await _percept(aconn)

    await interaction.composing(kernel)({"text": "halo"}, {
        "conn": aconn, "event": trigger, "now": START,
        "descriptor": {"communicative": True}, "long_running": False})

    assert "typing" in body.shows


async def test_show_switches_write_one_line_and_apply(aconn, tmp_path):
    (tmp_path / "config.yaml").write_text(
        "version: 2\ninteraction:\n  thinking: owner_direct\n  steps: long\n", encoding="utf-8")
    kernel = await _kernel(aconn)
    kernel.deployment_root = tmp_path

    first = await interaction.show_switch(kernel, _percept_event(), ["thinking", "off"])
    second = await interaction.show_switch(kernel, _percept_event(), ["steps", "off"])

    assert first == "thinking is now off"
    assert second == "steps is now off"
    text = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert "  thinking: off" in text and "  steps: off" in text
    assert kernel.settings["interaction"]["thinking"] == "off"
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'plugin.config_changed'")
    assert (await cur.fetchone())[0]["key"] == "interaction.thinking"


def _percept_event() -> Event:
    return Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                 body="telegram", payload={"description": "halo"}, description="halo")


def test_the_boot_report_names_each_body_and_the_surfaces_it_has():
    from monoagens.middleware.loader import BootReport

    report = BootReport()

    Loader(ROOT, env={"TELEGRAM_BOT_TOKEN": "t"})._check_adapter(
        "telegram", __import__("monoagens.adapters.telegram", fromlist=["create"]).create(
            {}, {"TELEGRAM_BOT_TOKEN": "t"}), report)
    Loader(ROOT, env={"TELEGRAM_BOT_TOKEN": "t"})._check_adapter(
        "cli", __import__("monoagens.adapters.cli", fromlist=["create"]).create({}, {}), report)
    loader_lines = "\n".join(report.lines)
    assert "adapter telegram: surfaces render" in loader_lines
    assert "adapter cli: surfaces render" in loader_lines
    assert "adapter telegram: files 10 per message" in loader_lines
    assert "adapter cli: files 1 per message" in loader_lines


class _RichScreen(Screen):
    """Screen that supports editing steps, reactions, read receipts and stickers."""

    def __init__(self, name: str = "telegram",
                 shows: tuple[str, ...] = ("typing", "edit", "reactions", "read_receipts"),
                 *, gaps: dict[str, str] | None = None) -> None:
        super().__init__(name=name, shows=shows)
        self.gaps = gaps or {}
        self._msg_seq = 0

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        if action.capability == "say":
            self._msg_seq += 1
            mid = f"m{self._msg_seq}"
            choices = list((action.arguments or {}).get("choices") or ())
            if choices and "reactions" in self.shows and "buttons" not in self.shows:
                for emoji in ("👍", "👎"):
                    self.shown.append((action.place, "reactions",
                                       {"message_id": mid, "emoji": emoji}))
                return {"message_id": mid, "choices": [
                    {"label": choices[0], "answer": "👍"},
                    {"label": choices[1], "answer": "👎"},
                ]}
            if choices:
                return {"message_id": mid, "choices": [
                    {"label": opt, "answer": str(i)} for i, opt in enumerate(choices, 1)]}
            return {"message_id": mid}
        return {"text": "hasil" if action.capability == "run" else None}

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        self.shown.append((place, kind, dict(data)))
        if kind == "edit":
            return {"message_id": data.get("message_id") or "prog-1"}
        return None


async def test_typing_fires_only_for_communicative_acts_and_respects_body_override(aconn):
    screen = _RichScreen(shows=("typing", "edit"))
    kernel = await _kernel(aconn, body=screen)
    trigger = await _percept(aconn)
    rule = interaction.composing(kernel)

    # Non-communicative long_running tool -> NO typing.
    await rule({}, {"conn": aconn, "event": trigger, "now": START,
                    "descriptor": {"communicative": False, "long_running": True}})
    assert not screen.shown

    # Communicative act -> shows typing.
    await rule({"text": "halo"}, {"conn": aconn, "event": trigger, "now": START,
                                  "descriptor": {"communicative": True}})
    assert [item.name for item in screen.rendered] == ["composing"]

    # Per-adapter show override off -> suppressed.
    kernel.settings.setdefault("bodies", {})["telegram"] = {"show": {"typing": "off"}}
    screen.shown.clear()
    await rule({"text": "halo"}, {"conn": aconn, "event": trigger, "now": START,
                                  "descriptor": {"communicative": True}})
    assert not screen.shown


async def test_thinking_all_shows_group_thoughts_but_hides_private_dm_thoughts(aconn):
    screen = _RichScreen(shows=("edit",))
    kernel = await _kernel(aconn, body=screen, interaction_settings={"thinking": "all", "steps": "off"})
    dm_trigger = await _percept(aconn, place=OWNER_DM, who="person:owner", text="secret")
    await kernel._emit(dm_trigger, "decision.note", {"act": "think", "thought": "private thought"})

    grp_trigger = await _percept(aconn, place=GROUP, who="person:budi", text="hi group")
    grp_note = await kernel._emit(grp_trigger, "decision.note",
                                  {"act": "think", "thought": "group thought"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:budi",
                               place=GROUP, body="telegram", caused_by=grp_trigger.id,
                               payload={"capability": "telegram.say", "arguments": {"text": "yo"}})
    # Force another note referencing OWNER_DM inside same chain? Cross-place leak test:
    # A note whose own `place` is OWNER_DM is invisible in GROUP, while grp_note is visible.
    dm_note_in_chain = await append_event(
        aconn, ts=START, kind="decision.note", who="person:owner", place=OWNER_DM,
        body="telegram", caused_by=grp_trigger.id,
        payload={"act": "think", "thought": "leaked dm"})
    assert dm_note_in_chain.id is not None and grp_note.id is not None

    await interaction.narrate(kernel)(later, {"conn": aconn, "now": START})

    texts = [str(item.data.get("text")) for item in screen.rendered if item.name == "thought"]
    assert "(berpikir) group thought" in texts
    assert not any("leaked dm" in t or "private thought" in t for t in texts)



async def test_show_switch_writes_one_interaction_setting_and_refuses_a_bodys_option(
        aconn, tmp_path):
    """§2.4.5: `/show` sets the middleware's own decisions; a body's render option is its own."""
    (tmp_path / "config.yaml").write_text("version: 2\ninteraction:\n  thinking: owner_direct\n",
                                          encoding="utf-8")
    kernel = await _kernel(aconn, body=_RichScreen(shows=("typing", "edit")))
    kernel.deployment_root = tmp_path

    ok = await interaction.show_switch(kernel, _percept_event(), ["thinking", "all"])
    assert ok == "thinking is now all"
    assert interaction.switches(kernel)["thinking"] == "all"
    written = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert "thinking: all" in written

    refused = await interaction.show_switch(kernel, _percept_event(),
                                            ["typing", "off", "telegram"])
    assert refused.startswith("usage: /show")


async def test_reaction_approval_answered_after_restart_still_counts(aconn):
    screen = _RichScreen(shows=("typing", "reactions"))
    kernel = await _kernel(aconn, body=screen)
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram", payload={"description": "halo", "place_kind": "direct"})
    trigger = await _percept(aconn)
    asked = await kernel._emit(trigger, interaction.REQUEST, {
        "capability": "telegram.run", "arguments": {"command": "echo hi"},
        "who": "person:owner", "place": OWNER_DM, "reason": "needs reaction approval"})

    await interaction.request(kernel, trigger, int(asked.id), "telegram.run",
                              {"command": "echo hi"}, "runs work")

    assert any(k == "reactions" and d["emoji"] == "👍" for _, k, d in screen.shown)
    assert any(k == "reactions" and d["emoji"] == "👎" for _, k, d in screen.shown)
    msg_id = screen.shown[-1][2]["message_id"]

    # Simulate a full process restart: a fresh kernel with zero in-memory prompt state.
    restarted = await _kernel(aconn, body=_RichScreen(shows=("typing", "reactions")))
    reaction_event = Event(
        id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
        body="telegram",
        payload={"description": "", "data": {
            "reply_to": {"message_id": msg_id},
            "reaction": {"emoji": "👍", "message_id": msg_id},
            "answer": "👍",
        }}, description="",
    )
    await interaction.answer(restarted)(reaction_event, {"conn": aconn, "now": START})
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'approval.answered'")
    row = await cur.fetchone()
    assert row is not None and row[0]["allowed"] is True
    assert row[0]["request"] == asked.id
    queued = restarted.queue.get_nowait().event
    assert queued.kind == "intent" and queued.payload["approved"] == asked.id


async def test_progress_step_from_invisible_source_place_is_not_shown(aconn):
    """P.9: progress_show skips steps whose source/causal chain is not visible at the target place."""
    screen = _RichScreen(shows=("edit",))
    kernel = await _kernel(aconn, body=screen,
                           interaction_settings={"thinking": "off", "steps": "long", "progress": "edit"})
    dm_trigger = await _percept(aconn, place=OWNER_DM, who="person:owner", text="check secret")
    group_trigger = await _percept(aconn, place=GROUP, who="person:budi", text="halo")
    assert group_trigger.id is not None

    # A long-running act result rendered in GROUP whose causal root is OWNER_DM must be suppressed.
    leaked_step = await append_event(
        aconn, ts=START, kind="act.result", who="person:owner", place=GROUP,
        body="telegram", caused_by=dm_trigger.id,
        payload={"capability": "telegram.run", "result": {"text": "secret output"}})
    await interaction.narrate(kernel)(leaked_step, {"conn": aconn, "now": START})
    assert not screen.rendered and not screen.actions

    # Same-place step in GROUP IS visible and shown.
    group_step = await append_event(
        aconn, ts=START, kind="act.result", who="person:budi", place=GROUP,
        body="telegram", caused_by=group_trigger.id,
        payload={"capability": "telegram.run", "result": {"text": "public progress"}})
    await interaction.narrate(kernel)(group_step, {"conn": aconn, "now": START})
    assert len(screen.rendered) == 1
    assert "public progress" in str(screen.rendered[0].data.get("text"))

