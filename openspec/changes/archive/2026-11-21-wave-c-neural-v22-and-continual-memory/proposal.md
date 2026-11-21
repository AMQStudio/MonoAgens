# Proposal

## Why

Wave A established the calibrated Pure-Rust neural foundation (`150k + minGRU`, `TaskPrefix` parity, non-vacuous `CIC`, Stage 4 DPO/KTO + `CFPO`) and Wave B landed the multi-day evaluation suite (`dev-09..13`, `val-03`), out-of-band `I-HITL`, and deterministic multi-bot `PeerLoopBrake`. However, the Wave B 5-seed offline baseline exposed the exact ceiling of `feature_schema_version = 1` (`18d` `x_num`): `dev-10-multiparty-self-repair` (`3/5`), `dev-13-multi-agent-pingpong-and-chorus` (`1/5`), and `dev-07-habituation` (`0/5`) suffer because the neural controller cannot observe repair geometry, interleaved threads, per-participant history, or automated-peer cues, every `Go` wake invokes monolithic `High` reasoning effort, and `EpisodeBook`/`BeliefBook`/`SkillBook` lack multi-topic disentanglement, `MemCon` recall-depth control, `RD-Forget` superseded recall, and three-timescale preference drift protection.

## What Changes

- **Unified `feature_schema_version = 2` Neural Cognitive Controller (`crates/core/src/neural/`, `crates/core/src/gate/`, `crates/core/src/ports.rs`, `crates/reason/src/model/`, `crates/gateway/src/runtime.rs`):**
  - Extend model-agnostic post-`VisibleScope` numeric features from `18d` (`schema = 1`) to `32d` (`schema = 2`) by appending `6d` conversational repair features (`R-68`), `4d` thread/actor features (`R-69`), and `4d` peer-agent features (`R-71`), while preserving 100% backward compatibility with `schema = 1` `18d` `safetensors` artifacts.
  - Add two lightweight optional V2.2 heads (`< +9K` params): `head.compute_tier` (`[Fast, Standard, Deep]` coupled with `p_verify`/`epistemic_uncertainty` escalation floors → `ContextPlan.compute_tier` → per-request `ModelRequest.reasoning_effort`) and `head.p_user_correction` (driving Loop A targeted repair wake metadata and Loop B event-sourced propensity-weighted delayed negative `AdaptFeedbackApplied` on `GateArm::Go`, `Taint`-guarded).
  - Add optional factorized per-participant working state `h_actor` (`24d`–`32d` `minGRU`/`Gated DeltaNet-2`) keyed strictly by `(PlaceId, CanonicalAudienceHash, ActorId)` (`Delta_NI = 0`), capped at `K_ACTORS = 32` per room via deterministic `(last_active_ms, ActorId)` LRU eviction in `BTreeMap`, persisted with snapshot upcast, and scrubbed cleanly on `EventKind::HardForget` + audited in `HardForgetAudit`.
  - Fix `state/mod.rs` `actor_exchanges` so participant exchange statistics update in group chats (`audience.len() > 2`).
- **Streaming 3W Disentanglement & Continual Chat Memory (`crates/core/src/consolidation/`, `crates/core/src/recall/`, `crates/core/src/beliefs/`, `crates/core/src/context.rs`, `crates/core/src/turn/`, `crates/reason/src/prompt/`):**
  - Upgrade `EpisodeBook` from single-slot `active_by_place` to `M <= 6` active thread slots keyed by `(PlaceId, CanonicalAudienceHash)` with deterministic 3W (`Who`/`What`/`When` + `reply_to` fast-path + `EventId` tie-break) thread assignment, recording `thread_id` on `PlaceHistoryEntry`/`MemoryEntry` and prioritizing trigger-thread siblings in `[PLACE_TAIL]`.
  - Extend `PlaceAdaptState` with a bounded `MemCon`-style discrete recall-depth bandit (`Skip / Light / Standard / Deep+Proc`, neutral fallback keeps fixed defaults).
  - Add `RD-Forget` `RecallTimeMode { CurrentOnly, IncludeSuperseded }` to `RecallQuery` and separate `recalled_event_ids` vs `cited_event_ids`.
  - Add `max_fade_fraction` and durable preference anchor protection in `fade.rs`, plus audience-locked tier-2 temporal episode rollups.
  - Add three-timescale preference drift in `BeliefBook` (fast `BetaCredence` shadow `lambda_fast = 0.80/day`, episode-gated slow `BeliefBook` strictly `DomainId`-scoped against `MEMDRIFT`, and immutable non-decaying `OwnerSigned` `IdentityAnchor`/`principles`).
  - Upgrade `SkillBook` with Bayesian UCB procedure ranking (`posterior_mean + c * ucb_bonus`) and 4-state `SkillForge` lifecycle (`Trial -> Active -> Stable -> Retired`) supporting single-room multi-session `Trial` chat procedures (`distinct_episodes >= 2`).
