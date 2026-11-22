# Proposal

## Why

Live Lab evaluation revealed four harness and prompt-formatting issues that distort metrics or drop valid wakes:
1. `When2SpeakScorer` only checks `pred.spoke`, misclassifying `chat.speak("SILENT")` as `SPEAK`, and never calls `judge.record_outcome(...)`, causing `CONVERSATION_SAMPLES.md` to fall back to `heuristic_match` against `"<"`.
2. `lab/profiles/deepseek-v4.1-flash-high.toml` sets `output_limits.reason = 4096`, which allows reasoning tokens to exhaust the completion budget and truncate the JSON `ThoughtBatch` (`EOF while parsing`), and `drive_model_effects` fails the wake immediately without a bounded retry.
3. `[ANCHOR]` `output_contract` lacks guidance for yes/no and single-letter multiple-choice prompts, leading models to bury the answer inside explanatory prose.
4. `monoagens-lab benchmark --live --suite in_tree_only --only ...` does not auto-start `EmbeddingGemma 2` or require the semantic embedder unless a category/smoke suite is selected.

## What Changes

- Update `When2SpeakScorer::score()` in `crates/lab/src/datasets/when2speak.rs` to treat `chat.speak` payloads starting with/equal to `"SILENT"` or `">"` (trimmed, case-insensitive) as `SILENT` and record per-item outcomes via `judge.record_outcome(...)` (`metric_name: "when2speak_turn"`).
- Increase `output_limits.reason` in `lab/profiles/deepseek-v4.1-flash-high.toml` from `4096` to `8192`, and add a single budget-checked retry in `crates/lab/src/datasets/framework.rs` when `provider.invoke(&req)` fails with `ProviderFailureClass::InvalidOutput` containing `"EOF while parsing"`.
- Add one concise sentence to `[ANCHOR]` `output_contract` in `crates/reason/src/render.rs` instructing models to place an explicitly requested yes/no word or single option letter first at the start of `payload`.
- Update `run_benchmark_cmd` in `crates/lab/src/main.rs` so `--live` benchmark runs enable `EmbeddingGemma 2` auto-start and `require_semantic_embedder` regardless of `--suite` (unless `--allow-no-embedder` or `--preflight-only` is set), preserving offline `in_tree_only` behavior.

## Capabilities

### New Capabilities
- `lab-live-eval-hygiene`: Accurate `when2speak` SILENT/SPEAK scoring & outcome recording, `8192` reason output limit & truncated-JSON EOF single-retry resilience, direct-answer `[ANCHOR]` `output_contract` formatting, and `--live` semantic embedder auto-start.

### Modified Capabilities
- *(none)*

## Impact

- `crates/lab/src/datasets/when2speak.rs`, `crates/lab/src/datasets/framework.rs`, `crates/lab/src/main.rs`, `crates/reason/src/render.rs`, `lab/profiles/deepseek-v4.1-flash-high.toml`, and unit/integration tests in `crates/lab` and `crates/reason`.
- Zero changes to `crates/core`, `crates/valve`, `ARCHITECTURE.md`, or `lab/scenarios/sealed/`.
