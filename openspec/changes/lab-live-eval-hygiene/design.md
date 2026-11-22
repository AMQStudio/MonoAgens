# Design

## Architecture & Decisions

1. **`when2speak` SILENT Normalization & Outcome Recording (`crates/lab/src/datasets/when2speak.rs`)**:
   - Determine `predicted_speak`: `false` if `!pred.spoke`; otherwise inspect `pred.response_text`, trim whitespace, uppercase, and set `predicted_speak = false` if it starts with `"SILENT"` or `">"` (covering `"SILENT"` and `">"` exactly as well as prefixed variants), else `true`.
   - Record every scored prediction through `judge.record_outcome(ScorerItemOutcome { ... })` with `metric_name: "when2speak_turn"`, `score: Some(if hit { 1.0 } else { 0.0 })`, `passed: Some(hit)`, and `category: entry.category.clone()`, eliminating `heuristic_match` fallback in `CONVERSATION_SAMPLES.md`.

2. **Reason Output Limit & Truncated JSON EOF Resilience (`lab/profiles/deepseek-v4.1-flash-high.toml`, `crates/lab/src/datasets/framework.rs`)**:
   - Bump `output_limits.reason` from `4096` to `8192` so `reasoning_effort = "high"` responses do not truncate the trailing JSON `ThoughtBatch`.
   - In `drive_model_effects`, if `provider.invoke(&req)` returns `Err(fail)` with `fail.class == ProviderFailureClass::InvalidOutput` and `fail.detail.contains("EOF while parsing")`, retry `provider.invoke(&req)` at most once when `guard` still has remaining USD/token budget before logging failure and feeding `fail.to_core_input()` to Core.

3. **Direct-Answer Formatting in `[ANCHOR]` (`crates/reason/src/render.rs`)**:
   - Append a single generic rule to `output_contract` inside `render_slot_0_anchor`: when a user explicitly asks for a yes/no answer or a single option letter, place that exact word or option letter first at the start of `payload` before any explanation.

4. **All-Suite `--live` Semantic Embedder Auto-Start (`crates/lab/src/main.rs`)**:
   - Include `live` in both the `configured_embedder` startup guard and `require_semantic_embedder` flag inside `run_benchmark_cmd`, while gating on `!allow_no_embedder && !preflight_only`.
   - Offline runs (`live == false` with `--suite in_tree_only` or `--suite standard`) keep `configured_embedder = None` and `require_semantic_embedder = false`.
