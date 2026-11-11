"""Tests for the cognition layer: Situation, signals, appraisal, hooks, and tokens."""

from __future__ import annotations

import asyncio
import json
from dataclasses import replace
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

import httpx
import pytest

from monoagens import memory
from monoagens.clock import FakeClock
from monoagens.cognition import (SIGNALS, Appraisal, Situation, appraise,
                                 assemble_situation, seeded_rng, validate_signals)
from monoagens.kernel import Kernel
from monoagens.memory import FakeEmbedder
from monoagens.middleware import Continue, HookPipeline, HookRegistry, Replace, Stop
from monoagens.pfc import PFC
from monoagens.store import (append_event, insert_event_vector, read_event_vector,
                             read_state, write_state)
from monoagens.bodies.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event

SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
    "memory": {"retrieve_k": 5},
    "cognition": {
        "seed": 1,
        "weights": {"name_mentioned": 0.5, "info_gap": 0.35, "feeling_of_knowing": 0.35,
                    "relevance": 0.2, "social_cost": -0.4},
    },
    "expectations": {"default_expires_days": 30, "max_active": 200},
}
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

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _pfc(fake: FakeLLM) -> PFC:
    return PFC("https://llm.test/v1", "test-key", "fake-model", soul=SOUL,
               transport=httpx.MockTransport(fake.handler))


def _msg(text: str = "halo", *, message_id: int = 1, place: str = PLACE,
         place_kind: str = "direct", addressed: bool = True) -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=place, body="telegram",
                 payload={"description": text, "addressed": addressed,
                          "place_kind": place_kind, "data": {"message_id": message_id}})


def _situation(**overrides) -> Situation:
    event = overrides.pop("event", _msg())
    return Situation(event=event, **overrides)


async def _feed(kernel: Kernel, event: Event) -> Event:
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _kinds(conn) -> list[str]:
    return [row[0] for row in await (await conn.execute(
        "SELECT kind FROM events ORDER BY id")).fetchall()]


async def _payload(conn, kind: str) -> dict:
    cur = await conn.execute("SELECT payload FROM events WHERE kind = %s ORDER BY id", (kind,))
    return (await cur.fetchone())[0]


def _volatile(fake: FakeLLM) -> str:
    return fake.requests[-1]["messages"][2]["content"]


# --- signals and validation --------------------------------------------------


def test_the_five_initial_signals_are_registered():
    assert set(SIGNALS) == {"name_mentioned", "info_gap", "feeling_of_knowing", "relevance",
                            "social_cost"}


def test_validate_signals_accepts_the_configured_weights_and_fails_otherwise():
    validate_signals({name: 1.0 for name in SIGNALS})
    with pytest.raises(ValueError, match="missing"):
        validate_signals({"name_mentioned": 1.0})
    with pytest.raises(ValueError, match="unknown"):
        validate_signals({**{name: 0.0 for name in SIGNALS}, "bogus": 1.0})


def test_signals_are_pure_bounded_functions_of_a_situation():
    situation = _situation(vector=[0.0] * 384, recent=())
    for signal in SIGNALS.values():
        value = signal(situation)
        assert isinstance(value, float)
        assert 0.0 <= value <= 1.0


# --- seeded randomness -------------------------------------------------------


def test_seeded_rng_is_reproducible_and_depends_on_the_event():
    assert seeded_rng(1, 7).random() == seeded_rng(1, 7).random()
    assert seeded_rng(1, 7).random() != seeded_rng(1, 8).random()
    assert seeded_rng(1, None).random() == seeded_rng(1, 0).random()


# --- appraisal ---------------------------------------------------------------


def test_appraisal_payload_carries_every_number_and_the_signals():
    appraisal = Appraisal("note", "why", "reflex", {"a": 0.5}, 0.4, 0.3, 0.6, 0.9)
    assert appraisal.payload() == {
        "decision": "note", "reason": "why", "reflex": "reflex", "signals": {"a": 0.5},
        "s": 0.4, "theta": 0.3, "p": 0.6, "u": 0.9,
    }


async def test_addressed_message_is_a_wake_reflex():
    event = _msg("mona, halo", addressed=True)
    appraisal = await appraise(event, soul=SOUL)
    assert (appraisal.decision, appraisal.reflex, appraisal.reason) == (
        "wake", "addressed", "addressed message")
    assert (appraisal.s, appraisal.theta, appraisal.p) == (1.0, 0.0, 1.0)
    assert 0.0 <= appraisal.u < 1.0
    assert set(appraisal.signals) == set(SIGNALS)
    assert all(value == 0.0 for value in appraisal.signals.values())


