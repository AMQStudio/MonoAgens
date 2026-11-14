"""Privacy-safe INFO observability lines for live kernel event flow."""

from __future__ import annotations

import logging
from typing import Any

from monoagens.infra import config
from monoagens.infra.store import payload_text
from monoagens.types import Event

LOGGER = logging.getLogger("kernel.events")


def _one_line(value: Any, *, fallback: str = "-") -> str:
    text = " ".join(str(value or "").split())
    return text or fallback


def _text_length(event: Any) -> int:
    payload = getattr(event, "payload", None) or {}
    kind = str(getattr(event, "kind", None) or ("percept" if hasattr(event, "actor") else ""))
    if kind == "percept":
        # The envelope's field, never the payload (2.2); the fallback reads the one raw shape
        # an adapter may still hand in before the door upcasts it.
        return len(getattr(event, "description", "") or str(
            payload.get("description") or payload.get("text") or ""))
    if kind == "act.out":
        args = payload.get("arguments") or {}
        if isinstance(args, dict) and args.get("text"):
            return len(str(args["text"]))
    return len(payload_text(payload))


def _is_gate_decision(event: Any) -> bool:
    if getattr(event, "kind", None) in ("decision.silent", "decision.wake"):
        return True
    kind = str(getattr(event, "kind", None) or "")
    return kind.startswith("decision.") and "reason" in (getattr(event, "payload", None) or {})


class EventObserver:
    """Emit one concise INFO line per key event, never leaking message text."""

    def __init__(self, *, summary_every: int, conn: Any = None) -> None:
        self.summary_every = max(1, int(summary_every))
        self.conn = conn
        self.imported_count = 0

    def observe(self, event: Any) -> None:
        """Log one INFO line for a key event, whether it arrives as a percept or an event."""
        payload = getattr(event, "payload", None) or {}
        kind = str(getattr(event, "kind", None) or
                   ("percept" if hasattr(event, "actor") else "unknown"))
        adapter = _one_line(getattr(event, "body", None) or payload.get("adapter"))
        place = _one_line(getattr(event, "place", ""))
        who = _one_line(getattr(event, "who", None) or getattr(event, "actor", ""))
        imported = getattr(event, "imported", None)
        if kind == "percept" and (imported is True or payload.get("imported") is True):
            self.imported_count += 1
            if self.imported_count % self.summary_every == 0:
                LOGGER.info(
                    "kind=percept.imported count=%d adapter=%s place=%s",
                    self.imported_count, adapter, place,
                )
            return
        if kind in ("percept", "act.out"):
            LOGGER.info(
                "kind=%s adapter=%s place=%s who=%s len=%d",
                kind, adapter, place, who, _text_length(event),
            )
            return
        if _is_gate_decision(event):
            self.observe_decision(event, kind, payload.get("reason"))
            return
        if kind == "guard.stopped":
            reason = _one_line(payload.get("reason"))
            LOGGER.info(
                "kind=guard.stopped adapter=%s place=%s who=%s len=%d reason=%s",
                adapter, place, who, 0, reason,
            )
            return
        if kind == "error":
            raw = _one_line(payload.get("error"))
            if self.conn is not None and hasattr(self.conn, "info"):
                raw = config.redact_connection_secrets(raw, self.conn)
            LOGGER.info(
                "kind=error adapter=%s place=%s who=%s len=%d reason=%s",
                adapter, place, who, 0, raw,
            )

    def observe_decision(self, event: Event, kind: str, reason: Any) -> None:
        """Log one gate decision INFO line (used directly for reflex wakes too)."""
        payload = event.payload or {}
        adapter = _one_line(event.body or payload.get("adapter"))
        place = _one_line(event.place)
        who = _one_line(event.who)
        LOGGER.info(
            "kind=%s adapter=%s place=%s who=%s len=%d reason=%s",
            kind, adapter, place, who, 0, _one_line(reason),
        )
