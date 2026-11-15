# Spec Delta

## Purpose

The cognitive loop is the mind's single thread of processing: it takes one event at a time,
records it, appraises it and carries out what follows. It is the same loop whatever is
switched off, and it runs the same way every time it is given the same events.

## ADDED Requirements

### Requirement: One queue and one consumer
Every source of events SHALL only put events into one queue. Only the loop SHALL process them,
one at a time, in order. Results of background work, including model results and receipts,
SHALL return as events through the same queue.

#### Scenario: Two bodies speak at once
- **WHEN** two bodies submit events at the same moment
- **THEN** the loop processes one completely before it starts the other

#### Scenario: A model result arrives while another event is processed
- **WHEN** reasoning finishes while the loop is processing a different event
- **THEN** the result waits in the queue and is processed as its own event

### Requirement: A slow reasoning call does not block the loop
The loop SHALL keep processing events while a model call is running.

#### Scenario: Events arrive during a long model call
- **WHEN** a model call takes longer than the arrival of three further events
- **THEN** all three events are recorded and appraised before the model call returns

### Requirement: The core knows only ids, numbers and descriptions
The core SHALL decide from opaque ids, numeric features and capability descriptions only. It
SHALL NOT read an event's payload, and SHALL have no notion of person, owner, group, role,
platform, plugin or model provider.

#### Scenario: The payload is replaced
- **WHEN** the same events are processed twice, the second time with every payload replaced by different bytes of the same features
- **THEN** every gate decision, expectation transition and recall ranking is identical

### Requirement: The core is autonomous with nothing at its doors
The core SHALL run a complete loop, including reasoning and acting, when no rule is applied at
its doors.

#### Scenario: The doors apply no rule
- **WHEN** no rule is installed at any door and an event allocated to the soul arrives
- **THEN** the event is recorded, reasoning wakes, and its action is carried out

### Requirement: Three doors are hook points for whatever governs the soul
Every percept SHALL pass the inbound door, every assembled context SHALL pass the context door
and every action SHALL pass the action door, each with all its facts. A door's rule SHALL be
able to remove context items, refuse a wake or refuse an action, and each such verdict SHALL be
recorded with the name of the rule.

#### Scenario: A rule refuses a wake
- **WHEN** a rule at the context door refuses a wake
- **THEN** no model call is made and the refusal is recorded with the rule's name

#### Scenario: A rule removes a context item
- **WHEN** a rule at the context door removes one item from a context
- **THEN** the model is shown the context without that item and the removal is recorded

#### Scenario: A rule refuses an action
- **WHEN** a rule at the action door refuses an action
- **THEN** the action ends in a receipt with outcome refused and no body receives it

### Requirement: The facts later policy needs are always recorded
The actor that started each chain of events and the audience of each event SHALL be recorded
for every event, whether or not any rule uses them. The origin SHALL be inherited along the
chain and SHALL NOT be settable by anything a model outputs.

#### Scenario: Reading an old event
- **WHEN** any recorded percept, thought, action or receipt is read
- **THEN** it states the actor that started its chain and the audience that could perceive it

#### Scenario: A model claims a different origin
- **WHEN** a reasoning result contains text or fields claiming another origin for its acts
- **THEN** the recorded origin of those acts is still the origin inherited from the event that caused the wake

### Requirement: Time and randomness are injected
The core SHALL take the current time only from an injected clock and randomness only from a
generator seeded from a configured seed and the event being processed.

#### Scenario: The same events on a different day
- **WHEN** a recorded sequence of events is processed again at a later wall-clock time with the same injected timestamps and seed
- **THEN** all decisions are identical to the first run

### Requirement: Every mechanism has a neutral value
Each mechanism of the core SHALL be registered with an id, a claim (what gets worse without
it) and a neutral value at which it has no effect. Switching a mechanism off SHALL mean setting
it to its neutral value. The loop SHALL run with any set of mechanisms at their neutral values.

#### Scenario: All mechanisms are neutral
- **WHEN** every registered mechanism is set to its neutral value and events arrive
- **THEN** the loop records every event and completes every turn without error

#### Scenario: A mechanism without a claim cannot be registered
- **WHEN** a mechanism is registered without a claim or without a neutral value
- **THEN** the program fails to build or fails its start-up check, naming the mechanism

### Requirement: Dispositions come from a versioned prior
The numeric dispositions the mechanisms use (such as talkativeness, spontaneity and memory
persistence) SHALL come from a prior that carries a format version and a content version. The
prior in use SHALL be identifiable in every recorded run.

#### Scenario: A prior value is out of range
- **WHEN** the prior sets a disposition outside its documented range
- **THEN** start-up fails with an error naming the disposition and its range

### Requirement: Idle time produces ticks
When no event arrives, the loop SHALL still receive periodic tick events carrying the current
time, so that time-based conditions are noticed without any external stimulus. A tick that changes
state SHALL be recorded; a tick that changes nothing SHALL leave no event.

#### Scenario: A due time passes while nothing happens
- **WHEN** no body reports anything and the due time of an open expectation passes
- **THEN** a tick is recorded and the expectation fires within one tick interval of its due time

#### Scenario: Nothing is due
- **WHEN** no body reports anything for an hour and no time-based condition falls due
- **THEN** the log gains no events during that hour

### Requirement: One part can fail without freezing the loop
A failure while processing one event SHALL be recorded and SHALL NOT stop the processing of
later events. Background work SHALL run under a time limit.

#### Scenario: Reasoning times out
- **WHEN** a model call exceeds its time limit
- **THEN** a failure event is recorded for that wake and the loop continues with the next event
