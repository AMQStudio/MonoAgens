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
- **THEN** on the next start the event is in the log and is processed

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

### Requirement: Replay reproduces decisions
Replaying a recorded log with the same seed, prior and switches SHALL produce the same gate
decisions, the same expectation transitions and the same recall rankings as the original run,
without calling a model.

#### Scenario: A recorded run is replayed
- **WHEN** a log is replayed with the seed, prior and switches it was recorded with
- **THEN** every decision event produced by the replay equals the recorded one

#### Scenario: A divergence is reported
- **WHEN** a replay produces a decision that differs from the recorded one
- **THEN** the replay reports the first differing event and both values
