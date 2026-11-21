# Research brief 5 for MonoAgens: Slim Deterministic Harness + Pure-Rust Neural Cognitive Controller (`R-59`–`R-63`)

You are the researcher for MonoAgens. Read `research/researcher-prompt-2.md` for the project context
and standing research rules; they still hold. This round investigates how to dramatically reduce the
complexity and maintenance burden of `crates/core` (currently ~34k LOC, 22 hand-crafted mechanisms,
184 fields in `Prior`) by shifting soft cognitive gating/scoring into a **model-agnostic, Pure-Rust
Neural Cognitive Controller** (~50k–3M parameters) while keeping the harness focused strictly on
deterministic mechanics, routing, policy, privacy, and the Valve.

## Read first

Repository: local workspace `/home/muqsith/Projects/Mono` (branch `main`).

1. `ARCHITECTURE.md` — especially the **Architecture invariants**, Core, Valve, Reproducibility,
   and Complexity budget sections.
2. `crates/core/src/registry.rs`, `crates/core/src/prior.rs`, `crates/core/src/gate/signals.rs`,
   `crates/core/src/gate/decide.rs`, and `crates/reason/src/info_gap_clf.rs` (existing frozen
   logistic head pattern).
3. `research/backlog.md` and `research/findings/2026-11-17-run-3.md` (existing cataloged datasets
   for memory, proactive/prospective, timing/group-chat, ToM, Indonesian & security).
4. `research/researcher-prompt-2.md` — standing verification and citation rules.

**Never read `lab/scenarios/sealed/`. Do not modify any Rust source code, `ARCHITECTURE.md`, or
OpenSpec files during this research run.**

## Rules for this brief

- Every factual or empirical claim needs a primary source: peer-reviewed paper / arXiv, official
  crate/library docs, official repo with pinned version/commit, or exact local workspace file +
  line range.
- Say plainly when something cannot be confirmed empirically without running local benchmarks.
- Do not choose for the owner. Where an architectural or design choice is needed, present clear
  options, tradeoffs, and concrete implications.

## Questions

**R-59 Pruning the 22 mechanisms into a Slim Deterministic Harness + Neural Policy.**
Inspect all 22 mechanisms in `crates/core/src/registry.rs` (`M01`–`M22`) and the 184 fields in
`crates/core/src/prior.rs`:
- Which mechanisms MUST remain deterministic in the harness (safety, privacy `VisibleScope`,
  event log / replay, explicit commitments/deadlines, self-caused echo/agency, hard forgetting)
  and why a neural network cannot replace them?
- Which mechanisms are fundamentally soft scoring / thresholding / attention / gating functions
  that can be unified into a small learned policy over frozen embeddings + compact numeric state
  features?
- What is the minimal, model-agnostic numeric feature vector (e.g. 12–25 scalar features alongside
  the existing `256d` `EmbeddingGemma 2` vector) that preserves the signal expressiveness of
  `GateSignals` without 184 hand-tuned knobs?
- Estimate the realistic LOC and knob reduction in `crates/core` under a slim harness architecture.

**R-60 Neural Controller Architecture, Parameter Budget (50k vs 200k vs 1M vs 3M) & Calibration.**
Using primary ML literature on tabular + dense-embedding fusion, selective prediction, and small
policy/gating networks:
- Compare candidate architectures for ~270–300d inputs (frozen `256d` embedding + ~15–25 numeric
  features): plain MLP, Residual MLP (Pre-LN + SiLU/GELU + Dropout), Gated/SwiGLU MLP, and
  modality-fusion heads (separate numeric projection + embedding projection).
- Analyze parameter scaling (`~50k`, `~200k`, `~1M`, `~3M` parameters) vs dataset size (`1k`, `10k`,
  `50k`, `100k+` labeled events): where does capacity help vs where does a 3M-parameter MLP
  overfit on low-dimensional structured + frozen-embedding inputs?
- What minimal output heads actually respect MonoAgens's Core boundary (Core decides *when to
  wake / note / stay silent* and *memory relevance*, NOT language understanding or policy
  override)? How should probability calibration (temperature scaling, Brier score, ECE,
  selective prediction risk-coverage) be measured?

**R-61 Pure-Rust Deterministic Inference & Weight Format (Zero Heavy Runtime vs ONNX/Candle/Burn).**
Evaluate running the Neural Controller inside `crates/core` (or a thin leaf crate) in Rust:
- Compare pure-Rust forward pass (`f32` / `f16` / `int8` quantized weights, ~100–250 LOC, zero
  heavy deps) against `ort` (ONNX Runtime), `candle-core`, and `burn`: binary size, compile time,
  dependency weight, CPU single-thread latency for `150k` / `500k` / `3M` params, and **exact
  cross-platform deterministic replay** (`DecisionTrace::recompute`).
