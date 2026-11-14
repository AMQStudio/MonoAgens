"""One typed line becomes one neutral event; the terminal is the owner's direct chat."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.clock import Clock
from monoagens.contract import Answer, Command, Percept, describe_attachments

PLACE = "space:cli"
OWNER = "person:owner"


def match_choice(text: str, last_prompt: tuple[str, list[str]] | None) -> tuple[str, str] | None:
    """Match a numbered answer or option label against the latest terminal choices prompt."""
    if not last_prompt:
        return None
    prompt_id, options = last_prompt
    raw = text.strip()
    if raw.isdigit():
        idx = int(raw)
        if 1 <= idx <= len(options):
            return options[idx - 1], prompt_id
    low = raw.lower()
    for opt in options:
        if opt.strip().lower() == low:
            return opt, prompt_id
    return None


def event(text: str, *, clock: Clock, name: str = "cli",
          attachments: Sequence[Mapping[str, Any]] = (),
          last_prompt: tuple[str, list[str]] | None = None) -> Percept:
    """Build the percept of one typed line (the owner typing in their own terminal) (§2.4.2)."""
    answer = None
    choice_matched = match_choice(text, last_prompt)
    if choice_matched is not None:
        choice_value, question_ref = choice_matched
        answer = Answer(prompt=str(question_ref), value=choice_value)
    summary = ""
    if attachments:
        summary = describe_attachments(attachments, sent_by=OWNER)
    description = f"{text} {summary}".strip() if text else summary
    return Percept(
        actor=OWNER,
        place=PLACE,
        occurred_at=clock.now(),
        description=description,
        ref=str(int(clock.now().timestamp() * 1000)),
        answer=answer,
        attachments=tuple(attachments),
        features={"addressed": 1, "named": 0, "addressed_other": 0},
    )


def command_event(line: str, *, clock: Clock, name: str = "cli") -> Percept | None:
    """Return the percept of one typed command (`/help` or `:help`), or None (§2.4.7)."""
    text = str(line).strip()
    if not text or text[0] not in "/:":
        return None
    parts = text[1:].split()
    if not parts:
        return None
    command = parts[0].partition("@")[0].lower()
    return Percept(
        actor=OWNER,
        place=PLACE,
        occurred_at=clock.now(),
        description=text,
        command=Command(name=command, args=tuple(parts[1:])),
        features={"addressed": 1, "named": 0, "addressed_other": 0},
    )


def split_line(line: str) -> tuple[list[str], str]:
    """Split `@a.txt|@b.txt|please read` into its paths and its text."""
    if not str(line).startswith("@"):
        return [], str(line)
    paths, words = [], []
    for part in str(line).split("|"):
        piece = part.strip()
        if piece.startswith("@"):
            paths.append(piece[1:].strip())
        elif piece:
            words.append(piece)
    return paths, " ".join(words)
