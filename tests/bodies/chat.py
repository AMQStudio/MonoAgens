"""The chat body: private and shared places, quotes, files (contract.md §8)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from monoagens.contract import (Attachment, Lifecycle, Percept, Receipt, Action,
                               describe_attachments)
from tests.bodies.base import T0, FakeBody

GROUP = "chat:group:1"


class ChatBody(FakeBody):
    """A messenger with one place per person and one per group."""

    name = "chat"
    actor = "person:owner"
    section = (
        "You are in a chat. A private place is named chat:dm:<person> and a group "
        "chat:group:<n>. say writes a message, react adds an emoji to a message. "
        "Messages can quote an earlier message with reply_to, and may carry files."
    )
    declared = (
        {
            "name": "say",
            "description": "Write a message in a chat place.",
            "schema": {
                "type": "object",
                "properties": {"text": {"type": "string"},
                               "attachments": {"type": "array", "items": {"type": "string"}},
                               "reply_to": {"type": "string"}},
                "required": ["text"],
            },
            "facts": (),
            "communicative": True,
            "long_running": False,
            "identity_changing": False,
            "permission": "chat.say",
        },
        {
            "name": "react",
            "description": "Put one emoji on a message.",
            "schema": {
                "type": "object",
                "properties": {"emoji": {"type": "string"}, "message": {"type": "string"}},
                "required": ["emoji", "message"],
            },
            "facts": (),
            "communicative": False,
            "long_running": False,
            "identity_changing": False,
            "permission": "chat.react",
        },
    )

    def __init__(self) -> None:
        """Start with no place seen and no message written."""
        super().__init__()
        self.written: list[tuple[str, str]] = []
        self.reactions: list[tuple[str, str, str]] = []

    def carry(self, action: Action) -> Receipt:
        """Write the message or add the reaction; an unknown place is refused."""
        if action.place not in self.members:
            return Receipt("refused", reason=f"no such place: {action.place}")
        if action.capability == "say":
            text = str(action.arguments.get("text") or "")
            self.written.append((action.place, text))
            return Receipt("done", result={"ref": f"{action.place}:{len(self.written)}"})
        if action.capability == "react":
            self.reactions.append((action.place, str(action.arguments.get("emoji") or ""),
                                   str(action.arguments.get("message") or "")))
            return Receipt("done")
        return Receipt("refused", reason=f"{action.capability} is not a chat capability")

    def plan(self) -> Sequence[Action]:
        """Return one message and one reaction, both in the group place."""
        return (Action(id="chat:1", capability="say", place=GROUP,
                       arguments={"text": "halo"}),
                Action(id="chat:2", capability="react", place=GROUP,
                       arguments={"emoji": "ok", "message": "chat:1"}))

    async def message(self, place: str, actor: str, text: str, *, at: datetime = T0,
                      ref: str = "", about: str = "",
                      features: dict[str, float] | None = None,
                      files: tuple[Attachment, ...] = ()) -> None:
        """Sense one message in one place, reporting the place's audience first."""
        if place not in self.members:
            await self.announce(place, [actor], at=at)
        description = text
        if files:
            description += ". " + describe_attachments(files, sent_by=actor)
        await self.emit(Percept(actor=actor, place=place, occurred_at=at,
                                description=description, ref=ref, about=about,
                                attachments=files, features=features or {}))

    async def join(self, place: str, actor: str, *, at: datetime = T0) -> None:
        """Add one member to a group and report the new audience."""
        known = self.members.get(place, set()) | {actor}
        await self.announce(place, known, at=at)

    async def leave(self, place: str, actor: str) -> None:
        """Remove one member from a group and report the new audience."""
        known = self.members.get(place, set()) - {actor}
        await self.announce(place, known)

    async def demo(self) -> None:
        """Sense one direct message and one group message."""
        await self.join(GROUP, self.actor)
        await self.message("chat:dm:person:budi", "person:budi", "halo",
                           features={"addressed": 1}, ref="chat:m1")
        await self.message(GROUP, "person:budi", "lihat ini",
                           features={"addressed_other": 1}, ref="chat:m2")

    # ---------------------------------------------------------- optional surfaces

    async def render(self, event: Lifecycle) -> None:
        """Show one lifecycle event the way this body would (a line on the screen)."""
        self.rendered.append(event)

    async def ask(self, place: str, question: str, choices: Sequence[str]) -> str:
        """Ask a question with buttons and return its ref."""
        ref = f"chat:q{len(self.asked)}"
        self.asked.append((place, question, tuple(choices)))
        return ref

