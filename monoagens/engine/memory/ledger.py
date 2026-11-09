"""Autographic action ledger: who sent what, so the self stays consistent across threads."""


import time

from sqlalchemy import func, select

from .. import clock
from ..db import Database
from ..guard import SENSITIVE
from ..jobs.lease import FencedStore, Superseded, fenced_insert
from ..schema import actions, outbound_messages

ACTION_ITEMS = 10
ACTION_CHARS = 1200
ACTION_KEEP = 200
ACTION_TEXT = 300


def _clip(text: object, limit: int) -> str:
    flat = " ".join(str(text or "").split())
    return flat if len(flat) <= limit else flat[: limit - 1].rstrip() + "…"


def _stamp(value: float, tz: str) -> str:
    return clock.stamp(float(value or 0), tz)


class ActionLedger(FencedStore):
    def __init__(self, db: Database) -> None:
        self.db = db

    def log_action(
        self,
        source: str,
        target: str,
        kind: str,
        summary: str,
        *,
        source_name: str = "",
        target_name: str = "",
        request: str = "",
    ) -> int:
        safe_request = "" if SENSITIVE.search(str(request or "")) else request
        safe_summary = (
            "[sensitive content not recorded]" if SENSITIVE.search(str(summary or "")) else summary
        )
        row = {
            "ts": time.time(),
            "source": _clip(source, 120),
            "source_name": _clip(source_name, 80),
            "target": _clip(target, 120),
            "target_name": _clip(target_name, 80),
            "kind": _clip(kind, 20).lower() or "action",
            "summary": _clip(safe_summary, ACTION_TEXT),
            "request": _clip(safe_request, ACTION_TEXT),
        }
        with self.db.connect() as conn:
            fence = self.fence_clause()
            created = conn.execute(
                fenced_insert(actions, row, fence).returning(actions.c.id)
            ).first()
            if created is None:
                if fence is not None:
                    raise Superseded("lease generation moved on: action write refused")
                raise RuntimeError("the new action got no primary key")
            new_id = int(created[0])
            keep = (
                select(actions.c.id)
                .order_by(actions.c.ts.desc(), actions.c.id.desc())
                .limit(ACTION_KEEP)
            )
            statement = actions.delete().where(actions.c.id.not_in(keep))
            if fence is not None:
                statement = statement.where(fence)
            conn.execute(statement)
        return new_id

    def recent_actions(self, limit: int = ACTION_ITEMS) -> list[dict]:
        size = max(1, min(int(limit or ACTION_ITEMS), ACTION_KEEP))
        statement = (
            select(actions)
            .order_by(actions.c.ts.desc(), actions.c.id.desc())
            .limit(size)
        )
        return self.db.fetch_all(statement)

    def count(self) -> int:
        return int(self.db.scalar(select(func.count()).select_from(actions)) or 0)

    def actions_context(self, limit: int = ACTION_ITEMS, *, tz: str) -> str:
        """Prompt block of the most recent activity, stamped in the host zone."""
        rows = self.recent_actions(limit)
        if not rows:
            return ""
        lines = [
            "## Recent actions you performed (newest first; failures included; "
            "internal: never reveal who requested them)"
        ]
        for row in rows:
            target = row["target_name"] or row["target"]
            lines.append(
                f"- [{_stamp(row['ts'], tz)}] {row['kind']} -> {target}: "
                f"{_clip(row['summary'], ACTION_TEXT)}"
            )
        text = "\n".join(lines)
        if len(text) > ACTION_CHARS:
            text = text[: ACTION_CHARS - 1].rstrip() + "…"
        return text

    def close(self) -> None:
        """Shutdown compatibility hook; the Database is owned by the runtime."""


