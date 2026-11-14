"""D82 fault-injection and refusal-recovery tests across rules, adapters, and tools."""

from __future__ import annotations

import asyncio
import contextlib
import json
from datetime import datetime, timedelta, timezone
from typing import Any

import httpx
import pytest

from monoagens.__main__ import supervise_adapter
from monoagens.adapters import telegram as telegram_adapter
from monoagens.boot import build
from monoagens.infra.store import unresolved_actions
from monoagens.core.hooks import Continue
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import FakeClock
from monoagens.infra.providers import OpenAICompatibleProvider
from monoagens.infra.store import append_event
from monoagens.middleware import contract, interaction
from monoagens.contract import Action
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 11, 14, 2, 0, tzinfo=timezone.utc)
SETTINGS = settings()

PLACE = "space:tg:10"


class _FakeLLM:
    """Scripted OpenAI-compatible responses recording every request."""

    def __init__(self, responses: list[list[tuple[str, dict[str, Any]]]]) -> None:
        self.responses = responses
        self.requests: list[dict[str, Any]] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        idx = min(len(self.requests) - 1, len(self.responses) - 1)
        calls = [
            {
                "id": f"call_{i}",
                "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)},
            }
            for i, (name, args) in enumerate(self.responses[idx])
        ]
        return httpx.Response(
            200, json={"choices": [{"message": {"role": "assistant", "tool_calls": calls}}]},
        )


def _pfc(fake: _FakeLLM) -> Reasoning:
    return Reasoning(
        OpenAICompatibleProvider(
            "https://llm.test/v1", "test-key", "fake-model",
            retries=0, transport=httpx.MockTransport(fake.handler),
        ),
        settings=SETTINGS,
    )


class _FaultyBody:
    """Adapter supporting all D82 shows that can fail on `show` or `act` or `run`."""

    name = "telegram"
    shows = ("typing", "read_receipts", "edit")
    reads = ("text",)
    gaps: dict[str, str] = {}

    def __init__(self, *, fail_show: set[str] | None = None, fail_act: bool = False) -> None:
        self.fail_show = set(fail_show or ())
        self.fail_act = fail_act
        self.actions: list[Action] = []
        self.shown: list[tuple[str, str, dict[str, Any]]] = []

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        return telegram_adapter.CAPABILITIES

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        if self.fail_act:
            raise RuntimeError("adapter send failed")
        return None

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        self.shown.append((place, kind, dict(data)))
        if kind in self.fail_show:
            raise RuntimeError(f"{kind} transport down")
        return {"message_id": "shown-1"}


class _RaisingTool:
    """Plugin-like tool whose capability raises on call."""

    name = "search"

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        return (
            {
                "name": "query",
                "description": "Search external docs.",
                "schema": {"type": "object", "properties": {"q": {"type": "string"}},
                           "required": ["q"]},
                "communicative": False,
                "long_running": True,
                "identity_changing": False,
            },
        )

    async def act(self, action: Action) -> Any:
        del action
        raise ConnectionError("search service unavailable")


async def test_compose_distinguishes_guard_from_best_effort() -> None:
    """Guards propagate failures; best-effort rules record error and continue."""

    async def guard_rule(item: Any, ctx: Any) -> Any:
        raise RuntimeError("guard failure")

    async def broken_rule(item: Any, ctx: Any) -> Any:
        raise RuntimeError("best-effort down")

    class _FakeKernel:
        def __init__(self) -> None:
            self.errors: list[tuple[Event, str]] = []

        async def _record_error(self, event: Event, message: str) -> None:
            self.errors.append((event, message))

    kernel = _FakeKernel()
    ev = Event(id=1, ts=START, kind="percept", who="person:budi", place="space:tg:1",
               body="telegram", payload={"description": "halo"}, description="halo")

    best_effort_mw = contract.compose(
        contract.mark_rule(broken_rule, name="typing", kind="best-effort"), kernel=kernel)
    assert isinstance(await best_effort_mw(ev, {"conn": None}), Continue)
    assert len(kernel.errors) == 1
    msg = kernel.errors[0][1]
    assert "typing" in msg and "telegram" in msg

    guard_mw = contract.compose(guard_rule, kernel=kernel)
    with pytest.raises(RuntimeError, match="guard failure"):
        await guard_mw(ev, {"conn": None})


