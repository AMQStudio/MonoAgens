# Spec Delta

## Purpose

Turns verified external datasets into `VisibleScope`-valid MonoAgens behavioral runs or `reason`
embedder probes, guards `--live` runs with preflight budgets, and scores them by aligned official
protocols (`research/findings/2026-11-17-run-3.md`, `R-52`–`R-54`) into a one-row-per-dataset
scorecard with contamination metadata.

## ADDED Requirements

### Requirement: The standard suite runs only on verified datasets
`monoagens-lab benchmark --suite standard` SHALL run only when every required dataset not marked
`unavailable` verifies. It SHALL fail before any run, naming each dataset that is missing or
unverified.

#### Scenario: One required dataset fails verification
- **WHEN** the standard suite starts and one required dataset has a checksum mismatch
- **THEN** no dataset is run, the command names that dataset and exits non-zero

### Requirement: A subset run is marked partial
`monoagens-lab benchmark --only <ids>` SHALL run just the named verified datasets. Its report and
run records SHALL be marked `partial` and list the datasets left out, so a partial result is
never read as the standard suite.

#### Scenario: Running two datasets
- **WHEN** the owner runs `monoagens-lab benchmark --only locomo,longmemeval`
- **THEN** the report is marked partial and lists every other registered dataset as not run

### Requirement: Converters feed the soul only what a body could perceive
Each behavioral dataset SHALL convert its units into valid 6-fact MonoAgens percepts and a separate `GoldBook`, without relaxing Core privacy (`probe.audience ⊆ evidence.audience` under `VisibleScope::allows`). Gold answers, `_abs`/`.trace` tags, and summaries SHALL NOT reach the soul. Multi-probe runs SHALL clone post-ingestion checkpoints, and missing timestamps, historical turns, or ToM visibility SHALL follow Run 3 `R-53` rules.

#### Scenario: A question carries its gold answer
- **WHEN** a converted item has a question and a gold answer
- **THEN** the soul receives only the question as a percept, and the gold answer is held by the scorer

#### Scenario: Probe audiences obey VisibleScope subset rules across every dataset
- **WHEN** any dataset fixture test runs `VisibleScope::allows` from the question probe's audience against every gold evidence memory turn
- **THEN** non-conflict gold evidence is allowed without `[soul]`-only loopholes (`[speaker_a, soul]` on `locomo`; `[user, soul] ⊆ [user, prior_assistant, soul]` on `longmemeval`/`stale`/`LME(S*)`; `[lab_proctor, soul]` with `lab_proctor` in scene audiences on `memory_agent_bench` non-LME, `molweni`, `ubuntu_irc`, and ToM), while cross-channel `group_mem_bench` items lacking the asker are marked `privacy_conflict` in `GoldBook`, stay blocked by `VisibleScope`, and score refusal and non-conflict accuracy separately

#### Scenario: Datasets lack per-turn timestamps or use structured vs. prose ToM
- **WHEN** a dataset provides session dates or turn order only, or represents structured vs. prose ToM
- **THEN** converters apply deterministic monotonic offsets (`+1,000 ms`/turn, `+60,000 ms`/stage for `dyntom`, `+60,000 ms`/chunk for `memory_agent_bench` `stream_source`), Mode 1 dynamic `audience` (`{occupants..., lab_proctor, soul}`) on structured ToM (`fantom`, `hi_tom`, `exploretom`, `tomi`), and Mode 2 (`[narrator, lab_proctor, soul]`) on prose ToM (`opentom`, `dyntom`)

#### Scenario: Non-conversational register datasets route to the reason Embedder probe lane
- **WHEN** a dataset lacks actors, places, audiences, timestamps, and turns (`multilexnorm`, `indocollex`)
- **THEN** it carries `core_applicability = "not_mappable"` for Core `run_scenario`, executes as a `monoagens-reason::Embedder` component probe (`twin_retrieval_r1_256d`) that rejects `HashEmbedder`, `NoneEmbedder`, and `ScriptedEmbedder` outside unit tests, and reports `not_scored` if local semantic weights are absent

