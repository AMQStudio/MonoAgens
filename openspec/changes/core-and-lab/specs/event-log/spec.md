# Spec Delta

## Purpose

The event log is the soul's permanent record. Everything that persists is an event, state is a
projection of the log, and the log is what makes the soul's decisions replayable and auditable.

## ADDED Requirements

### Requirement: Record first, then act
An incoming event SHALL be written to the log before it is processed, and an action SHALL be
written to the log before it is handed to a body.

#### Scenario: A crash after recording loses nothing
- **WHEN** the process stops after an event was recorded and before it was processed
- **THEN** on the next `Core::open` start the event is discovered in the unconsumed tail and processed

### Requirement: Structured event catalog including withheld, feedback, and erasure
The append-only log SHALL record 30 canonical event kinds, including `withheld` silence outcomes, asynchronous `action_feedback` updates, authorized `hard_forget` erasure tombstones, and `prior_loaded` records of the prior in effect, enabling deterministic replay and causal audit without implicit external state.

#### Scenario: Deliberate silence and late feedback are logged
- **WHEN** appraisal withholds speech or a body emits delayed delivery `action_feedback`
- **THEN** the log appends a typed `withheld` or `action_feedback` event preserving turn correlation

### Requirement: Segmented storage, sealed shards, and checksummed snapshots
The store SHALL partition active writes into WAL-mode `hot.db` and seal older windows into read-only temporal shards alongside an abstracted `VectorIndex` trait (`usearch` `i8`/`bf16` HNSW primary with `hnsw_rs` pure-Rust and `BruteForceFlat` exact fallbacks) and `blake3`-checksummed `postcard` or `minicbor` snapshots, recovering unconsumed tails inside `Core::open` (`R-40`).

#### Scenario: Startup verifies snapshot and replays only tail events
- **WHEN** `Core::open` initializes storage with a valid snapshot and unconsumed `hot.db` events
- **THEN** the store verifies the `blake3` checksum, loads the snapshot, and replays only tail events after the checkpoint cursor

#### Scenario: Swappable VectorIndex backends satisfy pre-filtered recall SLOs
- **WHEN** queries filter HNSW candidates by `(embed_model_ver, dim_d, prompt_regime_id)` and `LiveVecBitmap ∩ VisibleScope` down to `1%` selectivity using `usearch` or fallback `hnsw_rs`
- **THEN** vector search executes traversal-time filtering and returns valid live candidates within the configured latency budget (`< 5 ms` p99 target on `10^6` `256d` vectors with `Recall@10 >= 0.93`)

### Requirement: Authorized hard-forget atomic erasure
As the sole explicit exception to append-only immutability, an authorized `hard_forget` command SHALL atomically scrub matching event, episode, and FTS5 records using SQLite `PRAGMA secure_delete = ON` and FTS5 `'secure-delete'=1`, purge/rebuild `VectorIndex` shards, and write a post-erasure snapshot anchored by Two-Epoch Replay.

#### Scenario: Hard-forget permanently scrubs matching records
- **WHEN** an authorized `hard_forget` command targets an event or entity identifier
- **THEN** SQLite secure deletion, FTS5 purge, vector removal, and post-erasure snapshot checkpointing complete atomically so erased content cannot resurrect on replay

#### Scenario: A failed write prevents the act
- **WHEN** writing an action to the log fails
- **THEN** the action is not handed to any body and the failure is reported

### Requirement: The log is append-only
Recorded events SHALL never be modified or removed by normal operation. A fact that changes
SHALL be recorded as a new event that supersedes the old one.

#### Scenario: A correction supersedes
- **WHEN** a recorded fact is later corrected
- **THEN** the original event is still in the log and a later event marks it as superseded

### Requirement: Events are totally ordered
Every recorded event SHALL receive a position in one total order, and reading the log SHALL
return events in that order.

#### Scenario: Events from two bodies interleave deterministically
- **WHEN** two bodies submit events that are recorded one after the other
- **THEN** every reader sees them in the order they were recorded

### Requirement: State is a projection of the log
All state the soul keeps SHALL be derivable from the log alone. Rebuilding state from the log
SHALL produce the same state as the running program held.

#### Scenario: State is rebuilt after loss
- **WHEN** all derived state is discarded and rebuilt from the log
- **THEN** open expectations, habituation, activation counts and identity equal their values before the loss

### Requirement: Decisions and model calls are events
A decision of the gate, including a decision to stay silent, SHALL be recorded as an event with
the numbers that produced it. Every model call SHALL be recorded with its purpose, model and
cost.

#### Scenario: Silence is on the record
- **WHEN** the gate decides an event is not worth reasoning about
- **THEN** the log contains a decision event for it with outcome silent or note, the salience, the threshold, the probability and the draw

### Requirement: Stored events carry their version
Every stored event SHALL carry the version of its format. An older version SHALL be upcast when
read and SHALL NOT be rewritten in the log. A version newer than the program knows SHALL fail
with a clear error.

#### Scenario: An old event is read
- **WHEN** the log contains an event in an older supported format
- **THEN** it is read as the current format and its stored bytes are unchanged

#### Scenario: A log from a newer program
- **WHEN** the log contains an event in a format newer than the program knows
- **THEN** start-up fails with an error naming the event format and both versions

### Requirement: Secrets never enter the log
Credentials and endpoint secrets SHALL NOT appear in any recorded event or in any receipt text.

#### Scenario: A failing model call is recorded
- **WHEN** a model call fails with an authentication error
- **THEN** the recorded event states the failure and the provider name and contains no key or token

### Requirement: The prior in effect is recorded in the log
`Core::open` SHALL append a `prior_loaded` event carrying the full prior content (format version,
content version and every disposition) before admitting any new input, whenever the log holds no
`prior_loaded` event yet or the given prior differs from the one most recently recorded. When the
given prior equals the recorded one, no event SHALL be appended.

#### Scenario: A soul is restarted with an edited prior
- **WHEN** a soul that ran under prior A is reopened with prior B
- **THEN** the log gains one `prior_loaded` event carrying B before the first new input, and earlier events stay attributed to A

#### Scenario: A soul is restarted with the same prior
- **WHEN** a soul is reopened with the prior already recorded in its log
- **THEN** no `prior_loaded` event is appended

### Requirement: A new prior governs from its event onward
A new prior SHALL govern decisions from its `prior_loaded` event onward; state learned before it
SHALL stay as recorded. `prior_loaded` events SHALL never be erased by `hard_forget`.

#### Scenario: Erasure leaves the prior record intact
- **WHEN** an authorized `hard_forget` erases events recorded under prior B
- **THEN** the `prior_loaded` event for B remains and post-erasure replay still applies B

### Requirement: Replay reproduces decisions
Replaying a recorded log with the same seed and switches SHALL produce the same gate
decisions, the same expectation transitions and the same recall rankings as the original run,
without calling a model. Replay SHALL take the prior from the log's `prior_loaded` events,
applying each from its position onward, and SHALL NOT take it from the caller.

#### Scenario: A recorded run is replayed
- **WHEN** a log is replayed with the seed and switches it was recorded with
- **THEN** every decision event produced by the replay equals the recorded one

#### Scenario: A log spanning two priors is replayed
- **WHEN** a log contains decisions under prior A, then a `prior_loaded` event for prior B, then further decisions
- **THEN** replay applies A before the event and B after it, and every decision equals the recorded one

#### Scenario: A divergence is reported
- **WHEN** a replay produces a decision that differs from the recorded one
- **THEN** the replay reports the first differing event and both values
