# Proposal

## Why

Deep research Runs 6 (`R-65`–`R-67`) and 7 (`R-68`–`R-71`) validated MonoAgens Slim Core V2 against November 2026 autonomous chat-agent literature while uncovering seven concrete, high-impact bugs and blind spots (`C-1`–`C-4`, `C-8`–`C-10`) already present in `crates/core` and `training/`, alongside an unmeasured `$0` Indonesian embedder baseline (`IndoCollex` / `MultiLexNorm`) and unimplemented offline preference/counterfactual post-training (`E-70.1`) needed to cure `dev-07-habituation` (`0/5`) and `dev-08-patience` (`4/5`). Fixing these P0 correctness, privacy, replay, and calibration gaps now establishes a rock-solid foundation before adding Wave B governance (`approval/resolve`) or Wave C `feature_schema_version = 2` neural heads.

## What Changes

- **`C-1` — Train/Serve `EmbeddingGemma 2` `TaskPrefix` Parity:** Align `training/src/monoagens_train/datasets.py` shard precomputation with runtime `TaskPrefix::format_text` (`crates/reason/src/embed/mod.rs`) and export `expected_embed_model_ver` + `expected_prompt_regime_id` in `.safetensors` metadata so train/serve embedding geometry matches on short colloquial and Indonesian turns.
- **`C-2` — Multi-Capability Procedure Schema Matching Bugfix:** Replace the broken per-capability hash loop in `crates/core/src/context.rs` (`select_tools_for_context`) with canonical combined-digest verification (`schema_matches_catalog` / `compute_procedure_schema_hash`) so multi-capability procedures are no longer silently dropped as stale.
- **`C-3` — Gist Centroids, `SkillBook` Hard-Forget & `Audience`, Event-Sourced `place_adapt`, and Non-Zero FiLM `context_hint`:**
  - Attach the episode's normalized `centroid` (`EpisodeRecord.centroid`) as the immediate `TaggedEmbedding` on consolidated gists in `crates/core/src/turn/step.rs` so gists are visible to vector-partitioned recall.
  - Add `SkillBook::forget(erased_events, erased_actors)` called from `CoreState::apply_event(HardForget)` so event-level `hard_forget` scrubs derived lessons/procedures cleanly, and add `audience: Audience` to `LessonEntry` gated by `VisibleScope`.
  - Event-source `PlaceAdaptState` updates through `CoreState::apply_event` (additive `EventDetail::AdaptFeedbackApplied`) so log replay reconstructs `place_adapt` deterministically, and pass a real bounded 2-scalar numeric `context_hint` instead of `[0.0, 0.0]` so FiLM scale adaptation is live.
- **`C-4` — Non-Vacuous Clopper-Pearson `cic_threshold`, Taint-Guarded `PlaceAdaptState`, & `p_verify` Wiring:**
  - Fix `fit_cic_threshold` in `training/src/monoagens_train/calibrate.py` (`target_risk = 0.05`, candidate floor `> 0.25`) so `cic_threshold` is non-vacuous on 4-class softmax (`max_prob >= 0.25` is currently always true).
  - Guard `PlaceAdaptState` against `Taint::Untrusted` logit poisoning (preventing adversarial strangers in public rooms from driving `Go` logit to `-0.5`).
  - Wire Phase 2 `p_verify` / `p_uncertainty` outputs into Core's context assembly / verification hint metadata (strictly neutral `None` when no model or Phase 1 weights are loaded).