async def test_fired_expectation_is_a_wake_reflex():
    event = Event(None, START, "expectation.fired", who=WHO, place=PLACE,
                  payload={"expectation_id": 1})
    appraisal = await appraise(event, soul=SOUL)
    assert (appraisal.decision, appraisal.reflex, appraisal.p) == (
        "wake", "pfc_expectation", 1.0)


async def test_an_ordinary_message_is_silent_and_reproducible():
    event = Event(42, START, "percept", who=WHO, place=PLACE, payload={"text": "obrolan"})
    first = await appraise(event, soul=SOUL)
    second = await appraise(event, soul=SOUL)
    assert (first.decision, first.reflex, first.reason) == (
        "silent", None, "gate score below silent threshold")
    assert (first.s, first.theta, first.p) == (0.0, 0.0, 0.0)
    assert first.u == second.u == seeded_rng(1, 42).random()


# --- situation and encode once ----------------------------------------------


async def test_assemble_situation_holds_vector_recent_and_state(aconn):
    earlier = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                 body="telegram", payload={"text": "kopi"})
    await insert_event_vector(aconn, earlier.id, [0.5] * 384)
    await write_state(aconn, "cognition:weights", {"name_mentioned": 1.0}, earlier.id)
    await write_state(aconn, f"habit:{PLACE}:{WHO}", {"h": 1}, earlier.id)
    current = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                 body="telegram", payload={"text": "halo"})

    situation = await assemble_situation(aconn, current, now=START, soul=SOUL)
    assert situation.event.id == current.id
    assert situation.vector is None
    assert [seen.id for seen in situation.recent] == [earlier.id, current.id]
    assert situation.state["cognition:weights"] == {"name_mentioned": 1.0}
    assert situation.state[f"habit:{PLACE}:{WHO}"] == {"h": 1}
    assert f"budget:{START.astimezone(JAKARTA).date().isoformat()}" not in situation.state

    older = await assemble_situation(aconn, earlier, now=START, soul=SOUL)
    assert older.vector is not None and len(older.vector) == 384


async def test_kernel_encodes_a_message_once_and_retrieval_reuses_the_vector(aconn):
    embedder = CountingEmbedder()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=None, soul=SOUL, embedder=embedder)

    stored = await _feed(kernel, _msg("halo"))

    assert stored.id is not None
    assert embedder.calls == 1
    vector = await read_event_vector(aconn, stored.id)
    assert vector is not None and len(vector) == 384

    await memory.retrieve(aconn, embedder, "halo", now=START, event_id=stored.id)
    assert embedder.calls == 1
    await memory.retrieve(aconn, embedder, "halo", now=START)
    assert embedder.calls == 2


async def test_kernel_encodes_the_reply_once_too(aconn):
    embedder = CountingEmbedder()
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL,
                    embedder=embedder)
    kernel.register_body(RecordingBody())

    await _feed(kernel, _msg("halo"))

    assert embedder.calls == 2  # the incoming message and the reply; retrieval reused the first
    rows = await (await aconn.execute(
        "SELECT event_id FROM event_vectors ORDER BY event_id")).fetchall()
    assert len(rows) == 2


# --- kernel decisions and token usage ---------------------------------------


async def test_silent_decision_event_carries_the_full_appraisal(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    await _feed(kernel, _msg("obrolan biasa", place=GROUP, place_kind="group",
                             addressed=False))

    assert await _kinds(aconn) == ["percept", "decision.silent"]
    payload = await _payload(aconn, "decision.silent")
    assert set(payload) == {"decision", "reason", "reflex", "signals", "s", "theta", "p", "u"}
    assert (payload["decision"], payload["reason"], payload["reflex"]) == (
        "silent", "gate score below silent threshold", None)
    assert (payload["s"], payload["theta"], payload["p"]) == (0.0, 0.0, 0.0)
    stored = await (await aconn.execute("SELECT id FROM events WHERE kind = 'percept'")).fetchone()
    assert payload["u"] == seeded_rng(1, stored[0]).random()
    assert fake.requests == []


async def test_llm_token_usage_lands_in_the_daily_budget_key(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])
    pfc = _pfc(fake)
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=pfc, soul=SOUL)
    kernel.register_body(RecordingBody())

    await _feed(kernel, _msg("halo"))

    assert pfc.last_usage == {"prompt_tokens": 10, "cached_tokens": 4,
                              "completion_tokens": 5, "total_tokens": 15}
    key = f"budget:{START.astimezone(JAKARTA).date().isoformat()}"
    assert await read_state(aconn, key) == {"prompt_tokens": 10, "cached_tokens": 4,
                                            "completion_tokens": 5, "total_tokens": 15}


