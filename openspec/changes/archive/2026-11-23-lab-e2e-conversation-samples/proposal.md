# Proposal

## Why

Running full `monoagens-lab` live benchmark suites across 23+ datasets takes significant wall-clock time and API tokens, and `generate_benchmark_report` currently renders Markdown transcripts only for failed items (`wrong`), making it slow to quickly inspect representative PASS/FAIL conversations or validate Indonesian multi-party group & DM-privacy behaviors end-to-end.

## What Changes

- Add a **Conversation Samples Digest (`CONVERSATION_SAMPLES.md`)** generator to `crates/lab/src/datasets/report.rs` (automatically invoked during `generate_benchmark_report` and directly via `monoagens-lab report --samples-per-dataset <N>`), rendering human-readable chat logs (`[actor]`, `[Otak S1: outcome]`, `[LLM]`, `[Mono -> chat.speak]`, Gold vs Soul answer, and verdict) for both `PASS` and `FAIL` items per dataset.
- Add a token-capped **`--suite smoke`** (`smoke-e2e`) benchmark preset in `crates/lab/src/main.rs` that selects 4 core behavioral datasets (`when2speak`, `proactive_bench`, `longmemeval`, `fantom`), defaults to `--pilot-items 2` (`8` items total) and `--max-cost-usd 0.25`, and automatically produces `REPORT.md`, `index.html`, and `CONVERSATION_SAMPLES.md`.
- Add a self-contained 3-scenario **Indonesian Group & Privacy Smoke E2E (`smoke-id`)** runner (`crates/lab/src/indonesian_smoke.rs` + `monoagens-lab smoke-id`) supporting both zero-token offline/scripted verification (using `NeuralModel` + `EmbeddingGemma 2` / `HashEmbedder` + `Valve`) and optional `--live --profile <path>`, emitting `CONVERSATION_SAMPLES.md`.

## Capabilities

### New Capabilities
- `lab-e2e-conversation-samples`: Representative PASS/FAIL `CONVERSATION_SAMPLES.md` transcript digest, low-cost `--suite smoke` 4-dataset pilot preset, and 3-scenario Indonesian E2E smoke runner (`smoke-id`).

### Modified Capabilities
- *(none)*

## Impact

- `crates/lab/src/datasets/report.rs`, `crates/lab/src/datasets/mod.rs`, `crates/lab/src/indonesian_smoke.rs`, `crates/lab/src/lib.rs`, `crates/lab/src/main.rs`, and `crates/lab/tests/`.
- Zero changes to `crates/core`, `crates/valve`, `ARCHITECTURE.md`, or `lab/scenarios/sealed/`.
