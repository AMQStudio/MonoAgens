# MonoAgens

An autonomous agent runtime built around a tiny, inspectable four-layer core (`Bodies <-> Gateway <-> Valve <-> Core`) where every action emits one of five explicit outcomes (`done`, `refused`, `held`, `failed`, `unknown`) and zero hidden prompts, unattributed state, or silent failures cross system boundaries.

> Architectural invariants live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Open empirical benchmarks and deferred runtime hardware probes are tracked in [`research/backlog.md`](research/backlog.md).

---

## Architecture & Two-Change Split

MonoAgens follows a strict two-change design in [`openspec/changes/`](openspec/changes/):

1. **[`core-and-lab`](openspec/changes/core-and-lab/proposal.md)** — Deterministic stateful **Core** (`crates/core`, `crates/store`), stateless LLM/embedder **Reason** (`crates/reason`), 22-mechanism ablation + Pareto **Lab** (`crates/lab`).
2. **[`waist-valve-and-bodies`](openspec/changes/waist-valve-and-bodies/proposal.md)** — Canonical **Waist-2 (`monoagens.body.v1`)** protocol & JSON/OpenRPC contracts (`crates/protocol`), Cedar + SymCC policy **Valve** (`crates/valve`), runtime **Gateway** (`crates/gateway`), black-box **Conformance TCK** (`crates/conformance`), SDKs (`sdk/`), external **Bodies** (`bodies/`), and operator **CLI** (`crates/cli`).

```text
Bodies / Adapters (bodies/, examples/, sdk/)
   │  NDJSON JSON-RPC 2.0 (Waist-2: monoagens.body.v1)
   ▼
Gateway (crates/gateway) ── kappa(b,a) identity · <=256 KiB strip · Tool RAG · Core-owner loop
   │  Four-Door Check (inbound / context / action / memory_write)
   ▼
Valve (crates/valve)     ── Cedar + cvc5 UNSAT proof · H_act · taint quarantine · quiet hours
   │
   ▼
Core + Store + Reason    ── Event-sourced SQLite WAL · Petrov/hybrid memory · Two-Epoch Replay
```

---

## Workspace Layout

| Directory | Contents |
|---|---|
| [`crates/protocol/`](crates/protocol/) | Waist-1 & Waist-2 (`monoagens.body.v1`) types, SemVer negotiation, [`openrpc/`](crates/protocol/openrpc/monoagens.body.v1.openrpc.json), [`jsonschema/`](crates/protocol/jsonschema/) |
| [`crates/core/`](crates/core/) | Deterministic decision loop, `VisibleScope`, `Prior`, 22-mechanism registry, expectations, `Two-Epoch Replay` |
| [`crates/store/`](crates/store/) | SQLite `hot.db`, FTS5, `usearch` HNSW vectors, checksummed snapshots, synchronous `hard_forget` |
| [`crates/reason/`](crates/reason/) | OpenAI-compatible & scripted providers, Spotlighting, `[SELF_GUIDE]` renderer, `EmbeddingGemma 2` embedders |
| [`crates/valve/`](crates/valve/) | Cedar + `cedar-policy-symcc` (`cvc5`) verification, `R_0..R_3` approvals, 3-door taint quarantine, DST quiet hours |
| [`crates/gateway/`](crates/gateway/) | Single Core-owner runtime, `κ(b, a)` linking, `XChaCha20Poly1305` attachment store, Tool RAG pre-queue |
| [`crates/conformance/`](crates/conformance/) | Black-box `monoagens-conformance` runner (`B-MUST/SHOULD/MAY`, `C-MUST/SHOULD/MAY`) |
| [`crates/lab/`](crates/lab/) | 22-row mechanism ablation, seeded offline replay, baseline adapters, Pareto scorecard |
| [`crates/cli/`](crates/cli/) | `monoagens` operator CLI (`chat`, `log`, `replay`, `approvals`, `link`, `conformance`, `inspector`) |
| [`bodies/`](bodies/) | `body-kit`, `body-cli`, `body-mcp-bridge`, `body-sim-actuator`, `body-voice`, `plugin-a2a-bridge`, `plugin-ros2-bridge` |
| [`sdk/`](sdk/) | Clean-room Waist-2 SDK shims for Rust (`sdk/rust`), TypeScript (`sdk/typescript`), and Python (`sdk/python`) |
| [`examples/cleanroom-body/`](examples/cleanroom-body/) | 181 LOC clean-room Waist-2 body passing `monoagens-conformance body --level must` |
| [`docs/`](docs/) | [`docs/waist2.md`](docs/waist2.md), [`docs/valve.md`](docs/valve.md), [`docs/conformance.md`](docs/conformance.md) |

