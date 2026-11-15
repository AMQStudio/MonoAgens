# Spec Delta

## Purpose

Sleep is when the soul turns what happened into what it knows. While idle it groups events
into episodes, writes their gist, links and supersedes beliefs and lets unused detail fade,
without ever replacing the record itself.

## ADDED Requirements

### Requirement: Consolidation runs asynchronously on immutable snapshots when idle
Consolidation SHALL start when idle `>= T_idle` and unconsolidated events `>= N_min` (`A-01`,
not `>`), reading an immutable snapshot `t_snapshot` without delaying live events (`Δθ_sleep = 0`,
`A-04`).

#### Scenario: A message arrives during sleep
- **WHEN** an event arrives while snapshot-isolated consolidation is running
- **THEN** the event is appraised at normal `θ` (`Δθ_sleep = 0`) and wakes immediately without waiting for consolidation

#### Scenario: Exactly N_min unconsolidated events at idle
- **WHEN** unconsolidated events equal `N_min` and global idle reaches `T_idle`
- **THEN** background consolidation triggers without stalling until `N_min + 1`

### Requirement: Episodes use normalized EM-LLM boundaries and provisional lifecycle
Events SHALL group per place into `{open, provisional, sealed}` episodes (`A-05`) split on `[0, 1]`
normalized `EM-LLM` surprise with cold-start shrinkage (`A-06`), never on fixed intervals. Sleep
SHALL soft-close qualifying idle tails into `provisional` (reopenable with `superseded_by` on arrival).

#### Scenario: Pre-sleep open tail avoids overnight spin loop
- **WHEN** the soul goes idle before sleep with `>= N_ep_min` events in an unclosed conversation tail
- **THEN** sleep soft-closes the episode as `provisional`, writes its gist, and leaves `0` unconsolidated backlog so no spin loop occurs

#### Scenario: Conversation resumes seamlessly after sleep
- **WHEN** a new event arrives within continuity bounds against a `provisional` episode
- **THEN** the episode reopens to `open`, marks the stale gist `superseded_by`, and preserves boundary continuity

### Requirement: Each sealed or provisional episode yields a provenance-preserving gist
For each `sealed` or `provisional` episode, `consolidate` SHALL write a gist linked to source
event ids, preserving raw events intact and propagating `min(trust)` taint (`R-43`) so stranger
text never becomes trusted identity knowledge.

#### Scenario: Reading a gist from mixed-trust conversation
- **WHEN** a consolidated episode includes an untrusted external actor's messages
- **THEN** every source event remains intact, reachable from the gist, and the gist inherits untrusted taint (`min(trust)`)

### Requirement: Episodes merge only when people, place and time agree
Consolidation SHALL merge two episodes or their gists only when the place is the same, their
participants overlap strongly and their times overlap or are adjacent for that place.
Similarity of meaning alone SHALL NOT merge them.

#### Scenario: Two similar conversations with different people
- **WHEN** two episodes are about the same subject and involve different sources in different places
- **THEN** they stay separate memories

### Requirement: Consolidation supersedes stale beliefs
When an episode contradicts an existing belief, consolidation SHALL record the new belief and
mark the old one as ended, pointing to the new one.

#### Scenario: A plan was cancelled
- **WHEN** an earlier belief says an event will happen and a later episode says it was cancelled
- **THEN** after sleep the earlier belief is marked ended and recall presents the cancellation as current

### Requirement: Unused detail fades on Petrov base activation B while the record stays
During sleep, vectors of events with Petrov base activation `B < fade_threshold` (`−3.29`, `A-01`,
never undefined total `A`) SHALL release via `vector_released`, while events, FTS5 text, and
gists remain intact. Domain centroids warm-start and align via Hungarian matching (`A-12`).

#### Scenario: An old unused event fades after 30 days without domain ID shuffling
- **WHEN** an event reaches `B < −3.29` and sleep recomputes domain clusters
- **THEN** its vector releases (`vector_released`), text search and gist links still resolve it, and `domains_updated` preserves cluster ID continuity via Hungarian matching

### Requirement: Sleep is on the record and within budget
Every consolidation run SHALL be recorded with the events it covered, what it produced and its
cost, and its model calls SHALL be counted like any other.

#### Scenario: Cost of a night
- **WHEN** a consolidation run completes
- **THEN** the log states which events it covered, which gists and belief changes it produced, and the tokens it used

### Requirement: Consolidation is interruptible and resumable
If the program stops during consolidation, the next run SHALL continue with the events not yet
consolidated, and no episode SHALL be consolidated twice.

#### Scenario: Stopped mid-sleep
- **WHEN** the program stops after some episodes were consolidated and starts again
- **THEN** the remaining events are consolidated later and no gist is duplicated

### Requirement: Contrastive lessons and schema-hashed procedures retire rather than vanish
Shared opaque failure clusters paired with successes (`A-11`) SHALL yield contrastive `lesson`s.
Cross-place `goal_id`/`caused_by` chains resolved as `reached` SHALL yield slot-parameterized
`procedure`s with `capability_schema_hash` (`R-38`), CUSUM failure drift, and non-destructive
retirement (`allow_chat_habit_bypass = false`).

#### Scenario: The same mistake twice yields contrastive lesson
- **WHEN** two episodes contain matching opaque prediction-error signatures alongside a contrasting successful trace
- **THEN** a contrastive lesson links to those episodes with `(α, β)` credence and taint provenance

#### Scenario: A cross-place goal resolved as reached yields validated procedure
- **WHEN** an active goal across places resolves with outcome `reached` and consistent capability schema hashes
- **THEN** a slot-parameterized procedure records with credence, enters `CUSUM` monitoring, and retires safely upon capability schema drift or repeated failures

#### Scenario: A single error
- **WHEN** a prediction error appears in one episode only
- **THEN** no lesson is recorded for it
