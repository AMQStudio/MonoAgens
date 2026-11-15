# Spec Delta

## Purpose

Memory recall selects what the soul remembers for a moment. It ranks memories by activation
(how recently and how often they were used, and how well they fit the present), so that what
matters stays reachable for years without a session transcript.

## ADDED Requirements

### Requirement: Recall reaches the whole log
Recall SHALL be able to return any recorded item, however old, without it having been part of
a recent conversation.

#### Scenario: A fact from long ago
- **WHEN** a fact was recorded and thirty days pass with many unrelated events
- **THEN** a query about that fact, phrased in different words, returns it among the top results

### Requirement: Candidates come from meaning and from wording
Recall SHALL gather candidates both by vector similarity and by text match, and SHALL rank the
union.

#### Scenario: Different words, same meaning
- **WHEN** the query shares meaning but no words with a memory and vectors are available
- **THEN** the memory is among the candidates

#### Scenario: A rare exact term
- **WHEN** the query contains a rare term that a memory contains exactly
- **THEN** the memory is among the candidates even if its vector is not among the nearest

### Requirement: Ranking is by activation
Candidates SHALL be ranked by total activation: a base level that grows with the number of
uses and decays with age as a power law, plus spreading activation from the current context,
plus importance.

#### Scenario: Often used beats merely similar
- **WHEN** two memories are equally similar to the context and one has been used far more often
- **THEN** the often-used one ranks higher

#### Scenario: Old unused memories fade but stay reachable
- **WHEN** a memory has not been used for a long time
- **THEN** its activation is lower than when it was fresh, and it is still returned when it is the only match

### Requirement: Use strengthens a memory
When a memory is placed into a context, that use SHALL be recorded and SHALL raise the memory's
later base-level activation.

#### Scenario: A recalled memory is recalled more easily
- **WHEN** a memory is used in a context and the same query is made again later
- **THEN** its base-level activation is higher than if it had not been used

### Requirement: Recall works without vectors
When no embedding is available, recall SHALL still return candidates by text match and rank
them by base-level activation and importance.

#### Scenario: No embedding provider is configured
- **WHEN** the soul runs without an embedding provider and is asked about a recorded fact using one of its words
- **THEN** the fact is returned

### Requirement: Pre-Step-5 VisibleScope filtering enforces privacy non-interference
All candidate events, summaries, and activations SHALL be projected through `VisibleScope`
(`audience(context) ⊆ audience(item)` or explicit logged `DeclassToken`) **before**
feeling-of-knowing scoring, top-`k` recall selection, and activation use-count updates (`R-33`).

#### Scenario: A private memory never leaks into Step-4 FOK or Step-5 gate urgency
- **WHEN** an event arrives in a public channel while a highly relevant private secret exists in a narrower audience without a `DeclassToken`
- **THEN** the private secret is excluded before FOK and top-`k` ranking, its use count stays unchanged, and gate decisions match an empty private history (`Delta_NI == 0`)

#### Scenario: An owner-signed DeclassToken grants scoped cross-audience recall
- **WHEN** a logged `DeclassToken` explicitly authorizes named source items for a target place
- **THEN** those authorized items become visible inside `VisibleScope` for that target only while unrelated private items remain hidden

### Requirement: Recall preserves provenance and blind AttachmentHandles
Each visible recalled item SHALL carry its source event ids, time, place, actor, `basis`,
`taint`, audience, and opaque `AttachmentHandle` (`R-37`), with raw bytes never readable in core.

#### Scenario: Recalled item carries verifiable provenance
- **WHEN** recall returns an episode summary or event across places
- **THEN** it includes source event ids, provenance `basis`, and `taint` for `ContextPlan` rendering and Door checks

### Requirement: Reasoning can ask for recall explicitly
The core SHALL offer its own capability, used through act, for recalling from the whole log by
a query. It SHALL pass the action door like any action, and its result SHALL return as an
event in the same chain of work.

#### Scenario: Reasoning needs more before answering
- **WHEN** reasoning uses the recall capability with a query
- **THEN** the next wake of that chain has the recalled items in its context

#### Scenario: A rule refuses the recall
- **WHEN** a rule at the action door refuses a recall
- **THEN** the act ends in a receipt with outcome refused and nothing is recalled

### Requirement: Dual-process familiarity and recollection without 30-day starvation
Recall SHALL separate continuous familiarity (`A` + similarity) from episodic recollection (`R` =
recovered source event matching people, place, and time grain), without blocking long-gap
familiarity hits behind mandatory LLM verification (`A-09`). Explicit Row-10 weight `w_B = 0`
SHALL yield pure similarity ranking.

#### Scenario: Only a gist remains after thirty days
- **WHEN** a query matches a summary at a thirty-day gap and its source events remain valid or match via familiarity
- **THEN** automatic recall retrieves the memory cleanly tagged as familiar-only unless full verbatim detail recovers `R`, never dropping valid long-horizon recall

#### Scenario: The original events are recovered
- **WHEN** a query recovers recorded events matching the claimed people, place and time grain
- **THEN** the items are marked recollected

#### Scenario: The vectors have faded
- **WHEN** a summary's source events no longer have vectors (`B < -3.29`) and remain in the log
- **THEN** the summary can still verify recollection through its source event links

#### Scenario: Row-10 activation weight neutral switch
- **WHEN** `w_B` is set to zero for ablation
- **THEN** recall ranking depends strictly on similarity and importance with base-level zeroed

### Requirement: Cognitive map links and unified 256d index isolation
Core SHALL maintain opaque Successor-Representation / co-occurrence links across `{actor, place,
concept}` ids (`R-32`) inside existing books, and Gateway `usearch` vector lookups SHALL filter
by `VisibleScope` and matching `(embed_model_ver, dim_d, prompt_regime_id)` (`R-37`, `R-40`).

#### Scenario: Multi-hop recall across incompatible embedder versions
- **WHEN** older `EmbeddingGemma-300M` vectors coexist with `EmbeddingGemma 2` `256d` vectors and SR graph edges
- **THEN** vector search queries only matching `embed_model_ver` vectors while SR graph links bridge related actors and places without reading text

### Requirement: Superseded items do not pass as current
Recall SHALL NOT return an item that has been superseded as if it were current. When a
superseded item is returned, it SHALL be marked as no longer valid and point to what replaced
it.

#### Scenario: An address changed
- **WHEN** a source's old address was superseded by a new one and the address is asked for
- **THEN** the new address ranks as current and the old one, if returned, is marked superseded

### Requirement: Importance comes from reasoning, commitment and surprise
An item's importance SHALL be the highest of: the importance reasoning gave it when writing it,
full importance when it is linked to a promise, goal or expectation, and its surprise at the
time it was recorded.

#### Scenario: A promise among routine events
- **WHEN** an event linked to a promise and a routine event are equally old and equally similar to the query
- **THEN** the linked one ranks higher
