"""Tests for Step 6: expectations, the time and event paths, and MVP test (c)."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone
from dataclasses import replace

import httpx
import pytest

from monoagens.core import expectations
from monoagens.infra.clock import FakeClock
from monoagens.infra.duration import parse_duration
from monoagens.core.kernel import Kernel
from monoagens.core.reasoning import Reasoning
from monoagens.infra.providers import OpenAICompatibleProvider
from monoagens.infra.store import append_event, read_expectations, write_state
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event
from monoagens.boot import build
from tests.support import settings

SETTINGS = settings()
PLACE = "space:tg:1"
WHO = "person:owner"
START = datetime(2026, 11, 10, 23, 0, tzinfo=timezone.utc)  # 06:00 in Jakarta
SEVEN = datetime(2026, 11, 11, 0, 0, tzinfo=timezone.utc)  # 07:00 in Jakarta


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

    @property
    def last_prompt(self) -> str:
        return self.requests[-1]["messages"][2]["content"]


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning(
        OpenAICompatibleProvider("https://llm.test/v1", "test-key", "fake-model",
                                 transport=httpx.MockTransport(fake.handler)),
        settings=SETTINGS)


class RecordingBody:
    name = "telegram"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _msg(text: str = "halo", *, message_id: int = 1, addressed: bool = True) -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                 payload={"description": text, "addressed": addressed, "place_kind": "direct",
                          "data": {"message_id": message_id}}, description=text)


def _source() -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                 payload={"text": "source"}, description="source")


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


async def _status(conn, exp_id: int) -> str:
    cur = await conn.execute("SELECT status, fire_count FROM expectations WHERE id = %s", (exp_id,))
    return (await cur.fetchone())[0]


async def _create(conn, **overrides) -> tuple[int, Event]:
    """Create one expectation through the module API; returns (id, created event)."""
    data = {"about": "test expectation", "cue_time": {"at": START.isoformat()},
            "expect": {"none": True}, "weight": 1.0, "policy": {"once": True}}
    data.update(overrides)
    source = await append_event(conn, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                                payload={"text": "source"})
    created = await expectations.create(conn, data, now=START, settings=SETTINGS, source=source)
    return int(created.payload["expectation_id"]), created


# --- durations and cue times -------------------------------------------------


def test_parse_duration_units():
    assert parse_duration("30s") == timedelta(seconds=30)
    assert parse_duration("30m") == timedelta(minutes=30)
    assert parse_duration("2h") == timedelta(hours=2)
    assert parse_duration("1d") == timedelta(days=1)
    assert parse_duration("1.5h") == timedelta(minutes=90)
    for bad in ("2w", "abc", "", "10"):
        with pytest.raises(ValueError):
            parse_duration(bad)


def test_next_check_at_every_and_daily_in_the_configured_timezone():
    jakarta = expectations.tz(SETTINGS)
    assert expectations.next_check({"at": "2026-11-11T07:00:00+07:00"}, START, jakarta) == SEVEN
    assert expectations.next_check({"every": "2d"}, START, jakarta) == START + timedelta(days=2)
    # 06:00 Jakarta -> 07:00 the same day; 08:00 Jakarta -> 07:00 the next day
    assert expectations.next_check({"daily": "07:00"}, START, jakarta) == SEVEN
    late = datetime(2026, 11, 11, 1, 0, tzinfo=timezone.utc)  # 08:00 Jakarta
    assert expectations.next_check({"daily": "07:00"}, late, jakarta) == SEVEN + timedelta(days=1)
    assert expectations.next_check(None, START, jakarta) is None
    with pytest.raises(ValueError):
        expectations.next_check({"whenever": True}, START, jakarta)


def test_validation_defaults_and_rejections():
    row = expectations.validate({"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                          now=START, settings=SETTINGS)
    assert row["weight"] == 0.5
    assert row["policy"] == {"once": True}
    assert row["expires_at"] == START + timedelta(days=30)
    assert row["next_check_at"] == START + timedelta(days=1)

    explicit = expectations.validate({"about": "x", "cue_time": {"at": START.isoformat()},
                                "expect": {"none": True}, "weight": 0.2,
                                "expires_at": "2026-12-01T00:00:00+00:00"}, now=START, settings=SETTINGS)
    assert explicit["expires_at"] == datetime(2026, 12, 1, tzinfo=timezone.utc)

    bad = [
        {"about": "", "cue_time": {"every": "1d"}, "expect": {"none": True}},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"nothing": True}},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}, "weight": 2.0},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}, "weight": -0.1},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"state": "k", "op": "like", "value": 1}},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"event_seen": {"kind": "percept"}}},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}, "policy": {"once": False}},
        {"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}, "policy": {"cooldown": "soon"}},
        {"about": "x", "expect": {"none": True}},
    ]
    for data in bad:
        with pytest.raises(ValueError):
            expectations.validate(data, now=START, settings=SETTINGS)


async def test_max_active_is_enforced(aconn):
    runtime = settings(constants={"max_active": 1})
    await expectations.create(aconn, {"about": "first", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                        now=START, settings=runtime, source=_source())
    with pytest.raises(ValueError, match="too many active"):
        await expectations.create(aconn, {"about": "second", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                            now=START, settings=runtime, source=_source())


# --- the three operators -----------------------------------------------------


async def test_operator_none_is_never_met(aconn):
    assert await expectations.is_met(aconn, {"none": True}, START) is False


async def test_operator_event_seen_checks_the_window(aconn):
    old = await append_event(aconn, ts=START - timedelta(days=3), kind="percept", who=WHO, place=PLACE,
                             body="telegram", payload={"text": "I did my WORKOUT"})
    assert old.id is not None
    pattern = {"kind": "percept", "who": WHO}
    assert await expectations.is_met(aconn, {"event_seen": pattern, "within": "4d"}, START) is True
    assert await expectations.is_met(aconn, {"event_seen": pattern, "within": "2d"}, START) is False
    other = {"event_seen": {"kind": "percept", "who": "person:other"}, "within": "4d"}
    assert await expectations.is_met(aconn, other, START) is False


async def test_operator_state_compares_eq_gt_lt(aconn):
    seed = await append_event(aconn, ts=START, kind="decision.note", place=PLACE)
    await write_state(aconn, "mood", {"value": 5}, seed.id)

    assert await expectations.is_met(aconn, {"state": "mood", "op": "eq", "value": 5}, START) is True
    assert await expectations.is_met(aconn, {"state": "mood", "op": "gt", "value": 3}, START) is True
    assert await expectations.is_met(aconn, {"state": "mood", "op": "lt", "value": 3}, START) is False
    assert await expectations.is_met(aconn, {"state": "missing", "op": "eq", "value": 1}, START) is False


def test_matches_cue_is_exact_on_ids_and_substring_on_text():
    event = _msg("waktu Workout sore")
    assert expectations.matches_cue({"kind": "percept", "who": WHO, "place": PLACE}, event) is True
    assert expectations.matches_cue({"text_like": "WORKOUT"}, event) is True
    assert expectations.matches_cue({"who": "person:other"}, event) is False
    assert expectations.matches_cue({"kind": "act.out"}, event) is False
    assert expectations.matches_cue(None, event) is False


# --- the time path and the event path ----------------------------------------


async def test_time_path_fires_only_when_next_check_is_due(aconn):
    exp_id, _ = await _create(aconn, weight=1.0)
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "fired"
    assert (await _kinds(aconn)).count("expectation.fired") == 1

    cur = await aconn.execute("SELECT fire_count, last_fired_at FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == (1, START)


async def test_time_path_ignores_future_expectations(aconn):
    exp_id, _ = await _create(aconn, cue_time={"at": (START + timedelta(hours=1)).isoformat()})
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "active"
    assert "expectation.fired" not in await _kinds(aconn)


async def test_event_path_fires_on_a_matching_event(aconn):
    exp_id, _ = await _create(aconn, cue_time=None,
                              cue_event={"kind": "percept", "place": "space:tg:2"})
    kernel = build(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

    await _feed(kernel, _msg("halo", message_id=1))
    assert await _status(aconn, exp_id) == "active"

    await _feed(kernel, replace(_msg("saya sudah WORKOUT", message_id=2), place="space:tg:2"))
    assert await _status(aconn, exp_id) == "fired"


async def test_event_seen_expectation_that_is_met_never_fires(aconn):
    pattern = {"kind": "percept", "who": WHO}
    exp_id, _ = await _create(aconn, cue_time=None, cue_event=pattern,
                              expect={"event_seen": pattern, "within": "2d"})
    kernel = build(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

    await _feed(kernel, _msg("saya sudah workout hari ini"))

    kinds = await _kinds(aconn)
    assert "expectation.fired" not in kinds
    assert await _status(aconn, exp_id) == "satisfied"
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'expectation.resolved'")
    assert (await cur.fetchone())[0]["status"] == "satisfied"


async def test_time_path_met_event_seen_resolves_without_firing(aconn):
    pattern = {"kind": "percept", "who": WHO}
    await append_event(aconn, ts=START - timedelta(hours=1), kind="percept", who=WHO, place=PLACE,
                       body="telegram", payload={"text": "workout done"})
    exp_id, _ = await _create(aconn, expect={"event_seen": pattern, "within": "2d"})
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.fired" not in await _kinds(aconn)


# --- policies ----------------------------------------------------------------


async def test_low_weight_fired_expectation_still_wakes_the_pfc_once(aconn):
    """A missed expectation below the silent band fires the Reasoning anyway (DESIGN appraisal)."""
    exp_id, _ = await _create(aconn, weight=0.3)
    clock = FakeClock(START)
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})]])
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _tick(kernel, clock)

    kinds = await _kinds(aconn)
    assert kinds.count("expectation.fired") == 1
    assert kinds.count("act.out") == 1
    assert len(fake.requests) == 1
    assert await _status(aconn, exp_id) == "satisfied"


async def test_an_expectation_wake_renders_the_place_it_happened_in(aconn):
    """A promise in the owner's chat renders that place, and never a payload default."""
    exp_id, _ = await _create(aconn, weight=0.3)
    clock = FakeClock(START)
    fake = FakeLLM()
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))
    await _tick(kernel, clock)

    assert await _status(aconn, exp_id) == "satisfied"
    assert f"<place>{PLACE}</place>" in fake.last_prompt


