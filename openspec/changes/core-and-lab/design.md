# Design

## Context

See `proposal.md` for motivation. The repository holds `ARCHITECTURE.md`, `RESEARCH.md`, Run 1
(`research/findings/2026-11-15-run-1.md`), Run 2 (`research/findings/2026-11-16-run-2.md` and
`research/findings/run-2-notes/`), and no code yet. The owner's decisions so far:

- Rust (latest stable toolchain via `rustup`, Edition 2024) for core crates, paired with latest stable TypeScript/Python SDK & benchmark adapter runtimes resolved in lockfiles and verified against official docs via **`context7` MCP** (`resolve-library-id` -> `query-docs`) before coding.
- This change builds the complete `core`, `store`, `reason`, and `lab` stack covering `RESEARCH.md`
  sections 1–26 as audited and upgraded by Run 2 (`A-01`–`A-16`, `R-29`–`R-40`, `R-43`, `R-46`,
  `R-48`–`R-50`), co-evolving across Phases 0–3 with successor change `waist-valve-and-bodies`.
- `core` enforces structural privacy non-interference (`VisibleScope` before Step 5, `R-33`),
  atomic erasure (`hard_forget` + `Two-Epoch Replay`, `R-34`), provenance taint tracking (`R-43`),
  and `OwnerSigned` identity locks (`R-48`), while runtime `cedar-policy` Valve rules and live
  bodies live in `waist-valve-and-bodies`.
- Research Run 1 (`2026-11-15`) and Run 2 (`2026-11-16`) + 2026-11-16 Owner Decisions (Option A
  additive `Receipt` keeping `ARCHITECTURE.md` L198 untouched; `EmbeddingGemma 2` `256d` Apache 2.0
  unified Gateway space; dual-change split) are accepted as specification input.

Reading the rules below: text tagged **(R-nn)** / **(A-nn)** comes from that finding. Findings
separate established literature from engineering synthesis; quantitative start values carry bounded
ranges tuned deterministically in the Lab. Open hardware profiling and Lab calibration points stay
explicitly flagged for Lab runs (Decision 19).

Constraints:

- The invariants of `ARCHITECTURE.md` are fixed (never edit `ARCHITECTURE.md`).
- Canonical terms follow `ARCHITECTURE.md`: gateway, doors, valve, Waist 1, Waist 2.

## Goals / Non-Goals

**Goals:**

- A sans-IO, bit-identical replayable `Core` implementing `RESEARCH.md` §§1–26 with zero Round-1
  contradictions or sign/denominator/spin-loop bugs (`A-01`–`A-16`).
- Structural `Delta_NI = 0` pre-Step-5 privacy projection (`R-33`), zero-resurrection `hard_forget`
  (`R-34`), event-derived 4-pillar `self_card` (`R-35`), 3-door taint safety (`R-43`), and
  commitment integrity (`R-49`).
- A Lab covering all **22** brain-map ablation metrics (`A-02`), `S >= 354–471` paired NI power
  (`A-16`), `BaselineAdapter` & 5-domain Pareto scorecard (`R-39`), Indonesian/`MultiLexNorm`
  suites (`A-15`), and black-box `Core TCK` (`C-MUST-01..04`, `R-50`).
- Freeze clean `Waist 1` (`monoagens.core.v1`) so `waist-valve-and-bodies` connects cleanly from
  Phase 0 onward without reworking `Core`.

**Non-Goals:**

- Live network/stdio daemon process (`monoagens-gateway`), runtime `cedar-policy` policy bundle,
  keyed-`BLAKE3` media server, `body-cli`, `body-mcp-bridge`, and standalone `monoagens-conformance`
  CLI runner (owned by sibling change `waist-valve-and-bodies`).
- Unrequested runtime emotion classification or unapproved edits to `ARCHITECTURE.md`.
- `no_std` builds; running paid live LLMs in `cargo test`.

## Decisions

### 1. The owner's rulings on four contradictions

| Contradiction | Ruling |
|---|---|
| "Owner" appears in the core (`RESEARCH.md` §7, §8) | The core never knows an owner. Whoever runs the soul supplies two numbers per actor: a reliability prior (α₀, β₀) and a teacher weight. |
| A text index in the core (§5) | Text search sits behind the store port. The core passes the id of the query event; the store reads text and returns ids. |
| A reply that answers a prediction is marked self-caused (§17 against §19) | Predictions have two kinds. A matched *effect* prediction is self-caused. An answered *response* prediction wakes the soul. |
| Who owns the hooks (§16 against `ARCHITECTURE.md`) | The core owns the three hook points and calls them inside its turn. What runs in them (the valve, plugin hooks) belongs to the gateway. |

Also ruled (Run 1 & Run 2 Owner Decisions `2026-11-16`): post-eval partitioned habituation κ &
guarded Petrov hybrid base-level (`n > k ∧ t_n > t_k`); explicit `memory.recall` capability;
split `A-14` `dialogue_act_gap` contrastive logistic head `(w, b)` vs `own_info_gap`; **Option A**
additive `Receipt` `{termination_reason, progress, delivered_units}` preserving `ARCHITECTURE.md`
L198 5 outcomes; **`EmbeddingGemma 2` (`256d` MRL, Apache 2.0)** unified text+image+`audio_voice`
embedder with `embed_model_ver` partitioning; and dual OpenSpec split (`core-and-lab` +
`waist-valve-and-bodies`).

### 2. Five crates; dependencies point one way

