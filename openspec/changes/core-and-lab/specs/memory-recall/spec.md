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

### Requirement: Recall returns provenance
Each recalled item SHALL carry the id of the event it comes from, its time, its place, its
actor and its audience, so that a policy at the context door can judge what may be shown and to
whom. Recall itself SHALL NOT leave an item out because of its audience.

#### Scenario: A recalled item comes from a narrower audience
- **WHEN** recall returns an item whose audience does not include the present audience
- **THEN** the item is returned with its audience stated, so that a rule at the context door could remove it

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

### Requirement: Recollection is told apart from familiarity
A recalled item SHALL be marked recollected only when at least one recorded event, not a
summary, is recovered whose actor or audience contains the people concerned, whose place
matches and whose time falls within the grain claimed. Otherwise it SHALL be marked familiar
only. A summary whose events have lost their vectors SHALL still be tested through the events
it links to.

#### Scenario: Only a gist remains
- **WHEN** a query matches a summary and no recorded event matching its people, place and time is recovered
- **THEN** the item is marked familiar only

#### Scenario: The original events are recovered
- **WHEN** a query matches recorded events with matching people, place and time
- **THEN** the items are marked recollected

#### Scenario: The vectors have faded
- **WHEN** a summary's source events no longer have vectors and are still in the log
- **THEN** the summary can still be marked recollected through its links

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