# --- middleware hooks --------------------------------------------------------


async def test_inbound_stop_records_guard_stopped_and_skips_cognition(aconn):
    fake = FakeLLM()

    async def block(item, ctx):
        return Stop("not now")

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    kernel.hooks.inbound.add("block", block)
    await _feed(kernel, _msg("halo"))

    assert fake.requests == []
    assert await _kinds(aconn) == ["percept", "guard.stopped"]
    assert await _payload(aconn, "guard.stopped") == {"hook": "inbound", "reason": "not now"}


async def test_inbound_exception_fails_closed_with_an_error_and_a_stop(aconn):
    async def boom(item, ctx):
        raise RuntimeError("nope")

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), soul=SOUL)
    kernel.hooks.inbound.add("boom", boom)
    await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["percept", "error", "guard.stopped"]
    assert await _payload(aconn, "guard.stopped") == {"hook": "inbound", "reason": "nope"}


async def test_inbound_replace_swaps_the_event_before_appraisal(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace(replace(item, payload={**item.payload, "addressed": True}))

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    kernel.register_body(RecordingBody())
    kernel.hooks.inbound.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo", addressed=False))

    assert len(fake.requests) == 1
    assert "act.out" in await _kinds(aconn)


async def test_context_hook_can_replace_the_pfc_context(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace(replace(item, reason="rewritten by middleware"))

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    kernel.register_body(RecordingBody())
    kernel.hooks.context.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo"))

    assert "<why_awake>rewritten by middleware</why_awake>" in _volatile(fake)


async def test_action_hook_replaces_the_payload_and_continue_passes(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def rewrite(item, ctx):
        return Replace({**item, "text": "hai (checked)"})

    async def note(item, ctx):
        return Continue()

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    body = RecordingBody()
    kernel.register_body(body)
    kernel.hooks.action.add("note", note)
    kernel.hooks.action.add("rewrite", rewrite)
    await _feed(kernel, _msg("halo"))

    assert body.actions == [Action(kind="say", place=PLACE,
                                   payload={"text": "hai (checked)"}, body="telegram")]
    assert (await _payload(aconn, "act.out"))["arguments"]["text"] == "hai"


async def test_action_hook_stop_keeps_the_logged_message_but_never_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def block(item, ctx):
        return Stop("privacy")

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    body = RecordingBody()
    kernel.register_body(body)
    kernel.hooks.action.add("block", block)
    await _feed(kernel, _msg("halo"))

    assert body.actions == []
    assert (await _payload(aconn, "act.out"))["arguments"]["text"] == "hai"
    assert await _payload(aconn, "guard.stopped") == {"hook": "action", "reason": "privacy"}


async def test_organ_own_action_hook_runs_only_for_its_organ(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "hai"}})]])

    async def block(item, ctx):
        return Stop("telegram only")

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), soul=SOUL)
    body = RecordingBody()
    kernel.register_body(body)
    kernel.hooks.action.add("other-organ", block, organ="cli")
    await _feed(kernel, _msg("halo"))

    assert len(body.actions) == 1


async def test_inbound_organ_own_hook_skips_events_from_another_organ(aconn):
    async def block(item, ctx):
        return Stop("telegram only")

    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), soul=SOUL)
    kernel.hooks.inbound.add("other-organ", block, organ="cli")
    await _feed(kernel, _msg("halo"))

    assert "guard.stopped" not in await _kinds(aconn)


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


def test_hook_slots_order_middleware_and_scope_by_organ():
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
    pipeline.add("own", own, organ="telegram")

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


def test_kernel_refuses_a_soul_whose_signal_weights_do_not_match(conn):
    with pytest.raises(ValueError, match="cognition.weights mismatch"):
        Kernel(conn, soul={**SOUL, "cognition": {"weights": {"bogus": 1.0}}})
    with pytest.raises(ValueError, match="cognition.weights mismatch"):
        Kernel(conn, soul={key: value for key, value in SOUL.items() if key != "cognition"})
