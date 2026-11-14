"""Shared runtime settings for the tests: fixed values, no file or database needed."""

from __future__ import annotations

from typing import Any

IDENTITY = {
    "name": "Mona",
    "language": "id",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
}

CONSTANTS = {
    "seed": 1,
    "own_name_weight": 0.5,
    "info_gap_weight": 0.35,
    "feeling_of_knowing_weight": 0.35,
    "relevance_weight": 0.2,
    "social_cost_weight": -0.4,
    "budget_weight": 0.3,
    "talkativeness_theta_base": [0.85, 0.35],
    "spontaneity_tau": [0.02, 0.2],
    "talk_rate_window_minutes": 10,
    "promise_debt_weight": 0.1,
    "note_band": 0.2,
    "retrieve_k": 5,
    "recent_events": 20,
    "recent_messages": 20,
    "recent_elsewhere": 10,
    "recent_tokens": 2000,
    "memory_tokens": 1500,
    "thread_result_tokens": 3000,
    "volatile_tokens_max": 8000,
    "recall_results": 10,
    "max_task_steps": 30,
    "max_open_tasks": 3,
    "max_thought_rounds": 3,
    "model_call_retries": 1,
    "rrf_k": 60,
    "decay": 0.995,
    "persistence_decay_d": [0.7, 0.3],
    "boredom_gamma_per_minute": [0.99, 0.8],
    "context_weight": 1.0,
    "importance_weight": 0.5,
    "default_expires_days": 30,
    "max_active": 200,
    "job_timeout_seconds": 5.0,
}


def settings(**sections: dict[str, Any]) -> dict[str, Any]:
    """Return one settings view, overriding whole sections by name."""
    value: dict[str, Any] = {
        "identity": dict(IDENTITY),
        "timezone": "Asia/Jakarta",
        "owner_settings": {"daily_token_budget": 0},
        "runtime": {"tick_seconds": 30,
                    "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2}},
        "bodies": {},
        "plugins": [],
        "knobs": {"memory_persistence": 0.5, "boredom_with_repeats": 0.5,
                  "talkativeness": 0.45, "restraint_after_speaking": 0.5, "spontaneity": 0.2,
                  "reserve_when_unfamiliar": 0.3, "patience_seconds": 90},
        "calibration": {"wake_gate_lo": 0.35},
        "constants": dict(CONSTANTS),
        "references": {},
        "lessons": [],
        "procedures": [],
        "manifest": {},
    }
    for name, patch in sections.items():
        if isinstance(patch, dict) and isinstance(value.get(name), dict):
            value[name] = {**value[name], **patch}
        else:
            value[name] = patch
    return value


def body_tiers(*capability_sets) -> dict[str, str]:
    """Return the base tier a test body declares for each capability it is handed.

    A body's talking and expression capabilities are what everyone may use (contract.md §10);
    a test body that borrows a real adapter's descriptors declares its tiers with this.
    """
    found: dict[str, str] = {}
    for capabilities in capability_sets:
        for capability in capabilities:
            found[str(capability["name"])] = "guest"
    return found


def open_book(who: str, *permissions: str):
    """Return a role book in which one person holds these permissions (guard tests)."""
    from monoagens.middleware import roles

    return roles.RoleBook(
        roles={"open": roles.Role("open", base="guest", allow=frozenset(permissions))},
        people={who: roles.Person(who, roles=("open",))},
        bases={**roles.CORE_PERMISSIONS, **{name: None for name in permissions}})


async def last_receipt(conn) -> dict:
    """Return the newest `act.result` payload, which is the one receipt a test reads."""
    cur = await conn.execute("SELECT payload FROM events WHERE kind = 'act.result' "
                             "ORDER BY id DESC LIMIT 1")
    row = await cur.fetchone()
    return dict(row[0]) if row else {}


def receipt_result(payload: dict) -> dict:
    """Return one receipt's answer, with a failure's reason under `error` for a test to read."""
    if payload.get("state") in (None, "done"):
        return payload.get("result") if isinstance(payload.get("result"), dict) else {}
    return {"error": str(payload.get("reason") or "")}


async def publish_audience(conn, place: str, actors, *, unknown: bool = False, ts=None):
    """Report who can perceive a place, the one fact privacy reads (contract.md §4.3)."""
    from datetime import datetime, timezone

    from monoagens.infra.store import append_event

    return await append_event(
        conn, ts=ts or datetime(2026, 11, 12, 8, 0, tzinfo=timezone.utc),
        kind="place.audience", place=place,
        payload={"place": place, "actors": list(actors), "unknown": unknown})


def context(*, now, place: str, reason: str, core_memory: str = "",
            people=(), expectations=(), memories=(), withheld=(), recent=(),
            recent_elsewhere=(), doing=(), trigger=None, last_interaction=None,
            capabilities=(), results=(), blocks=(), self_text: str = ""):
    """Build a desk the way the old field shape did, as the items the core renders now.

    A test that says "a desk with core memory, a card and a withheld note" keeps saying
    that; the items are its only form (contract.md §3).
    """
    from monoagens.contract import ContextItem
    from monoagens.core.context.layout import PFCContext

    items = []
    if core_memory:
        items.append(ContextItem(section="core_memory", text=core_memory))
    items.extend(ContextItem(section="doing", text=text, kind="chain") for text in doing)
    items.extend(ContextItem(section="expectations", text=text, kind="expectation")
                 for text in expectations)
    items.extend(ContextItem(section="memories", text=str(memory),
                             sources=tuple(getattr(memory, "sources", ()) or ()))
                 for memory in memories)
    if withheld:
        items.append(ContextItem(section="notes", text=withheld_line(dict(withheld))))
    items.extend(ContextItem(section="recent", text=_line(event), place=event.place or "",
                             sources=(event.id,) if event.id else ())
                 for event in recent)
    items.extend(ContextItem(section="elsewhere", text=_line(event),
                             place=event.place or "",
                             sources=(event.id,) if event.id else ())
                 for event in recent_elsewhere)
    items.extend(ContextItem(section="cards", text=text) for text in people)
    if trigger is not None:
        items.append(ContextItem(section="trigger",
                                 text=f"{trigger.who or '?'}: {trigger.description}",
                                 place=trigger.place or "",
                                 sources=(trigger.id,) if trigger.id else ()))
    return PFCContext(now=now, place=place, reason=reason, items=tuple(items),
                      self_text=self_text, trigger=trigger,
                      last_interaction=last_interaction, capabilities=capabilities,
                      results=results)


def withheld_line(counts) -> str:
    """Render the D56 note for a test that withheld a count from a label."""
    from monoagens.middleware.privacy import withheld_line as render

    return render(counts)


def _line(event) -> str:
    """Render one recent line the way the core does, for a test-built desk."""
    return f"{event.ts:%H:%M} {event.who or '?'}: {event.description}"
