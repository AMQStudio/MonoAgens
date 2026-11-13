"""What an adapter senses and puts into the queue (contract.md §4.2)."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field, replace
from datetime import datetime
from typing import Any

from .attachments import UNKNOWN, Attachment
from .envelope import SCHEMA_VERSION, Event

# The version 1 payload keys that carry a percept; a payload with any of them is upcast.
V1_PERCEPT_KEYS = frozenset({"text", "addressed", "sent_at", "place_kind", "attachments",
                             "ref", "about", "involves", "actor"})


@dataclass(frozen=True, slots=True)
class Answer:
    """One structured answer to a question (§4.6)."""

    prompt: str
    value: str


@dataclass(frozen=True, slots=True)
class Command:
    """One operator command a person typed or pressed (§4.7)."""

    name: str
    args: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Percept:
    """One thing a body observed, in the six facts of the contract."""

    actor: str
    place: str
    occurred_at: datetime
    description: str
    ref: str = ""
    about: str = ""
    involves: tuple[str, ...] = ()
    attachments: tuple[Attachment, ...] = ()
    features: Mapping[str, float] = field(default_factory=dict)
    answer: Answer | None = None
    command: Command | None = None
    backfill: bool = False
    imported: bool = False
    data: Mapping[str, Any] = field(default_factory=dict)


def to_event(percept: Percept, *, body: str, ts: datetime, id: int | None = None,
             caused_by: int | None = None, origin_actor: str | None = None) -> Event:
    """Return the `percept` event one percept becomes when it enters the Log."""
    return Event(id=id, ts=ts, kind="percept", occurred_at=percept.occurred_at,
                 description=percept.description, who=percept.actor, place=percept.place,
                 body=body, caused_by=caused_by, origin_actor=origin_actor,
                 features=dict(percept.features), backfill=percept.backfill,
                 imported=percept.imported, payload=_payload(percept))


def upcast_percept(raw: Mapping[str, Any]) -> Percept:
    """Return one version 1 percept payload as the current `Percept`.

    A version 1 producer wrote `who`/`addressed`/`sent_at`/`text`; they are read here, once,
    and never again (contract.md §9). The event's own time is read from `ts` when the payload
    carries none.
    """
    raw = dict(raw or {})
    occurred_at = _as_time(raw.get("occurred_at") or raw.get("sent_at") or raw.get("ts"))
    if occurred_at is None:
        raise ValueError("percept without a time: give `occurred_at`, `sent_at` or `ts`")
    features = dict(raw.get("features") or {})
    if raw.get("addressed") is True:
        features["addressed"] = 1
    involves = raw.get("involves") or ()
    if isinstance(involves, str):
        involves = (involves,)
    return Percept(
        actor=str(raw.get("who") or raw.get("actor") or UNKNOWN),
        place=str(raw.get("place") or ""),
        occurred_at=occurred_at,
        description=str(raw.get("description") or raw.get("text") or ""),
        ref=str(raw.get("ref") or ""),
        about=str(raw.get("about") or ""),
        involves=tuple(str(actor) for actor in involves),
        attachments=_attachments(raw.get("attachments")),
        features=features,
        answer=_answer(raw.get("answer")),
        command=_command(raw.get("command")),
        backfill=bool(raw.get("backfill") or False),
        imported=bool(raw.get("imported") or False),
        data=dict(raw.get("data") or {}),
    )


def from_legacy_event(event: Event) -> Event:
    """Return one version 1 producer's event as a version 2 event (D-M).

    An event that is already version 2, or whose payload carries no percept at all, is
    returned unchanged so an internal bookkeeping event keeps its kind and fields.
    """
    if event.schema_version >= SCHEMA_VERSION:
        return event
    payload = dict(event.payload or {})
    if not V1_PERCEPT_KEYS.intersection(payload):
        return event
    raw = dict(payload)
    raw.setdefault("ts", event.ts)
    raw.setdefault("place", event.place)
    percept = upcast_percept(raw)
    translated = to_event(percept, body=event.body or "", ts=event.ts, id=event.id,
                          caused_by=event.caused_by, origin_actor=event.origin_actor)
    return replace(translated, kind=event.kind)


def _payload(percept: Percept) -> dict[str, Any]:
    """Return the payload one percept is stored with (the only keys middleware reads)."""
    return {
        "data": dict(percept.data),
        "ref": percept.ref,
        "about": percept.about,
        "involves": list(percept.involves),
        "attachments": [{"ref": item.ref, "name": item.name, "mime": item.mime,
                         "size": item.size} for item in percept.attachments],
        "answer": None if percept.answer is None else {"prompt": percept.answer.prompt,
                                                      "value": percept.answer.value},
        "command": None if percept.command is None else {"name": percept.command.name,
                                                         "args": list(percept.command.args)},
    }


def _attachments(value: Any) -> tuple[Attachment, ...]:
    """Return one payload's attachments as contract attachments."""
    found = []
    for item in value or ():
        if isinstance(item, Attachment):
            found.append(item)
            continue
        if isinstance(item, Mapping):
            found.append(Attachment(ref=str(item.get("ref") or ""),
                                    name=str(item.get("name") or ""),
                                    mime=str(item.get("mime") or ""),
                                    size=int(item.get("size") or 0)))
    return tuple(found)


def _answer(value: Any) -> Answer | None:
    """Return one payload's answer, when it has one."""
    if not isinstance(value, Mapping) or "prompt" not in value:
        return None
    return Answer(prompt=str(value.get("prompt") or ""), value=str(value.get("value") or ""))


def _command(value: Any) -> Command | None:
    """Return one payload's command, when it has one."""
    if not isinstance(value, Mapping) or not value.get("name"):
        return None
    args = value.get("args") or ()
    if isinstance(args, str):
        args = (args,)
    return Command(name=str(value["name"]), args=tuple(str(arg) for arg in args))


def _as_time(value: Any) -> datetime | None:
    """Return one stored time as a datetime, or None when it is absent or unreadable."""
    if isinstance(value, datetime):
        return value
    if isinstance(value, str) and value:
        try:
            return datetime.fromisoformat(value)
        except ValueError:
            return None
    return None
