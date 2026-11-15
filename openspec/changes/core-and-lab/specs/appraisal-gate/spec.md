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
computed from numeric features and state only. The gate SHALL provide the signals own_name,
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
repeat in that place, and SHALL recover as time passes without them. The strength of
habituation SHALL be a gain that, at zero, leaves salience unchanged.

#### Scenario: The same source repeats quickly
- **WHEN** one source sends ten similar events in one place within a minute
- **THEN** the effective salience of the tenth is lower than that of the first

#### Scenario: Recovery after a pause
- **WHEN** the same source returns after a long pause
- **THEN** the effective salience is close to that of a first stimulus

#### Scenario: The habituation gain is zero
- **WHEN** the habituation gain is zero and one source repeats ten times
- **THEN** effective salience equals salience for each of the ten events

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

### Requirement: Patience before taking an unallocated turn
A go on a turn that was not allocated to the soul SHALL NOT wake reasoning at once. It SHALL
open a wait: if a source other than the sender replies in that place within the patience time,
the decision SHALL end as silent; otherwise reasoning SHALL wake.

#### Scenario: Someone else answers first
- **WHEN** the gate decides go on an unallocated question and another participant answers within the patience time
- **THEN** no model call is made and the decision is recorded as silent with the reason yielded

#### Scenario: Nobody answers
- **WHEN** the patience time passes with no reply from anyone but the sender
- **THEN** reasoning wakes for that event

#### Scenario: An allocated turn does not wait
- **WHEN** the turn was allocated to the soul
- **THEN** reasoning wakes without a wait

### Requirement: A backlog in one place is appraised as one chunk
Events of one place that reach the loop later than that place's patience time, or that arrived
while the soul was stopped or asleep, SHALL be appraised as one chunk with one decision. Each
signal of the chunk SHALL be the highest of its members' values after each is discounted by
its age with the decay that memory uses. At most four chunks SHALL be handled in one wake, the
most salient first.

#### Scenario: Thirty missed messages
- **WHEN** thirty events of one place arrive together, hours after they happened, none allocated to the soul
- **THEN** one decision is recorded for the chunk, not thirty, and all thirty events are in the log

#### Scenario: An allocated turn inside a backlog
- **WHEN** one event in a backlog was allocated to the soul
- **THEN** the chunk's decision is go with the reason commitment

#### Scenario: Six places have a backlog
- **WHEN** six places each have a backlog after downtime
- **THEN** the first wake handles the four most salient chunks and the other two are handled afterwards

### Requirement: Closeness comes from exchanges, familiarity from presence
Closeness to a source SHALL be the activation of the soul's exchanges with it: events it
addressed to the soul, the soul's acts addressed to it, and reply-linked pairs. Familiarity of
a place SHALL combine the activation of the soul's presence there with its closeness to those
present. Being in the same place SHALL NOT by itself count as an exchange. Both SHALL fade when
contact stops.

#### Scenario: A large channel
- **WHEN** the soul is present in a place with hundreds of sources it never exchanged with
- **THEN** its closeness to each of them is zero

#### Scenario: A place left long ago
- **WHEN** the soul has not been present in a place for a long time
- **THEN** the familiarity of that place is lower than it was during regular contact

### Requirement: The floor of others carries a cost
The social_cost signal SHALL be high when an event selected someone other than the soul as the
next speaker, or when a third party has already responded to it, and SHALL be lowered by the
soul's closeness to the sender.

#### Scenario: Another person was selected
- **WHEN** an event is addressed to a source other than the soul, from a sender the soul has no history with
- **THEN** social_cost is at its maximum

#### Scenario: Someone already answered
- **WHEN** a third party has responded to the event before the soul appraises it
- **THEN** social_cost is at its maximum for a sender the soul has no history with

#### Scenario: A close sender
- **WHEN** the same event comes from a sender the soul has exchanged with often
- **THEN** social_cost is lower than for a stranger
