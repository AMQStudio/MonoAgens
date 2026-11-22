# lab-e2e-conversation-samples Specification

## Purpose
TBD - created by archiving change lab-e2e-conversation-samples. Update Purpose after archive.

## Requirements

### Requirement: Representative PASS/FAIL conversation digest in Lab reports
`monoagens-lab report` and `generate_benchmark_report` SHALL emit `CONVERSATION_SAMPLES.md` containing readable turn-by-turn conversation transcripts (up to `N` items per dataset balancing passing and failing examples, defaulting to `2`), recording speaker percepts, System 1 gate outcomes, LLM/action replies, Gold vs Soul answers, and verdicts without requiring live API calls.

#### Scenario: Report generates CONVERSATION_SAMPLES.md from recorded JSONL transcripts
- **WHEN** `monoagens-lab report --run <dir>` executes against a run containing recorded `outcomes.json` and `transcripts/<dataset>/*.jsonl`
- **THEN** `<out_dir>/CONVERSATION_SAMPLES.md` is created containing formatted chat transcripts for both passing and failing sample items and is linked from `REPORT.md`

### Requirement: Low-budget `--suite smoke` (`smoke-e2e`) benchmark preset
`monoagens-lab benchmark --suite smoke` (and alias `smoke-e2e`) SHALL default to the 4 representative behavioral datasets (`when2speak`, `proactive_bench`, `longmemeval`, `fantom`), `--pilot-items 2`, `--max-cost-usd 0.25`, and automatic report + `CONVERSATION_SAMPLES.md` generation unless explicitly overridden by CLI flags.

#### Scenario: Smoke preset resolves minimal dataset subset and safety cost guard
- **WHEN** a user invokes `monoagens-lab benchmark --suite smoke --preflight-only` (or `--live --profile ...`)
- **THEN** the Lab runner restricts execution to `when2speak,proactive_bench,longmemeval,fantom`, caps items to 2 per dataset, sets the `$0.25` default cost cap, and enables report + sample generation

### Requirement: Three-scenario Indonesian E2E group & privacy smoke suite (`smoke-id`)
`monoagens-lab smoke-id` SHALL execute 3 Indonesian multi-party scenarios (casual chatter restraint, proactive group helping without `@mono`, and DM-vs-group `Delta_NI = 0` isolation) through `Core` and `Valve` (offline `$0` by default, or `--live --profile <path>`) and write a readable `CONVERSATION_SAMPLES.md` transcript report.

#### Scenario: Offline smoke-id verifies System 1 gate decisions, Delta_NI=0, and writes Indonesian transcript samples
- **WHEN** `monoagens-lab smoke-id --out-dir <dir>` runs offline with `--model <v22.safetensors>`
- **THEN** all 3 Indonesian scenarios pass gate/privacy invariants at `$0` API cost and `<dir>/CONVERSATION_SAMPLES.md` records the full formatted dialogues
