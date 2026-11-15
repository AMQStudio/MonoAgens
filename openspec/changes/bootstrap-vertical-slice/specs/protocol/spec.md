# Spec Delta

## Purpose

The protocol is the one contract between the core, the gateway and plugins. It describes every
body, present or future, with the same six facts so that nothing platform-specific crosses a
layer boundary.

## ADDED Requirements

### Requirement: Six facts describe every body
The protocol SHALL define exactly six facts: actor (who caused an event), place (where it
happened and where an action lands), audience (who could perceive it), event (what happened),
capability (what a body can do) and receipt (the outcome of an action). Every message that
crosses a layer boundary SHALL be expressed in these facts.

#### Scenario: A percept carries actor, place and audience
- **WHEN** a body reports that something happened
- **THEN** the percept names the actor that caused it, the place it happened in and the audience that could perceive it

#### Scenario: A percept without an audience is rejected
- **WHEN** a plugin submits a percept that names no audience
- **THEN** the percept is rejected with an error that names the missing fact, and it is not recorded as a percept

### Requirement: Ids are opaque
Actors, places and capabilities SHALL be identified by opaque ids. No part of the protocol
SHALL require an id to follow a platform's format, and no layer below the plugin SHALL parse
an id for meaning.

#### Scenario: Ids from different bodies are treated alike
- **WHEN** two bodies report places whose ids have different shapes
- **THEN** both are accepted and routed by exact id match only

### Requirement: Content is carried, not interpreted
A percept and an action SHALL carry their content as an opaque payload, separate from the
numeric features that describe it (such as an embedding vector, or whether the turn was
allocated to the soul). The payload SHALL pass through the layers unchanged.

#### Scenario: Payload survives the round trip
- **WHEN** a percept with a payload is recorded and later placed into a context
- **THEN** the payload in the context is byte-for-byte the payload the plugin submitted

### Requirement: Every action ends in exactly one receipt
Each action handed to a body SHALL end in exactly one receipt whose outcome is one of: done,
refused, held, failed or unknown. A receipt SHALL name the action it belongs to.

#### Scenario: A body carries out an action
- **WHEN** a body completes an action
- **THEN** one receipt with outcome done is returned for that action

#### Scenario: A body does not answer in time
- **WHEN** a body returns no receipt before the action's time limit
- **THEN** one receipt with outcome unknown is produced for that action, and no second receipt for it is accepted afterwards

### Requirement: Capabilities are declared by descriptors
A body SHALL declare what it can do as capability descriptors. Each descriptor SHALL carry an
id, a description for reasoning, the places it applies to and its flags, including whether it
is identity-changing.

#### Scenario: A declared capability is offered to reasoning
- **WHEN** a body registers a capability for a place
- **THEN** that capability can appear in a context built for that place, with its description

### Requirement: The protocol is versioned and changes additively
Every serialized protocol message SHALL carry the protocol version. A reader SHALL accept
messages from an older version, SHALL ignore fields it does not know, and SHALL fail with a
clear error on a version newer than it supports.

#### Scenario: An unknown field is ignored
- **WHEN** a message of a supported version contains a field the reader does not know
- **THEN** the message is accepted and the unknown field is ignored

#### Scenario: A newer version fails loudly
- **WHEN** a message declares a protocol version newer than the reader supports
- **THEN** the reader rejects it with an error that states both versions

### Requirement: Every boundary message has a serialized form
Every message that crosses the gateway–plugin boundary SHALL have a self-describing serialized
form, so that a plugin running as a separate program can speak the protocol without sharing
code with the gateway.

#### Scenario: A message round-trips through its serialized form
- **WHEN** any boundary message is serialized and read back
- **THEN** the result equals the original message
