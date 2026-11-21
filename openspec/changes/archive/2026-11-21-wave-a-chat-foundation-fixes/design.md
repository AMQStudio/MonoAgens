# Design — Wave A: Chat Foundation Bugfixes, Stage 4 Preference Post-Training & `$0` Indonesian Probe Baseline

## Context

See `proposal.md` for motivation and scope. Wave A resolves the seven concrete P0 bugs and structural blind spots (`C-1`–`C-4`, `C-8`–`C-10`) uncovered during Runs 6–7 (`research/findings/2026-11-21-run-6.md` and `2026-11-21-run-7.md`), executes offline Stage 4 preference/counterfactual post-training (`E-70.1`) on the local RTX 4060 Laptop GPU (8 GB VRAM), and captures the first measured `$0` local baseline on Indonesian colloquial/code-mixed embedder probes (`IndoCollex` and `MultiLexNorm`).

All work must honor `ARCHITECTURE.md` invariants:
- Core sees only opaque IDs, numbers, timestamps, `Taint`, `Audience`, and `TaggedEmbedding` floats (zero raw text, zero word lists, zero simulated emotion).
- Mandatory `None` neutral fallback and single cognitive loop preserved.
- Zero breaking changes to `monoagens_protocol` (the 6 facts) and `+0` net Core primitives.
- `lab/scenarios/sealed/` is never read, listed, or mounted.
- Zero paid live LLM API calls.

## Goals / Non-Goals

**Goals:**
- Eliminate train/serve `EmbeddingGemma 2` `TaskPrefix` manifold shift (`C-1`) and verify it empirically on Indonesian slang/code-mixing (`IndoCollex` / `MultiLexNorm`).
- Fix every verified Core correctness, privacy, and replay gap (`C-2`, `C-3`, `C-4`, `C-8`, `C-9`, `C-10`) with targeted unit, property, parity, and `hard_forget` tests.
- Add Stage 4 offline preference + counterfactual post-training (`DPO/Margin + KTO + CFPO`) and non-vacuous Clopper–Pearson `CIC` calibration (`target_risk = 0.05`, `tau > 0.25`) to `training/`, retraining the `1M + Gated DeltaNet-2` artifact on RTX 4060 and verifying `--release` 5-seed Lab parity/improvement.

**Non-Goals:**
- Wiring `approval/resolve` / `I-HITL` or adding `dev-09..13` / `val-03` (deferred cleanly to Wave B).
- Bumping `feature_schema_version` (`1 -> 2`) or adding `head.compute_tier` / `head.p_user_correction` / `h_actor` / multi-slot `EpisodeBook` (deferred cleanly to Wave C).
- Ingesting new external SEA corpora (`SEADialogues`, `IndoDialect`, `PingPong`) into `datasets.manifest.toml` (Wave C).
- Running any paid live LLM API benchmarks (Wave D, requires separate owner sign-off).

## Decisions

### 1. Train/Serve `TaskPrefix` Parity (`C-1`)
- **Decision:** Standardize training shard precomputation in `training/src/monoagens_train/datasets.py` to prepend the canonical `TaskPrefix::Document` instruction string (`"title: none | text: "`, `regime_id = "t_d"`)—matching `crates/reason/src/embed/mod.rs` for stored event percepts—configurable via a CLI/env parameter (`prompt_regime_id = "t_d"` default) and exported into `.safetensors` `__metadata__` as `expected_embed_model_ver = "egemma2-256"` and `expected_prompt_regime_id = "t_d"`.
- **Rust Loader Behavior (`crates/core/src/neural/weights.rs`):** Parse `expected_embed_model_ver` and `expected_prompt_regime_id` as optional metadata fields on `NeuralModelConfig` (`#[serde(default)]`) so legacy v1 test fixtures without those metadata keys still load cleanly, while `monoagens-lab controller --json` surfaces them for inspection and `extract_embedding_for_model` (`features.rs`) prefers embeddings matching the declared `(embed_model_ver, prompt_regime_id)`.
- **Alternative Considered:** Removing `TaskPrefix` from runtime `embeddinggemma2.rs`. Rejected because Google's `EmbeddingGemma 2` model card (`arXiv:2610.37364`) explicitly requires task prefixes for optimal retrieval/classification geometry.

