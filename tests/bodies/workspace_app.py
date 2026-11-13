"""The workspace app body: a document, collaborators, edits with conflicts (contract.md §8)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from monoagens.contract import Action, Percept, Receipt
from tests.bodies.base import T0, FakeBody

DOC = "workspace:doc:plan"


class WorkspaceBody(FakeBody):
    """A document with collaborators, where two edits over the same text conflict."""

    name = "workspace_app"
    actor = "person:owner"
    section = (
        "You work in a document named workspace:doc:<id>. edit replaces one range of "
        "characters with new text, comment leaves a remark on a range. A range is "
        "{start, end} in characters. An edit that overlaps someone else's unanswered "
        "edit is refused and you have to read the document again."
    )
    declared = (
        {
            "name": "edit",
            "description": "Replace one range of a document with new text.",
            "schema": {
                "type": "object",
                "properties": {
                    "range": {"type": "object",
                              "properties": {"start": {"type": "integer"},
                                             "end": {"type": "integer"}},
                              "required": ["start", "end"]},
                    "text": {"type": "string"},
                },
                "required": ["range", "text"],
            },
            "facts": ("changes",),
            "communicative": False,
            "long_running": False,
            "identity_changing": False,
            "permission": "workspace_app.edit",
        },
        {
            "name": "comment",
            "description": "Leave a remark on one range of a document.",
            "schema": {
                "type": "object",
                "properties": {
                    "range": {"type": "object",
                              "properties": {"start": {"type": "integer"},
                                             "end": {"type": "integer"}},
                              "required": ["start", "end"]},
                    "text": {"type": "string"},
                },
                "required": ["range", "text"],
            },
            "facts": ("changes",),
            "communicative": True,
            "long_running": False,
            "identity_changing": False,
            "permission": "workspace_app.comment",
        },
    )

    def __init__(self) -> None:
        """Start with an empty document and no edit made."""
        super().__init__()
        self.text: dict[str, str] = {}
        self.edits: list[tuple[str, int, int, str]] = []
        self.comments: list[tuple[str, int, int, str]] = []
        self.ranges: dict[str, list[tuple[int, int]]] = {}

    def carry(self, action: Action) -> Receipt:
        """Apply the edit, refusing one that overlaps an unfinished edit."""
        if action.place not in self.members:
            return Receipt("refused", reason=f"no such document: {action.place}")
        span = action.arguments.get("range") or {}
        start, end = int(span.get("start") or 0), int(span.get("end") or 0)
        text = str(action.arguments.get("text") or "")
        if action.capability == "edit":
            if self.conflicts(action.place, start, end):
                return Receipt("refused",
                               reason="the text changed under this edit; read it again")
            self.ranges.setdefault(action.place, []).append((start, end))
            self.edits.append((action.place, start, end, text))
            return Receipt("done", result={"ref": f"{action.place}:{len(self.edits)}"})
        if action.capability == "comment":
            self.comments.append((action.place, start, end, text))
            return Receipt("done", result={"ref": f"{action.place}:comment:"
                                                    f"{len(self.comments)}"})
        return Receipt("refused", reason=f"{action.capability} is not a document capability")

    def conflicts(self, place: str, start: int, end: int) -> bool:
        """Tell whether one range overlaps an edit already made to this document."""
        return any(start < other_end and other_start < end
                   for other_start, other_end in self.ranges.get(place, ()))

    def plan(self) -> Sequence[Action]:
        """Return one edit, a comment, and a second edit over the same characters."""
        return (Action(id="workspace:1", capability="edit", place=DOC,
                       arguments={"range": {"start": 0, "end": 5}, "text": "rencana"}),
                Action(id="workspace:2", capability="comment", place=DOC,
                       arguments={"range": {"start": 6, "end": 9}, "text": "periksa"}),
                Action(id="workspace:3", capability="edit", place=DOC,
                       arguments={"range": {"start": 2, "end": 4}, "text": "x"}))

    async def open(self, doc: str, people: list[str], *, at: datetime = T0) -> None:
        """Open one document with its collaborators, reporting its audience."""
        await self.announce(doc, people, at=at)

    async def changed(self, doc: str, who: str, description: str, *,
                      at: datetime = T0) -> None:
        """Sense one change to a document."""
        await self.emit(Percept(actor=who, place=doc, occurred_at=at,
                                description=description, ref=f"{doc}:{at.isoformat()}"))

    async def demo(self) -> None:
        """Open the plan with one collaborator and sense one change."""
        await self.open(DOC, [self.actor, "person:budi"])
        await self.changed(DOC, "person:budi", "paragraph 3 changed")
