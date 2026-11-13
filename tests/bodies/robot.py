"""The robot body: a room, people coming and going, speech, a gesture (contract.md §8)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from monoagens.contract import Action, Percept, Receipt
from tests.bodies.base import T0, FakeBody


class RobotBody(FakeBody):
    """A body in a room, with a body of its own and the people it can see."""

    name = "robot"
    actor = "robot:1"
    section = (
        "You are a robot in a room named robot:room:<name>. speak says something out loud, "
        "move walks to another room, gesture makes one gesture. You see the people in the "
        "room; when you are not sure who is there, say so instead of guessing."
    )
    declared = (
        {
            "name": "speak",
            "description": "Say something out loud in the room.",
            "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                       "required": ["text"]},
            "facts": (),
            "communicative": True,
            "long_running": False,
            "identity_changing": False,
            "permission": "robot.speak",
        },
        {
            "name": "move",
            "description": "Walk to another room.",
            "schema": {"type": "object", "properties": {"to": {"type": "string"}},
                       "required": ["to"]},
            "facts": ("changes",),
            "communicative": False,
            "long_running": True,
            "identity_changing": False,
            "permission": "robot.move",
        },
        {
            "name": "gesture",
            "description": "Make one gesture with an arm.",
            "schema": {"type": "object", "properties": {"name": {"type": "string"}},
                       "required": ["name"]},
            "facts": ("changes",),
            "communicative": False,
            "long_running": False,
            "identity_changing": False,
            "permission": "robot.gesture",
        },
    )

    def __init__(self) -> None:
        """Start in the kitchen with nobody seen."""
        super().__init__()
        self.spoken: list[tuple[str, str]] = []
        self.gestures: list[tuple[str, str]] = []
        self.moved: list[str] = []

    def carry(self, action: Action) -> Receipt:
        """Speak, walk or gesture; a room this body is not in is refused."""
        if action.place not in self.members:
            return Receipt("refused", reason=f"not in {action.place}")
        if action.capability == "speak":
            self.spoken.append((action.place, str(action.arguments.get("text") or "")))
            return Receipt("done")
        if action.capability == "move":
            target = str(action.arguments.get("to") or "")
            self.moved.append(target)
            return Receipt("done", result={"to": target})
        if action.capability == "gesture":
            self.gestures.append((action.place, str(action.arguments.get("name") or "")))
            return Receipt("done")
        return Receipt("refused", reason=f"{action.capability} is not a robot capability")

    def plan(self) -> Sequence[Action]:
        """Return one spoken line and one gesture in the kitchen."""
        place = "robot:room:kitchen"
        return (Action(id="robot:1", capability="speak", place=place,
                       arguments={"text": "selamat pagi"}),
                Action(id="robot:2", capability="gesture", place=place,
                       arguments={"name": "wave"}))

    async def enter(self, room: str, person: str, *, at: datetime = T0) -> None:
        """See one person walk into a room, reporting who is there."""
        seen = self.members.get(room, set()) | {person}
        await self.announce(room, seen, at=at)
        await self.emit(Percept(actor=person, place=room, occurred_at=at,
                                description=f"{person} walked into {room}",
                                features={"addressed": 1}))

    async def leave(self, room: str, person: str) -> None:
        """See one person walk out of a room, reporting who is there now."""
        await self.announce(room, self.members.get(room, set()) - {person})

    async def cannot_tell(self, room: str) -> None:
        """Report a room whose audience this body cannot know."""
        await self.announce(room, self.members.get(room, set()), unknown=True)

    async def demo(self) -> None:
        """See one person enter the kitchen and one room this body cannot judge."""
        await self.enter("robot:room:kitchen", "person:budi")
        await self.cannot_tell("robot:room:hall")
