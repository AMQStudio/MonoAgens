"""Staged approvals as a structured tool result and a requirement block (ADR-0013 §4.4-4.5)."""

from typing import TYPE_CHECKING

from .. import io
from .confirmations import pending_result

if TYPE_CHECKING:
    from .runtime import TurnRuntime
    from .state import TurnState

class ApprovalFlow:
    """Report staged approvals as data; a reply word never decides one."""

    runtime: TurnRuntime

    async def _pending_block(self, state: TurnState) -> str:
        """The structured `pending_approval` block while rows wait for this person.

        ADR-0013 §4.5: the model reads the state and asks the person to use the control; it
        never decides the row, and no reply word stands in for the control.
        """
        rows = await io.run(
            self.runtime.approvals.pending, state.thread_id, state.sender_id
        )
        return pending_result(rows, "waiting") if rows else ""

    async def _parked_ui(
        self, thread_id: str, sender: str, since: float
    ) -> dict:
        """The requirement block of a reply that just parked an action (ADR-0013 §4.4).

        The core names the approval; the body decides how a person gestures on it.
        """
        rows = await io.run(self.runtime.approvals.pending, thread_id, sender)
        fresh = [row for row in rows if float(row.created_at) >= since - 2.0]
        if not fresh:
            return {}
        return {"kind": "approval", "approval_id": int(fresh[-1].id)}
