"""Access notice limiter: one refusal per person per window in a direct chat (ADR-0013 §4.4).

The window state lives in memory and resets on restart: a restarted process may send one extra
notice, which is harmless and keeps the gate free of a second store.
"""


import time
from collections.abc import Callable


class AccessNoticeLimiter:
    """Remember the last notice per `(body, person-or-thread)` inside one window."""

    def __init__(
        self,
        hours: float,
        *,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.window = max(0.0, float(hours) * 3600.0)
        self._clock = clock
        self._last: dict[tuple[str, str], float] = {}

    def claim(self, gateway: str, person: str) -> bool:
        """True when a notice is due; a due claim records the time and blocks the window."""
        key = (str(gateway or ""), str(person or ""))
        now = self._clock()
        last = self._last.get(key)
        if last is not None and self.window > 0 and now - last < self.window:
            return False
        self._last[key] = now
        return True