### 2. Multi-Capability Procedure Schema Matching (`C-2`)
- **Decision:** Replace the per-capability equality loop in `crates/core/src/context.rs` (`select_tools_for_context`) with a helper calling `compute_procedure_schema_hash` (from `crates/core/src/consolidation/skills.rs`) over `proc_item.required_cap_ids` mapped through `active_cap_hashes`, verifying every required capability is present and active, and comparing the resulting canonical digest against `proc_item.capability_schema_hash`.
- **Rationale:** Guarantees 1:1 consistency between extraction (`skills.rs:757`) and context selection (`context.rs:365`) for both 1-cap and multi-cap procedures.

### 3. Gist Centroid Embeddings, `SkillBook::forget`, `LessonEntry` Audience, & Partial `EpisodeBook::forget` (`C-3`, `C-8`)
- **Decision A (Gist Centroids):** When `crates/core/src/turn/step.rs` builds a gist `MemoryEntry` for `matched_ep_id`, copy `ep.centroid.clone().into_iter().collect()` into `entry.embeddings` instead of `Vec::new()`.
- **Decision B (`EpisodeBook::forget`, `C-8`):** Inside `crates/core/src/consolidation/episodes.rs::forget`, whenever `ep.event_ids` shrinks (`ep.event_ids.len() < before_len`) on a surviving partial episode (`!ep.event_ids.is_empty()`), clear `ep.centroid = None` (or recompute strictly from surviving non-erased `memory_entries` passed in from `CoreState::apply_event(HardForget)`). Clearing and recomputing from surviving entries ensures zero residual float traces of erased events remain in `ep.centroid`.
- **Decision C (`SkillBook::forget` & `LessonEntry.audience`, `C-3`):**
  - Add `#[serde(default)] pub audience: Audience` to `LessonEntry` (`skills.rs:82`), populated from the contributing episodes' `Audience` (strictly preserving privacy; never widening).
  - Add `SkillBook::forget(&mut self, erased_events: &[EventId], erased_actors: &[ActorId])` removing any failure/success episode, lesson (`lesson_id`, `failure_ids`, `contrastive_success_ids`, `actors`), or procedure (`procedure_id`, `source_goal_id`, `source_event_ids`, `actors`, `audience`) that cites an erased event or actor, and invoke it in `CoreState::apply_event(HardForget)` (`state/mod.rs:1546`).
  - Gate any lesson retrieval/exposure through `VisibleScope::allows(&lesson.audience, Some(lesson.lesson_id), now)`.

### 4. Event-Sourced, Non-Zero-Hint, Taint-Guarded, Time-Discounted `PlaceAdaptState` (`C-3`, `C-4`, `C-10`)
- **Decision A (Event-Sourced Projection):** Add an additive `EventDetail::AdaptFeedbackApplied { place: PlaceId, arm: GateArm, reward: f32, is_delayed_one_sided: bool, context_hint: [f32; 2], source_taint: Taint }` variant (on `EventKind::Verdict` or dedicated internal `EventDetail`) appended via `append_and_apply` and applied strictly inside `CoreState::apply_event`. This satisfies `ARCHITECTURE.md:237` ("state is a projection of the log and can be rebuilt from it").
- **Decision B (Non-Zero `context_hint`):** Compute a deterministic 2-scalar bounded numeric hint in `[-1.0, 1.0]` from the place's current observable state (e.g., `[dyadic_vs_group_signed, recent_soul_share_centered]`, derived solely from `place_participants` and `talk_turns`) instead of `[0.0, 0.0]`, bringing `film_scale_deltas` to life without raw text.
- **Decision C (Taint Guard, `C-4`):** Reject or ignore `AdaptFeedbackApplied` mutations when `source_taint == Taint::Untrusted` and the feedback is uncorroborated negative/delayed social feedback (while still allowing trusted transport/core/owner outcomes), preventing untrusted strangers in public rooms from driving `Go` logit to `-0.5`.
- **Decision D (Time Decay, `C-10`):** Add `#[serde(default)] pub last_updated_ms: Option<Time>` to `PlaceAdaptState` and `#[serde(default = "default_adapt_daily_lambda")] pub adapt_daily_lambda: f32` (default `0.90`, `1.0` disables decay) to `AdaptConfig`. Store effective pull counts cleanly (`arm_pulls_decayed: [f64; 4]` or relax `logit_offsets`, `film_scale_deltas`, `film_shift_deltas`, `arm_reward_sums`, and `arm_pulls` via `gamma = lambda.powf(dt_days)` using event timestamp `now`) before updates and effective-offset reads. Maintain backward-compatible serde defaults and bump `SNAPSHOT_VERSION` (`10 -> 11`) with clean v10 upcast if snapshot binary/struct layout changes.
- **Decision E (Remove Premature `Go` `+0.5` Reward, `C-10`):** In `crates/core/src/turn/step.rs:790-793`, stop calling `record_place_adapt_feedback(..., GateArm::Go, 0.5, false)` immediately on `ReceiptOutcome::Done`; retain the `-0.5` negative feedback on non-fault `Failed`/`Refused`, and credit `GateArm::Go` positively only on genuine response/engagement resolution (or keep neutral on transport delivery).

