"""The `flows` table behind one fenced store (ADR-0013 §4.3, ADR-0002 §4.3).

Every mutation carries the lease fence exactly like `ActionLedger`: an insert goes through
`fenced_insert(...).returning(...)`, an update repeats the fence, and a refused fence raises
`Superseded` while writing nothing. An unbound store (CLI, admin, tests) writes with no
predicate at all, so the fence is omitted, never faked.

The claim copies `AutomationStore.claim`: `FOR UPDATE SKIP LOCKED`, a lease deadline, a claim
generation, and an attempt counter. `running` is the queue; `waiting` rows are owned by a
requirement and are only moved by `expire` or by the runner that satisfies them.
"""

import logging
import time

from sqlalchemy import ColumnElement, TextClause, select, update

from ..db import Database
from ..jobs.lease import FencedStore, Superseded, fenced_insert
from ..schema import flows

log = logging.getLogger(__name__)

# How long one claim owns a flow. It must exceed the step timeout, so a slow step is never
# re-claimed mid-run; a dead process is reaped after it.
CLAIM_SECONDS = 120.0

_KEEP = object()

# What `save` accepts and how each value is stored; a field left at `_KEEP` is not written.
_CONVERT = {
    "step": str,
    "state": lambda value: dict(value),
    "awaiting": lambda value: None if value is None else dict(value),
    "deciders": lambda value: [str(item) for item in value],
    "status": str,
    "result": str,
    "expires_at": lambda value: None if value is None else float(value),
    "claimed_by": str,
    "lease_until": float,
}


def _changes(**fields) -> dict:
    """The columns one `save` call changes; a value of `_KEEP` leaves the column alone."""
    values: dict = {}
    for name, convert in _CONVERT.items():
        value = fields.get(name, _KEEP)
        if value is _KEEP:
            continue
        values[name] = convert(value)
    return values


