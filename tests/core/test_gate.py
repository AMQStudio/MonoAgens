"""Tests for the cognition layer: Situation, signals, appraisal, hooks, and tokens."""

from __future__ import annotations

import asyncio
import json
import math
from dataclasses import replace
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

import httpx
import pytest

from monoagens.core import memory
from monoagens.infra.clock import FakeClock
from monoagens.core.gate import (SIGNALS, Appraisal, Situation, appraise, assemble_situation,
                                 formula, seeded_rng, validate_signals)
from monoagens.core.gate.situation import boredom_gamma, effective_salience, habituate
from monoagens.core.kernel import Kernel
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.hooks import Continue, HookPipeline, HookRegistry, Replace, Stop
from monoagens.core.reasoning import Reasoning
from monoagens.infra.providers import OpenAICompatibleProvider
from monoagens.infra.store import (append_event, insert_event_vector, read_event_vector,
                                  read_state, write_state)
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.contract import Action
from monoagens.types import Event
from monoagens.boot import build, install
from tests.support import body_tiers, open_book, settings

SETTINGS = settings()
PLACE = "space:tg:1"
GROUP = "space:tg:-100123"
WHO = "person:owner"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
JAKARTA = ZoneInfo("Asia/Jakarta")


class FakeLLM:
    """Scripted OpenAI-compatible endpoint that also reports token usage."""

    def __init__(self, responses: list[list[tuple[str, dict]]] | None = None) -> None:
        self.responses = responses or [[]]
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        tool_calls = [{"id": f"call_{i}", "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments)}}
                      for i, (name, arguments) in enumerate(calls)]
        return httpx.Response(200, json={
            "choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
                      "prompt_tokens_details": {"cached_tokens": 4}},
        })


class CountingEmbedder:
    """A FakeEmbedder that counts how many embed calls reached it."""

    def __init__(self) -> None:
        self.calls = 0
        self._inner = FakeEmbedder()

    async def embed(self, texts):
        self.calls += 1
        return await self._inner.embed(texts)


class RecordingBody:
    name = "telegram"
    PERMISSIONS = body_tiers(TELEGRAM_CAPABILITIES)

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning(
        OpenAICompatibleProvider("https://llm.test/v1", "test-key", "fake-model",
                                 transport=httpx.MockTransport(fake.handler)),
        settings=SETTINGS)


def _msg(text: str = "halo", *, message_id: int = 1, place: str = PLACE,
         place_kind: str = "direct", addressed: bool = True,
         features: dict[str, float] | None = None) -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=place, body="telegram",
                 description=text,
                 features={**({"addressed": 1} if addressed else {}), **(features or {})},
                 payload={"place_kind": place_kind, "data": {"message_id": message_id}})


def _situation(**overrides) -> Situation:
    event = overrides.pop("event", _msg())
    return Situation(event=event, **overrides)


async def _feed(kernel: Kernel, event: Event) -> Event:
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _kinds(conn) -> list[str]:
    return [row[0] for row in await (await conn.execute(
        "SELECT kind FROM events WHERE kind NOT IN ('job.started', 'job.result') "
        "ORDER BY id")).fetchall()]


async def _payload(conn, kind: str) -> dict:
    cur = await conn.execute("SELECT payload FROM events WHERE kind = %s ORDER BY id", (kind,))
    return (await cur.fetchone())[0]


def _volatile(fake: FakeLLM) -> str:
    return fake.requests[-1]["messages"][2]["content"]


# --- signals and validation --------------------------------------------------


def test_the_five_initial_signals_are_registered():
    assert set(SIGNALS) == {"own_name", "info_gap", "feeling_of_knowing", "relevance",
                            "social_cost"}


def test_validate_signals_accepts_the_configured_weights_and_fails_otherwise():
    validate_signals({name: 1.0 for name in SIGNALS})
    with pytest.raises(ValueError, match="missing"):
        validate_signals({"own_name": 1.0})
    with pytest.raises(ValueError, match="unknown"):
        validate_signals({**{name: 0.0 for name in SIGNALS}, "bogus": 1.0})


def test_signals_are_pure_bounded_functions_of_a_situation():
    situation = _situation(vector=[0.0] * 384, recent=())
    for signal in SIGNALS.values():
        value = signal(situation)
        assert isinstance(value, float)
        assert 0.0 <= value <= 1.0
        assert signal(situation) == value


def test_own_name_scores_one_only_when_the_turn_is_not_allocated():
    """`own_name` reads the two facts the adapter reports, never the text (contract.md §3)."""
    named = _msg("dokumen baru sudah aku kirim", addressed=False, features={"named": 1})
    tagged = _msg("lihat ini", addressed=True, features={"named": 1})
    other = _msg("dokumen baru sudah aku kirim", addressed=False)
    assert SIGNALS["own_name"](_situation(event=named, self_name="Mona")) == 1.0
    assert SIGNALS["own_name"](_situation(event=tagged, self_name="Mona")) == 0.0
    assert SIGNALS["own_name"](_situation(event=other, self_name="Mona")) == 0.0


