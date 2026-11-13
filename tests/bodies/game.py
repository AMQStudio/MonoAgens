"""The game body: a world, players in range, movement (contract.md §8)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from monoagens.contract import Action, Percept, Receipt
from tests.bodies.base import T0, FakeBody

WORLD = "game:world:shared"


class GameBody(FakeBody):
    """A shared world where the players in range can see what happens."""

    name = "game"
    actor = "game:player:me"
    section = (
        "You play in game:world:<id>. walk moves to another region, build places a "
        "structure. The players in range see what you do; a player who walks away stops "
        "seeing it."
    )
    declared = (
        {
            "name": "walk",
            "description": "Walk to another region of the world.",
            "schema": {"type": "object", "properties": {"to": {"type": "string"}},
                       "required": ["to"]},
            "facts": ("changes",),
            "communicative": False,
            "long_running": False,
            "identity_changing": False,
            "permission": "game.walk",
        },
        {
            "name": "build",
            "description": "Build one structure where you stand.",
            "schema": {"type": "object", "properties": {"name": {"type": "string"}},
                       "required": ["name"]},
            "facts": ("changes",),
            "communicative": False,
            "long_running": True,
            "identity_changing": False,
            "permission": "game.build",
        },
    )

    def __init__(self) -> None:
        """Start standing still with nothing built."""
        super().__init__()
        self.built: list[tuple[str, str]] = []
        self.walked: list[str] = []

    def carry(self, action: Action) -> Receipt:
        """Walk or build; an action for a world this body is not in is refused."""
        if action.place not in self.members:
            return Receipt("refused", reason=f"not in {action.place}")
        if action.capability == "walk":
            self.walked.append(str(action.arguments.get("to") or ""))
            return Receipt("done")
        if action.capability == "build":
            self.built.append((action.place, str(action.arguments.get("name") or "")))
            return Receipt("done", result={"built": len(self.built)})
        return Receipt("refused", reason=f"{action.capability} is not a game capability")

    def plan(self) -> Sequence[Action]:
        """Return one walk and one build in the shared world."""
        return (Action(id="game:1", capability="walk", place=WORLD,
                       arguments={"to": "north"}),
                Action(id="game:2", capability="build", place=WORLD,
                       arguments={"name": "hut"}))

    async def arrived(self, world: str, player: str, *, at: datetime = T0) -> None:
        """See one player come into range, reporting who is in range."""
        in_range = self.members.get(world, set()) | {player}
        await self.announce(world, in_range, at=at)
        await self.emit(Percept(actor=player, place=world, occurred_at=at,
                                description=f"{player} came into range"))

    async def left(self, world: str, player: str) -> None:
        """See one player walk out of range, reporting who is in range now."""
        await self.announce(world, self.members.get(world, set()) - {player})

    async def demo(self) -> None:
        """See one player arrive in the shared world."""
        await self.arrived(WORLD, "person:budi")
