# Spec Delta

## Purpose

The protocol is the one contract between the core and everything outside it. It describes
every body, present or future, with the same six facts so that nothing platform-specific
reaches the core.

## ADDED Requirements

### Requirement: Six domain facts describe every body behind Waist 1
The protocol SHALL define six domain facts across Waist 1: actor, place, audience, event,
capability, and receipt, separated cleanly from Waist-2 JSON-RPC lifecycle envelopes.

#### Scenario: A percept carries actor, place and audience
- **WHEN** a body reports that something happened
- **THEN** the percept names the actor that caused it, the place it happened in and the audience that could perceive it

#### Scenario: A percept without an audience is rejected
- **WHEN** a body submits a percept that names no audience
- **THEN** the percept is rejected with an error that names the missing fact, and it is not recorded as a percept

#### Scenario: Waist-2 lifecycle messages do not pollute domain facts
- **WHEN** Gateway and a body exchange discovery, heartbeat, cancellation, or approval lifecycle RPCs
- **THEN** Waist-2 lifecycle envelopes translate into the six domain facts (`capability`, `body_state`, `receipt`, `percept`) without leaking transport details into the core

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
Each action SHALL end in one receipt naming the action with outcome `done`, `refused`, `held`,
`failed`, or `unknown`, plus optional additive metadata `termination_reason`, `progress`, and
`delivered_units` (`R-36`, `R-37`).

#### Scenario: A body carries out an action
- **WHEN** a body completes an action
- **THEN** one receipt with outcome done is returned for that action

#### Scenario: A body does not answer in time
- **WHEN** a body returns no receipt before the action's time limit
- **THEN** one receipt with outcome unknown and termination_reason timed_out is produced, and no second receipt is accepted afterwards

#### Scenario: A streaming or embodied action is preempted or cancelled
- **WHEN** active output (such as speech) is interrupted externally by user barge-in or cancelled by the core
- **THEN** a single terminal receipt (`failed`) records `termination_reason` (`preempted_external` or `canceled_by_core`), `progress` in `[0, 1]`, and `delivered_units`

### Requirement: Capabilities are declared by descriptors
A body SHALL declare capabilities via sorted descriptors carrying id, description, places,
flags (`communicative`, `identity_changing`, `irreversible`), typical latency,
`capability_schema_hash` (`R-38`), and optional sensed effect `(channel, window, noise, Δ_pred)`.

#### Scenario: A declared capability is offered to reasoning
- **WHEN** a body registers a capability for a place
- **THEN** that capability appears deterministically sorted in a context built for that place with its description and schema hash

#### Scenario: A capability with a sensed effect
- **WHEN** a body declares that pushing a door changes a door sensor within two seconds with expected delta
- **THEN** an act using that capability carries `channel`, window, and `Δ_pred` so the core can predict the effect

### Requirement: Multimodal attachments use opaque handles
A percept carrying media (`image`, `audio_voice`, `video`, `sensor_blob`) SHALL expose only
opaque `AttachmentHandle` metadata (`handle_id`, `modality_slot`, `byte_len`, `expires_at_ms`,
`embed_model_ver`) and zero raw media bytes across Waist 1 (`R-37`).

#### Scenario: Multimodal percept preserves core blindness
- **WHEN** a body submits an image or speech utterance through Gateway
- **THEN** Core receives an opaque `AttachmentHandle`, its 256d embedding, and numeric slots while raw bytes remain solely in Gateway's keyed-BLAKE3 Attachment Store

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

### Requirement: Reasoning's acts and consolidation outputs are part of the protocol
The protocol SHALL define the five thought acts (`think`, `act`, `expect`, `resolve`, `revise`)
with plural `places`/`people`, optional `domain?`, `RecurrenceSpec`/`PromiseMeta`, `resolve`
outcomes (`kept`, `kept_late`, `renegotiated`, `released`, `failed`, `cancelled`, `answered`,
`reached`), and typed `consolidate` acts (`gist`, `lesson`, `procedure`) under strict nullable
schemas (`R | null`).

#### Scenario: A provider's output is adapted
- **WHEN** a provider's output is translated into thought acts
- **THEN** each act validates against the strict schema, and an output that fits no act is reported invalid with its reason

### Requirement: A percept carries what its body can report as numbers and ids
A percept SHALL carry optional `addressees`, `named`, `orient(e, l)`, `reply_to`, `correlation_id`,
`channel`, `Δ_obs`, `length`, `negative_flag`, `repair`, `continued_attention`, the 10-element
structural acoustic/prosody slot `f_modal`, and embeddings tagged by `(embed_model_ver, dim_d,
prompt_regime_id)`.

#### Scenario: A reply
- **WHEN** a body reports a message that replies to the soul's earlier action
- **THEN** the percept names that action as what it replies to

#### Scenario: A body that reports nothing extra
- **WHEN** a percept carries only actor, place, audience and content
- **THEN** it is accepted, and the mechanisms that need the missing facts take their neutral values

### Requirement: Governance tokens and schema hashes are serializable facts
The protocol SHALL define `VisibleScope`/`DeclassToken`, `OwnerSigned` identity/principle bundles,
`PrincipalLink`, and canonical action hash `H_act` (`R-33`, `R-42`, `R-44`, `R-48`) across Waist 1.

#### Scenario: Cross-audience declassification token validates explicitly
- **WHEN** an owner-signed `DeclassToken` authorizes sharing specific event ids with a wider target place
- **THEN** the protocol serializes and preserves its issuer, scope, expiry, and target without loss