```
crates/
  protocol/src/     monoagens-protocol
    ids.rs          opaque ids: actor, place, capability, event
    facts.rs        audience, capability descriptor, receipt outcomes
    message.rs      percept and its reportable facts, action, receipt, body state, actor facts
    thought.rs      think, act, expect, resolve, revise; classification verdict
    version.rs      accept older, ignore unknown fields, reject newer
  core/src/         monoagens-core   (depends on protocol + serde only)
    lib.rs          Core::open, step, next_due
    event.rs        event kinds, format versions, upcasting
    ports.rs        Store and Doors traits
    mem_store.rs    in-memory Store for tests
    turn.rs         the order of one turn
    state.rs        the six books: projections rebuilt from the log
    rng.rs          seeded draw from (seed, event id)
    prior.rs        dispositions, ranges, validation
    registry.rs     the brain map: id, claim, neutral value; Switches
    evidence.rs     beta counts and discount, rule of three, 95% interval
    activation.rs   Petrov's hybrid base level (shared by memory, closeness, familiarity)
    gate/           signals.rs salience.rs threshold.rs decide.rs patience.rs backlog.rs
    memory/         recall.rs importance.rs closeness.rs recollection.rs
    expect/         record.rs timing.rs promise.rs goal.rs wake_gate.rs grounding.rs
                    agency.rs silence.rs
    think/          context.rs acts.rs bars.rs report.rs
    belief/         basis.rs validity.rs credence.rs reliability.rs domain.rs
    sleep/          trigger.rs boundaries.rs consolidate.rs merge.rs fade.rs
    ledger/         cards.rs self_card.rs
    learn/          outcome.rs delta.rs bounded.rs band.rs rollback.rs
    map/            graph.rs habits.rs conventions.rs
    privacy/        visible_scope.rs declass.rs
    replay.rs       reprocess a log and compare decisions (Two-Epoch post-forget)
  store/src/        monoagens-store  (SQLite + VectorIndex[usearch|hnsw_rs|flat] + blake3 + postcard): schema.rs log.rs text.rs vectors.rs shards.rs forget.rs snapshots.rs
  reason/src/       monoagens-reason (what the core needs in order to think)
    render.rs tools.rs parse.rs secrets.rs info_gap_clf.rs
    model/          scripted.rs openai.rs
    embed/          none.rs hash.rs openai.rs embeddinggemma2.rs scripted.rs  (ort/LiteRT/sidecar local backends)
  lab/src/          monoagens-lab    (library + `monoagens-lab` binary)
    scenario.rs soul_dir.rs driver.rs clock.rs test_body.rs guard.rs checks.rs metrics.rs
    ablation.rs coverage.rs calibrate.rs baseline.rs benchmark.rs pareto.rs tck_hooks.rs record.rs probe.rs inspect.rs main.rs
prior/              dispositions.json
lab/scenarios/      dev/ validation/ sealed/
lab/runs/           run records; reference/ for published results
research/           researcher-prompt.md, findings/, backlog.md
docs/               lab.md testing.md mechanisms.md
```

A test fails the build if `core` depends on anything but `protocol` and `serde`. `reason` and
`store` depend on `core`; `lab` depends on all.

### 3. The core is a sans-IO state machine with synchronous ports

```rust
impl<S: Store> Core<S> {
    fn open(store: S, prior: Prior, switches: Switches, seed: u64) -> Result<Self>;
    fn step(&mut self, input: Input, now: Time, doors: &mut dyn Doors) -> Result<Vec<Effect>>;
    fn next_due(&self) -> Option<Time>;
}
```

- `Input`: percept, receipt, model result, model failure, tick, identity, body state, actor
  facts.
- `Effect`: `Model { purpose, plan }` with purpose reason, classify or consolidate; and
  `Dispatch(Action)`.
- `Store` (port): append, read in order, get, candidates by vector, candidates by text (given
  the id of the query event), release a vector.
- `Doors` (port): `inbound`, `context`, `action`, called by the core at steps 3, 6 and 7 of
  its turn. The core ships a no-op implementation.
- `step` appends the input first, then follows `ARCHITECTURE.md` *One turn of the loop*.
  On `Core::open` (A-01 crash recovery), Core inspects trailing recorded inputs lacking a terminal
  `decision`/`receipt_processed` marker and resumes them in deterministic timestamp order before
  accepting new inputs (`Two-Epoch Replay` post-snapshot after `hard_forget`, R-34).

### 4. Six books, one log (R-12, R-34, R-40)

All state is a projection of the log, in six books (`postcard`/`minicbor` snapshot serialization
with `blake3` checksums over `hot.db` WAL + read-only sealed temporal shards + `usearch` HNSW, R-40):

| Book | Holds |
|---|---|
| log | every event, with use times for activation (`hard_forget`-scrubbable, R-34) |
| memories | episodes (`open`/`provisional`/`sealed`), gists, lessons, procedures, SR graph (R-32), domain centroids |
| expectations | effect, response, wait, promise (`PromiseMeta`), goal, monitor (`RecurrenceSpec`), `pending_approval` |
| sources | ledger cards (`tick` promisee-aware), `self_card` (4 pillars, R-35) |
| credences | (α, β) counts per belief (`told`-initialized + `k_corrob` taint) and per (source, Hungarian-aligned domain) |
| dispositions | knobs, weights, per-place/per-source offsets, partitioned `h_{(p, k_src)}` |

Event kinds (30 canonical): `percept`, `decision`, `wake_started`, `model_call`, `thought`,
`withheld`, `action`, `action_feedback`, `receipt`, `expectation_opened`, `expectation_changed`,
`verdict`, `memory_used`, `belief_changed`, `evidence`, `episode_closed`, `sleep_run`,
`domains_updated`, `vector_released`, `timing_updated`, `learning_change`, `rollback`,
`door_verdict`, `identity_changed`, `body_state`, `actor_facts`, `tick`, `failure`, `hard_forget`,
`prior_loaded` (added by Decision 20).