### 5. `Gated DeltaNet-2` `h_t` Stepping on Soul's Own Replies (`C-9`)
- **Decision:** When a `SelfCaused` percept is processed (`step.rs` / `state/mod.rs:624-629`) and a `NeuralModel` is loaded, run `forward_step(Some(model), &e_256, &x_num, &h_prev, None)` strictly to compute `h_next` (never waking reasoning and keeping `outcome = Silent, reason = SelfCaused`) and persist `h_next` in `DecisionTrace.neural.h_next` so `CoreState::apply_event` (`state/mod.rs:668-680`) updates `place_memory[place]` identically in live runs and replay.
- **Rationale:** Zero extra event kinds needed; reuses the exact existing `DecisionTrace` trace-pinned replay contract (`ARCHITECTURE.md:237`) while giving `Gated DeltaNet-2` full visibility into Soul's outgoing turns.

### 6. Non-Vacuous `CIC` Threshold & `p_verify` Context Propagation (`C-4`)
- **Decision A (`calibrate.py`):** Update `fit_cic_threshold` defaults to `target_risk = 0.05` (`delta = 0.05`) and candidate search grid `np.linspace(0.30, 0.98, 69)` (strictly above the 4-class simplex floor `0.25`), verifying `cic_threshold > 0.25` in unit tests and manifest output. Ensure that in `crates/core/src/gate/decide.rs`, `CommitmentInputs` (`turn_allocated`, `fires_or_answers_expectation`, `continues_active_chain`) always remain strictly ahead of `cic_abstain` so legitimate owner turns or due commitments are never suppressed.
- **Decision B (`p_verify` & `p_uncertainty` Wiring):** Add `#[serde(default)] pub neural_p_verify: Option<f32>` and `#[serde(default)] pub neural_p_uncertainty: Option<f32>` to `ContextPlan` (`crates/core/src/ports.rs`) and populate them from the trigger decision's `NeuralOutputs` inside `context_emit.rs` (`None` under neutral fallback or Phase 1 weights). Surface them as structured read-only verification hints in context assembly without adding word lists or weakening Valve rules.

### 7. Stage 4 Offline Preference & Counterfactual Post-Training (`E-70.1`)
- **Decision:** Create `training/src/monoagens_train/posttrain_preference.py` (invoked optionally via `--stage4-preference` in `train_multitask.py` or standalone CLI) that starts from the Phase 3 `1M + Gated DeltaNet-2` SFT checkpoint and applies:
  1. **Closed-form categorical pairwise DPO + Margin loss** (`beta = 0.2`, `margin = 0.25`, weighted by `TabM` epistemic disagreement `T-DUR`) over preference triples (`Silent > Wait > Go` on habituation/unaddressed chatter; `Wait > Silent > Go` on open-floor loose mentions; `Go > Wait > Silent` on direct dyadic/allocated turns).
  2. **Asymmetric unpaired KTO loss** (`lambda_u = 2.5`, `lambda_d = 1.0`) penalizing false-speak `Go` errors harder than conservative deferrals.
  3. **Counterfactual vectorless invariance (`CFPO`)** minimizing Jensen–Shannon divergence between `(e_256, x_num)` and `(0_256, x_num)` on timing-dominated habituation/patience sequences, eliminating the `4.0 s` vectorless gap shortcut behind `dev-07-habituation`.
  4. **Re-calibration & Export:** Re-fit `temperature` and non-vacuous `cic_threshold` (`target_risk = 0.05`, `tau > 0.25`) on the held-out calibration split, export `training/artifacts/neural_phase3_best.safetensors` + `manifest_phase3.json` + regenerated Phase 3 golden fixtures, and verify Rust↔Python parity (`<= 1e-5`).

### 8. Zero-API Indonesian MRL & Prefix Probe Baseline
- **Decision:** Add a clean `$0` offline evaluation script/subcommand (using local `EmbeddingGemma 2` weights at `lab/datasets/embedder_models/embeddinggemma-2` and the exact `IndoCollex` `MIT` + `MultiLexNorm` `ind`/`ind_eng` datasets already pinned in `lab/datasets.manifest.toml`) sweeping `dim_d ∈ {768, 512, 256, 128}` × `prefix ∈ {t_q/t_d, t_sts, unprefixed}`, and write the measured baseline table to `research/findings/2026-11-21-indonesian-embedder-baseline.md`.

