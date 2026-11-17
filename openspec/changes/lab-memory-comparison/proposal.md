# Proposal

## Why

The first live pilot (2026-11-18, `lab/runs/2026-11-17-full-live-deepseek-v4.1-flash/pilot/`)
did not measure MonoAgens: the Lab replaced the soul's own prompt with the whole conversation
transcript (`crates/lab/src/datasets/framework.rs`, `build_probe_soul_prompt`), and history was
loaded with a scripted silent model. Its extrapolated full run (about 109,000 probes and around
one billion input tokens across 20 datasets in one mixed report) was neither affordable nor
readable. The owner wants one question answered at a time, starting with memory: how well does
the **full** MonoAgens remember, and at what token cost.

## What Changes

Owner rulings of 2026-11-18:

- **Our own fair standard, no published numbers.** Every model call — the participant's own and
  the judge's — uses `deepseek-v4.1-flash` through OpenCode Go with reasoning effort `high` and a
  stable `x-opencode-session` per conversation for prompt caching. Published scores of other
  systems are never shown beside ours, because they were measured with other models. The public
  benchmark items, categories and judge prompt texts are reused as published (Run 4, `R-55`), run
  by the DeepSeek judge, under our own metric names.
- **Phase 0 now: the full MonoAgens alone** on all 10 LoCoMo conversations (categories 1–4 as
  the headline, category 5 apart) and all 500 LongMemEval-S questions, once, from the current
  code. It is the starting point for improving memory and proves the full-MonoAgens path before
  any other system is run. A pilot (one LoCoMo conversation and 20 LongMemEval-S questions) runs
  first and stops for the owner's approval. Estimated cost: about $6.5–9 at list price, inside the
  OpenCode Go monthly allowance.
- **Head-to-head later:** after memory has been improved and only when the owner decides, the
  full MonoAgens runs against Mem0, Letta, Zep/Graphiti and LangMem under the same model, judge,
  items and prompts. No full-transcript or no-memory arm takes part in any phase.
- Every LLM call of every participant, ingestion included, is metered the same way: input, cached
  input, output and reasoning tokens, calls, latency and plan cost.
- The memory suite has its own memory-only report.
- Depends on the `lab-datasets` owner revision of 2026-11-18 (full-MonoAgens rule, suites per
  category, cached-token telemetry), recorded in that change.

## Capabilities

### New Capabilities
- `memory-comparison`: the memory suite — phase 0 baseline, head-to-head participants and parity
  rules, judge and metrics, token metering, pilot gates and the memory report.

### Modified Capabilities

None in `openspec/specs/` (empty). The `lab-datasets` change (not yet archived) is revised in
place for the full-MonoAgens rule and per-category suites.

## Impact

- **Code (phase 0)**: `crates/lab` gains the memory suite (phase 0), the LoCoMo/LongMemEval-S
  judge with the published prompt texts, scoring and the memory report; a Lab-local metering
  proxy fronts OpenCode Go for MonoAgens and the judge.
- **Code (head-to-head, later)**: real `lab/adapters/` for `mem0`, `letta`, `zep_graphiti` and
  `langgraph_langmem`, an OpenAI-compatible `/v1/embeddings` route on the EmbeddingGemma 2
  sidecar, and the infrastructure choices listed in Run 4 (`R-56`, `R-57`).
- **Cost**: phase 0 ≈ 40 M input (most of it cacheable prefix) and 7–9 M output incl. reasoning
  tokens ≈ $6.5–9 at list price, inside the Go allowance; run outside the Go peak-hour windows.
- **Research**: Run 4 (`research/findings/2026-11-18-run-4.md`, `R-55`–`R-58`).
