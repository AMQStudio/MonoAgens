"""What one flow step may see: read-only data plus the two helpers it is allowed to use.

A step gets no database handle and no gateway handle (ADR-0013 §4.3). It reads its own state,
asks whether an account is connected, and returns an outcome; the runner owns persistence,
delivery, and the lease.
"""

from dataclasses import dataclass

from .accounts import AccountProviders


@dataclass(frozen=True, slots=True)
class FlowContext:
    """One step's inputs: who asked, where, which state, and the account registry."""

    flow_id: int
    requester: str
    thread_id: str
    gateway: str
    state: dict
    now: float
    direct: bool
    accounts: AccountProviders

    def connected(self, kind: str) -> bool:
        """Whether the requester linked that account kind (fail closed, never raises)."""
        return self.accounts.connected(kind, self.requester)