### 9. Reproducible Offline Markdown Export of Learned State (`monoagens export-learning`)
- **Decision:** Add a `Commands::ExportLearning(ExportLearningArgs)` subcommand in `crates/cli/src/lib.rs` (`monoagens export-learning --out <path.md> [--data-dir <dir>] [--log-json <path>] [--place <place_id>] [--include-private] [--json]`) that reconstructs `CoreState` offline via deterministic log replay (zero LLM API calls), orders all tables canonically by `PlaceId` / `EventId` (`BTreeMap`), and renders 6 human-readable sections:
  1. Audit & Reproducibility header (`log_seq`, `state_epoch`, `model_blake3`, `online_adaptation`, `export_blake3`).
  2. Per-place adaptation equalizer (`PlaceAdaptState`: `logit_offsets`, `film_scale_deltas`, `film_shift_deltas`, `arm_pulls`, `last_updated_ms`).
  3. Learned procedures (`SkillBook::procedures`: name, steps, `required_cap_ids`, lifecycle state, Beta reliability `alpha/(alpha+beta)`, `trial_count`/`success_count`, `cusum_stat`, retirement reason).
  4. Contrastive lessons (`SkillBook::lessons`: pattern, failure signature, domain, credence, `audience`, source IDs).
  5. Beliefs & conventions (`BeliefBook`: active beliefs/conventions, credence, domain, taint, superseded marker).
  6. Consolidated episodes & gists (`EpisodeBook` + surviving gist summaries).
- **Privacy & `hard_forget` Guarantee:** By default (`--include-private = false`), items whose `Audience` is strictly private (e.g., `<= 2` actors / DM-scoped) are excluded from the exported Markdown unless the owner explicitly passes `--include-private` (and every exported section labels its `Audience`). Because export runs on post-`HardForget` replayed state (including `C-3` `SkillBook::forget` and `C-8` `EpisodeBook::forget`), erased events and actors leave zero bytes in the exported Markdown.

## Risks / Trade-offs

- **[Risk] Tightening `cic_threshold` (`> 0.25`, `target_risk = 0.05`) could cause excess `CicAbstention` on low-confidence valid `dev`/`val` turns.** → Mitigation: Verify `development` and `validation` 5-seed `--release` suites immediately after calibration; cap `min_coverage >= 0.50` (or verify `validation` `when_to_speak_f1 == 1.0`) and keep `CommitmentInputs` strictly ahead of `cic_abstain`.
- **[Risk] Re-computing `EmbeddingGemma 2` training shards with `"title: none | text: "` prefix invalidates old unprefixed shard cache files.** → Mitigation: Version the shard cache filename or cache key with `prompt_regime_id` (e.g., `phase1_td_shards.npz`) so unprefixed and prefixed caches never collide silently.
- **[Risk] Changing `PlaceAdaptState` or `LessonEntry` structs could break existing serialized snapshots (`SNAPSHOT_VERSION = 10`).** → Mitigation: Use `#[serde(default)]` on additive fields and test v10 snapshot deserialization compatibility (or bump `SNAPSHOT_VERSION` cleanly with backward-compatible upcast).
- **[Risk] Removing premature `Go` `+0.5` reward on `ReceiptOutcome::Done` could shift existing unit tests that specifically assert `place_adapt.arm_pulls[Go] > 0` after a receipt.** → Mitigation: Run GitNexus `impact` before editing `step.rs` and `adapt.rs`, audit every affected test, and update tests intentionally to reflect the new spec contract.

## Migration Plan

1. Implement Core harness & neural controller fixes (`C-2`, `C-3`, `C-4`, `C-8`, `C-9`, `C-10`) first, verifying with `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
2. Implement training pipeline fixes (`C-1` `TaskPrefix` parity, `C-4` non-vacuous `CIC`, `E-70.1` Stage 4 DPO/KTO + `CFPO`), run `uv run pytest -v`, regenerate prefixed shards + retrain Phase 3 + Stage 4 on RTX 4060, and verify `cargo test -p monoagens-core neural_parity`.
3. Rebuild `./target/release/monoagens-lab` and run full 5-seed offline `--release` `development` + `validation` suites & ablations (`neutral`, `online_adapt`), updating `research/findings/2026-11-21-v2-offline-test-drive.md`.
4. Run the `$0` offline Indonesian MRL/prefix probe sweep (`IndoCollex` + `MultiLexNorm`) and write `research/findings/2026-11-21-indonesian-embedder-baseline.md`.
