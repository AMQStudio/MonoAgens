"""The ratchet: the count of findings per `(file, ban)` may only go down."""

from __future__ import annotations

from tests.architecture.support import ACTIVE, counts, load_baseline, scan


def test_no_new_violations() -> None:
    """A new place that breaks a ban fails here, with its lines."""
    findings = scan()
    baseline = load_baseline()
    lines: dict[tuple[str, str], list[int]] = {}
    for finding in findings:
        lines.setdefault((finding.file, finding.ban), []).append(finding.line)
    worse = []
    for key, value in sorted(counts(findings).items()):
        allowed = baseline.get(key, 0)
        if key[1] not in ACTIVE or value <= allowed:
            continue
        worse.append(f"{key[0]} [{key[1]}]: {value} findings, {allowed} allowed, "
                     f"lines {sorted(lines[key])[allowed:]}")
    assert not worse, "new architecture violations:\n" + "\n".join(worse)


def test_no_stale_baseline() -> None:
    """A listed violation that is gone must leave the baseline: the ratchet only shrinks."""
    current = counts(scan())
    stale = []
    for key, allowed in sorted(load_baseline().items()):
        value = current.get(key, 0)
        if value < allowed:
            stale.append(f"{key[0]} [{key[1]}]: {allowed} listed, {value} found")
    assert not stale, (
        "the ratchet can only shrink: delete the extra lines from the baseline\n"
        + "\n".join(stale))
