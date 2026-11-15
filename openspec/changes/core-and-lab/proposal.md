# Proposal

## Why

MonoAgens asks one question: how far can a harness carry a mind? Today there is no code, so
nothing in `RESEARCH.md` can be measured. Following Deep Research Run 1 (`2026-11-15-run-1.md`)
and Run 2 (`2026-11-16-run-2.md`), this change specifies the complete sans-IO `Core`, `Store`,
`Reason`, and `Lab` stack (fixing all audited Round-1 defects and folding in Run-2 cognitive,
privacy-projection, erasure, long-horizon, self-model, security-taint, commitment, and TCK
contracts), paired cleanly with successor change `waist-valve-and-bodies` across Phases 0–3.

## What Changes

- Add a Rust workspace of five crates: `protocol`, `core`, `store`, `reason` and `lab`. The
  core has no IO, no async runtime, no clock and no threads of its own.
- Add the protocol's six facts and the five thought acts as versioned, serializable types.
- Add the append-only event log, record-first ordering, projections and exact replay.
- Add the cognitive loop with one queue and one consumer, three doors as empty hook points,
  seeded randomness, an injected clock and a registry of mechanisms with neutral values.
- Implement every verified cognitive, safety-projection, and Lab mechanism of `RESEARCH.md` §§1–26
  as corrected by Run 1 and Run 2 (`A-01`–`A-16`, `R-29`–`R-40`, `R-43`, `R-46`, `R-48`–`R-50`):
  - the gate: `A-14` split `dialogue_act_gap` logistic head vs epistemic `own_info_gap`, noisy-OR
    graded address `deg(e, l)` (`R-29`), post-evaluation partitioned habituation `h_{(p, k_src)}`
    (`A-03`, `R-36`), async snapshot sleep without `"or asleep"` deferral (`A-04`), shrinkage
    patience estimator (`A-08`), cold-start shrinkage (`A-07`), idle breakpoints & opportunity-cost
    drives (`R-30`, `R-31`), reachable neutral parameters (`A-01`);
  - memory & long-horizon store: guarded Petrov hybrid base-level (`n > k ∧ t_n > t_k`), exact
    `fade_threshold = -3.29` on `B`, `w_B` neutral switch (`A-01`), dual-process familiarity +
    recollection (`A-09`), Successor-Representation cognitive maps (`R-32`), pre-Step-5
    `VisibleScope` filtering (`R-33` `Delta_NI = 0`), segmented SQLite `hot.db` + sealed shards +
    abstracted `VectorIndex` (`usearch` primary HNSW + `hnsw_rs` pure-Rust / `BruteForceFlat`
    fallback) + `postcard`/`minicbor` (`R-40`), atomic `hard_forget` + `Two-Epoch Replay` (`R-34`);
  - expectations & commitments: plural `places`/`people`, `continued_attention`, `(channel,
    Δ_obs, Δ_pred)` agency, Option A preemption/barge-in `Receipt` extensions (`termination_reason`,
    `progress`, `delivered_units`, preserving `ARCHITECTURE.md` L198 5 outcomes), `RecurrenceSpec`
    silent polling (`R-46`), `PendingApproval` (`R-42`), `PromiseMeta` conflicts/repair &
    pre-utterance `CommitCheck` (`R-49`);
  - reasoning & beliefs: `[0, 20]` anchored confidence + `0/0` `P_rc` guard + `CitationLock`
    (`A-10`), progressive `Tool RAG` disclosure (`<= 4` static core + `k_cap = 8` capabilities +
    `k_proc = 3` procedures masked by `ActiveCaps ∩ VisibleScope`, `R-38`), Spotlighting
    `ContextPlan` & min-trust taint compaction (`R-43`), `OwnerSigned` `IdentityAnchor` +
    `[ANCHOR_CHECK]` (`R-48`), source credence including `told` & skeptical `k_corrob` (`A-01`,
    `R-43`), Hungarian-aligned domain centroids (`A-12`);
  - sleep, skills & self-model: `{open, provisional, sealed}` episodes (`A-05`), `[0, 1]` EM-LLM
    boundaries (`A-06`), contrastive lessons & `capability_schema_hash` / CUSUM procedures
    (`A-11`, `R-38`), and 4-pillar event-derived `sources.self_card` (`R-35`);
  - learning: sign-corrected `r ∈ {0, 1}` + clipped IPS `min(M_ips, 1/ρ)` (`A-01`, `R-19`),
    and Beta non-penalization guard on `canceled_by_core`/`preempted_external` receipts (`R-36`).
- Add `reason`: deterministic rendering, strict nullable JSON schemas (`A-10`), ablatable
  zero-drift `[SELF_GUIDE]` prefix rendering + just-in-time working-set reason cues synthesized
  from live `(CoreSelfSnapshot, ValveSelfSnapshot)` state (`self_mechanics_guide = false` neutral),
  unified `EmbeddingGemma 2` (`256d` MRL, Apache 2.0, `arXiv:2610.37364`) via native `ort` ONNX
  Runtime / `LiteRT` or local stdio/HTTP sidecar with mandatory task prefixes & `embed_model_ver`
  partitioning (`A-15`, `R-37`), `AttachmentHandle` resolution post-`VisibleScope`/`Valve`, plus
  scripted and OpenAI-compatible providers.
