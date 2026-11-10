"""Seeded randomness: a decision is random to the user, reproducible in replay."""

from __future__ import annotations

import hashlib
import random


def seeded_rng(seed: int | str, event_id: int | None) -> random.Random:
    """Return a generator seeded by the soul's seed and one event id."""
    key = f"{seed}:{event_id or 0}".encode("utf-8")
    seed_val = int.from_bytes(hashlib.sha256(key).digest()[:8], "big")
    return random.Random(seed_val)
