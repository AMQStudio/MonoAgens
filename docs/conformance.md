# Waist-1 & Waist-2 Conformance (`monoagens-conformance`)

> Source of truth: [`crates/conformance/src/lib.rs`](../crates/conformance/src/lib.rs), [`crates/conformance/src/main.rs`](../crates/conformance/src/main.rs), [`crates/conformance/tests/body_tck.rs`](../crates/conformance/tests/body_tck.rs), and [`crates/conformance/tests/core_tck.rs`](../crates/conformance/tests/core_tck.rs). (`15/15` tests passing in `monoagens-conformance`: `11` in `body_tck.rs`, `2` in `conformance_runner.rs`, `2` in `core_tck.rs`.)

`monoagens-conformance` is a black-box conformance runner that spawns a compiled Body or Core harness as a subprocess over real NDJSON `stdio` JSON-RPC 2.0 (with zero in-memory shortcuts), passes an isolated `MONOAGENS_BODY_STATE_DIR` or `MONOAGENS_CORE_STATE_DIR` temporary directory, and writes a deterministic `CompatibilityReport` JSON file.

---

## 1. CLI Syntax and Exit Codes

Defined in [`crates/conformance/src/main.rs`](../crates/conformance/src/main.rs):

```sh
monoagens-conformance body --target <cmd> [--target-arg <arg>...] [--level must|should|may] [--json-report <path>]
monoagens-conformance core --target <cmd> [--target-arg <arg>...] [--level must|should|may] [--json-report <path>]
```

- **Exit `0`**: Every check required at `--level` passed (`--level must`: all `*-MUST-*` `pass`; `--level should`: all `*-MUST-*` and `*-SHOULD-*` `pass`; `--level may`: all `*-MUST-*`/`*-SHOULD-*` `pass` and no `*-MAY-*` `fail`).
- **Exit `1`**: One or more required checks returned `fail` or `skip`.
- **Exit `2`**: CLI usage error, target spawn failure, or JSON write failure.

---

## 2. Check Suites

### Body TCK (`monoagens-conformance body`)

| ID | Level | Title |
|---|---|---|
| `B-MUST-01` | `must` | `body/discover` returns deterministically sorted capabilities with `capability_schema_hash` |
| `B-MUST-02` | `must` | 3x identical `idempotency_key` `action/execute` yields single execution and matching receipt |
| `B-MUST-03` | `must` | `SIGKILL` crash recovery answers `state/query` within `5000 ms` without duplicate execution |
| `B-MUST-04` | `must` | `action/execute` and `percept/emit` enforce declared `place` and `audience` isolation |
| `B-MUST-05` | `must` | Held action receipt (`payload == "hold"`) round-trips through `approval/resolve` exactly once |
| `B-MUST-06` | `must` | Corrupted out-of-band blob bytes fail with `-32005 ERR_HASH_MISMATCH` before percept reference |
| `B-MUST-07` | `must` | Unknown capability yields typed `-32001 ERR_UNKNOWN_CAPABILITY` refusal |
| `B-SHOULD-01` | `should` | `body/heartbeat` reports liveness acknowledgement with monotonic timestamp |
| `B-SHOULD-02` | `should` | Honest delivery timing enforces `d_art = 0 ms` and `1s` `working_cue` / `10s` `progress_note` contract |
| `B-SHOULD-03` | `should` | `percept/emit` and `action/execute` reject omitted/empty audience with `-32006 ERR_UNDECLARED_AUDIENCE` |
| `B-MAY-01` | `may` | Optional extended body hints (currently emits `skip`, non-failing at `--level may`) |

### Core TCK (`monoagens-conformance core`)

Tested live against `target/debug/tck-core-harness` (`cargo test -p monoagens-conformance --test core_tck`):

| ID | Level | Title |
|---|---|---|
| `C-MUST-01` | `must` | `VisibleScope` non-interference `Delta_NI == 0` across private vs public histories |
| `C-MUST-02` | `must` | `hard_forget` erasure leaves zero residue in store/FTS5/snapshot/replay |
| `C-MUST-03` | `must` | Deterministic seeded log replay is bit-identical across runs |
| `C-MUST-04` | `must` | Neutral-profile 10k-tick soak maintains finite bounded state without crashes |
| `C-SHOULD-01` | `should` | Core loop input queue ordering preserves priority without dropping percepts |
| `C-SHOULD-02` | `should` | Single-receipt preemption records cancellation without spurious learning penalty |
| `C-SHOULD-03` | `should` | Record-first WAL crash recovery restores exact event state on `Core::open` |
| `C-SHOULD-04` | `should` | Pre-Step-5 `VisibleScope` and `Taint` isolation prevents tainted private bleed |
| `C-MAY-01` | `may` | Optional extended core hints (currently emits `skip`) |