async def test_policy_once_is_satisfied_after_the_turn(aconn):
    exp_id, _ = await _create(aconn)
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(kernel, clock)

    await expectations.settle(aconn, exp_id, now=START, settings=SETTINGS)
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.resolved" in await _kinds(aconn)


async def test_recurring_policy_reschedules_by_cooldown_then_drops_at_max_fires(aconn):
    exp_id, _ = await _create(aconn, policy={"cooldown": "6h", "max_fires": 2})
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    await expectations.settle(aconn, exp_id, now=START, settings=SETTINGS)
    cur = await aconn.execute("SELECT status, next_check_at FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == ("active", START + timedelta(hours=6))

    clock.set_time(START + timedelta(hours=5))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("expectation.fired") == 1

    clock.set_time(START + timedelta(hours=6))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("expectation.fired") == 2
    await expectations.settle(aconn, exp_id, now=clock.now(), settings=SETTINGS)
    cur = await aconn.execute("SELECT status, fire_count FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == ("dropped", 2)


# --- the Reasoning tools -----------------------------------------------------------


async def test_expect_tool_creates_an_expectation_and_records_it(aconn):
    fake = FakeLLM([[("expect", {"about": "remind me at 7", "cue_time": {"daily": "07:00"},
                                "expect": {"none": True}, "intent": "remind gently",
                                "weight": 0.9, "policy": {"once": True}})], []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("ingatkan saya jam 7"))

    assert await _kinds(aconn) == ["percept", "model.call", "model.call",
                                   "expectation.created", "decision.silent"]
    cur = await aconn.execute("SELECT who, place, about, weight, status, next_check_at, expires_at "
                              "FROM expectations")
    who, place, about, weight, status, next_check, expires = await cur.fetchone()
    assert (who, place, about, weight, status) == (WHO, PLACE, "remind me at 7", pytest.approx(0.9), "active")
    assert next_check == SEVEN
    assert expires == START + timedelta(days=30)


async def test_expect_with_an_invalid_weight_is_refused_in_the_same_turn(aconn):
    """D51: the refusal's reason is a result line, so the model can correct it at once."""
    fake = FakeLLM([[("expect", {"about": "bad", "cue_time": {"daily": "07:00"},
                                "expect": {"none": True}, "weight": 2.0})], []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("buat janji"))

    assert "<results>\nexpect refused: weight out of range: 2.0" in \
        fake.requests[1]["messages"][-1]["content"]
    assert await _kinds(aconn) == ["percept", "model.call", "model.call", "decision.silent"]
    cur = await aconn.execute("SELECT count(*) FROM expectations")
    assert (await cur.fetchone())[0] == 0


async def test_resolve_tool_closes_an_expectation(aconn):
    exp_id, _ = await _create(aconn)
    fake = FakeLLM([[("resolve", {"id": exp_id, "status": "dropped"})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("lupakan saja"))

    assert await _status(aconn, exp_id) == "dropped"
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'expectation.resolved'")
    assert (await cur.fetchone())[0] == {"expectation_id": exp_id, "about": "test expectation",
                                         "status": "dropped"}


# --- MVP test (c) ------------------------------------------------------------


async def test_mvp_c_remind_me_at_seven_exactly_once_across_a_restart(aconn):
    """PLAN.md Step 6: "remind me at 7" -> exactly one act.out at 7, even after a restart."""
    clock = FakeClock(START)  # 06:00 Jakarta
    fake = FakeLLM([
        [("expect", {"about": "remind the owner at 7", "cue_time": {"daily": "07:00"},
                     "expect": {"none": True}, "intent": "remind gently", "weight": 1.0,
                     "policy": {"once": True}})],
        [],
        [("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})],
    ])
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
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
    assert [(a.kind, a.place, dict(a.arguments)) for a in body.actions] == [
        ("say", PLACE, {"text": "sudah jam 7"})]
    assert "expectation missed: remind the owner at 7" in fake.last_prompt.split("<why_awake>")[1]

    cur = await aconn.execute("SELECT status, fire_count FROM expectations")
    assert await cur.fetchone() == ("satisfied", 1)

    # The act.out is linked to the fired event, which is what recovery relies on.
    cur = await aconn.execute("SELECT f.id, m.caused_by FROM events f JOIN events m ON m.caused_by = f.id "
                              "WHERE f.kind = 'expectation.fired' AND m.kind = 'act.out'")
    assert (await cur.fetchone())[0] is not None

    # Restart right after the reminder: no second message, no second LLM call.
    restarted_fake = FakeLLM()
    restarted = build(aconn, clock=clock, pfc=_pfc(restarted_fake), settings=SETTINGS)
    restarted.register_adapter(RecordingBody())
    await restarted.recover()
    await restarted.run_until_idle()
    clock.advance(600)
    await _tick(restarted, clock)
    assert restarted_fake.requests == []
    assert (await _kinds(aconn)).count("act.out") == 1


# --- startup recovery --------------------------------------------------------


async def test_recovery_wakes_once_for_a_fired_expectation_without_a_reply(aconn):
    exp_id, _ = await _create(aconn)
    clock = FakeClock(START)
    # No Reasoning in the first process: the reminder fired but was never answered.
    crashed = build(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(crashed, clock)
    assert await _status(aconn, exp_id) == "fired"

    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})]])
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())
    await kernel.recover()
    await kernel.run_until_idle()

    assert len(fake.requests) == 1
    assert (await _kinds(aconn)).count("act.out") == 1
    assert await _status(aconn, exp_id) == "satisfied"

    # A second restart finds nothing left to do.
    again_fake = FakeLLM()
    again = build(aconn, clock=clock, pfc=_pfc(again_fake), settings=SETTINGS)
    await again.recover()
    await again.run_until_idle()
    assert again_fake.requests == []


async def test_recovery_only_resolves_when_the_reply_already_exists(aconn):
    exp_id, _ = await _create(aconn)
    clock = FakeClock(START)
    crashed = build(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(crashed, clock)

    cur = await aconn.execute("SELECT id FROM events WHERE kind = 'expectation.fired'")
    fired_id = (await cur.fetchone())[0]
    await append_event(aconn, ts=START, kind="act.out", who=WHO, place=PLACE, body="telegram",
                       payload={"text": "sudah jam 7"}, caused_by=fired_id)

    fake = FakeLLM()
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    await kernel.recover()
    await kernel.run_until_idle()

    assert fake.requests == []
    assert await _status(aconn, exp_id) == "satisfied"
    assert (await _kinds(aconn)).count("act.out") == 1


async def test_recovery_does_not_touch_active_expectations(aconn):
    exp_id, _ = await _create(aconn, cue_time={"at": (START + timedelta(days=1)).isoformat()})
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    await kernel.recover()
    await kernel.run_until_idle()

    assert fake.requests == []
    assert await _status(aconn, exp_id) == "active"


# --- kinds ------------------------------------------------------------------

KINDS = ("promise", "wait", "predict", "guard")


@pytest.mark.parametrize("kind", KINDS)
def test_validation_accepts_every_kind(kind):
    row = expectations.validate({"about": "x", "kind": kind, "cue_time": {"every": "1d"},
                           "expect": {"none": True}}, now=START, settings=SETTINGS)
    assert row["kind"] == kind
    assert row["condition"] == {"none": True}


def test_validation_defaults_to_promise_and_rejects_unknown_kinds():
    row = expectations.validate({"about": "x", "cue_time": {"every": "1d"},
                           "expect": {"none": True}}, now=START, settings=SETTINGS)
    assert row["kind"] == "promise"
    with pytest.raises(ValueError, match="unknown expectation kind"):
        expectations.validate({"about": "x", "kind": "wish", "cue_time": {"every": "1d"},
                         "expect": {"none": True}}, now=START, settings=SETTINGS)


async def test_create_stores_kind_origin_actor_and_lineage(aconn):
    exp_id, created = await _create(aconn, kind="predict")
    cur = await aconn.execute(
        "SELECT kind, origin_actor, derived_from, condition FROM expectations WHERE id = %s",
        (exp_id,))
    assert await cur.fetchone() == ("predict", WHO, [created.caused_by], {"none": True})


async def test_guard_never_fires_or_wakes_the_pfc_on_a_tick(aconn):
    exp_id, _ = await _create(aconn, kind="guard", weight=1.0)
    fake = FakeLLM()
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)

    await _tick(kernel, clock)

    assert await _status(aconn, exp_id) == "active"
    assert "expectation.fired" not in await _kinds(aconn)
    assert fake.requests == []


async def test_guard_never_fires_when_its_cue_event_arrives(aconn):
    exp_id, _ = await _create(aconn, kind="guard", cue_time=None,
                              cue_event={"kind": "percept", "who": WHO})
    kernel = build(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

    await _feed(kernel, _msg("saya sudah WORKOUT"))

    assert await _status(aconn, exp_id) == "active"
    assert "expectation.fired" not in await _kinds(aconn)


# --- turn-taking and patience (Task 2 Part 6) --------------------------------

PATIENCE = settings(knobs={"patience_seconds": 90})
GROUP = "space:tg:-1002"
OTHER = "person:tg:4002"


def _group_msg(text: str, *, who: str = OTHER, addressed: bool = False) -> Event:
    return Event(id=None, ts=START, kind="percept", who=who, place=GROUP, body="telegram",
                 payload={"description": text, "addressed": addressed,
                          "place_kind": "shared"}, description=text)


def _say(text: str) -> list[tuple[str, dict]]:
    return [("act", {"capability": "telegram.say", "arguments": {"text": text}})]


async def test_a_go_on_a_turn_not_allocated_creates_a_wait_from_patience(aconn):
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})

    created = await expectations.create_wait(aconn, source, now=START, settings=PATIENCE)

    cur = await aconn.execute(
        "SELECT kind, who, place, cue_time, cue_event, next_check_at, status "
        "FROM expectations WHERE id = %s", (created.payload["expectation_id"],))
    kind, who, place, cue_time, cue_event, next_at, status = await cur.fetchone()
    assert (kind, who, place, status) == ("wait", WHO, GROUP, "active")
    assert cue_time == {"at": (START + timedelta(seconds=90)).isoformat()}
    assert next_at == START + timedelta(seconds=90)
    assert cue_event == {"kind": "percept", "place": GROUP, "not_who": WHO}
    assert created.kind == "expectation.created"
    assert created.payload["kind"] == "wait"


async def test_a_go_on_an_addressed_turn_creates_no_wait(aconn):
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "Mona, halo",
                                                          "addressed": True})
    quiet = await append_event(aconn, ts=START, kind="percept", who=WHO, body="telegram",
                               payload={"text": "tanpa tempat"})

    assert await expectations.create_wait(aconn, source, now=START, settings=PATIENCE) is None
    assert await expectations.create_wait(aconn, quiet, now=START, settings=PATIENCE) is None
    cur = await aconn.execute("SELECT count(*) FROM expectations")
    assert (await cur.fetchone())[0] == 0