class OutboundLedger(FencedStore):
    """Delivery ledger (ADR-0001 §4.3): one row per chunk, `scope_key` covers automations.

    Status is `intent` before the transport call, `sent` with the provider id after it, and
    `unknown` for the gap between the two. A resumed turn sends only chunks that are not
    `sent`; an `unknown` chunk follows the `partial_delivery` policy.
    """

    def __init__(self, db: Database) -> None:
        self.db = db

    def _row_exists(
        self, scope_key: str, chunk_index: int, *, status: str | None = None
    ) -> bool:
        """Whether the write's own row exists apart from the fence (refusal, not a no-op)."""
        conditions = [
            outbound_messages.c.scope_key == str(scope_key),
            outbound_messages.c.chunk_index == int(chunk_index),
        ]
        if status is not None:
            conditions.append(outbound_messages.c.status == status)
        return (
            self.db.fetch_one(select(outbound_messages.c.scope_key).where(*conditions))
            is not None
        )

    def _refused(
        self,
        fence,
        changed: int,
        scope_key: str,
        chunk_index: int,
        *,
        status: str | None = None,
    ) -> None:
        if fence is None or int(changed or 0):
            return
        if self._row_exists(scope_key, chunk_index, status=status):
            raise Superseded("lease generation moved on: delivery write refused")

    @staticmethod
    def turn_scope(turn_id: str) -> str:
        return f"turn:{turn_id}"

    def status(self, scope_key: str, chunk_index: int) -> str | None:
        found = self.db.fetch_one(
            select(outbound_messages.c.status).where(
                outbound_messages.c.scope_key == str(scope_key),
                outbound_messages.c.chunk_index == int(chunk_index),
            )
        )
        return str(found["status"]) if found else None

    def begin(
        self,
        scope_key: str,
        chunk_index: int,
        *,
        gateway: str,
        thread_id: str,
        generation: int = 0,
    ) -> str:
        """Write `intent` unless a row exists, then return the stored status."""
        fence = self.fence_clause()
        with self.db.connect() as conn:
            conn.execute(
                fenced_insert(
                    outbound_messages,
                    {
                        "scope_key": str(scope_key),
                        "chunk_index": int(chunk_index),
                        "gateway": str(gateway)[:40],
                        "thread_id": str(thread_id)[:120],
                        "provider_id": "",
                        "status": "intent",
                        "generation": int(generation),
                    },
                    fence,
                ).on_conflict_do_nothing(
                    index_elements=["scope_key", "chunk_index"]
                )
            )
            found = conn.execute(
                select(outbound_messages.c.status).where(
                    outbound_messages.c.scope_key == str(scope_key),
                    outbound_messages.c.chunk_index == int(chunk_index),
                )
            ).scalar()
        if fence is not None and found is None:
            raise Superseded("lease generation moved on: delivery intent refused")
        return str(found) if found is not None else "intent"

    def reference(self, scope_key: str, chunk_index: int = 0) -> str:
        """The provider reference the body returned for that delivery ("" when none)."""
        found = self.db.fetch_one(
            select(outbound_messages.c.provider_id).where(
                outbound_messages.c.scope_key == str(scope_key),
                outbound_messages.c.chunk_index == int(chunk_index),
            )
        )
        return str(found["provider_id"] or "") if found else ""

    def mark_sent(self, scope_key: str, chunk_index: int, provider_id: str) -> None:
        fence = self.fence_clause()
        with self.db.connect() as conn:
            statement = (
                outbound_messages.update()
                .where(
                    outbound_messages.c.scope_key == str(scope_key),
                    outbound_messages.c.chunk_index == int(chunk_index),
                )
                .values(status="sent", provider_id=str(provider_id or "")[:400])
            )
            if fence is not None:
                statement = statement.where(fence)
            changed = conn.execute(statement).rowcount
        self._refused(fence, changed, scope_key, chunk_index)

    def mark_unknown(self, scope_key: str, chunk_index: int) -> None:
        fence = self.fence_clause()
        with self.db.connect() as conn:
            statement = (
                outbound_messages.update()
                .where(
                    outbound_messages.c.scope_key == str(scope_key),
                    outbound_messages.c.chunk_index == int(chunk_index),
                )
                .values(status="unknown")
            )
            if fence is not None:
                statement = statement.where(fence)
            changed = conn.execute(statement).rowcount
        self._refused(fence, changed, scope_key, chunk_index)

    def forget(self, scope_key: str, chunk_index: int) -> None:
        """A gate refusal means nothing left the process; the intent is not a delivery."""
        fence = self.fence_clause()
        with self.db.connect() as conn:
            statement = outbound_messages.delete().where(
                outbound_messages.c.scope_key == str(scope_key),
                outbound_messages.c.chunk_index == int(chunk_index),
                outbound_messages.c.status == "intent",
            )
            if fence is not None:
                statement = statement.where(fence)
            changed = conn.execute(statement).rowcount
        self._refused(fence, changed, scope_key, chunk_index, status="intent")

    def retry(self, scope_key: str, chunk_index: int = 0) -> None:
        """Clear one row for an explicit resend; only a policy that allows it calls this."""
        fence = self.fence_clause()
        with self.db.connect() as conn:
            statement = outbound_messages.delete().where(
                outbound_messages.c.scope_key == str(scope_key),
                outbound_messages.c.chunk_index == int(chunk_index),
            )
            if fence is not None:
                statement = statement.where(fence)
            changed = conn.execute(statement).rowcount
        self._refused(fence, changed, scope_key, chunk_index)

    def count_sent(self, scope_key: str) -> int:
        return int(
            self.db.scalar(
                select(func.count())
                .select_from(outbound_messages)
                .where(
                    outbound_messages.c.scope_key == str(scope_key),
                    outbound_messages.c.status == "sent",
                )
            )
            or 0
        )