def test_info_gap_is_the_cosine_to_the_centroid_of_the_prior_references():
    # The centroid of [1, 0] and [0, 1] is [0.5, 0.5]: cosine with [1, 0] is 1/sqrt(2).
    situation = _situation(vector=[1.0, 0.0], references=([1.0, 0.0], [0.0, 1.0]))
    assert SIGNALS["info_gap"](situation) == pytest.approx(1 / math.sqrt(2))
    unrelated = _situation(vector=[1.0, 0.0], references=([0.0, 1.0], [-1.0, 0.0]))
    assert SIGNALS["info_gap"](unrelated) == 0.0
    empty = _situation(vector=[1.0, 0.0], references=())
    assert SIGNALS["info_gap"](empty) == 0.0


def test_feeling_of_knowing_is_high_only_when_the_answer_is_known():
    known = _situation(best_activation=3.0, fok_threshold=0.0, fok_scale=1.0)
    unknown = _situation(best_activation=-3.0, fok_threshold=0.0, fok_scale=1.0)
    assert SIGNALS["feeling_of_knowing"](known) > 0.9
    assert SIGNALS["feeling_of_knowing"](unknown) < 0.1
    scaled = _situation(best_activation=1.0, fok_threshold=0.0, fok_scale=2.0)
    assert SIGNALS["feeling_of_knowing"](scaled) == pytest.approx(1 / (1 + math.exp(-0.5)))


def test_relevance_matches_active_promises_and_core_memory():
    promise = _situation(vector=[1.0, 0.0], promise_vectors=([0.0, 1.0],))
    core = _situation(vector=[1.0, 0.0], core_vector=[1.0, 0.0])
    neither = _situation(vector=[1.0, 0.0], core_vector=[0.0, 1.0])
    assert SIGNALS["relevance"](promise) == 0.0
    assert SIGNALS["relevance"](core) == 1.0
    assert SIGNALS["relevance"](neither) == 0.0


def test_social_cost_belongs_to_someone_else_and_is_lowered_by_closeness():
    other = _msg("halo", addressed=False, features={"addressed_other": 1})
    selected = _situation(event=other)
    answered = _situation(event=_msg("halo", addressed=False), soul_answered=True)
    close = _situation(event=other, closeness=0.8)
    tagged = _situation(event=_msg("halo", addressed=True,
                                   features={"addressed_other": 1}))
    assert SIGNALS["social_cost"](selected) == 1.0
    assert SIGNALS["social_cost"](answered) == 1.0
    assert SIGNALS["social_cost"](close) == pytest.approx(0.2)
    assert SIGNALS["social_cost"](tagged) == 0.0


def test_habituation_fades_repeats_with_gamma_from_boredom():
    slow = settings(knobs={"boredom_with_repeats": 0.0})
    fast = settings(knobs={"boredom_with_repeats": 1.0})
    assert boredom_gamma(slow) == pytest.approx(0.99)
    assert boredom_gamma(fast) == pytest.approx(0.8)
    h, saliences = 0.0, []
    for _ in range(3):
        h = habituate({"h": h, "ts": START.isoformat()}, START, 0.8)
        saliences.append(effective_salience(1.0, h))
    assert saliences == [0.5, pytest.approx(1 / 3), pytest.approx(0.25)]
    decayed = habituate({"h": 1.0, "ts": START.isoformat()},
                        START + timedelta(minutes=10), 0.9)
    assert decayed == pytest.approx(0.9**10 + 1)


# --- seeded randomness -------------------------------------------------------


def test_seeded_rng_is_reproducible_and_depends_on_the_event():
    assert seeded_rng(1, 7).random() == seeded_rng(1, 7).random()
    assert seeded_rng(1, 7).random() != seeded_rng(1, 8).random()
    assert seeded_rng(1, None).random() == seeded_rng(1, 0).random()


# --- appraisal ---------------------------------------------------------------


def test_appraisal_payload_carries_every_number_and_the_signals():
    appraisal = Appraisal("note", "why", "reflex", {"a": 0.5}, 0.4, 0.3, 0.6, 0.9,
                          0.35, 0.056, 2.0, 0.25, 0.5, 0.125, 0.75)
    assert appraisal.record() == {
        "decision": "note", "reason": "why", "reflex": "reflex", "signals": {"a": 0.5},
        "s": 0.4, "s_eff": 0.35, "theta": 0.3, "tau": 0.056, "p": 0.6, "u": 0.9,
        "habituation": 2.0, "talk_rate": 0.25, "budget_used": 0.5, "promise_debt": 0.125,
        "familiarity": 0.75,
    }


async def test_addressed_message_is_a_wake_reflex():
    event = _msg("mona, halo", addressed=True)
    appraisal = await appraise(event, settings=SETTINGS)
    assert (appraisal.decision, appraisal.reflex, appraisal.reason) == (
        "wake", "addressed", "addressed message")
    assert (appraisal.s, appraisal.theta, appraisal.p) == (1.0, 0.0, 1.0)
    assert 0.0 <= appraisal.u < 1.0
    assert set(appraisal.signals) == set(SIGNALS)
    assert all(value == 0.0 for value in appraisal.signals.values())


async def test_fired_expectation_is_a_wake_reflex():
    event = Event(None, START, "expectation.fired", who=WHO, place=PLACE,
                  payload={"expectation_id": 1})
    appraisal = await appraise(event, settings=SETTINGS)
    assert (appraisal.decision, appraisal.reflex, appraisal.p) == (
        "wake", "pfc_expectation", 1.0)


