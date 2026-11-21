# Spec Delta — multi-agent-coordination

## Purpose

Defines deterministic protocol, Valve, Core, and A2A bridge mechanisms that prevent inter-agent ping-pong loops, multi-bot chorus flooding, multi-hop delegation cycles, and integrative-compromise expertise neglect in shared multi-party group chats while preserving `ARCHITECTURE.md` invariants.

## ADDED Requirements

### Requirement: Additive edge-attested peer metadata in protocol
The protocol SHALL support strictly additive `#[serde(default)]` `automated_peer: bool` on `ActorFacts` (translated strictly from platform/body attestation at the edge, never from natural-language text) and `peer_expertise_margin: Option<f32>` on `Percept` (bounded `[0.0, 1.0]` cosine margin computed at the edge).

#### Scenario: Legacy protocol payloads deserialize cleanly with neutral defaults
- **WHEN** an existing serialized `ActorFacts` or `Percept` JSON payload omits `automated_peer` and `peer_expertise_margin`
- **THEN** deserialization succeeds with `automated_peer = false` and `peer_expertise_margin = None`

### Requirement: Deterministic Valve `PeerLoopBrake` and A2A multi-hop cycle guard
The Valve SHALL track consecutive automated-actor turns per place (including unthreaded flat rooms without `reply_to`) from the event log, trip `ValveReasonTag::PeerLoopBrake` when `consecutive_automated_turns >= max_consecutive_automated_turns` (default `4`), reset the counter to `0` on any non-automated turn, and enforce `max_a2a_hops` (default `3`) alongside `hop_count` / `visited_agents` cycle detection in `plugin-a2a-bridge`.

#### Scenario: Flat-room bot ping-pong stops deterministically and resets when a human speaks
- **WHEN** two automated peers exchange messages in a shared room without `reply_to` links until `consecutive_automated_turns` reaches `max_consecutive_automated_turns`
- **THEN** Valve refuses or holds further autonomous automated-turn wakes with `PeerLoopBrake`, and immediately resets the counter to `0` when a non-automated actor speaks

#### Scenario: Transitive A2A delegation cycle fails closed
- **WHEN** an outbound `a2a.*` delegation exceeds `max_a2a_hops` or revisits an agent already present in `visited_agents`
- **THEN** Valve or `plugin-a2a-bridge` fails closed with `ReceiptOutcome::Refused` (`ERR_A2A_CYCLE`) without dispatching the remote task

### Requirement: Scoped peer commitment guard and seeded expertise-scaled `OpenWait` jitter
The Core harness SHALL disable unconditional `CommitmentInputs::turn_allocated` bypass when an `automated_peer` repeatedly addresses the soul after `consecutive_automated_turns >= 2` (routing the event through neural/neutral gating instead), and SHALL apply deterministic `SeededRng` per-soul jitter scaled by `(1 + 2 * peer_expertise_margin)` to `OpenWait.due_ms` in multi-participant rooms so faster or specialist peers answer first and slower bots cleanly emit `DecisionReason::Yielded`.

#### Scenario: Repeated peer-bot mention cannot force infinite wake loop
- **WHEN** an `automated_peer` addresses the soul in consecutive bot-only turns after `consecutive_automated_turns >= 2`
- **THEN** Core does not force `DecisionReason::Commitment`, allowing the gate and Valve loop brake to suppress the loop

#### Scenario: Seeded jitter and expertise margin prevent homogeneous multi-bot chorus
- **WHEN** an open question arrives in a room with multiple bots (or a higher-expertise specialist peer with `peer_expertise_margin > 0`)
- **THEN** Core deterministically spreads `OpenWait.due_ms` via `SeededRng` scaled by `peer_expertise_margin`, cancelling `OpenWait` with `DecisionReason::Yielded` when another peer replies first
