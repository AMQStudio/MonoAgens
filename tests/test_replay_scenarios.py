"""The scenario contract of Part 10: every scenario names its brain function and capability."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

import pytest

ROOT = pathlib.Path(__file__).resolve().parent.parent
_SPEC = importlib.util.spec_from_file_location("replay_script", ROOT / "scripts" / "replay.py")
replay = importlib.util.module_from_spec(_SPEC)
assert _SPEC.loader is not None
sys.modules[_SPEC.name] = replay
_SPEC.loader.exec_module(replay)


def test_every_scenario_declares_its_function_and_capability() -> None:
    paths = sorted((ROOT / "scenarios").rglob("*.yaml"))
    assert paths
    for path in paths:
        scenario = replay.load_scenario(path, ZoneInfo("UTC"))
        assert scenario.function == path.parent.name, path
        assert scenario.capability and "\n" not in scenario.capability, path


def test_a_scenario_without_a_capability_is_rejected(tmp_path: pathlib.Path) -> None:
    folder = tmp_path / "memory"
    folder.mkdir()
    path = folder / "99_missing.yaml"
    path.write_text("name: 99_missing\nfunction: memory\nsteps:\n  - text: hi\n",
                    encoding="utf-8")
    with pytest.raises(ValueError, match="capability"):
        replay.load_scenario(path, ZoneInfo("UTC"))


def test_a_scenario_whose_function_does_not_match_its_folder_is_rejected(
        tmp_path: pathlib.Path) -> None:
    folder = tmp_path / "memory"
    folder.mkdir()
    path = folder / "99_mismatch.yaml"
    path.write_text("name: 99_mismatch\nfunction: silence\ncapability: x\n"
                    "steps:\n  - text: hi\n", encoding="utf-8")
    with pytest.raises(ValueError, match="folder"):
        replay.load_scenario(path, ZoneInfo("UTC"))


def _claims() -> list[replay.Capability]:
    return [replay.Capability(scenario.name, scenario.function, scenario.capability)
            for scenario in (replay.load_scenario(path, ZoneInfo("UTC"))
                             for path in sorted((ROOT / "scenarios").rglob("*.yaml")))]


def test_the_capability_record_covers_every_function_and_capability() -> None:
    claims = _claims()
    when = datetime(2026, 11, 12, 9, 0, tzinfo=timezone.utc)
    text = replay.build_capability_record(claims, [], when)
    assert text.startswith(replay.GENERATED_MARK)
    assert "| Capability | Scenario | Fake result | Real model |" in text
    for function in replay.SCENARIO_FUNCTIONS:
        assert f"\n## {function}\n" in text
    for claim in claims:
        row = f"| {replay._cell(claim.capability)} | {claim.name} |"
        assert text.count(row) == 1, claim.name
        assert text.count(f"| {claim.name} |") == 1, claim.name
    assert "not run" in text
    assert text.count("| Capability |") == sum(
        1 for function in replay.SCENARIO_FUNCTIONS
        if any(claim.function == function for claim in claims))


def test_the_committed_capability_record_covers_every_capability() -> None:
    path = ROOT / "scenarios" / "README.md"
    text = path.read_text(encoding="utf-8")
    assert text.startswith(replay.GENERATED_MARK)
    for claim in _claims():
        assert claim.capability in text, claim.name
        assert claim.name in text, claim.name
    for function in replay.SCENARIO_FUNCTIONS:
        assert f"\n## {function}\n" in text


def test_parse_report_reads_mode_model_and_verdicts(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "replay-20261111-165656.md"
    path.write_text(
        "# Replay report — 2026-11-11 16:56\n\n"
        "- Mode: **real LLM**\n- Model: `some/real-model`\n\n"
        "## Headline metrics\n\n1. **Missed-help rate: 0.0%** — fine.\n\n"
        "- Report generated: 2026-11-11 16:56:56\n\n"
        "## 25_help_place_name\n\n**Expectations: PASS**\n\n"
        "- **PASS** — one: yes\n- **PASS** — two: yes\n\n"
        "## 24_gate_threshold\n\n**Expectations: FAIL**\n\n"
        "- **PASS** — one: yes\n- **FAIL** — two: no\n",
        encoding="utf-8")
    run = replay.parse_report(path)
    assert run is not None
    assert (run.mode, run.model) == ("real", "some/real-model")
    assert run.when == datetime(2026, 11, 11, 16, 56, 56, tzinfo=timezone.utc)
    assert run.results["25_help_place_name"].passed == 2
    assert run.results["25_help_place_name"].total == 2
    assert run.results["24_gate_threshold"].passed == 1
    assert "Headline metrics" not in run.results


def test_a_report_without_scenarios_is_ignored(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "replay-20261111-120000.md"
    path.write_text("# Replay report — 2026-11-11 12:00\n\n- Mode: **fake LLM (free)**\n",
                    encoding="utf-8")
    assert replay.parse_report(path) is None


def _detail_section(events: list[dict]) -> str:
    """Render one scenario section whose single step carries these events."""
    scenario = replay.Scenario(path=pathlib.Path("01_x.yaml"), name="01_x", description="",
                               steps=[], expectations=[], answers={}, zone=ZoneInfo("UTC"))
    at = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
    step = replay.StepRecord(index=4, kind="message", at=at, who="person:owner",
                             place="space:tg:1", text="hi", addressed=True,
                             should=replay.SHOULD_ANSWER, appraisals=[], replies=[],
                             events=events)
    result = replay.ScenarioResult(scenario=scenario, steps=[step], replies=[], prompts=[],
                                   events=events, memories=0, usage=replay.Usage())
    return "\n".join(replay._scenario_section(result, ZoneInfo("UTC")))


def test_the_report_prints_the_payload_text_of_errors_and_reasons(monkeypatch) -> None:
    monkeypatch.setenv("LLM_API_KEY", "sk-live-secret-value")
    long_error = "x" * 400
    text = _detail_section([
        {"kind": "percept", "payload": {"description": "Budi tadi kirim file apa ya?"}},
        {"kind": "error", "payload": {"error": long_error}},
        {"kind": "decision.silent", "payload": {"reason": "gate score below silent threshold"}},
        {"kind": "decision.note", "payload": {"act": "think", "thought": "file belongs to Budi"}},
    ])
    details = text.split("**Step details**\n\n")[1].split("\n\n")[0].splitlines()
    assert len(details) == 3
    assert details[0].startswith("- step 4 — error: xxx")
    assert len(details[0]) == len("- step 4 — error: ") + replay.DETAIL_LIMIT
    assert details[0].endswith("…")
    assert details[1] == "- step 4 — decision.silent: gate score below silent threshold"
    assert details[2] == "- step 4 — decision.note (think): file belongs to Budi"


def test_the_step_details_never_print_a_secret(monkeypatch) -> None:
    monkeypatch.setenv("LLM_API_KEY", "sk-live-secret-value")
    text = _detail_section([
        {"kind": "error", "payload": {"error": "bad key sk-live-secret-value"}},
    ])
    assert "sk-live-secret-value" not in text
    assert "- step 4 — error: bad key ***" in text


def test_the_capability_record_counts_passes_over_runs() -> None:
    claim = replay.Capability("01_group_chitchat_greeting", "silence", "A greeting is ignored.")
    day = datetime(2026, 11, 11, 10, 0, tzinfo=timezone.utc)
    runs = [
        replay.ReportRun(pathlib.Path("a"), day, "real", "m-1",
                         {"01_group_chitchat_greeting": replay.CapabilityVerdict(2, 2)}),
        replay.ReportRun(pathlib.Path("b"), day - timedelta(days=1), "real", "m-1",
                         {"01_group_chitchat_greeting": replay.CapabilityVerdict(1, 2)}),
        replay.ReportRun(pathlib.Path("c"), day, "fake", "scripted",
                         {"01_group_chitchat_greeting": replay.CapabilityVerdict(3, 3)}),
    ]
    text = replay.build_capability_record([claim], runs, day)
    row = next(line for line in text.splitlines() if line.startswith("| A greeting"))
    assert row.split(" | ")[2] == "PASS 1/1 · 2026-11-11"
    assert row.split(" | ")[3].startswith("FAIL 1/2 · m-1 · 2026-11-11")


def test_the_capability_record_keeps_only_its_recent_runs() -> None:
    claim = replay.Capability("01_group_chitchat_greeting", "silence", "A greeting is ignored.")
    day = datetime(2026, 11, 11, 10, 0, tzinfo=timezone.utc)
    runs = [replay.ReportRun(pathlib.Path(str(index)), day - timedelta(days=index), "real",
                             "m", {"01_group_chitchat_greeting": replay.CapabilityVerdict(1, 1)})
            for index in range(replay.RECORD_RUNS + 2)]
    text = replay.build_capability_record([claim], runs, day)
    assert "PASS 3/3" in text
    assert "PASS 4/4" not in text and "PASS 5/5" not in text


def test_write_capability_record_writes_the_given_path(tmp_path: pathlib.Path) -> None:
    target = tmp_path / "scenarios" / "README.md"
    path = replay.write_capability_record("x\n", target)
    assert path == target
    assert target.read_text(encoding="utf-8") == "x\n"


def test_the_report_records_model_and_effort(tmp_path: pathlib.Path) -> None:
    when = datetime(2026, 11, 12, 9, 0, tzinfo=timezone.utc)
    text = replay.render_report([], replay.Usage(), "real LLM", "some/real-model",
                                {"constants": {}, "knobs": {}, "calibration": {}}, when,
                                ZoneInfo("UTC"), "high")
    assert "- Model: `some/real-model`" in text
    assert "- Effort: `high`" in text

    path = tmp_path / "replay-20261112-090000.md"
    path.write_text(text.replace("## Headline metrics",
                                 "## 01_group_chitchat_greeting\n\n- **PASS** — one: yes\n\n"
                                 "## Headline metrics"), encoding="utf-8")
    run = replay.parse_report(path)
    assert run is not None and (run.model, run.effort) == ("some/real-model", "high")


def test_the_capability_record_names_the_effort_of_a_real_run() -> None:
    claim = replay.Capability("01_group_chitchat_greeting", "silence", "A greeting is ignored.")
    day = datetime(2026, 11, 12, 10, 0, tzinfo=timezone.utc)
    runs = [replay.ReportRun(pathlib.Path("a"), day, "real", "stronger/model",
                             {"01_group_chitchat_greeting": replay.CapabilityVerdict(2, 2)},
                             "low")]
    text = replay.build_capability_record([claim], runs, day)
    row = next(line for line in text.splitlines() if line.startswith("| A greeting"))
    assert "stronger/model (effort low)" in row


def test_the_report_prints_what_each_expectation_watches() -> None:
    text = _detail_section([
        {"kind": "expectation.created",
         "payload": {"expectation_id": 7, "kind": "promise",
                     "about": "Budi sends the proposal"},
         "cue_event": {"kind": "percept", "place": "space:tg:1", "text_like": "proposal"},
         "condition": {"at": "2026-11-11T11:00:00+00:00"}},
        {"kind": "expectation.fired",
         "payload": {"expectation_id": 7, "about": "Budi sends the proposal",
                     "text": "proposal sudah dikirim", "reminder": True}},
        {"kind": "expectation.resolved",
         "payload": {"expectation_id": 7, "status": "satisfied"}},
    ])
    details = text.split("**Step details**\n\n")[1].split("\n\n")[0].splitlines()
    assert details[0] == (
        "- step 4 — expectation.created: kind=promise "
        'cue_event={"kind":"percept","place":"space:tg:1","text_like":"proposal"} '
        'condition={"at":"2026-11-11T11:00:00+00:00"} about=Budi sends the proposal')
    assert details[1] == "- step 4 — expectation.fired: proposal sudah dikirim"
    assert details[2] == "- step 4 — expectation.resolved: satisfied"


def test_the_expectation_detail_is_redacted_and_truncated(monkeypatch) -> None:
    monkeypatch.setenv("LLM_API_KEY", "sk-live-secret-value")
    text = _detail_section([
        {"kind": "expectation.created",
         "payload": {"kind": "watch", "about": "x" * 500},
         "cue_event": {"text_like": "key sk-live-secret-value"},
         "condition": {"none": True}},
    ])
    line = text.split("**Step details**\n\n")[1].split("\n\n")[0].splitlines()[0]
    assert "sk-live-secret-value" not in text
    assert 'cue_event={"text_like":"key ***"}' in line
    assert len(line) == len("- step 4 — ") + replay.EXPECTATION_LIMIT
    assert line.endswith("…")