async def _feed(kernel: Any, event: Event) -> Any:
    """Put one event and drive the kernel until idle without deadlocking on `ack`."""
    put = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    await kernel.run_until_idle()
    return await put


@pytest.mark.parametrize("failing_show", ["typing", "read_receipts", "edit"])
async def test_show_failure_records_error_and_turn_continues(aconn, failing_show: str) -> None:
    """Every show rule (`typing`, `read_receipts`, `progress`) failing never stops the turn."""
    fake = _FakeLLM([
        [("act", {"capability": "search.query", "arguments": {"q": "status"}})],
        [("act", {"capability": "telegram.say", "arguments": {"text": "halo juga"}})],
    ])
    cfg = settings(bodies={"telegram": {"enabled": True, "show": {
        "typing": "on", "read_receipts": "on", "steps": "all", "progress": "edit",
    }}})
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=cfg)
    body = _FaultyBody(fail_show={failing_show})
    kernel.register_adapter(body)
    tool = _RaisingTool()
    kernel.tools.add_tool(tool, {"query": {"facts": ("reads",), "permission": "search.query"}})

    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner",
        place="space:tg:10", body="telegram", origin_actor="person:owner",
        payload={"description": "halo", "place_kind": "direct",
                 "addressed": True, "data": {"message_id": "m1"}}, description="halo",
    ))

    assert [a.arguments["text"] for a in body.actions] == ["halo juga"]
    errs = [row[0]["error"] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()]
    assert errs, f"expected error event when {failing_show} failed"
    expected_show = "progress" if failing_show == "edit" else failing_show
    assert any("telegram" in err and expected_show in err for err in errs), errs


async def test_thinking_show_failure_records_error_and_continues(aconn) -> None:
    """Failing `thinking` show records an error naming rule, adapter, and `thinking`."""
    cfg = settings(bodies={"telegram": {"enabled": True, "show": {
        "thinking": "all", "steps": "off",
    }}})
    kernel = build(aconn, clock=FakeClock(START), settings=cfg)
    body = _FaultyBody(fail_show={"edit"})
    kernel.register_adapter(body)
    trigger = await append_event(
        aconn, ts=START, kind="percept", who="person:owner", place="space:tg:10",
        body="telegram", origin_actor="person:owner",
        payload={"description": "halo", "place_kind": "direct"},
    )
    await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "aku ragu"})
    follow = await append_event(
        aconn, ts=START, kind="act.out", who="person:owner", place="space:tg:10",
        body="telegram", caused_by=trigger.id, origin_actor="person:owner", payload={},
    )

    outcome = await interaction.narrate(kernel)(follow, {"conn": aconn, "now": START})

    assert isinstance(outcome, Continue)
    errs = [row[0]["error"] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()]
    assert any("narrate.thinking" in e and "telegram" in e for e in errs)


