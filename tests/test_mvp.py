"""MVP acceptance: tests (a)-(d) from PLAN.md Step 8, end to end.

Each test runs the real kernel, the real Postgres Log, and the real cognition layer.
Only the LLM (scripted httpx.MockTransport), HTTP, the clock (FakeClock), and the
embedder (FakeEmbedder) are fake.
"""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx

from monoagens.core import expectations
from monoagens.infra.clock import FakeClock
from monoagens.core.kernel import Kernel
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.reasoning import Reasoning
from monoagens.infra.store import append_event, read_state
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event
from tests.support import settings

SETTINGS = settings()
PLACE = "space:tg:1"
GROUP = "space:tg:-100123"
WHO = "person:owner"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
MORNING = datetime(2026, 11, 10, 23, 0, tzinfo=timezone.utc)  # 06:00 in Jakarta
SEVEN = datetime(2026, 11, 11, 0, 0, tzinfo=timezone.utc)  # 07:00 in Jakarta
SLEEP_ARGS = {
    "items": [
        {"text": "Muqsith suka kopi tubruk tanpa gula", "importance": 0.9,
         "who": "person:owner", "place": PLACE},
        {"text": "Anjing Muqsith bernama Bruno", "importance": 0.3,
         "who": "person:owner", "place": PLACE},
    ],
    "core_memory": "Muqsith tinggal di Bandung dan suka kopi tubruk.",
}


class FakeLLM:
    """Scripted OpenAI-compatible endpoint; records every request it receives."""

    def __init__(self, responses: list[list[tuple[str, dict]]] | None = None) -> None:
        self.responses = responses or [[]]
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        tool_calls = [{"id": f"call_{i}", "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments)}}
                      for i, (name, arguments) in enumerate(calls)]
        return httpx.Response(200, json={"choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}]})


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning("https://llm.test/v1", "test-key", "fake-model", settings=SETTINGS,
               transport=httpx.MockTransport(fake.handler))


class RecordingBody:
    name = "telegram"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _msg(text: str, *, message_id: int = 1, place: str = PLACE, place_kind: str = "direct",
         addressed: bool = True, who: str = WHO) -> Event:
    return Event(id=None, ts=START, kind="percept", who=who, place=place, body="telegram",
                 payload={"description": text, "addressed": addressed,
                          "place_kind": place_kind, "data": {"message_id": message_id}})


async def _feed(kernel: Kernel, event: Event) -> Event:
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _tick(kernel: Kernel, clock: FakeClock) -> None:
    await clock.tick(kernel.put)
    await kernel.step()
    await kernel.run_until_idle()


async def _kinds(conn) -> list[str]:
    return [row[0] for row in await (await conn.execute(
        "SELECT kind FROM events WHERE kind NOT IN ('job.started', 'job.result') "
        "ORDER BY id")).fetchall()]


def _volatile(fake: FakeLLM) -> str:
    return fake.requests[-1]["messages"][2]["content"]


def _recent(fake: FakeLLM) -> str:
    return _volatile(fake).split("<recent>", 1)[1].split("</recent>", 1)[0]


async def test_mvp_a_restart_preserves_conversation_context(aconn):
    """MVP (a): a restart keeps the topic; the next reply continues it."""
    clock = FakeClock(START)

    first_fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "kita bahas kopi ya"}})]])
    first = Kernel(aconn, clock=clock, pfc=_pfc(first_fake), settings=SETTINGS)
    first.register_adapter(RecordingBody())
    await _feed(first, _msg("besok kita bahas apa?"))

    # Restart: a brand-new Kernel and Reasoning on the same database.
    second_fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "lanjut soal kopi"}})]])
    second = Kernel(aconn, clock=clock, pfc=_pfc(second_fake), settings=SETTINGS)
    body = RecordingBody()
    second.register_adapter(body)
    await _feed(second, _msg("lanjut", message_id=2))

    assert len(second_fake.requests) == 1
    volatile = _volatile(second_fake)
    recent = _recent(second_fake)
    assert "besok kita bahas apa?" in recent
    assert "Mona: kita bahas kopi ya" in recent
    assert "<event>person:owner: lanjut</event>" in volatile
    assert body.actions == [Action(kind="say", place=PLACE, payload={"text": "lanjut soal kopi"},
                                   adapter="telegram")]
    assert (await _kinds(aconn)).count("act.out") == 2


