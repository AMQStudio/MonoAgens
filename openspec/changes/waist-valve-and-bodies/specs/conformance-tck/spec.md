# Spec Delta

## Purpose

Provides the standalone black-box conformance suite (`monoagens-conformance`) and OpenRPC SDK
generators (`< 200 LOC` Rust, TypeScript, Python) verifying any external body (`B-MUST`,
`B-SHOULD`) and any swappable Core (`C-MUST`) against frozen Waist 1 & Waist 2 contracts (`R-50`).

## ADDED Requirements

### Requirement: Public black-box TCK decouples wire conformance from Lab ablations
The `monoagens-conformance` runner SHALL execute strictly against public Waist-1 and Waist-2
interfaces (`stdio`, Unix socket, or wire harness) independent of internal `monoagens-lab`
cognitive ablations, grading implementations across `--level must|should|may` and emitting signed
JSON `compatibility.json` reports.

#### Scenario: Reference bodies run through real Waist-2 framing without shortcuts
- **WHEN** `monoagens-conformance body --level must` tests `TestBody`, `body-cli`, or `body-mcp-bridge`
- **THEN** every RPC executes over real NDJSON `stdio` or Unix socket RPC without internal in-memory bypass structs

### Requirement: Body TCK enforces B-MUST-01 through B-MUST-07 and B-SHOULD-01..03
The Body TCK SHALL verify: `B-MUST-01` sorted `body/discover` OpenRPC schemas; `B-MUST-02`
single-receipt and `idempotency_key` deduplication; `B-MUST-03` crash recovery (`state/query <=
5000 ms`, zero double exec); `B-MUST-04` audience/place isolation; `B-MUST-05` `held` round-trip;
`B-MUST-06` blob SHA-256 `ERR_HASH_MISMATCH`; `B-MUST-07` unknown capability refusal; plus
`B-SHOULD-01..03`.

#### Scenario: Fault injection catches duplicate execution and corrupted hashes
- **WHEN** the harness replays an `idempotency_key` three times, `SIGKILL`s an execution mid-flight, and uploads corrupt blob bytes
- **THEN** any duplicate side effect, missing `state/query` recovery, or accepted bad digest fails the corresponding `B-MUST` assertion in `compatibility.json`

### Requirement: Core TCK enforces C-MUST-01 through C-MUST-04 across swappable Cores
The Core TCK SHALL validate any Waist-1 implementation against `C-MUST-01` privacy non-interference
(`Delta_NI = 0.0`), `C-MUST-02` atomic `hard_forget` zero-resurrection erasure, `C-MUST-03` seeded
bit-identical decision replay, and `C-MUST-04` 10k-tick neutral-profile stability.

#### Scenario: Swappable Core fails C-MUST-01 if out-of-scope data alters wakes
- **WHEN** `C-MUST-01` feeds paired logs differing only in restricted out-of-scope memories
- **THEN** any shift in public recall counts, gate draw probabilities, or action verdicts (`Delta_NI > 0.0`) fails `C-MUST-01`

### Requirement: OpenRPC codegen produces sub-200 LOC Rust, TS, and Python SDKs
Waist-2 `OpenRPC 1.4.1` and `JSON Schema Draft 2020-12` artifacts SHALL generate Rust, TypeScript,
and Python SDK shims handling RPC framing, `idempotency_key` deduplication, crash journaling,
SHA-256 blob transfer, and heartbeats so custom body adapters require at most `200 LOC`.

#### Scenario: Minimal custom body wraps SDK and passes Body TCK
- **WHEN** an author writes under `200 LOC` implementing manifest, `execute`, `cancel`, and `on_approval` over a generated SDK
- **THEN** the resulting body binary passes all `B-MUST-01..07` conformance checks
