# Spec Delta

## Purpose

The protocol is the one contract between the core and everything outside it. It describes
every body, present or future, with the same six facts so that nothing platform-specific
reaches the core.

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
- **WHEN** a body submits a percept that names no audience
- **THEN** the percept is rejected with an error that names the missing fact, and it is not recorded as a percept

### Requirement: Ids are opaque
Actors, places and capabilities SHALL be identified by opaque ids. No part of the protocol
SHALL require an id to follow a platform's format, and the core SHALL NOT parse an id for
meaning.

#### Scenario: Ids from different bodies are treated alike
- **WHEN** two bodies report places whose ids have different shapes
- **THEN** both are accepted and routed by exact id match only

### Requirement: Content is carried, not interpreted
A percept and an action SHALL carry their content as an opaque payload, separate from the
numeric features that describe it (such as an embedding vector, or whether the turn was
allocated to the soul). The payload SHALL pass through the layers unchanged.

#### Scenario: Payload survives the round trip
- **WHEN** a percept with a payload is recorded and later placed into a context
- **THEN** the payload in the context is byte-for-byte the payload the body submitted

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
id, a description for reasoning, the places it applies to, its flags (communicative,
identity-changing), the typical latency of a response in its places and, when using it has a
direct effect the body can sense, the channel, time window and noise of that effect.

#### Scenario: A declared capability is offered to reasoning
- **WHEN** a body registers a capability for a place
- **THEN** that capability can appear in a context built for that place, with its description

#### Scenario: A capability with a sensed effect
- **WHEN** a body declares that pushing a door changes a door sensor within two seconds
- **THEN** an act using that capability carries the channel and window, so the core can predict the effect

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

### Requirement: Every protocol message has a serialized form
Every protocol message SHALL have a self-describing serialized form, so that a body or a
harness written in another language can speak the protocol without sharing code with the core.

#### Scenario: A message round-trips through its serialized form
- **WHEN** any protocol message is serialized and read back
- **THEN** the result equals the original message

### Requirement: Reasoning's acts are part of the protocol
The protocol SHALL define the five thought acts (think, act, expect, resolve, revise) with
their fields as numbers, ids and opaque payloads, so that any model provider can be adapted to
them.

#### Scenario: A provider's output is adapted
- **WHEN** a provider's output is translated into thought acts
- **THEN** each act validates against the protocol, and an output that fits no act is reported as invalid with the reason

### Requirement: A percept carries what its body can report as numbers and ids
A percept SHALL be able to carry, beside its content: the actors it is addressed to, whether it
names the soul, the event or action it replies to, the correlation id of the action it echoes,
its length and a negative flag. A body that cannot report one of these SHALL omit it.

#### Scenario: A reply
- **WHEN** a body reports a message that replies to the soul's earlier action
- **THEN** the percept names that action as what it replies to

#### Scenario: A body that reports nothing extra
- **WHEN** a percept carries only actor, place, audience and content
- **THEN** it is accepted, and the mechanisms that need the missing facts take their neutral values
