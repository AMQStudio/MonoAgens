# neural-controller Specification

## Purpose
Defines the Pure-Rust unified neural cognitive controller ("1 Neural Link") that provides calibrated, model-agnostic cognitive control decisions over frozen embeddings and numeric harness features with per-place working memory and bounded online adaptation.

## Requirements

### Requirement: Dimension-agnostic `safetensors` weight loading with integrity verification
The neural controller SHALL load `safetensors` weights (`~50k`–`~3M` params), verify `BLAKE3` when configured, support backward-compatible `feature_schema_version = 1` (`18d` `x_num`) and `feature_schema_version = 2` (`32d` `x_num`), parse optional embedding contract metadata (`expected_embed_model_ver`, `expected_prompt_regime_id`), and load optional V2.2 heads (`has_v22_heads`) and `h_actor` cell (`h_actor_dim > 0`) when declared.

#### Scenario: Valid weight file loads across parameter scales
- **WHEN** a valid `safetensors` artifact with `150k`, `500k`, `1M`, or `3M` parameters matching `feature_schema_version` `1` or `2` is provided
- **THEN** the controller loads the weights, computes and records its `BLAKE3` digest, parses any declared embedding contract, V2.2 head flags, and `h_actor_dim`, and becomes active for inference

#### Scenario: Corrupted or mismatched weight file fails loudly
- **WHEN** a configured weight file fails `BLAKE3` verification, has missing required tensors, contains non-finite values, or has incompatible tensor shapes
- **THEN** loading returns an explicit error rather than silently running with corrupt weights

#### Scenario: Schema v1 artifacts remain loadable alongside schema v2 artifacts
- **WHEN** a legacy `feature_schema_version = 1` (`18d` `x_num`, no V2.2 heads, `h_actor_dim = 0`) artifact is loaded on the V2.2 runtime
- **THEN** loading succeeds, inference slices the first `18` `x_num` channels cleanly, and V2.2 optional outputs return `None`

### Requirement: Safe neutral fallback when no model is loaded
When no neural weight file is configured or neural inference is disabled, the controller SHALL return a deterministic neutral fallback so the single cognitive loop continues running without alternate control-flow branches.

#### Scenario: Autonomous operation without neural weights
- **WHEN** the Core runs with neural model set to `None`
- **THEN** appraisal produces a deterministic neutral score, records a neutral model sentinel in `DecisionTrace`, and hard commitments still wake reasoning normally

### Requirement: Per-place recurrent working memory (`h_t`)
The neural controller SHALL maintain a fixed-width per-place recurrent working memory state vector (`h_t`, default `64` floats) updated deterministically on each appraised event AND on the soul's own completed replies / `SelfCaused` percepts (without waking reasoning) via the artifact's configured recurrent cell (`minGRU` or decoupled erase/write `Gated DeltaNet-2`), and included in Core state snapshots.

#### Scenario: Working memory persists across snapshot and restart
- **WHEN** the Core checkpoints a snapshot and reopens from that snapshot plus trailing event log
- **THEN** each place's working memory vector `h_t` is restored identically and subsequent turns produce identical state transitions

#### Scenario: New place initializes cleanly
- **WHEN** the first event arrives for a previously unseen place
- **THEN** the controller initializes that place's working memory state `h_t` to the zero vector

#### Scenario: Soul reply advances place working memory without waking
- **WHEN** a `SelfCaused` percept or completed soul communicative action is applied to a place while a neural model is loaded
- **THEN** `place_memory[place]` (`h_t`) advances deterministically using the reply's embedding (or zero vector if vectorless) without triggering a reasoning wake, and replay restores the exact updated `h_t`

### Requirement: Phased multi-head cognitive control outputs with implicit-ensemble uncertainty
The neural controller SHALL expose Phase 1 (`gate_4`, `wait_delta_t`, `cic_abstain`, `epistemic_uncertainty`), optional Phase 2 (`p_uncertainty`, `p_verify`), optional Phase 3 (`p_memory_relevance`, `p_encode_importance`), and optional V2.2 (`compute_tier` over `[Fast, Standard, Deep]` with `p_verify`/uncertainty escalation floors → `ContextPlan.compute_tier` → `ModelRequest.reasoning_effort`, and `p_user_correction` driving targeted repair hints and `Taint`-guarded delayed negative `Go` feedback).

#### Scenario: Phase 1 artifact runs when optional Phase 2/3 heads are absent
- **WHEN** a Phase 1 `safetensors` artifact containing only `gate_4` and `wait_delta_t` tensors is loaded
- **THEN** Phase 1 outputs are evaluated normally and Phase 2, Phase 3, and V2.2 optional outputs cleanly return `None` (neutral)

#### Scenario: Implicit-ensemble disagreement exposes epistemic uncertainty
- **WHEN** a `TabM` artifact with `k > 1` implicit sub-models evaluates an ambiguous percept
- **THEN** the controller reports both the ensemble-mean probabilities and a bounded non-negative epistemic disagreement score

#### Scenario: Wait decision schedules bounded patience hold
- **WHEN** `gate_4` selects `Wait` with `wait_delta_t`
- **THEN** the harness clamps `wait_delta_t` to the configured `[0, max_wait_ms]` range and yields if another participant answers before the timer expires