async def test_reminded_expectation_is_a_wake_reflex_that_says_reminded():
    event = Event(None, START, "expectation.reminded", who=WHO, place=PLACE,
                  payload={"expectation_id": 1})
    appraisal = await appraise(event, settings=SETTINGS)
    assert (appraisal.decision, appraisal.reflex, appraisal.reason, appraisal.p) == (
        "wake", "pfc_expectation", "reminded", 1.0)


async def test_an_ordinary_message_is_silent_and_reproducible():
    event = Event(42, START, "percept", who=WHO, place=PLACE, payload={"text": "obrolan"}, description="obrolan")
    first = await appraise(event, settings=SETTINGS)
    second = await appraise(event, settings=SETTINGS)
    assert (first.decision, first.reflex, first.reason) == (
        "silent", None, "gate score below silent threshold")
    assert first.s == 0.0
    assert first.theta == pytest.approx(0.925)  # base 0.625 + reserve 0.3, no log
    assert first.p < 1e-6
    assert first.u == second.u == seeded_rng(1, 42).random()


# --- the gate: knob mapping, theta, probability, circuit breaker --------------


def test_gate_params_map_the_prior_knobs_in_one_place():
    low = settings(knobs={"talkativeness": 0.0, "spontaneity": 0.0,
                          "boredom_with_repeats": 0.0, "restraint_after_speaking": 0.0,
                          "reserve_when_unfamiliar": 0.0})
    high = settings(knobs={"talkativeness": 1.0, "spontaneity": 1.0,
                           "boredom_with_repeats": 1.0, "restraint_after_speaking": 1.0,
                           "reserve_when_unfamiliar": 1.0})
    floor, ceiling = formula.gate_params(low), formula.gate_params(high)
    assert (floor.theta_base, floor.tau, floor.gamma) == (0.85, 0.02, 0.99)
    assert (ceiling.theta_base, ceiling.tau, ceiling.gamma) == (0.35, 0.2, 0.8)
    assert (floor.talk_weight, floor.reserve) == (0.0, 0.0)
    assert (ceiling.talk_weight, ceiling.reserve) == (1.0, 1.0)
    assert floor.weights == formula.gate_params(SETTINGS).weights
    assert floor.weights["own_name"] == 0.5 and floor.weights["social_cost"] == -0.4
    assert formula.gate_params(SETTINGS).note_band == 0.2


def test_theta_is_the_documented_sum_of_the_drives():
    params = formula.gate_params(SETTINGS)
    value = formula.theta(params, talk_rate=0.2, budget_used=0.5, promise_debt=0.4,
                       familiarity=0.25)
    expected = 0.625 + 0.5 * 0.2 + 0.3 * 0.5 - 0.1 * 0.4 + 0.3 * (1.0 - 0.25)
    assert value == pytest.approx(expected)
    assert formula.theta(params, talk_rate=-0.5, budget_used=0.0, promise_debt=0.0,
                      familiarity=1.0) == pytest.approx(0.625)


def test_probability_is_a_sigmoid_around_theta_and_the_bands_are_explicit():
    params = formula.gate_params(SETTINGS)
    assert formula.probability(0.4, 0.4, params.tau) == 0.5
    assert formula.probability(0.5, 0.3, params.tau) > 0.9
    assert formula.probability(0.1, 0.9, params.tau) < 1e-6
    assert formula.decide(0.1, 0.5, 0.2) == ("wake", "gate opened: u < P")
    assert formula.decide(0.9, 0.5, 0.2) == ("note", "gate score in the note band, no go")
    assert formula.decide(0.9, 0.05, 0.2) == ("silent",
                                           "gate score below silent threshold")


async def test_same_event_and_seed_give_the_same_decision():
    event = Event(7, START, "percept", who=WHO, place=GROUP, body="telegram",
                  payload={"description": "obrolan grup"}, description="obrolan grup")
    situation = _situation(event=event, vector=[1.0, 0.0], best_activation=1.0,
                           familiarity=0.5)
    first = await appraise(event, situation=situation, settings=SETTINGS)
    second = await appraise(event, situation=situation, settings=SETTINGS)
    assert first.record() == second.record()
    assert first.u == seeded_rng(1, 7).random()
    reseeded = settings()
    reseeded["constants"] = {**reseeded["constants"], "seed": 2}
    other = await appraise(event, situation=situation, settings=reseeded)
    assert other.u == seeded_rng(2, 7).random() and other.u != first.u


async def test_the_gate_opens_on_a_salient_unaddressed_message():
    event = Event(3, START, "percept", who=WHO, place=GROUP, body="telegram",
                  description="Mona, ada yang tahu nggak?", features={"named": 1})
    situation = _situation(event=event, vector=[1.0, 0.0], references=([1.0, 0.0],),
                           self_name="Mona", best_activation=-3.0, familiarity=1.0)
    appraisal = await appraise(event, situation=situation, settings=SETTINGS)
    assert appraisal.decision == "wake" and appraisal.reflex is None
    assert appraisal.reason == "gate opened: u < P"
    assert appraisal.u < appraisal.p and appraisal.p > 0.9
    assert appraisal.s_eff > appraisal.theta


