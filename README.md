# MonoAgens

An autonomous agent runtime built around a tiny, inspectable four-layer core (`Bodies <-> Gateway <-> Valve <-> Core`) where every action emits one of five explicit outcomes (`done`, `refused`, `held`, `failed`, `unknown`) and zero hidden prompts, unattributed state, or silent failures cross system boundaries.

> Architectural invariants live in [`ARCHITECTURE.md`](ARCHITECTURE.md). Open empirical benchmarks and deferred runtime hardware probes are tracked in [`research/backlog.md`](research/backlog.md).

---

## Architecture & OpenSpec Layout

MonoAgens is governed by canonical behavior specifications in [`openspec/specs/`](openspec/specs/) (including archived V3 Neural Cognitive Controller `v3-synaptic-active-inference`, `cli-interactive-chat`, `proactive-group-embed-s1`, `lab-live-eval-hygiene`, `context-chronological-recall-and-hygiene`, `lab-e2e-conversation-samples`, `hitl-governance`, `multi-agent-coordination`, `multiday-chat-lab`, `neural-controller`, `neural-training-pipeline`, and `slim-core-harness`), while active feature and infrastructure changes go through [`openspec/changes/`](openspec/changes/) (including [`gateway-multi-body-daemon`](openspec/changes/gateway-multi-body-daemon/proposal.md) for WhatsApp/Telegram multi-body readiness, [`waist-valve-and-bodies`](openspec/changes/waist-valve-and-bodies/proposal.md), [`lab-datasets`](openspec/changes/lab-datasets/proposal.md), and [`lab-memory-comparison`](openspec/changes/lab-memory-comparison/proposal.md)).

```text
Plugins — Adapters & Capabilities (plugins/, examples/, sdk/ — e.g. cli, telegram, whatsapp, web-search, workspace, pdf-reader, soul-tuning)
   │  NDJSON JSON-RPC 2.0 (Waist-2: monoagens.body.v1)
   ▼
Gateway (crates/gateway) ── kappa(b,a) identity · <=256 KiB strip · Tool RAG · EmbeddingGemma 2 (256d)
   │  Four-Door Check (inbound / context / action / memory_write)
   ▼
Valve (crates/valve)     ── Cedar + cvc5 UNSAT proof · H_act · taint quarantine · quiet hours
   │
   ▼
Core + Store + Reason    ── Event-sourced SQLite WAL · V3 Neural Controller (36d, W_fast, world_pred) · Replay
```

Key runtime capabilities:
- **V3 Neural Cognitive Controller (`crates/core/src/neural/`)**: `36d` feature schema v3 (`feature_schema_version = 3`), `InfoGapClassifier`, `PlaceCosBaseline`, three-factor synaptic fast-weight plasticity (`W_fast`), and `head.world_pred` predictive world-state head.
- **Local Semantic Embedder (`crates/reason/`, `crates/gateway/src/prequeue.rs`)**: Local `EmbeddingGemma 2` (`256d`) HTTP/stdio sidecar for pre-queue percept enrichment and hybrid BM25 + vector Tool RAG (with deterministic fallback).
- **Operator CLI & Multi-Body Daemon (`crates/cli/`)**: Interactive terminal REPL (`monoagens chat --interactive`) and persistent multi-body daemon (`monoagens serve`) managing `[[bodies]]` adapters over Waist-2.

---

## Workspace Layout

| Directory | Contents |
|---|---|
| [`crates/protocol/`](crates/protocol/) | Waist-1 & Waist-2 (`monoagens.body.v1`) types, SemVer negotiation, [`openrpc/`](crates/protocol/openrpc/monoagens.body.v1.openrpc.json), [`jsonschema/`](crates/protocol/jsonschema/) |
| [`crates/core/`](crates/core/) | Deterministic decision loop, `VisibleScope`, V3 Neural Controller (`36d`, `W_fast`, `PlaceCosBaseline`, `head.world_pred`), expectations, `Two-Epoch Replay` |
| [`crates/store/`](crates/store/) | SQLite `hot.db`, FTS5, `usearch` HNSW vectors, checksummed snapshots, synchronous `hard_forget` |
| [`crates/reason/`](crates/reason/) | OpenAI-compatible & scripted providers, Spotlighting, `[SELF_GUIDE]` renderer, `EmbeddingGemma 2` (`256d`) & `InfoGapClassifier` |
| [`crates/valve/`](crates/valve/) | Cedar + `cedar-policy-symcc` (`cvc5`) verification, `R_0..R_3` approvals, 3-door taint quarantine, DST quiet hours |
| [`crates/gateway/`](crates/gateway/) | Single Core-owner runtime, `κ(b, a)` linking, `XChaCha20Poly1305` attachment store, Tool RAG pre-queue, attested OOB `approval/resolve`, dynamic place ownership |
| [`crates/conformance/`](crates/conformance/) | Black-box `monoagens-conformance` runner (`B-MUST/SHOULD/MAY`, `C-MUST/SHOULD/MAY`) |
| [`crates/lab/`](crates/lab/) | 22-row mechanism ablation, seeded offline replay, external dataset converters, Pareto scorecard & live conversation sampling |
| [`crates/cli/`](crates/cli/) | `monoagens` operator CLI (`chat [--interactive]`, `serve`, `log`, `replay`, `approvals`, `link`, `conformance`, `inspector`, `export-learning`) |
| [`plugins/`](plugins/) | `kit`, adapters (`cli`, `telegram`, `whatsapp`, `voice`, `sim-actuator`), bridges (`mcp-bridge`, `a2a-bridge`, `ros2-bridge`), and capability plugins (`web-search`, `workspace`, `pdf-reader`, `soul-tuning`) |
| [`sdk/`](sdk/) | Clean-room Waist-2 SDK shims for Rust (`sdk/rust`), TypeScript (`sdk/typescript`), and Python (`sdk/python`) |
| [`examples/cleanroom-body/`](examples/cleanroom-body/) | 181 LOC clean-room Waist-2 body passing `monoagens-conformance body --level must` |
| [`docs/`](docs/) | [`docs/README.md`](docs/README.md), [`docs/messaging-bodies.md`](docs/messaging-bodies.md), [`docs/waist2.md`](docs/waist2.md), [`docs/valve.md`](docs/valve.md), [`docs/conformance.md`](docs/conformance.md), [`docs/lab-datasets.md`](docs/lab-datasets.md), and [`docs/eval-logs/`](docs/eval-logs/hasil-percakapan-live-v3-chrono.md) |

