# Spec Delta

## Purpose

Measures how well the full MonoAgens remembers, and at what token cost, on public memory
benchmarks under the Lab's own fair standard: first alone as a baseline, later head-to-head
against dedicated memory frameworks, in one memory-only report.

## ADDED Requirements

### Requirement: Every model call uses one shared model and judge
Every LLM call of every participant, ingestion included, and every judge call SHALL use
`deepseek-v4.1-flash` through OpenCode Go with reasoning effort `high`. Reports SHALL NOT show
scores published by others beside the Lab's scores.

#### Scenario: Checking a run's model calls
- **WHEN** the ledger of any memory suite run is read
- **THEN** every participant and judge call shows model `deepseek-v4.1-flash` and effort `high`, and the report contains no externally published score

### Requirement: Phase 0 measures the full MonoAgens alone
Phase 0 SHALL run only the full MonoAgens (as the `lab-datasets` full-MonoAgens rule defines it),
once, on all 10 LoCoMo conversations and all 500 LongMemEval-S questions, from the current code.
Its run record SHALL state `development_exposure = "none"` per benchmark, meaning no MonoAgens
change was informed by either benchmark, and any later run SHALL record its exposure.

#### Scenario: Reading the phase 0 report
- **WHEN** phase 0 completes
- **THEN** the report shows MonoAgens' scores per LoCoMo category and per LongMemEval-S question type, its token use, and `development_exposure = "none"`

### Requirement: Every run starts with a pilot and an owner gate
Every phase SHALL start with a pilot and stop: for phase 0, one LoCoMo conversation and 20
LongMemEval-S questions chosen by a seeded rule stratified by question type; for head-to-head, one
LoCoMo conversation per participant. The pilot report SHALL show measured tokens per call,
cached share, latency, errors and the projected cost of the rest. The rest SHALL run only after
the owner's approval and with an explicit `--max-cost-usd`.

#### Scenario: The phase 0 pilot finishes
- **WHEN** the phase 0 pilot completes
- **THEN** the run stops and reports measured tokens, cached share and the projected cost of the remaining nine conversations and 480 questions

### Requirement: The render dry run checks what each call is asked
Before any live run, the render dry run SHALL fail when any rendered request contains Core
bookkeeping (`detail_kind`), when a classification or consolidation request has no `[TASK]` block
or its task lacks the material it judges (condition, earlier message, gist sources, episode
events) or its labels, or when a reasoning request carries a task. Its sample SHALL span enough
history for a sleep to run, and its scripted consolidation SHALL return a gist so gist
verification is rendered too.

#### Scenario: A classification without its question
- **WHEN** a classify request in the dry run renders only a commitment line and the trigger
- **THEN** the dry run fails and no live call is made

### Requirement: Scoring reuses the published items and judge prompts
LoCoMo SHALL be scored per category with its official token F1 and with the judge running the
published `ACCURACY_PROMPT` text (categories 1–4 headline; category 5 apart with the official
refusal check). LongMemEval-S SHALL be scored per question type with its five published judge
prompts, abstention apart. Judge metrics SHALL be named `judge_acc`, never after a model; the
judge model, profile and prompt version SHALL be recorded; intervals are cluster-bootstrap.

#### Scenario: Reading the LoCoMo results
- **WHEN** a LoCoMo run completes
- **THEN** each participant shows F1 and `judge_acc` for categories 1–4 with 95 % intervals and category 5 separately

#### Scenario: The soul stays silent on a question
- **WHEN** the full MonoAgens dispatches no answer to a question
- **THEN** the empty answer is judged as given and counted in a reported silence rate

### Requirement: Head-to-head runs only dedicated memory frameworks under parity
Head-to-head SHALL start only after the owner's decision. It SHALL run exactly the full
MonoAgens, Mem0, Letta, Zep/Graphiti and LangMem, with the same items in the same order and the
same answer-format instruction; each stores and recalls memories its own way. It SHALL NOT run a
full-transcript, no-memory or any substitute arm. A framework that cannot run under parity SHALL
be reported as a gap with its reason.

#### Scenario: A framework cannot run under parity
- **WHEN** one framework cannot be configured with the shared model, judge or items
- **THEN** its rows are reported as gaps with the reason, and no other system or setting is substituted for it

### Requirement: Every LLM call is metered alike
Every LLM call of any participant or the judge SHALL be metered the same way and attributed to
its participant, dataset, conversation and phase (ingest or answer): input, cached input, output
and reasoning tokens, latency and plan cost. Each participant and conversation SHALL use its own
stable `x-opencode-session` value. Header values and keys SHALL never be recorded.

#### Scenario: A framework calls the model while ingesting
- **WHEN** a framework makes LLM calls to extract memories from ingested turns
- **THEN** those calls appear in its ledger as ingest calls with their tokens and cost, and count toward its totals

### Requirement: The memory report shows quality against token cost
The suite SHALL write a memory-only report generated from its run records: a setup block (model,
effort, judge, prompt versions, items), results per dataset and category, a quality-versus-tokens
chart, and per participant the ingest and answer tokens (input, cached, output, reasoning),
calls, latency p50/p95, silence rate and plan cost, plus failure examples with transcripts. It
SHALL contain no other benchmark category.

#### Scenario: Comparing efficiency
- **WHEN** the owner reads a memory report
- **THEN** it shows, for each participant, the judge score next to tokens per question split into ingest and answer, with cached tokens shown separately
