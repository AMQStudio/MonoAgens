# Design

## Architecture & Decisions

1. **Readable Conversation Digest (`CONVERSATION_SAMPLES.md`) in `crates/lab/src/datasets/report.rs`**:
   - Keep `ReportOptions` and `GeneratedReportSummary` struct fields 100% backward compatible so all existing callers compile unchanged.
   - `generate_benchmark_report` automatically generates `<report_dir>/CONVERSATION_SAMPLES.md` alongside `REPORT.md` / `index.html` and links it inside `REPORT.md` and `index.html`.
   - Also expose `generate_conversation_samples(run_dir, out_path, samples_per_dataset)` (and CLI `--samples-per-dataset <N>` on `monoagens-lab report`, default `2`) which can inspect both `by_ds` and raw `*.outcomes.json` / `transcripts/<dataset>/*.jsonl` files even on partial or historical run directories (`$0` API cost).
   - For each dataset, sample selection deterministically picks up to 1 `PASS` item and 1 `FAIL`/low-score item (filling up to `N` slots cleanly without duplicate `item_id`s) and formats `TranscriptRecord` JSONL entries turn-by-turn into Markdown chat logs.

2. **Token-Budgeted `--suite smoke` (`smoke-e2e`) in `crates/lab/src/main.rs`**:
   - Defaults `--only` to `when2speak,proactive_bench,longmemeval,fantom`, `--pilot-items` to `2`, `--max-cost-usd` to `0.25`, and enables `--report` automatically (while respecting explicit overrides from CLI flags and outputting to `lab/runs/smoke` when `--out-dir` is omitted).
   - Works seamlessly with `--render-dry-run` (`$0`) or `--live --profile <path>`.

3. **Indonesian 3-Scenario E2E Smoke Suite (`monoagens-lab smoke-id`)**:
   - Exercises real `Core` + `Valve` + `NeuralModel` (`crates/lab/src/indonesian_smoke.rs`):
     1. `id_01_grup_santai_silent`: casual multi-party chatter without mention or memory match -> `Silent` (0 LLM wakes).
     2. `id_02_proactive_unanswered_question`: group fact shared -> same-topic confirmation (`Silent`) -> unaddressed unanswered question matching visible memory -> System 1 wakes proactively (`Go` or `Wait`) without `@mono`.
     3. `id_03_dm_privacy_delta_ni_zero`: owner DM secret (`20%`) vs group query (`@mono progres...`) -> `VisibleScope` isolates DM secret from group context (`Delta_NI = 0`).
   - Runs at `$0` by default (scripted System 2 + `HashEmbedder` or local `EmbeddingGemma 2` sidecar when configured/available) or with `--live --profile <path>`, and writes `<out_dir>/CONVERSATION_SAMPLES.md`.
