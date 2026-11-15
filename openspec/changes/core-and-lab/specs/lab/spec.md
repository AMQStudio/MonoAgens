# Spec Delta

## Purpose

The Lab measures the soul. It runs scenarios against the real core through scripted test
bodies, switches mechanisms to their neutral values and reports the difference, so that
nothing is claimed about the soul that a run does not show.

## ADDED Requirements

### Requirement: Scenarios run against the real core over canonical Waist-2 codecs
A scenario SHALL execute against the real core, store, and reason crates with scripted test
bodies speaking canonical Waist-2 schema payloads (`monoagens.body.v1`, `R-50`), never bypassing
doors or writing directly to the log.

#### Scenario: A scenario drives two test bodies
- **WHEN** a scenario scripts percepts in two places from two test bodies
- **THEN** the events traverse Waist-2 schema encoding and the inbound door, are recorded, and are appraised by the core

### Requirement: Test bodies stand for any kind of body
A scenario SHALL be able to declare test bodies with their own places, sources, capabilities,
scripted receipts and scripted percepts that follow an action, so that a chat room, a device
with sensors or a game world can be scripted with the same means.

#### Scenario: A device body
- **WHEN** a scenario declares a body with a "push" capability and a sensor percept that follows it
- **THEN** the soul's act through that body ends in a receipt and the sensor percept answers the act's prediction

#### Scenario: The sensor stays silent
- **WHEN** the scenario scripts no sensor percept after the act
- **THEN** the act's prediction expires and that absence is recorded

### Requirement: Scenarios run in virtual time
A scenario SHALL control the injected clock, including how long model calls take, so that
hours or days pass without real waiting.

#### Scenario: Thirty days in a scenario
- **WHEN** a scenario places two events thirty days apart
- **THEN** the run completes without waiting, and the recorded events are thirty days apart

#### Scenario: Events during a slow model call
- **WHEN** a scenario sets a model latency longer than the gap between its next events
- **THEN** those events are appraised before the model's result is processed

### Requirement: A scenario states its expectations as checks
A scenario SHALL declare checks on the outcome: an action did or did not happen in a place
within a time; a context did or did not contain an item or a label; an expectation, belief or
ledger card reached a given state; a receipt had a given outcome; a decision had a given
outcome or reason; an action's payload does or does not contain given text. A run SHALL report
each check as passed or failed.

#### Scenario: A check fails
- **WHEN** a scenario requires silence in a place and the soul acts there
- **THEN** the run reports that check as failed with the event that violated it, and the exit status signals failure

### Requirement: Scripted runs are deterministic
With the scripted model provider, the scripted embedder and a fixed seed, two runs of the same
scenario SHALL produce identical logs.

#### Scenario: The same scenario twice
- **WHEN** a scripted scenario is run twice with the same seed
- **THEN** the two logs are identical event for event

### Requirement: Live runs use a real model
The Lab SHALL be able to run the same scenarios with a real model provider. Live runs SHALL be
started only on explicit request and SHALL never be part of the default test run. Checks SHALL
be tagged structural or wording, and live results SHALL report the two separately.

#### Scenario: Live credentials are missing
- **WHEN** a live run is requested and no provider settings are present
- **THEN** the run stops before any scenario starts, with a message naming the missing settings

#### Scenario: A live run finishes
- **WHEN** a live run completes
- **THEN** structural checks are reported as passed or failed, wording checks as a pass rate, and the token cost is in the run record

### Requirement: A cost guard bounds every run
The Lab SHALL install at the context door a guard that refuses further wakes once a run's
token limit is reached or a chain's wake limit is reached, and SHALL record each refusal. Both
limits SHALL be adjustable and removable per run.

#### Scenario: A chain keeps waking itself
- **WHEN** a chain reaches the wake limit
- **THEN** its next wake is refused, the refusal is recorded and the run reports it

#### Scenario: The token limit is reached
- **WHEN** a live run reaches its token limit
- **THEN** no further model call is made and the run ends with a report that the limit stopped it

### Requirement: Ablation sets a mechanism to its neutral value
The Lab SHALL be able to run any scenario set with any registered mechanism at its neutral
value. It SHALL run each side with at least five seeds and SHALL report, per metric, the
difference with a confidence interval, and the cost. A difference within seed noise SHALL be
reported as no measured effect.

#### Scenario: Habituation is ablated
- **WHEN** a scenario with a rapidly repeating source is run with habituation active and neutral
- **THEN** the report shows the number of wakes for both, their difference and its confidence interval

