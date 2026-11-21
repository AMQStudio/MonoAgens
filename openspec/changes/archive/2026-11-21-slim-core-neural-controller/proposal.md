# Proposal: Slim Deterministic Core + Pure-Rust Neural Cognitive Controller (`V2`)

## Why

`crates/core/src/` has grown to **34,222 LOC** implementing **22 hand-crafted cognitive mechanisms** (`M01`–`M22`) and **184 manual hyperparameters** in `Prior`, making the core hard to read, tune, and maintain—while zero empirical Lab runs have validated those 184 knobs (`research/backlog.md`). Replacing soft cognitive scoring, timing, and adaptation heuristics with a **model-agnostic, Pure-Rust Neural Cognitive Controller** (`~150k–3M` parameters over frozen `256d` `EmbeddingGemma 2` vectors + `18` compact numeric features) cuts core complexity dramatically, works out of the box without touching LLM weights, and keeps the harness focused strictly on deterministic safety, privacy, routing, and the Valve.

## What Changes

- **Archive V1 (`core-and-lab` 22-mechanism core) to a dedicated git branch (`archive/v1-22-mechanisms`)** before refactoring `crates/core` on `main`, avoiding the burden of maintaining two parallel cores simultaneously.
- **BREAKING — Slim Deterministic Harness (`crates/core`):**
  - Remove the closed 22-mechanism registry/switch matrix (`M01`–`M22`) and collapse `Prior` from `184` fields to `~15–25` structural/safety configuration fields (`CoreConfig`).
  - Retain only hard deterministic invariants in the harness: append-only event log & crash recovery, `VisibleScope` privacy projection (`Delta_NI = 0`), `CommitmentBreaker` (`M07`), `ProspectiveMemory` deadlines/promises (`M13`), `Agency` self-echo detection (`M20`), provenance (`M16`/`M19` subset), hard daily budget/queue caps, and Waist-1 `Store`/`Doors` ports.
  - Extract a compact, model-agnostic `18`-scalar numeric feature vector (`x_num`) per percept (with optional `10d` `FModal` extension) alongside the existing frozen `256d` `EmbeddingGemma 2` embedding.
  - Adopt **Trace-Pinned Replay**: persist `(neural_outputs, model_blake3, feature_schema_version)` directly on `DecisionTrace` so cross-OS replay is 100% deterministic without sacrificing runtime SIMD speed.
- **Unified Pure-Rust Neural Cognitive Controller ("1 Neural Link"):**
  - Implement a zero-heavy-runtime Pure-Rust `f32` inference module (~250–400 LOC, no `ort`/`candle`/`burn`) loading dimension-agnostic **`safetensors`** weight files with `BLAKE3` verification and a safe neutral fallback (`None`) when no weights are loaded.
  - Architecture (grounded in 2025–2026 literature): Modality-Fusion **`TabM`** parameter-efficient implicit ensemble (`ICLR 2025`, `arXiv:2410.24210`, supplying free inter-member epistemic uncertainty) over `PLR` + Pre-LN SwiGLU trunk + per-place recurrent working memory (`64d`, sweeping `minGRU` vs **`Gated DeltaNet-2`** decoupled erase/write gates, `arXiv:2605.22791`) + bounded per-place **Warm-Start UCB online adaptation equalizer** (`MemCon` `arXiv:2607.13591` & `arXiv:2610.23812`, frozen backbone + bounded bias/FiLM with an explicit `online_adaptation: bool` debug toggle).
  - Phased multi-head outputs respecting Core's boundary (no raw text understanding, no Valve override):
    - **Phase 1:** `gate_4` (`Silent | Note | Wait | Go`) + `wait_delta_t` (aligned with `arXiv:2605.30152`, `arXiv:2610.05159`)
    - **Phase 2:** `p_uncertainty` (augmented by `TabM` ensemble variance) + `p_verify` (metacognitive context/budget hints)
    - **Phase 3:** `p_memory_relevance` (Retrieval Gate) + `p_encode_importance` (Memory Write Gate, `arXiv:2609.00237`)
