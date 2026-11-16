# Spec Delta

## Purpose

The `monoagens` CLI provides owner and developer entry points for interactive chat, deterministic
log inspection and replay, `PendingApproval` review with canonical `H_act` rendering, bilateral
cross-body linking, public conformance TCK runs, Lab ablations, and live trace inspection.

## ADDED Requirements

### Requirement: Interactive chat and clean first-run bootstrap
`monoagens chat` SHALL boot Gateway, Cedar Valve, and `body-cli` against the configured data store
(`--config` / `MONOAGENS_HOME`). On an unconfigured system, it SHALL write default `monoagens.toml`,
report its path and required env secrets on stderr, and exit non-zero.

#### Scenario: First-run bootstrap creates config cleanly without credentials
- **WHEN** `monoagens chat` starts with no existing config or provider credentials
- **THEN** it generates valid default `monoagens.toml`, prints missing env settings to stderr, and exits non-zero

### Requirement: Machine-readable audit log, explanation, and offline replay
`monoagens log` (filterable by place, source, kind, time, and `--explain <event>`), `monoagens
replay`, and `monoagens lab` SHALL support `--json` structured stdout output, emit errors to stderr
with non-zero status, and verify logged decisions deterministically without model calls.

#### Scenario: Explaining a silent turn and replaying offline
- **WHEN** the owner executes `monoagens log --explain <id> --json` followed by `monoagens replay`
- **THEN** structured JSON reveals exact signals, threshold terms, and door verdicts, and offline replay exits zero

### Requirement: Approvals, bilateral linking, conformance, and inspector commands
The CLI SHALL expose `monoagens approvals` (inspect canonical `H_act` JSON, grant/deny held
actions), `monoagens link` (initiate/verify `R-44` dual-nonce actor challenges), `monoagens
conformance` (drive Core and Body TCKs), and `monoagens inspector` (stream `traceparent` traces and
active Cedar bundles).

#### Scenario: Granting a pending approval and validating an external body
- **WHEN** the owner runs `monoagens approvals grant <approval_id>` and a developer runs `monoagens conformance body --target <cmd> --level must --json-report compatibility.json`
- **THEN** the gateway verifies `H_act` atomically before child execution and the TCK outputs structured `compatibility.json` results exiting non-zero on failure

### Requirement: The soul starts with the configured prior and replays with the recorded one
`monoagens.toml` SHALL accept an optional `[soul] prior = "<path>"`. `monoagens chat` and the
gateway SHALL load that prior through the shared prior loader of `core-and-lab` (Decision 20), or
the built-in default when the key is absent, and SHALL fail start-up on an unknown key, an
out-of-range disposition or an unreadable path. `monoagens replay` SHALL take the prior from the
log's `prior_loaded` events and SHALL NOT accept or apply a prior of its own.

#### Scenario: The owner changes the prior between sessions and replays
- **WHEN** the owner chats under the default prior, sets `[soul] prior` to an edited file, chats again, then runs `monoagens replay`
- **THEN** the log holds a `prior_loaded` event for each prior, and replay reproduces every decision of both sessions and exits zero
