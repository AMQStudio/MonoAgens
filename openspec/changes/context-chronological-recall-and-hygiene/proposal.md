# Proposal

## Why

Context assembly/rendering and evaluation scoring surfaced four general architectural and hygiene issues:
1. `[RECALLED]` segments are currently rendered in relevance-rank order and can overlap with `[PLACE_TAIL]` when plans carry shared `event_id`s, scrambling temporal narrative coherence for downstream reasoning.
2. Models occasionally place a `place_id` (such as `"room_1"`) into `ThoughtAct::Act.people`, causing downstream actor-targeted expectations/receipts or routing checks to mismatch instead of treating room-wide replies cleanly (`people: []`).
3. `[ANCHOR]` `output_contract` lacks explicit guidance on keeping direct factual answers concise and omitting excluded items when listing matching names/items.
4. `ProactiveBenchScorer` only inspects `pred.spoke` (misclassifying `chat.speak("SILENT")` as a proactive intervention) and never calls `judge.record_outcome(...)`, while `FantomScorer` fails to call `judge.record_outcome(...)` for `BeliefQ[Dist.]` items scored by `fantom_eval.py`, causing `CONVERSATION_SAMPLES.md` to fall back to `heuristic_match`.

## What Changes

- Update `render_recalled` in `crates/reason/src/render.rs` (invoked by `render_context_plan_with`) to skip any recalled segment whose `event_id` is already present in `sections.place_tail` and sort remaining `[RECALLED]` segments chronologically ascending by `(timestamp_ms, event_id.as_u64())`, while keeping Core's relevance-ranked selection and `enforce_context_budget` tail eviction unchanged.
- Update `render_slot_0_anchor` in `crates/reason/src/render.rs` to clarify in `output_contract` that `places` takes room/place IDs whereas `people` is strictly for actor IDs (`[]` for the whole room, never a `place_id`), and to keep direct factual answers concise and list only matching names/items without mentioning excluded ones.
- Normalize `ThoughtAct::Act.people` in both `crates/reason/src/parse.rs` (`parse_single_act`) and `crates/core/src/turn/step.rs` (`execute_turn`) by stripping any entry matching an entry in `places` (exact case-sensitive match), preserving all legitimate actor IDs and existing `VisibleScope`/`Valve` enforcement.
- Update `ProactiveBenchScorer::score` in `crates/lab/src/datasets/proactive_bench.rs` to treat trimmed uppercase `response_text` starting with `"SILENT"` as `predicted_spoke = false` and emit `ScorerItemOutcome` (`metric_name: "proactive_trigger"`) for every prediction via `judge.record_outcome(...)`.
- Update `FantomScorer::score` in `crates/lab/src/datasets/fantom.rs` to emit `ScorerItemOutcome` (`metric_name: "fantom_all_star"`, `category: Some("BeliefQ[Dist.]")`) via `judge.record_outcome(...)` for each `BeliefQ[Dist.]` item returned by `fantom_eval.py`, mapping back cleanly to `(item_id, probe_id, cluster_id)`.

## Capabilities

### New Capabilities
- `context-chronological-recall-and-hygiene`: Chronological deduplicated `[RECALLED]` rendering, `place_id`-in-`people` normalization and `[ANCHOR]` contract clarity, concise direct-answer/list formatting instructions, and `proactive_bench` / `fantom` `BeliefQ[Dist.]` outcome recording.

### Modified Capabilities
- *(none)*

## Impact

- `crates/reason/src/render.rs`, `crates/reason/src/parse.rs`, `crates/core/src/turn/step.rs`, `crates/lab/src/datasets/proactive_bench.rs`, `crates/lab/src/datasets/fantom.rs`, and tests in `crates/reason`, `crates/core`, and `crates/lab`.
- Preserves `VisibleScope` isolation, `Valve` enforcement, `ARCHITECTURE.md`, and `lab/scenarios/sealed/`.
