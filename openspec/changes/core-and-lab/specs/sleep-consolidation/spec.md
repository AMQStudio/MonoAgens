# Spec Delta

## Purpose

Sleep is when the soul turns what happened into what it knows. While idle it groups events
into episodes, writes their gist, links and supersedes beliefs and lets unused detail fade,
without ever replacing the record itself.

## ADDED Requirements

### Requirement: Consolidation runs only when idle
Consolidation SHALL start only when the loop has been idle for the configured time and
unconsolidated events exceed the configured backlog. It SHALL run as background work whose
results return as events, and SHALL never delay the appraisal of an incoming event.

#### Scenario: A message arrives during sleep
- **WHEN** an event allocated to the soul arrives while consolidation is running
- **THEN** the event is appraised and reasoning wakes without waiting for consolidation to finish

#### Scenario: A busy period
- **WHEN** events keep arriving without an idle period
- **THEN** no consolidation starts

### Requirement: Episodes end at event boundaries
Unconsolidated events SHALL be grouped per place into episodes whose boundaries fall where
surprise rises above its recent level in that place. Surprise SHALL be the highest of the
prediction error by time, the prediction error by content and the novelty of the event against
the episode so far. Without vectors, a boundary SHALL fall at a marked change of participants,
an unusually long gap for the place, or a missed prediction. A fixed interval SHALL NOT end an
episode.

#### Scenario: A topic changes abruptly
- **WHEN** a conversation in one place turns to an unrelated matter
- **THEN** the events before and after the turn fall into different episodes

#### Scenario: A long, steady conversation
- **WHEN** a conversation continues on one matter for hours
- **THEN** it forms one episode

#### Scenario: Without vectors
- **WHEN** no vectors are available and the participants of a place change almost entirely
- **THEN** a boundary falls at that change

### Requirement: Each episode yields a gist that points to its events
For each closed episode, consolidation SHALL record a gist with the sources, place and time
span of the episode and the ids of the events it rests on. The events SHALL remain in the log
unchanged.

#### Scenario: Reading a gist
- **WHEN** a gist is read
- **THEN** every event it summarizes can be reached from it, and each is unchanged

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

### Requirement: Unused detail fades and the record stays
During sleep, the per-event vectors of events whose activation has fallen below the fade
threshold SHALL be released. The event, its text and its gist SHALL remain.

#### Scenario: An old, unused event
- **WHEN** an event has not been used for a long time and its activation is below the threshold
- **THEN** it is no longer found by vector similarity, is still found by its words, and is still reachable from its gist

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

### Requirement: Recurring errors become lessons and successful sequences become procedures
When two or more episodes share a prediction error, consolidation SHALL record a lesson as a
situation, an action and an outcome. When a sequence of actions reached a goal, it SHALL record
that sequence as a procedure. Both SHALL link to their source events, carry credence and be
recallable by reasoning.

#### Scenario: The same mistake twice
- **WHEN** two episodes contain the same kind of prediction error
- **THEN** a lesson is recorded that links to both episodes

#### Scenario: A goal was reached
- **WHEN** an episode contains a goal that was resolved as reached
- **THEN** a procedure is recorded with the actions that led there, in order

#### Scenario: A single error
- **WHEN** a prediction error appears in one episode only
- **THEN** no lesson is recorded for it
