# Spec Delta

## Purpose

The appraisal gate decides, cheaply and without any model call, whether a moment deserves
reasoning. It is what lets the soul stay silent on purpose and keeps the cost of perceiving
far below the cost of thinking.

## ADDED Requirements

### Requirement: Appraisal uses no model call
The gate SHALL reach its decision for every event without calling a language model.

#### Scenario: A busy place the soul is not part of
- **WHEN** one hundred events arrive in a place and none is allocated to the soul or engages a commitment
- **THEN** one hundred decisions are recorded and the number of model calls caused by decisions with outcome silent or note is zero

### Requirement: Signals are bounded numbers from features
Each signal SHALL be a pure function of the current situation with a value from 0 to 1,
computed from numeric features and state only. The slice SHALL provide the signals own_name,
info_gap, feeling_of_knowing, relevance and social_cost.

#### Scenario: A feature is missing
- **WHEN** an event arrives without an embedding vector
- **THEN** every signal that needs the vector takes its neutral value and the gate still decides

### Requirement: Salience is a weighted, clamped sum
Salience SHALL be the sum of the signals multiplied by their weights, clamped to the range 0
to 1. The weights SHALL come from the prior.

#### Scenario: A negative weight lowers salience
- **WHEN** social_cost is high and its weight is negative
- **THEN** salience is lower than for the same event with social_cost at zero

### Requirement: Repeated stimuli habituate
For each pair of place and source, effective salience SHALL fall as stimuli from that source
repeat in that place, and SHALL recover as time passes without them.

#### Scenario: The same source repeats quickly
- **WHEN** one source sends ten similar events in one place within a minute
- **THEN** the effective salience of the tenth is lower than that of the first

#### Scenario: Recovery after a pause
- **WHEN** the same source returns after a long pause
- **THEN** the effective salience is close to that of a first stimulus

### Requirement: The threshold moves with drives
The threshold SHALL rise with the soul's recent share of talk in the place, with the share of
the daily budget already used and with unfamiliarity of the place and the sources present, and
SHALL fall with the share of the soul's promises that are due soon.

#### Scenario: The soul has just spoken a lot
- **WHEN** the soul's recent acts in a place exceed its fair share among the active participants
- **THEN** the threshold for the next event in that place is higher than before those acts

#### Scenario: The budget is nearly spent
- **WHEN** most of the daily budget has been used
- **THEN** the threshold is higher than at the start of the day for the same event

### Requirement: Go or no-go is a seeded draw
The gate SHALL turn effective salience and threshold into a probability and compare it with a
draw seeded from the configured seed and the event's id. The outcome SHALL be go, note or
silent, where a no-go with a probability below the note band is silent.

#### Scenario: The same event twice
- **WHEN** the gate appraises the same event with the same state and seed twice
- **THEN** both decisions have the same probability, the same draw and the same outcome

#### Scenario: Probabilities are honoured over many events
- **WHEN** many distinct events with the same probability of 0.3 are appraised
- **THEN** the share of go outcomes approaches 0.3

### Requirement: A commitment bypasses the draw
The gate SHALL wake reasoning without a draw when the event engages a commitment: the turn was
allocated to the soul, the event fires or answers an open expectation, or it continues an
active chain of the soul's own work.

#### Scenario: The soul is addressed
- **WHEN** an event arrives whose turn is allocated to the soul
- **THEN** the decision is go with the reason commitment, whatever the salience

#### Scenario: A promise falls due
- **WHEN** an open promise reaches its due time
- **THEN** the decision is go with the reason commitment

### Requirement: Each decision explains itself
Every decision event SHALL contain each signal's value, the salience, the habituation factor,
the threshold with its terms, the probability, the draw, the outcome and the reason.

#### Scenario: Reading a decision
- **WHEN** a decision is read from the log
- **THEN** its outcome can be recomputed from the numbers stored in that same event
