## 1. Conversation Samples Digest (`CONVERSATION_SAMPLES.md`)

- [x] 1.1 Implement `generate_conversation_samples` and integrate `CONVERSATION_SAMPLES.md` generation + `REPORT.md`/`index.html` links in `crates/lab/src/datasets/report.rs` and `monoagens-lab report --samples-per-dataset`.

## 2. Token-Capped Smoke Benchmark & Indonesian E2E Runner

- [x] 2.1 Add `--suite smoke` (`smoke-e2e`) defaults (`when2speak,proactive_bench,longmemeval,fantom`, `--pilot-items 2`, `--max-cost-usd 0.25`, auto-report) to `crates/lab/src/main.rs`.
- [x] 2.2 Implement `crates/lab/src/indonesian_smoke.rs` and `monoagens-lab smoke-id` covering the 3 Indonesian group/proactive/privacy scenarios (offline `$0` + optional `--live`) and outputting `CONVERSATION_SAMPLES.md`.

## 3. Tests & Verification

- [x] 3.1 Add integration/unit tests for `CONVERSATION_SAMPLES.md` generation (both PASS and FAIL sampling), `--suite smoke` argument defaults, and `smoke-id` offline E2E invariants; validate via `cargo test -p monoagens-lab`, `cargo clippy --workspace -- -D warnings`, and `openspec validate lab-e2e-conversation-samples --strict`.