#### Scenario: A mechanism changes nothing
- **WHEN** ablating a mechanism changes no reported metric beyond seed noise across the development and validation sets
- **THEN** the report flags that mechanism as having no measured effect

### Requirement: Every mechanism has a scenario that tests its claim
For each registered mechanism, the development set SHALL contain at least one scenario whose
checks or metrics are expected to differ when that mechanism is neutral.

#### Scenario: A mechanism without a scenario
- **WHEN** the Lab lists mechanisms and one has no scenario naming it
- **THEN** the listing marks it as unproven and the coverage command exits with a non-zero status

### Requirement: Every run records what produced it
Each run SHALL write a record containing the core version, the prior version, the model and
embedder, the door rules installed, the mechanism switches, the learning switches, the seed,
the scenario set, the result of every check, the metrics and the cost by purpose.

#### Scenario: Comparing two runs
- **WHEN** two run records are compared
- **THEN** every difference in core version, prior, model, door rules, switches or seed between them is visible from the records alone

### Requirement: Scenarios are split into three sets
Scenarios SHALL be grouped into a development set, a validation set and a sealed set. For the
sealed set, the Lab SHALL report only aggregate results and SHALL NOT print scenario contents,
contexts or per-step traces.

#### Scenario: Running the sealed set
- **WHEN** the sealed set is run
- **THEN** the output contains pass counts and metrics only, and the run record marks the run as sealed

### Requirement: The Lab reports behaviour against annotation
A scenario SHALL annotate warranted replies (`SPEAK`/`SILENT`), floor ownership (`sel_other`/
`answered`), and causal labels so the Lab reports when-to-speak precision/recall, false speaks,
missed replies, `floor_violations_per_100`, `wakes_per_event`, `double_answers`, `self_echo_rates`,
`boundary_f1`, `1/n` message share (`[0.5, 2.0]` band check, `R-03`), and reply-delay distributions.

#### Scenario: An annotated group conversation
- **WHEN** a scenario marks five of forty events as warranting a reply and the soul replies to four of those and to one other
- **THEN** the report states precision 0.8 and recall 0.8 and verifies whether message rate sits within `0.5x..2.0x` of `1/n`

### Requirement: The Lab reports full 22-row cognition, honesty, privacy, and calibration metrics
The Lab SHALL report metrics covering all 22 brain-map rows (`A-02`): 1/7/30-day recall hits,
promises kept on time, `kept_late`, broken, and repaired (`R-49`), `detail_without_recollection`,
`stale_beliefs`, false-belief accuracy, `[0,20]` Brier/ECE (`R-35`), `sleep_spin_count` (`== 0`),
tokens/calls per event, cache share, and narrower-to-wider leakage (`Delta_NI == 0`, `R-33`).

#### Scenario: A promise and sleep audit scenario
- **WHEN** a scenario with three promises (two on time, one recovered after downtime) and pre-sleep idle chatter runs
- **THEN** the report states two kept on time, one kept late, zero sleep spin loops, and `Delta_NI == 0`

### Requirement: External BaselineAdapter benchmarks Mem0, Letta, and peer frameworks under parity
The Lab SHALL expose an out-of-process `BaselineAdapter` protocol evaluating external agents—such
as `Stateless ReAct`, `Arm F`, `Mem0`, `Letta/MemGPT`, `LangGraph+LangMem`, `Zep/Graphiti`, and
`Hermes Agent`—with injected virtual time (`sim_time_rfc3339`), pinned RNG seeds, `PerceptView`
oracle stripping, `restart_epoch` RAM eviction, and `SEALED_MODE=1` isolation (`R-39`).

#### Scenario: External adapters run under virtual time without label leakage
- **WHEN** `monoagens-lab baseline` evaluates `Mem0`, `Letta`, `LangGraph+LangMem`, `Zep/Graphiti`, or `Hermes` over a 30-day multi-party scenario
- **THEN** each adapter records injected `sim_time_rfc3339` timestamps, clears ephemeral RAM on `restart_epoch`, and receives no ground-truth labels

#### Scenario: Sealed mode rejects payload leakage from any adapter
- **WHEN** a `SEALED_MODE=1` baseline worker attempts to write message payloads or rendered prompts to disk or stdout
- **THEN** preflight isolation aborts the run with a non-zero exit status and reveals no scenario text

