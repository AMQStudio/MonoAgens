# Spec Delta

## Purpose

Defines Waist 2 (`monoagens.body.v1`), the tiny stateless JSON-RPC 2.0 wire contract between
`monoagens-gateway` and all in-process or out-of-process bodies (`TestBody`, CLI, chat, voice,
robotics, MCP/A2A/ROS-2 bridges), preserving the six domain facts, Option A preemption receipts,
and out-of-band attachment transfer (`R-36`, `R-37`, `R-45`, `R-50`).

## ADDED Requirements

### Requirement: Twelve stateless JSON-RPC methods define Waist 2
The Waist-2 protocol (`monoagens.body.v1`) SHALL expose at most twelve stateless JSON-RPC 2.0
methods over NDJSON `stdio` or Unix socket: `body/discover`, `body/heartbeat`, `body/shutdown`,
`percept/emit`, `action/execute`, `action/cancel`, `action/receipt`, `approval/resolve`,
`question/answer`, `state/query`, `blob/put`, and `blob/get`, generated from `OpenRPC 1.4.1` and
`JSON Schema Draft 2020-12`.

#### Scenario: Discovery and crash state query avoid implicit session state
- **WHEN** a body connects initially or restarts after an unexpected termination
- **THEN** it advertises deterministic `body/discover` capabilities and answers `state/query` within `5000 ms` with `in_flight`, terminal receipt, or `unknown` without hidden session state

### Requirement: Envelopes separate lifecycle headers from six domain facts
Every Waist-2 envelope SHALL separate transport headers (`idempotency_key`, `protocol_version`,
reverse-DNS `_meta`, `traceparent`) from the six core domain facts. Additive upgrades `(M_msg ==
M_gw && m_msg <= m_gw)` SHALL ignore unknown keys; incompatible versions, undeclared audiences, or
bad transitions SHALL return `-32001` through `-32006`.

#### Scenario: Additive fields and older minor versions remain compatible
- **WHEN** a `1.0` body sends messages carrying unknown reverse-DNS `_meta` keys to a `1.1` gateway
- **THEN** the gateway ignores unknown keys cleanly and processes the envelope without error

#### Scenario: Unsupported version or undeclared audience returns typed error
- **WHEN** a body sends a newer incompatible version or emits a percept missing `audience`
- **THEN** Waist 2 rejects the RPC with `-32004 ERR_UNSUPPORTED_VERSION` or `-32006 ERR_UNDECLARED_AUDIENCE`

### Requirement: Option A receipts preserve five outcomes and preemption metadata
Every `action/execute` invocation SHALL settle in at most one `action/receipt` keyed by
`idempotency_key` and carrying one of the five core outcomes (`done`, `refused`, `held`,
`failed`, `unknown`), optionally paired with Option A fields (`termination_reason`, `progress`,
`delivered_units`). Duplicate or post-timeout receipts SHALL be ignored.

#### Scenario: Idempotency key prevents duplicate side effects on retry
- **WHEN** `action/execute` is delivered three times with an identical `idempotency_key`
- **THEN** the body executes the underlying operation once and returns the matching single `action/receipt`

#### Scenario: Preempted streaming action emits Option A cancellation fields
- **WHEN** `action/cancel` or external speech barge-in halts an in-flight action mid-stream
- **THEN** the body emits one `failed` receipt carrying `termination_reason` (`canceled_by_core` or `preempted_external`), `progress`, and `delivered_units`

### Requirement: Binary attachments split at 256 KiB via digest transfer
Media payloads at or below `256 KiB` MAY ride inline on `percept/emit` and SHALL be stripped by
Gateway before entering Core's queue. Larger media MUST transfer out-of-band via `blob/put` and
`blob/get` (HTTP, Unix socket, or FD pass) under a `60 s` staging TTL, rejecting mismatched
checksums with `-32005 ERR_HASH_MISMATCH`.

#### Scenario: Corrupted out-of-band blob upload fails checksum verification
- **WHEN** `blob/put` receives payload bytes whose SHA-256 digest fails verification
- **THEN** Waist 2 fails the transfer with `-32005 ERR_HASH_MISMATCH` before referencing the blob in any percept
