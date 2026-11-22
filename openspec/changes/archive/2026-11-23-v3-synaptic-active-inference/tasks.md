## 1. Core Schema v3, Homeostatic Surprise, & Three-Factor Synaptic Plasticity

- [x] 1.1 Add `FEATURE_SCHEMA_VERSION_V3 = 3` (`36d`), `PlaceCosBaseline`, `PlaceUnansweredState`, and `extract_features_v3_from_state` (`[32] dialogue_act_gap`, `[33] mem_cos_surprise`, `[34] topic_shift_surprise`, `[35] unanswered_gap_pressure`) in `crates/core/src/neural/features.rs` & `crates/core/src/state/`, with `SNAPSHOT_VERSION = 14` backwards compatibility.
- [x] 1.2 Add optional `InfoGapHeadWeights` (`head.dialogue_act_gap`) and `V3WorldHeadWeights` (`head.world_pred`) to `crates/core/src/neural/weights.rs` and `Schema v3` backward-compatible slicing (`18d`/`32d`/`36d`) in `forward.rs`.
- [x] 1.3 Implement three-factor associative fast weights (`synaptic_weights`, `SynapticTrace`, `synaptic_features_from_x_num`) inside `crates/core/src/neural/adapt.rs`, wire context-aware event-sourced updates (`Yielded`, peer follow-up engagement, `p_user_correction`) in `crates/core/src/turn/`, and add `crates/core` unit tests covering parity, snapshot compat, and taint/decay guards.

## 2. Gateway Auto-Wiring of `A-14 InfoGapClassifier`

- [x] 2.1 Auto-construct `InfoGapClassifier` & `Core::set_dialogue_act_head` in `crates/gateway/src/runtime.rs` when the loaded `NeuralModel` carries `head.dialogue_act_gap` weights, plus an integration test verifying live enrichment & neutral fallback.

## 3. Training Pipeline, Predictive World Model, & Calibrated V3 Weights

- [x] 3.1 Update `training/src/monoagens_train/model.py`, `datasets.py`, and `train_multitask.py` for `feature_schema_version = 3` (`36d`), self-supervised `head.world_pred`, and joint `EmbeddingGemma 2` `t_clf` `head.dialogue_act_gap` training, plus `training/tests/` parity & schema-v3 tests.
- [x] 3.2 Train and calibrate the V3 controller into `training/artifacts/neural_v22_best.safetensors` / `neural_v3_best.safetensors` on local GPU and update `/home/muqsith/.config/monoagens/monoagens.toml` to point to the calibrated checkpoint.

## 4. End-to-End Verification

- [x] 4.1 Verify `pytest training/tests/`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, offline `monoagens-lab compare` (`development` & `validation`), `monoagens-lab smoke-id` (`3/3`), and `openspec validate v3-synaptic-active-inference --strict`.