async def test_mvp_b_silence_and_mention(aconn):
    """MVP (b): 20 ordinary group messages stay silent; a mention gets a reply."""
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "iya, ada apa?"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    for i in range(20):
        await _feed(kernel, _msg(f"obrolan {i}", message_id=i + 1, who=f"person:tg:{i}",
                                 place=GROUP, place_kind="shared", addressed=False))

    assert fake.requests == []
    kinds = await _kinds(aconn)
    assert kinds.count("percept") == 20
    assert kinds.count("act.out") == 0
    assert kinds.count("decision.silent") == 20

    payloads = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'decision.silent' ORDER BY id")).fetchall()]
    for payload in payloads:
        assert set(payload) == {"decision", "reason", "reflex", "signals", "s", "s_eff",
                                "theta", "tau", "p", "u", "habituation", "talk_rate",
                                "budget_used", "promise_debt", "familiarity"}
        assert payload["decision"] == "silent"
        assert payload["reason"] == "gate score below silent threshold"
        assert payload["reflex"] is None
        assert payload["s_eff"] < payload["theta"] and payload["p"] < 1e-3
        assert payload["talk_rate"] == 0.0 and payload["budget_used"] == 0.0
        assert 0.0 <= payload["u"] < 1.0
        assert set(payload["signals"]) == {
            "own_name", "info_gap", "feeling_of_knowing", "relevance", "social_cost",
        }

    await _feed(kernel, _msg("mona, halo?", message_id=99, place=GROUP, place_kind="shared",
                             addressed=True))

    assert len(fake.requests) == 1
    assert "<why_awake>addressed message</why_awake>" in _volatile(fake)
    assert (await _kinds(aconn)).count("act.out") == 1
    assert body.actions == [Action(kind="say", place=GROUP, payload={"text": "iya, ada apa?"},
                                   adapter="telegram")]


async def test_mvp_c_expectation_remind_at_seven_and_restart(aconn):
    """MVP (c): exactly one reminder at 7 across a restart; a met event_seen never fires."""
    clock = FakeClock(MORNING)  # 06:00 Jakarta
    fake = FakeLLM([
        [("expect", {"about": "remind the owner at 7", "cue_time": {"daily": "07:00"},
                     "expect": {"none": True}, "intent": "remind gently", "weight": 1.0,
                     "policy": {"once": True}})],
        [],
        [("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})],
    ])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("ingatkan saya jam 7"))
    assert await _kinds(aconn) == ["percept", "model.call", "model.call",
                                   "expectation.created", "decision.silent"]

    clock.set_time(SEVEN - timedelta(minutes=10))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("act.out") == 0

    clock.set_time(SEVEN)
    await _tick(kernel, clock)
    kinds = await _kinds(aconn)
    assert kinds.count("expectation.fired") == 1
    assert kinds.count("act.out") == 1
    assert body.actions == [Action(kind="say", place=PLACE, payload={"text": "sudah jam 7"},
                                   adapter="telegram")]
    assert "expectation missed: remind the owner at 7" in _volatile(fake).split("<why_awake>")[1]
    assert "<event>[expectation.fired]</event>" in _volatile(fake)

    cur = await aconn.execute("SELECT status, fire_count FROM expectations")
    assert await cur.fetchone() == ("satisfied", 1)

    # Restart around 7: the reminder already answered, so nothing fires again.
    restarted_fake = FakeLLM()
    restarted = Kernel(aconn, clock=clock, pfc=_pfc(restarted_fake), settings=SETTINGS)
    restarted.register_adapter(RecordingBody())
    await restarted.recover()
    await restarted.run_until_idle()
    clock.advance(600)
    await _tick(restarted, clock)
    assert restarted_fake.requests == []
    assert (await _kinds(aconn)).count("act.out") == 1

    # An event_seen expectation that is met is resolved, never fired.
    pattern = {"kind": "percept", "who": WHO, "text_like": "workout"}
    source = await append_event(aconn, ts=clock.now(), kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "source"})
    created = await expectations.create(
        aconn, {"about": "nudge me if I skip workouts", "cue_event": pattern,
                "expect": {"event_seen": pattern, "within": "2d"},
                "weight": 1.0, "policy": {"once": True}},
        now=clock.now(), settings=SETTINGS, source=source)
    exp_id = int(created.payload["expectation_id"])

    await _feed(restarted, _msg("saya sudah WORKOUT hari ini", message_id=2, place=GROUP,
                                place_kind="shared", addressed=False))

    fired = await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'expectation.fired' "
        "AND payload->>'expectation_id' = %s", (str(exp_id),))).fetchone()
    assert fired[0] == 0
    cur = await aconn.execute("SELECT status FROM expectations WHERE id = %s", (exp_id,))
    assert (await cur.fetchone())[0] == "satisfied"
    resolved = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'expectation.resolved' "
        "AND payload->>'expectation_id' = %s", (str(exp_id),))).fetchone()
    assert resolved[0]["status"] == "satisfied"
    assert restarted_fake.requests == []
    assert (await _kinds(aconn)).count("act.out") == 1