---

## 3. `CompatibilityReport` JSON Schema

Defined by `CompatibilityReport` in [`crates/conformance/src/lib.rs`](../crates/conformance/src/lib.rs):

```json
{
  "schema_version": "1.0",
  "target": "target/debug/cleanroom-body",
  "level": "must",
  "checks": [
    {
      "id": "B-MUST-01",
      "title": "...",
      "status": "pass | fail | skip",
      "detail": "..."
    }
  ]
}
```

Repeated runs against a conforming body produce byte-identical JSON output (`body_tck::good_fixture_passes_all_must_and_should_and_reports_are_byte_identical`).

---

## 4. Negative Fixtures & Anti-Regression Tests

[`crates/conformance/tests/fixtures/mock_body.rs`](../crates/conformance/tests/fixtures/mock_body.rs) provides `good` plus 10 deliberately broken modes verified in [`crates/conformance/tests/body_tck.rs`](../crates/conformance/tests/body_tck.rs):
- `unsorted_discover` -> fails `B-MUST-01`
- `duplicate_execution` -> fails `B-MUST-02`
- `silent_crash` -> fails `B-MUST-03`
- `cross_place` -> fails `B-MUST-04`
- `double_receipt` -> fails `B-MUST-05`
- `bad_digest` -> fails `B-MUST-06`
- `unknown_capability` -> fails `B-MUST-07`
- `bad_heartbeat` -> fails `B-SHOULD-01`
- `bad_delivery_delay` -> fails `B-SHOULD-02`
- `accept_undeclared_audience` -> fails `B-SHOULD-03`

---

## 5. Clean-Room External Adapter Proof (`examples/cleanroom-body`)

[`examples/cleanroom-body/`](../examples/cleanroom-body/) implements a minimal external Waist-2 body (`cleanroom-body`, `181` LOC in [`src/main.rs`](../examples/cleanroom-body/src/main.rs)) using **only** [`sdk/rust`](../sdk/rust/Cargo.toml) (`monoagens-body-sdk`), `serde`, and `serde_json` (without depending on `monoagens-body-kit` or internal workspace crates).

Build and verify:

```sh
wc -l examples/cleanroom-body/src/*.rs
cargo build -p cleanroom-body -p monoagens-conformance
./target/debug/monoagens-conformance body \
  --target target/debug/cleanroom-body \
  --level must \
  --json-report /tmp/cleanroom.json
```

Observed output (`181` LOC, exit `0`, `--level must`):

```json
{
  "schema_version": "1.0",
  "target": "target/debug/cleanroom-body",
  "level": "must",
  "checks": [
    {
      "id": "B-MUST-01",
      "title": "body/discover returns deterministically sorted capabilities with capability_schema_hash",
      "status": "pass",
      "detail": "body_id='body:cleanroom', protocol_version=1.0, capabilities=1"
    },
    {
      "id": "B-MUST-02",
      "title": "3x identical idempotency_key action/execute yields single execution and matching receipt",
      "status": "pass",
      "detail": "3x idempotency_key replay produced 1 execution and identical receipt"
    },
    {
      "id": "B-MUST-03",
      "title": "SIGKILL crash recovery answers state/query within 5000 ms without duplicate execution",
      "status": "pass",
      "detail": "SIGKILL restart handshake + state/query answered within 5000ms bound without duplicate execution (status=terminal)"
    },
    {
      "id": "B-MUST-04",
      "title": "action/execute and percept/emit enforce declared place and audience isolation",
      "status": "pass",
      "detail": "unowned place percept/action refused and owned place/audience stayed isolated"
    },
    {
      "id": "B-MUST-05",
      "title": "held action receipt round-trips through approval/resolve exactly once",
      "status": "pass",
      "detail": "held receipt settled once via approval/resolve and rejected duplicate resolution"
    },
    {
      "id": "B-MUST-06",
      "title": "corrupted out-of-band blob bytes fail with -32005 ERR_HASH_MISMATCH before percept reference",
      "status": "pass",
      "detail": "corrupted blob bytes rejected with -32005 ERR_HASH_MISMATCH without percept reference"
    },
    {
      "id": "B-MUST-07",
      "title": "unknown capability yields typed -32001 ERR_UNKNOWN_CAPABILITY refusal",
      "status": "pass",
      "detail": "unknown capability rejected with -32001 ERR_UNKNOWN_CAPABILITY"
    }
  ]
}
```
