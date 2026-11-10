"""Clock: periodic ticks, fakeable in tests."""
from __future__ import annotations
import asyncio
from datetime import datetime, timedelta, timezone
from monoagens.types import Event

class Clock:
    def __init__(self, tick_seconds: float = 30.0) -> None:
        self.tick_seconds = tick_seconds
    def now(self) -> datetime:
        return datetime.now(timezone.utc)
    async def run(self, put, stop_event=None) -> None:
        while stop_event is None or not stop_event.is_set():
            await asyncio.sleep(self.tick_seconds)
            if stop_event is None or not stop_event.is_set():
                await put(Event(id=None, ts=self.now(), kind="tick"))

class FakeClock(Clock):
    def __init__(self, initial_time=None, tick_seconds: float = 30.0) -> None:
        super().__init__(tick_seconds=tick_seconds)
        self._current_time = initial_time or datetime(2026, 1, 1, tzinfo=timezone.utc)
    def now(self) -> datetime:
        return self._current_time
    def set_time(self, dt) -> None:
        self._current_time = dt
    def advance(self, seconds):
        self._current_time += timedelta(seconds=seconds) if isinstance(seconds, (int, float)) else seconds
        return self._current_time
    async def tick(self, put) -> Event:
        event = Event(id=None, ts=self.now(), kind="tick")
        await put(event)
        return event
