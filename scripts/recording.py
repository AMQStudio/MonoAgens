"""Turn one body's recording into a committed fixture (§4.7).

A recording holds what a body really saw: people's names, their words, their numbers. A fixture
must hold the shape and nothing that identifies anyone, so this tool replaces every id, name and
number with a stable placeholder and every line of text with its length. The same input gives the
same output, so a fixture can be regenerated and diffed.

    uv run python scripts/recording.py anonymize var/recordings/telegram.jsonl \\
        tests/fixtures/recorded/telegram.jsonl
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from typing import Any

ID_KEYS = frozenset({"actor", "about", "ref", "prompt", "place", "who", "body", "person",
                     "chat", "chat_id", "message_id", "message_ids", "caller", "account"})
TEXT_KEYS = frozenset({"description", "text", "question", "value", "name", "title", "label"})
SENSITIVE = re.compile(r"\d{6,}")


class Anonymizer:
    """Stable placeholders: the first id seen becomes `actor:1`, the next `actor:2`, and so on."""

    def __init__(self) -> None:
        """Start with no placeholder assigned."""
        self.names: dict[str, str] = {}

    def token(self, value: str, prefix: str = "id") -> str:
        """Return the one placeholder this value is known by, assigning the next if it is new."""
        if value not in self.names:
            self.names[value] = f"{prefix}:{len(self.names) + 1}"
        return self.names[value]

    def value(self, key: str, value: Any) -> Any:
        """Anonymize one value under the key that names what it is."""
        if isinstance(value, dict):
            return {key: self.value(key, item) for key, item in value.items()}
        if isinstance(value, list):
            return [self.value(key, item) for item in value]
        if isinstance(value, str) and key in TEXT_KEYS:
            return f"<text:{len(value)}>"
        if isinstance(value, str) and (key in ID_KEYS or SENSITIVE.search(value)):
            prefix = "place" if key.startswith("place") else "actor" if key in ID_KEYS else "id"
            return self.token(value, prefix)
        return value


def anonymize_line(line: str, anon: Anonymizer) -> str:
    """Return one recording line with every identity and every word replaced."""
    entry = json.loads(line)
    return json.dumps(anon.value("entry", entry), ensure_ascii=False, sort_keys=True)


def main(argv: list[str]) -> int:
    """Anonymize one recording file into another; refuse anything else."""
    if len(argv) != 4 or argv[1] != "anonymize":
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2
    source, target = pathlib.Path(argv[2]), pathlib.Path(argv[3])
    if not source.is_file():
        print(f"no recording at {source}", file=sys.stderr)
        return 1
    anon = Anonymizer()
    target.parent.mkdir(parents=True, exist_ok=True)
    with source.open(encoding="utf-8") as handle, target.open("w", encoding="utf-8") as out:
        for line in handle:
            if line.strip():
                out.write(anonymize_line(line, anon) + "\n")
    print(f"wrote {target}: {len(anon.names)} identities replaced")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
