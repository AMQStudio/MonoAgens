# Spec Delta

## Purpose

The soul's identity is who it is: its name, persona and principles. It is the core's own, it
always exists, and it changes only on the record. Restricting who may change it is policy,
added later at the doors.

## ADDED Requirements

### Requirement: An identity always exists
The soul SHALL always have an identity consisting of a name, a persona and principles. When
none is given at first start, a default identity SHALL be created and recorded.

#### Scenario: First start without an identity
- **WHEN** a soul is started for the first time with no identity given
- **THEN** a default identity is recorded and used

### Requirement: The identity has one source
The identity SHALL have exactly one source, given from outside the core when the soul starts.
What the soul learns SHALL NOT be written into the identity.

#### Scenario: The identity is edited between runs
- **WHEN** the persona given at start differs from the last recorded one
- **THEN** the soul uses the new persona and the change is recorded

### Requirement: Every identity change is recorded
Each change to the identity SHALL be recorded as an identity-change event that states what
changed, when and on whose origin.

#### Scenario: Reading the history of the name
- **WHEN** the name was changed twice
- **THEN** the log contains two identity-change events with the old and new names in order

### Requirement: Identity is present in every context
Every context built for reasoning SHALL contain the current identity.

#### Scenario: Any wake
- **WHEN** reasoning wakes for any reason
- **THEN** the context contains the soul's current name, persona and principles

### Requirement: Identity is independent of body and model
The identity SHALL be the same through every body and SHALL NOT change when the model provider
or model changes.

#### Scenario: The model is swapped
- **WHEN** the same soul is run with a different model
- **THEN** no identity-change event is recorded and the identity in the context is unchanged

### Requirement: The core treats identity as opaque
The core SHALL carry the identity without reading it. The soul's name SHALL reach the gate only
as a feature reported with an event, never by the core searching text.

#### Scenario: The name is said in a message
- **WHEN** a body reports that an event names the soul
- **THEN** the gate uses that reported feature, and the core does not inspect the message for the name