async def test_a_busy_group_keeps_the_soul_at_or_below_its_fair_share(aconn):
    for index in range(6):
        await append_event(aconn, ts=START - timedelta(seconds=30), kind="percept",
                           who=f"person:tg:{index}", place=GROUP, body="telegram",
                           payload={"description": f"obrolan {index}"})
    current = await append_event(aconn, ts=START, kind="percept", who="person:tg:0",
                                 place=GROUP, body="telegram",
                                 payload={"description": "halo semua"})
    quiet = await appraise(current, conn=aconn, now=START, settings=SETTINGS)
    assert quiet.talk_rate == 0.0
    assert quiet.decision == "silent"

    for _ in range(10):
        await append_event(aconn, ts=START, kind="act.out", who=WHO, place=GROUP,
                           payload={"capability": "telegram.say"})
    talkative = await appraise(current, conn=aconn, now=START, settings=SETTINGS)
    assert talkative.talk_rate == pytest.approx(10 / 17 - 1 / 7)
    assert talkative.theta > quiet.theta
    assert talkative.p < quiet.p


async def test_promise_debt_is_the_share_due_within_the_hour(aconn):
    from monoagens.core import expectations
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                body="telegram", payload={"text": "halo"})
    for minutes in (30, 60 * 24 * 3):
        await expectations.create(aconn, {
            "about": "janji", "kind": "promise",
            "cue_time": {"at": (START + timedelta(minutes=minutes)).isoformat()},
            "expect": {"none": True}, "policy": {"once": True}},
            now=START, settings=SETTINGS, source=source)
    inputs = await formula.inputs(aconn, source, now=START, settings=SETTINGS)
    assert inputs.promise_debt == pytest.approx(0.5)
    params = formula.gate_params(SETTINGS)
    assert formula.theta(params, talk_rate=0.0, budget_used=0.0, promise_debt=0.5,
                      familiarity=1.0) == pytest.approx(0.625 - 0.05)


async def test_familiarity_comes_from_act_r_activation_of_the_place(aconn):
    fresh = Event(1, START, "percept", who=WHO, place="space:tg:999", payload={})
    unknown = await formula.inputs(aconn, fresh, now=START, settings=SETTINGS)
    assert unknown.familiarity == 0.0
    for index in range(3):
        await append_event(aconn, ts=START - timedelta(minutes=30 - index), kind="percept",
                           who=WHO, place="space:tg:999", body="telegram",
                           payload={"text": "halo"})
    known = await formula.inputs(aconn, fresh, now=START, settings=SETTINGS)
    activation = memory.base_level(2, 0.5, memory.decay_d(SETTINGS))
    assert known.familiarity == pytest.approx(formula.familiarity(activation))
    assert known.familiarity > 0.5


async def test_addressed_always_wakes_even_with_the_gate_saturated(aconn):
    runtime = settings(owner_settings={"daily_token_budget": 100})
    key = f"budget:{START.astimezone(JAKARTA).date().isoformat()}"
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=GROUP,
                                body="telegram", payload={"text": "halo"})
    await write_state(aconn, key, {"total_tokens": 500}, source.id)
    for _ in range(20):
        await append_event(aconn, ts=START, kind="act.out", who=WHO, place=GROUP,
                           payload={"capability": "telegram.say"})
    tagged = Event(None, START, "percept", who=WHO, place=GROUP, body="telegram",
                   description="Mona, halo?", features={"addressed": 1})
    appraisal = await appraise(tagged, conn=aconn, now=START, settings=runtime)
    assert (appraisal.decision, appraisal.reflex, appraisal.reason) == (
        "wake", "addressed", "addressed message")
    params = formula.gate_params(runtime)
    inputs = await formula.inputs(aconn, tagged, now=START, settings=runtime)
    knew = formula.theta(params, talk_rate=inputs.talk_rate, budget_used=1.0,
                      promise_debt=inputs.promise_debt, familiarity=inputs.familiarity)
    assert knew > 0.9  # the draw could not have opened the gate
    assert formula.probability(0.0, knew, params.tau) < 1e-3


async def test_a_gate_go_on_an_unaddressed_turn_creates_a_wait_instead_of_waking(aconn):
    go = Appraisal(decision="wake", reason="gate opened: u < P", s=0.9, s_eff=0.9,
                   theta=0.6, p=1.0, u=0.1)

    class GateKernel(Kernel):
        async def _appraise(self, event):
            return go

    fake = FakeLLM()
    kernel = GateKernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    install(kernel)
    await _feed(kernel, _msg("obrolan grup", place=GROUP, place_kind="shared",
                             addressed=False))

    assert fake.requests == []
    kind, payload = (await (await aconn.execute(
        "SELECT kind, payload FROM events WHERE kind IN ('decision.wake', "
        "'expectation.created') ORDER BY id")).fetchall())[1]
    assert kind == "expectation.created" and payload["kind"] == "wait"
    stored = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'decision.wake'")).fetchone()
    assert stored[0]["p"] == 1.0 and stored[0]["u"] == 0.1


