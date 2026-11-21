# Run 5 Supporting Note 1: Mechanism, `Prior` Field, & `crates/core` LOC Audit (`R-59`)

- **Workspace Anchor:** `/home/muqsith/Projects/Mono` (`main`)
- **Total `crates/core/src/` LOC observed:** `34,222` lines across 44 `.rs` files (`find crates/core/src -name '*.rs' | sort | xargs wc -l`).
- **Total `Prior` fields observed:** `184` `pub` fields in [`crates/core/src/prior.rs`](file:///home/muqsith/Projects/Mono/crates/core/src/prior.rs#L579-L1158).

---

## 1. Full 22-Mechanism Audit (`M01`–`M22`, [`crates/core/src/registry.rs`](file:///home/muqsith/Projects/Mono/crates/core/src/registry.rs#L16-L492))

| ID | Mechanism (`registry.rs`) | Category | Keep Deterministic vs Neuralize | Rationale & Boundary |
|---|---|---|---|---|
| `M01` | `Salience` | **Soft scoring** | **Neuralize** (`gate_4` head) | Linear weighted sum `sum w_k * s_k` over 6 signals (`salience.rs:205-217`); directly replaced by learned policy over `(emb_256d, x_num)`. |
| `M02` | `FloorCost` | **Soft scoring** | **Neuralize** (`gate_4` + `wait_delta_t`) | Noisy-OR `deg(e, l)` + `social_cost` (`signals.rs:208-377`) uses 13 hand-tuned parameters (`lambda_c`, `w_c`, `w_o`, `gamma_c`, `tau_c_s`, `k_c`, `beta_fan`, etc.) to approximate floor etiquette. |
| `M03` | `Habituation` | **Soft temporal gating** | **Neuralize (via `h_t` or compact decay feature)** | `S / (1 + kappa * h_pre)` (`salience.rs:98-124`) is a soft repetition penalty; can be absorbed by recurrent `h_t` or a single decayed recency feature. |
| `M04` | `DriveThreshold` | **Hybrid (soft + hard budget)** | **Split:** Keep **hard daily budget / Valve cap** deterministic; **Neuralize** soft `talk_rate` / `promise_debt` weighting | Hard budget enforcement belongs in Valve/harness; soft trade-off between `talk_rate`, `budget_used_ratio`, and `promise_debt` (`threshold.rs:222-290`) is a learned gating surface. |
| `M05` | `FamiliarityReserve` | **Soft scoring** | **Neuralize** | `r * (1 - familiarity)` (`threshold.rs:240-244`) is a 1-feature linear reserve; `familiarity` scalar in `x_num` lets the controller learn place-novelty caution. |
| `M06` | `StochasticGoNoGo` | **Sampling / decision rule** | **Keep thin seeded draw (optional) or argmax** | Seeded RNG draw `SeededRng::draw_unit(seed, event_id)` (`decide.rs:292`) is 10 lines; if `gate_4` outputs calibrated probabilities, seeded sampling or deterministic threshold remains trivial and replayable. |
| `M07` | `CommitmentBreaker` | **Hard commitment / safety** | **Keep deterministic** | Explicit allocated turns, open deadlines, and active chains (`decide.rs:75-90`) MUST never be dropped stochastically by a neural net. |
| `M08` | `Patience` | **Soft timing** | **Neuralize** (`wait_delta_t` head) | Log-normal shrunk quantile wait (`patience.rs`, 8 `Prior` knobs) predicts how long to hold before speaking; natural regression/quantile head on `gate_4`. |
| `M09` | `ReportBarOnThought` | **Soft metacognition** | **Neuralize** (`p_uncertainty` / `p_verify` hint) | Speaking/certainty bars (`prior.rs:596-599`) filter low-value/uncertain outputs; neural confidence hint informs budget/context without overriding Valve. |
| `M10` | `ActivationRecall` | **Soft ranking** | **Neuralize** (`p_memory_relevance` reranker) | `w_B * B + context_weight * cos + importance_weight * imp` (`signals.rs:490-491`) is a 3-feature linear scorer over `VisibleScope`-filtered candidates. |
| `M11` | `VectorFade` | **Hard lifecycle / storage bound** | **Hybrid:** Keep **hard storage/retention floor** deterministic; **Neuralize** `p_encode_importance` | Hard forgetting (`R-34`) and bounded vector-index pruning require deterministic invariants; importance scoring of what to retain/fade can use `p_encode_importance`. |
| `M12` | `PredictionAndGrounding` | **Hybrid (structural + soft)** | **Keep prediction ledger deterministic**; **Neuralize** surprise/uptake scoring | Recording open `Effect`/`Response` expectations and matching receipts (`expectations/mod.rs`) is deterministic state; semantic mismatch scoring can use neural/cosine heads. |
| `M13` | `ProspectiveMemory` | **Hard commitments / deadlines** | **Keep deterministic** | Typed promises, `RecurrenceSpec`, `CommitCheck`, due-time triggers (`expectations/mod.rs`) are hard contracts; neural nets cannot guarantee deadline recall. |
| `M14` | `Credence` | **Soft belief calibration** | **Neuralize** (`p_uncertainty` / `p_verify`) or **Keep lightweight Beta count** | Domain-decayed Beta counts (`beliefs/credence.rs`, 1,097 LOC) are heavily parameterized; source-reliability tracking can be slimmed or surfaced as numeric features to `p_verify`. |
| `M15` | `BoundedLearning` | **Online adaptation** | **Replace with fast online head / `h_t` adaptation** | Sign-corrected IPS-weighted delta rule (`learn/mod.rs`, 465 LOC) tunes linear salience weights; replaced by bounded last-layer / bias / FiLM or `h_t` adaptation (`R-64`). |
| `M16` | `MemoryTruth` | **Hard provenance & safety** | **Keep deterministic** | `basis`, `valid_to`, event-beats-gist, `VisibleScope` (`privacy/visible_scope.rs`) are anti-confabulation and privacy invariants (`R-33`, `R-43`); never neural. |
| `M17` | `ReportControl` | **Soft metacognition** | **Neuralize** (`p_uncertainty` & `p_verify`) | Calibrated confidence vs certainty bar (`prior.rs:596`) maps directly to selective-prediction uncertainty head. |
| `M18` | `ConsolidationAndMerge` | **Hybrid (log projection + soft gating)** | **Keep log/episode state machine deterministic**; **Neuralize** `p_encode_importance` | Append-only `{open, provisional, sealed}` lifecycle and `hard_forget` are deterministic; deciding importance/merge priority can use `p_encode_importance`. |
| `M19` | `PerceptionLedger` | **Hard provenance / identity** | **Keep deterministic** | Promisee-aware ledger cards & 4-pillar `self_card` (`ledger/mod.rs`, 761 LOC) track who-knows-what (`VisibleScope` / ToM provenance) and capability LCBs; slimmed, not neuralized. |
| `M20` | `Agency` | **Hard self-echo safety** | **Keep deterministic** | Matching `(channel, delta_obs, delta_pred)` effect predictions (`ARCHITECTURE.md:198-200`) prevents infinite self-reply loops; must be deterministic. |
| `M21` | `EventBoundaries` | **Soft segmentation** | **Neuralize** (via `h_t` state / boundary head) | EM-LLM surprise + empirical-Bayes shrinkage (`consolidation/boundaries.rs`, 271 LOC, 11 `Prior` knobs) is a soft sequential surprise detector. |
| `M22` | `BacklogChunking` | **Hybrid (queue bound + pacing)** | **Keep max-chunk bound deterministic**; **Neuralize** pacing/salience | Bounded queue chunking (`gate/backlog.rs`, 229 LOC) prevents post-downtime storms; `h_t` + `x_num` handles urgency. |

---

## 2. `crates/core/src/` Module-by-Module LOC Breakdown & Slim Harness Estimate

Measured directly via `wc -l` on `crates/core/src/`:

| Module / File(s) | Current LOC | Slim Harness Role | Estimated Slim LOC | Reduction |
|---|---:|---|---:|---:|
| `prior.rs` | 2,725 | Replace 184-knob `Prior` + validators with ~15–25 structural config fields + model manifest ref | ~300–450 | **~85%** |
| `gate/` (`signals`, `decide`, `salience`, `threshold`, `drives`, `patience`, `backlog`, `mod`) | 2,266 | Keep `CommitmentInputs`, `DecisionTrace` (neural scores + BLAKE3 hash), seeded RNG, and feature extraction (`x_num`); replace hand-crafted signal/drive/patience math with pure-Rust neural forward pass (~150–250 LOC) | ~550–750 | **~70%** |
| `beliefs/` (`credence.rs`, `domains.rs`, `mod.rs`) | 1,734 | Replace spherical k-means Hungarian domain alignment (`domains.rs`, 624 LOC) & complex Beta decay with minimal source-reliability summary or neural `p_verify` | ~250–400 | **~78%** |
| `consolidation/` (`boundaries`, `episodes`, `fade`, `skills`, `mod`) | 1,761 | Keep deterministic episode log lifecycle & hard fade floor; prune EM-LLM shrinkage & complex CUSUM/contrastive heuristics | ~500–700 | **~62%** |
| `map/` (`graph.rs`, `mod.rs`) | 445 | Optional/pruned if `p_memory_relevance` + `h_t` subsume SR/PPR graph (`map_*` = 12 `Prior` fields) | ~0–150 | **~65–100%** |
| `learn/mod.rs` | 465 | Replaced by frozen backbone + compact online head/bias (`~100–150` LOC) or `h_t` | ~100–150 | **~70%** |
| `recall/` (`engine`, `importance`, `recollection`, `mod`) | 680 | Keep `VisibleScope` pre-filter + store query + neural `p_memory_relevance` reranker | ~250–350 | **~50%** |
| `event.rs`, `state.rs`, `turn.rs`, `context.rs` | 17,228 | Core event schema, append-only log projection, `ContextPlan` assembly, turn loop. Substantial boilerplate shrinks as `Prior`/mechanism state fields drop from `CoreState` and `EventDetail` | ~10,000–12,500 | **~30–40%** |
| `privacy/`, `replay.rs`, `rng.rs`, `blake3.rs`, `ports.rs`, `mem_store.rs`, `registry.rs`, `tck_hooks.rs`, `lib.rs`, `loop/`, `ledger/`, `expectations/`, `activation.rs` | 6,918 | Keep `VisibleScope`, `replay.rs`, `rng.rs`, `blake3.rs`, `ports.rs`, `expectations/` (commitments/agency), `ledger/` (provenance), slimmed `registry.rs` | ~4,500–5,500 | **~25%** |
| **Total `crates/core/src/`** | **34,222** | **Slim Deterministic Harness + Pure-Rust Neural Controller** | **~16,500–20,500** | **~40–52%** |

*(Note: Estimates for `event.rs`/`state.rs`/`turn.rs` are inferred from the fraction of fields tied to the 22 mechanisms and 184 `Prior` knobs; exact LOC depends on whether `crates/core` is pruned in place or rewritten as `core-v2`.)*

---

## 3. Proposed Minimal Model-Agnostic Numeric Feature Vector (`x_num`, 18 Scalars)

Combined with the frozen `256d` `EmbeddingGemma 2` vector (`EMBEDDING_GEMMA_2_DIM = 256` in `crates/reason/src/embed/embeddinggemma2.rs:34`), total input dimension is **`274d`** (or `284d` if `FModal(pub [f32; 10])` from `crates/protocol/src/message.rs:84` is concatenated directly for embodied/voice bodies):

| # | Feature | Range / Transform | Replaces Signals / Knobs |
|---|---|---|---|
| 1 | `is_named_soul` | `{0.0, 1.0}` | `own_name` |
| 2 | `is_explicit_addressee` | `{0.0, 1.0}` | `explicit` in `compute_deg` |
| 3 | `is_reply_or_expect_link` | `{0.0, 1.0}` | `link` / `expectation_hit` |
| 4 | `is_dyadic_place` | `{0.0, 1.0}` | `is_dyadic_chat` |
| 5 | `active_participant_count_norm` | `log1p(N) / log1p(64)` in `[0, 1]` | `active_participants.len()` |
| 6 | `dt_since_last_place_event` | `log1p(dt_s) / log1p(86400)` in `[0, 1]` | Place cadence / `tau_c_s` |
| 7 | `dt_since_last_soul_act` | `log1p(dt_s) / log1p(86400)` in `[0, 1]` | `h_init` / cooldown |
| 8 | `dt_since_sender_last_act` | `log1p(dt_s) / log1p(86400)` in `[0, 1]` | `compute_deg` continuity |
| 9 | `soul_talk_share_window` | `soul_acts / max(1, total_acts)` in `[0, 1]` | `talk_rate` (`a_scale`, `talk_rate_window_minutes`) |
| 10 | `sender_familiarity` | `1 - exp(-exchange_count / 5)` in `[0, 1]` | `closeness_sender` + `familiarity` |
| 11 | `place_familiarity` | `1 - exp(-place_events / 20)` in `[0, 1]` | `FamiliarityReserve` (`M05`) |
| 12 | `budget_used_ratio` | `[0.0, 1.0]` | `budget_weight` (`M04`) |
| 13 | `imminent_promise_debt` | `clamp(debt, 0, 1)` | `promise_debt` (`M04`, `M13`) |
| 14 | `max_visible_mem_cos` | `clamp(cos, 0, 1)` (`0.0` if none) | `relevance` (`M10`) |
| 15 | `max_visible_mem_recency` | `log1p(age_s) / log1p(2_592_000)` | `Petrov B` / `w_B` (`M10`) |
| 16 | `pending_belief_uncertainty` | `max_belief_voi` in `[0, 1]` | `own_info_gap` (`M14`, `R-31`) |
| 17 | `backlog_queue_depth_norm` | `log1p(q) / log1p(256)` in `[0, 1]` | `BacklogChunking` (`M22`) |
| 18 | `asr_conf_or_one` | `FModal[9]` if voice else `1.0` | `asr_clarification_boost` (`R-37`) |

*(Optional `FModal` extension: append remaining 9 channels of `FModal([f32; 10])` for continuous sensor/robot/voice bodies, yielding `284d` total.)*
