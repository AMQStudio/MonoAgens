"""Identity: born from the birth certificate, then kept in `state: identity`."""

from __future__ import annotations

from datetime import datetime
from typing import Any, Mapping

from monoagens.store import append_event, project_event, read_state

KEY = "identity"
BORN = "identity.born"
UPDATED = "identity.updated"
FIELDS = ("name", "language", "persona", "principles")
CAPABILITIES = (
    {"name": "set_identity",
     "description": "Change who the soul is: name, language, persona or principles. "
                    "Only the owner may invoke it.",
     "schema": {"type": "object", "additionalProperties": False, "properties": {
         "name": {"type": "string", "description": "The new name."},
         "language": {"type": "string", "description": "The language it speaks with the owner."},
         "persona": {"type": "string", "description": "The persona, in any language."},
         "principles": {"type": "array", "items": {"type": "string"},
                        "description": "The principles it must never break."}}},
     "communicative": False, "long_running": False, "identity_changing": True},
)


def from_certificate(certificate: Mapping[str, Any]) -> dict[str, Any]:
    """Return the identity a birth certificate describes."""
    identity = {field: certificate.get(field) for field in FIELDS}
    identity["principles"] = [str(item) for item in certificate.get("principles") or []]
    return identity


async def load(conn, certificate: Mapping[str, Any]) -> dict[str, Any]:
    """Return `state: identity`, falling back to the birth certificate before birth."""
    stored = await read_state(conn, KEY)
    return dict(stored) if stored is not None else from_certificate(certificate)


async def ensure(conn, certificate: Mapping[str, Any], *, now: datetime) -> dict[str, Any]:
    """Write the birth certificate into `state: identity` the first time only."""
    stored = await read_state(conn, KEY)
    if stored is not None:
        return dict(stored)
    identity = from_certificate(certificate)
    async with conn.transaction():
        event = await append_event(conn, ts=now, kind=BORN, payload={"identity": identity})
        await project_event(conn, event)
    return identity


def _update(current: Mapping[str, Any], arguments: Mapping[str, Any]) -> dict[str, Any]:
    """Merge one set_identity call into the current identity, rejecting bad fields."""
    unknown = sorted(set(arguments) - set(FIELDS))
    if unknown:
        raise ValueError(f"set_identity: unknown fields {unknown}")
    update = {field: arguments[field] for field in FIELDS if field in arguments}
    for field in ("name", "language", "persona"):
        if field in update and not str(update[field]).strip():
            raise ValueError(f"set_identity: {field} cannot be empty")
    if "principles" in update:
        if not isinstance(update["principles"], list):
            raise ValueError("set_identity: principles must be a list")
        update["principles"] = [str(item) for item in update["principles"]]
    if not update:
        raise ValueError("set_identity: nothing to change")
    return {**current, **update}


class SelfOrgan:
    """The one organ inside the core: it changes the soul's own identity."""

    name = "self"

    def __init__(self, conn, clock) -> None:
        """Store the connection and the injected clock."""
        self.conn, self.clock = conn, clock

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors this organ declares."""
        return CAPABILITIES

    async def act(self, action) -> dict[str, Any]:
        """Apply an owner-approved identity change and record it in the Log."""
        if action.kind != "set_identity":
            raise ValueError(f"Unsupported action kind: {action.kind!r}")
        if action.event_id is None:
            raise ValueError("set_identity needs the act.out event it belongs to")
        current = await read_state(self.conn, KEY) or {}
        identity = _update(current, action.payload)
        async with self.conn.transaction():
            event = await append_event(self.conn, ts=self.clock.now(), kind=UPDATED,
                                       caused_by=action.event_id,
                                       payload={"identity": identity})
            await project_event(self.conn, event)
        return {"identity": identity}
