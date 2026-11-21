# Design: Wave C — Unified `schema = 2` Neural Layer V2.2 & Continual Chat Memory

## Context

Wave A fixed foundational bugs (`C-1`–`C-4`, `C-8`–`C-10`) and proved `$0` Stage 4 DPO/KTO + `CFPO` post-training on `schema = 1` (`18d` `x_num`). Wave B landed `I-HITL` out-of-band `approval/resolve`, `PeerLoopBrake`, and the 5-seed multi-day Lab benchmark (`dev-09..13`, `val-03`). Wave B's offline sweep confirmed `15/15` PASS on `validation` (`TC-ECE = 0.0178`) and `52/65` on `development`, pinning the exact gaps that Wave C resolves:
1. `dev-10-multiparty-self-repair` (`3/5`) and `dev-13-multi-agent-pingpong-and-chorus` (`1/5`) need repair/thread/actor/peer-agent features (`32d` `x_num` `schema = 2`) and updated offline curricula.
2. Monolithic `High` `reasoning_effort` on every `Go` wake (`C-12`) needs `head.compute_tier` routed cleanly through `ContextPlan` → `ModelRequest.reasoning_effort` with `p_verify`/`epistemic_uncertainty` escalation floors.
3. Single-slot `EpisodeBook` (`C-11`), fixed recall depth, single-mode `RecallQuery`, unguarded `fade.rs`, single-timescale `BetaCredence` (`C-15`), and cross-place-only `SkillBook` procedures need streaming 3W disentanglement, `MemCon` recall-depth bandit, `RD-Forget` `RecallTimeMode`, `max_fade_fraction` + preference-anchor protection, three-timescale `DomainId`-scoped drift, and `SkillForge` UCB single-room `Trial` procedures.

## Goals / Non-Goals

**Goals:**
- Upgrade `NeuralModel` and `CoreState` to `feature_schema_version = 2` (`32d` `x_num`, optional `has_v22_heads`, optional `h_actor_dim` keyed by `(PlaceId, CanonicalAudienceHash, ActorId)`) with zero breakage of `schema = 1` (`18d`) weights.
- Keep `+0` net Core primitives, zero raw text/word lists/emotions in Core, mandatory `None` neutral fallback, `Delta_NI = 0`, deterministic event-sourced replay, and bit-clean `hard_forget`.
- Achieve clean parallel implementation across 3 strictly disjoint subagent tracks, then retrain `schema = 2` offline on RTX 4060 (`$0` API) and verify with `--release` 5-seed `13 dev + 3 val` suites.

**Non-Goals:**
- No edits to `ARCHITECTURE.md` or `crates/protocol` breaking wire contracts.
- No reading or touching `lab/scenarios/sealed/`.
- No paid live LLM API calls (Wave D remains separate and requires explicit owner approval).

## Decisions

