# neural-controller Specification

## Purpose
Defines the Pure-Rust unified neural cognitive controller ("1 Neural Link") that provides calibrated, model-agnostic cognitive control decisions over frozen embeddings and numeric harness features with per-place working memory and bounded online adaptation.

## Requirements

### Requirement: Dimension-agnostic `safetensors` weight loading with integrity verification
The neural controller SHALL load model weights from a `safetensors` artifact, verify its `BLAKE3` digest when an expected digest is configured, infer layer dimensions from the tensor metadata so the same runtime supports models from `~50k` up to `~3M` parameters without code changes, and parse optional embedding contract metadata (`expected_embed_model_ver`, `expected_prompt_regime_id`) so train/serve embedding mismatches are surfaced deterministically.

#### Scenario: Valid weight file loads across parameter scales
- **WHEN** a valid `safetensors` artifact with `150k`, `500k`, `1M`, or `3M` parameters matching the declared schema metadata is provided
- **THEN** the controller loads the weights, computes and records its `BLAKE3` digest, parses any declared `expected_embed_model_ver` and `expected_prompt_regime_id`, and becomes active for inference

#### Scenario: Corrupted or mismatched weight file fails loudly
- **WHEN** a configured weight file fails `BLAKE3` verification, has missing required tensors, contains non-finite values, or has incompatible tensor shapes
- **THEN** loading returns an explicit error rather than silently running with corrupt weights

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
The neural controller SHALL expose bounded, calibrated Phase 1 gate outputs (`gate_4` over `Silent`, `Note`, `Wait`, `Go`, bounded `wait_delta_t`, calibrated non-vacuous `cic_abstain`, and implicit-ensemble epistemic spread when `k > 1`) and optional Phase 2 (`p_uncertainty`, `p_verify`) and Phase 3 (`p_memory_relevance`, `p_encode_importance`) heads, propagating `p_verify` and `p_uncertainty` hints into Core context assembly when present, without parsing raw language or overriding Valve policy.

#### Scenario: Phase 1 artifact runs when optional Phase 2/3 heads are absent
- **WHEN** a Phase 1 `safetensors` artifact containing only `gate_4` and `wait_delta_t` tensors is loaded
- **THEN** Phase 1 outputs are evaluated normally and Phase 2/3 outputs cleanly return `None` (neutral)

#### Scenario: Implicit-ensemble disagreement exposes epistemic uncertainty
- **WHEN** a `TabM` artifact with `k > 1` implicit sub-models evaluates an ambiguous percept
- **THEN** the controller reports both the ensemble-mean probabilities and a bounded non-negative epistemic disagreement score

#### Scenario: Wait decision schedules bounded patience hold
- **WHEN** `gate_4` selects `Wait` with `wait_delta_t`
- **THEN** the harness clamps `wait_delta_t` to the configured `[0, max_wait_ms]` range and yields if another participant answers before the timer expires

#### Scenario: Phase 2 verification hints flow into wake context metadata
- **WHEN** a loaded model emits `p_verify` and `p_uncertainty` on a `Go` wake
- **THEN** Core propagates those bounded scores into the wake's `ContextPlan` metadata (and `None` under neutral fallback) so downstream reasoning/verification gates can observe epistemic doubt without altering Valve rules

### Requirement: Bounded per-place online adaptation with debug toggle
The neural controller SHALL support fast online adaptation via a frozen main backbone paired with a small bounded per-place adaptation state (Warm-Start UCB contextual bandit bias and non-zero-hint FiLM offsets) updated via event-sourced outcomes/receipts, guarded by an explicit `online_adaptation` boolean toggle, protected against `Taint::Untrusted` poisoning, and relaxed deterministically over event timestamps (`last_updated_ms`, `adapt_daily_lambda`) so room habits adapt across weeks/months.

#### Scenario: Adaptation stays strictly within safety bounds
- **WHEN** repeated positive or negative outcome feedback is applied to a place's adaptation vector with non-zero bounded `context_hint`
- **THEN** every logit offset and FiLM scale/shift delta remains clamped within `[-max_adapt_delta, +max_adapt_delta]`, FiLM scale deltas move non-trivially from `0.0`, and backbone weights remain immutable

#### Scenario: Debug toggle disables online adaptation cleanly
- **WHEN** `online_adaptation` is set to `false`
- **THEN** per-place adaptation vectors are neither applied nor mutated, isolating base-model behavior for debugging and evaluation

#### Scenario: Untrusted percepts cannot poison online adaptation logits
- **WHEN** a negative or delayed feedback signal originates strictly from a `Taint::Untrusted` source without trusted corroboration
- **THEN** Core skips or blocks mutating `PlaceAdaptState` so adversarial untrusted messages cannot push a place's `Go` logit to `-max_adapt_delta`

#### Scenario: Time-discounted adaptation decays deterministically across idle days
- **WHEN** days elapse between event timestamps in a place with prior adaptation offsets
- **THEN** `PlaceAdaptState` relaxes its pull counts, reward sums, logit offsets, and FiLM deltas using deterministic exponential decay over `now - last_updated_ms` before applying new updates or evaluating effective offsets
