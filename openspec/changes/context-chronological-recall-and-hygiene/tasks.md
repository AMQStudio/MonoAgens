## 1. Chronological `[RECALLED]` & `[PLACE_TAIL]` Deduplication

- [x] 1.1 Update `render_recalled` in `crates/reason/src/render.rs` to deduplicate against `sections.place_tail` `event_id`s and sort ascending by `(timestamp_ms, event_id.as_u64())`, with unit tests in `crates/reason/tests/it/render_tests.rs`.

## 2. `act.people` Place-ID Normalization & `[ANCHOR]` Contract

- [x] 2.1 Clarify `places` vs `people` (`[]` for whole room, never a `place_id`) and concise direct-answer/list formatting in `render_slot_0_anchor` (`crates/reason/src/render.rs`), and update assertions in `crates/reason/tests/it/render_tests.rs`.
- [x] 2.2 Strip exact-matching `places` entries from `act.people` in `crates/reason/src/parse.rs` (`parse_single_act`) and `crates/core/src/turn/step.rs` (`execute_turn`), with unit tests in `crates/reason/tests/it/reason_suite.rs` and `crates/core/tests/it/context_plan_c5.rs`.

## 3. `proactive_bench` & `fantom` Scorer Hygiene

- [x] 3.1 Update `ProactiveBenchScorer::score` in `crates/lab/src/datasets/proactive_bench.rs` to treat `"SILENT"`-prefixed `chat.speak` replies as non-speaking and record `proactive_trigger` outcomes via `judge.record_outcome(...)`, with unit tests.
- [x] 3.2 Update `FantomScorer::score` in `crates/lab/src/datasets/fantom.rs` to record `fantom_all_star` (`category: BeliefQ[Dist.]`) outcomes via `judge.record_outcome(...)` when `fantom_eval.py` returns results, with unit test.

## 4. Verification

- [x] 4.1 Run `openspec validate context-chronological-recall-and-hygiene --strict`, workspace tests (`monoagens-core`, `monoagens-reason`, `monoagens-lab`), `cargo clippy --workspace --all-targets -- -D warnings`, release build of `monoagens-lab` and `monoagens-cli`, and GitNexus `detect_changes`.