Every event stores kind, format version, time, actor, place, audience, `origin_actor`, `taint`,
`caused_by`, numeric features (`f_modal`, `orient`, `deg`, etc.) and an opaque payload (plus
optional `AttachmentHandle`s, never raw media bytes). A **chain** links events via `caused_by`;
`origin_actor` and min-trust taint propagate monotonically and cannot be stripped by a model. A
tick appends only when it fires or updates recurring state (`RecurrenceSpec`).

### 5. What the core is told, as numbers and ids

| Fact | Supplied by | Used for |
|---|---|---|
| event vector, `embed_model_ver`/`dim_d`/`prompt_regime_id` | `reason` (`EmbeddingGemma 2` `256d`) | signals, recall, wake gate, novelty, misses |
| `dialogue_act_gap` logistic score `(w, b)` | `reason` (`A-14`) | non-word-list `info_gap` gate signal |
| addressees, `named`, `orient(e, l)`, `continued_attention` | the body | `deg(e, l)` (R-29), circuit breaker, `social_cost`, grounding |
| `reply_to`, correlation id, `channel`, `Δ_obs`, `Δ_pred` | the body | grounding, agency (R-09), goals |
| length, negative flag, `repair`, 10-slot `f_modal` | the body | content miss, learning, blind speech/turn-taking (`R-37`) |
| `capability_schema_hash`, latency, effect window/noise | the capability descriptor | timing, agency, procedure schema validation (`R-38`) |
| condition and expected-reply vectors, `domain?` | `reason` / external body | wake gate, goals, misses, belief domain (`A-01`) |
| `AttachmentHandle` (`handle_id`, `modality_slot`, `ttl`) | Gateway (`blake3` store) | passed blindly to `ModelProvider` post-`VisibleScope`/`Valve` |
| reliability prior (α₀, β₀), `teacher_weight` | Gateway per actor | reliability (`told` incl.), learning (`R-26`) |
| `VisibleScope` / `DeclassToken` / `OwnerSigned` anchor | Gateway / Valve | pre-Step-5 privacy (`R-33`), identity lock (`R-48`) |
| daily budget, knowledge cutoff | Gateway | budget drive, time grounding |

### 6. Gate (§2–4, §15, §22–24; A-01, A-03, A-04, A-07, A-08, A-14, R-02..R-04, R-14, R-29..R-31, R-36)

- **Signals (A-14, R-29):** `own_name`, `feeling_of_knowing` (`A*` over `VisibleScope`-filtered
  candidates), `relevance`, `dialogue_act_gap = σ(w_gap·e + b_gap)` + `own_info_gap` (`A-14`),
  plus `asr_conf < 0.45` clarification boost (`R-37`).
- **Graded Address & `social_cost` (R-02, R-29):** Noisy-OR `deg(e, l) ∈ [0, 1]` over explicit
  address, open prediction link, dyadic momentum, and `orient(e, l)`.
  `social_cost(e) = max(clamp(max_{l≠soul} deg(e, l) − deg(e, soul), 0, 1), answered(e)) · (1 − λ_c · closeness(sender))`.
- **Salience & Reachable Neutral Weights (A-01, R-03):** `S = clamp(Σ wₖ·sₖ, 0, 1)` with
  `w_info = info_scale·helpfulness`, `w_fok = fok_scale·helpfulness`, `w_sc = −restraint_scale·restraint`
  (reaches exact `0` when `restraint = 0` or `social_cost_off`).
- **Post-Eval Partitioned Habituation (A-03, R-36):** Partitioned by `(place p, source_channel k_src)`:
  `h_pre = h_{(p, k_src)}·γ^Δt`; evaluate `S_eff = S / (1 + κ·h_pre)` FIRST; then `h ← h_pre + 1`.
  `κ = 0` is exact neutral (`S_eff = S`); stimulus 1 is never halved.
- **Threshold & Drives (A-01, A-04, R-30, R-31):**
  `θ = θ₀ + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity) + Δθ_init`, where
  `a = a_scale·restraint` (reaches `0`), `Δθ_sleep = 0` permanently (`A-04`), `promise_debt` sums
  urgency weights `Σ w_p` over imminent non-recurring promises (`A-01`/`R-46`), `talk_rate` uses
  cold-start shrinkage (`A-07`), and `R-31` drives adjust idle/switch/depth budgets.
- **Closeness & Guarded Petrov Base Level (A-01, R-04):** Guarded Petrov hybrid (`k = 1`): use tail
  integral when `n > k ∧ t_n > t_k`, else exact `ln(Σ t_j^(−d))` (`0/0` guarded; 1-min floor);
  `n = 0` yields neutral `0`. `B_ctx = (B_P + Σ_{p∈Present} B_p)/(1 + |Present|)`.
- **Draw, Breaker & Idle Initiative (A-01, R-30):** Explicit step limit `S_eff >= θ` as `τ → 0`.
  Breaker fires on allocated/high-`deg` turns, expectations, or active goals. Unprompted idle
  initiatives respect `EM-LLM` breakpoints and raise temporary habituation cooldowns.
- **Patience (A-08):** Unallocated `go` opens `wait` with right-censored shrinkage quantile
  estimator `clamp(t_p, [t_min, t_max])`; `0` when disabled.
- **Backlog (A-04, R-14):** Delete `"or asleep"` — backlog chunking triggers ONLY on
  `now − t_event > patience_place` or genuine `stopped/down` downtime, combining via event-level
  age-discounted `max` (≤ 4 chunks/wake).

### 7. Memory & Pre-Step-5 Privacy (§5, §14, §25; A-01, A-09, R-05, R-16, R-32, R-33, R-37, R-40)

- **Pre-Step-5 `VisibleScope` (R-33):** Candidates are projected through `audience(context) ⊆
  audience(item)` (or explicit `DeclassToken`) **before** Step-4 `feeling_of_knowing`/recall and
  Step-5 gate scoring (`Δ_NI = 0`).