### Requirement: Conversion is deterministic and versioned
A converter SHALL produce byte-identical output for the same verified input and converter version.
Its output SHALL carry the dataset id, pinned revision and converter version, and SHALL be
rebuilt when the converter version changes.

#### Scenario: Converting twice
- **WHEN** the same verified dataset is converted twice with the same converter version
- **THEN** both outputs are byte-identical

### Requirement: Scoring follows each benchmark's official protocol
Each primary metric SHALL align with its verified `R-52` official protocol, keeping `exploretom` separate from `dyntom` and binding `ubuntu_irc` to `jkkummerfeld/irc-disentanglement`. Any metric differing from an upstream definition or using a non-official judge model SHALL carry a distinct non-official name, and judge-scored runs SHALL record the judge model and prompt version.

#### Scenario: A metric needs a model judge
- **WHEN** a dataset's official metric uses an API model judge (`longmemeval`, `memory_agent_bench` `LME(S*)`, `group_mem_bench`, `stale`, `capa`)
- **THEN** the run record names the judge model and prompt version, renames any metric whose configured judge differs from the official model (`gpt-4o`, `gpt-5`, `Gemini-3.1-flash-lite`), and reports `not_scored` without `--live`

#### Scenario: A dataset uses deterministic or local non-API scoring
- **WHEN** `locomo`, `pm_bench`, `pitch`, `when2speak`, `ubuntu_irc`, `molweni`, `fantom` (`lab/adapters` local `SentenceTransformer`), `opentom`, `hi_tom`, `exploretom`, `dyntom`, `tomi`, or `proactive_bench` (`TGL` trigger parity) runs offline on a verified fixture
- **THEN** no paid judge API call is required to compute its deterministic or local-adapter metric

### Requirement: External results never tune the prior
External datasets SHALL form a fourth scenario set, `external`, beside development, validation and
sealed. Their results SHALL be reported, and SHALL NOT be used to calibrate dispositions,
thresholds or any other prior value.

#### Scenario: Calibration runs while external results exist
- **WHEN** the Lab calibrates values from the development set
- **THEN** no external dataset result is read as calibration input

### Requirement: Every dataset run writes a run record
Each dataset run SHALL write a run record with the fields `core-and-lab` requires of every run,
plus the dataset id, pinned revision, manifest digest, converter/probe and scorer versions, the
items sampled, and per-metric scores with confidence intervals.

#### Scenario: Two benchmark runs differ
- **WHEN** two benchmark runs of the same dataset are compared
- **THEN** any difference in dataset revision, converter, scorer, sample, prior, model or seed is visible from their run records alone

### Requirement: Items are sampled by a recorded rule
When a run uses fewer items than a dataset holds, it SHALL select them via a seeded stratified rule fixed in advance (`R-54` default splits and `S = 471`/`1,000` rules, or contiguous log windows on `ubuntu_irc`), using identical item ids across arms and recording both the rule and chosen ids in the run record. Benchmarks whose items share conversations, channels, rows, or stories SHALL additionally report cluster-bootstrap confidence intervals.

#### Scenario: Comparing two arms on a clustered sample
- **WHEN** two arms are scored on a sampled or clustered subset (`locomo`, `memory_agent_bench`, `group_mem_bench`, `capa`, `pitch`, `fantom`, `molweni` MRC, `hi_tom`, `dyntom`) with the same seed
- **THEN** both arms receive the exact same item ids, and both run records report cluster-bootstrap confidence intervals

### Requirement: The scorecard feeds the Pareto report
The standard suite SHALL emit **exactly one row per registered dataset** in `ParetoScorecardInput` form carrying metrics, cost, `lane` (`core_behavioral` vs `embedder_probe`), `status` (`run`, `partial`, `not_scored`, `unavailable`, `not_mappable`, or `unverified`), and `contamination_risk`. Datasets without scores SHALL remain gaps and NEVER be imputed.

