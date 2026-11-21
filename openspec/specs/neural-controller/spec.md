# neural-controller Specification

## Purpose
Defines the Pure-Rust unified neural cognitive controller ("1 Neural Link") that provides calibrated, model-agnostic cognitive control decisions over frozen embeddings and numeric harness features with per-place working memory and bounded online adaptation.

## Requirements

### Requirement: Dimension-agnostic `safetensors` weight loading with integrity verification
The neural controller SHALL load model weights from a `safetensors` artifact, verify its `BLAKE3` digest when an expected digest is configured, and infer layer dimensions from the tensor metadata so the same runtime supports models from `~50k` up to `~3M` parameters without code changes.

#### Scenario: Valid weight file loads across parameter scales
- **WHEN** a valid `safetensors` artifact with `150k`, `500k`, `1M`, or `3M` parameters matching the declared schema metadata is provided
- **THEN** the controller loads the weights, computes and records its `BLAKE3` digest, and becomes active for inference

#### Scenario: Corrupted or mismatched weight file fails loudly
- **WHEN** a configured weight file fails `BLAKE3` verification, has missing required tensors, contains non-finite values, or has incompatible tensor shapes
- **THEN** loading returns an explicit error rather than silently running with corrupt weights

### Requirement: Safe neutral fallback when no model is loaded
When no neural weight file is configured or neural inference is disabled, the controller SHALL return a deterministic neutral fallback so the single cognitive loop continues running without alternate control-flow branches.

#### Scenario: Autonomous operation without neural weights
- **WHEN** the Core runs with neural model set to `None`
- **THEN** appraisal produces a deterministic neutral score, records a neutral model sentinel in `DecisionTrace`, and hard commitments still wake reasoning normally

### Requirement: Per-place recurrent working memory (`h_t`)
The neural controller SHALL maintain a fixed-width per-place recurrent working memory state vector (`h_t`, default `64` floats) updated deterministically on each appraised event via the artifact's configured recurrent cell (`minGRU` or decoupled erase/write `Gated DeltaNet-2`) and included in Core state snapshots.

#### Scenario: Working memory persists across snapshot and restart
- **WHEN** the Core checkpoints a snapshot and reopens from that snapshot plus trailing event log
- **THEN** each place's working memory vector `h_t` is restored identically and subsequent turns produce identical state transitions

#### Scenario: New place initializes cleanly
- **WHEN** the first event arrives for a previously unseen place
- **THEN** the controller initializes that place's working memory state `h_t` to the zero vector

### Requirement: Phased multi-head cognitive control outputs with implicit-ensemble uncertainty
The neural controller SHALL expose bounded, calibrated Phase 1 gate outputs (`gate_4` over `Silent`, `Note`, `Wait`, `Go` plus bounded `wait_delta_t` and implicit-ensemble epistemic spread when `k > 1`) and optional Phase 2 (`p_uncertainty`, `p_verify`) and Phase 3 (`p_memory_relevance`, `p_encode_importance`) heads, without parsing raw language or overriding Valve policy.

#### Scenario: Phase 1 artifact runs when optional Phase 2/3 heads are absent
- **WHEN** a Phase 1 `safetensors` artifact containing only `gate_4` and `wait_delta_t` tensors is loaded
- **THEN** Phase 1 outputs are evaluated normally and Phase 2/3 outputs cleanly return `None` (neutral)

#### Scenario: Implicit-ensemble disagreement exposes epistemic uncertainty
- **WHEN** a `TabM` artifact with `k > 1` implicit sub-models evaluates an ambiguous percept
- **THEN** the controller reports both the ensemble-mean probabilities and a bounded non-negative epistemic disagreement score

#### Scenario: Wait decision schedules bounded patience hold
- **WHEN** `gate_4` selects `Wait` with `wait_delta_t`
- **THEN** the harness clamps `wait_delta_t` to the configured `[0, max_wait_ms]` range and yields if another participant answers before the timer expires

### Requirement: Bounded per-place online adaptation with debug toggle
The neural controller SHALL support fast online adaptation via a frozen main backbone paired with a small bounded per-place adaptation state (Warm-Start UCB contextual bandit bias/FiLM offsets) updated from observed or delayed one-sided outcomes/receipts, guarded by an explicit `online_adaptation` boolean toggle.

#### Scenario: Adaptation stays strictly within safety bounds
- **WHEN** repeated positive or negative outcome feedback is applied to a place's adaptation vector
- **THEN** every adaptation parameter remains clamped within `[-max_adapt_delta, +max_adapt_delta]` and backbone weights remain immutable

#### Scenario: Debug toggle disables online adaptation cleanly
- **WHEN** `online_adaptation` is set to `false`
- **THEN** per-place adaptation vectors are neither applied nor mutated, isolating base-model behavior for debugging and evaluation
