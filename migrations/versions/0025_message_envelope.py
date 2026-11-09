"""the message envelope: speaker, actor, addressee, kind, trust, validity, and label provenance

Revision ID: 0025
Revises: 0024
Create Date: 2026-11-10

ADR-0012 §4.1/§4.2: every stored message carries envelope labels written once at ingress, so a
later reader never has to re-derive who spoke, who was addressed, and how far the row may be
trusted. This revision only adds the columns with safe defaults: existing rows read `unknown`
(the documented value for rows that cannot be labelled), and every existing writer keeps working
because it never names them. Conditions live in `valid_until` (null = always valid) and per-label
provenance in `label_meta`.
"""

import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects import postgresql

revision = "0025"
down_revision = "0024"
branch_labels = None
depends_on = None

SPEAKERS = "'self','sender','other_person','system_receipt','unknown'"
KINDS = "'chat','question','command','tool_result','notice','unknown'"
TRUSTS = "'owner','member','untrusted_content','unknown'"


def upgrade() -> None:
    op.add_column(
        "messages",
        sa.Column("speaker", sa.String(20), nullable=False, server_default="unknown"),
    )
    op.add_column(
        "messages",
        sa.Column("actor", sa.String(160), nullable=False, server_default=""),
    )
    op.add_column(
        "messages",
        sa.Column("addressed_to", sa.String(160), nullable=False, server_default="unknown"),
    )
    op.add_column(
        "messages",
        sa.Column("reply_to_self", sa.Boolean(), nullable=False, server_default=sa.text("false")),
    )
    op.add_column(
        "messages",
        sa.Column("kind", sa.String(20), nullable=False, server_default="unknown"),
    )
    op.add_column(
        "messages",
        sa.Column("trust", sa.String(24), nullable=False, server_default="unknown"),
    )
    op.add_column(
        "messages",
        sa.Column(
            "valid_until",
            postgresql.JSONB(astext_type=sa.Text()),
            nullable=True,
        ),
    )
    op.add_column(
        "messages",
        sa.Column(
            "label_meta",
            postgresql.JSONB(astext_type=sa.Text()),
            nullable=False,
            server_default=sa.text("'{}'::jsonb"),
        ),
    )
    op.create_check_constraint("ck_messages_speaker", "messages", f"speaker IN ({SPEAKERS})")
    op.create_check_constraint("ck_messages_kind", "messages", f"kind IN ({KINDS})")
    op.create_check_constraint("ck_messages_trust", "messages", f"trust IN ({TRUSTS})")


def downgrade() -> None:
    op.drop_constraint("ck_messages_trust", "messages", type_="check")
    op.drop_constraint("ck_messages_kind", "messages", type_="check")
    op.drop_constraint("ck_messages_speaker", "messages", type_="check")
    op.drop_column("messages", "label_meta")
    op.drop_column("messages", "valid_until")
    op.drop_column("messages", "trust")
    op.drop_column("messages", "kind")
    op.drop_column("messages", "reply_to_self")
    op.drop_column("messages", "addressed_to")
    op.drop_column("messages", "actor")
    op.drop_column("messages", "speaker")