- **Training Pipeline V2.2 & Indonesian/SEA Calibration (`training/src/monoagens_train/`):**
  - Upgrade PyTorch `MonoNeuralController`, `features.py`, `export.py`, `train_multitask.py`, and `posttrain_preference.py` to support `feature_schema_version = 2` (`32d` `x_num`, optional `h_actor`, `head.compute_tier`, `head.p_user_correction`) with `<= 1e-5` Python↔Rust parity on both `v1` and `v2` golden fixtures.
  - Add Regime 4 (*pecah balon* multi-bubble burst patience → `Wait`), Regime 5 (spherical code-switch drift + `max_visible_mem_cos` attenuation), multi-party self-repair (`dev-10`), peer-agent (`dev-13`), repair (`p_user_correction`), and `compute_tier` curricula in `augment.py`, plus OSI-clean dataset loaders enforcing `ALLOWED_OSI_SPDX`.
  - Retrain and export `schema = 2` weights offline on RTX 4060 (`$0` API) and validate with `--release` 5-seed `13 dev + 3 val` suites.

## Capabilities

### New Capabilities
- *(none — all Wave C additions extend existing core, neural, and training capabilities with `+0` net Core primitives)*

### Modified Capabilities
- `neural-controller`: Add backward-compatible `feature_schema_version = 2` (`32d` `x_num`), optional `head.compute_tier` + `head.p_user_correction` heads, optional audience-scoped `h_actor` working state, and `MemCon` recall-depth bandit on `PlaceAdaptState`.
- `slim-core-harness`: Extend post-`VisibleScope` numeric feature extraction to `32d` (`schema = 2`), add streaming 3W multi-topic disentanglement in `EpisodeBook` + `[PLACE_TAIL]`, add `RD-Forget` `RecallTimeMode`, `max_fade_fraction` & preference anchor fade protection, three-timescale `DomainId`-scoped preference drift, and `SkillForge` UCB single-room `Trial` procedures.
- `neural-training-pipeline`: Support `feature_schema_version = 2` (`32d` `x_num`, `h_actor`, `compute_tier`, `p_user_correction`) export/parity, Regime 4 (*pecah balon*) & Regime 5 (code-switch drift) + self-repair/peer-agent curricula, and OSI-clean SEA/preference dataset loaders.

## Impact

- **Core (`crates/core`):** `neural/{mod,weights,features,forward,adapt}.rs`, `gate/decide.rs`, `ports.rs`, `state/{mod,snapshot}.rs`, `consolidation/{episodes,fade,skills}.rs`, `recall/{engine,recollection}.rs`, `beliefs/credence.rs`, `context.rs`, `turn/{step,context_emit,sleep_and_beliefs}.rs`.
- **Reason & Gateway (`crates/reason`, `crates/gateway`):** Per-request `ModelRequest.reasoning_effort` override from `ContextPlan.compute_tier`, thread-aware `[PLACE_TAIL]` rendering.
- **Training (`training/`):** `model.py`, `features.py`, `augment.py`, `datasets.py`, `train_multitask.py`, `posttrain_preference.py`, `export.py`, `licenses.py`, plus pytest & Rust parity tests.
- **Invariants:** `+0` net Core primitives, zero raw text/word lists/emotions in Core, mandatory `None` neutral fallback, `Delta_NI = 0`, deterministic replay, bit-clean `hard_forget`, zero paid API calls, zero `ARCHITECTURE.md` edits, zero `lab/scenarios/sealed/` access.