### Requirement: Baseline comparisons enforce NOOP silence parity and unified 5-tuple E_tok billing
Every external `BaselineAdapter` wake SHALL provide explicit `noop()`/`[SILENT]` abstention tools
and record `wake_invoked_llm`, wall timeout `step_wall_timeout_s`, normalized 5-tuple effective
tokens (`E_tok`, `C_share`), and USD cost alongside MonoAgens (`0` LLM tokens on zero-LLM Gate
abstentions, `R-39`).

#### Scenario: LLM cost of chosen silence is billed symmetrically across harnesses
- **WHEN** a multi-party scenario includes irrelevant turns where an external harness wakes its LLM only to emit `noop()`
- **THEN** the harness ledger records full `E_tok`, latency, and `wake_invoked_llm = true` while MonoAgens records zero model tokens for gate suppression

### Requirement: The Lab benchmarks 5 cognitive domains, Indonesian suites, Tool RAG, and security
`monoagens-lab benchmark` SHALL run standardized evaluation suites covering all five cognitive
domains (Memory, Prospective, Proactivity/Timing, Group Chat, and Theory of Mind), Indonesian informal
register retrieval (`256d`), `Tool RAG` capability scaling (`N_caps = 50..200`), and security/privacy
suites (`R-38`, `R-39`, `R-43`, `A-15`), while reporting Black-Box `Core TCK` checks (`R-50`).

#### Scenario: Five-domain benchmark suite enforces dataset caveats and Cat-5 coverage
- **WHEN** `monoagens-lab benchmark --suite standard --pareto` executes across `LoCoMo` (including Category-5 adversarial), `LongMemEval`, `MemoryAgentBench`, `GroupMemBench`, `STALE`, `PM-Bench`, `PIS`, `PITCH`, `ProACT`, `ProactiveBench`, `CAPA`, synthetic `When2Speak`, `Ubuntu IRC`, `Molweni`, `FANToM` (`ALL*`), `OpenToM`, `HI-TOM`, and `ExploreToM`
- **THEN** the Lab emits domain-specific scorecards (`ALL*`, `PR`, `FIR`/`MIR`, `Delta_NI`), restricts `ToMi` to shortcut-perturbation checks, and excludes dyadic `MuTual` from multi-party addressee claims

#### Scenario: Security, Indonesian register, and Tool RAG suites report dedicated metrics
- **WHEN** `MPBench`, `VisibleScope`, `MultiLexNorm`/`IndoCollex`, and `Tool RAG` (`N_caps = 50..200`) suites execute
- **THEN** the Lab reports compaction poisoning ASR, `Delta_NI`, `256d` retrieval accuracy, `Core TCK` verdicts, and schema token savings across connected capabilities

### Requirement: Confirmatory paired comparisons report Holm-corrected statistics and Convex Pareto frontiers
For confirmatory non-inferiority (`Arm T` restart parity and thinking stratification, `A-16`) and
baseline comparisons, the Lab SHALL run cluster-paired suites (`S >= 354–471`) reporting
`Holm-Bonferroni` step-down p-values (`alpha = 0.05`), Cohen's `d_z`, Kerby rank-biserial `r_rb`,
co-primary NI checks, and an undominated `Convex Pareto` frontier (`Quality` vs `E_tok`/latency vs
risk `Δ_NI`/ASR) in JSON (`monoagens-lab pareto`, `R-39`).

#### Scenario: Pareto generator identifies undominated quality-cost-safety trade-offs
- **WHEN** `monoagens-lab pareto` compares MonoAgens against baselines across `E_tok`, `p95_latency`, quality, `FIR`, and `Δ_NI`
- **THEN** the JSON report emits cluster-adjusted confidence intervals, Holm-adjusted significance, and convex hull membership without collapsing raw 5-tuple token logs

### Requirement: Property-based proptest suites verify core and protocol invariants
The workspace SHALL execute randomized `proptest` suites validating seeded bit-identical
`Core::step` replay, delimiter-safe `κ(b, a)` identity hashing, monotonic Petrov hybrid
activation (`n > k ∧ t_n > t_k`), `VisibleScope` non-interference (`Delta_NI == 0`), and binary
serialization round-trips.

#### Scenario: Randomized traces preserve determinism and collision resistance
- **WHEN** `proptest` generates random multi-body percept schedules and colon-bearing actor IDs
- **THEN** replays match bit-for-bit, private items produce zero `Delta_NI` divergence, and `κ(b, a)` yields zero hash collisions