- **`C-8` — Fix Partial `EpisodeBook::forget` Centroid Leak:** Clear or recompute `EpisodeRecord.centroid` inside `EpisodeBook::forget` whenever any `target_event_ids` are removed from a surviving partial episode, eliminating residual float traces after `hard_forget`.
- **`C-9` — Fix `Gated DeltaNet-2` `h_t` Blind Spot on Soul's Own Replies:** Advance `place_memory` (`h_t`) deterministically on Soul's own completed replies / `SelfCaused` percepts (using the reply's `TaggedEmbedding` when available or zero vector, without triggering a wake) so recurrent working memory tracks what the Soul itself just said.
- **`C-10` — Fix Premature `Go` `+0.5` Transport Reward & Add Deterministic Time Decay to `PlaceAdaptState`:**
  - Stop rewarding `GateArm::Go` `+0.5` unconditionally on `ReceiptOutcome::Done` (transport delivery ≠ social utility); keep fault negative feedback (`Failed`/`Refused` excluding `PreemptedExternal`/`CanceledByCore`) and reserve positive/negative social feedback for response/delayed-uptake outcomes.
  - Add timestamp-driven exponential relaxation (`last_updated_ms`, `adapt_daily_lambda`) to `PlaceAdaptState` using event timestamps (`now`) so room habits smoothly adapt across weeks/months while preserving bit-exact deterministic replay.
- **`E-70.1` — Offline Stage 4 Preference & Counterfactual Post-Training (`$0` API, RTX 4060):** Add closed-form categorical pairwise DPO/margin + asymmetric KTO (`lambda_u > lambda_d`) + `CFPO` counterfactual `(e_256, 0_256)` invariance post-training in `training/`, retrain Phase 3 on RTX 4060 (`+0` runtime params, `+0 µs` latency), verify Rust↔Python parity (`<= 1e-5`), and re-run `--release` 5-seed offline Lab suites/ablations targeting `dev-07` & `dev-08` recovery with zero `validation` regression.
- **Indonesian `$0` Offline Embedder Probe Baseline:** Run the first local scored baseline of `EmbeddingGemma 2` on `IndoCollex` (`MIT`) and `MultiLexNorm` (`ind` + `ind_eng`, offline diagnostic only) across MRL dimensions (`768d / 512d / 256d / 128d`) and task-prefix regimes (`t_q/t_d`, `t_sts`, unprefixed), recording measured numbers in `research/findings/`.
- **Reproducible Offline Markdown Export of Learned State (`monoagens export-learning`):** Add a zero-API CLI subcommand (`monoagens export-learning --out <path.md> [--data-dir <dir>] [--log-json <path>] [--place <place_id>] [--include-private] [--json]`) that deterministically replays `CoreState` and exports `PlaceAdaptState`, `SkillBook` procedures & lessons, `BeliefBook`, and `EpisodeBook` gists into a canonical, privacy-safe, `hard_forget`-clean Markdown report with `BLAKE3` audit digest.

## Capabilities

### New Capabilities

### Modified Capabilities
- `slim-core-harness`: Fix multi-capability procedure schema matching (`C-2`), gist centroid embedding and `SkillBook` event-level `hard_forget` / `LessonEntry` `Audience` (`C-3`), partial `EpisodeBook::forget` centroid scrubbing (`C-8`), event-sourced `place_adapt` transitions (`C-3`), remove premature `Go` transport reward (`C-10`), and add reproducible Markdown export of learned state (`monoagens export-learning`).
- `neural-controller`: Enforce `expected_embed_model_ver` / `expected_prompt_regime_id` metadata contract (`C-1`), step `place_memory` `h_t` on Soul's own replies (`C-9`), wire non-zero FiLM `context_hint` (`C-3`), taint-guard and time-discount `PlaceAdaptState` (`C-4`, `C-10`), and consume calibrated `cic_threshold` and `p_verify` verification hints (`C-4`).
- `neural-training-pipeline`: Enforce `TaskPrefix` train/serve parity in embedding shard precomputation (`C-1`), non-vacuous Clopper-Pearson `CIC` threshold calibration (`C-4`), Stage 4 offline DPO/KTO + `CFPO` counterfactual post-training (`E-70.1`), and local `$0` Indonesian MRL/prefix embedder probe benchmarking (`IndoCollex` / `MultiLexNorm`).

## Impact

- **Core (`crates/core`):** `context.rs`, `consolidation/episodes.rs`, `consolidation/skills.rs`, `neural/adapt.rs`, `neural/mod.rs`, `neural/weights.rs`, `neural/forward.rs`, `gate/decide.rs`, `state/mod.rs`, `state/snapshot.rs`, `event.rs`, `ports.rs`, `turn/step.rs`, `turn/sleep_and_beliefs.rs`, `turn/context_emit.rs`, plus unit/integration/parity/hard-forget tests.
- **Training (`training/`):** `datasets.py`, `model.py`, `calibrate.py`, `train_multitask.py`, new `posttrain_preference.py` (or Stage 4 module), `tests/`, regenerated Phase 3 golden fixtures & `training/artifacts/neural_phase3_best.safetensors` / `manifest_phase3.json`.
- **Lab & Findings (`crates/lab`, `research/findings/`):** Offline `--release` 5-seed test drive & ablation re-verification, plus local `$0` `IndoCollex` / `MultiLexNorm` MRL/prefix probe measurement script/runner and findings report.
- **Invariants & Scope Guardrails:** Zero `monoagens_protocol` breaking changes, zero `ARCHITECTURE.md` edits, `+0` net Core primitives, zero raw text or word lists inside `crates/core`, zero access to `lab/scenarios/sealed/`, and **`0` paid LLM API calls**.
