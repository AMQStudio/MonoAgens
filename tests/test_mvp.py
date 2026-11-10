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

from monoagens import expect
from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.memory import FakeEmbedder
from monoagens.pfc import PFC
from monoagens.store import append_event, read_state
from monoagens.types import Action, Event

SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "persona": "Calm and honest.",
    "principles": ["Never lie.", "Silence is better than interrupting."],
    "cognition": {
        "seed": 1,
        "weights": {"name_mentioned": 0.5, "info_gap": 0.35, "feeling_of_knowing": 0.35,
                    "relevance": 0.2, "social_cost": -0.4},
    },
    "sleep": {"idle_minutes": 30, "hour": 2},
    "memory": {"retrieve_k": 5},
    "expectations": {"default_expires_days": 30, "max_active": 200},
}
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
        self.responses = responses or [[("stay_silent", {"reason": "testing"})]]
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        tool_calls = [{"id": f"call_{i}", "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments)}}
                      for i, (name, arguments) in enumerate(calls)]
        return httpx.Response(200, json={"choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}]})


def _pfc(fake: FakeLLM) -> PFC:
    return PFC("https://llm.test/v1", "test-key", "fake-model", soul=SOUL,
               transport=httpx.MockTransport(fake.handler))


class RecordingBody:
    name = "telegram"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _msg(text: str, *, message_id: int = 1, place: str = PLACE, place_kind: str = "direct",
         addressed: bool = True, who: str = WHO) -> Event:
    return Event(id=None, ts=START, kind="msg.in", who=who, place=place, body="telegram",
                 payload={"text": text, "message_id": message_id, "addressed": addressed,
                          "place_kind": place_kind})


async def _feed(kernel: Kernel, event: Event) -> Event:
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _tick(kernel: Kernel, clock: FakeClock) -> None:
    await clock.tick(kernel.put)
    await kernel.step()


async def _kinds(conn) -> list[str]:
    return [row[0] for row in await (await conn.execute("SELECT kind FROM events ORDER BY id")).fetchall()]


def _volatile(fake: FakeLLM) -> str:
    return fake.requests[-1]["messages"][2]["content"]


def _recent(fake: FakeLLM) -> str:
    return _volatile(fake).split("<recent>", 1)[1].split("</recent>", 1)[0]


async def test_mvp_a_restart_preserves_conversation_context(aconn):
    """MVP (a): a restart keeps the topic; the next reply continues it."""
    clock = FakeClock(START)

    first_fake = FakeLLM([[("say", {"text": "kita bahas kopi ya"})]])
    first = Kernel(aconn, clock=clock, pfc=_pfc(first_fake), soul=SOUL)
    first.register_body(RecordingBody())
    await _feed(first, _msg("besok kita bahas apa?"))

    # Restart: a brand-new Kernel and PFC on the same database.
    second_fake = FakeLLM([[("say", {"text": "lanjut soal kopi"})]])
    second = Kernel(aconn, clock=clock, pfc=_pfc(second_fake), soul=SOUL)
    body = RecordingBody()
    second.register_body(body)
    await _feed(second, _msg("lanjut", message_id=2))

    assert len(second_fake.requests) == 1
    volatile = _volatile(second_fake)
    recent = _recent(second_fake)
    assert "besok kita bahas apa?" in recent
    assert "Mona: kita bahas kopi ya" in recent
    assert "<event>owner: lanjut</event>" in volatile
    assert body.actions == [Action(kind="say", place=PLACE, payload={"text": "lanjut soal kopi"},
                                   body="telegram")]
    assert (await _kinds(aconn)).count("msg.out") == 2


async def test_mvp_b_silence_and_mention(aconn):
    """MVP (b): 20 ordinary group messages stay silent; a mention gets a reply."""
    fake = FakeLLM([[("say", {"text": "iya, ada apa?"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    body = RecordingBody()
    kernel.register_body(body)

    for i in range(20):
        await _feed(kernel, _msg(f"obrolan {i}", message_id=i + 1, who=f"person:tg:{i}",
                                 place=GROUP, place_kind="group", addressed=False))

    assert fake.requests == []
    kinds = await _kinds(aconn)
    assert kinds.count("msg.in") == 20
    assert kinds.count("msg.out") == 0
    assert kinds.count("decision.silent") == 20

    payloads = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'decision.silent' ORDER BY id")).fetchall()]
    for payload in payloads:
        assert set(payload) == {"decision", "reason", "reflex", "signals", "s", "theta", "p", "u"}
        assert payload["decision"] == "silent"
        assert payload["reason"] == "gate score below silent threshold"
        assert payload["reflex"] is None
        assert payload["s"] == 0.0 and payload["theta"] == 0.0 and payload["p"] == 0.0
        assert 0.0 <= payload["u"] < 1.0
        assert set(payload["signals"]) == {
            "name_mentioned", "info_gap", "feeling_of_knowing", "relevance", "social_cost",
        }

    await _feed(kernel, _msg("mona, halo?", message_id=99, place=GROUP, place_kind="group",
                             addressed=True))

    assert len(fake.requests) == 1
    assert "<why_awake>addressed message</why_awake>" in _volatile(fake)
    assert (await _kinds(aconn)).count("msg.out") == 1
    assert body.actions == [Action(kind="say", place=GROUP, payload={"text": "iya, ada apa?"},
                                   body="telegram")]


async def test_mvp_c_expectation_remind_at_seven_and_restart(aconn):
    """MVP (c): exactly one reminder at 7 across a restart; a met event_seen never fires."""
    clock = FakeClock(MORNING)  # 06:00 Jakarta
    fake = FakeLLM([
        [("expect", {"about": "remind the owner at 7", "cue_time": {"daily": "07:00"},
                     "expect": {"none": True}, "intent": "remind gently", "weight": 1.0,
                     "policy": {"once": True}})],
        [("say", {"text": "sudah jam 7"})],
    ])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), soul=SOUL)
    body = RecordingBody()
    kernel.register_body(body)

    await _feed(kernel, _msg("ingatkan saya jam 7"))
    assert await _kinds(aconn) == ["msg.in", "expectation.created"]

    clock.set_time(SEVEN - timedelta(minutes=10))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("msg.out") == 0

    clock.set_time(SEVEN)
    await _tick(kernel, clock)
    kinds = await _kinds(aconn)
    assert kinds.count("expectation.fired") == 1
    assert kinds.count("msg.out") == 1
    assert body.actions == [Action(kind="say", place=PLACE, payload={"text": "sudah jam 7"},
                                   body="telegram")]
    assert "expectation missed: remind the owner at 7" in _volatile(fake).split("<why_awake>")[1]

    cur = await aconn.execute("SELECT status, fire_count FROM expectations")
    assert await cur.fetchone() == ("satisfied", 1)

    # Restart around 7: the reminder already answered, so nothing fires again.
    restarted_fake = FakeLLM()
    restarted = Kernel(aconn, clock=clock, pfc=_pfc(restarted_fake), soul=SOUL)
    restarted.register_body(RecordingBody())
    await restarted.recover()
    clock.advance(600)
    await _tick(restarted, clock)
    assert restarted_fake.requests == []
    assert (await _kinds(aconn)).count("msg.out") == 1

    # An event_seen expectation that is met is resolved, never fired.
    pattern = {"kind": "msg.in", "who": WHO, "text_like": "workout"}
    source = await append_event(aconn, ts=clock.now(), kind="msg.in", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "source"})
    created = await expect.create(
        aconn, {"about": "nudge me if I skip workouts", "cue_event": pattern,
                "expect": {"event_seen": pattern, "within": "2d"},
                "weight": 1.0, "policy": {"once": True}},
        now=clock.now(), soul=SOUL, source=source)
    exp_id = int(created.payload["expectation_id"])

    await _feed(restarted, _msg("saya sudah WORKOUT hari ini", message_id=2, place=GROUP,
                                place_kind="group", addressed=False))

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
    assert (await _kinds(aconn)).count("msg.out") == 1


async def test_mvp_d_sleep_consolidation_and_retrieval(aconn):
    """MVP (d): sleep writes memories and core_memory, never edits events, and a
    differently worded question retrieves the right memory."""
    clock = FakeClock(START)
    await append_event(aconn, ts=START, kind="msg.in", who=WHO, place=PLACE, body="telegram",
                       payload={"text": "Muqsith suka kopi tubruk tanpa gula"})
    last = await append_event(aconn, ts=START + timedelta(minutes=1), kind="msg.out", who=WHO,
                              place=PLACE, body="telegram", payload={"text": "baik, dicatat"})
    max_before = last.id
    before = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events WHERE id <= %s ORDER BY id",
        (max_before,))).fetchall()

    fake = FakeLLM([[("store_memories", SLEEP_ARGS)], [("stay_silent", {"reason": "noted"})]])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), soul=SOUL, embedder=FakeEmbedder())
    kernel.register_body(RecordingBody())

    # Idle for 45 minutes: the tick path runs sleep.
    clock.set_time(START + timedelta(minutes=45))
    await _tick(kernel, clock)

    memories = await (await aconn.execute("SELECT text, embedding, source_events "
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