### 1. Canonical `feature_schema_version = 2` Contract (`32d` `x_num`, Backward Compatible)
- `x_num[0..18]`: Exact `schema = 1` features (`DEFAULT_NUM_FEATURES_V1 = 18`).
- `x_num[18..24]` (**6d Repair, `R-68`**):
  - `18`: `has_recent_visible_soul_act` (`0.0` or `1.0`, visible soul act within recent window)
  - `19`: `cos_soul_user` (`cosine_similarity(percept_emb, last_visible_soul_emb)` in `[0, 1]`, `0.0` if vectorless)
  - `20`: `cos_prev_user_rephrase` (`cosine_similarity(percept_emb, last_visible_sender_emb)` in `[0, 1]`)
  - `21`: `cos_expected_reply` (max cosine similarity between `percept_emb` and visible active `ResponsePrediction` embedding, `0.0` if none)
  - `22`: `is_reply_to_last_soul` (`1.0` if `reply_to` points to last visible soul event)
  - `23`: `soul_intervened_since_sender_last` (`1.0` if a visible soul act occurred after sender's previous visible message)
- `x_num[24..28]` (**4d Thread/Actor, `R-69`**):
  - `24`: `max_active_thread_cos` (best cosine similarity against visible active thread centroids in `(PlaceId, CanonicalAudienceHash)`)
  - `25`: `active_thread_count_norm` (`min(active_threads, 6) / 6.0`)
  - `26`: `sender_in_top_thread` (`1.0` if sender participates in the highest-scoring active thread)
  - `27`: `sender_reliability` (`BeliefBook::source_reliability_at(sender, general, now).expected()` in `[0, 1]`)
- `x_num[28..32]` (**4d Peer-Agent, `R-71`**):
  - `28`: `is_sender_automated_peer` (`1.0` if `actor_facts[sender].automated_peer`)
  - `29`: `room_has_other_automated_peers` (`1.0` if another visible participant is `automated_peer`)
  - `30`: `consecutive_automated_turns_norm` (`min(consecutive_automated_turns, 6) / 6.0`)
  - `31`: `peer_expertise_margin` (`percept.peer_expertise_margin` mapped `[-1, 1] -> [0, 1]` via `0.5 * (m.clamp(-1, 1) + 1.0)`, `0.5` when `None`)
- When a `schema = 1` (`d_in_num == 18`) model is loaded, `forward_step` accepts `x_num[..18]` transparently.

### 2. Optional V2.2 Heads (`has_v22_heads`) & Factorized `h_actor`
- **Tensor names (`safetensors` `F32`):**
  - `head.compute_tier.weight`: `[3, d_model]`, `head.compute_tier.bias`: `[3]` (`[Fast, Standard, Deep]`).
  - `head.p_user_correction.weight`: `[1, d_model]`, `head.p_user_correction.bias`: `[1]`.
  - Optional `h_actor_dim > 0` (`default 0`, V2.2 `24` or `32`):
    - `stem.fusion.weight`: `[d_model, d_e + d_n + h_dim + h_actor_dim]` (backward compatible because when `h_actor_dim == 0` width is unchanged!).
    - Actor recurrent cell tensors prefixed `actor_memory.mingru.*` or `actor_memory.gdn2.*` matching `cell_type`.
- **Compute-tier escalation floor (`ODAR`):** If `p_verify >= 0.70 || epistemic_uncertainty >= 0.35`, escalate `Fast -> Standard` (and `Standard -> Deep` if `p_verify >= 0.85 || epistemic_uncertainty >= 0.55`). Attach `ComputeTier` (`Fast | Standard | Deep`) to `ContextPlan.compute_tier`, mapped in Gateway/Reason to `ModelRequest.reasoning_effort` (`Low | Medium | High`) overriding static `OpenAiModelConfig.reasoning_effort` per request.
- **`p_user_correction` Dual Loops:**
  - Loop A: When `p_user_correction >= 0.65` and a recent visible soul event exists, populate `ContextPlan.repair_context_ids = vec![last_soul_event_id, prior_user_event_id]` on wake.
  - Loop B: Emit event-sourced `EventDetail::AdaptFeedbackApplied` with propensity-weighted delayed negative reward (`-0.35 * (p_user_correction / p_go.max(0.15)).clamp(0.0, 1.0)`, `is_delayed_one_sided: true`) on `GateArm::Go`, strictly subject to existing `Taint::Untrusted` guards (corroborated/trusted sources only).
- **Audience-scoped `h_actor` (`Delta_NI = 0`):** Stored in `CoreState::actor_memory: BTreeMap<(PlaceId, [u8; 32], ActorId), ActorMemorySlot>` (`{ vector: Vec<f32>, last_active_ms: Time }`), capped at `K_ACTORS = 32` per `(PlaceId, AudienceHash)` with deterministic `(last_active_ms, ActorId)` LRU eviction. Persisted in `snapshot.rs` (snapshot format version bump with clean backward-compatible read of older snapshots), wiped completely in `CoreState::apply_event` on `EventKind::HardForget` and checked in `HardForgetAudit`.

### 3. Streaming 3W Multi-Topic Disentanglement & Continual Chat Memory
- **Canonical Audience Hash:** `canonical_audience_hash(audience: &Audience) -> [u8; 32]` via BLAKE3 over sorted unique actor IDs (reused consistently by both `h_actor` and `EpisodeBook`).
- **`EpisodeBook` Multi-Slot (`M = 6`):**
  - Replace single-slot lookup with `active_slots: BTreeMap<(PlaceId, [u8; 32]), Vec<EventId>>` (keeping `active_by_place` synchronized with the most-recently-updated slot per place for backward compatibility with existing callers/snapshots).
  - Score active slots via `0.55 * sem_cos + 0.25 * actor_overlap + 0.20 * exp(-dt_s / 600.0)` with `reply_to`/`caused_by` fast-path (`+1.0` match if parent event belongs to an active slot) and deterministic `EventId` tie-breaking. Open a new slot if best score `< 0.35` or `missed_expectation`; if already at `M = 6`, seal the oldest-`end_ms` slot.
  - Record `#[serde(default)] pub thread_id: Option<EventId>` on `PlaceHistoryEntry` and `MemoryEntry`. In `context_emit.rs` `[PLACE_TAIL]`, keep trigger-thread siblings prioritized ahead of interleaved off-thread chatter while respecting byte budgets and `VisibleScope`.
  - Fix `state/mod.rs` `actor_exchanges` guard so multi-party rooms (`audience.len() > 2`) update sender exchange traces (instead of dropping exchanges whenever `recipients > 3`).
- **`MemCon` Recall-Depth Bandit (`adapt.rs` + `context_emit.rs`):**
  - Add `#[serde(default)] pub recall_depth_bandit: RecallDepthBanditState` (`4` arms: `Skip(0)`, `Light(4)`, `Standard(default)`, `DeepPlusProc(max*2)`) to `PlaceAdaptState`, selected via deterministic Warm-Start UCB when `online_adaptation = true` and `model.is_some()`, preserving fixed `config.max_recalled_memories` under neutral fallback or `online_adaptation = false`.
- **`RD-Forget` `RecallTimeMode` & Cited vs Recalled (`recall/engine.rs` + `context_emit.rs`):**
  - Add `#[serde(default)] pub time_mode: RecallTimeMode` (`CurrentOnly` default, `IncludeSuperseded`) to `RecallQuery` and `#[serde(default)] pub cited_event_ids: Vec<EventId>` alongside `recalled_event_ids` on `RecallResult`/wake tracking.
- **Fade Guard & Tier-2 Rollups (`consolidation/fade.rs` & `episodes.rs`):**
  - Add `#[serde(default)] pub is_preference_anchor: bool` on `MemoryEntry` (exempt from Petrov fade) and cap `select_fade_candidates` at `max_fade_fraction` (default `0.25`, deterministically selecting lowest-`B` candidates with `EventId` tie-break).
  - Add audience-locked tier-2 temporal rollup helper on `EpisodeBook` summarizing sealed episodes within the same `(PlaceId, CanonicalAudienceHash)` without ever crossing audience boundaries.
- **Three-Timescale `BeliefBook` (`beliefs/credence.rs`):**
  - Add `fast_credence: Option<BetaCredence>` (`lambda_fast = 0.80/day`) alongside medium/slow `credence` (`0.98/day`), strict `(DomainId, claim_key)` key isolation in `active_claims` so chat-domain preference updates never overwrite tool-domain beliefs (`MEMDRIFT` guard), and verify `OwnerSigned` `IdentityAnchor`/`principles` never decay.
- **`SkillForge` Lifecycle & Bayesian UCB (`consolidation/skills.rs` & `context.rs`):**
  - Extend `ProcedureLifecycleState` with `Trial` and `Stable` (`Trial -> Active -> Stable -> Retired`, keeping `Warned` backward-compatible).
  - Allow single-room multi-session `Trial` chat procedure admission when `distinct_episodes >= 2` (even within 1 place), promoting `Trial -> Active` on corroborated successes and `Active -> Stable` at high posterior reliability.
  - Add `ucb_score(total_proc_trials, c)` on `TrackedProcedure` and expose `reliability_ucb: f64` on `ProcedureEntry` so `select_tools_for_context` ranks procedures deterministically by relevance + Bayesian UCB.

### 4. Parallel Subagent Track Partition (Zero File Overlap)
- **Subagent 1 (`Wave C Neural V2.2`):** `crates/core/src/neural/*`, `crates/core/src/gate/*`, `crates/core/src/ports.rs`, `crates/core/src/state/{mod,snapshot}.rs`, `crates/core/src/turn/step.rs`, `crates/core/src/event.rs` (only if `DecisionTrace` / `AdaptFeedbackApplied` needs additive fields), `crates/core/src/tck_hooks.rs`, `crates/reason/src/model/{mod,openai}.rs`, `crates/gateway/src/runtime.rs`.
- **Subagent 2 (`Wave C Memory & Disentanglement`):** `crates/core/src/consolidation/{episodes,fade,skills}.rs`, `crates/core/src/recall/{engine,recollection}.rs`, `crates/core/src/beliefs/credence.rs`, `crates/core/src/context.rs`, `crates/core/src/turn/{context_emit,sleep_and_beliefs}.rs`, `crates/reason/src/prompt/render.rs`.
  - *Clean boundary coordination:* Subagent 1 adds `canonical_audience_hash` helper + `actor_memory` + `PlaceAdaptState::recall_depth_bandit` + `ContextPlan::{compute_tier, repair_context_ids}`. Subagent 2 owns `EpisodeBook` multi-slot + `fade.rs` + `skills.rs` + `beliefs/credence.rs` + `recall/engine.rs` + `context.rs` + `context_emit.rs` + `render.rs`. Any thin glue between Track 1 and Track 2 (e.g., wiring `EpisodeBook` thread features into `features.rs` or `recall_depth_bandit` into `context_emit.rs`) is cleanly finalized by the Coordinator during integration merge!
- **Subagent 3 (`Wave C Training V2.2`):** Exclusively `training/` (`model.py`, `features.py`, `augment.py`, `datasets.py`, `train_multitask.py`, `posttrain_preference.py`, `export.py`, `licenses.py`, `tests/*`). Zero Rust edits!

## Risks / Trade-offs

- **Snapshot Compatibility:** Adding `actor_memory`, `thread_id`, `RecallDepthBanditState`, and `ProcedureLifecycleState::{Trial, Stable}` touches serialized Core state. Mitigation: bump `SNAPSHOT_VERSION` cleanly in `snapshot.rs` with backward-compatible upcast for older versions, and annotate all new struct fields with `#[serde(default)]`.
- **Schema v1 vs v2 Parity:** Existing Rust tests and `training/artifacts/phase3_mingru_150k_stage4.safetensors` use `schema = 1`. Mitigation: keep `schema = 1` (`18d`) 100% supported alongside `schema = 2` (`32d`), and verify both golden fixtures in `crates/core/tests/it/neural_parity.rs`.
- **Cross-Track Glue Conflicts:** Track 1 (`state/mod.rs`, `neural/features.rs`) reads Track 2 (`EpisodeBook` multi-slot, `BeliefBook`). Mitigation: keep Track 1 and Track 2 strictly disjoint on files, merge both branches in the Coordinator workspace, wire the 2–3 cross-module glue lines (reading `EpisodeBook` active thread stats in `features.rs` and `PlaceAdaptState::select_recall_depth` in `context_emit.rs`), and run the full workspace test suite + RTX 4060 retrain.