async def test_the_circuit_breaker_wakes_on_a_fired_expectation_and_a_goal_result(aconn):
    from monoagens.core import expectations
    goal = Event(11, START, "act.result", who=WHO, place=PLACE, body="cli",
                 payload={"capability": "cli.run", "result": {"ok": True}})
    continued = await appraise(goal, settings=SETTINGS)
    assert (continued.decision, continued.reflex, continued.reason) == (
        "wake", "goal", "goal continued")
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                body="telegram", payload={"text": "halo"})
    await expectations.create(aconn, {
        "about": "balasan Budi", "kind": "promise",
        "cue_event": {"kind": "percept", "place": PLACE},
        "expect": {"none": True}, "policy": {"once": True}},
        now=START, settings=SETTINGS, source=source)
    answer = Event(None, START, "percept", who="person:budi", place=PLACE, body="telegram",
                   payload={"description": "budi menjawab"}, description="budi menjawab")
    engaged = await appraise(answer, conn=aconn, now=START, settings=SETTINGS)
    assert (engaged.decision, engaged.reflex, engaged.reason) == (
        "wake", "expectation", "expectation engaged")


# --- situation and encode once ----------------------------------------------


async def test_assemble_situation_holds_vector_recent_and_state(aconn):
    earlier = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                 body="telegram", payload={"text": "kopi"})
    await insert_event_vector(aconn, earlier.id, [0.5] * 384)
    await write_state(aconn, "cognition:weights", {"own_name": 1.0}, earlier.id)
    await write_state(aconn, f"habit:{PLACE}:{WHO}", {"h": 1}, earlier.id)
    current = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                 body="telegram", payload={"text": "halo"})

    situation = await assemble_situation(aconn, current, now=START, settings=SETTINGS)
    assert situation.event.id == current.id
    assert situation.vector is None
    assert [seen.id for seen in situation.recent] == [earlier.id, current.id]
    assert situation.state["cognition:weights"] == {"own_name": 1.0}
    assert situation.habituation == pytest.approx(2.0)
    assert situation.state[f"habit:{PLACE}:{WHO}"] == {"h": 2.0, "ts": START.isoformat()}
    assert f"budget:{START.astimezone(JAKARTA).date().isoformat()}" not in situation.state

    older = await assemble_situation(aconn, earlier, now=START, settings=SETTINGS)
    assert older.vector is not None and len(older.vector) == 384


async def test_assemble_situation_habituates_per_place_and_person(aconn):
    previous = await append_event(aconn, ts=START - timedelta(minutes=10), kind="percept",
                                  who=WHO, place=PLACE, body="telegram",
                                  payload={"text": "stiker lagi"})
    await write_state(aconn, f"habit:{PLACE}:{WHO}", {
        "h": 2.0, "ts": (START - timedelta(minutes=10)).isoformat()}, previous.id)
    current = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                 body="telegram", payload={"text": "stiker lagi"})

    situation = await assemble_situation(aconn, current, now=START, settings=SETTINGS)
    expected = 2.0 * boredom_gamma(SETTINGS) ** 10 + 1

    assert situation.habituation == pytest.approx(expected)
    assert (await read_state(aconn, f"habit:{PLACE}:{WHO}"))["h"] == pytest.approx(expected)


async def test_fok_scores_high_only_when_the_answer_is_known(aconn):
    embedder = FakeEmbedder()
    unknown = "lupa nama tempatnya apa ya"
    cue = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                             body="telegram", payload={"description": unknown})

    before = await assemble_situation(aconn, cue, now=START, settings=SETTINGS,
                                      embedder=embedder)
    assert SIGNALS["feeling_of_knowing"](before) < 0.6

    answer = (await embedder.embed(["nama tempat itu Bandung"]))[0]
    await aconn.execute(
        "INSERT INTO memories (who, place, text, embedding, importance, last_accessed_at, "
        "created_at) VALUES (%s, %s, %s, %s, %s, %s, %s)",
        (WHO, PLACE, "nama tempat itu Bandung", answer, 0.9, START, START))

    after = await assemble_situation(aconn, cue, now=START, settings=SETTINGS,
                                     embedder=embedder)
    assert SIGNALS["feeling_of_knowing"](after) > 0.9


async def test_kernel_encodes_a_message_once_and_retrieval_reuses_the_vector(aconn):
    embedder = CountingEmbedder()
    kernel = build(aconn, clock=FakeClock(START), pfc=None, settings=SETTINGS,
                    embedder=embedder)

    stored = await _feed(kernel, _msg("halo"))

    assert stored.id is not None
    assert embedder.calls == 1
    vector = await read_event_vector(aconn, stored.id)
    assert vector is not None and len(vector) == 384

    values = SETTINGS["constants"]
    await memory.retrieve(aconn, embedder, "halo", now=START, event_id=stored.id,
                          k=values["retrieve_k"], settings=SETTINGS)
    assert embedder.calls == 1
    await memory.retrieve(aconn, embedder, "halo", now=START,
                          k=values["retrieve_k"], settings=SETTINGS)
    assert embedder.calls == 2


async def test_kernel_encodes_the_reply_once_too(aconn):
    embedder = CountingEmbedder()
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS,
                    embedder=embedder)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))

    # The incoming message and the reply are encoded by background jobs; the wake
    # that decides the reply runs before the intake vector lands, so the FOK
    # retrieval and the memory retrieval each embed their query once more.
    assert embedder.calls == 4
    rows = await (await aconn.execute(
        "SELECT event_id FROM event_vectors ORDER BY event_id")).fetchall()
    assert len(rows) == 2


# --- kernel decisions and token usage ---------------------------------------


