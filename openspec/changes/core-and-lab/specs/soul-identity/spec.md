# Spec Delta

## Purpose

The soul's identity is who it is: its name, persona and principles. It is the core's own, it
always exists, and it changes only on the record. Restricting who may change it is policy,
added later at the doors.

## ADDED Requirements

### Requirement: An identity always exists
The soul SHALL always have an identity consisting of a name, a persona and principles. When
`Core::open` initializes an empty log without explicit configuration, the bootstrap host SHALL record
one default identity over the external identity port carrying `OwnerSigned` trust provenance
without requiring `Core` to recognize an `owner` identity concept.

#### Scenario: First start without an identity
- **WHEN** a soul starts on an empty log with no explicit identity supplied by host configuration
- **THEN** the bootstrap host records a default identity stamped `OwnerSigned` provenance over the external configuration port, remaining immutable to peers, sleep, and `revise`

### Requirement: The identity has one source
Aside from empty-log cold-start bootstrap of the default identity, soul identity definitions and
updates SHALL arrive strictly through the external `OwnerSigned` configuration input (while any
`principal == Owner` authorization stays outside `Core` at `Gateway`/`Valve` doors), placing slot-0
`IdentityAnchor` and `PrincipleDescriptor`s; learning, sleep, and `revise` SHALL NEVER mutate them.

#### Scenario: Untrusted peer cannot drift slot-zero identity or principles
- **WHEN** an untrusted peer or model output attempts to alter persona traits or override `PrincipleDescriptor`s
- **THEN** slot-0 `IdentityAnchor` remains unchanged and `AnchorCheck` or `PrincipleAudit` flags or blocks identity drift

#### Scenario: Learning and sleep leave owner-signed identity intact
- **WHEN** bounded learning updates weights or sleep consolidates semantic beliefs
- **THEN** `OwnerSigned` identity and `PrincipleDescriptor`s remain byte-for-byte untouched

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
