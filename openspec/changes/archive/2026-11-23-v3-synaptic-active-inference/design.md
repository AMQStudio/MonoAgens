# Design: V3 Brain-Like Synaptic Plasticity & Active Inference

## Architectural Principles (`ARCHITECTURE.md` Alignment)

Every mechanism below operates strictly on opaque IDs, normalized vectors, timestamps, and calibrated probabilities inside `crates/core`—never inspecting raw bytes or hardcoding keyword lists—and degrades identically to deterministic neutral behavior when `model = None`, `[embed] = none`, or `online_adaptation = false`.

## 1. Feature Schema v3 (`36d` `x_num`, Backward-Compatible with v1/v2)

- `feature_schema_version = 3`, `DEFAULT_NUM_FEATURES_V3 = 36`.
- `forward_step_with_actor` slices `x_num`:
  - `18d` when loaded model declares `feature_schema_version == 1` (`d_in_num == 18`),
  - `32d` when loaded model declares `feature_schema_version == 2` (`d_in_num == 32`),
  - `36d` when loaded model declares `feature_schema_version == 3` (`d_in_num == 36`).
- New indices `32..36` (all clamped finite in `[0.0, 1.0]`, strictly post-`VisibleScope`):
  - **`x_num[32]` (`dialogue_act_gap`)**: Explicit `percept.dialogue_act_gap`, else `state.dialogue_act_head.score(&percept.embeddings)`, else `0.5`.
  - **`x_num[33]` (`mem_cos_surprise`)**: Relative predictive-coding surprise of `max_visible_mem_cos` normalized against per-place homeostatic `PlaceCosBaseline { mean, var, count }` (updated via bounded EWMA `alpha = 0.12` on every visible non-self percept with finite cosine):
    - When `count >= 3`: `sigmoid((raw_cos - mean) / sigma.max(0.08))`
    - When `count < 3`: `((raw_cos - 0.5).max(0.0) * 2.0).clamp(0.0, 1.0)`
  - **`x_num[34]` (`topic_shift_surprise`)**: `(1.0 - cos(e_t, last_visible_place_emb)).clamp(0.0, 1.0)` when both current and previous visible turn embeddings exist at `place`, else `0.5`.
  - **`x_num[35]` (`unanswered_gap_pressure`)**: Time-decayed residual epistemic gap stored per place (`PlaceUnansweredState`), accumulating `dialogue_act_gap` on non-soul turns and decaying exponentially with half-life `120s` (resetting toward `0` when the soul speaks or a low-gap peer response resolves an open turn).

## 2. Self-Contained `A-14 InfoGapClassifier` Wiring

- Optional `safetensors` tensors + metadata (`has_info_gap_head = "true"`):
  - `head.dialogue_act_gap.weight`: `[d_in_emb]` (`f32`)
  - `head.dialogue_act_gap.bias`: `[1]` (`f32`)
  - Optional metadata `info_gap_embed_model_ver`, `info_gap_prompt_regime_id` (defaulting to matching `t_clf`).
- `GatewayRuntime::open_with_options` inspects the loaded `NeuralModel.info_gap`; if present, populates both `PreQueueService::new(..., Some(info_gap_clf), ...)` and `core.set_dialogue_act_head(Some(head))`.
- If absent or if `[embed]` is `none`, `InfoGapClassifier` cleanly falls back to `sigmoid(b_gap)` and preserves exact existing behavior.

## 3. Three-Factor Synaptic Fast Weights (`W_fast`)

- Extend `PlaceAdaptState` (`SNAPSHOT_VERSION = 14`, reading `10..=13` with zero-init defaults):
  - `synaptic_weights: [[f32; 8]; 4]` (initialized to `0.0`), representing fast context-conditioned neuromodulated synapses for `[Silent, Note, Wait, Go]`.
  - `pending_traces: Vec<SynapticTrace>` (capped at 4 recent per-place turn traces `{ event_id, arm, features_8, timestamp_ms }`) recording the presynaptic context vector emitted on non-committed `Silent`, `Wait`, or `Go` decisions.
- Content-blind presynaptic vector `synaptic_features_from_x_num(x_num: &[f32]) -> [f32; 8]` (symmetric/bounded in `[-1.0, 1.0]`):
  - `[0]`: `1.0` (tonic excitability)
  - `[1]`: `(x_num[0].max(x_num[1]) * 2.0 - 1.0)` (directness)
  - `[2]`: `(x_num[3] * 2.0 - 1.0)` (dyadic vs group)
  - `[3]`: `((x_num.get(33).copied().unwrap_or(0.5)) * 2.0 - 1.0)` (memory cosine surprise)
  - `[4]`: `((x_num.get(32).copied().unwrap_or(0.5)) * 2.0 - 1.0)` (dialogue act gap)
  - `[5]`: `((x_num.get(34).copied().unwrap_or(0.5)) * 2.0 - 1.0)` (topic shift surprise)
  - `[6]`: `(x_num.get(19).copied().unwrap_or(0.0).max(x_num.get(22).copied().unwrap_or(0.0)) * 2.0 - 1.0)` (repair/reply link)
  - `[7]`: `(x_num.get(28).copied().unwrap_or(0.0) * 2.0 - 1.0)` (automated peer)
- Forward pass (`forward_step_with_actor`):
  - Adds `W_fast[arm] · features_8` (scaled and total delta clamped within `[-max_adapt_delta, +max_adapt_delta]`) alongside UCB `effective_logit_offsets` when `online_adaptation == true`.
  - `apply_time_decay` relaxes `synaptic_weights` by `gamma_f32` alongside `logit_offsets`.
- Event-sourced update via `EventDetail::AdaptFeedbackApplied` (extends optional `context_features: Option<[f32; 8]>` with snapshot backward compatibility):
  - Fires on:
    1. `OpenWait` `Yielded`: reinforces `Wait` (`+0.35`) and inhibits `Go` (`-0.25`) along the armed `Wait`'s recorded presynaptic trace.
    2. `p_user_correction >= 0.65` (Loop B): inhibits `Go` (`-reward`) and gently reinforces `Silent`/`Wait` along the most recent soul `Go` trace.
    3. Engaged peer follow-up (`reply_to_last_soul` with low `p_user_correction < 0.35`): reinforces `Go` (`+0.25`) along the prior `Go` trace.
  - Strictly enforces `source_taint == Taint::Untrusted` guard against poisoning.

## 4. Self-Supervised Predictive Coding & Multilingual `t_clf` Calibration

- Optional V3 predictive coding head (`has_v3_world_head = "true"`):
  - `head.world_pred.weight`: `[3, d_model]`, `head.world_pred.bias`: `[3]`
  - Trained on unlabelled sequence transitions to predict normalized next-turn `(peer_will_reply_next, next_turn_gap, next_turn_topic_shift)`, regularizing `h_t` and trunk features to model conversational dynamics rather than memorized gate templates.
- Contrastive/logistic `head.dialogue_act_gap` trained over real `t_clf` `EmbeddingGemma 2` vectors of diverse multilingual information-seeking turns vs non-requesting statements/reactions.
- Jointly train and evaluate schema-v3 controller on RTX 4060 GPU with temperature scaling + CIC calibration and Lab offline verification.