async def test_silent_decision_event_carries_the_full_appraisal(aconn):
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    await _feed(kernel, _msg("obrolan biasa", place=GROUP, place_kind="shared",
                             addressed=False))

    assert await _kinds(aconn) == ["percept", "decision.silent"]
    payload = await _payload(aconn, "decision.silent")
    assert set(payload) == {"decision", "reason", "reflex", "signals", "s", "s_eff", "theta",
                            "tau", "p", "u", "habituation", "talk_rate", "budget_used",
                            "promise_debt", "familiarity"}
    assert (payload["decision"], payload["reason"], payload["reflex"]) == (
        "silent", "gate score below silent threshold", None)
    assert payload["s"] == pytest.approx(0.175)  # no memory: FOK sits at 0.5
    assert payload["s_eff"] == pytest.approx(payload["s"] / 2)  # first stimulus, h = 1
    assert payload["theta"] == pytest.approx(0.6298, abs=1e-3)
    assert payload["p"] < 1e-3
    assert payload["talk_rate"] == 0.0 and payload["budget_used"] == 0.0
    assert payload["promise_debt"] == 0.0 and payload["familiarity"] > 0.9
    stored = await (await aconn.execute("SELECT id FROM events WHERE kind = 'percept'")).fetchone()
    assert payload["u"] == seeded_rng(1, stored[0]).random()
    assert fake.requests == []


async def test_llm_token_usage_lands_in_the_daily_budget_key(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])
    pfc = _pfc(fake)
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))

    assert pfc.last_usage == {"prompt_tokens": 10, "cached_tokens": 4,
                              "completion_tokens": 5, "reasoning_tokens": 0,
                              "total_tokens": 15}
    key = f"budget:{START.astimezone(JAKARTA).date().isoformat()}"
    assert await read_state(aconn, key) == {"prompt_tokens": 10, "cached_tokens": 4,
                                            "completion_tokens": 5, "reasoning_tokens": 0,
                                            "total_tokens": 15}
    call, caused_by = await (await aconn.execute(
        "SELECT payload, caused_by FROM events WHERE kind = 'model.call'")).fetchone()
    percept = await (await aconn.execute(
        "SELECT id FROM events WHERE kind = 'percept'")).fetchone()
    assert call == {"purpose": "pfc.decide", "model": "fake-model",
                    "usage": {"prompt_tokens": 10, "cached_tokens": 4,
                              "completion_tokens": 5, "reasoning_tokens": 0,
                              "total_tokens": 15}}
    assert caused_by == percept[0]


async def test_reasoning_tokens_are_recorded_and_counted_in_the_budget(aconn):
    class ReasoningLLM(FakeLLM):
        def handler(self, request):
            response = super().handler(request)
            payload = json.loads(response.content)
            payload["usage"] = {"prompt_tokens": 100, "completion_tokens": 50,
                                "total_tokens": 150,
                                "completion_tokens_details": {"reasoning_tokens": 40}}
            return httpx.Response(200, json=payload)

    fake = ReasoningLLM([[("act", {"capability": "telegram.say",
                                   "arguments": {"text": "hai"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))

    call = (await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'model.call'")).fetchone())[0]
    assert call["usage"] == {"prompt_tokens": 100, "cached_tokens": 0,
                             "completion_tokens": 50, "reasoning_tokens": 40,
                             "total_tokens": 150}
    key = f"budget:{START.astimezone(JAKARTA).date().isoformat()}"
    assert await read_state(aconn, key) == call["usage"]


async def test_the_budget_spent_raises_theta(aconn):
    runtime = settings(owner_settings={"daily_token_budget": 100})
    key = f"budget:{START.astimezone(JAKARTA).date().isoformat()}"
    source = await append_event(aconn, ts=START, kind="percept", payload={"text": "obrolan"})
    limit = 100.0
    base = (await appraise(source, conn=aconn, now=START, settings=runtime,
                           daily_limit=limit)).theta
    assert base > 0.0

    await write_state(aconn, key, {"total_tokens": 50}, source.id)
    theta = (await appraise(source, conn=aconn, now=START, settings=runtime,
                            daily_limit=limit)).theta
    assert theta - base == pytest.approx(0.15)

    await write_state(aconn, key, {"total_tokens": 250}, source.id)
    theta = (await appraise(source, conn=aconn, now=START, settings=runtime,
                            daily_limit=limit)).theta
    assert theta - base == pytest.approx(0.3)


async def test_budget_exhausted_is_recorded_once_per_day(aconn):
    runtime = settings(owner_settings={"daily_token_budget": 10})
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=runtime)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))
    assert (await _kinds(aconn)).count("budget.exhausted") == 1
    assert await _payload(aconn, "budget.exhausted") == {
        "date": START.astimezone(JAKARTA).date().isoformat(), "used": 15, "budget": 10.0}

    await _feed(kernel, _msg("lagi", message_id=2))
    assert (await _kinds(aconn)).count("budget.exhausted") == 1


# --- middleware hooks --------------------------------------------------------


async def test_inbound_stop_records_guard_stopped_and_skips_cognition(aconn):
    fake = FakeLLM()

    async def block(item, ctx):
        return Stop("not now")

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.hooks.inbound.add("block", block)
    await _feed(kernel, _msg("halo"))

    assert fake.requests == []
    assert await _kinds(aconn) == ["percept", "guard.stopped"]
    assert await _payload(aconn, "guard.stopped") == {"hook": "inbound", "held": False, "reason": "not now"}


async def test_inbound_exception_fails_closed_with_an_error_and_a_stop(aconn):
    async def boom(item, ctx):
        raise RuntimeError("nope")

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), settings=SETTINGS)
    kernel.hooks.inbound.add("boom", boom)
    await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["percept", "error", "guard.stopped"]
    assert await _payload(aconn, "guard.stopped") == {"hook": "inbound", "held": False, "reason": "nope"}


