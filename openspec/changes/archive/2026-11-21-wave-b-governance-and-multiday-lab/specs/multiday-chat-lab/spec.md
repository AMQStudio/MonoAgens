# Spec Delta — multiday-chat-lab

## Purpose

Defines multi-day longitudinal chat evaluation metrics, stateful continuous-timeline benchmark execution for `pm_bench`, and six offline multi-day, adversarial, and multi-agent Lab scenarios (`dev-09..13`, `val-03`) executed at `$0` API cost without touching `lab/scenarios/sealed/`.

## ADDED Requirements

### Requirement: Multi-checkpoint trajectory calibration and proactive/governance Lab metrics
`crates/lab` SHALL compute multi-checkpoint `trajectory_checkpoint_ece` (with early and late signed calibration errors while preserving `tc_ece`) alongside proactive timeline, abstention, and HITL governance metrics (`stale_replan_leak_rate`, `self_repair_patience_rate`, `annoyance_index`, `warning_over_abstention_delta`, `paired_abstention_accuracy`, and HITL fatigue/spoof counters).

#### Scenario: Multi-day trajectory report exposes early vs late calibration drift
- **WHEN** `crates/lab` evaluates a multi-day trajectory with labeled gate decisions across multiple checkpoints
- **THEN** the metrics report populates `trajectory_checkpoint_ece`, `early_signed_cal_error`, and `late_signed_cal_error` alongside existing `ece`, `tc_ece`, `brier`, and `aurc`

### Requirement: Stateful continuous timeline mode (`HistoryMode::LiveSequential`) for `pm_bench`
The `pm_bench` dataset converter and runner SHALL support an opt-in continuous stateful mode (`HistoryMode::LiveSequential`) that replays 7-day Virtual Week timelines sequentially without resetting `ExpectationBook` or per-place `h_t` between checkpoints, while preserving existing `HistoryMode::StaticSilent` compatibility.

#### Scenario: LiveSequential pm_bench preserves expectations across days
- **WHEN** `pm_bench` runs in `HistoryMode::LiveSequential` over a multi-day item sequence
- **THEN** Core's `ExpectationBook` and `place_memory` persist continuously across turns within the timeline rather than resetting per probe

### Requirement: Six offline multi-day, abstention, HITL, and multi-agent Lab scenarios
The Lab `development` and `validation` suites SHALL include six deterministic, `$0` offline scenarios exercising Wave B capabilities without touching `lab/scenarios/sealed/`: `dev-09-multiday-replan-and-cancel`, `dev-10-multiparty-self-repair`, `dev-11-warning-over-abstention-dos`, `dev-12-approval-fatigue-and-ihitl`, `dev-13-multi-agent-pingpong-and-chorus`, and `val-03-7day-trajectory-calibration`.

#### Scenario: All six new Wave B scenarios load and pass under deterministic offline suite execution
- **WHEN** `monoagens-lab suite --set development` and `monoagens-lab suite --set validation` run offline across seeds `1..5`
- **THEN** all six new scenarios execute deterministically with zero API calls, validate their structural/governance/multi-bot/calibration assertions, and never access `lab/scenarios/sealed/`
