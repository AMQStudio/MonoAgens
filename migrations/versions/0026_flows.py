"""durable flows table

Revision ID: 0026
Revises: 0025
Create Date: 2026-11-10

ADR-0013 §4.3: the flow runner gets its own table. `automations`/`automation_runs` are
schedule-specific (event, condition, occurrence key, daily budget), while a flow is a step
machine over one awaited requirement. The claim columns and the generation copy the run-table
pattern so a restart resumes instead of duplicating (ADR-0003).
"""

import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects import postgresql

revision = "0026"
down_revision = "0025"
branch_labels = None
depends_on = None

STATUSES = "'running','waiting','done','refused','expired','cancelled','failed'"


def upgrade() -> None:
    op.create_table(
        "flows",
        sa.Column("id", sa.BigInteger(), autoincrement=True, nullable=False),
        sa.Column("owner", sa.Text(), nullable=False),
        sa.Column("name", sa.Text(), nullable=False),
        sa.Column("requester", sa.Text(), nullable=False),
        sa.Column("thread_id", sa.String(120), nullable=False, server_default=""),
        sa.Column("gateway", sa.String(40), nullable=False, server_default=""),
        sa.Column("step", sa.Text(), nullable=False, server_default=""),
        sa.Column(
            "state",
            postgresql.JSONB(astext_type=sa.Text()),
            nullable=False,
            server_default=sa.text("'{}'::jsonb"),
        ),
        sa.Column("awaiting", postgresql.JSONB(astext_type=sa.Text()), nullable=True),
        sa.Column(
            "deciders",
            postgresql.JSONB(astext_type=sa.Text()),
            nullable=False,
            server_default=sa.text("'[]'::jsonb"),
        ),
        sa.Column("status", sa.String(20), nullable=False, server_default="running"),
        sa.Column("result", sa.Text(), nullable=False, server_default=""),
        sa.Column("expires_at", sa.Float(), nullable=True),
        sa.Column("claimed_by", sa.Text(), nullable=False, server_default=""),
        sa.Column("lease_until", sa.Float(), nullable=False, server_default="0"),
        sa.Column("generation", sa.BigInteger(), nullable=False, server_default="0"),
        sa.Column("attempts", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("created_at", sa.Float(), nullable=False),
        sa.Column("updated_at", sa.Float(), nullable=False),
        sa.PrimaryKeyConstraint("id", name="pk_flows"),
        sa.CheckConstraint(f"status IN ({STATUSES})", name="ck_flows_status"),
    )
    op.create_index("ix_flows_claim", "flows", ["status", "updated_at"])
    op.create_index("ix_flows_person", "flows", ["requester", "status"])


def downgrade() -> None:
    op.drop_index("ix_flows_person", table_name="flows")
    op.drop_index("ix_flows_claim", table_name="flows")
    op.drop_table("flows")