async def test_fault_injection_across_best_effort_progress_and_adapter_and_tool(aconn) -> None:
    """Adapter `run`/`act`, progress show, and plugin tools fault-inject cleanly without stopping."""
    # 1. Faulty tool raises -> honest act.result error -> reasoning still answers;
    #    failing progress show also records error naming rule, adapter, and progress without crash.
    fake = _FakeLLM([
        [("act", {"capability": "search.query", "arguments": {"q": "cuaca"}})],
        [("act", {"capability": "telegram.say", "arguments": {"text": "pencarian gagal"}})],
    ])
    cfg = settings(bodies={"telegram": {"enabled": True, "show": {
        "steps": "all", "progress": "edit",
    }}})
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=cfg)
    body = _FaultyBody(fail_show={"edit"})
    kernel.register_adapter(body)
    kernel.tools.add_tool(_RaisingTool(), {
        "query": {"facts": ("network",), "permission": "search.query"},
    })

    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner",
        place="space:tg:owner", body="telegram", origin_actor="person:owner",
        payload={"description": "cari cuaca", "place_kind": "direct", "addressed": True}, description="cari cuaca",
    ))

    assert [a.arguments["text"] for a in body.actions] == ["pencarian gagal"]
    results = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id")).fetchall()]
    assert any("search.query failed (ConnectionError)" in r.get("reason", "") for r in results)
    errs = [row[0]["error"] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()]
    assert any("narrate.progress" in e and "telegram" in e for e in errs)

    # 2. Adapter `act` raises -> honest act.result error (type name only), no unhandled crash.
    body_fail_act = _FaultyBody(fail_act=True)
    fake2 = _FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "tes"}})]])
    kernel2 = build(aconn, clock=FakeClock(START), pfc=_pfc(fake2), settings=SETTINGS)
    kernel2.register_adapter(body_fail_act)
    await _feed(kernel2, Event(
        id=None, ts=START, kind="percept", who="person:owner",
        place="space:tg:owner2", body="telegram", origin_actor="person:owner",
        payload={"description": "tes", "place_kind": "direct", "addressed": True}, description="tes",
    ))
    res2 = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id")).fetchall()]
    assert any("telegram.say failed (RuntimeError)" in r.get("reason", "") for r in res2)


async def test_tool_failure_never_leaks_exception_message_secrets_to_log(aconn, caplog) -> None:
    """An adapter/tool exception carrying a token never writes the message to Log or logs."""
    secret = "bot123456:FAKE_TOKEN_SECRET"

    class _LeakyBody(_FaultyBody):
        async def act(self, action: Action) -> Any:
            self.actions.append(action)
            raise RuntimeError(f"POST https://api.telegram.org/{secret}/sendMessage refused")

    fake = _FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "halo"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(_LeakyBody())

    with caplog.at_level("ERROR"):
        await _feed(kernel, Event(
            id=None, ts=START, kind="percept", who="person:owner",
            place="space:tg:owner_secret", body="telegram", origin_actor="person:owner",
            payload={"description": "tes", "place_kind": "direct", "addressed": True}, description="tes",
        ))

    res = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id DESC LIMIT 1"
    )).fetchone()
    assert res is not None
    assert res[0]["state"] == "failed"
    assert res[0]["reason"] == "telegram.say failed (RuntimeError)"
    all_rows = await (await aconn.execute("SELECT payload::text FROM events")).fetchall()
    assert all(secret not in row[0] and "bot123456" not in row[0] for row in all_rows)
    assert secret not in caplog.text and "bot123456" not in caplog.text


async def test_fault_injection_adapter_run_restarts_without_stopping_soul(aconn) -> None:
    """An adapter whose `run()` fails records an error naming it and restarts."""
    cfg = settings(runtime={
        "tick_seconds": 30,
        "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2},
        "adapter_retry": {"initial_seconds": 0.01, "max_seconds": 0.02, "factor": 2.0},
    })
    kernel = build(aconn, clock=FakeClock(START), settings=cfg)

    class _RunFailBody(_FaultyBody):
        def __init__(self) -> None:
            super().__init__()
            self.attempts = 0
            self.recovered = asyncio.Event()

        async def run(self, put) -> None:
            self.attempts += 1
            if self.attempts == 1:
                raise RuntimeError("run disconnected")
            self.recovered.set()
            await asyncio.Future()

    adapter = _RunFailBody()
    kernel.register_adapter(adapter)
    stop = asyncio.Event()
    task = asyncio.create_task(supervise_adapter(kernel, adapter, stop))
    try:
        await asyncio.wait_for(adapter.recovered.wait(), timeout=3.0)
        await asyncio.sleep(0.05)
        await kernel.run_until_idle()
    finally:
        stop.set()
        task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await task

    errs = [row[0]["error"] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()]
    assert any("telegram" in e and "run disconnected" in e for e in errs), errs


