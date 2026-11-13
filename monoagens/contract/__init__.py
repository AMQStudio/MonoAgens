"""The one contract between the layers (contract.md), as code.

Pure types and validators: no behaviour, no I/O, no clock, no import from any other
`monoagens` package. Its hash is locked in `contract.lock` (contract.md §7).
"""

from __future__ import annotations

from .action import (OUTPUT_MARKER, RECEIPT_STATES, Action, Receipt,
                                       action_id)
from .attachments import UNKNOWN, Attachment, describe_attachments
from .audience import PLACE_AUDIENCE, Audience, audience_event, audience_of
from .capability import (FACTS, FLAGS, KEYWORDS, PERMISSION, TYPES,
                                           descriptor_errors, schema_errors)
from .context import SECTIONS, Context, ContextItem
from .envelope import FEATURES, SCHEMA_VERSION, Event, upcast_event
from .hooks import (OWNER_POLICY, RULE_KINDS, SLOTS, Continue, HookOutcome,
                                      Middleware, Registration, Replace, Stop)
from .lifecycle import LIFECYCLE, Lifecycle, composing, progress, seen, thought
from .percept import (V1_PERCEPT_KEYS, Answer, Command, Percept,
                                        from_legacy_event, to_event, upcast_percept)
from .surfaces import (OPTIONAL_SURFACES, REQUIRED_MEMBERS, Ask, Body, Fetch,
                                         History, Render, missing_members, surfaces)

__all__ = [
    "FACTS", "FEATURES", "FLAGS", "KEYWORDS", "LIFECYCLE", "OPTIONAL_SURFACES", "OUTPUT_MARKER",
    "OWNER_POLICY", "PERMISSION", "PLACE_AUDIENCE", "RECEIPT_STATES", "REQUIRED_MEMBERS",
    "RULE_KINDS", "SCHEMA_VERSION", "SECTIONS", "SLOTS", "TYPES", "UNKNOWN", "V1_PERCEPT_KEYS",
    "Action", "Answer", "Ask", "Attachment", "Audience", "Body", "Command", "Context",
    "ContextItem", "Continue", "Event", "Fetch", "History", "HookOutcome", "Lifecycle",
    "Middleware", "Percept", "Receipt", "Registration", "Render", "Replace", "Stop",
    "action_id", "audience_event", "audience_of", "composing", "describe_attachments",
    "descriptor_errors", "from_legacy_event", "missing_members", "progress", "schema_errors",
    "seen", "surfaces", "thought", "to_event", "upcast_event", "upcast_percept",
]
