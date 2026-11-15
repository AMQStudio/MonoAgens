# Spec Delta

## Purpose

The soul's identity is who it is: its name, persona and principles. It is the core's own, it
always exists, and it changes only on the record. Restricting who may change it is policy,
added later in the valve.

## ADDED Requirements

### Requirement: An identity always exists
The soul SHALL always have an identity consisting of a name, a persona and principles. On
first start, when none is configured, a default identity SHALL be created.

#### Scenario: First start without an identity
- **WHEN** the program starts for the first time and the configuration names no identity
- **THEN** a default identity is created, written to the configuration and recorded

### Requirement: The configuration is the one source
The identity SHALL have exactly one source, in the configuration. What the soul learns SHALL
NOT be written into the identity.

#### Scenario: The owner edits the identity while the program is stopped
- **WHEN** the owner changes the persona in the configuration and starts the program
- **THEN** the soul uses the new persona and the change is recorded as an identity change made by the owner

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
- **WHEN** the owner configures a different model and restarts
- **THEN** no identity-change event is recorded and the identity in the context is unchanged

### Requirement: A running soul can be asked to change its identity
The gateway SHALL offer a capability, flagged as identity-changing, through which reasoning can
revise the identity. A successful use SHALL update the configuration and record the change.

#### Scenario: The owner asks for a new name in conversation
- **WHEN** the owner asks the soul to take a new name and reasoning uses the identity capability
- **THEN** the configuration and the identity in later contexts carry the new name and an identity-change event is recorded

#### Scenario: Someone else asks
- **WHEN** a source that is not the owner asks the soul to take a new name and reasoning uses the identity capability
- **THEN** the identity changes, and the identity-change event names that source as the origin of the change