async def test_a_body_that_may_not_post_refuses_and_reasoning_answers_elsewhere(aconn) -> None:
    """A place the body cannot post in answers `refused`; there is no fact for it (contract §4.4)."""
    class _ReadOnly(_FaultyBody):
        async def act(self, action: Action) -> Any:
            if action.place == "space:tg:ro":
                raise PermissionError("this channel is read-only")
            self.actions.append(action)

    fake = _FakeLLM([
        [("act", {"capability": "telegram.say",
                  "arguments": {"text": "pengumuman", "to": "space:tg:ro"}})],
        [("act", {"capability": "telegram.say",
                  "arguments": {"text": "Channel itu read-only; kubalas di sini ya."}})],
    ])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = _ReadOnly()
    kernel.register_adapter(body)

    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner", place="space:tg:ro",
        body="telegram", origin_actor="person:owner",
        payload={"description": "broadcast"}, description="broadcast"))
    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner", place="space:tg:dm",
        body="telegram", origin_actor="person:owner",
        payload={"description": "kirim ke channel News dong"},
        features={"addressed": 1}, description="kirim ke channel News dong"))

    receipts = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id")).fetchall()]
    assert receipts and receipts[0]["state"] == "refused"
    assert "read-only" in str(receipts[0]["reason"])
    assert await unresolved_actions(aconn) == []

async def test_repeated_refusal_is_bounded_by_brake_and_notifies_requester(aconn) -> None:
    """Repeated refusals of the same act stop at `max_task_steps` and tell the requester."""
    fake = _FakeLLM([
        [("act", {"capability": "telegram.say",
                  "arguments": {"text": "coba terus", "to": "space:tg:ro"}})],
    ])
    cfg = settings(owner_settings={"max_task_steps": 2, "max_wakes_per_hour": 20})
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=cfg)

    class _ReadOnly(_FaultyBody):
        async def act(self, action: Action) -> Any:
            if action.place == "space:tg:ro":
                raise PermissionError("this channel is read-only")
            self.actions.append(action)

    body = _ReadOnly()
    kernel.register_adapter(body)

    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner", place="space:tg:ro",
        body="telegram", origin_actor="person:owner",
        payload={"description": "broadcast"}, description="broadcast"))
    await _feed(kernel, Event(
        id=None, ts=START, kind="percept", who="person:owner", place="space:tg:dm",
        body="telegram", origin_actor="person:owner",
        payload={"description": "kirim ke channel News terus",
                 "addressed": True}, description="kirim ke channel News terus",
    ))

    # Stops at max_task_steps=2: the refusals cost attempts, the brake then stops the chain.
    assert len(fake.requests) == 3
    assert await unresolved_actions(aconn) == []
    stopped = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'guard.stopped' ORDER BY id")).fetchall()]
    assert any("max_task_steps" in str(s.get("reason") or "") for s in stopped)
    held = await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'brake.held'")).fetchone()
    assert held == (1,)
    assert any("max_task_steps" in str(row[0]) for row in await (await aconn.execute(
        "SELECT payload::text FROM events")).fetchall())


