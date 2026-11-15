# Spec Delta

## Purpose

Bounded learning lets the soul adjust itself from the outcomes of its own decisions, slowly
and inside limits. Every adjustment is an event, so what the soul has learned can be read,
replayed and rolled back.

## ADDED Requirements

### Requirement: Learning uses the outcome of a prediction
The teaching signal for a decision SHALL be the outcome of the prediction it opened. Good: a
response prediction answered in time with no content miss and no repair, or an effect
prediction matched. Bad: an absence, a content miss, a repair, a failed receipt, a negative
flag from the body, or a judged silence that was missed. A decision with no resolved
prediction SHALL teach nothing.

#### Scenario: A reply that landed
- **WHEN** the soul spoke on an unallocated turn and its response prediction was answered in time without a miss
- **THEN** that decision is available to learning with a good outcome

#### Scenario: A negative flag
- **WHEN** a body reports a negative flag on the soul's act
- **THEN** that decision is available to learning with a bad outcome

#### Scenario: Nothing came back yet
- **WHEN** a decision's prediction is still open
- **THEN** learning does not use that decision

### Requirement: Signal weights follow the delta rule
When weight learning is enabled, each signal weight SHALL move by the learning rate times the
difference between the outcome and the probability the gate had given, times that signal's
value, clamped to the weight's range.

#### Scenario: Speaking that was not welcome
- **WHEN** the gate gave a high probability, the soul spoke, and the outcome was bad
- **THEN** the weights of the signals that were high for that decision decrease

#### Scenario: The learning rate is zero
- **WHEN** the learning rate is at its neutral value of zero
- **THEN** no weight changes

### Requirement: Knobs move only on enough evidence
When knob learning is enabled, a disposition SHALL move only when the 95% interval of the
evidence excludes "no change".

#### Scenario: Three observations
- **WHEN** only a handful of outcomes point one way
- **THEN** the disposition does not move

### Requirement: Every step and every day is bounded
Each change to a disposition SHALL be at most the configured share of its range, the total
change in one day SHALL be at most five percent of its range, and no disposition SHALL leave
its floor and ceiling.

#### Scenario: Strong evidence in one day
- **WHEN** a day's evidence would justify a large move
- **THEN** the disposition's total change that day is no more than five percent of its range

### Requirement: A disposition stuck at a bound freezes and is reported
A disposition that sits at its floor or ceiling for the configured number of days SHALL stop
learning and SHALL be reported.

#### Scenario: Pinned at the ceiling
- **WHEN** a disposition has been at its ceiling for the configured number of days
- **THEN** a report event names it, and it no longer changes until released

### Requirement: Global values learn by teacher weight and scoped values from everyone
Each source SHALL have a teacher weight supplied from outside the core. An outcome SHALL move
global weights and dispositions in proportion to the teacher weight of the source involved,
and recorded classification verdicts SHALL count in full. Offsets scoped to one place or one
source SHALL learn from the outcomes of everyone there, by the same bounded rule.

#### Scenario: A stranger's reactions
- **WHEN** a source with teacher weight zero reacts badly to the soul's messages many times
- **THEN** the global values are unchanged, and the offset for that place or source changes

#### Scenario: A teacher's reactions
- **WHEN** a source with a high teacher weight reacts badly many times
- **THEN** the global values move, within their bounds

### Requirement: The lower wake band learns from verdicts
When band learning is enabled, the lower band of the wake gate SHALL move toward the lowest
similarity that received a positive classification, less a margin, within its bounds and by
the same bounded steps.

#### Scenario: Positive verdicts appear lower than the band expected
- **WHEN** events just above the lower band are repeatedly classified as meeting their condition
- **THEN** the lower band moves down by bounded steps

### Requirement: Each change is an event that carries its own delta
Every learned change SHALL be recorded as an event stating what changed, by how much and on
which evidence, without needing any message content to interpret it.

#### Scenario: Reading what was learned
- **WHEN** the log is filtered for learning events
- **THEN** the current value of every learned weight and disposition equals its prior value plus the recorded deltas

### Requirement: Learned changes can be rolled back
It SHALL be possible to return any learned value to an earlier state by recording a reversing
event, without editing the log.

#### Scenario: Undoing a week of learning
- **WHEN** a rollback to the state of seven days ago is requested
- **THEN** the values equal those of seven days ago and the log contains the rollback event after the changes it reverses

### Requirement: Learning is off by default in measurement
Each kind of learning (weights, knobs, wake band) SHALL have its own switch, and with all
switches off the soul's dispositions SHALL stay exactly at the prior.

#### Scenario: A run with learning off
- **WHEN** a run is made with all learning switches off
- **THEN** the log contains no learning event