- **Pre-Filtered Search & Unified `256d` Space (A-15, R-12, R-37, R-40):** Union of `usearch` HNSW
  vector top-40 (within matching `(embed_model_ver, dim_d, prompt_regime_id)`) and Gateway `Store`
  text top-20. Rank by `A = w_B·B + context_weight·cos + importance_weight·importance`, where
  `w_B` enables exact Row-10 neutral (`similarity only`).
- **Dual-Process Recollection (A-09, R-16):** Decouples continuous familiarity (`A` + `F`) from
  recollection (`R` = matched verbatim event IDs) so 30-day recall hits are never blocked by
  mandatory LLM verification.
- **Importance (R-05):** `max(i_reason, i_commit, i_surprise)` (`A-06` normalized surprise).
- **Cognitive Maps & Fade (A-01, R-32, R-40):** Core maintains opaque ID transition/co-occurrence
  edges (`Successor Representation`) inside `memories` (`R-32`). Vector fade checks **base level
  `B < fade_threshold = −3.29`** (`Petrov` exact 30-day threshold), never undefined total `A`.

### 8. Expectations, Routines & Commitments (§6, §17, §19–20; A-01, R-07..10, R-18..19, R-36, R-42, R-46, R-49)

- **Kinds:** `effect`, `response`, `wait`, `promise`, `goal`, `monitor`, `pending_approval`.
  Records carry plural **`places`** and **`people`**, optional `domain?`, `RecurrenceSpec` (`R-46`:
  `cron`/`interval`, `coalesce_one`/`skip_to_next`, isolated from `promise_debt`), and `PromiseMeta`
  (`promisee_ids`, `hard_deadline`, `depends_on?`, conflict marker, `R-49`).
- **Timing & Grounding (A-08, R-08):** Shrinkage quantiles per `(place, medium)`; answering cues
  include `done` receipt, addressee follow-up, or `continued_attention` (`A-01`).
- **Agency (R-09, R-36):** Effect match checks correlation ID, self-actor, or `(channel,
  [lat_min, lat_max], |Δ_obs − Δ_pred| ≤ k·σ_channel)` (`basis = self_caused`).
- **Preemption Receipts & Promises (R-10, R-36, R-37, R-42, R-49):** `Receipt` keeps 5 outcomes
  `{done, refused, held, failed, unknown}` (`ARCHITECTURE.md` L198) + additive `{termination_reason,
  progress, delivered_units}` (`normal | canceled_by_core | preempted_external | timed_out`).
  `held` enters non-retrying **`PendingApproval`** pinned to canonical `H_act` (`R-42`);
  `canceled_by_core`/`preempted_external` do not trigger false retry storms or reliability loss;
  `resolve` supports `{kept, kept_late, renegotiated, released, failed, cancelled, answered, reached}`.
- **Content Miss & Judged Silence (R-18, R-19):** Unchanged wake gate (`q_0.10` / rule of three),
  plus IPS `w_ips = min(M_ips, 1/ρ)` weighting on sampled judged silences.

### 9. Thinking, Self-Card, Taint & Anchors (§4, §11, §14; A-01, A-10, A-11, R-11..15, R-35, R-43, R-48..49)

- **Acts (A-01, A-11, R-12):** `think(thought, motivation 1–5, confidence_0_20, used_ids[],
  importance?, basis?, value?, cost?, domain?)`, `act(..., expected?)`, `expect(kind, about_ids,
  places, people, condition, due, expiry, recurrence?, promise_meta?)`, `resolve(id, outcome)`,
  `revise(target_id, new_version)` + typed `consolidate` acts (`gist`, `lesson`, `procedure` with
  `required_cap_ids`, `capability_schema_hash`). Strict nullable schemas (`R | null`).
- **Calibrated Self-Report & Citation Lock (A-10, R-15):** Reasoning reports integer `[0, 20]`
  mapped to `c_raw = k/20` and domain-calibrated (`P_rc` guarded `0/0 -> certainty_bar`; explicit
  auto-recall query; emits `withheld` event with raw & calibrated numbers). `CitationLock` rejects
  uncited or hallucinated `used_ids[]`.
- **`ContextPlan`, Progressive `Tool RAG`, Self-Card, Derived `[SELF_GUIDE]`, Spotlighting & Anchor Checks (R-11, R-35, R-38, R-43, R-48, R-49):**
  Stable-to-volatile slot order with **`IdentityAnchor` (`OwnerSigned`) in slot 0** (`[ANCHOR]`), `<= 4`
  static core tools + typed **Derived Self-Guide (`[SELF_GUIDE]`, optional behind `Switches.self_mechanics_guide`, neutral `false`)**
  followed by hybrid BM25/`256d` progressive disclosure of top-`k_cap = 8` capabilities and
  top-`k_proc = 3` active procedures filtered by `ActiveCaps ∩ VisibleScope` & `capability_schema_hash`
  (neutral switch `k_cap = |C_avail|, k_proc = 0`, `R-38`), compact ledgers + read-only **4-pillar
  `self_card`** (`(α_c, β_c)`+latency, `[0,20]` Brier/ECE/`meta-d'`, headroom, claim contradictions),
  commitments, Spotlighting delimiters isolating `[UNTRUSTED_QUOTED]` from `[TRUSTED]`/`[SYSTEM]`.
  For `[SELF_GUIDE]`, `Core` stays content-blind and Valve-unaware: it emits only a typed
  `CoreSelfSnapshot` (active `Switches`/registry flags, `VisibleScope`, `auto_recall` trigger state,
  protocol version) and `Doors::context` allows `Valve` (`waist-valve-and-bodies`) to attach an
  optional `ValveSelfSnapshot` (`valve_mode`, `quiet_hours`, `PendingApproval`, `C_daily`, `taint`
  gating). `reason/src/render.rs` compiles both snapshots via exhaustive Rust `match` templates into
  a `~150–250`-token cacheable prefix block (zero prose stored inside `Core`, zero hidden formulas or
  thresholds such as `θ` or `P_rc` formulas exposed, byte-identical across turns while config holds),
  paired with single-line just-in-time working-set cues when `auto_recall` runs or `Valve` returns
  `held`/`refused` with a canonical reason tag. Pre-act **`[ANCHOR_CHECK]`**, **`PrincipleAudit`**
  (`R-48`), and **`CommitCheck`** (`R-49`) validate identity adherence & promise non-conflict before
  external dispatch.

