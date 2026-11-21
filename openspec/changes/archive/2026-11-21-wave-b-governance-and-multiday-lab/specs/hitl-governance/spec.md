# Spec Delta — hitl-governance

## Purpose

Defines decoupled, out-of-band inalienable human-in-the-loop (`I-HITL`) approval resolution, event-sourced approval and digest restart persistence, anti-approval-fatigue quota partitioning, structural `Verify-Before-Abstain` over-abstention defense, and replannable digest reconsideration for MonoAgens.

## ADDED Requirements

### Requirement: End-to-end decoupled out-of-band `approval/resolve` with cryptographic attestation
The Gateway, Valve, and CLI SHALL wire Waist-2 `approval/resolve` end-to-end through `OwnerTaskContext` with fail-closed validation of `(action_id, h_act, state_epoch, TTL, owner_actor_id)` and cryptographic out-of-band attestation over canonical resolution fields, rejecting any resolution arriving from an untrusted or shared in-band chat body.

#### Scenario: Attested out-of-band owner approval settles and dispatches held action once
- **WHEN** an attested out-of-band owner channel submits `approval/resolve` with `approved = true`, matching `h_act` and `state_epoch`, unexpired TTL, and a valid owner signature
- **THEN** Valve records the approval grant, settles the held `PendingApproval` once, and dispatches the canonical action without returning `-32601`

#### Scenario: In-band chat body or invalid signature resolution fails closed
- **WHEN** an untrusted/in-band chat body or a request with a missing, blank, or invalid cryptographic signature submits `approval/resolve`
- **THEN** Gateway and Valve fail closed without granting approval and record a refusal/spoof audit trace

#### Scenario: Post-approval payload mutation or stale epoch fails closed
- **WHEN** an action payload mutates after hold (`h_act` mismatch) or `state_epoch` advances before execution
- **THEN** Valve rejects execution as `stale_or_mismatched_approval` without dispatching the action

### Requirement: Event-sourced persistence of ApprovalManager and DigestQueue across restarts
The Gateway and Valve SHALL project `ApprovalManager` pending items, grants, denials, prompt timestamps, and `DigestQueue` scheduled items from append-only events so restart and CLI inspection (`monoagens approvals list/grant/deny`) reconstruct exact `state_epoch` and pending state deterministically.

#### Scenario: Pending approval survives restart with exact state_epoch
- **WHEN** an `R2` action is held as `PendingApproval`, the process restarts, and state is replayed from the event log
- **THEN** `ApprovalManager` restores the pending item with its exact `h_act`, `state_epoch`, and expiry timestamp, and CLI `monoagens approvals list` renders a structured 3-line impact card with the real `state_epoch`

### Requirement: Anti-approval-fatigue quota partitioning and telemetry
The Valve SHALL keep `R0` and `R1` actions strictly prompt-free, partition the sliding hourly approval budget (`MAX_PROMPTS_PER_HOUR = 6`) so at least `2` slots are reserved for `Taint::Trusted` owner-originated chains while capping untrusted escalations separately (`<= 2/h`), and emit approval latency and rubber-stamp telemetry.

#### Scenario: Flood of untrusted R2 escalations cannot starve trusted owner approval
- **WHEN** an untrusted flood triggers repeated `R2` holds in a 1-hour window exceeding the untrusted cap, followed by a `Taint::Trusted` owner-originated `R2` action
- **THEN** excess untrusted holds fail closed as `oversight_budget_exhausted` while the reserved trusted slot admits the owner's `R2` approval prompt

### Requirement: Structural `Verify-Before-Abstain` provenance framing in `[SELF_GUIDE]`
The Valve Context Door SHALL structurally separate authoritative state (`[TRUSTED_VALVE_SNAPSHOT]`, `[OWNER_MEMORY]`) from low-trust inputs (`[UNTRUSTED_PERCEPT]`, `[PROVISIONAL]`) and state the deterministic `Verify-Before-Abstain` rule without keyword scanning, so fabricated safety warnings inside untrusted content cannot induce unilateral abstention on authorized tasks.

#### Scenario: Untrusted fake security warning does not block authorized owner task
- **WHEN** an untrusted percept embeds a fabricated policy-block warning alongside an owner-authorized action allowed by `ValveSelfSnapshot`
- **THEN** `[SELF_GUIDE]` marks the warning as untrusted data rather than an authoritative Valve block, preserving execution of the authorized task while genuinely unauthorized actions remain blocked by Valve

### Requirement: Replannable digest reconsideration before send
Before `DigestQueue::drain_due` dispatches a deferred proactive message, the Valve/Core boundary SHALL check whether an intervening event in the same place superseded or cancelled the queued item's topic or expectation, dropping stale digests rather than delivering outdated messages.

#### Scenario: Intervening cancellation suppresses queued digest before delivery
- **WHEN** a proactive reminder is queued in `DigestQueue` for a future digest slot and an intervening event cancels or resolves the underlying expectation/topic before `scheduled_at_ms`
- **THEN** `DigestQueue` marks the queued entry superseded/cancelled and emits zero outbound action at drain time