async def test_f2_hung_tool_times_out_under_watchdog_and_records_honest_act_result(aconn) -> None:
    """F.2: a hung tool times out under job_timeout_seconds and yields an honest act.result."""
    class _HangingTool:
        name = "slow"

        def capabilities(self) -> tuple[dict[str, Any], ...]:
            return ({
                "name": "hang",
                "description": "Hang forever.",
                "schema": {"type": "object", "properties": {}, "required": []},
                "communicative": False, "long_running": False, "identity_changing": False,
            },)

        async def act(self, action: Action) -> Any:
            del action
            await asyncio.sleep(5)
            return {"ok": True}

    fake = _FakeLLM([
        [("act", {"capability": "slow.hang", "arguments": {}})],
        [("act", {"capability": "telegram.say", "arguments": {"text": "alatnya timeout"}})],
    ])
    cfg = settings(constants={"job_timeout_seconds": 0.05})
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=cfg)
    body = _FaultyBody()
    kernel.register_adapter(body)
    kernel.tools.add_tool(_HangingTool(), {"hang": {"facts": ("reads",), "permission": "slow.hang"}})

    await asyncio.wait_for(
        _feed(kernel, Event(
            id=None, ts=START, kind="percept", who="person:owner",
            place="space:tg:10", body="telegram", origin_actor="person:owner",
            payload={"description": "coba", "place_kind": "direct", "addressed": True}, description="coba",
        )),
        timeout=2.0,
    )

    assert [a.arguments["text"] for a in body.actions] == ["alatnya timeout"]
    results = [row[0] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id")).fetchall()]
    assert any(r.get("capability") == "slow.hang" and "timed out" in r.get("reason", "")
               for r in results), results


async def test_f3_failing_call_in_apply_calls_lets_siblings_run_and_counts_task_step(
        aconn, monkeypatch) -> None:
    """F.3: one failing call inside apply_calls becomes act.result error, siblings run, task counts step."""
    from monoagens.core import jobs, threads
    from monoagens.infra.store import read_state

    class _RecordingTool:
        name = "work"

        def __init__(self) -> None:
            self.actions: list[Action] = []

        def capabilities(self) -> tuple[dict[str, Any], ...]:
            return (
                {"name": "first", "description": "First step.",
                 "schema": {"type": "object", "properties": {}, "required": []},
                 "communicative": False, "long_running": True, "identity_changing": False},
                {"name": "second", "description": "Second step.",
                 "schema": {"type": "object", "properties": {}, "required": []},
                 "communicative": False, "long_running": False, "identity_changing": False},
            )

        async def act(self, action: Action) -> Any:
            self.actions.append(action)
            return {"ok": True, "ran": action.capability}

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(_FakeLLM([])), settings=SETTINGS)
    tool = _RecordingTool()
    kernel.tools.add_tool(tool, {
        "first": {"facts": ("reads",), "permission": "work.first"},
        "second": {"facts": ("reads",), "permission": "work.second"},
    })
    root = await append_event(
        aconn, ts=START, kind="percept", who="person:owner", place="space:tg:10",
        body="telegram", origin_actor="person:owner",
        payload={"description": "kerjakan dua hal", "place_kind": "direct"},
    )
    task = await threads.open_for(kernel, root)
    assert task is not None

    real_tools_act = kernel.tools.act

    async def boom_tools_act(action: Any) -> Any:
        if action.capability == "work.first":
            raise RuntimeError("broken tool dispatcher on first call")
        return await real_tools_act(action)

    monkeypatch.setattr(kernel.tools, "act", boom_tools_act)
    acts = await jobs.apply_calls(kernel, root, [{"calls": [
        {"name": "act", "arguments": {"capability": "work.first", "arguments": {}}},
        {"name": "act", "arguments": {"capability": "work.second", "arguments": {}}},
    ]}], jobs.JobRef(f"pfc:{task['place']}", int(root.id)))
    fake_job = jobs.Job("pfc", f"pfc:{task['place']}", root, task["place"])
    await threads.after_step(kernel, fake_job, root, acts)
    await kernel.run_until_idle()

    assert [a.capability for a in tool.actions] == ["second"]
    rows = await (await aconn.execute(
        "SELECT id, caused_by, payload FROM events WHERE kind IN ('act.out', 'act.result') "
        "ORDER BY id"
    )).fetchall()
    first_out = next(eid for eid, _, p in rows
                     if p.get("capability") == "work.first" and "arguments" in p)
    first_res, first_cb = next((p, cb) for _, cb, p in rows
                               if p.get("capability") == "work.first" and "state" in p)
    assert first_cb == first_out
    assert first_res["state"] == "failed" and "RuntimeError" in first_res["reason"]
    saved_task = await read_state(aconn, f"task:{task['place']}")
    assert saved_task is not None and saved_task["steps"] == 2


