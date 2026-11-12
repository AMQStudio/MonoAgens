"""The modes of the soul's speech: one rule, one bounded knob offset, three examples.

A mode is data, never a second persona: the rule sits in the volatile `<soul_tuning>` block
and the offsets are recorded as `disposition.updated`, so the middleware (not this plugin)
computes the effective knobs (middleware.md §2, lever 6).
"""

from __future__ import annotations

import pathlib
from typing import Any

import yaml

MODES: dict[str, dict[str, Any]] = {
    "daily": {
        "rule": "Speak casually, like a friend in the same chat; short lines.",
        "offsets": {"talkativeness": 0.05},
    },
    "professional": {
        "rule": "Speak plainly and professionally; no slang, no filler, complete sentences.",
        "offsets": {"talkativeness": -0.05},
    },
}
NAMES = tuple(MODES)
DEFAULT_MODE = "daily"
EXAMPLES_PATH = pathlib.Path(__file__).with_name("examples.yaml")
MAX_EXCHANGES = 5


def examples(path: pathlib.Path = EXAMPLES_PATH) -> dict[str, list[tuple[str, str]]]:
    """Read the example exchanges; refuse a missing mode or too many exchanges."""
    with open(path, encoding="utf-8") as handle:
        loaded = yaml.safe_load(handle) or {}
    found: dict[str, list[tuple[str, str]]] = {}
    for mode in NAMES:
        if mode not in loaded:
            raise ValueError(f"soul-tuning: examples.yaml has no {mode!r} exchanges")
        pairs = [(str(speaker), str(text)) for speaker, text in loaded[mode]]
        if len(pairs) > MAX_EXCHANGES * 2:
            raise ValueError(f"soul-tuning: more than {MAX_EXCHANGES} {mode!r} exchanges")
        found[mode] = pairs
    return found


EXAMPLES = examples()
