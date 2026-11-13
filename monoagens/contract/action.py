"""Actions and receipts: what a body is asked to do and what it answers (contract.md §3)."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any

# Every action ends in exactly one receipt with one of these states (contract.md §3).
RECEIPT_STATES = ("done", "refused", "held", "failed", "unknown")

# The one marker for a cut tool output; a tool never invents a second one.
OUTPUT_MARKER = (
    "\n...[truncated: output exceeds {limit} chars; "
    "full in {ref}; continue from character {n}]"
)


@dataclass(frozen=True, slots=True)
class Action:
    """One thing a body is asked to do, with the id the body deduplicates on."""

    id: str
    capability: str
    place: str
    arguments: Mapping[str, Any] = field(default_factory=dict)
    caused_by: int | None = None


@dataclass(frozen=True, slots=True)
class Receipt:
    """The honest outcome of one action."""

    state: str
    reason: str = ""
    result: Any = None

    def __post_init__(self) -> None:
        """Refuse a state the contract does not define instead of passing it on."""
        if self.state not in RECEIPT_STATES:
            raise ValueError(f"receipt state {self.state!r} is not one of "
                             f"{', '.join(RECEIPT_STATES)}")


def action_id(job_key: str, job_event_id: int, round_index: int, n: int) -> str:
    """Return the deterministic id of the `n`th act of one reasoning round.

    The id is derived from the job and the step (contract.md §3), so a resume after a
    restart derives the same id and a body answers a repeated id from its own memory.
    """
    return f"{job_key}:{job_event_id}:{round_index}:{n}"
