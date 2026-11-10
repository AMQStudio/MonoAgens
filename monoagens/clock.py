"""Clock: periodic ticks, fakeable in tests."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

from monoagens.types import Event


class Clock:
    """The wall clock and the periodic tick source."""

    def __init__(self, tick_seconds: float = 30.0) -> None:
        """Store the tick period."""
        self.tick_seconds = tick_seconds

    def now(self) -> datetime:
        """Return the current time in UTC."""
        return datetime.now(timezone.utc)

    async def run(self, put, stop_event=None) -> None:
        """Put one tick event every tick_seconds until stopped."""
        while stop_event is None or not stop_event.is_set():
            await asyncio.sleep(self.tick_seconds)
            if stop_event is None or not stop_event.is_set():
                await put(Event(id=None, ts=self.now(), kind="tick"))


class FakeClock(Clock):
    """A clock the tests move by hand."""

    def __init__(self, initial_time=None, tick_seconds: float = 30.0) -> None:
        """Start at initial_time (2026-01-01 UTC by default)."""
        super().__init__(tick_seconds=tick_seconds)
        self._current_time = initial_time or datetime(2026, 1, 1, tzinfo=timezone.utc)

    def now(self) -> datetime:
        """Return the frozen current time."""
        return self._current_time

    def set_time(self, dt) -> None:
        """Set the current time."""
        self._current_time = dt

    def advance(self, seconds):
        """Advance by seconds or a timedelta and return the new time."""
        if isinstance(seconds, (int, float)):
            self._current_time += timedelta(seconds=seconds)
        else:
            self._current_time += seconds
        return self._current_time

    async def tick(self, put) -> Event:
        """Build a tick event at the current time, put it, and return it."""
        event = Event(id=None, ts=self.now(), kind="tick")
        await put(event)
        return event
