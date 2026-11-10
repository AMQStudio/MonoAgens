"""FlowStore: one row per flow, a fenced write, an exclusive claim, expiry, and the reaper."""

import time

import pytest
from sqlalchemy import text
from support import database

from monoagens.engine.flows.store import FlowStore
from monoagens.engine.jobs.lease import Superseded


class FakeLease:
    """A bound lease that never matches the database row (a deposed instance)."""

    def __init__(self, generation: int = 3, instance_id: str = "deposed") -> None:
        self.generation = generation
        self.instance_id = instance_id


def store() -> FlowStore:
    return FlowStore(database())


def create(store: FlowStore, **overrides) -> int:
    values = {
        "owner": "core",
        "name": "core/demo",
        "requester": "p1",
        "thread_id": "chat",
        "gateway": "fake",
        "step": "start",
        "state": {"seen": True},
        "now": 1000.0,
    }
    values.update(overrides)
    return store.create(**values)


def test_create_and_get_round_trip() -> None:
    flows = store()
    flow_id = create(flows)

    row = flows.get(flow_id)
    assert row is not None
    assert row["owner"] == "core"
    assert row["name"] == "core/demo"
    assert row["requester"] == "p1"
    assert row["thread_id"] == "chat"
    assert row["gateway"] == "fake"
    assert row["step"] == "start"
    assert row["state"] == {"seen": True}
    assert row["awaiting"] is None
    assert row["deciders"] == []
    assert row["status"] == "running"
    assert row["result"] == ""
    assert row["expires_at"] is None
    assert row["attempts"] == 0
    assert row["created_at"] == 1000.0
    assert flows.get("not-an-id") is None
    assert flows.get(flow_id + 1) is None


def test_claim_is_exclusive() -> None:
    flows = store()
    flow_id = create(flows)

    first = flows.claim(1000.0, worker="a", generation=0, limit=5)
    second = flows.claim(1000.0, worker="b", generation=0, limit=5)

    assert [row["id"] for row in first] == [flow_id]
    assert first[0]["claimed_by"] == "a"
    assert first[0]["attempts"] == 1
    assert second == []
    row = flows.get(flow_id)
    assert row is not None
    assert row["claimed_by"] == "a"
    assert row["lease_until"] > 1000.0


def test_a_bound_store_at_a_stale_generation_writes_nothing() -> None:
    flows = store()
    flow_id = create(flows)
    deposed = store()
    deposed.bind_lease(FakeLease())

    with pytest.raises(Superseded):
        create(deposed)
    with pytest.raises(Superseded):
        deposed.save(flow_id, status="done")
    assert deposed.claim(1000.0, worker="stale", generation=3, limit=5) == []

    row = flows.get(flow_id)
    assert row is not None
    assert row["status"] == "running"
    assert row["claimed_by"] == ""
    assert len(flows.pending_for("p1")) == 1


def test_a_store_bound_at_generation_zero_refuses() -> None:
    flows = store()
    flow_id = create(flows)
    standby = store()
    standby.bind_lease(FakeLease(generation=0))

    with pytest.raises(Superseded):
        standby.save(flow_id, status="done")


def test_save_is_conditional_on_the_expected_state() -> None:
    flows = store()
    flow_id = create(flows)

    assert flows.save(flow_id, status="waiting", expect_status="running") is True
    assert flows.save(flow_id, status="done", expect_status="running") is False
    row = flows.get(flow_id)
    assert row is not None
    assert row["status"] == "waiting"


def test_expire_turns_an_overdue_waiting_flow_into_expired() -> None:
    flows = store()
    due = create(flows)
    later = create(flows, requester="p2")
    flows.save(due, status="waiting", expires_at=1100.0, awaiting={"kind": "connect",
                                                                 "account_kind": "google",
                                                                 "instruction": ""})
    flows.save(later, status="waiting", expires_at=5000.0, awaiting={"kind": "connect",
                                                                   "account_kind": "google",
                                                                   "instruction": ""})

    assert [row["id"] for row in flows.overdue(1200.0)] == [due]
    assert flows.expire(1200.0) == 1
    expired = flows.get(due)
    still_waiting = flows.get(later)
    assert expired is not None
    assert expired["status"] == "expired"
    assert still_waiting is not None
    assert still_waiting["status"] == "waiting"
    assert flows.expire(1200.0) == 0


def test_reap_releases_a_dead_claim_and_keeps_attempts() -> None:
    flows = store()
    flow_id = create(flows)
    flows.claim(1000.0, worker="dead", generation=0, limit=1)
    with flows.db.connect() as conn:
        conn.execute(
            text("UPDATE flows SET lease_until = 1001 WHERE id = :id"), {"id": flow_id}
        )

    assert flows.reap(2000.0) == 1
    row = flows.get(flow_id)
    assert row is not None
    assert row["status"] == "running"
    assert row["claimed_by"] == ""
    assert row["lease_until"] == 0
    assert row["attempts"] == 1
    assert flows.reap(2000.0) == 0


def test_pending_and_waiting_reads_are_scoped() -> None:
    flows = store()
    waiting = create(flows, requester="p1")
    done = create(flows, requester="p1", now=1001.0)
    other = create(flows, requester="p2")
    flows.save(waiting, status="waiting", now=1002.0)
    flows.save(done, status="done", now=1003.0)

    assert [row["id"] for row in flows.pending_for("p1")] == [waiting]
    assert [row["id"] for row in flows.waiting(10)] == [waiting]
    assert other not in [row["id"] for row in flows.waiting(10)]
    assert flows.pending_for("nobody") == []
    assert time.time() > 0
