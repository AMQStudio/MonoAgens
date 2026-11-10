"""The outcome protocol: every step and every waiting capability answers with one of three.

ADR-0013 §4.1. `Done` finishes, `Refused` stops with a reason a person may read, and `Needs`
names the requirement to satisfy plus the `Resume` (the next step and the state patch it
needs). Nothing here touches storage or a gateway.
"""

from dataclasses import dataclass, field

from .requirements import Requirement


@dataclass(frozen=True, slots=True)
class Resume:
    """The step to call once the requirement is satisfied, and the state it needs."""

    step: str
    state: dict = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class Done:
    result: str = ""


@dataclass(frozen=True, slots=True)
class Refused:
    reason: str


@dataclass(frozen=True, slots=True)
class Needs:
    requirement: Requirement
    resume: Resume


Outcome = Done | Refused | Needs
