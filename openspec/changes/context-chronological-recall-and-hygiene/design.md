# Design

## Architecture & Decisions

1. **Chronological `[RECALLED]` & `[PLACE_TAIL]` Deduplication (`crates/reason/src/render.rs`)**:
   - Keep Core's recall ranking (`run_recall_with_model`) and `assemble_full_context_plan` / `enforce_context_budget` unchanged so top-K selection by activation/hybrid similarity and budget eviction of lowest-ranked tail items remain intact without touching `VisibleScope` isolation.
   - Inside `render_recalled` (called by `render_context_plan_with`), collect `place_tail` `EventId`s into a `BTreeSet<EventId>`, filter out any recalled segment whose `event_id` is in `place_tail`, and sort remaining segments chronologically ascending by `(timestamp_ms.map_or(u64::MAX, Time::as_millis), event_id.map_or(u64::MAX, EventId::as_u64))`. Emit `- none` if no recalled items remain after deduplication.

2. **Normalize `place_id` Out of `act.people` (`crates/reason/src/parse.rs`, `crates/core/src/turn/step.rs`)**:
   - Clarify in `[ANCHOR]` `output_contract` (`render_slot_0_anchor`) that `places` holds room/place IDs (e.g. `[trigger place]`), whereas `people` is strictly for actor IDs (`[]` to reply to the whole room; never put a place ID into `people`).
   - In `parse_single_act` (`"act"` arm) and in `execute_turn` (`ThoughtAct::Act` arm), retain only entries in `people` where `!places.iter().any(|pl| pl.as_str() == person.as_str())`. Using both parse-time and Core-step normalization protects both parsed model outputs and direct `Input::ModelResult` callers while keeping legitimate/unknown actor IDs and `VisibleScope`/`Valve` checks intact.

3. **Concise Direct-Answer & List Formatting in `[ANCHOR]` (`crates/reason/src/render.rs`)**:
   - Extend `output_contract` in `render_slot_0_anchor` to instruct models to keep direct factual answers concise, and when asked to list matching names/items, output only the matching names/items without mentioning excluded ones.

4. **`proactive_bench` `"SILENT"` Normalization & `fantom` `BeliefQ[Dist.]` Outcomes (`crates/lab/src/datasets/proactive_bench.rs`, `crates/lab/src/datasets/fantom.rs`)**:
   - In `ProactiveBenchScorer::score`, compute `predicted_spoke = pred.spoke && !pred.response_text.as_deref().unwrap_or("").trim().to_ascii_uppercase().starts_with("SILENT")`, update `(cd, fd, mn, nr)` and `rm_payload` using `predicted_spoke`, and record `ScorerItemOutcome` (`metric_name: "proactive_trigger"`, `score: Some(if hit { 1.0 } else { 0.0 })`, `passed: Some(hit)`, `category: entry.category.clone()`) via `judge.record_outcome(...)`.
   - In `FantomScorer::score`, maintain `st_meta: Vec<(String, String, String)>` alongside `st_payload` carrying `(item_id, probe_id, cluster_id)`. When `fantom_eval.py` returns `Ok(Some(val))`, map each result entry back to its `(item_id, probe_id, cluster_id)` (matching positional index verified by `item_id`, with `item_id` fallback) and call `judge.record_outcome(...)` with `metric_name: "fantom_all_star"`, `category: Some("BeliefQ[Dist.]".to_owned())`, `score: Some(if ok { 1.0 } else { 0.0 })`, and `passed: Some(ok)`.