- How should IEEE-754 floating-point determinism (e.g., FMA contraction differences across x86/ARM,
  SIMD reduction order) be handled so event-log replay never drifts? (Including recording neural
  output scores + BLAKE3 model hash + feature schema version on every `DecisionTrace`.)
- Propose a clean, **dimension-agnostic** weight file format (e.g., `safetensors` or minimal
  versioned binary header + BLAKE3) so the exact same Rust inference code runs `50k`, `200k`,
  `1M`, or `3M` parameter weights interchangeably, with a neutral fallback (`None` → neutral score)
  when no weight file is loaded.

**R-62 Out-of-the-Box Training Pipeline & Dataset Strategy (RTX 4060 8 GB VRAM, 14 GiB RAM).**
How do we train a default "out-of-the-box" Neural Controller that generalizes across LLMs without
requiring users to tune knobs?
- Which datasets already cataloged in `research/findings/2026-11-17-run-3.md` and
  `crates/lab/src/datasets/` (plus other open, permissively licensed datasets for turn-taking,
  proactive interruption, dialogue-act/clarification, and memory relevance) can be converted into
  training examples for `(embedding_256d, numeric_features) -> (p_wake, p_relevance, ...)`?
- Compare training stages: supervised multi-task pretraining → teacher-student distillation (from
  strong LLM or deterministic oracle) → DAgger / hard-negative mining → offline contextual bandit.
- Give realistic VRAM, RAM, and wall-clock training budgets on an **RTX 4060 Laptop (8 GB VRAM)**
  for `150k`–`3M` parameter models over frozen `256d` embeddings.
- Specify strict anti-leakage hygiene so training never touches `lab/scenarios/sealed/` or
  validation splits.

**R-63 Invariant Impact (`ARCHITECTURE.md`) & Migration Options.**
- Identify which specific **Architecture invariants** in `ARCHITECTURE.md` are preserved as-is and
  which ones require an explicit owner decision/amendment (e.g., closed 22-mechanism brain map,
  neutral-value single loop, Lab-evidence-gated changes, complexity budget).
- Lay out concrete migration options (e.g., Option A: Shadow-mode Neural Gate in existing Core →
  Lab Pareto proof → progressive pruning; Option B: Clean-slate `core-v2` slim harness + neural
  controller validated side-by-side in `crates/lab` before replacing `crates/core`) with clear
  engineering tradeoffs.

**R-64 Unified "1 Neural Link" Cognitive Backbone (Working Memory `h_t` + 5 Cognitive Heads + Fast Online Adaptation).**
The owner wants to evaluate fitting all non-linguistic human cognitive control functions into a
single unified Pure-Rust neural backbone ("1 Neural Link") while keeping LLM reasoning, Valve
policy/privacy, and explicit commitments/logs outside the neural layer:
- **Recurrent Working Memory (`h_t`)**: Compare stateless MLP vs tiny recurrent state (`GRU`,
  `minGRU`, `MGU`, or gated EMA `32d`–`128d` per `place`) for capturing cross-turn dynamics
  (habituation M03, event boundaries M21, turn-taking rhythm, backlog pacing M22) with exact
  deterministic replay and compact snapshot serialization.
- **Unified Multi-Head Cognitive Control**: Evaluate attaching 5 compact heads to 1 shared
  backbone:
  1. `gate_4` (`Silent | Note | Wait | Go`) + `wait_delta_t` (social inhibition / M02 + M08);
  2. `p_uncertainty` & `p_verify` (metacognition / Feeling of Knowing / skepticism — replacing
     M09, M14 heuristics, M17, `info_gap_clf`, used strictly as numeric context/budget hints);
  3. `p_encode_importance` (hippocampal memory encoding / forgetting gate — replacing M11/M18
     heuristics);
  4. `p_memory_relevance` (reranker over `VisibleScope` candidates — M10);
  5. **Fast Online Adaptation (Neuromodulation / M15)**: compare frozen backbone + bounded
     per-place bias/FiLM/linear-head bandit updates vs pure `h_t` recurrent in-context adaptation
     without catastrophic forgetting or breaking deterministic replay.
- **Multi-Task Interference & Phased Rollout**: Cite primary literature on multi-task negative
  transfer across heterogeneous heads on small/medium datasets (e.g., Kendall et al. uncertainty
  weighting, PCGrad, staged head training), and outline which heads can be trained/validated
  together vs phased.

## Deliver

Write:
1. `research/findings/2026-11-21-run-5.md` with an **Executive Summary & Owner Choices Needed**
   section followed by one thorough section per question (`R-59` through `R-64`).
2. Supporting detailed tables/notes in `research/findings/run-5-notes/` as needed.