def test_not_who_cue_matches_everyone_but_the_sender():
    cue = {"kind": "percept", "place": GROUP, "not_who": WHO}

    assert expectations.matches_cue(cue, _group_msg("halo", who=OTHER)) is True
    assert expectations.matches_cue(cue, _group_msg("halo", who=WHO)) is False
    assert expectations.matches_cue({"kind": "percept", "who": WHO, "not_who": OTHER},
                              _group_msg("halo", who=WHO)) is True


async def test_another_person_taking_the_floor_ends_the_wait_silently(aconn):
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expectations.create_wait(aconn, source, now=START, settings=PATIENCE)

    await _feed(kernel, _group_msg("aku jawab ya, bagian dua aman"))

    exp_id = int(created.payload["expectation_id"])
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.fired" not in await _kinds(aconn)
    assert "act.out" not in await _kinds(aconn)
    assert fake.requests == []


async def test_the_senders_own_follow_up_does_not_end_the_wait(aconn):
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expectations.create_wait(aconn, source, now=START, settings=PATIENCE)

    await _feed(kernel, _group_msg("maksudku bagian dua", who=WHO))

    exp_id = int(created.payload["expectation_id"])
    assert await _status(aconn, exp_id) == "active"
    assert fake.requests == []


async def test_patience_fires_exactly_one_reply_when_nobody_takes_the_floor(aconn):
    fake = FakeLLM([_say("bagian dua soal alur data")])
    clock = FakeClock(START)
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=PATIENCE)
    kernel.register_adapter(RecordingBody())
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expectations.create_wait(aconn, source, now=START, settings=PATIENCE)

    clock.advance(timedelta(seconds=89))
    await _tick(kernel, clock)
    assert await _status(aconn, int(created.payload["expectation_id"])) == "active"
    assert "act.out" not in await _kinds(aconn)

    clock.advance(timedelta(seconds=1))
    await _tick(kernel, clock)
    assert "expectation.fired" in await _kinds(aconn)
    assert await _status(aconn, int(created.payload["expectation_id"])) == "satisfied"

    clock.advance(timedelta(minutes=5))
    await _tick(kernel, clock)
    cur = await aconn.execute("SELECT place, payload FROM events WHERE kind = 'act.out'")
    replies = await cur.fetchall()
    assert [place for place, _ in replies] == [GROUP]
    assert "bagian dua soal alur data" in json.dumps(replies[0][1])
    assert len(fake.requests) == 1


