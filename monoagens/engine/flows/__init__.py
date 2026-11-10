"""Durable flows: outcomes, requirements, definitions, runner, and worker (ADR-0013 §4).

A flow is ordinary Python: each step receives a read-only :class:`FlowContext` and returns
:class:`Done`, :class:`Refused`, or :class:`Needs`. The core knows only four requirement
kinds (``approval``, ``connect``, ``external``, ``config``); a new kind needs an ADR
(ADR-0013 §4.2).

The runner gets its own ``flows`` table instead of reusing ``automations``: those tables are
schedule-specific (event, condition, occurrence key, daily budget) and a flow carries none of
that. What it reuses is the *pattern*: a lease-fenced write through :class:`FencedStore`, a
claim with ``FOR UPDATE SKIP LOCKED`` and a lease deadline, a claim generation column, and a
reap pass for claims whose process died (``jobs/automation_store.py``).
"""

from .accounts import AccountProviders as AccountProviders
from .context import FlowContext as FlowContext
from .definition import Flow as Flow
from .definition import FlowRegistry as FlowRegistry
from .definition import Step as Step
from .outcomes import Done as Done
from .outcomes import Needs as Needs
from .outcomes import Outcome as Outcome
from .outcomes import Refused as Refused
from .outcomes import Resume as Resume
from .requirements import KINDS as REQUIREMENT_KINDS
from .requirements import Approval as Approval
from .requirements import Connect as Connect
from .requirements import ExternalAction as ExternalAction
from .requirements import OperatorConfig as OperatorConfig
from .requirements import Requirement as Requirement
from .requirements import from_json as requirement_from_json

__all__ = [
    "REQUIREMENT_KINDS",
    "AccountProviders",
    "Approval",
    "Connect",
    "Done",
    "ExternalAction",
    "Flow",
    "FlowContext",
    "FlowRegistry",
    "Needs",
    "OperatorConfig",
    "Outcome",
    "Refused",
    "Requirement",
    "Resume",
    "Step",
    "requirement_from_json",
]
