# context-chronological-recall-and-hygiene Specification

## Purpose
TBD - created by archiving change context-chronological-recall-and-hygiene. Update Purpose after archive.

## Requirements

### Requirement: Chronological `[RECALLED]` rendering and `[PLACE_TAIL]` deduplication
`render_recalled` in `crates/reason/src/render.rs` (used by `render_context_plan_with`) SHALL skip any recalled segment whose `event_id` already appears in `sections.place_tail` and SHALL emit the remaining `[RECALLED]` segments sorted chronologically ascending by `(timestamp_ms, event_id.as_u64())`, while Core recall selection continues ranking top-K candidates under unchanged `VisibleScope` isolation.

#### Scenario: Out-of-order and overlapping recalled items render chronologically without duplicating place_tail
- **WHEN** `render_context_plan_with` renders a `ContextPlan` whose `recalled` segments are out of chronological order and include an `event_id` also present in `place_tail`
- **THEN** the overlapping `event_id` is omitted from `[RECALLED]` (remaining in `[PLACE_TAIL]`) and the remaining `[RECALLED]` segments appear in ascending `(timestamp_ms, event_id)` order

### Requirement: `act.people` strips exact `places` entries while preserving legitimate actor IDs
`render_slot_0_anchor` in `crates/reason/src/render.rs` SHALL clarify in `output_contract` that `places` takes room/place IDs and `people` takes actor IDs (`[]` for the whole room, never a place ID). Both `parse_single_act` in `crates/reason/src/parse.rs` and `execute_turn` in `crates/core/src/turn/step.rs` SHALL strip any `act.people` entry matching an `act.places` entry (case-sensitive), keeping all other actor IDs and `VisibleScope`/`Valve` enforcement intact.

#### Scenario: Place ID in act.people is stripped while actor IDs are preserved
- **WHEN** a `ThoughtAct::Act` has `places: ["room_1"]` and `people: ["room_1"]` or `people: ["alice", "room_1"]`, or `people: ["unknown_actor"]`
- **THEN** `people` normalizes to `[]` for `["room_1"]`, `["alice"]` for `["alice", "room_1"]`, and leaves `["unknown_actor"]` intact

### Requirement: Concise direct-answer and matching-only list formatting in `[ANCHOR]`
`render_slot_0_anchor` in `crates/reason/src/render.rs` SHALL instruct in `output_contract` to keep direct factual answers concise and, when asked to list matching names/items, to output only the matching names/items without mentioning excluded ones.

#### Scenario: Rendered `[ANCHOR]` includes concise direct-answer and matching-only list guidance
- **WHEN** `render_context_plan` renders a `ContextPlan`
- **THEN** the `[ANCHOR]` `output_contract` contains the concise direct factual answer and matching-only list instructions

### Requirement: ProactiveBench scorer normalizes SILENT speak payloads and records outcomes
`ProactiveBenchScorer::score` in `crates/lab/src/datasets/proactive_bench.rs` SHALL treat `pred.spoke == true` with trimmed uppercase `response_text` starting with `"SILENT"` as `predicted_spoke = false` and SHALL record a `ScorerItemOutcome` (`metric_name: "proactive_trigger"`, `score: Some(if hit { 1.0 } else { 0.0 })`, `passed: Some(hit)`, `category: entry.category`) via `judge.record_outcome(...)` for every scored prediction.

#### Scenario: chat.speak("SILENT") on proactive_bench is scored as silent and recorded
- **WHEN** `ProactiveBenchScorer::score` evaluates `chat.speak("SILENT")` on both `need = true` and `need = false` gold entries
- **THEN** `need = true` counts as a missed need (`f1 = 0.0`, `passed = Some(false)`), `need = false` counts as a true negative (`false_detection = 0.0`, `passed = Some(true)`), and `ScorerDetailRecorder` records `proactive_trigger` outcomes for both

### Requirement: Fantom scorer records per-item `BeliefQ[Dist.]` outcomes from `fantom_eval.py`
When `fantom_eval.py` returns results (`Ok(Some(val))`), `FantomScorer::score` in `crates/lab/src/datasets/fantom.rs` SHALL record a `ScorerItemOutcome` (`metric_name: "fantom_all_star"`, `category: Some("BeliefQ[Dist.]")`, `score: Some(if ok { 1.0 } else { 0.0 })`, `passed: Some(ok)`) via `judge.record_outcome(...)` for each `BeliefQ[Dist.]` item mapped back to its `(item_id, probe_id, cluster_id)`.

#### Scenario: `BeliefQ[Dist.]` results from `fantom_eval.py` are recorded in `ScorerDetailRecorder`
- **WHEN** `FantomScorer` records `fantom_eval.py` `results` for `BeliefQ[Dist.]` probes (including multiple probes sharing an `item_id`)
- **THEN** `ScorerDetailRecorder` contains one `fantom_all_star` outcome per `BeliefQ[Dist.]` probe with matching `(item_id, probe_id, cluster_id)`, `category = Some("BeliefQ[Dist.]")`, `score`, and `passed`