class FlowStore(FencedStore):
    """Create, read, claim, save, list, expire, and reap flow rows; no delivery lives here."""

    def __init__(self, db: Database) -> None:
        self.db = db

    # ---------------------------------------------------------------- create

    def create(
        self,
        *,
        owner: str,
        name: str,
        requester: str,
        thread_id: str,
        gateway: str,
        step: str,
        state: dict | None = None,
        expires_at: float | None = None,
        now: float | None = None,
    ) -> int:
        """Insert one `running` row at `step`; returns its id, or raises `Superseded`."""
        moment = time.time() if now is None else float(now)
        row = {
            "owner": str(owner),
            "name": str(name),
            "requester": str(requester),
            "thread_id": str(thread_id)[:120],
            "gateway": str(gateway)[:40],
            "step": str(step),
            "state": dict(state or {}),
            "awaiting": None,
            "deciders": [],
            "status": "running",
            "result": "",
            "expires_at": None if expires_at is None else float(expires_at),
            "claimed_by": "",
            "lease_until": 0.0,
            "generation": 0,
            "attempts": 0,
            "created_at": moment,
            "updated_at": moment,
        }
        fence = self.fence_clause()
        with self.db.connect() as conn:
            found = conn.execute(
                fenced_insert(flows, row, fence).returning(flows.c.id)
            ).scalar()
        if found is None:
            if fence is not None:
                raise Superseded("lease generation moved on: flow refused")
            raise RuntimeError("the new flow got no primary key")
        return int(found)

    def get(self, flow_id: int | str) -> dict | None:
        try:
            key = int(flow_id)
        except (TypeError, ValueError):
            return None
        return self.db.fetch_one(select(flows).where(flows.c.id == key))

    # ----------------------------------------------------------------- claim

    def claim(
        self,
        now: float,
        *,
        worker: str,
        generation: int = 0,
        limit: int = 5,
        lease_seconds: float = CLAIM_SECONDS,
    ) -> list[dict]:
        """Claim up to `limit` runnable rows; each update repeats its own predicates."""
        moment = float(now)
        lease_until = moment + max(1.0, float(lease_seconds))
        fence = self.guard_clause(generation)
        claimed: list[dict] = []
        with self.db.connect() as conn:
            rows = (
                conn.execute(
                    select(flows)
                    .where(
                        flows.c.status == "running",
                        flows.c.lease_until <= moment,
                    )
                    .order_by(flows.c.updated_at, flows.c.id)
                    .limit(max(1, int(limit)))
                    .with_for_update(skip_locked=True)
                )
                .mappings()
                .all()
            )
            for row in rows:
                statement = (
                    update(flows)
                    .where(
                        flows.c.id == int(row["id"]),
                        flows.c.status == "running",
                        flows.c.lease_until <= moment,
                    )
                    .values(
                        claimed_by=str(worker),
                        lease_until=lease_until,
                        generation=int(generation),
                        attempts=int(row["attempts"]) + 1,
                        updated_at=moment,
                    )
                )
                if fence is not None:
                    statement = statement.where(fence)
                if not conn.execute(statement).rowcount:
                    continue
                body = dict(row)
                body.update(
                    claimed_by=str(worker),
                    lease_until=lease_until,
                    generation=int(generation),
                    attempts=int(row["attempts"]) + 1,
                )
                claimed.append(body)
        return claimed

    # ------------------------------------------------------------------ save

    def save(
        self,
        flow_id: int | str,
        *,
        now: float | None = None,
        step: object = _KEEP,
        state: object = _KEEP,
        awaiting: object = _KEEP,
        deciders: object = _KEEP,
        status: object = _KEEP,
        result: object = _KEEP,
        expires_at: object = _KEEP,
        claimed_by: object = _KEEP,
        lease_until: object = _KEEP,
        expect_status: str = "",
        expect_step: str | None = None,
    ) -> bool:
        """One fenced update; `False` when the caller's expected state no longer holds.

        `expect_status`/`expect_step` make a transition conditional, so two workers (or a
        retry) can never apply the same outcome twice. A bound store refuses with
        `Superseded` when the lease moved on, even though the row itself still exists.
        """
        moment = time.time() if now is None else float(now)
        try:
            key = int(flow_id)
        except (TypeError, ValueError):
            return False
        values = _changes(
            step=step,
            state=state,
            awaiting=awaiting,
            deciders=deciders,
            status=status,
            result=result,
            expires_at=expires_at,
            claimed_by=claimed_by,
            lease_until=lease_until,
        )
        values["updated_at"] = moment
        where: list[ColumnElement[bool] | TextClause] = [flows.c.id == key]
        if expect_status:
            where.append(flows.c.status == str(expect_status))
        if expect_step is not None:
            where.append(flows.c.step == str(expect_step))
        fence = self.fence_clause()
        if fence is not None:
            where.append(fence)
        with self.db.connect() as conn:
            changed = int(
                conn.execute(update(flows).where(*where).values(**values)).rowcount or 0
            )
        if not changed and fence is not None:
            raise Superseded("lease generation moved on: flow write refused")
        return bool(changed)

    # ----------------------------------------------------------------- reads

    def pending_for(self, person: str) -> list[dict]:
        """The open flows this person asked for, oldest first."""
        return self.db.fetch_all(
            select(flows)
            .where(
                flows.c.requester == str(person),
                flows.c.status.in_(("running", "waiting")),
            )
            .order_by(flows.c.created_at, flows.c.id)
        )

    def waiting(self, limit: int = 50) -> list[dict]:
        """Rows parked on a requirement, for the poller's re-check."""
        return self.db.fetch_all(
            select(flows)
            .where(flows.c.status == "waiting")
            .order_by(flows.c.updated_at, flows.c.id)
            .limit(max(1, int(limit)))
        )

    def overdue(self, now: float) -> list[dict]:
        """Rows whose expiry has passed but whose status is not terminal yet."""
        return self.db.fetch_all(
            select(flows).where(
                flows.c.status.in_(("running", "waiting")),
                flows.c.expires_at.is_not(None),
                flows.c.expires_at < float(now),
            )
        )

    # --------------------------------------------------------------- expiry

    def expire(self, now: float | None = None) -> int:
        """`waiting`/`running` rows past `expires_at` become `expired`; returns the count."""
        moment = time.time() if now is None else float(now)
        where: list[ColumnElement[bool] | TextClause] = [
            flows.c.status.in_(("running", "waiting")),
            flows.c.expires_at.is_not(None),
            flows.c.expires_at < moment,
        ]
        fence = self.fence_clause()
        if fence is not None:
            where.append(fence)
        with self.db.connect() as conn:
            return int(
                conn.execute(
                    update(flows)
                    .where(*where)
                    .values(
                        status="expired",
                        claimed_by="",
                        lease_until=0.0,
                        updated_at=moment,
                    )
                ).rowcount
                or 0
            )

    def reap(self, now: float | None = None) -> int:
        """A claim whose process died goes back to `running`; attempts stay counted."""
        moment = time.time() if now is None else float(now)
        where: list[ColumnElement[bool] | TextClause] = [
            flows.c.status == "running",
            flows.c.lease_until > 0,
            flows.c.lease_until < moment,
        ]
        fence = self.fence_clause()
        if fence is not None:
            where.append(fence)
        with self.db.connect() as conn:
            released = int(
                conn.execute(
                    update(flows)
                    .where(*where)
                    .values(claimed_by="", lease_until=0.0, updated_at=moment)
                ).rowcount
                or 0
            )
        if released:
            log.info("flow reaper released %d stale claim(s)", released)
        return released
