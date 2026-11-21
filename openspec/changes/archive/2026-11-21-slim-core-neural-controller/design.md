# Design: Slim Deterministic Core + Pure-Rust Neural Cognitive Controller (`V2`)

## Context

See `proposal.md` (Why & What Changes) and `research/findings/2026-11-21-run-5.md` (`R-59`–`R-64`) for the full empirical and literature foundation.

Currently, `crates/core/src/` spans `34,222` LOC across 44 files, implementing 22 hand-crafted mechanisms (`M01`–`M22` in `crates/core/src/registry.rs`) and `184` fields in `Prior` (`crates/core/src/prior.rs`). Meanwhile, `crates/gateway`, `crates/valve`, `crates/store`, `crates/reason`, `crates/protocol`, `crates/lab`, and `bodies/*` are already built around clean Waist-1 (`Store`, `Doors`) and Waist-2 boundaries.

## Goals / Non-Goals

**Goals:**
- Reduce `crates/core/src/` complexity and knob surface dramatically (`34.2k` → `~15k–20k` LOC, `184` → `~15–25` config fields) so the core is readable, maintainable, and works out of the box.
- Implement a zero-heavy-runtime Pure-Rust `f32` Neural Cognitive Controller ("1 Neural Link") supporting `50k`–`3M` parameter weights via `safetensors`, with `minGRU` (`64d`) per-place working memory and bounded per-place online adaptation.
- Keep **100% compatibility** with existing Waist-1 (`Store`, `Doors`), Gateway, Valve, Bodies, and Lab dataset infrastructure (`waist-valve-and-bodies`, `lab-datasets`, `lab-memory-comparison`).
- Preserve all core safety/privacy/audit invariants (`VisibleScope` `Delta_NI = 0`, deterministic Valve, record-first event log, deterministic replay, no raw text or word lists in Core).

**Non-Goals:**
- Fine-tuning or modifying LLM weights (controller remains 100% LLM-agnostic).
- Replacing deterministic Valve policies, `VisibleScope` privacy filtering, explicit prospective deadlines (`M13`), or self-echo agency (`M20`) with neural predictions.
- Maintaining two parallel Core engines (`V1` and `V2`) in `main` (V1 will be preserved cleanly in git branch `archive/v1-22-mechanisms`).
- Running paid live LLM benchmarks automatically without explicit owner approval.

## Decisions

### Decision 1: Archive V1 to Git Branch Before Pruning `crates/core`
- **Choice:** Create an immutable git archive branch `archive/v1-22-mechanisms` from current `main`, then refactor `crates/core` directly on `main` into the Slim Core V2 harness.
- **Alternatives considered:**
  - *Shadow mode inside V1 `crates/core`*: keeps the 6.5k-LOC `turn.rs` and 184-field `Prior` alive indefinitely during transition.
  - *Parallel `crates/core-v2` crate*: doubles maintenance burden across Gateway, Lab, and Conformance tests.
- **Rationale:** Owner explicitly prioritized readability and maintainability over maintaining two parallel cores; git history preserves V1 cleanly for any future reference.

### Decision 2: `ARCHITECTURE.md` Amendment (Owner-Approved)
- **Choice:** Update three V1-specific invariants in `ARCHITECTURE.md` under explicit owner sign-off before code refactoring:
  1. *Closed 22-mechanism brain map* (`ARCHITECTURE.md:66-68`) → *Minimal deterministic harness primitives + 1 unified Neural Cognitive Controller*.
  2. *Per-mechanism neutral switches* (`ARCHITECTURE.md:89-91`) → *Neural controller has a mandatory neutral fallback (`None`) so the single loop still runs without weights*.
  3. *Complexity budget "one gate formula"* (`ARCHITECTURE.md:268-270`) → *Fixed deterministic primitives + one versioned neural controller interface*.
- **Preserved unchanged:** Protocol 6 facts, Core no-raw-content / no-word-lists, Valve first-and-last deterministic doors, Two Absolute Lines (owner identity + `VisibleScope` privacy), record-first single queue, and reproducible replay.

