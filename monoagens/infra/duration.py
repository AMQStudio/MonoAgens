"""Durations: the small grammar shared by cue times and conditions (docs/)."""

from __future__ import annotations

import re
from datetime import timedelta

_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400}
_PATTERN = re.compile(r"^(\d+(?:\.\d+)?)\s*([smhd])$")


def parse_duration(value):
    """Parse a duration like "2d", "6h", "30m", or "45s" into a timedelta."""
    match = _PATTERN.match(str(value).strip().lower())
    if match is None:
        raise ValueError(f"invalid duration: {value!r}")
    return timedelta(seconds=float(match[1]) * _UNITS[match[2]])
