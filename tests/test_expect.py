"""Tests for Step 6: expectations, the time and event paths, and MVP test (c)."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens import expect
from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.pfc import PFC
from monoagens.store import append_event, write_state
from monoagens.bodies.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event
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


def _pfc(fake: FakeLLM) -> PFC:
    return PFC("https://llm.test/v1", "test-key", "fake-model", settings=SETTINGS,
               transport=httpx.MockTransport(fake.handler))


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
                          "data": {"message_id": message_id}})


def _source() -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                 payload={"text": "source"})


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
    created = await expect.create(conn, data, now=START, settings=SETTINGS, source=source)
    return int(created.payload["expectation_id"]), created


# --- durations and cue times -------------------------------------------------


def test_parse_duration_units():
    assert expect.parse_duration("30s") == timedelta(seconds=30)
    assert expect.parse_duration("30m") == timedelta(minutes=30)
    assert expect.parse_duration("2h") == timedelta(hours=2)
    assert expect.parse_duration("1d") == timedelta(days=1)
    assert expect.parse_duration("1.5h") == timedelta(minutes=90)
    for bad in ("2w", "abc", "", "10"):
        with pytest.raises(ValueError):
            expect.parse_duration(bad)


def test_next_check_at_every_and_daily_in_the_configured_timezone():
    jakarta = expect.tz(SETTINGS)
    assert expect.next_check({"at": "2026-11-11T07:00:00+07:00"}, START, jakarta) == SEVEN
    assert expect.next_check({"every": "2d"}, START, jakarta) == START + timedelta(days=2)
    # 06:00 Jakarta -> 07:00 the same day; 08:00 Jakarta -> 07:00 the next day
    assert expect.next_check({"daily": "07:00"}, START, jakarta) == SEVEN
    late = datetime(2026, 11, 11, 1, 0, tzinfo=timezone.utc)  # 08:00 Jakarta
    assert expect.next_check({"daily": "07:00"}, late, jakarta) == SEVEN + timedelta(days=1)
    assert expect.next_check(None, START, jakarta) is None
    with pytest.raises(ValueError):
        expect.next_check({"whenever": True}, START, jakarta)


def test_validation_defaults_and_rejections():
    row = expect.validate({"about": "x", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                          now=START, settings=SETTINGS)
    assert row["weight"] == 0.5
    assert row["policy"] == {"once": True}
    assert row["expires_at"] == START + timedelta(days=30)
    assert row["next_check_at"] == START + timedelta(days=1)

    explicit = expect.validate({"about": "x", "cue_time": {"at": START.isoformat()},
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
            expect.validate(data, now=START, settings=SETTINGS)


async def test_max_active_is_enforced(aconn):
    runtime = settings(constants={"max_active": 1})
    await expect.create(aconn, {"about": "first", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                        now=START, settings=runtime, source=_source())
    with pytest.raises(ValueError, match="too many active"):
        await expect.create(aconn, {"about": "second", "cue_time": {"every": "1d"}, "expect": {"none": True}},
                            now=START, settings=runtime, source=_source())


# --- the three operators -----------------------------------------------------


async def test_operator_none_is_never_met(aconn):
    assert await expect.is_met(aconn, {"none": True}, START) is False


async def test_operator_event_seen_checks_the_window(aconn):
    old = await append_event(aconn, ts=START - timedelta(days=3), kind="percept", who=WHO, place=PLACE,
                             body="telegram", payload={"text": "I did my WORKOUT"})
    assert old.id is not None
    pattern = {"kind": "percept", "who": WHO, "text_like": "workout"}
    assert await expect.is_met(aconn, {"event_seen": pattern, "within": "4d"}, START) is True
    assert await expect.is_met(aconn, {"event_seen": pattern, "within": "2d"}, START) is False
    assert await expect.is_met(aconn, {"event_seen": {"kind": "percept", "text_like": "swim"}, "within": "4d"},
                               START) is False


async def test_operator_state_compares_eq_gt_lt(aconn):
    seed = await append_event(aconn, ts=START, kind="decision.note", place=PLACE)
    await write_state(aconn, "mood", {"value": 5}, seed.id)

    assert await expect.is_met(aconn, {"state": "mood", "op": "eq", "value": 5}, START) is True
    assert await expect.is_met(aconn, {"state": "mood", "op": "gt", "value": 3}, START) is True
    assert await expect.is_met(aconn, {"state": "mood", "op": "lt", "value": 3}, START) is False
    assert await expect.is_met(aconn, {"state": "missing", "op": "eq", "value": 1}, START) is False


def test_matches_cue_is_exact_on_ids_and_substring_on_text():
    event = _msg("waktu Workout sore")
    assert expect.matches_cue({"kind": "percept", "who": WHO, "place": PLACE}, event) is True
    assert expect.matches_cue({"text_like": "WORKOUT"}, event) is True
    assert expect.matches_cue({"who": "person:other"}, event) is False
    assert expect.matches_cue({"kind": "act.out"}, event) is False
    assert expect.matches_cue(None, event) is False


# --- the time path and the event path ----------------------------------------


async def test_time_path_fires_only_when_next_check_is_due(aconn):
    exp_id, _ = await _create(aconn, weight=1.0)
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "fired"
    assert (await _kinds(aconn)).count("expectation.fired") == 1

    cur = await aconn.execute("SELECT fire_count, last_fired_at FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == (1, START)


async def test_time_path_ignores_future_expectations(aconn):
    exp_id, _ = await _create(aconn, cue_time={"at": (START + timedelta(hours=1)).isoformat()})
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "active"
    assert "expectation.fired" not in await _kinds(aconn)


async def test_event_path_fires_on_a_matching_event(aconn):
    exp_id, _ = await _create(aconn, cue_time=None,
                              cue_event={"kind": "percept", "who": WHO, "text_like": "workout"})
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

    await _feed(kernel, _msg("halo", message_id=1))
    assert await _status(aconn, exp_id) == "active"

    await _feed(kernel, _msg("saya sudah WORKOUT", message_id=2))
    assert await _status(aconn, exp_id) == "fired"


async def test_event_seen_expectation_that_is_met_never_fires(aconn):
    pattern = {"kind": "percept", "who": WHO, "text_like": "workout"}
    exp_id, _ = await _create(aconn, cue_time=None, cue_event=pattern,
                              expect={"event_seen": pattern, "within": "2d"})
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

    await _feed(kernel, _msg("saya sudah workout hari ini"))

    kinds = await _kinds(aconn)
    assert "expectation.fired" not in kinds
    assert await _status(aconn, exp_id) == "satisfied"
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'expectation.resolved'")
    assert (await cur.fetchone())[0]["status"] == "satisfied"


async def test_time_path_met_event_seen_resolves_without_firing(aconn):
    pattern = {"kind": "percept", "who": WHO, "text_like": "workout"}
    await append_event(aconn, ts=START - timedelta(hours=1), kind="percept", who=WHO, place=PLACE,
                       body="telegram", payload={"text": "workout done"})
    exp_id, _ = await _create(aconn, expect={"event_seen": pattern, "within": "2d"})
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.fired" not in await _kinds(aconn)


# --- policies ----------------------------------------------------------------


async def test_low_weight_fired_expectation_still_wakes_the_pfc_once(aconn):
    """A missed expectation below the silent band fires the PFC anyway (DESIGN appraisal)."""
    exp_id, _ = await _create(aconn, weight=0.3)
    clock = FakeClock(START)
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})]])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_body(RecordingBody())

    await _tick(kernel, clock)

    kinds = await _kinds(aconn)
    assert kinds.count("expectation.fired") == 1
    assert kinds.count("act.out") == 1
    assert len(fake.requests) == 1
    assert await _status(aconn, exp_id) == "satisfied"


async def test_policy_once_is_satisfied_after_the_turn(aconn):
    exp_id, _ = await _create(aconn)
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(kernel, clock)

    await expect.settle(aconn, exp_id, now=START, settings=SETTINGS)
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.resolved" in await _kinds(aconn)


async def test_recurring_policy_reschedules_by_cooldown_then_drops_at_max_fires(aconn):
    exp_id, _ = await _create(aconn, policy={"cooldown": "6h", "max_fires": 2})
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)

    await _tick(kernel, clock)
    await expect.settle(aconn, exp_id, now=START, settings=SETTINGS)
    cur = await aconn.execute("SELECT status, next_check_at FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == ("active", START + timedelta(hours=6))

    clock.set_time(START + timedelta(hours=5))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("expectation.fired") == 1

    clock.set_time(START + timedelta(hours=6))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("expectation.fired") == 2
    await expect.settle(aconn, exp_id, now=clock.now(), settings=SETTINGS)
    cur = await aconn.execute("SELECT status, fire_count FROM expectations WHERE id = %s", (exp_id,))
    assert await cur.fetchone() == ("dropped", 2)


# --- the PFC tools -----------------------------------------------------------


async def test_expect_tool_creates_an_expectation_and_records_it(aconn):
    fake = FakeLLM([[("expect", {"about": "remind me at 7", "cue_time": {"daily": "07:00"},
                                "expect": {"none": True}, "intent": "remind gently",
                                "weight": 0.9, "policy": {"once": True}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("ingatkan saya jam 7"))

    assert await _kinds(aconn) == ["percept", "model.call", "expectation.created", "decision.silent"]
    cur = await aconn.execute("SELECT who, place, about, weight, status, next_check_at, expires_at "
                              "FROM expectations")
    who, place, about, weight, status, next_check, expires = await cur.fetchone()
    assert (who, place, about, weight, status) == (WHO, PLACE, "remind me at 7", pytest.approx(0.9), "active")
    assert next_check == SEVEN
    assert expires == START + timedelta(days=30)


async def test_expect_with_an_invalid_weight_becomes_an_error_event(aconn):
    fake = FakeLLM([[("expect", {"about": "bad", "cue_time": {"daily": "07:00"},
                                "expect": {"none": True}, "weight": 2.0})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("buat janji"))

    kinds = await _kinds(aconn)
    assert kinds == ["percept", "model.call", "error"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'error'")
    assert "weight" in (await cur.fetchone())[0]["error"]
    cur = await aconn.execute("SELECT count(*) FROM expectations")
    assert (await cur.fetchone())[0] == 0


async def test_resolve_tool_closes_an_expectation(aconn):
    exp_id, _ = await _create(aconn)
    fake = FakeLLM([[("resolve", {"id": exp_id, "status": "dropped"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

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
        [("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})],
    ])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_body(body)

    await _feed(kernel, _msg("ingatkan saya jam 7"))
    assert await _kinds(aconn) == ["percept", "model.call", "expectation.created", "decision.silent"]

    clock.set_time(SEVEN - timedelta(minutes=10))
    await _tick(kernel, clock)
    assert (await _kinds(aconn)).count("act.out") == 0

    clock.set_time(SEVEN)
    await _tick(kernel, clock)
    kinds = await _kinds(aconn)
    assert kinds.count("expectation.fired") == 1
    assert kinds.count("act.out") == 1
    assert body.actions == [Action(kind="say", place=PLACE, payload={"text": "sudah jam 7"}, body="telegram")]
    assert "expectation missed: remind the owner at 7" in fake.last_prompt.split("<why_awake>")[1]

    cur = await aconn.execute("SELECT status, fire_count FROM expectations")
    assert await cur.fetchone() == ("satisfied", 1)

    # The act.out is linked to the fired event, which is what recovery relies on.
    cur = await aconn.execute("SELECT f.id, m.caused_by FROM events f JOIN events m ON m.caused_by = f.id "
                              "WHERE f.kind = 'expectation.fired' AND m.kind = 'act.out'")
    assert (await cur.fetchone())[0] is not None

    # Restart right after the reminder: no second message, no second LLM call.
    restarted_fake = FakeLLM()
    restarted = Kernel(aconn, clock=clock, pfc=_pfc(restarted_fake), settings=SETTINGS)
    restarted.register_body(RecordingBody())
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
    # No PFC in the first process: the reminder fired but was never answered.
    crashed = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(crashed, clock)
    assert await _status(aconn, exp_id) == "fired"

    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "sudah jam 7"}})]])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_body(RecordingBody())
    await kernel.recover()
    await kernel.run_until_idle()

    assert len(fake.requests) == 1
    assert (await _kinds(aconn)).count("act.out") == 1
    assert await _status(aconn, exp_id) == "satisfied"

    # A second restart finds nothing left to do.
    again_fake = FakeLLM()
    again = Kernel(aconn, clock=clock, pfc=_pfc(again_fake), settings=SETTINGS)
    await again.recover()
    await again.run_until_idle()
    assert again_fake.requests == []


async def test_recovery_only_resolves_when_the_reply_already_exists(aconn):
    exp_id, _ = await _create(aconn)
    clock = FakeClock(START)
    crashed = Kernel(aconn, clock=clock, pfc=None, settings=SETTINGS)
    await _tick(crashed, clock)

    cur = await aconn.execute("SELECT id FROM events WHERE kind = 'expectation.fired'")
    fired_id = (await cur.fetchone())[0]
    await append_event(aconn, ts=START, kind="act.out", who=WHO, place=PLACE, body="telegram",
                       payload={"text": "sudah jam 7"}, caused_by=fired_id)

    fake = FakeLLM()
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)
    await kernel.recover()
    await kernel.run_until_idle()

    assert fake.requests == []
    assert await _status(aconn, exp_id) == "satisfied"
    assert (await _kinds(aconn)).count("act.out") == 1


async def test_recovery_does_not_touch_active_expectations(aconn):
    exp_id, _ = await _create(aconn, cue_time={"at": (START + timedelta(days=1)).isoformat()})
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    await kernel.recover()
    await kernel.run_until_idle()

    assert fake.requests == []
    assert await _status(aconn, exp_id) == "active"


# --- kinds ------------------------------------------------------------------

KINDS = ("promise", "wait", "predict", "guard")


@pytest.mark.parametrize("kind", KINDS)
def test_validation_accepts_every_kind(kind):
    row = expect.validate({"about": "x", "kind": kind, "cue_time": {"every": "1d"},
                           "expect": {"none": True}}, now=START, settings=SETTINGS)
    assert row["kind"] == kind
    assert row["condition"] == {"none": True}


def test_validation_defaults_to_promise_and_rejects_unknown_kinds():
    row = expect.validate({"about": "x", "cue_time": {"every": "1d"},
                           "expect": {"none": True}}, now=START, settings=SETTINGS)
    assert row["kind"] == "promise"
    with pytest.raises(ValueError, match="unknown expectation kind"):
        expect.validate({"about": "x", "kind": "wish", "cue_time": {"every": "1d"},
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
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS)

    await _tick(kernel, clock)

    assert await _status(aconn, exp_id) == "active"
    assert "expectation.fired" not in await _kinds(aconn)
    assert fake.requests == []


async def test_guard_never_fires_when_its_cue_event_arrives(aconn):
    exp_id, _ = await _create(aconn, kind="guard", cue_time=None,
                              cue_event={"kind": "percept", "who": WHO,
                                         "text_like": "workout"})
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS)

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
                          "place_kind": "shared"})


def _say(text: str) -> list[tuple[str, dict]]:
    return [("act", {"capability": "telegram.say", "arguments": {"text": text}})]


async def test_a_go_on_a_turn_not_allocated_creates_a_wait_from_patience(aconn):
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})

    created = await expect.create_wait(aconn, source, now=START, settings=PATIENCE)

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

    assert await expect.create_wait(aconn, source, now=START, settings=PATIENCE) is None
    assert await expect.create_wait(aconn, quiet, now=START, settings=PATIENCE) is None
    cur = await aconn.execute("SELECT count(*) FROM expectations")
    assert (await cur.fetchone())[0] == 0


def test_not_who_cue_matches_everyone_but_the_sender():
    cue = {"kind": "percept", "place": GROUP, "not_who": WHO}

    assert expect.matches_cue(cue, _group_msg("halo", who=OTHER)) is True
    assert expect.matches_cue(cue, _group_msg("halo", who=WHO)) is False
    assert expect.matches_cue({"kind": "percept", "who": WHO, "not_who": OTHER},
                              _group_msg("halo", who=WHO)) is True


async def test_another_person_taking_the_floor_ends_the_wait_silently(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expect.create_wait(aconn, source, now=START, settings=PATIENCE)

    await _feed(kernel, _group_msg("aku jawab ya, bagian dua aman"))

    exp_id = int(created.payload["expectation_id"])
    assert await _status(aconn, exp_id) == "satisfied"
    assert "expectation.fired" not in await _kinds(aconn)
    assert "act.out" not in await _kinds(aconn)
    assert fake.requests == []


async def test_the_senders_own_follow_up_does_not_end_the_wait(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expect.create_wait(aconn, source, now=START, settings=PATIENCE)

    await _feed(kernel, _group_msg("maksudku bagian dua", who=WHO))

    exp_id = int(created.payload["expectation_id"])
    assert await _status(aconn, exp_id) == "active"
    assert fake.requests == []


async def test_patience_fires_exactly_one_reply_when_nobody_takes_the_floor(aconn):
    fake = FakeLLM([_say("bagian dua soal alur data")])
    clock = FakeClock(START)
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), settings=PATIENCE)
    kernel.register_body(RecordingBody())
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "obrolan biasa"})
    created = await expect.create_wait(aconn, source, now=START, settings=PATIENCE)

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
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=PATIENCE)
    kernel.register_body(RecordingBody())

    await _feed(kernel, _group_msg("Mona, jelaskan bagian dua ya", addressed=True))

    kinds = await _kinds(aconn)
    assert kinds.count("act.out") == 1
    assert "expectation.created" not in kinds
    cur = await aconn.execute("SELECT place FROM events WHERE kind = 'act.out'")
    assert await cur.fetchall() == [(GROUP,)]
