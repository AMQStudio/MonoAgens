"""The modes of the soul's speech: one rule, one bounded knob offset, three examples.

A mode is data, never a second persona: the rule sits in the volatile `<soul_tuning>` block
and the offsets are recorded as `disposition.updated`, so the middleware (not this plugin)
computes the effective knobs (middleware.md §2, lever 6).
"""

from __future__ import annotations

import pathlib
from collections.abc import Mapping
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
        found[mode] = _pairs(mode, loaded[mode])
    return found


def _pairs(mode: str, raw: Any) -> list[tuple[str, str]]:
    """Validate one mode's exchanges the same way for the file and for the owner's option."""
    try:
        pairs = [(str(speaker), str(text)) for speaker, text in raw]
    except (TypeError, ValueError):
        raise ValueError(f"soul-tuning: {mode!r} exchanges must be pairs of speaker and text") \
            from None
    if len(pairs) > MAX_EXCHANGES * 2:
        raise ValueError(f"soul-tuning: more than {MAX_EXCHANGES} {mode!r} exchanges")
    return pairs


def from_options(options: Mapping[str, Any],
                 base: Mapping[str, list[tuple[str, str]]] | None = None
                 ) -> dict[str, list[tuple[str, str]]]:
    """Return the file's examples with the owner's own exchanges on top, validated alike.

    `config.d/soul-tuning.yaml` may carry `options.examples` as the same map the file holds
    (`{daily: [[person, "..."], ...]}`); a mode it names replaces that mode, the rest stay
    the file's, and an unknown mode or a bad pair fails at boot.
    """
    found = {mode: list(pairs) for mode, pairs in (base or EXAMPLES).items()}
    given = (options or {}).get("examples") or {}
    if not isinstance(given, Mapping):
        raise ValueError("soul-tuning: options.examples must be a map of mode to exchanges")
    for mode, raw in given.items():
        if str(mode) not in MODES:
            raise ValueError(f"soul-tuning: examples for unknown mode {mode!r}")
        found[str(mode)] = _pairs(str(mode), raw)
    return found


EXAMPLES = examples()
