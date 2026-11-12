"""Tests for the free-text wake gate: cosine filter, classifier, learned bands."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx

from monoagens import condition, expect
from monoagens.clock import FakeClock
from monoagens.embedding import FakeEmbedder
from monoagens.kernel import Kernel
from monoagens.pfc import PFC
from monoagens.store import append_event, read_expectations, read_state
from tests.support import settings

SETTINGS = settings()
START = datetime(2026, 11, 10, 23, 0, tzinfo=timezone.utc)  # 06:00 in Jakarta
PLACE = "space:tg:1001"
GROUP = "space:tg:-1004"
WHO = "person:owner"
OTHER = "person:tg:7001"


class ScriptedLLM:
    """Scripted endpoint: one general verdict for classify, recorded answers else."""

    def __init__(self, decide: list[list[tuple[str, dict]]] | None = None,
                 verdict: str = "yes") -> None:
        """Start with the PFC's answers and the classifier's verdict."""
        self.decide = list(decide or [])
        self.verdict, self.requests = verdict, []

    def handler(self, request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        self.requests.append(body)
        names = {tool["function"]["name"] for tool in body.get("tools") or []}
        calls = [("classify", {"verdict": self.verdict})] if "classify" in names else (
            self.decide.pop(0) if self.decide else [])
        tool_calls = [{"id": f"call_{i}", "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments)}}
                      for i, (name, arguments) in enumerate(calls)]
        return httpx.Response(200, json={
            "choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}})


def _pfc(fake: ScriptedLLM) -> PFC:
    return PFC("https://llm.test/v1", "test-key", "fake-model", settings=SETTINGS,
               transport=httpx.MockTransport(fake.handler))


class RecordingBody:
    """A body that records every action dispatched to it."""

    name = "telegram"

    def __init__(self) -> None:
        """Start with an empty action list."""
        self.actions = []

    def capabilities(self):
        from monoagens.bodies.telegram import CAPABILITIES

        return CAPABILITIES

    async def act(self, action) -> None:
        """Record one action."""
        self.actions.append(action)


def _kernel(conn, fake: ScriptedLLM | None) -> Kernel:
    clock = FakeClock(START)
    pfc = _pfc(fake) if fake is not None else None
    kernel = Kernel(conn, clock=clock, pfc=pfc, settings=SETTINGS, embedder=FakeEmbedder())
    kernel.register_body(RecordingBody())
    return kernel


def _percept(text: str, *, message_id: int = 1, who: str = OTHER,
             place: str = GROUP) -> "object":
    from monoagens.types import Event

    return Event(id=None, ts=START, kind="percept", who=who, place=place, body="telegram",
                 payload={"description": text, "addressed": False, "place_kind": "shared",
                          "data": {"message_id": message_id}})


async def _feed(kernel: Kernel, event) -> None:
    put = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put, 1.0)


async def _promise(conn, *, text: str, place: str = GROUP, expires: datetime | None = None,
                    source_place: str = PLACE) -> int:
    """Create one free-text promise through the module API."""
    source = await append_event(conn, ts=START, kind="percept", who=WHO, place=source_place,
                                body="telegram", payload={"description": "minta dikabari"})
    data = {"about": f"kabari kalau {text}", "kind": "promise",
            "cue_event": {"kind": "percept", "place": place}, "expect": {"text": text},
            "intent": "beri tahu owner", "weight": 0.8, "policy": {"once": True}}
    if expires is not None:
        data["expires_at"] = expires.isoformat()
    created = await expect.create(conn, data, now=START, settings=SETTINGS, source=source)
    return int(created.payload["expectation_id"])


async def _kinds(conn, name: str) -> list[dict]:
    cur = await conn.execute("SELECT payload FROM events WHERE kind = %s ORDER BY id", (name,))
    return [row[0] for row in await cur.fetchall()]


async def _status(conn, exp_id: int) -> str:
    rows = await read_expectations(conn, "ex.id = %s", (exp_id,))
    return rows[0]["status"]


async def test_a_candidate_below_lo_is_ignored_without_a_model_call(aconn):
    exp_id = await _promise(aconn, text="server down")
    fake = ScriptedLLM()
    kernel = _kernel(aconn, fake)

    await _feed(kernel, _percept("deploy staging aman"))

    calls = await _kinds(aconn, "model.call")
    assert [call for call in calls if call.get("purpose") == "classifier"] == []
    assert await _kinds(aconn, "classifier.verdict") == []
    assert await _status(aconn, exp_id) == "active"


async def test_one_candidate_makes_one_recorded_classifier_call_and_its_verdict(aconn):
    exp_id = await _promise(aconn, text="server down")
    fake = ScriptedLLM(decide=[[("act", {"capability": "telegram.say",
                                         "arguments": {"text": "Server down, owner."}})]])

    await _feed(_kernel(aconn, fake), _percept("server down lagi nih guys"))

    calls = await _kinds(aconn, "model.call")
    classified = [call for call in calls if call.get("purpose") == "classifier"]
    assert len(classified) == 1
    assert classified[0]["usage"]["total_tokens"] == 15
    budget = await read_state(aconn, f"budget:{START.astimezone(expect.tz(SETTINGS)).date()}")
    assert budget["total_tokens"] == 15 * len(calls)  # the classifier call is counted
    verdict = await _kinds(aconn, "classifier.verdict")
    assert [item["verdict"] for item in verdict] == ["yes"]
    assert 0.6 < verdict[0]["m"] < 0.7
    assert await _status(aconn, exp_id) == "satisfied"
    assert len(await _kinds(aconn, "expectation.fired")) == 1
    assert len(await _kinds(aconn, "act.out")) == 1


async def test_a_no_verdict_leaves_the_promise_active(aconn):
    exp_id = await _promise(aconn, text="dokumen sudah selesai")
    fake = ScriptedLLM(verdict="no")

    await _feed(_kernel(aconn, fake), _percept("dokumen sudah selesai ya?"))

    verdict = await _kinds(aconn, "classifier.verdict")
    assert [item["verdict"] for item in verdict] == ["no"]
    assert verdict[0]["m"] > 0.35
    assert await _kinds(aconn, "expectation.fired") == []
    assert await _kinds(aconn, "act.out") == []
    assert await _status(aconn, exp_id) == "active"


async def test_a_clearly_yes_event_fires_without_a_classifier_call(aconn):
    runtime = settings(calibration={"wake_gate_lo": 0.35, "wake_gate_hi": 0.5})
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                body="telegram", payload={"description": "minta dikabari"})
    created = await expect.create(aconn, {
        "about": "kabari kalau server down", "kind": "promise",
        "cue_event": {"kind": "percept"}, "expect": {"text": "server down"},
        "policy": {"once": True}}, now=START, settings=runtime, source=source)
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(ScriptedLLM()), settings=runtime,
                    embedder=FakeEmbedder())

    await _feed(kernel, _percept("server down lagi nih guys"))

    assert [call for call in await _kinds(aconn, "model.call")
            if call.get("purpose") == "classifier"] == []
    assert await _kinds(aconn, "classifier.verdict") == []
    assert len(await _kinds(aconn, "expectation.fired")) == 1
    assert await _status(aconn, int(created.payload["expectation_id"])) == "satisfied"


async def test_a_condition_already_met_resolves_without_waking_anyone(aconn):
    done = await append_event(aconn, ts=START - timedelta(minutes=5), kind="act.out",
                              who=WHO, place=GROUP, body="dev",
                              payload={"capability": "dev.run", "arguments": {"command": "x"}})
    source = await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE,
                                body="telegram", payload={"description": "kabari kalau selesai"})
    created = await expect.create(aconn, {
        "about": "kabari kalau pekerjaan selesai", "kind": "promise",
        "cue_event": {"kind": "act.result"},
        "expect": {"event_seen": {"kind": "act.out", "place": GROUP}, "within": "1h"},
        "policy": {"once": True}}, now=START, settings=SETTINGS, source=source)
    kernel = _kernel(aconn, ScriptedLLM())
    result = await append_event(aconn, ts=START, kind="act.result", who=WHO, place=GROUP,
                                body="dev", payload={"result": {"ok": True}},
                                caused_by=done.id)

    await _feed(kernel, result)

    assert await _kinds(aconn, "expectation.fired") == []
    assert [event for event in await _kinds(aconn, "expectation.resolved")
            if event["status"] == "satisfied"]
    assert await _status(aconn, int(created.payload["expectation_id"])) == "satisfied"


async def test_a_stale_expectation_drops_on_the_wake_gate(aconn):
    exp_id = await _promise(aconn, text="server down", expires=START - timedelta(minutes=1))
    kernel = _kernel(aconn, ScriptedLLM())

    clock = kernel.clock
    await clock.tick(kernel.put)
    await kernel.step()
    await kernel.run_until_idle()

    assert await _status(aconn, exp_id) == "dropped"
    assert await _kinds(aconn, "expectation.fired") == []


async def _learn(conn, times: int, m: float, yes: bool):
    kernel = _kernel(conn, None)
    for index in range(times):
        trigger = await append_event(conn, ts=START, kind="classifier.verdict",
                                     place=GROUP, payload={"verdict": "yes" if yes else "no"})
        await condition.learn(kernel, trigger, m, yes)


async def test_lo_moves_toward_the_lowest_yes_within_bounds_and_gray_width(aconn):
    await _learn(aconn, 50, 0.30, True)
    stored = await read_state(aconn, condition.BANDS)
    assert stored["lo"] == 0.33  # one bounded step toward 0.30 - 0.05
    assert stored["hi"] == 1.0  # no verdict at or above the gray band's top
    assert stored["hi"] - stored["lo"] >= 0.2
    assert stored["moved"] <= 0.04
    changes = await _kinds(aconn, "disposition.updated")
    assert changes and changes[0]["key"] == condition.BANDS
    assert changes[0]["delta"][0] <= 0


async def test_the_clearly_yes_band_opens_after_fifty_clean_verdicts(aconn):
    await _learn(aconn, 50, 0.9, True)
    stored = await read_state(aconn, condition.BANDS)
    assert stored["hi"] == 0.9
    await _learn(aconn, 1, 0.95, False)
    stored = await read_state(aconn, condition.BANDS)
    assert stored["hi"] == 1.0
