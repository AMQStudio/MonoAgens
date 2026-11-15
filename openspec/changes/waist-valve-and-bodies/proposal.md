# Proposal: `waist-valve-and-bodies`

## Why

Deferring the Gateway, deterministic Valve, Waist-2 plugin IPC, and canonical bodies until all 22 `core-and-lab` cognitive rows finish behind mock `TestBody` risks late breaking wire rewrites (mirroring the `MCP 2024-11-05` → `2026-07-28` reset) and isolates Core from real-world protocol, security, and multimodal constraints (`R-50`). Introduced as the successor to early prototype `bootstrap-vertical-slice` (git commit `ac3b240`), `waist-valve-and-bodies` locks `monoagens.body.v1` in Phase 0 and ships thin `monoagens-gateway`, Cedar `Valve`, `body-cli`, `body-mcp-bridge`, and `monoagens-conformance` TCK across Phases 0–3 in lockstep with [`core-and-lab`](../core-and-lab/proposal.md).

## What Changes

- **Waist-2 Wire Contract (`waist2-protocol`):** Specify `monoagens.body.v1` as `<= 12` stateless/explicit-handle JSON-RPC 2.0 methods over NDJSON `stdio` or Unix socket (`OpenRPC 1.4.1` + `JSON Schema 2020-12`), carrying Option A additive `Receipt` metadata (`termination_reason`, `progress`, `delivered_units`), out-of-band `blob/put` & `blob/get`, `idempotency_key`, `traceparent`, and strict major/minor version rules (`R-36`, `R-37`, `R-45`, `R-50`).
- **Gateway Runtime & Multimodal/Identity Isolation (`gateway`):** Specify `tokio` single-writer dispatch to `Core::step`, length-prefixed `κ(b, a)` `per_` actor hashing and append-only bilateral challenge linking (`R-44`), scope-keyed `BLAKE3` Attachment Store with `hard_forget` cascade unlink (`R-34`, `R-37`), pre-queue `EmbeddingGemma 2` (`256d` MRL, Apache 2.0) encoding runnable via native `ort`/`LiteRT` or local stdio/HTTP sidecar (`A-15`, `R-37`), hybrid `Tool RAG` capability catalog indexing for `R-38` progressive disclosure, and honest `R-47` lifecycle signaling (`1 s` `working_cue`, `10 s` `progress_note`).
- **Deterministic Cedar Valve (`valve`):** Replace cost-only placeholder rules with latest stable `cedar-policy` + `cedar-policy-symcc` (`LocalSolver::cvc5()`) ABAC/ReBAC evaluation running first and last in every door (`R-41`), enforcing Absolute Lines 1 & 2, SMT static owner verification, non-retrying `PendingApproval` + canonical `H_act` binding (`R-42`), 3-door taint blocking (`R-43`), `DeclassToken` release (`R-33`), daily token budget/chain brake, and quiet-hours/`3×/day` digests (`R-46`).
- **Canonical Bodies & Ecosystem Bridges (`bodies`):** Specify in-process/SDK reference bodies `body-cli`, `body-mcp-bridge` (`MCP 2026-07-28` + `2025-11-25` fallback), multimodal full-duplex audio barge-in (`Silero VAD` + AEC `< 50 ms` mute, `R-37`), multi-chunk `pid_1..pid_M` delivery (`R-47`), Phase-2/3 `plugin-a2a-bridge` (`v1.0.1`) & `plugin-ros2-bridge` (`Lyrical Luth`), and Phase-3 `body-sim-actuator`.
- **Operator & Developer CLI (`cli`):** Provide `monoagens chat`, `monoagens log` (`--explain`, JSON mode), `monoagens replay`, `monoagens approvals`, `monoagens link`, `monoagens conformance`, and `monoagens inspector`.
- **Public Black-Box Conformance TCK (`conformance-tck`):** Split black-box Waist-1 (`C-MUST-01..04`) and Waist-2 (`B-MUST-01..07`, `B-SHOULD-01..03`) test suites plus `< 200 LOC` polyglot SDK templates (Rust, TypeScript, Python) into `monoagens-conformance`, keeping `monoagens-lab` focused on cognitive ablations (`R-50`).

## Capabilities

### New Capabilities
- `waist2-protocol`: Bidirectional JSON-RPC 2.0 `monoagens.body.v1` contract (`<= 12` methods, OpenRPC `1.4.1`, additive Option A `Receipt`, error codes, Attachment transfer).
- `gateway`: `tokio` runtime driver, cross-body `κ(b, a)` `per_` identity & bilateral linking, keyed-`BLAKE3` Attachment Store, pre-queue `EmbeddingGemma 2` `256d`, and honest `R-47` progress cues.
- `valve`: Deterministic `cedar-policy` + `symcc` ABAC/ReBAC PDP first-and-last in every door, Absolute Lines 1 & 2, `PendingApproval`/`H_act`, taint blocking, budget/brake, and quiet-hours/digests.
- `bodies`: Reference `body-cli`, `body-mcp-bridge` (MCP `2026-07-28`), `plugin-a2a-bridge`/`plugin-ros2-bridge`, Phase-3 `body-sim-actuator`, `< 50 ms` voice barge-in, and multi-chunk delivery.
- `cli`: `monoagens` CLI subcommands for chat, audit replay, approval/identity management, conformance runner, and OpenTelemetry inspector.
- `conformance-tck`: Black-box `monoagens-conformance` harness (`B-MUST-01..07`, `B-SHOULD-01..03`, `C-MUST-01..04`, `--level must|should|may`, JSON `compatibility.json`) and `< 200 LOC` SDK generators.

### Modified Capabilities
- None (`openspec/specs/` is empty; `core-and-lab` separately governs Core and Lab capabilities).

## Impact

- **Crates & Binaries:** Adds `crates/gateway`, `crates/valve`, `crates/conformance`, `crates/cli`, and reference bodies (`body-mcp-bridge`, `body-sim-actuator`) depending strictly downward on `monoagens-protocol` and `monoagens-core` (`ARCHITECTURE.md` invariants untouched).
- **Supersedes Historical Slice:** Replaces the early `bootstrap-vertical-slice` prototype (preserved in git commit `ac3b240`) as the active Gateway/Valve/Body/CLI specification synced with Run 2 ([`2026-11-16-run-2.md`](../../../research/findings/2026-11-16-run-2.md)).
- **Dependencies:** Target the **latest stable compatible releases** of `tokio`, `jsonrpsee`, `schemars`, `serde_json`, `blake3`, `cedar-policy`, `cedar-policy-symcc`, `toml_edit`, `reqwest`, TypeScript/Python SDK toolchains (plus `proptest` and `criterion` verification suites for `κ(b, a)` invariants, RPC codecs, and `< 50 μs` Valve latency), resolving exact versions in `Cargo.lock`/SDK lockfiles and verifying up-to-date APIs via **`context7` MCP** at build time — zero impact on `monoagens-core` (`protocol` + `serde` only).