async def test_mvp_d_sleep_consolidation_and_retrieval(aconn):
    """MVP (d): sleep writes memories and core_memory, never edits events, and a
    differently worded question retrieves the right memory."""
    clock = FakeClock(START)
    await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                       payload={"text": "Muqsith suka kopi tubruk tanpa gula"})
    last = await append_event(aconn, ts=START + timedelta(minutes=1), kind="act.out", who=WHO,
                              place=PLACE, body="telegram", payload={"text": "baik, dicatat"})
    max_before = last.id
    before = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events WHERE id <= %s ORDER BY id",
        (max_before,))).fetchall()

    fake = FakeLLM([[("store_memories", SLEEP_ARGS)], []])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS, embedder=FakeEmbedder())
    kernel.register_adapter(RecordingBody())

    # Idle for 45 minutes: the tick path runs sleep.
    clock.set_time(START + timedelta(minutes=45))
    await _tick(kernel, clock)

    memories = await (await aconn.execute("SELECT text, embedding, derived_from "
                                          "FROM memories ORDER BY id")).fetchall()
    assert [row[0] for row in memories] == [item["text"] for item in SLEEP_ARGS["items"]]
    assert all(len(row[1].to_list()) == 384 for row in memories)
    assert all(max_before in row[2] for row in memories)
    assert (await read_state(aconn, "core_memory"))["text"] == SLEEP_ARGS["core_memory"]
    assert (await read_state(aconn, "sleep:last_event"))["id"] == max_before

    # The Log is immutable: old rows are byte-for-byte untouched.
    untouched = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events WHERE id <= %s ORDER BY id",
        (max_before,))).fetchall()
    assert untouched == before
    assert (await (await aconn.execute("SELECT count(*) FROM events WHERE id <= %s",
                                        (max_before,))).fetchone())[0] == len(before)

    # A new question, worded differently, retrieves the coffee memory first.
    clock.set_time(START + timedelta(hours=2))
    await _feed(kernel, _msg("Apa minuman favorit saya?", message_id=2))

    assert len(fake.requests) == 2  # sleep, then the reply turn
    section = _volatile(fake).split("<memories>", 1)[1].split("</memories>", 1)[0]
    assert "kopi tubruk" in section
    assert section.index("kopi tubruk") < section.index("Bruno")

    accessed = await (await aconn.execute(
        "SELECT last_accessed_at FROM memories WHERE text LIKE %s",
        ("%kopi tubruk%",))).fetchone()
    assert accessed[0] == clock.now()
