"""Headline replay metrics: order, denominators, and the honesty and line-2 scans."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

from monoagens.cognition import Appraisal
from monoagens.store import append_event

ROOT = pathlib.Path(__file__).resolve().parent.parent
_SPEC = importlib.util.spec_from_file_location("replay_script", ROOT / "scripts" / "replay.py")
replay = importlib.util.module_from_spec(_SPEC)
assert _SPEC.loader is not None
sys.modules[_SPEC.name] = replay
_SPEC.loader.exec_module(replay)

NOW = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
ZONE = ZoneInfo("UTC")


def _step(index: int, *, at: datetime, should: str, woke: bool, replied: bool) -> object:
    """Build one StepRecord with the fields the metrics read."""
    appraisals = [Appraisal(decision="wake" if woke else "silent", reason="test")] if woke else []
    return replay.StepRecord(index=index, kind="message", at=at, who="person:owner",
                             place="space:tg:-1001", text=f"message {index}",
                             addressed=should == replay.SHOULD_ANSWER, should=should,
                             appraisals=appraisals,
                             replies=[("space:tg:-1001", "a reply")] if replied else [],
                             events=[])


def _result(*, name: str, steps: list, replies: list, violations: list[str],
            log_texts: list[str], expectations: list, tokens: int = 0) -> object:
    """Build one ScenarioResult with the fields the metrics read."""
    scenario = replay.Scenario(path=pathlib.Path(name), name=name, description="",
                               steps=[], expectations=expectations, answers={}, zone=ZONE)
    return replay.ScenarioResult(
        scenario=scenario, steps=steps, replies=replies, prompts=[], events=[],
        memories=0, usage=replay.Usage(total=tokens), checks=[],
        violations=violations, log_texts=log_texts)


def test_missed_help_and_false_speak_use_the_declared_ground_truth() -> None:
    steps = [
        _step(1, at=NOW, should=replay.SHOULD_ANSWER, woke=True, replied=True),
        _step(2, at=NOW, should=replay.SHOULD_ANSWER, woke=False, replied=False),
        _step(3, at=NOW, should=replay.SHOULD_SILENT, woke=False, replied=False),
        _step(4, at=NOW, should=replay.SHOULD_SILENT, woke=True, replied=True),
    ]
    replies = [replay.Reply(step.index, NOW, "space:tg:-1001", "a reply")
               for step in steps if step.replies]
    result = _result(name="scenario", steps=steps, replies=replies, violations=[],
                     log_texts=[], expectations=[])
    metrics = replay.compute_metrics([result], replay.Usage(total=1440))
    assert metrics.missed_help_rate() == 0.5
    assert metrics.false_speak_rate() == 0.5
    assert metrics.should_answer == 2 and metrics.answered == 1
    assert metrics.should_silent == 2 and metrics.false_speaks == 1


def test_false_speak_reads_communicative_replies_only() -> None:
    steps = [_step(1, at=NOW, should=replay.SHOULD_SILENT, woke=False, replied=False)]
    spoke = replay.Reply(1, NOW, "space:tg:-1001", "hi")
    result = _result(name="scenario", steps=steps, replies=[spoke], violations=[],
                     log_texts=["hi"], expectations=[])
    metrics = replay.compute_metrics([result], replay.Usage())
    assert metrics.false_speak_rate() == 1.0


def test_tokens_per_simulated_day_sums_the_scenario_spans() -> None:
    steps = [_step(1, at=NOW, should=replay.SHOULD_SILENT, woke=False, replied=False),
             _step(2, at=NOW + timedelta(days=1), should=replay.SHOULD_SILENT,
                   woke=False, replied=False)]
    result = _result(name="scenario", steps=steps, replies=[], violations=[],
                     log_texts=[], expectations=[])
    metrics = replay.compute_metrics([result], replay.Usage(total=2880))
    assert metrics.simulated_days == 1.0
    assert metrics.tokens_per_day() == 2880.0


def test_absolute_line_violations_combine_leaks_and_failed_privacy_checks() -> None:
    failing = replay.CheckResult("file line stays out of the prompt", False, "seen in step 6")
    spec = {"type": "prompt_absent", "text": "sketsa_logo", "label": "file line"}
    passing = {"type": "prompt_absent", "text": "other", "label": "other line"}
    result = _result(name="scenario", steps=[], replies=[], violations=["a reply leaked"],
                     log_texts=[], expectations=[spec, passing])
    result.checks = [failing, replay.CheckResult("other line", True, "none")]
    metrics = replay.compute_metrics([result], replay.Usage())
    assert metrics.violations == [
        "a reply leaked",
        "scenario: file line stays out of the prompt (seen in step 6)",
    ]


def test_honesty_scan_catches_only_quotes_absent_from_the_log() -> None:
    grounded = replay.Reply(
        1, NOW, "space:tg:-1001", 'Budi bilang "Cocok kok, warnanya agak terlalu terang".')
    invented = replay.Reply(2, NOW, "space:tg:-1001", 'Budi bilang "kita batal bertemu besok".')
    result = _result(name="scenario", steps=[], replies=[grounded, invented], violations=[],
                     log_texts=["Cocok kok, warnanya agak terlalu terang."], expectations=[])
    metrics = replay.compute_metrics([result], replay.Usage())
    assert metrics.honesty_compliance() == 0.5


async def test_the_desk_scan_counts_every_private_item_its_audience_does_not_cover(aconn):
    dm, group = "space:tg:owner", "space:tg:-500"
    said = await append_event(aconn, ts=NOW, kind="percept", who="person:owner", place=dm,
                              payload={"description": "rapat jam sembilan", "place_kind": "direct"})
    asked = await append_event(aconn, ts=NOW, kind="percept", who="person:budi", place=group,
                               payload={"description": "rapat jam berapa?", "place_kind": "shared"})
    leaked = replay.Desk(3, group, asked.id, (replay.DeskItem(dm, None),))
    assert await replay.desk_violations(aconn, [leaked]) == [
        f"step 3: an item from `{dm}` was on the `{group}` desk, "
        "whose audience it does not cover"]
    assert await replay.desk_violations(
        aconn, [replay.Desk(3, dm, said.id, (replay.DeskItem(dm, None),))]) == []

    grant = await append_event(aconn, ts=NOW, kind="share.granted", place=dm,
                               payload={"source_place": dm, "persons": ["person:budi"]})
    assert await replay.desk_violations(aconn, [leaked])
    assert await replay.desk_violations(
        aconn, [replay.Desk(4, group, grant.id, (replay.DeskItem(dm, None),))]) == []


def test_report_metrics_are_ordered_missed_help_first() -> None:
    steps = [_step(1, at=NOW, should=replay.SHOULD_ANSWER, woke=True, replied=True)]
    result = _result(name="scenario", steps=steps, replies=[], violations=[],
                     log_texts=[], expectations=[])
    metrics = replay.compute_metrics([result], replay.Usage(total=10))
    lines = replay.metric_lines(metrics)
    assert [line.split(".")[0] for line in lines] == ["1", "2", "3", "4", "5"]
    names = ["missed-help", "false-speak", "tokens per simulated day",
             "absolute-line violations", "honesty-principle compliance"]
    for line, name in zip(lines, names):
        assert name in line.lower()
    assert metrics.missed_help_rate() == 0.0 and metrics.false_speak_rate() is None