#### Scenario: Phase 2 verification hints flow into wake context metadata
- **WHEN** a loaded model emits `p_verify` and `p_uncertainty` on a `Go` wake
- **THEN** Core propagates those bounded scores into the wake's `ContextPlan` metadata (and `None` under neutral fallback) so downstream reasoning/verification gates can observe epistemic doubt without altering Valve rules

#### Scenario: V2.2 compute_tier routes per-request reasoning effort with epistemic escalation floor
- **WHEN** a V2.2 model emits `compute_tier` (`Fast`, `Standard`, or `Deep`) on a `Go` wake alongside `p_verify` or `epistemic_uncertainty`
- **THEN** Core applies the deterministic uncertainty/`p_verify` escalation floor, attaches `compute_tier` to `ContextPlan`, and Gateway/Reason maps it to per-request `ModelRequest.reasoning_effort` (`Low`, `Medium`, `High`) while neutral fallback omits the override

#### Scenario: V2.2 p_user_correction emits targeted repair metadata and taint-guarded delayed negative Go feedback
- **WHEN** a V2.2 model emits high calibrated `p_user_correction` following a recent visible soul reply
- **THEN** Core attaches `[last_soul_event_id, prior_user_event_id]` repair metadata to `ContextPlan` when waking and emits an event-sourced propensity-weighted delayed negative `AdaptFeedbackApplied` on `GateArm::Go` strictly subject to `Taint` guards

### Requirement: Bounded per-place online adaptation with debug toggle
The neural controller SHALL support fast online adaptation via a frozen backbone paired with bounded per-place `PlaceAdaptState` (Warm-Start UCB gate equalizer, FiLM offsets, and a `MemCon`-style discrete recall-depth bandit over `[Skip, Light, Standard, DeepPlusProc]`), updated strictly via event-sourced outcomes, guarded by `online_adaptation` and `Taint::Untrusted` poisoning blocks, and relaxed deterministically over event timestamps (`last_updated_ms`, `adapt_daily_lambda`).

#### Scenario: Adaptation stays strictly within safety bounds
- **WHEN** repeated positive or negative outcome feedback is applied to a place's adaptation vector with non-zero bounded `context_hint`
- **THEN** every logit offset and FiLM scale/shift delta remains clamped within `[-max_adapt_delta, +max_adapt_delta]`, FiLM scale deltas move non-trivially from `0.0`, and backbone weights remain immutable

#### Scenario: Debug toggle disables online adaptation cleanly
- **WHEN** `online_adaptation` is set to `false`
- **THEN** per-place adaptation vectors and recall-depth bandit weights are neither applied nor mutated, isolating base-model and fixed-recall behavior for debugging and evaluation

#### Scenario: Untrusted percepts cannot poison online adaptation logits
- **WHEN** a negative or delayed feedback signal originates strictly from a `Taint::Untrusted` source without trusted corroboration
- **THEN** Core skips or blocks mutating `PlaceAdaptState` so adversarial untrusted messages cannot push a place's `Go` logit to `-max_adapt_delta`

#### Scenario: Time-discounted adaptation decays deterministically across idle days
- **WHEN** days elapse between event timestamps in a place with prior adaptation offsets
- **THEN** `PlaceAdaptState` relaxes its pull counts, reward sums, logit offsets, FiLM deltas, and recall-depth stats using deterministic exponential decay over `now - last_updated_ms` before applying new updates or evaluating effective offsets

#### Scenario: MemCon recall-depth bandit selects bounded depth and degrades neutrally
- **WHEN** `online_adaptation` is enabled and a place receives wake recall-depth outcomes
- **THEN** `PlaceAdaptState` selects a discrete recall depth (`Skip`, `Light`, `Standard`, or `DeepPlusProc`) via deterministic Warm-Start UCB, while `model = None` or `online_adaptation = false` preserves the fixed `CoreConfig::max_recalled_memories` default

### Requirement: Factorized per-participant working state (`h_actor`) with strict audience scoping
When a loaded V2.2 model declares `h_actor_dim > 0`, the neural controller and Core state SHALL maintain a compact recurrent per-participant working vector (`h_actor`, `24d`–`32d`) keyed strictly by `(PlaceId, CanonicalAudienceHash, ActorId)` (`Delta_NI = 0`), bounded to `K_ACTORS = 32` entries per `(PlaceId, CanonicalAudienceHash)` via deterministic `(last_active_ms, ActorId)` LRU eviction in `BTreeMap`, restored identically via snapshot/replay, and erased completely on `EventKind::HardForget`.

#### Scenario: Private DM actor state never leaks into group chat (`Delta_NI = 0`)
- **WHEN** an actor interacts in a private DM and subsequently posts in a multi-party group room
- **THEN** the `(PlaceId, CanonicalAudienceHash, ActorId)` keys are disjoint and the group room `h_actor` initializes independently at zero without any cross-audience influence

#### Scenario: Per-room LRU bound evicts oldest actor deterministically
- **WHEN** more than `32` distinct actors post inside the same `(PlaceId, CanonicalAudienceHash)` scope
- **THEN** Core evicts the actor with the smallest `(last_active_ms, ActorId)` key deterministically so `actor_memory` never exceeds `32` entries per scope

#### Scenario: HardForget wipes actor working state with zero residual traces
- **WHEN** `EventKind::HardForget` targets an actor or place events
- **THEN** Core removes all matching `actor_memory` entries immediately and `HardForgetAudit` verifies zero remaining traces for erased actors