async def test_inbound_replace_swaps_the_event_before_appraisal(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace(replace(item, features={**item.features, "addressed": 1}))

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())
    kernel.hooks.inbound.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo", addressed=False))

    assert len(fake.requests) == 1
    assert "act.out" in await _kinds(aconn)


async def test_context_hook_can_replace_the_pfc_context(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace(replace(item, reason="rewritten by middleware"))

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())
    kernel.hooks.context.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo"))

    assert "<why_awake>rewritten by middleware</why_awake>" in _volatile(fake)


async def test_action_hook_replaces_the_payload_and_continue_passes(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace({**item, "text": "hai (checked)"})

    async def note(item, ctx):
        return Continue()

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)
    kernel.hooks.action.add("note", note)
    kernel.hooks.action.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo"))

    assert [(a.capability, a.place, dict(a.arguments)) for a in body.actions] == [
        ("say", PLACE, {"text": "hai (checked)"})]
    # F.10: the act records the arguments the action hook returned, not the ones asked for.
    assert (await _payload(aconn, "act.out"))["arguments"]["text"] == "hai (checked)"


async def test_action_hook_stop_records_the_stop_and_neither_records_nor_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def block(item, ctx):
        return Stop("privacy")

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)
    kernel.hooks.action.add("block", block)
    await _feed(kernel, _msg("halo"))

    assert body.actions == []
    assert "act.out" not in await _kinds(aconn)
    assert await _payload(aconn, "guard.stopped") == {"hook": "action", "held": False, "reason": "privacy"}


async def test_tool_own_action_hook_runs_only_for_its_tool(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def block(item, ctx):
        return Stop("telegram only")

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)
    kernel.hooks.action.add("other-tool", block, tool="cli")
    await _feed(kernel, _msg("halo"))

    assert len(body.actions) == 1


async def test_inbound_tool_own_hook_skips_events_from_another_tool(aconn):
    async def block(item, ctx):
        return Stop("telegram only")

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), settings=SETTINGS)
    kernel.hooks.inbound.add("other-tool", block, tool="cli")
    await _feed(kernel, _msg("halo"))

    assert "guard.stopped" not in await _kinds(aconn)


async def test_hanging_hook_times_out_as_a_stop_and_the_loop_goes_on(aconn):
    async def hang(item, ctx):
        await asyncio.sleep(10)
        return Continue()

    runtime = settings(constants={"hook_timeout_ms": 10})
    kernel = build(aconn, clock=FakeClock(START), settings=runtime)
    kernel.hooks.inbound.add("hang", hang, tool="telegram")

    await _feed(kernel, _msg("slow"))
    second = await _feed(kernel, replace(_msg("next", message_id=2), body="cli"))

    kinds = await _kinds(aconn)
    assert kinds.count("error") == 1 and kinds.count("guard.stopped") == 1
    timed_out = "inbound hook timed out after 0.01s"
    # 12.17: the error event names the exception, so it is never an empty string.
    assert (await _payload(aconn, "error"))["error"] == f"TimeoutError: {timed_out}"
    assert await _payload(aconn, "guard.stopped") == {"hook": "inbound", "held": False, "reason": timed_out}
    assert second.id is not None  # the next event was recorded and processed
    cur = await aconn.execute("SELECT description FROM events "
                             "WHERE kind = 'percept' ORDER BY id")
    assert [row[0] for row in await cur.fetchall()] == ["slow", "next"]


def test_owner_policy_runs_first_and_last_around_every_hook():
    order: list[str] = []

    async def policy(item, ctx):
        order.append("policy")
        return Continue()

    async def middle(item, ctx):
        order.append("middle")
        return Continue()

    async def ignore(reason):
        pass

    async def loop_hook():
        registry = HookRegistry()
        for pipeline in registry:
            pipeline.add("middle", middle)
            pipeline.set_policy(policy)
            assert await pipeline.run("item", {}, on_stop=ignore, on_error=ignore) == "item"
        return order

    assert asyncio.run(loop_hook()) == ["policy", "middle", "policy"] * 3


def test_hook_slots_order_middleware_and_scope_by_tool():
    async def early(item, ctx):
        return Continue()

    async def mid(item, ctx):
        return Continue()

    async def late(item, ctx):
        return Continue()

    async def own(item, ctx):
        return Continue()

    pipeline = HookPipeline("action")
    pipeline.add("late", late, slot="last")
    pipeline.add("mid", mid)
    pipeline.add("early", early, slot="first")
    pipeline.add("own", own, tool="telegram")

    assert pipeline.middlewares() == (early, mid, late)
    assert pipeline.middlewares("telegram") == (early, mid, own, late)
    assert pipeline.middlewares("cli") == (early, mid, late)


