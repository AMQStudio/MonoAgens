# Proposal

## Why

Training ad-hoc per-scenario hard negatives fixes individual failure cases (like casual slang chatter vs proactive questions), but cannot generalize across millions of unforeseen multi-party, multilingual, and shifting-topic conversations. Relying solely on raw absolute embedding cosine (`max_visible_mem_cos`), an un-wired `A-14` `InfoGapClassifier`, and a room-wide 4-scalar bandit bias leaves Neural System 1 blind to:
1. **Embedding baseline anisotropy** across different languages and room styles (e.g., casual Indonesian slang yields ~0.78–0.80 cosine even on unrelated chatter), where biological sensory systems respond to **relative prediction-error surprise** rather than raw magnitude.
2. **Epistemic dialogue-act information gaps (`A-14` `dialogue_act_gap`)** because `GatewayRuntime` passes `None` for `InfoGapClassifier`, `NeuralModel` does not bundle `(w_gap, b_gap)`, and System 1 numeric features (`x_num`) do not ingest `dialogue_act_gap`.
3. **Context-specific fast synaptic plasticity**: `PlaceAdaptState` currently stores only a 4-scalar `logit_offsets` per room rather than a three-factor Hebbian fast weight matrix (`EligibilityTrace x Neuromodulator`) conditioned on content-blind conversational context channels, so learning to stay quiet during banter would globally suppress helpful answers to direct/epistemic questions in the same room.
4. **Self-supervised conversational dynamics**: Training only on supervised gate labels limits representation generalization compared to a predictive-coding auxiliary objective over raw unlabelled conversation streams.

## What Changes

- **Schema v3 (`36d` `x_num`) Active-Inference & Homeostatic Surprise Channels** in `crates/core`:
  - Add per-place content-blind `PlaceCosBaseline` EWMA statistics (`SNAPSHOT_VERSION = 14`, backwards-compatible with `10..=13`) and expose `[32] dialogue_act_gap`, `[33] mem_cos_surprise`, `[34] topic_shift_surprise`, and `[35] unanswered_gap_pressure` strictly post-`VisibleScope` (`Delta_NI = 0`).
  - Keep backwards compatibility for `feature_schema_version = 1` (`18d`), `2` (`32d`), and neutral fallback (`model = None`).
- **Self-Contained `A-14 InfoGapClassifier` Auto-Wiring** across `crates/core`, `crates/gateway`, and `training/`:
  - Optional `head.dialogue_act_gap` tensors inside `safetensors` (`NeuralModel`), auto-wired by `GatewayRuntime` into `PreQueueService` (`t_clf`) and `Core::set_dialogue_act_head(...)`, degrading cleanly to the frozen neutral prior when `[embed]` is `none`.
- **Three-Factor Synaptic Fast-Weight Plasticity (`W_fast`)** inside `crates/core/src/neural/adapt.rs` & `crates/core/src/turn/`:
  - Bounded `4 x 8` associative synaptic matrix over compact content-blind context features, updated via event-sourced natural outcome neuromodulators (`Yielded`, peer follow-up engagement, and `p_user_correction` repair), subject to daily decay, `online_adaptation`, and `Taint::Untrusted` guards.
- **Self-Supervised Predictive World Model & Real Multilingual Speech-Act Training** in `training/`:
  - Optional `head.world_pred` (`[3, d_model]`) trained self-supervised on unlabelled turn transitions, joint `t_clf` `dialogue_act_gap` fitting, GPU training/export of `neural_v3_best.safetensors` (and/or updating active checkpoint), plus full offline & unit/integration verification.

## Capabilities

### New Capabilities
- `v3-synaptic-active-inference`: Domain-agnostic homeostatic cosine surprise normalization, auto-wired `A-14` `dialogue_act_gap` into `x_num`, three-factor synaptic fast-weight plasticity (`W_fast`), and self-supervised predictive world-model training.

### Modified Capabilities
- *(none)*

## Impact

- `crates/core/src/neural/`, `crates/core/src/state/`, `crates/core/src/turn/`, `crates/core/src/event.rs`
- `crates/gateway/src/runtime.rs`
- `training/src/monoagens_train/` & `training/tests/`
- `crates/lab/` offline checks
- Zero changes to `ARCHITECTURE.md` invariants, zero hardcoded language/keyword lists in Core, zero reads of `lab/scenarios/sealed/`, and zero unrequested paid live runs.
