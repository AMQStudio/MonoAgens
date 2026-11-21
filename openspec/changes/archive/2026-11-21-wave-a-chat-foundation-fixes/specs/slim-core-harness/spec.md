# Spec Delta — slim-core-harness

## MODIFIED Requirements

### Requirement: Hard deterministic invariants bypass neural scoring
The Core harness SHALL enforce privacy (`VisibleScope`), Valve doors (`inbound`, `context`, `action`), hard daily budget caps, explicit commitments and due prospective deadlines, self-caused echo filtering, and bit-clean `hard_forget` erasure (including episode centroids, `SkillBook` lessons/procedures, and `LessonEntry` audience gating) deterministically, independent of neural controller outputs.

#### Scenario: Private memory never leaks into neural features or context
- **WHEN** an event arrives in a place whose `VisibleScope` excludes a previously stored private event or private lesson
- **THEN** the excluded private event and private lesson are filtered out before both numeric feature extraction and context assembly, producing zero information leakage (`Delta_NI = 0`)

#### Scenario: Commitment or due deadline forces wake
- **WHEN** an incoming percept directly allocates a turn to the soul, continues an active chain, or triggers a due prospective commitment
- **THEN** the harness immediately routes to `Go` with reason `Commitment` without allowing the neural controller to suppress the wake

#### Scenario: Self-caused echo is suppressed deterministically
- **WHEN** an incoming percept matches an open action effect prediction from the soul's own receipt
- **THEN** the harness marks the percept as `SelfCaused` and does not invoke reasoning or neural gating

#### Scenario: Partial hard-forget clears residual episode centroids and scrubs SkillBook event references
- **WHEN** `EventKind::HardForget` erases one or more `target_event_ids` belonging to a multi-event episode or cited by stored lessons/procedures
- **THEN** any surviving partial episode clears or recomputes its `centroid` strictly from remaining non-erased events, and `SkillBook` scrubs or removes all lessons and procedures citing the erased `target_event_ids` or `target_actor_ids`

## ADDED Requirements

### Requirement: Canonical multi-capability procedure schema matching
During context assembly, the Core harness SHALL validate every active procedure's `capability_schema_hash` against the active capability catalog using canonical combined-digest matching (identical to procedure extraction) rather than comparing the combined digest against individual capability hashes.

#### Scenario: Multi-capability procedure passes freshness check when catalog matches
- **WHEN** an active procedure requires two or more capabilities whose combined canonical `BLAKE3` digest matches `capability_schema_hash`
- **THEN** context assembly retains the procedure as eligible for tool selection rather than counting it as stale

#### Scenario: Multi-capability procedure is excluded when any required capability schema drifts
- **WHEN** one of a multi-capability procedure's required capabilities changes schema hash or becomes inactive
- **THEN** context assembly excludes the procedure and increments `excluded_stale_procedures`

### Requirement: Vector-indexed consolidated gists
When sleep consolidation records a gist memory entry for an episode that has a valid normalized `centroid` embedding, the Core harness SHALL attach that centroid as an immediate `TaggedEmbedding` on the gist `MemoryEntry` so vector-partitioned recall can retrieve consolidated gists without waiting for an external re-embed round trip.

#### Scenario: Consolidated gist is retrievable by vector recall
- **WHEN** sleep consolidation writes a gist for an episode whose `centroid` is present in the active embedding partition
- **THEN** the stored gist `MemoryEntry` carries a non-empty `embeddings` vector matching the episode centroid and is eligible during vector-partitioned recall

### Requirement: Event-sourced online adaptation and social-outcome Go feedback
The Core harness SHALL apply `PlaceAdaptState` mutations strictly through event-log projection (`CoreState::apply_event`) so replay reconstructs adaptation state identically, and SHALL NOT reward `GateArm::Go` positively solely upon transport delivery (`ReceiptOutcome::Done`).

#### Scenario: Transport delivery does not prematurely reward Go
- **WHEN** a communicative action settles with `ReceiptOutcome::Done`
- **THEN** Core does not immediately apply a positive `GateArm::Go` reward on delivery alone, while non-fault `Failed` or `Refused` receipts still record bounded negative feedback

#### Scenario: Replay reproduces PlaceAdaptState bit-identically
- **WHEN** an event log containing online adaptation feedback events is replayed from scratch or from a snapshot plus trailing log
- **THEN** the reconstructed `PlaceAdaptState` for every place matches the live run bit-identically

### Requirement: Reproducible Markdown export of learned state
The CLI SHALL provide a zero-API offline command (`monoagens export-learning`) that replays the event log deterministically and exports learned state (`PlaceAdaptState`, `SkillBook` procedures and lessons, `BeliefBook`, and `EpisodeBook` gists) into a canonically ordered Markdown file with a `BLAKE3` audit digest, excluding private-audience items unless `--include-private` is explicitly passed and containing zero traces of `hard_forget`-erased events or actors.

#### Scenario: Repeated exports over the same log are bit-identical
- **WHEN** `monoagens export-learning --out <path>` is executed twice against the same store or `--log-json` input
- **THEN** both exported Markdown files are byte-for-byte identical and report the same `BLAKE3` digest

#### Scenario: Private-audience items are excluded by default and included only with explicit opt-in
- **WHEN** `monoagens export-learning` runs against a log containing both multi-party group items and private DM items
- **THEN** default invocation omits private-audience items from the Markdown output, while `--include-private` includes them with explicit `Audience` labels

#### Scenario: Hard-forgotten events and actors leave zero traces in Markdown export
- **WHEN** an event or actor has been erased via `EventKind::HardForget` and `monoagens export-learning --include-private` is run
- **THEN** zero bytes or identifiers from the erased event, actor, lesson, procedure, or gist appear in the exported Markdown file