---

## Build, Test, Lint, and Conformance

```sh
source "$HOME/.cargo/env"

# Core, Store, Reason, Protocol, Conformance, Lab suites
cargo test -p monoagens-protocol -p monoagens-store -p monoagens-reason -p monoagens-core
cargo test -p monoagens-conformance -p monoagens-lab
cargo test -p monoagens-valve

# Bodies, Clean-Room Example, and CLI tests + strict Clippy
cargo test -p monoagens-body-kit -p body-cli -p body-mcp-bridge \
  -p body-sim-actuator -p plugin-a2a-bridge -p plugin-ros2-bridge \
  -p body-voice -p monoagens-cli -p cleanroom-body

cargo clippy --all-targets -p monoagens-body-kit -p body-cli -p body-mcp-bridge \
  -p body-sim-actuator -p plugin-a2a-bridge -p plugin-ros2-bridge \
  -p body-voice -p monoagens-cli -p cleanroom-body -- -D warnings

# Valve door verdict benchmark
cargo bench -p monoagens-valve --bench door_verdict

# Conformance checks (build binaries first)
./target/debug/monoagens-conformance core --target target/debug/tck-core-harness --level should
./target/debug/monoagens-conformance body --target target/debug/body-cli --level must
./target/debug/monoagens-conformance body --target target/debug/body-mcp-bridge --level must
./target/debug/monoagens-conformance body --target target/debug/body-sim-actuator --level must
./target/debug/monoagens-conformance body --target target/debug/plugin-a2a-bridge --level must
./target/debug/monoagens-conformance body --target target/debug/body-voice --level must
./target/debug/monoagens-conformance body --target target/debug/cleanroom-body --level must
```

---

## Privacy & Safety Guarantees

- **Zero-Interference `VisibleScope` (`Δ_NI == 0`)**: Private memories outside a wake's declared audience and place are filtered prior to `ContextPlan` assembly (`C-MUST-01`, `C-SHOULD-04`) unless authorized by a logged `DeclassToken`.
- **Two Absolute Lines + First-and-Last Valve**: Non-owners can never trigger `SelfModify`/`identity_changing` actions (`default.cedar`), and secondary door hooks may only narrow access (`valve_v1`). Cedar policies are SMT-verified via `cvc5` on load.
- **Synchronous `hard_forget` Zero-Residue Erasure**: Removes targeted events across SQLite `hot.db`, FTS5, vector indices, encrypted attachment storage, and snapshots while preserving replay anchors (`C-MUST-02`, `attachment_tests`).
- **Deterministic Identity & No Heuristic Merging**: `crates/gateway/src/identity.rs` computes length-prefixed `per_<hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))>` IDs with heuristic merging permanently disabled (`HEURISTIC_MERGE_ENABLED = false`).
- **Secret Isolation**: API keys and attachment master keys load exclusively from `secrets.toml` or environment variables; inline secrets in config/scenario/soul files fail startup (`gateway::secrets`, `reason_suite`).

---

## Observed Status

- **Verified Green**: `monoagens-protocol` (`17/17`), `monoagens-core` (`81/81`), `monoagens-store` (`8/8`), `monoagens-reason` (`16/16`), `monoagens-valve` (`18/18`; `door_verdict` bench `[12.814 µs, 13.096 µs, 13.454 µs]`), `monoagens-conformance` (`15/15`, Core TCK `C-MUST-01..04` + `C-SHOULD-01..04`), `monoagens-lab` (`21/21`).
- **Verified Bodies & CLI**: `body-cli`, `body-mcp-bridge`, `body-sim-actuator`, `plugin-a2a-bridge`, `body-voice`, and `cleanroom-body` (`181` LOC) pass `monoagens-conformance body --level must` (`B-MUST-01..07`); `monoagens-cli` tests pass.
- **Partial / Unverified / Blocked Items**:
  - `crates/gateway`: `24/25` tests pass; `gateway_tests::mock_http_provider_sends_store_false_tracks_usage_and_redacts_canary_on_429_and_500` failed on `Failed(RateLimit)` vs `Success`. Inbound Gateway `approval/resolve` returns `-32601` (`crates/gateway/src/waist2.rs:709-716`), so CLI `approvals grant|deny` reports blocked cleanly.
  - `plugin-ros2-bridge` against live ROS 2 (`Lyrical Luth`) is **UNVERIFIED** (ROS 2 absent on host).
  - `body-voice` live Silero ONNX weights are **UNVERIFIED** (weights absent on host; verified using `webrtc-vad` fallback + `simulated_clock`).
  - Live LLM Lab sweeps & comparative baseline scoreboards (`lab/runs/`) are unexecuted.