- Add the Lab: all 22 brain-map ablation metrics (`A-02`), Indonesian/`MultiLexNorm`/`IndoCollex`
  benchmarks (`A-15`), `S >= 354–471` paired 3-arm non-inferiority protocol (`A-16`), standardized
  `BaselineAdapter` comparing against `Stateless ReAct`, `Arm F`, `Mem0`, `Letta/MemGPT`,
  `LangGraph+LangMem`, `Zep/Graphiti`, and `Hermes Agent` on Holm-Bonferroni & convex Pareto
  scorecards across all 5 domains (`R-39`), `Delta_NI = 0` & `hard_forget` audits, identity drift
  & poisoning suites, `proptest` property tests + `criterion` `R-40` long-log/HNSW latency
  microbenchmarks, and black-box Core TCK hooks (`R-50`).

Left for paired change `waist-valve-and-bodies` (superseding early `bootstrap-vertical-slice` in git commit `ac3b240`,
co-developed across Phases 0–3): live `monoagens-gateway`, `cedar-policy` `Valve`, Waist-2
`monoagens.body.v1` JSON-RPC 2.0 transport, cross-body identity linking (`R-44`), keyed-`BLAKE3`
Attachment Store & Silero/VAP audio pipeline (`R-37`), `body-cli`, `body-mcp-bridge` (`MCP
2026-07-28`), honest delivery adapter (`R-47`), and standalone `monoagens-conformance` TCK
runner (`R-50`).

## Capabilities

### New Capabilities

- `protocol`: the six facts, the thought acts, versions and the serialized form.
- `event-log`: the append-only record, record-first ordering, projections and replay.
- `cognitive-loop`: the single queue and consumer, the doors as hook points, reproducibility
  and the mechanism registry.
- `appraisal-gate`: the model-free decision of whether a moment deserves reasoning, including
  patience and backlog chunking.
- `memory-recall`: ranking by activation, explicit recall, and telling recollection from
  familiarity.
- `expectations`: promises, predictions and waits; their conditions, the wake gate, grounding
  and judged silence.
- `reasoning`: context assembly, the thought acts, inner-thought bars and control of report.
- `beliefs`: what the soul may assert: basis, trace, validity, credence and source
  reliability.
- `sleep-consolidation`: consolidation while idle, event boundaries, gist, pattern separation
  and fading.
- `perception-ledger`: per-source facts about who witnessed what, and the soul's card of
  itself.
- `bounded-learning`: learning from outcomes within bounds, with every change on the record.
- `soul-identity`: the soul's name, persona and principles and how they change.
- `model-providers`: interchangeable language-model and embedding providers.
- `lab`: scenarios, test bodies, ablation, run records, scenario sets, metrics, live runs and
  the Lab's commands.

### Modified Capabilities

None. `openspec/specs/` is empty.

## Impact

- **Code**: new Rust workspace (`crates/`), `prior/`, `lab/` (scenarios, run records),
  `research/`, `docs/`. There is no existing code.
- **Toolchain**: Rust stable is required and is not installed on the development machine yet.
- **Dependencies**: SQLite (`rusqlite`, `PRAGMA secure_delete = ON`), abstracted `VectorIndex`
  (`usearch` pre-filtered HNSW primary + `hnsw_rs` pure-Rust / `BruteForceFlat` fallback), `blake3`,
  `postcard`/`minicbor` binary encoding (`store`), blocking HTTP + native `ort` (`ONNX Runtime`) /
  `LiteRT` or local sidecar (`reason`), and `proptest` + `criterion` for property invariants and
  scale microbenchmarks (`dev-dependencies`). `core` depends on `protocol`, `serde`, and pure
  deterministic math only (`0` IO/async/threads).
- **External systems**: live runs need an OpenAI-compatible chat endpoint and `EmbeddingGemma 2`
  (`256d` Apache 2.0, native `ort`/`LiteRT`, local sidecar, or endpoint) for text+multimodal recall.
  Default tests need neither.
- **Cost**: sleep, classification and learning add model calls. Each is recorded with its
  purpose, and the Lab's cost guard caps tokens per run and wakes per chain.
- **Documents**: `RESEARCH.md` and `research/findings/2026-11-16-run-2.md` are synchronized with
  `ARCHITECTURE.md`.
- **Architecture safety**: `Core` enforces pre-Step-5 `VisibleScope` non-interference (`R-33`),
  `hard_forget` erasure (`R-34`), `OwnerSigned` identity locking (`R-48`), and taint metadata
  (`R-43`), while runtime `cedar-policy` Valve rules and real body processes live in
  `waist-valve-and-bodies`.
- **Research**: Both Deep Research Run 1 (`2026-11-15-run-1.md`) and Run 2
  (`2026-11-16-run-2.md`, plus `run-2-notes/`) are folded into this specification along with
  owner-ratified decisions (`2026-11-16`): dual-change split with `waist-valve-and-bodies`,
  Option A additive `Receipt` fields preserving `ARCHITECTURE.md` L198, and `EmbeddingGemma 2`
  (`256d`, Apache 2.0) unified Gateway embedding. Remaining quantitative parameters are tuned in
  the Lab via deterministic ablation and the `S >= 354–471` paired protocol.
  Most formulas are the researcher's inference from sourced shapes, so they are expected to
  move with Lab results.
- **Evidence rule**: by `ARCHITECTURE.md`, a mechanism whose removal changes nothing measurable
  is removed. Some mechanisms built here may therefore be taken out again by their own
  ablation result.