### Decision 3: Unified "1 Neural Link" Architecture (`PLR` + `TabM`/SwiGLU + `Gated DeltaNet-2`/`minGRU` + Warm-Start UCB Adaptation)
- **Choice (updated with 2025–2026 literature; validated by `arXiv:2605.30152`, where a `~1M`-param local temporal controller ran in `13.99 ms` on laptop and gained `+16.7` mean F1 across 14 LLM backbones):**
  - **Input (`274d` base, optional `284d` with full `FModal`):**
    - Frozen `256d` `EmbeddingGemma 2` vector (`e_256`, zero-vector if absent).
    - `18` normalized numeric scalars (`x_num`, defined in `research/findings/run-5-notes/01_mechanism_and_loc_audit.md` §3: addressivity, cadence, familiarity, budget ratio, promise debt, visible memory cosine/recency, belief uncertainty, queue depth, ASR confidence).
  - **Modality-Fusion Stem:**
    - `e_proj = LayerNorm(W_e * e_256 + b_e)` → `d_e`
    - `n_proj = PLR(x_num)` (Periodic `sin/cos` + Linear + SiLU, *Gorishniy et al., NeurIPS 2022*) → `d_n`
    - Concat `[e_proj, n_proj, h_{t-1}]` → project to `d_model`.
  - **Trunk (`TabM` Implicit Ensemble + Pre-LN SwiGLU):**
    - `N` Pre-LN Residual SwiGLU blocks (*Shazeer, 2020*) wrapped with **`TabM` parameter-efficient `k`-member BatchEnsemble-style adapters** (*Gorishniy et al., ICLR 2025*, `arXiv:2410.24210`, default `k=8`, `k=1` degrades cleanly to a single MLP).
    - Dimension-agnostic: `d_model`, `N`, and `k` are read from `safetensors` metadata (`50k`–`3M` params supported by the same Rust code).
    - Inter-member disagreement (predictive variance / JS divergence across the `k` implicit ensemble outputs) supplies **free epistemic uncertainty** directly augmenting `p_uncertainty` and selective abstention.
  - **Per-Place Working Memory (`64d`, `Gated DeltaNet-2` vs `minGRU`):**
    - Support both compact recurrent cells via metadata selector (`cell_type`):
      - **`minGRU`** (*Feng et al., 2024*, `arXiv:2410.01201`): `z_t = sigmoid(W_z x + b_z)`, `h_t = (1 - z_t) * h_{t-1} + z_t * h_tilde`.
      - **`Gated DeltaNet-2` diagonal/channel-wise cell** (*Hatamizadeh, Choi, & Kautz / NVIDIA, May 2026*, `arXiv:2605.22791`; cf. `FG²-GDN` `arXiv:2604.19021`): decouples channel-wise **erase gate `b_t`** and **write gate `w_t`** (`h_t = (1 - b_t) * h_{t-1} + w_t * delta_t`), enabling selective erasure of stale topics without overwriting active ones.
    - Both run in `O(1)` per event in Pure Rust (~25–45 LOC) and serialize as `[f32; 64]` per active place in snapshots; Phase 1 benchmarks both head-to-head on multi-turn datasets.
  - **Bounded Per-Place Online Adaptation ("Equalizer" — Warm-Start UCB Bandit):**
    - Backbone + recurrent cell stay **frozen** at runtime (zero catastrophic forgetting).
    - Per-place vector `a_p` (`4` gate logit offsets + `4` FiLM scale/shift scalars, clamped to `[-0.5, +0.5]`, plus per-arm pull counts & reward stats) updated via **Warm-Start UCB Contextual Bandit** (*MemCon* `arXiv:2607.13591` & *Personalized Memory Gating* `arXiv:2610.23812`, specifically formulated for delayed, one-sided user feedback such as ignored messages in group chats).
    - Guarded by `CoreConfig.online_adaptation: bool` (can be disabled cleanly for debugging/ablation).
  - **Phased Output Heads:**
    - **Phase 1 (Required):** `gate_4` logits (`[Silent, Note, Wait, Go]`, divided by calibrated temperature `T` with `CIC` abstention threshold) + `wait_delta_t` (sigmoid × `max_wait_ms`; aligned with replannable timelines in `arXiv:2610.05159` and `ProActor` `arXiv:2605.24900`).
    - **Phase 2 (Optional):** `p_uncertainty` (learned head + `TabM` ensemble variance), `p_verify` (numeric hints for ContextBuilder/auto-recall depth).
    - **Phase 3 (Optional):** `p_encode_importance` (Memory Write Gate) and `p_memory_relevance` (Retrieval Gate over `VisibleScope`-filtered candidates, aligned with `Gated-Memory Routing` `arXiv:2609.00237` and `Memory-R1` `arXiv:2508.19828`).
- **Alternatives considered:** Plain stateless single MLP (weaker tabular accuracy, no free ensemble uncertainty, misses cross-turn cadence); full online backprop (catastrophic forgetting & replay drift); 8 separate heads from day 1 (negative transfer on small data).

### Decision 4: Pure-Rust Inference + `safetensors` + Trace-Pinned Replay
- **Choice:**
  - Pure-Rust `f32` matrix-vector forward pass inside `crates/core/src/neural/` (~200–350 LOC) depending only on the lightweight `safetensors` crate (plus workspace `blake3`). No `ort` (ONNX Runtime), `candle-core`, or `burn`.
  - **Trace-Pinned Replay:** Every `EventDetail::Decision` stores `DecisionTrace` containing `NeuralOutputs` (probabilities/scalars), `model_blake3: Option<[u8; 32]>`, `feature_schema_version: u16`, `online_adaptation_applied: bool`, and final `DecisionOutcome` + `DecisionReason`. During replay, `CoreState` trusts the recorded `DecisionTrace` directly, guaranteeing 100% cross-OS determinism without disabling CPU SIMD/FMA optimizations during live inference.