def test_duplicate_middleware_names_fail_at_registration():
    async def middleware(item, ctx):
        return Continue()

    pipeline = HookPipeline("inbound")
    pipeline.add("alias-map", middleware)
    with pytest.raises(ValueError, match="name collision"):
        pipeline.add("alias-map", middleware)
    with pytest.raises(ValueError, match="name collision"):
        pipeline.add("policy", middleware)
    pipeline.set_policy(middleware)
    with pytest.raises(ValueError, match="owner policy already set"):
        pipeline.set_policy(middleware)
    with pytest.raises(ValueError, match="unknown hook"):
        HookPipeline("outbound")


def test_kernel_refuses_settings_whose_signal_weights_do_not_match(conn):
    broken = settings()
    broken["constants"] = {"job_timeout_seconds": 5.0}
    with pytest.raises(ValueError, match="signal weights missing"):
        build(conn, settings=broken)


class IdentityBody(RecordingBody):
    """A body with one capability that changes who the soul is."""

    def capabilities(self):
        return [{"name": "set_identity", "description": "change who the soul is",
                 "communicative": False, "long_running": False,
                 "identity_changing": True, "permission": "identity.set"}]


async def test_identity_changing_capability_is_refused_for_a_non_owner(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.set_identity", "arguments": {}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = IdentityBody()
    kernel.register_adapter(body)
    kernel.roles = open_book("person:other", "identity.set")  # so the guard is what refuses

    await _feed(kernel, replace(_msg("ubah dirimu"), who="person:other"))

    assert body.actions == []
    assert await _payload(aconn, "guard.stopped") == {
        "hook": "action", "held": False,
        "reason": "only the owner may invoke identity_changing capabilities"}


async def test_identity_changing_capability_passes_for_the_owner(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.set_identity", "arguments": {}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = IdentityBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("ubah dirimu"))

    assert len(body.actions) == 1
    # set_identity is not communicative, so no act.confirmed closes it.
    assert await _kinds(aconn) == ["percept", "model.call", "act.out", "act.result"]


async def test_forty_offline_messages_across_three_groups_appraised_once_per_group(aconn):
    """D37: 40 offline messages in 3 groups appraise <=1 per group; old unaddressed stay silent."""
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "siap, aku cek"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS,
                   embedder=FakeEmbedder())
    body = RecordingBody()
    kernel.register_adapter(body)

    g1, g2, g3 = "space:wa:g1@g.us", "space:wa:g2@g.us", "space:wa:g3@g.us"
    pending: list[asyncio.Task] = []
    for idx in range(40):
        if idx < 15:
            place, addressed, text = g1, False, f"obrolan grup 1 #{idx}"
        elif idx < 30:
            place, addressed, text = g2, False, f"obrolan grup 2 #{idx}"
        elif idx < 39:
            place, addressed, text = g3, False, f"obrolan grup 3 #{idx}"
        else:
            place, addressed, text = g3, True, "tolong cek catatan ini dong"
        pending.append(asyncio.create_task(kernel.put(Event(
            id=None, ts=START, kind="percept", who=f"person:u{idx % 5}",
            place=place, body="telegram",
            payload={
                "description": text, "place_kind": "shared",
                "addressed": addressed, "backfill": True,
                "sent_at": (START - timedelta(minutes=30 + idx)).isoformat(),
            }, description=text,
        ))))

    while not all(task.done() for task in pending):
        await kernel.run_until_idle()
        await asyncio.sleep(0)
    await kernel.run_until_idle()

    rows = await (await aconn.execute(
        "SELECT kind, place, payload FROM events ORDER BY id ASC")).fetchall()
    assert sum(1 for k, _, _ in rows if k == "percept") == 40
    decisions = [(k, p, pl) for k, p, pl in rows if k.startswith("decision.")]
    # G1 and G2 each get 1 silent/note decision; G3 wakes via addressed reflex (1 model call).
    for g in (g1, g2, g3):
        assert sum(1 for _, p, _ in decisions if p == g) <= 1
    assert [p for k, p, _ in decisions if p in (g1, g2) and k == "decision.wake"] == []
    assert [a.place for a in body.actions] == [g3]
    assert len(fake.requests) == 1





async def test_kernel_run_flushes_backfill_batch_once_after_settle(aconn):
    """Live Kernel.run() accumulates sequential backfill events and appraises once after settle."""
    cfg = settings()
    cfg["runtime"] = {**cfg.get("runtime", {}), "backfill_settle_seconds": 0.15}
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=cfg,
                   embedder=FakeEmbedder())
    kernel.register_adapter(RecordingBody())
    stop = asyncio.Event()
    runner = asyncio.create_task(kernel.run(stop))
    try:
        for idx in range(5):
            await kernel.put(Event(
                id=None, ts=START, kind="percept", who=f"person:u{idx}",
                place=GROUP, body="telegram",
                payload={
                    "description": f"pesan lama {idx}", "place_kind": "shared",
                    "addressed": False, "backfill": True,
                    "sent_at": (START - timedelta(hours=2)).isoformat(),
                }, description=f"pesan lama {idx}",
            ))
        await asyncio.sleep(0.35)
    finally:
        stop.set()
        await runner

    decisions = await (await aconn.execute(
        "SELECT kind FROM events WHERE kind LIKE 'decision.%' ORDER BY id ASC")).fetchall()
    assert len(decisions) == 1
    assert decisions[0][0] == "decision.silent"
    assert fake.requests == []
