## ADDED Requirements

### Requirement: Domain-agnostic schema v3 (`36d`) perception & homeostatic cosine surprise
`Core` SHALL support `feature_schema_version = 3` (`36d` `x_num` = `32d` v2 + `[dialogue_act_gap, mem_cos_surprise, topic_shift_surprise, unanswered_gap_pressure]`) strictly post-`VisibleScope` (`Delta_NI = 0`) via per-place EWMA cosine baselines and decayed gap state without inspecting raw text, preserving v1 (`18d`), v2 (`32d`), and neutral fallback (`model = None`) compatibility.

#### Scenario: High-baseline group chatter yields low homeostatic surprise while genuinely relevant turns yield high surprise
- **WHEN** a place exhibits consistently elevated background cosine similarity across unrelated visible chatter turns
- **THEN** `PlaceCosBaseline` adapts its running `(mean, variance)` smoothly so ambient chatter maps near neutral surprise (`~0.5`) in `x_num[33]`, while an anomalous higher-than-baseline match registers elevated `mem_cos_surprise`

### Requirement: Self-contained `A-14` `InfoGapClassifier` auto-wiring
`NeuralModel` SHALL support optional `head.dialogue_act_gap` (`w_gap`, `b_gap`) tensors in `safetensors`, and `GatewayRuntime` SHALL automatically wire that classifier into `PreQueueService` (`t_clf`) and `Core::set_dialogue_act_head(...)` whenever a loaded model includes it, degrading cleanly to `sigmoid(b_gap)` when `[embed]` is `none` or degraded.

#### Scenario: Gateway auto-wires `dialogue_act_gap` scorer from loaded `NeuralModel`
- **WHEN** `GatewayRuntime` opens with `[embed]` enabled and a `NeuralModel` containing `head.dialogue_act_gap`
- **THEN** `PreQueueService` populates `percept.dialogue_act_gap` via `t_clf` embeddings (or neutral prior on outage) and `Core` feeds that value into `x_num[32]`

### Requirement: Three-factor synaptic fast-weight plasticity (`W_fast`)
`PlaceAdaptState` SHALL maintain bounded associative fast weights `synaptic_weights: [[f32; 8]; 4]` updated via event-sourced `AdaptFeedbackApplied` records combining natural neuromodulators (`Yielded`, peer follow-up, `p_user_correction`) with presynaptic context traces, obeying daily decay, `SNAPSHOT_VERSION = 14` (`10..=13` compatible), `online_adaptation = false`, and `Taint::Untrusted` guards.

#### Scenario: Contextual inhibition reduces spam in banter without suppressing direct or high-gap requests
- **WHEN** negative feedback occurs after a low-directness, low-gap intervention while positive/neutral outcomes occur on high-gap or directly addressed turns in the same place
- **THEN** `synaptic_weights` depresses `Go` selectively along low-gap/unaddressed feature directions while leaving high-gap and directly addressed responsiveness intact

### Requirement: Self-supervised predictive coding world-model head & calibrated V3 training
The Python training pipeline SHALL support `feature_schema_version = 3` (`36d`), train the optional `head.world_pred` predictive-coding auxiliary head over turn transitions alongside multi-task + Stage-4 calibration and real `EmbeddingGemma 2` `t_clf` fitting of `head.dialogue_act_gap`, export Rust-parity `safetensors`, and pass Python unit tests, Rust workspace tests/clippy, and offline Lab comparison/smoke checks.

#### Scenario: V3 model passes Python-to-Rust numerical parity and offline E2E checks
- **WHEN** a schema-v3 `safetensors` checkpoint with `head.dialogue_act_gap` and `head.world_pred` is trained and loaded into `monoagens-core`
- **THEN** Rust `forward_step_with_actor` matches PyTorch logits within floating-point tolerance (`< 1e-4`), Lab offline ablations show zero regressions, and `monoagens-lab smoke-id` passes `3/3`