### 10. Beliefs & Taint Provenance (§7, §14; A-01, A-12, R-17, R-20, R-21, R-32, R-34, R-43)

- **Basis & Provenance (A-01, R-17, R-43):** `told` items initialize via source `(α₀, β₀)` (`A-01`
  fixes L56 vs L86); verbatim-vs-gist conflict verification runs in `reason` (`consolidate`), not
  inside blind `core`; writes enforce skeptical corroboration (`k_corrob`) & min-trust taint
  propagation (`min(trust)` on compaction so non-owners never launder trusted lessons).
- **Domains & `hard_forget` (A-12, R-20, R-34):** Domains map via Hungarian-aligned centroids (`K`)
  or explicit `domain?`. Peer conventions (`R-32`) never override safety doors or direct observations.
  `hard_forget` atomically recomputes all surviving `(α, β)` counts cleanly (`R-34`).

### 11. Sleep, Consolidation & Procedures (§12, §21, §25; A-04..06, A-11, A-12, R-22..24, R-38, R-40)

- **Async Trigger & 3-State Lifecycle (A-01, A-04, A-05):** Runs asynchronously on immutable
  `t_snapshot` when `unconsolidated ≥ N_min` (not `>`) and global idle `≥ T_idle`, with zero delay
  on live events (`Δθ_sleep = 0`). Episodes transition **`{open, provisional, sealed}`** so pre-sleep
  tails consolidate cleanly without spin loops and reopen (`superseded_by`) on seamless resumption.
- **Normalized Boundaries, Merge, Clusters & Fade (A-01, A-06, A-12):** `PE_time`, `PE_content`, and
  novelty are normalized to `[0, 1]` (`EM-LLM`, shrunk cold start); merging checks `places`, `J`,
  and time intervals (`R-24`). Sleep warm-starts spherical k-means and applies **Hungarian matching**
  with split/merge Beta fusion (`domains_updated`, `A-12`), and fades vectors on `B < −3.29` (`A-01`).
- **Contrastive Lessons & Retireable Procedures (A-11, R-32, R-38):** Groups shared opaque error
  signatures with contrastive successes for `lesson`s; traces cross-place `goal_id`/`caused_by`
  chains into slot-parameterized `procedure`s tagged with `capability_schema_hash`, CUSUM drift
  detection, retirement (never silent deletion), and **embodied-only** habit bypass (`allow_chat_habit_bypass = false`).

### 12. Ledger & Sign-Corrected Bounded Learning (§18, §8; A-01, A-13, R-19, R-25, R-26, R-35, R-36)

- **Ledger & 4-Pillar `self_card` (A-01, R-25, R-35):** Compact per-actor cards (`tick` wakes
  include promisees alongside trigger audience) plus event-derived 4-pillar `self_card` (`R-35`)
  supporting deterministic confidence recalibration at external doors.
- **Sign-Corrected & Preemption-Guarded Delta Rule (A-01, R-19, R-26, R-36):**
  Target **`r = 1`** for `{good_act, missed_silence}` and **`r = 0`** for `{bad_act, answered_silence}`
  (fixing the `R-19+R-26` sign inversion), scaled by Gateway `teacher_weight` and clipped IPS
  `min(M_ips, 1/ρ)`. **Critical Guard:** `canceled_by_core` and `preempted_external` receipts (`R-36`,
  `R-37`) NEVER count as failures in `(α_c, β_c)` or weight updates. `wake_gate_lo` uses shrunk `q_0.10`.

### 13. Structural Privacy, Erasure & Trust Doors Inside Core (R-33, R-34, R-43, R-48)

Replaces the provisional departure from `ARCHITECTURE.md`: even inside standalone Lab runs, `Core`
enforces pre-Step-5 **`VisibleScope`** projection (`Δ_NI = 0` via `DeclassToken` isolation, `R-33`),
atomic **`hard_forget`** (`SQLite secure_delete` + FTS5 + `Two-Epoch Replay`, `R-34`), monotonic
`origin_actor` & min-trust taint propagation + `ContextPlan` Spotlighting (`R-43`), and
**`OwnerSigned`** identity locks (`R-48`). Runtime multi-actor `cedar-policy` bundles and `H_act`
interactive approvals live cleanly across Waist 1 in sibling change `waist-valve-and-bodies`.

### 14. `reason`: Unified `EmbeddingGemma 2` & Strictly Typed Adaptation (A-10, A-13, A-15, R-37)

Blocking HTTP (`ureq`) or local ONNX (`ort`) / `LiteRT` / stdio-HTTP sidecar runner behind one embedder trait; `openai-compatible` chat/tools with strict
nullable JSON schemas (`R | null`, `A-10`) and `[0, 20]` anchored confidence prompts.
- **Default Gateway Embedder (`EmbeddingGemma 2`, `256d` MRL, Apache 2.0, `arXiv:2610.37364`):**
  Maps text, images, and `audio_voice` into one `256d` vector space with mandatory task instruction
  prefixes (`t_clf`, `t_sts`, `t_q`/`t_d`), tagged by `(embed_model_ver, dim_d, prompt_regime_id)`
  so incompatible v1 (`EmbeddingGemma-300M`, kept for Lab ablation) and v2 vectors never mix.
