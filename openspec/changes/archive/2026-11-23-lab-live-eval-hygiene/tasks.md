## 1. When2Speak Scorer & Outcome Recording

- [x] 1.1 Update `When2SpeakScorer::score()` in `crates/lab/src/datasets/when2speak.rs` to treat `"SILENT"` / `">"` `chat.speak` payloads as `SILENT` and emit `ScorerItemOutcome` (`when2speak_turn`) via `judge.record_outcome(...)`, with unit tests.

## 2. Profile Limit & Truncated JSON EOF Resilience

- [x] 2.1 Raise `output_limits.reason` to `8192` in `lab/profiles/deepseek-v4.1-flash-high.toml` and update profile test assertion in `crates/lab/tests/it/memory_comparison.rs`.
- [x] 2.2 Add bounded single-retry on `InvalidOutput` (`"EOF while parsing"`) when `CostGuard` has budget in `crates/lab/src/datasets/framework.rs` with unit test.

## 3. Direct-Answer `[ANCHOR]` Formatting

- [x] 3.1 Add direct-answer rule to `output_contract` in `crates/reason/src/render.rs` and verify `monoagens-reason` `[ANCHOR]` tests.

## 4. Live Benchmark Embedder Auto-Start & Verification

- [x] 4.1 Update `run_benchmark_cmd` in `crates/lab/src/main.rs` so `--live` runs auto-start `EmbeddingGemma 2` and set `require_semantic_embedder` unless `--allow-no-embedder` or `--preflight-only` is passed, keeping offline `in_tree_only` unchanged.
- [x] 4.2 Run `openspec validate lab-live-eval-hygiene --strict`, workspace tests, clippy, and release build of `monoagens-lab`.
