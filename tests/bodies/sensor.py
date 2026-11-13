"""The sensor body: no actor, a value changing past a threshold, one read (contract.md §8)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from monoagens.contract import UNKNOWN, Action, Percept, Receipt
from tests.bodies.base import T0, FakeBody

KITCHEN = "sensor:kitchen"


class SensorBody(FakeBody):
    """A thermometer: nobody is in its audience and no person acts on it."""

    name = "sensor"
    actor = UNKNOWN
    section = (
        "You are a sensor in sensor:<name>. read returns the value it last measured. "
        "Nobody can see what happens here besides the owner; the world is the actor of "
        "every reading."
    )
    declared = (
        {
            "name": "read",
            "description": "Read the sensor's last value.",
            "schema": {"type": "object", "properties": {"what": {"type": "string"}}},
            "facts": ("reads",),
            "communicative": False,
            "long_running": False,
            "identity_changing": False,
            "permission": "sensor.read",
        },
    )

    def __init__(self) -> None:
        """Start with no value measured."""
        super().__init__()
        self.values: dict[str, float] = {}
        self.reads: list[tuple[str, str]] = []

    def carry(self, action: Action) -> Receipt:
        """Return the last value of the device this action names."""
        if action.capability != "read":
            return Receipt("refused", reason=f"{action.capability} is not a sensor capability")
        what = str(action.arguments.get("what") or action.place)
        value = self.values.get(action.place)
        if value is None:
            return Receipt("refused", reason=f"no value measured at {action.place}")
        self.reads.append((action.place, what))
        return Receipt("done", result={"value": value})

    def plan(self) -> Sequence[Action]:
        """Return one read of the kitchen thermometer."""
        return (Action(id="sensor:1", capability="read", place=KITCHEN,
                       arguments={"what": "temperature"}),)

    async def measure(self, device: str, value: float, *, at: datetime = T0) -> None:
        """Sense one measurement and report the place's (empty) audience first."""
        if device not in self.members:
            await self.announce(device, [], at=at)
        self.values[device] = value
        await self.emit(Percept(actor=UNKNOWN, place=device, occurred_at=at,
                                description=f"{device} at {value:g} °C"))

    async def demo(self) -> None:
        """Sense one measurement in the kitchen."""
        await self.measure(KITCHEN, 31.0)