async def test_an_addressed_turn_is_answered_without_a_wait(aconn):
    fake = FakeLLM([_say("bagian dua soal alur data")])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _group_msg("Mona, jelaskan bagian dua ya", addressed=True))

    kinds = await _kinds(aconn)
    assert kinds.count("act.out") == 1
    assert "expectation.created" not in kinds
    cur = await aconn.execute("SELECT place FROM events WHERE kind = 'act.out'")
    assert await cur.fetchall() == [(GROUP,)]


async def test_a_message_ends_the_step_so_its_result_does_not_wake_again(aconn):
    """12.16: one fired promise produces one message and one wake.

    The act that spoke ends its own step (F.4): its result is recorded as a silent decision
    instead of waking reasoning a second time for a message it already sent.
    """
    clock = FakeClock(START)
    fake = FakeLLM([
        [("expect", {"about": "remind the owner at 7", "cue_time": {"daily": "07:00"},
                     "expect": {"none": True}, "intent": "remind gently", "weight": 1.0,
                     "policy": {"once": True}})],
        [],
        [("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})],
    ])
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("ingatkan saya jam 7"))
    clock.set_time(SEVEN)
    await _tick(kernel, clock)

    kinds = await _kinds(aconn)
    assert kinds.count("model.call") == 3
    cur = await aconn.execute("SELECT count(*) FROM events WHERE kind = 'act.out' "
                              "AND payload->>'communicative' = 'true'")
    assert (await cur.fetchone())[0] == 1
    assert kinds.count("act.out") == 1
    assert kinds.count("expectation.fired") == 1


async def test_a_cue_with_words_is_refused_with_its_reason(aconn):
    """A cue states facts; the words it would match belong to `expect.text`."""
    fake = FakeLLM([[("expect", {"about": "nudge me", "cue_event": {"kind": "percept",
                                                                   "text_like": "workout"},
                              "expect": {"none": True}, "policy": {"once": True}})],
                    []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("nudge me about workouts"))

    assert "cue_event takes facts only (kind, who, place, not_who)" in fake.last_prompt
    assert "use expect: {text: ...} for words" in fake.last_prompt
    cur = await aconn.execute("SELECT count(*) FROM expectations")
    assert await cur.fetchone() == (0,)


async def test_a_stored_cue_with_words_is_read_as_a_free_text_condition(aconn):
    """A row stored before the contract spoke only facts still reminds on its phrase."""
    cur = await aconn.execute(
        "INSERT INTO expectations (kind, who, place, about, cue_event, condition, "
        "condition_text, intent, weight, policy, status, next_check_at, source_event, "
        "derived_from, origin_actor, expires_at) VALUES "
        "('promise', %s, %s, 'minta dikabari', %s, %s, NULL, 'beri tahu', 0.8, %s, 'active', "
        "NULL, NULL, '{}'::bigint[], %s, %s) RETURNING id",
        (WHO, PLACE, json.dumps({"kind": "percept", "who": WHO, "text_like": "server down"}),
         json.dumps({"event_seen": {"kind": "percept", "text_like": "server down"},
                     "within": "2d"}), json.dumps({"once": True}), WHO,
         START + timedelta(days=1)))
    exp_id = int((await cur.fetchone())[0])

    rows = await read_expectations(aconn, "ex.id = %s", (exp_id,))

    assert rows[0]["cue_event"] == {"kind": "percept", "who": WHO}
    assert rows[0]["condition"] == {"text": "server down"}
    assert rows[0]["condition_text"] == "server down"
    # The stored row is never rewritten (contract.md §9).
    raw = await (await aconn.execute(
        "SELECT cue_event FROM expectations WHERE id = %s", (exp_id,))).fetchone()
    assert raw[0]["text_like"] == "server down"