---

## Build, Test, Lint, and Conformance

```sh
source "$HOME/.cargo/env"

# Core, Store, Reason, Protocol, Valve, Gateway, Conformance, Lab suites
cargo test -p monoagens-protocol -p monoagens-store -p monoagens-reason -p monoagens-core
cargo test -p monoagens-valve -p monoagens-gateway
cargo test -p monoagens-conformance -p monoagens-lab

# Bodies, Clean-Room Example, and CLI tests + strict Clippy
cargo test -p monoagens-body-kit -p body-cli -p body-mcp-bridge \
  -p body-sim-actuator -p plugin-a2a-bridge -p plugin-ros2-bridge \
  -p body-voice -p monoagens-cli -p cleanroom-body

cargo clippy --all-targets -p monoagens-body-kit -p body-cli -p body-mcp-bridge \
  -p body-sim-actuator -p plugin-a2a-bridge -p plugin-ros2-bridge \
  -p body-voice -p monoagens-cli -p cleanroom-body -- -D warnings

# Valve door verdict benchmark
cargo bench -p monoagens-valve --bench door_verdict

# Operator interactive REPL & multi-body daemon
cargo run -p monoagens-cli -- chat --interactive
cargo run -p monoagens-cli -- serve

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

- **Verified Workspace Suites**:
  - `monoagens-protocol`, `monoagens-core`, `monoagens-store`, `monoagens-reason`, `monoagens-lab`, `monoagens-body-kit`, `body-cli`, `body-mcp-bridge`, `body-sim-actuator`, `plugin-a2a-bridge`, `body-voice`, `monoagens-cli`, and `cleanroom-body` pass unit/integration suites.
  - `monoagens-valve`: **18/18 PASS**; `door_verdict` Criterion microbenchmark `[12.814 µs, 13.096 µs, 13.454 µs]`.
  - `monoagens-gateway`: **30/30 PASS**; inbound `approval/resolve` is implemented via attested out-of-band control channel (`handle_approval_resolve` in [`crates/gateway/src/waist2.rs`](crates/gateway/src/waist2.rs), rejecting in-band chat bodies and accepting attested OOB bodies).
  - `monoagens-conformance`: **15/15 PASS** (Core TCK `C-MUST-01..04` + `C-SHOULD-01..04`); `body-cli`, `body-mcp-bridge`, `body-sim-actuator`, `plugin-a2a-bridge`, `body-voice`, and `cleanroom-body` (`181` LOC) pass `monoagens-conformance body --level must` (`B-MUST-01..07`).
- **V3 Live Verification Logs ([`docs/eval-logs/`](docs/eval-logs/hasil-percakapan-live-v3-chrono.md))**:
  - `smoke-id` (Indonesian live suite): **3/3 PASS** (`id_01_grup_santai_silent`, `id_02_proactive_unanswered_question`, `id_03_dm_privacy_delta_ni_zero`) in [`hasil-percakapan-live-v3.md`](docs/eval-logs/hasil-percakapan-live-v3.md).
  - `monoagens chat` + `monoagens replay --mode pinned`: **100% matched decisions** (`5/5` turns, `60` replayed events, `first_divergence = null`).
  - 4-domain live pilot progression (`R1 -> R2 -> R3` across `when2speak`, `proactive_bench`, `pm_bench`, `fantom`): `0` failed items, `when2speak` `0/1 -> 1/1 PASS`, `pm_bench` `1/1 PASS`, `fantom` `14/24 -> 17/24` (`Answerability[Binary]` `2/6 -> 6/6`, `all-roberta-large-v1` wired) — documented in [`hasil-percakapan-live-v3-after.md`](docs/eval-logs/hasil-percakapan-live-v3-after.md) and [`hasil-percakapan-live-v3-chrono.md`](docs/eval-logs/hasil-percakapan-live-v3-chrono.md).
- **Host-Absent Hardware Items (Unverified on Host)**:
  - `plugin-ros2-bridge` against live ROS 2 (`Lyrical Luth`) is **UNVERIFIED** (ROS 2 absent on host).
  - `body-voice` live Silero ONNX weights are **UNVERIFIED** (weights absent on host; verified using `webrtc-vad` fallback + `simulated_clock`).