#### Scenario: The standard suite finishes
- **WHEN** `monoagens-lab benchmark --suite standard --pareto` completes
- **THEN** the output holds strictly one row per registered dataset (with `multilexnorm` and `indocollex` in `embedder_probe`), explicit status and `contamination_risk`, and a Pareto frontier computed from scored rows only

### Requirement: Default tests use committed fixtures only
Converters, embedder probes, and scorers SHALL be tested on small fixtures written in each
dataset's real file format and committed to the repository. The fixtures SHALL contain no text
copied from a real dataset. Default tests SHALL open no network connection and call no paid model.

#### Scenario: Running the test suite offline
- **WHEN** `cargo test -p monoagens-lab` runs with no network
- **THEN** every converter, probe, and scorer test passes using only committed fixtures

### Requirement: Live benchmark runs need the owner's authorization and cost caps
Paid `--live` runs SHALL start only with `--live` plus an explicit `--max-cost-usd` cap after emitting a scripted preflight wake/cost estimate. Without altering `monoagens-core`'s single cognitive loop, static memory and ToM histories SHALL load through Lab-side `ScriptedModelProvider` silent `think` reasoning (recording `history_reasoning_mode = "scripted_silent"` in `RunRecord`), and runs without `--live` SHALL report paid-model metrics as `not_scored`.

#### Scenario: Running the standard suite without --live or above budget
- **WHEN** the standard suite runs without `--live`, or a `--live` preflight exceeds `--max-cost-usd`
- **THEN** without `--live` model-dependent metrics report `not_scored`, and over-budget `--live` runs halt cleanly at `CostGuard` before exceeding the cap

### Requirement: Live runs pass and record provider options
`--live` runs SHALL accept a reasoning effort for the soul's model and for the judge (`--reasoning-effort`, `--judge-reasoning-effort`) and optional extra request headers, send them on every model call, and record the effort values and header names (never header values or keys) in every run record and report.

#### Scenario: A live run with medium soul effort and high judge effort
- **WHEN** the owner runs `--live --reasoning-effort medium --judge-reasoning-effort high`
- **THEN** every soul call carries effort `medium`, every judge call carries `high`, and the run records and report show both values without any secret

### Requirement: A full benchmark run produces a standard report
A standard-suite run SHALL write a report into its `lab/runs/<run-id>/` directory: a reproducibility block, per-domain results with confidence intervals, a status and gap table, cost and wake statistics, charts, failure examples and per-item transcripts. Every table and chart SHALL be generated from the run records, never typed by hand.

#### Scenario: Reproducibility and results sections
- **WHEN** the report is written
- **THEN** it names the commit, prior digest, manifest digest, model, reasoning effort, judge, seeds and dates, and shows each dataset's official or renamed metrics with intervals and per-category breakdowns

#### Scenario: Reading the report of a full live run
- **WHEN** a full `--live` standard-suite run completes
- **THEN** its run directory holds the report (Markdown and HTML), CSV tables, chart images, and one transcript file per scored item, and every number in them traces to a run record

### Requirement: Behavioral runs use the soul's semantic embedder
External behavioral runs SHALL embed every percept and probe with the configured semantic embedder (EmbeddingGemma 2, `256d`, with the task prefixes and `(embed_model_ver, dim_d, prompt_regime_id)` tags of `core-and-lab`) before the core sees it, and the run record SHALL name the embedder. A run without a semantic embedder SHALL be labelled `embedder = none` in every row, and SHALL NOT be reported as a standard run.

#### Scenario: A memory dataset runs with the semantic embedder
- **WHEN** `locomo` runs with EmbeddingGemma 2 configured
- **THEN** every ingested turn and probe carries a `256d` vector, recall can use vector candidates, and the run record names the embedder and its version

#### Scenario: The embedder is missing
- **WHEN** a standard-suite run starts and no semantic embedder is configured
- **THEN** the run refuses to start as a standard run and tells the owner how to fetch and configure the embedder
