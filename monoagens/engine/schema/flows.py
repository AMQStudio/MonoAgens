"""Durable flows: one row per flow instance and its awaited requirement (ADR-0013 §4.3).

`status` is the life cycle; `step` is the next step to run. `awaiting` holds the requirement
JSON (null while running), `deciders` the people an `Approval` waits on. `claimed_by`,
`lease_until`, `generation`, and `attempts` are the claim of the automation-run pattern; they
are what makes a restart resume instead of duplicate.
"""

from sqlalchemy import (
    BigInteger,
    CheckConstraint,
    Column,
    Float,
    Index,
    Integer,
    String,
    Table,
    Text,
    text,
)
from sqlalchemy.dialects.postgresql import JSONB

from .base import metadata

FLOW_STATUSES = (
    "running",
    "waiting",
    "done",
    "refused",
    "expired",
    "cancelled",
    "failed",
)
FLOW_TERMINAL = frozenset({"done", "refused", "expired", "cancelled", "failed"})

flows = Table(
    "flows",
    metadata,
    Column("id", BigInteger, primary_key=True, autoincrement=True),
    Column("owner", Text, nullable=False),
    Column("name", Text, nullable=False),
    Column("requester", Text, nullable=False),
    Column("thread_id", String(120), nullable=False, server_default=""),
    Column("gateway", String(40), nullable=False, server_default=""),
    # The next step to run, never the one that just ran.
    Column("step", Text, nullable=False, server_default=""),
    Column("state", JSONB, nullable=False, server_default=text("'{}'::jsonb")),
    Column("awaiting", JSONB, nullable=True),
    Column("deciders", JSONB, nullable=False, server_default=text("'[]'::jsonb")),
    Column("status", String(20), nullable=False, server_default="running"),
    Column("result", Text, nullable=False, server_default=""),
    # Null = never expires (`Approval.expires_in`/the flow's default also end up here).
    Column("expires_at", Float, nullable=True),
    Column("claimed_by", Text, nullable=False, server_default=""),
    Column("lease_until", Float, nullable=False, server_default="0"),
    Column("generation", BigInteger, nullable=False, server_default="0"),
    Column("attempts", Integer, nullable=False, server_default="0"),
    Column("created_at", Float, nullable=False),
    Column("updated_at", Float, nullable=False),
    CheckConstraint(
        "status IN ('running','waiting','done','refused','expired','cancelled','failed')",
        name="ck_flows_status",
    ),
)
# The worker claims by claimability and age; `/flow` lists one person's open rows.
Index("ix_flows_claim", flows.c.status, flows.c.updated_at)
Index("ix_flows_person", flows.c.requester, flows.c.status)