### Requirement: Criterion and R-40 scale benchmarks enforce long-log latency and erasure SLOs
The Lab SHALL include `criterion` microbenchmarks and `R-40` long-log stress runs verifying
`Core::open` startup (`< 100 ms`), filtered `VectorIndex` query latency and recall (`usearch` vs
`hnsw_rs` vs exact scan), and zero residual erased bytes after `hard_forget`.

#### Scenario: Long-log scale and filtered HNSW benchmarks hold latency and erasure SLOs
- **WHEN** `monoagens-lab` scale benchmarks evaluate `10^6` events at `1%` `VisibleScope` selectivity after `5%` `hard_forget` deletions
- **THEN** `Core::open` boots under `100 ms`, filtered HNSW maintains `Recall@10 >= 0.93` under `5 ms` p99, and raw storage byte scans find zero surviving erased vectors or payloads

### Requirement: The foundation is proven end to end
The development set SHALL contain end-to-end scenarios for: recall across places and a
thirty-day gap; a promise kept after silence and after downtime; recorded silence in a group
and a reply when addressed; a multi-step task across two places; a device body with a sensor;
and replay of each resulting log.

#### Scenario: The foundation scenarios pass scripted
- **WHEN** the foundation scenarios are run with the scripted provider
- **THEN** every check passes and every log replays cleanly

### Requirement: Wave 1 is proven end to end
The development set SHALL contain end-to-end scenarios for: yielding to another speaker; a
withheld unallocated turn; a free-text condition met later; a reply that shows an act did not
land; a missed backlog; a stale belief superseded after sleep; a false-belief question; told
against observed; an unreliable source; and bounded learning with rollback.

#### Scenario: The development set passes with the scripted provider
- **WHEN** the development set is run with the scripted provider
- **THEN** every check passes

#### Scenario: The development set is run live
- **WHEN** the development set is run with a real model
- **THEN** every structural check passes, and wording checks are reported with their pass rate

### Requirement: A soul can persist between Lab sessions
The Lab SHALL be able to keep a soul in a directory holding its identity and its log, and to
continue that soul in a later session.

#### Scenario: Continuing yesterday's soul
- **WHEN** a session is started on a soul directory used before
- **THEN** the soul's memories, open promises and learned values are those it had at the end of the earlier session

### Requirement: The owner can probe a soul from the terminal
The Lab SHALL offer a probe in which a person types as one source in one place of a persistent
soul in real time, and sees what the soul sends there, including messages the soul sends on
its own when a promise falls due.

#### Scenario: A reminder arrives unprompted
- **WHEN** the person asked for a reminder in two minutes and types nothing more
- **THEN** the soul's reminder appears after two minutes

### Requirement: Decisions can be explained and logs replayed
The Lab SHALL be able to show a log in order with filters, explain a single decision with all
its numbers, list the mechanisms with their claims, neutral and current values, and replay a
log without a model, reporting whether every decision was reproduced.

#### Scenario: Why did it stay silent
- **WHEN** a decision with outcome silent is explained
- **THEN** the signals, salience, habituation, threshold terms, probability and draw of that decision are printed

#### Scenario: A clean replay
- **WHEN** a log recorded by this version is replayed
- **THEN** the Lab reports that all decisions were reproduced and exits with status zero

### Requirement: Output is usable by programs
Commands that report results SHALL offer a machine-readable output format, and errors SHALL go
to the error stream with a non-zero exit status.

#### Scenario: A script reads Lab results
- **WHEN** a run is made with the machine-readable option
- **THEN** standard output contains only the structured result

### Requirement: Calibrated values are computed from the development set
The Lab SHALL compute the values that the research defines by calibration (the
feeling-of-knowing threshold and scale, and the wake band with its bounds) from the development
set for the embedder in use, and SHALL record them with the run. It SHALL NOT calibrate on the
validation or sealed sets.

#### Scenario: The embedder is changed
- **WHEN** a run uses a different embedder than the last calibration
- **THEN** the values are computed again for that embedder and recorded in the run record

### Requirement: The rebuilt context is compared with a transcript
The Lab SHALL be able to run a scenario with the context replaced by the full transcript of
the place and chain, and SHALL report task success, tokens and cache share for both forms,
including after a restart in the middle of a task.

#### Scenario: A restart in the middle of a task
- **WHEN** a multi-step scenario is run both ways with a restart between two steps
- **THEN** the report shows success and cost for the rebuilt context and for the transcript
