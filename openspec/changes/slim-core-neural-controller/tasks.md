# Tasks: Slim Deterministic Core + Pure-Rust Neural Cognitive Controller (`V2`)

## 1. Archive V1 & Owner Governance

- [x] 1.1 Create git archive branch `archive/v1-22-mechanisms` at current `main` HEAD (preserving the 22-mechanism V1 Core and `Prior`) and verify `git rev-parse archive/v1-22-mechanisms` resolves cleanly.
- [x] 1.2 Apply the owner-approved amendment to `ARCHITECTURE.md` replacing the three V1-specific invariants (closed 22-mechanism brain map, per-mechanism switches, "one gate formula") with *Deterministic Harness + Learned Neural Controller with Neutral Fallback*, and verify all privacy, Valve, protocol, record-first, and replay invariants remain intact.

## 2. Pure-Rust Neural Controller (`crates/core/src/neural/`)

- [x] 2.1 Add `safetensors` to workspace `Cargo.toml` and `crates/core/Cargo.toml` (no `ort`, `candle-core`, or `burn`), and verify `cargo check -p monoagens-core` compiles cleanly.
- [x] 2.2 Implement dimension-agnostic `safetensors` model loader with `BLAKE3` digest verification, finite-weight validation, and neutral fallback (`None`) in `crates/core/src/neural/weights.rs`, verified by unit tests covering valid loading, missing/corrupt tensor rejection, and `BLAKE3` mismatch failure.
- [x] 2.3 Implement Pure-Rust `f32` forward pass (`PLR` numeric embeddings, Modality-Fusion stem, `TabM` `k`-member implicit-ensemble Pre-LN SwiGLU trunk exposing epistemic spread, per-place `64d` recurrent working memory supporting both `minGRU` and decoupled erase/write `Gated DeltaNet-2`, and Phase 1 `gate_4` + `wait_delta_t` heads plus optional Phase 2/3 head hooks) in `crates/core/src/neural/forward.rs`, verified by unit tests across `150k` and `1M` synthetic weight shapes.
- [x] 2.4 Implement bounded per-place Warm-Start UCB contextual-bandit online adaptation equalizer (frozen backbone + clamped `[-max_adapt_delta, +max_adapt_delta]` per-place bias/FiLM vector handling delayed one-sided feedback + `online_adaptation: bool` debug toggle) in `crates/core/src/neural/adapt.rs`, verified by unit tests confirming clamping bounds, backbone immutability, and complete isolation when `online_adaptation = false`.

## 3. Offline Phase 1 Training & Calibration Pipeline (`training/`)

- [x] 3.1 Scaffold `training/` (`pyproject.toml` via `uv`, PyTorch + `safetensors` + `blake3`) with an OSI-permissive dataset filter enforcing `MIT`/`Apache-2.0`/`CC-BY-4.0`/`ISC` only and explicitly blocking `CC-BY-NC-4.0`, missing-license repos, and `lab/scenarios/sealed/`, verified by a dry-run license audit test.
- [x] 3.2 Implement `256d` `EmbeddingGemma 2` precomputation and `18d` `x_num` feature builder over `when2speak`, `ubuntu_irc` (A), `proactive_bench`, and `capa` (strictly excluding Lab validation/test splits), verified by shard schema & finite-value checks.
- [x] 3.3 Implement PyTorch `MonoNeuralController` (`PLR` + `TabM` SwiGLU + selectable `minGRU` vs `Gated DeltaNet-2` + `gate_4`/`wait_delta_t`), run a Phase 1 sweep (`150k`, `500k`, `1M`, `3M` and `minGRU` vs `Gated DeltaNet-2`) on RTX 4060 (8 GB VRAM), fit post-hoc temperature scaling + `CIC` finite-sample risk thresholds on the calibration split, and record validation `ECE`, `TC-ECE`, `Brier`, `AURC`, and macro-F1 in a training report.
- [x] 3.4 Export the best-calibrated Phase 1 `safetensors` artifact + `BLAKE3` manifest + golden input/output fixture, and verify Python↔Rust forward-pass parity within `1e-5` absolute tolerance via `cargo test -p monoagens-core neural_parity`.

## 4. Slim Deterministic Core Harness (`crates/core`)

- [x] 4.1 Replace the 184-field `Prior` and 22-mechanism `Switches`/`registry.rs` sprawl with compact `CoreConfig` (`~15–25` structural/safety fields, `online_adaptation` toggle, optional `model_path` + `expected_blake3`) and verify config validation unit tests fail loudly on invalid bounds.
- [x] 4.2 Implement the `18d` model-agnostic numeric feature extractor (`x_num`, plus optional `10d` `FModal`) strictly post-`VisibleScope` with `NaN`/`Inf` sanitization and vectorless `0.0` degradation, verified by unit tests including `Delta_NI = 0` privacy non-interference.
- [x] 4.3 Prune obsolete hand-crafted V1 mechanism modules (`gate/salience.rs`, `gate/threshold.rs`, `gate/drives.rs`, `gate/patience.rs`, `beliefs/domains.rs`, `consolidation/boundaries.rs`, `learn/mod.rs`, `map/graph.rs`) and split `turn.rs`/`state.rs` into focused submodules while retaining deterministic `CommitmentBreaker`, `ProspectiveMemory`, `Agency`, `VisibleScope`, and hard budget/queue bounds; verify `cargo test -p monoagens-core` passes.
- [x] 4.4 Implement Trace-Pinned Replay on `DecisionTrace` (persisting `NeuralOutputs`, `model_blake3`, `feature_schema_version`, `online_adaptation_applied`, and `h_t` snapshot serialization), verified by snapshot-restart and cross-run replay determinism tests.

## 5. Gateway, Lab & Workspace Integration

- [x] 5.1 Wire `CoreConfig` and optional `.safetensors` controller loading through `crates/gateway` and `crates/cli` while keeping Waist-1 (`Store`, `Doors`), Waist-2, Valve (`< 50 μs` p99), and all `bodies/*` intact; verify `cargo test -p monoagens-gateway -p monoagens-valve`.
- [x] 5.2 Update `crates/lab` to evaluate Slim Core + Neural Controller (replacing removed 22-mechanism ablation hooks with neural calibration metrics `ECE`/`TC-ECE`/`Brier`/`AURC`, `CIC` risk coverage, neutral-fallback checks, and `online_adaptation` on/off sweeps), verified by `cargo test -p monoagens-lab`.
- [x] 5.3 Run full workspace verification (`cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, and `crates/conformance` `Delta_NI = 0` checks) and record actual `crates/core/src/` LOC and CPU inference latency in `research/backlog.md`.

## 6. Phased Multi-Head Extension (Phase 2 & Phase 3)

- [x] 6.1 Extend `training/` with Phase 2 (`p_uncertainty` fused with `TabM` ensemble disagreement, `p_verify` on `longmemeval` `_abs`, `stale`, `hi_tom`, `dyntom`) using uncertainty loss weighting / `PCGrad`, calibrate `ECE`/`TC-ECE`/`CIC`/`AURC`, export `safetensors`, and verify Rust optional-head loading + parity within `1e-5`.
- [x] 6.2 Extend `training/` with Phase 3 (`p_memory_relevance`, `p_encode_importance` on `group_mem_bench`, `memory_agent_bench`), wire post-`VisibleScope` neural reranking into `crates/core/src/recall/`, and verify parity and Lab recall metrics offline.

## Workflow follow-up

- Reconcile or archive superseded V1 `core-and-lab` change after V2 passes workspace verification.
- Archive `slim-core-neural-controller` via `openspec archive slim-core-neural-controller` once all tracked tasks are complete.
