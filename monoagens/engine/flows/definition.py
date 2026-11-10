"""Flow definitions and their registry.

A flow is ordinary code, not a DAG or a branching language (ADR-0013 §4.3): a name, a map of
step callables, the first step, and an optional default expiry. Names are ``owner/flow`` so a
plugin's flows can never collide with the core's or another plugin's.
"""

import re
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field

from .context import FlowContext
from .outcomes import Outcome

Step = Callable[[FlowContext], Awaitable[Outcome]]

# `owner/flow`: a lower-case owner, one slash, a lower-case flow name (ADR-0014 §4).
NAME = re.compile(r"^[a-z][a-z0-9_-]{0,63}/[a-z][a-z0-9_-]{0,63}$")


@dataclass(frozen=True, slots=True)
class Flow:
    """One flow: its steps, where it starts, and how long a raised requirement lives."""

    name: str
    steps: dict[str, Step] = field(default_factory=dict)
    first: str = "start"
    default_expiry: float | None = None

    @property
    def owner(self) -> str:
        return self.name.split("/", 1)[0]

    def __post_init__(self) -> None:
        name = str(self.name or "")
        if not NAME.fullmatch(name):
            raise ValueError(f"a flow name must be 'owner/flow', got '{name}'")
        if not self.steps:
            raise ValueError(f"flow '{name}' has no steps")
        if self.first not in self.steps:
            raise ValueError(f"flow '{name}' starts at unknown step '{self.first}'")
        if self.default_expiry is not None and float(self.default_expiry) <= 0:
            raise ValueError(f"flow '{name}' needs a positive default_expiry")


class FlowRegistry:
    """The process's flow table; one flow per name, and a duplicate is refused."""

    def __init__(self) -> None:
        self._flows: dict[str, Flow] = {}

    def register(self, flow: Flow) -> Flow:
        name = str(getattr(flow, "name", "") or "")
        if name in self._flows:
            raise ValueError(f"flow '{name}' is already registered")
        self._flows[name] = flow
        return flow

    def get(self, name: str) -> Flow | None:
        return self._flows.get(str(name or ""))

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._flows))

    def __contains__(self, name: object) -> bool:
        return str(name) in self._flows

    def __len__(self) -> int:
        return len(self._flows)