- **`A-14` Contrastive Head & Blind `AttachmentHandle`s:** Runs `dialogue_act_gap` at `O(d)` and
  resolves permitted `AttachmentHandle`s into multimodal provider requests strictly *after*
  `VisibleScope` & `Valve` checks (`R-37`).

### 15. Lab: Full 22-Row Suite, External Baselines (`Mem0`, `Letta`, ...), Pareto, `criterion` & Core TCK (A-02, A-15, A-16, R-39, R-40, R-50)

Discrete-event virtual-time simulator driving real `Core::step` via `TestBody` (routed over canonical
Waist-2 schema shapes, `R-50`), plus `monoagens-lab` CLI (`run`, `compare`, `coverage`, `calibrate`,
`mechanisms`, `baseline`, `benchmark`, `pareto`, `tck-core`, `log`, `explain`, `replay`, `talk`):
- **All 22 Brain-Map & Safety Metrics (`A-02`, `specs/lab/spec.md`):** Enforces complete per-row metrics (floor
  violations/100, wakes/event, calls/tokens by purpose, promises kept/kept-late/broken/repair,
  ECE/Brier, detail-without-R, stale-belief leak, self-echo reaction rate, boundary F1,
  sleep-spin count = `0`, barge-in latency, and `Δ_NI = 0`).
- **External `BaselineAdapter` Roster & Fair-Play Contract (`R-39`):** Benchmarks `Stateless ReAct`,
  `Full-Transcript Arm F`, **`Mem0`**, **`Letta/MemGPT`**, **`LangGraph + LangMem`**,
  **`Zep/Graphiti`**, and **`Hermes Agent`** (plus pluggable harnesses) under injected `sim_time_rfc3339`
  clock shims, `PerceptView` oracle-stripping, **`NOOP`/`[SILENT]` token & latency parity** (`wake_invoked_llm`),
  unified 5-tuple `E_tok`/`C_share`, `restart_epoch` RAM clears, and `SEALED_MODE=1` isolation.
- **5-Domain + Scale/Security Matrix & Convex Pareto (`A-15`, `A-16`, `R-39`, `R-40`):** Evaluates
  Memory (`LoCoMo` incl. Cat-5, `LongMemEval`, `MemoryAgentBench`, `GroupMemBench`, `STALE`),
  Prospective (`PM-Bench`, `PIS`), Timing (`PITCH`, `ProACT`, `ProactiveBench`, `CAPA`, synthetic
  `When2Speak` FIR/MIR stress), Group (`Ubuntu IRC`, `Molweni` — banning dyadic `MuTual` as multi-party),
  ToM (`FANToM ALL*`, `OpenToM`, `HI-TOM`, `ExploreToM`/`DynToM`), Indonesian `MultiLexNorm`/`IndoCollex`,
  `Tool RAG` scaling (`N_caps = 50..200`), and `MPBench` poisoning on **Convex Pareto** `(Quality, E_tok, Risk)`
  fronts with `Holm-Bonferroni` (`d_z`, `r_rb`), sizing paired confirmatory trials at **$S \ge 354\text{--}471$** (`A-16`).
- **Property Tests & Scale Microbenchmarks (`proptest`, `criterion`, `R-40`):** Enforces `proptest`
  fuzzing for replay determinism, `κ(b, a)`, and `Δ_NI = 0`, exhaustiveness/zero-drift parity tests
  for `[SELF_GUIDE]` against `Switches` & `ValveSelfSnapshot`, plus `criterion` microbenchmarks on
  `Core::open` (`< 100 ms` across `10^5–10^7` events), `VectorIndex` (`usearch` vs `hnsw_rs` vs `Flat`,
  `< 5 ms` p99 & `Recall@10 >= 0.93` down to `1%` `VisibleScope` selectivity), and forensic `hard_forget` scans.
- **Core TCK Hooks (`R-50`):** Exposes black-box `C-MUST-01` (`Δ_NI = 0`), `C-MUST-02` (`hard_forget`
  0-byte/0-backflow), `C-MUST-03` (bit-exact determinism), and `C-MUST-04` (10k-tick neutral soak).

### 16. Tests in layers, with time budgets

| Layer | What | Budget |
|---|---|---|
| core unit | in-memory store, no IO (`monoagens-core`) | under 3 s |
| store, reason | in-memory SQLite, mock HTTP server | under 10 s |
| Lab scripted (dev) | development set, virtual time, one seed | under 30 s |
| Lab scripted (full) | development and validation sets, virtual time, one seed | under 60 s |
| Lab ablation | five seeds per side | on request |
| Lab live | real model | on request, never in `cargo test` |

A budget overrun is a defect. The `feeling_of_knowing` probe is benchmarked at 10⁵ events.

### 17. Four interleaved phases, each closed by evidence (R-50)

Aligned with Run-2's 4-phase standardization schedule across `core-and-lab` + `waist-valve-and-bodies`:
- **Phase 0 (Waist Freeze):** Finalize `monoagens.core.v1` and `monoagens.body.v1`; route `TestBody`
  through canonical Waist-2 JSON-RPC codecs (`0` shortcut structs).
- **Phase 1 (Minimum Viable Loop + Foundation):** Loop, log, base gate, recall, promises/goals,
  thinking, SQLite+`usearch` store, `reason`, `Lab`, paired with minimal `Gateway`/`Valve` +
  `body-cli` + `body-mcp-bridge` + `monoagens-conformance v0.1`. Closed by foundation scenarios,
  Core/Body TCK `MUST`, and `BaselineAdapter` transcript comparison.