async def test_f5_kernel_survives_backlog_flush_recover_and_expect_wake_errors(
        aconn, monkeypatch) -> None:
    """F.5: exceptions in backlog flush, recover(), and one fired expectation never kill the rest."""
    from monoagens.core import expectations

    # 1. Backlog flush exception in place A never blocks place B.
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(_FakeLLM([])), settings=SETTINGS)
    ev_a = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                              place="space:tg:a", body="telegram",
                              payload={"description": "a", "backfill": True})
    ev_b = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                              place="space:tg:b", body="telegram",
                              payload={"description": "b", "backfill": True})
    kernel._queue_backfill(ev_a)
    kernel._queue_backfill(ev_b)
    seen_places: list[str] = []

    async def flaky_wake(event: Event) -> None:
        seen_places.append(str(event.place))
        if event.place == "space:tg:a":
            raise RuntimeError("backlog a boom")

    monkeypatch.setattr(kernel, "_appraise_and_wake", flaky_wake)
    await kernel._flush_backlogs()
    assert seen_places == ["space:tg:a", "space:tg:b"]

    # 2. One fired expectation failing never drops the remaining fired expectations.
    fired_a = {"id": 1, "fired_event": ev_a.id, "who": "person:owner", "place": "space:tg:a",
               "source_body": "telegram", "about": "one", "intent": "one"}
    fired_b = {"id": 2, "fired_event": ev_b.id, "who": "person:owner", "place": "space:tg:b",
               "source_body": "telegram", "about": "two", "intent": "two"}
    woken_ids: list[int] = []

    async def flaky_expect_wake(item: dict[str, Any]) -> None:
        woken_ids.append(int(item["id"]))
        if item["id"] == 1:
            raise RuntimeError("expect 1 boom")

    async def fake_due(*args: Any, **kwargs: Any) -> list[dict[str, Any]]:
        return [fired_a, fired_b]

    monkeypatch.setattr(kernel, "_expect_wake", flaky_expect_wake)
    monkeypatch.setattr(expectations, "due", fake_due)
    tick = await append_event(aconn, ts=START, kind="tick")
    await kernel._wake(tick)
    assert woken_ids == [1, 2]

    # 3. recover() survives failure in sends.resume / one expectation and continues remaining items.
    woken_ids.clear()

    async def fake_recovery(*args: Any, **kwargs: Any) -> list[dict[str, Any]]:
        return [fired_a, fired_b]

    monkeypatch.setattr(expectations, "recovery", fake_recovery)
    async def boom(_now):
        raise RuntimeError("resume boom")

    monkeypatch.setattr(kernel, "_resume_actions", boom)
    await kernel.recover()
    assert woken_ids == [1, 2]

    errs = [row[0]["error"] for row in await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'error'")).fetchall()]
    assert any("backlog a boom" in e for e in errs)
    assert any("expect 1 boom" in e for e in errs)
    assert any("resume boom" in e for e in errs)


async def test_f5_one_malformed_expectation_loses_no_other_fired_one(aconn) -> None:
    """F.5: `expectations.due` reports one row's failure and still fires the others."""
    from monoagens.core import expectations

    source = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=PLACE, body="telegram",
                                payload={"description": "source"}, description="source")
    async with aconn.transaction():
        await aconn.execute(
            "INSERT INTO expectations (kind, who, place, about, cue_event, condition, "
            "condition_text, intent, weight, policy, status, next_check_at, source_event, "
            "derived_from, origin_actor, expires_at) VALUES "
            "('promise', %s, %s, 'broken row', %s, %s, NULL, NULL, 1.0, %s, 'active', NULL, "
            "%s, '{}'::bigint[], %s, %s), "
            "('promise', %s, %s, 'sound row', %s, %s, NULL, NULL, 1.0, %s, 'active', NULL, "
            "%s, '{}'::bigint[], %s, %s)",
            ("person:owner", PLACE, '{"kind": "percept"}',
             '{"event_seen": {"kind": "percept"}, "within": "not a duration"}',
             '{"once": true}', source.id, "person:owner", START + timedelta(days=1),
             "person:owner", PLACE, '{"kind": "percept"}', '{"none": true}',
             '{"once": true}', source.id, "person:owner", START + timedelta(days=1)))
    seen = await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                              body="telegram", payload={"description": "halo"},
                              description="halo")

    failures: list[BaseException] = []

    async def collect(exc: BaseException) -> None:
        failures.append(exc)

    wakes = await expectations.due(aconn, seen, now=START, settings=SETTINGS,
                                   on_error=collect)

    assert [type(exc) for exc in failures] == [ValueError]
    assert [item["about"] for item in wakes] == ["sound row"]
    fired = await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'expectation.fired'")).fetchone()
    assert fired == (1,)