- **Alternatives considered:** Strict bit-exact `libm` + no-FMA scalar matmul (slower, extra `libm` dependency); ONNX Runtime (large native binary, FFI complexity).

### Decision 5: Offline PyTorch Training & Calibration (`training/`)
- **Choice:**
  - Standalone `training/` directory (`uv` + PyTorch + `safetensors`, zero coupling to Cargo build).
  - Precompute `256d` `EmbeddingGemma 2` vectors once per dataset split into cached shards; train `150k` / `500k` / `1M` / `3M` models in `< 1–2 GB` VRAM on RTX 4060 Laptop (8 GB).
  - Strict OSI-permissive training corpus only (`when2speak`, `ubuntu_irc` A, `proactive_bench`, `pitch`, `capa`, `group_mem_bench`, `longmemeval`, `memory_agent_bench`, `stale`, `hi_tom`, `dyntom`, `MultiWOZ 2.2`). Exclude `CC-BY-NC-4.0` (`locomo`, `opentom`, `exploretom`, `tomi`) and missing-license repos from default weights. Never read `lab/scenarios/sealed/`.
  - Phased training with homoscedastic uncertainty weighting (*Kendall et al., 2018*) / `PCGrad` (*Yu et al., 2020*) when adding Phase 2/3 heads; post-hoc temperature scaling (*Guo et al., 2017*) + **2026 `CIC` finite-sample risk-controlled abstention thresholds** (*Dong & Shinnou, Jul 2026*, `arXiv:2607.04430`) evaluated via **`TC-ECE`** (*Lin et al., Sep 2026*, `arXiv:2609.07395`), single-turn `ECE`, `Brier`, and `AURC` (*Geifman & El-Yaniv, 2017*).

## Risks / Trade-offs

- **[Risk] Overfitting at `3M` parameters on `274d` frozen embeddings** → Mitigation: Phase 1 explicitly sweeps `150k`, `500k`, `1M`, and `3M` on held-out validation splits and ships the best-calibrated size (likely `150k–500k`) using the exact same Rust code.
- **[Risk] Breaking Gateway / Valve / Lab compilation during `crates/core` pruning** → Mitigation: Freeze Waist-1 (`ports.rs`, `protocol`, `ContextPlan`, `EventRecord` public surface) first; keep a thin compatibility adapter where `crates/lab` or `crates/gateway` reads legacy `Switches` until updated in the same change.
- **[Risk] Multi-task negative transfer across 5 heads** → Mitigation: Strict phased rollout (Phase 1 `gate_4`+`wait_delta_t` must pass calibration & Lab sanity checks before Phase 2/3 heads are trained).
- **[Risk] Indonesian/Gaul domain shift from English training corpora** → Mitigation: Rely on multilingual `EmbeddingGemma 2` (`256d`) + calibrate on held-out Indonesian dev traces once available; keep `online_adaptation` equalizer available per place.

## Migration Plan

1. **Step 0 — Git Archive:** Create branch `archive/v1-22-mechanisms` at current `main` HEAD (no code lost).
2. **Step 1 — Owner Sign-off on `ARCHITECTURE.md`:** Apply the agreed 3-invariant update to `ARCHITECTURE.md`.
3. **Step 2 — Pure-Rust Neural Engine + Phase 1 Training Pilot:** Build `crates/core/src/neural/` (`safetensors` loader, `PLR` + SwiGLU + `minGRU` + bounded adaptation + neutral fallback) and `training/` Phase 1 pipeline (`gate_4` + `wait_delta_t`), verifying Python↔Rust parity within `1e-5` and sweeping `150k`–`3M` on RTX 4060.
4. **Step 3 — Slim Core Pruning:** Replace the 22-mechanism gate/belief/consolidation/prior sprawl in `crates/core` with the minimal deterministic harness + `18d` feature extractor + Trace-Pinned Replay, keeping Waist-1 and all Gateway/Valve/Body/Lab tests green.
5. **Step 4 — Phase 2 & 3 Heads + Lab Verification:** Extend training and Rust optional heads to `p_uncertainty`/`p_verify` and `p_memory_relevance`/`p_encode_importance`, verify calibration (`ECE`, `Brier`, `AURC`) and conformance (`Delta_NI = 0`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`).

**Rollback:** Switch back to `archive/v1-22-mechanisms` at any time if V2 fails validation.