- **Phase 2 (Cognitive Depth Behind Frozen Waists):** All 22 brain-map mechanisms (`R-01`–`R-49`)
  landed & ablated (`compare`, 5+ seeds), Indonesian/`MultiLexNorm` evaluation (`A-15`), paired
  `S >= 354–471` NI check (`A-16`), and Pareto live runs (`R-39`) with `0` breaking Waist-2 changes.
- **Phase 3 (Cross-Body & Multi-Core Matrix):** Multimodal/embodied verification + 2nd Core interop.

A row whose ablation changes nothing beyond seed noise is listed for the owner; removing it is
the owner's decision.

### 18. The brain map (A-01, R-01)

The closed list of the core's cognitive mechanisms behind Waist 1. Every row has an exact reachable
neutral value (`w_sc = 0`, `a = 0`, `w_B = 0`, etc.) and an explicit falsifiable metric in `lab`.

| # | Function | Mechanism | Neutral value | Worse without it | § |
|---|---|---|---|---|---|
| 1 | Salience | weighted signals → S | all wₖ = 0 | when-to-speak recall, missed help | 2, 3 |
| 2 | Floor cost | `social_cost` (R-29 `deg` + `answered`) | w_sc = 0 (reachable) | floor violations per 100 | 2, 15 |
| 3 | Habituation | partitioned `h_{(p,k)}`, post-eval `S/(1+κh_pre)` | κ = 0 | wakes per repeated inconsequential stimulus | 3 |
| 4 | Drive threshold | θ terms a, b, c + opportunity costs (R-31) | a = b = c = 0 (reachable) | rate vs 1/n; budget overruns; overdue promises | 3 |
| 5 | Familiarity reserve | r·(1 − familiarity) | r = 0 | false speaks in new places | 3, 15 |
| 6 | Stochastic go / no-go | seeded σ draw + τ→0 step limit | τ → 0 (`S_eff >= θ`) | distance to human timing | 3 |
| 7 | Commitment breaker | bypass on turn, expectation, goal | off | unanswered allocated turns | 3, 15 |
| 8 | Patience | shrunk quantile `wait` expectation (A-08) | 0 s | double answers / floor collisions | 4 |
| 9 | Report bar on thought | speaking & certainty bars (`withheld` logged) | 1 and 0 | low-value messages | 4 |
| 10 | Activation recall | `w_B·B + context + importance` (A-01) | `w_B = 0` (similarity only) | recall at 1/7/30 days | 5 |
| 11 | Vector fade | release vectors where Petrov `B < -3.29` | threshold −∞ | storage and query latency | 5 |
| 12 | Prediction and grounding | predictions after acts and chosen silences | none | unnoticed non-uptake / missed silences | 6, 17, 20 |
| 13 | Prospective memory | typed promise + `RecurrenceSpec` + `CommitCheck` | left to reasoning | promise keeping & conflict rate | 6 |
| 14 | Credence | beta counts (`told` incl. + `λ_domain`) | frozen | Brier / ECE calibration | 7 |
| 15 | Bounded learning | sign-corrected & IPS-weighted delta rule | `learning_rate` = 0 | learning plateau over weeks | 8 |
| 16 | Memory truth | basis; dual-process R/F; event beats gist; `valid_to` | each off | misattribution; confabulation; lures; stale beliefs | 12, 14 |
| 17 | Report control | `0/0`-guarded P_rc vs `[0,20]` c, grain | P_rc = 0 | wrong volunteered answers | 14 |
| 18 | Consolidation and merge | async `{open,provisional,sealed}` gists; overlap merge | none; never | 30-day recall; spin loops; blended memories | 12, 14 |
| 19 | Perception ledger | promisee-aware cards & 4-pillar `self_card` (R-35) | absent | false-belief errors & self-claim contradictions | 18 |
| 20 | Agency | `(channel, Δ_obs, Δ_pred)` effect predictions | no marking | reactions to own echoes | 19 |
| 21 | Event boundaries | `[0,1]` normalized EM-LLM surprise + shrinkage | fixed interval | boundary F1 | 21 |
| 22 | Backlog chunking | `stopped/down`-only chunking (`Δθ_sleep = 0`, A-04) | per event | bursts after downtime; sleep responsiveness | 15 |

### 19. Open points resolved by Run 2 & remaining Lab calibration

All 16 Open Notes (`A-01`–`A-16`) and Round-2 questions (`R-29`–`R-50`) from `research/researcher-prompt-2.md`
are now folded into `core-and-lab` and sibling `waist-valve-and-bodies`. Remaining items strictly
requiring runtime hardware profiling or empirical Lab sweeps stay explicitly non-blocking and unguessed:

| Remaining empirical point | Status & handling |
|---|---|
| `EmbeddingGemma 2` (`439M`/`577M`/`740M` int4/int8) edge latency + Indonesian/Gaul `256d` vs `512d` curve (`A-15`, `R-37`) | Built with `EmbeddingGemma 2` `256d` + `embed_model_ver` tag; Gateway profiles target CPUs and sweeps `MultiLexNorm`/`IndoCollex` in Lab |
| Indonesian + code-mixed `VAP` (`τ_barge`, `θ_turn_end`) & owner hourly approval cap `B_hr` (`R-37`, `R-42`) | Bounded prior defaults shipped; tuned via Lab calibration (`B_hr` warning-fatigue sweep) |
| Comparative `S >= 354–471` non-inferiority & 5-domain Pareto scoreboard (`A-16`, `R-39`) | `lab/src/baseline.rs` + `pareto.rs` execute on explicit request once core & bodies are built |
| Post-`hard_forget` rollback-replay vs freeze for order-dependent adaptive weights (`R-34`) | `Two-Epoch Replay` freezes pre-forget weights at the epoch snapshot unless explicit full-replay flag is selected |
| Any mechanism whose Run-2 ablation shows no effect beyond seed noise (`R-01`, `A-02`) | Scripted ablation records land in git-ignored `lab/runs/<run-id>/`; the owner reviews them and promotes chosen records to `lab/runs/reference/`, where pruning candidates are flagged. A scripted run alone never removes a mechanism (Decision 20) |

