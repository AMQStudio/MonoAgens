# Spec Delta

## Purpose

Expectations are what the soul holds about the future: promises it made and predictions of
what should follow its acts. Their lifecycle lives in code, not in the model's attention, so a
promise is kept even when nobody mentions it again.

## ADDED Requirements

### Requirement: An expectation is a typed record
An expectation SHALL be a record with a kind (promise or prediction), a condition, the place
and source it concerns, an expiry and the event that created it. Its lifecycle SHALL be driven
by the loop, not by a model call.

#### Scenario: A promise is opened
- **WHEN** reasoning commits to doing something at a later time
- **THEN** an open promise with that due time, place and source is recorded

### Requirement: A time condition fires on its own
An expectation whose condition is a due time SHALL fire when the injected clock reaches that
time, with no external event needed, and firing SHALL wake reasoning as a commitment.

#### Scenario: A reminder comes due
- **WHEN** the soul promised a reminder in two hours and two hours pass in silence
- **THEN** reasoning wakes with the promise in its context and no earlier than the due time

#### Scenario: The soul was not running at the due time
- **WHEN** a promise falls due while the program is stopped and the program starts later
- **THEN** the promise fires at start-up and the lateness is recorded

### Requirement: A fired promise stays open until resolved
A promise SHALL remain open after firing until reasoning resolves it or revises it. If the
wake it caused fails, the promise SHALL fire again after a delay, and SHALL NOT be dropped
silently.

#### Scenario: The model is unreachable when a promise fires
- **WHEN** the wake caused by a fired promise fails
- **THEN** the promise is still open and fires again after the retry delay

#### Scenario: A promise is kept
- **WHEN** reasoning carries out the promised act and resolves the promise
- **THEN** the promise is recorded as resolved with a reference to the act that kept it

### Requirement: Every act opens a prediction
After each action, the loop SHALL open a prediction of what should follow, toward the source
and place the act was aimed at, with an expiry.

#### Scenario: The expected reply comes
- **WHEN** the source the soul acted toward responds in that place before the expiry
- **THEN** the prediction is recorded as answered and the response wakes reasoning as a commitment

#### Scenario: Nothing comes
- **WHEN** the expiry passes without a response
- **THEN** the prediction is recorded as expired, as a noticed absence

### Requirement: The echo of the soul's own act is not a new stimulus
An incoming event that matches the outcome of the soul's own open prediction of an act SHALL be
marked as self-caused and SHALL NOT be appraised as a new stimulus.

#### Scenario: A body reports the soul's own message back
- **WHEN** a body reports as a percept the very message the soul just sent through it
- **THEN** the percept is recorded as self-caused and causes no reasoning

### Requirement: Expectations can be revised and resolved by reasoning
Reasoning SHALL be able to resolve an open expectation or revise its condition or expiry. Each
change SHALL be recorded with the wake that made it.

#### Scenario: A reminder is moved
- **WHEN** the source asks to move a promised reminder to a later time
- **THEN** the promise's due time is revised, the old due time no longer fires and the revision is recorded

### Requirement: Promise debt is visible to the gate
The share of the soul's open promises that fall due soon SHALL be available to the gate as a
number.

#### Scenario: Several promises are due within the hour
- **WHEN** most open promises fall due within the next hour
- **THEN** the promise debt the gate sees is higher than when none is due
