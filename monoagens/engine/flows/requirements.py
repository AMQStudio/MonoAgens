"""Requirement kinds: what a flow needs before it can continue (ADR-0013 §4.2).

The core knows only the four kinds below. A new kind is an ADR: adding one enlarges what the
core promises to understand and what every gateway must be able to render.

Each kind round-trips through JSON exactly, because the awaiting requirement is stored in the
``flows.awaiting`` column and read back by a different process after a restart. `from_json`
fails closed: an unknown or malformed value raises `ValueError` and the caller treats the
flow as unreadable instead of guessing.
"""

from dataclasses import dataclass
from typing import ClassVar


def _object(data: object) -> dict:
    if not isinstance(data, dict):
        raise ValueError("a requirement must be a JSON object")
    return data


def _text(data: dict, key: str, *, allow_empty: bool = True) -> str:
    value = data.get(key)
    if not isinstance(value, str):
        raise ValueError(f"requirement field '{key}' must be a string")
    if not allow_empty and not value.strip():
        raise ValueError(f"requirement field '{key}' must not be empty")
    return value


def _name(data: dict, key: str) -> str:
    value = _text(data, key, allow_empty=False).strip()
    if not value:
        raise ValueError(f"requirement field '{key}' must not be empty")
    return value


def _strings(data: dict, key: str, *, allow_empty: bool = False) -> tuple[str, ...]:
    value = data.get(key)
    if not isinstance(value, (list, tuple)):
        raise ValueError(f"requirement field '{key}' must be a list of strings")
    found = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            raise ValueError(f"requirement field '{key}' must hold non-empty strings")
        found.append(item)
    if not found and not allow_empty:
        raise ValueError(f"requirement field '{key}' must not be empty")
    return tuple(found)


def _seconds(data: dict, key: str) -> float | None:
    value = data.get(key)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"requirement field '{key}' must be a number or null")
    return float(value)


class Requirement:
    """One requirement kind; subclasses declare `kind` and carry only their own fields."""

    kind: ClassVar[str] = ""

    def to_json(self) -> dict:  # pragma: no cover - every subclass overrides it
        raise NotImplementedError

    @classmethod
    def from_json(cls, data: object) -> Requirement:
        """Parse any requirement from its stored JSON; the subclass is the expected kind."""
        found = from_json(data)
        if cls is not Requirement and not isinstance(found, cls):
            raise ValueError(f"expected a '{cls.kind}' requirement, got '{found.kind}'")
        return found


@dataclass(frozen=True, slots=True)
class Approval(Requirement):
    """Consent of specific people; `deciders` are person ids and `action` is what they allow.

    `expires_in` overrides the flow's own expiry for this requirement alone.
    """

    deciders: tuple[str, ...]
    action: str
    expires_in: float | None = None
    kind: ClassVar[str] = "approval"

    def to_json(self) -> dict:
        return {
            "kind": self.kind,
            "deciders": list(self.deciders),
            "action": self.action,
            "expires_in": self.expires_in,
        }


@dataclass(frozen=True, slots=True)
class Connect(Requirement):
    """The person links an external account; the account provider reports when it is done."""

    account_kind: str
    instruction: str = ""
    kind: ClassVar[str] = "connect"

    def to_json(self) -> dict:
        return {
            "kind": self.kind,
            "account_kind": self.account_kind,
            "instruction": self.instruction,
        }


@dataclass(frozen=True, slots=True)
class ExternalAction(Requirement):
    """The person acts outside the engine; the flow's `verify` step decides when it is done."""

    instruction: str
    verify: str
    kind: ClassVar[str] = "external"

    def to_json(self) -> dict:
        return {
            "kind": self.kind,
            "instruction": self.instruction,
            "verify": self.verify,
        }


@dataclass(frozen=True, slots=True)
class OperatorConfig(Requirement):
    """A secret or setting is missing; the capability stays unavailable until it exists."""

    keys: tuple[str, ...]
    kind: ClassVar[str] = "config"

    def to_json(self) -> dict:
        return {"kind": self.kind, "keys": list(self.keys)}


KINDS: dict[str, type[Requirement]] = {
    Approval.kind: Approval,
    Connect.kind: Connect,
    ExternalAction.kind: ExternalAction,
    OperatorConfig.kind: OperatorConfig,
}


def from_json(data: object) -> Requirement:
    """Rebuild one stored requirement; an unknown kind is never guessed (fail closed)."""
    body = _object(data)
    kind = body.get("kind")
    if not isinstance(kind, str) or kind not in KINDS:
        raise ValueError(f"unknown requirement kind {kind!r}")
    if kind == Approval.kind:
        return Approval(
            deciders=_strings(body, "deciders"),
            action=_text(body, "action", allow_empty=False),
            expires_in=_seconds(body, "expires_in"),
        )
    if kind == Connect.kind:
        return Connect(
            account_kind=_name(body, "account_kind"),
            instruction=_text(body, "instruction"),
        )
    if kind == ExternalAction.kind:
        return ExternalAction(
            instruction=_text(body, "instruction", allow_empty=False),
            verify=_name(body, "verify"),
        )
    return OperatorConfig(keys=_strings(body, "keys"))