### 20. One prior file, recorded in the log (owner review 2026-11-17)

The 2026-11-17 review found that no code read `prior/dispositions.json`: every Lab command and
the CLI used `Prior::default()`, so run records named a content version the file could no longer
guarantee, and `monoagens replay` could not reproduce a soul started with another prior. Owner
rulings:

- **One source.** `prior/dispositions.json` is the canonical prior. `Prior::default()` stays in
  code (the core does no IO and has no JSON parser) and a test fails the build when it differs
  from the file.
- **Loading outside the core.** Hosts (Lab, gateway, CLI) read and parse the file with one shared
  loader, reject unknown keys and unreadable explicit paths, run `Prior::validate`, and compute
  the digest: `BLAKE3` over the canonical JSON serialization of `Prior`, so formatting does not
  change it. The core never computes the digest.
- **Recorded in the log.** `Core::open` appends `prior_loaded` (full prior content) when the given
  prior differs from the last recorded one. Replay and crash recovery take the prior from these
  events, never from the caller. Learned state is not reset by a new prior. `hard_forget` never
  erases `prior_loaded`, like `Two-Epoch Replay` anchors.
- **Bookkeeping stays neutral.** `prior_loaded` shifts log ids, so two rules keep behaviour
  unchanged: seeded draws key on the event id minus the preceding `prior_loaded` count, and Lab
  scenario event ids are counted from the scenario's own first event (translated by the number of
  events `Core::open` appended). Scenario files, sealed ones included, never need editing.
- **Scripted ablation is a first signal, not evidence.** Scripted runs over the development set
  check the core's side only. Their records go to git-ignored `lab/runs/<run-id>/` with a summary;
  only the owner promotes records to `lab/runs/reference/` or prunes a mechanism.

### 21. Reopened 2026-11-18: live model-call defects

The first live pilot (`lab-memory-comparison` phase 0, 2026-11-18) showed defects every scripted
test missed, because the scripted provider never reads its prompt:

- **Empty classification prompts.** `turn.rs` (~4703, ~4814) builds Classify plans with only
  `trigger_event_id` and `items`; the renderer reads `sections`, so the request says
  `trigger: none` with no event or condition text. The core SHALL fill the trigger section and a
  condition section for Classify plans the way it does for Reason plans.
- **No output limit.** With no limit the model ran to 65,536 output tokens on 121 calls (91 % of
  the pilot's LongMemEval cost). Limits become per purpose and per model profile, default 16,384;
  measured benchmark profile for `deepseek-v4.1-flash` at `high`: classify 1,024, reason 4,096,
  consolidate 8,192 (provisional).
- **Provider-specific code.** `openai.rs` hard-codes `x-opencode-session`, a `session_id` field and
  OpenCode usage structs. Owner rule: no provider is special-cased in code; headers
  (`{conversation_id}` placeholder), the output-limit parameter name and usage field paths come
  from configuration, standard OpenAI-compatible fields by default.
- **Output-format failures** (~113: invalid motivation, places, label, act fields) are diagnosed
  first from the recorded pilot; a retry that feeds the parse error back is an owner decision
  after the diagnosis.
- **Render dry run.** Every live Lab run is preceded by a no-network render of real requests for
  every purpose; an empty trigger or capability list blocks the run.

## Risks / Trade-offs

- [Most formulas are the researcher's inference, not established results] → Every parameter
  is a disposition in the prior with a range; the Lab sweeps them on the development set,
  freezes, checks on validation and reports once on the sealed set (R-03).
- [The change is large: twenty-two functions] → Two stages with evidence gates; every group
  ends in its own passing tests.
- [More model calls: classification for conditions, goals and judged silence; consolidation]
  → Calls are recorded by purpose, the cost guard bounds a run, and the Lab reports tokens
  per event.
- [Scripted model outputs cannot show that a real model uses labels, cards and bars well] →
  Scripted runs prove the core's side; live runs measure the model's side as wording checks.
- [Rules written for chat may not carry to physical/continuous bodies] → Continuous `deg(e, l)`
  (`R-29`), partitioned `h_{(p, k_src)}`, `f_modal` acoustic slots, and Tier-1 filtering (`R-36`,
  `R-37`) generalize cleanly while remaining algebraically neutral in `chat-only`.
- [Private items could bias Step-4/5 urgency even when omitted from prompts (`R-33`)] → Fixed by
  enforcing pre-Step-5 `VisibleScope` projection (`Δ_NI = 0`) and explicit `DeclassToken` logging.
- [Memory growth over multiple years] → Solved by segmented `hot.db` + read-only sealed shards +
  `usearch` HNSW + `postcard`/`minicbor` snapshots (`R-40`) + `B < -3.29` vector fading (`A-01`).
- [A row may show no measurable effect and be removed] → Expected by `ARCHITECTURE.md`; verified across
  22 explicit metrics (`A-02`) and `S >= 354–471` paired power (`A-16`).
- [Deferring real bodies until all 22 rows finish risks late Waist breakage (`R-50`)] → Resolved by
  co-evolving `waist-valve-and-bodies` alongside `core-and-lab` across Phases 0–3.

## Migration Plan

Greenfield. Install Rust stable with `rustup` and pin it; initialize git so the core version
in run records is a commit. Stored data carries format versions from the first event.

## Open Questions

- **License.** Dual MIT / Apache-2.0 is usual for Rust projects aiming at wide adoption;
  needed before a public release, not for this change.
- **Reference model for published results.** Any OpenAI-compatible model runs the live sets;
  which one is quoted can be chosen when results are first published.
- **Sealed scenarios.** The owner writes them; how many and when does not affect the build.