- **Offline Training & Calibration Pipeline (`training/`):**
  - Python/PyTorch training pipeline targeting RTX 4060 Laptop (8 GB VRAM) over precomputed `256d` embeddings using strictly **OSI-permissive datasets** (`when2speak`, `ubuntu_irc` A, `proactive_bench`, `pitch`, `capa`, `group_mem_bench`, `longmemeval`, `memory_agent_bench`, `stale`, `hi_tom`, `dyntom`, `MultiWOZ 2.2`), with strict anti-leakage hygiene (never reading `lab/scenarios/sealed/`), post-hoc temperature scaling + **2026 `CIC` finite-sample risk control** (`arXiv:2607.04430`) + multi-turn **`TC-ECE`** (`arXiv:2609.07395`), `ECE`, `Brier`, `AURC`, and 1-line `safetensors` export.
- **BREAKING — `ARCHITECTURE.md` Amendment (Owner-Approved):**
  - Update the three V1-specific invariants (closed 22-mechanism brain map, per-mechanism neutral switches, and "one gate formula") to specify *Deterministic Harness + Learned Neural Controller with Neutral Fallback*, while keeping all privacy, Valve, protocol, no-raw-content, record-first, and reproducibility invariants unchanged.

### Reconciliation with Existing In-Flight OpenSpec Changes

Because `openspec/specs/` is currently empty and four changes remain unarchived in `openspec/changes/`:

| Existing Change | Status | Relationship to `slim-core-neural-controller` |
|---|---|---|
| `core-and-lab` (`100/102` tasks) | 2 empirical run tasks open (`22.3`, `23.7`) | **Superseded for Core** after archiving V1 to `archive/v1-22-mechanisms`; `crates/store`, `crates/reason`, `crates/protocol`, and `crates/lab` harness remain reused. |
| `waist-valve-and-bodies` (`30/33` tasks) | 3 conformance/calibration tasks open (`7.1`, `8.2`, `8.3`) + Gateway `approval/resolve` gap (`-32601`) | **Preserved & Compatible.** Waist-1 (`Store`, `Doors`), Waist-2, Gateway, Valve, CLI, and Bodies remain intact; Core V2 MUST keep Waist-1 compatibility so completed Gateway/Valve/Body work is not discarded. |
| `lab-datasets` (`45/47` tasks) | 2 live-run tasks open (`8.3`, `10.4`) | **Preserved & Reused.** Existing dataset manifest, pin/fetch/verify CLI, and converters directly feed V2 training & held-out Lab evaluation. |
| `lab-memory-comparison` (`7/15` tasks) | Phase 0–2 adapters & runs open (`4.1`–`5.6`) | **Preserved & Orthogonal.** End-to-end LoCoMo/LongMemEval-S memory comparison works directly against Slim Core V2. |

## Capabilities

### New Capabilities

- `slim-core-harness`: Minimal deterministic cognitive loop, append-only event log, `VisibleScope` privacy projection, hard commitments/deadlines, self-echo agency detection, `18d` numeric feature extraction, and Trace-Pinned Replay.
- `neural-controller`: Pure-Rust `safetensors` neural cognitive controller (`PLR` + Pre-LN Residual/SwiGLU + `minGRU` `64d` working memory + bounded per-place online adaptation with debug toggle), `BLAKE3` verification, neutral fallback, and phased multi-head control outputs.
- `neural-training-pipeline`: Offline PyTorch multi-stage training, OSI-permissive dataset preparation with anti-leakage splits, probability calibration (`Temperature Scaling`, `ECE`, `Brier`, `AURC`), and `safetensors` export.

### Modified Capabilities

*(None — `openspec/specs/` currently contains no archived main specs; all V1 capabilities live in unarchived changes under `openspec/changes/` and are reconciled in the table above.)*

## Impact

- **Crates affected:** `crates/core` (major simplification: `34.2k` → `~16k` LOC and `184` → `~20` config fields; adds `safetensors` crate), `crates/gateway` (minor wiring to pass optional `.safetensors` model path and `CoreConfig`), `crates/lab` (add neural controller evaluation, calibration metrics `ECE`/`Brier`/`AURC`, and shadow/phase verification).
- **Crates preserved untouched (contract-compatible):** `crates/protocol`, `crates/valve`, `crates/store`, `crates/reason` (reuses `EmbeddingGemma2Embedder`), `crates/conformance`, `crates/cli`, and all `bodies/*`.
- **New directory:** `training/` (Python/PyTorch scripts managed via `uv`, strictly offline, zero runtime dependency from Rust).
- **Documentation:** Owner-approved amendment to `ARCHITECTURE.md` invariants; grounded in `research/findings/2026-11-21-run-5.md` (`R-59`–`R-64`).
