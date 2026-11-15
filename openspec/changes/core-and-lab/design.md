# Design

## Context

See `proposal.md` for motivation. The repository holds `ARCHITECTURE.md`, `RESEARCH.md`, the
research brief and its first findings, and no code. The owner's decisions so far:

- Rust for everything in this change.
- This change builds the core and the Lab only, with `RESEARCH.md` sections 1–21 and 26.
- Nothing is restricted at the doors. Policy comes later, on top, without changing the core.
- The architect designs and reviews; implementation is delegated to worker agents.
- The architect does not choose where the research is silent. Open points go to the
  researcher (`research/researcher-prompt.md`).
- Research run 1 (`research/findings/2026-11-15-run-1.md`) is accepted as the basis for
  R-01 to R-05 and R-07 to R-28. Every number in it is a start value to tune in the Lab.

Reading the rules below: text tagged **(R-nn)** comes from that finding. Findings separate
what the literature establishes from the researcher's inference; most formulas here are
inference with sourced shape, and are expected to move with Lab results. Where a finding left
something open, this design says so and does not fill it (Decision 19).

Constraints:

- The invariants of `ARCHITECTURE.md` are fixed.
- `RESEARCH.md` uses older terms (*host*, *hooks*, *dispatch door*). This design uses the
  terms of `ARCHITECTURE.md`: gateway, doors, valve.

## Goals / Non-Goals

**Goals:**

- A core that runs `RESEARCH.md` §1–21 as corrected by run 1, with no IO, replayable exactly.
- A Lab that can show, for each row of the brain map, what gets worse without it.
- Tests fast enough to run on every edit (Decision 16).
- Nothing in the core that the later gateway, bodies or policy would force us to change.

**Non-Goals:**

- Wave 2 and 3 (`RESEARCH.md` §22–25), the gateway, the valve, policy (§10), real bodies,
  tools, plugins, the product CLI, hard forgetting, configuration migration.
- Graded address (R-29). In this change an event is addressed to an actor or it is not.
- `no_std` builds, an LLM judge in the Lab, snapshots of projections, approximate vector
  indexes, the human blind test (R-27 gives the protocol; running it needs people).

## Decisions

### 1. The owner's rulings on four contradictions

| Contradiction | Ruling |
|---|---|
| "Owner" appears in the core (`RESEARCH.md` §7, §8) | The core never knows an owner. Whoever runs the soul supplies two numbers per actor: a reliability prior (α₀, β₀) and a teacher weight. |
| A text index in the core (§5) | Text search sits behind the store port. The core passes the id of the query event; the store reads text and returns ids. |
| A reply that answers a prediction is marked self-caused (§17 against §19) | Predictions have two kinds. A matched *effect* prediction is self-caused. An answered *response* prediction wakes the soul. |
| Who owns the hooks (§16 against `ARCHITECTURE.md`) | The core owns the three hook points and calls them inside its turn. What runs in them (the valve, plugin hooks) belongs to the gateway. |

Also ruled: both formula fixes of run 1 are adopted (habituation gain κ; Petrov's hybrid for
base-level activation); explicit recall is a capability used through `act`; `info_gap` takes
its reference set from an annotated corpus and stays only if the Lab shows its effect.

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
    replay.rs       reprocess a log and compare decisions
  store/src/        monoagens-store  (SQLite): schema.rs log.rs text.rs vectors.rs
  reason/src/       monoagens-reason (what the core needs in order to think)
    render.rs tools.rs parse.rs secrets.rs
    model/          scripted.rs openai.rs
    embed/          none.rs hash.rs openai.rs scripted.rs
  lab/src/          monoagens-lab    (library + `monoagens-lab` binary)
    scenario.rs soul_dir.rs driver.rs clock.rs test_body.rs guard.rs checks.rs metrics.rs
    ablation.rs coverage.rs calibrate.rs baseline.rs record.rs probe.rs inspect.rs main.rs
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

### 4. Six books, one log (R-12)

All state is a projection of the log, in six books:

| Book | Holds |
|---|---|
| log | every event, with use times for activation |
| memories | episodes, gists, lessons, procedures, each linked to its source events |
| expectations | effect, response, wait, promise, goal |
| sources | ledger cards, including the soul's own |
| credences | (α, β) counts per belief and per (source, domain) |
| dispositions | knobs, weights, and per-place and per-source offsets |

Event kinds: `percept`, `decision`, `wake_started`, `model_call`, `thought`, `action`,
`receipt`, `expectation_opened`, `expectation_changed`, `verdict`, `memory_used`,
`belief_changed`, `evidence`, `episode_closed`, `sleep_run`, `domains_updated`,
`vector_released`, `timing_updated`, `learning_change`, `rollback`, `door_verdict`,
`identity_changed`, `body_state`, `actor_facts`, `tick`, `failure`.

Every event stores kind, format version, time, actor, place, audience, `origin_actor`,
`caused_by`, numeric features and an opaque payload. A **chain** is the set of events linked
by `caused_by` back to an origin (R-07); `origin_actor` is inherited along it and cannot be
set by a model. A tick is appended only when it fires something.

### 5. What the core is told, as numbers and ids

| Fact | Supplied by | Used for |
|---|---|---|
| event vector, embedder tag | `reason` | signals, recall, wake gate, novelty, misses |
| addressees, `named` | the body | circuit breaker, `own_name`, `social_cost`, exchanges |
| `reply_to`, correlation id | the body | grounding, agency, goals |
| length, negative flag | the body | content miss, learning |
| typical latency; effect channel, window, noise | the capability descriptor | timing, agency |
| condition and expected-reply vectors | `reason`, from the text of `expect` and `act` | wake gate, goals, misses |
| reference vectors for `info_gap` | `reason` | `info_gap` |
| reliability prior (α₀, β₀), teacher weight | whoever runs the soul, per actor | reliability, learning |
| daily budget, knowledge cutoff | whoever runs the soul | budget drive, time grounding |

### 6. Gate (§2–4, §15; R-02, R-03, R-04, R-14)

- **Signals:** `own_name`, `info_gap`, `feeling_of_knowing` and `relevance` as in §2.
- **`social_cost` (R-02):**
  `max(sel_other, answered) · (1 − λ_c · closeness(sender))`, where `sel_other` = 1 if the
  event's addressees are non-empty and exclude the soul, and `answered` = 1 if an actor other
  than the sender and the soul has an event in that place after it.
- **Salience:** `S = clamp(Σ wₖ·sₖ, 0, 1)` with `w_info = 0.6·helpfulness`,
  `w_fok = 0.4·helpfulness`, `w_sc = −(0.2 + 0.6·restraint)` (R-03).
- **Habituation (R-01):** `h ← h·γ^Δt + 1`, `S_eff = S / (1 + κ·h)`. κ = 0 is neutral.
- **Threshold:** as §3, with `a = 0.1 + 0.5·restraint` (R-03).
- **Closeness and familiarity (R-04):** *presence* in a place (an event there with the soul in
  its audience, or the soul's own act there) feeds place familiarity; an *exchange* with a
  source (its event addressed to the soul, the soul's act addressed to it, or a reply-linked
  pair) feeds closeness. `closeness(p) = σ(B_p)`, 0 with no exchange.
  `B_ctx = (B_P + Σ_{p∈Present} B_p) / (1 + |Present|)`, `familiarity = σ(B_ctx)`, 0 with no
  history.
- **Base level (R-04), used everywhere:** Petrov's hybrid,
  `B ≈ ln[Σ_{j=1..k} t_j^(−d) + (n−k)(t_n^(1−d) − t_k^(1−d)) / ((1−d)(t_n − t_k))]`, with
  `t_j` the ages in hours of the k most recent uses, `t_n` the age of the first use, n the use
  count, and t floored at one minute.
- **Draw and circuit breaker:** as §3. The breaker fires for an allocated turn, an event that
  fires or answers an expectation, and an event attributed to an active goal (Decision 8).
- **Patience (§4):** a go on an unallocated turn opens a `wait` for the place's patience time
  (Decision 8). Another actor's reply closes it as silent.
- **Backlog (R-14):** an event is backlog if its ingest lag exceeds the place's patience, or it
  arrived while the soul was stopped or asleep. A place's backlog is one chunk with
  `s_k = max_i s_k(e_i)·(1 + L_i)^(−d)`. Any commitment in it fires the breaker. At most four
  chunks are handled per wake, most salient first.

### 7. Memory (§5, §14; R-05, R-16)

- Candidates: top 40 by vector, top 20 by text. Rank by
  `A = B + context_weight·cos + importance_weight·importance`.
- **Importance (R-05):** `max(i_reason, i_commit, i_surprise)`. `i_reason` is given by
  reasoning when it writes the memory; `i_commit` = 1 when the memory is linked to a promise,
  goal or expectation; `i_surprise = clamp(z / z_max, 0, 1)` with z the surprise z-score at
  encoding (Decision 11).
- **Explicit recall** is the core's own capability `memory.recall`, used through `act`. It
  passes the action door and its receipt (ids only) continues the chain.
- **Recollection (R-16):** R holds iff recall returns at least one log event, not a gist,
  whose actor or audience contains the people, whose place matches and whose time falls
  within the grain claimed. A gist is tested through its source ids, so faded vectors do not
  matter. `F = max(cosine, normalized activation)`; F alone marks an item familiar only.
- Superseded items rank below current ones and carry `valid_to` and `superseded_by`.

### 8. Expectations (§6, §17, §19, §20; R-07, R-08, R-09, R-10, R-18, R-19)

- **Kinds:** effect, response, wait, promise, goal. **Conditions:** due time, reply from a
  source, free text.
- **Timing per place (R-08):** `patience(P)` is the `q_p` quantile of the latency to the first
  third-party reply; `expiry(P)` is the `q_e` quantile of the latency from an act to the
  addressee's next event, times a stakes factor; the silence window uses the same rule for
  anyone's reply. Each is shrunk toward the body's declared latency with `n₀`
  pseudo-observations. Answering evidence is a `done` receipt, the addressee's next event in
  the place, or a continued-attention number the body reports.
- **Agency (R-09):** an event matches an effect prediction when it carries the act's
  correlation id or its actor is the soul itself, or when it is on the declared channel
  within `[t + lat_min, t + lat_max]` and `|Δ_obs − Δ_pred| ≤ k·σ_channel`. A match is
  recorded with basis `self` and is not appraised. An unmet effect prediction is an error.
- **Promises (R-10):** states pending → due → attempted → kept | failed → retry | overdue |
  renegotiated | cancelled. A failed, refused, held or unknown receipt is not completion; the
  promise stays open, counts in promise debt, and is retried at `t + b·2^(i−1)` for
  `i ≤ n_max`. After that, or when a retry cannot finish in time, the breaker wakes reasoning
  to inform or renegotiate. At start-up, promises with due < now are marked overdue and
  reasoning wakes once per place. "Kept late" is recorded apart from "kept".
- **Goals (R-07):** a goal is an expectation of kind `goal`, opened only by reasoning. It holds
  its chain, places, people, a completion condition (description and vector), a due time and
  an expiry. An incoming event is attributed in this order: (1) a receipt or reply-link to the
  goal's chain: attributed, breaker fires; (2) a match to the goal's open prediction:
  attributed, breaker fires; (3) cosine to the condition ≥ `wake_gate_lo` and one positive
  classification: attributed, no breaker, relevance raised; (4) otherwise it belongs to the
  main thread. The events attributed to a goal are its task thread.
- **Wake gate (§6):** `m = cos(event, condition)`; below `wake_gate_lo` ignore; otherwise one
  `classify` call recorded as a `verdict` with m. `wake_gate_hi` stays 1 until the rule of
  three allows a lower value (n = 50 at ε = 0.06).
- **Content miss (R-18):** the response prediction stores the act's vector and, optionally,
  the vector of a reply reasoning said it expected.
  `miss = 1 − max(cos(reply, act), cos(reply, expected))`. It is flagged when its z-score
  against the place's unrepaired replies exceeds `z_miss`. The event records who replied, the
  latency share, the length and the distance; reasoning judges the kind of miss.
- **Judged silence (R-19):** every `note` decision, and a seeded fraction ρ of `silent`
  decisions, opens a prediction "someone else responds to the sender within expiry". It is
  missed when no other actor's event is reply-linked or addressed to the sender in that time,
  or when the sender posts again with cosine to the original ≥ `wake_gate_lo` and one
  classification confirms it. A confirmed miss feeds learning.

### 9. Thinking (§4, §11, §14; R-11, R-12, R-15)

**Thought acts (R-12):**

- `think(thought, motivation 1–5, confidence, used_ids[], importance?, basis?, value?, cost?)`
- `act(capability, place, audience, args, expected?)`, which opens the predictions of
  Decision 8
- `expect(kind, about_ids, condition, due, expiry)`
- `resolve(id, outcome ∈ {kept, failed, cancelled, answered})`
- `revise(target_id, new_version)`: may supersede memories, belief statements, expectation
  conditions and times, and claims on the self card. It may not change log events, the
  identity, credence counts or dispositions.

`basis`, `value`, `cost` and `expected` are the fields that R-17, R-15 and R-18 need reasoning
to supply.

**Context (R-11).** The core builds a `ContextPlan`; `reason` renders it. Order, stable to
volatile: identity; capabilities (sorted); cards of the people present and the self card;
commitments and the active goal; the chain's working set (the soul's own thoughts and acts in
full, the last M observations in full, older ones as id placeholders); the top-k recalled
memories with basis and time; the last N events of the place; `<now>` with the model's
knowledge cutoff; the trigger. The plan passes the context door before rendering.

Each wake is one fresh request with the five acts offered as tools. Tool results are never
returned inside the same conversation: a receipt is an event and, through the circuit
breaker, causes the next wake with a rebuilt context.

- **Bars (§4):** on an unallocated turn a communicative act passes only if
  `motivation ≥ speaking_bar` and `confidence ≥ certainty_bar`.
- **Report control (R-15):** `P_rc = max(certainty_bar, cost / (value + cost))`. If
  `confidence < P_rc`: run `memory.recall` and wake again, up to `n_recall` times; then the
  plan asks for a coarser grain (exact time → day → period; name → "someone in the group";
  number → range); an answer still below `P_rc` is withheld.
- **Purposes:** `reason` (all five acts), `consolidate` (think and revise only), `classify`
  (verdict only).

### 10. Beliefs (§7, §14; R-17, R-20, R-21)

- **Basis (R-17)** belongs to beliefs and memories, not to events. The event "X said P" is
  observed; P is told by X. Reasoning assigns the basis in `think`; the core downgrades
  `observed` to `inferred` when the belief's links do not reach percepts the soul witnessed.
  Sensor readings are observed; another agent's message is told.
- **Validity:** `revise` and consolidation write `belief_changed` with `valid_to` and
  `superseded_by`. Nothing is rewritten. Times are stored absolute; relative wording is
  produced when rendering, against `<now>`.
- **Credence (R-20):** (α, β) per belief. An outcome credits only the items in the `used_ids`
  of the think that made the decision. Once per period the counts relax toward the prior:
  `α ← α₀ + (α − α₀)·λ^Δ`, likewise β.
- **Reliability (R-21):** (α, β) per (actor, domain), starting from the actor's supplied prior
  or Beta(1, 1). A domain is an opaque id: the nearest of K vector centroids recomputed during
  sleep and recorded as `domains_updated`, or an id supplied from outside or by reasoning.

### 11. Sleep (§12, §21, §14; R-22, R-23, R-24)

- **Trigger (R-22):** no event anywhere for `T_idle` and at least `N_min` unconsolidated
  events. Consolidation runs as background work.
- **Boundaries (R-23):** per place, `s_t = max(PE_time, PE_content, novelty)` with
  `novelty = 1 − cos(event, centroid of the open episode)`. A boundary falls where
  `s_t > μ_t + γ·σ_t` over the last w events. Without vectors: participant-set Jaccard < 0.5,
  a gap above the place's 95th percentile, or a prediction miss.
- **Products (R-22),** each linked to its source ids: a **gist** (a summary of one episode
  written by reasoning, basis inferred); a **lesson** (a situation, action and outcome,
  written when two or more episodes share a prediction error, with credence); a **procedure**
  (an action sequence that reached a goal, with credence, recallable as a skill).
- **Merging (R-24):** same place, participant Jaccard ≥ J, and time intervals that overlap or
  fall within the place's gap threshold. Vector similarity alone never merges.
- **Fading (§5):** events with `A < fade_threshold` get `vector_released`.
- **Domains (R-21):** the K centroids are recomputed during sleep.
- Progress is the `episode_closed` and `sleep_run` events, so a stopped run resumes without
  repeating an episode.

### 12. Ledger and learning (§18, §8; R-25, R-26)

**Ledger (R-25).** A card holds: what the source witnessed (event ids, last-seen time per
place); what the soul told it and whether that was grounded; its open requests; and what
changed since it last witnessed (candidate false beliefs). "Witnessed" is the audience fact
the body reports. One compact card per person present, with second-order facts only for the
soul's own claims; at most `ledger_items` entries per field. The self card holds what the soul
said, promised and claimed, to whom, and what its bodies can do now.

**Learning (R-26).** Each kind has its own switch and is off unless switched on.

- *Outcome:* r = 1 when a response prediction was answered in time with no miss or repair, or
  an effect prediction matched; r = 0 on absence, a miss, a repair, a failed receipt, a
  negative flag from the body, or a missed judged silence.
- *What learns from what:* salience weights and threshold terms from outcomes; timing from
  latencies (Decision 8); the wake band from classifier verdicts.
- *Weights:* `w ← clamp(w + learning_rate·(r − P)·s)`, scaled by the teacher weight of the
  actor involved.
- *Bounded updates (§8):* move only when the 95% interval excludes "no change"; each step a
  small share of the range; at most 5% of the range per day; floor, ceiling, freeze and
  report.
- *Scoped values:* per-place and per-source offsets keyed by opaque ids, learned by the same
  bounded rule from everyone's outcomes.
- *Band:* `wake_gate_lo` moves toward the lowest positively classified m minus `margin`,
  inside its bounds.
- Every change is a `learning_change` event with its delta; `rollback` records the reversing
  deltas.

### 13. Doors without a valve

The three doors are hook points in the core's turn, and `origin_actor`, `caused_by` and
audiences are recorded on every event. No rule ships in the core. The Lab installs one rule at
the context door, the cost guard (run token limit, chain wake limit).

**Departure from `ARCHITECTURE.md`:** the two absolute lines are not built in this change. The
document is left unchanged; the lines arrive with the policy change (R-33). The Lab reports
how many items from a narrower audience reach a wider context, as the baseline that change
must beat.

### 14. `reason`: providers without a runtime (R-13)

Blocking HTTP (`ureq`), no async runtime. `openai-compatible` speaks chat completions with
tools; `scripted` returns prepared outputs. Embedders: `none`, `hash`, `openai-compatible`,
`scripted` (maps a scenario's concept labels to vectors).

The reference embedding model is EmbeddingGemma-300M, truncated to 256 dimensions and
re-normalized in `reason` (R-13). Vectors are stored with their events and tagged with model,
dimension and quantization; vectors with different tags are never compared, and the tag is in
every run record. The `hash` embedder is the lexical fallback; it lives in `reason`, never in
the core.

Secrets come from `MONOAGENS_MODEL_BASE_URL`, `MONOAGENS_MODEL_API_KEY`,
`MONOAGENS_EMBEDDING_BASE_URL`, `MONOAGENS_EMBEDDING_API_KEY` or a `secrets.toml` with
owner-only permissions.

### 15. Lab: a discrete-event simulation of the world

The driver keeps a queue ordered by virtual time: scripted steps, body receipts and follow-up
percepts, model results (scheduled at request time plus a scripted or measured latency) and
`next_due()`. It pops one item, calls `Core::step`, and schedules the effects. Single thread,
no real waiting.

- **Scenario (TOML):** soul (identity, seed, actor facts, limits), test bodies (places,
  actors, capabilities with latency and effect declarations, scripted receipts and follow-up
  percepts), timed steps with their reportable facts and concept labels, scripted model
  outputs keyed by trigger and purpose, SPEAK / SILENT annotations (R-27), tagged checks.
- **Soul directory:** `soul.toml` and `log.db`.
- **Commands** (`monoagens-lab`): `run`, `compare`, `coverage`, `calibrate`, `mechanisms`,
  `log`, `explain`, `replay`, `talk`.
- **Ablation (R-01):** each side runs with at least five seeds; every metric is reported with
  a confidence interval and the cost. Rows 6, 11 and 22 of the brain map are tested first.
- **Calibration (R-03):** `calibrate` computes, on the development set only and per embedder:
  `fok_threshold` (median of A*), `fok_scale` (IQR(A*)/2.7), `wake_gate_lo` (95th percentile
  of cosine over unrelated pairs) and its bounds (80th and 99.5th percentiles).
- **Transcript baseline (R-11):** `baseline` runs a scenario with the context replaced by the
  full transcript, including a restart in mid-task, and reports success, tokens and cache
  share for both.
- **Run record:** one JSON file per run in `lab/runs/`.
- **Sealed set:** loaded, reported in aggregate only; the directory ships empty with a note.

### 16. Tests in layers, with time budgets

| Layer | What | Budget |
|---|---|---|
| core unit | in-memory store, no IO | under 5 s |
| store, reason | in-memory SQLite, mock HTTP server | under 10 s |
| Lab scripted | development and validation sets, virtual time, one seed | under 60 s |
| Lab ablation | five seeds per side | on request |
| Lab live | real model | on request, never in `cargo test` |

A budget overrun is a defect. The `feeling_of_knowing` probe is benchmarked at 10⁵ events.

### 17. Two stages, each closed by evidence

- **Stage A, foundation:** loop, log, base gate, recall, base expectations including promises
  and goals, thinking, store, `reason`, Lab. Closed by the foundation scenarios passing
  scripted and live, an ablation report and the transcript comparison.
- **Stage B, wave 1:** each mechanism lands with its row of the brain map, its development
  scenario and an ablation difference. Closed by a second ablation report and live run.

A row whose ablation changes nothing beyond seed noise is listed for the owner; removing it is
the owner's decision.

### 18. The brain map (R-01)

The closed list of the core's functions for §1–21. A row is one studied brain function,
implemented by one mechanism that reuses existing primitives, with a neutral value reachable
by a parameter and a claim an ablation can falsify. Engineering concerns (§9–11, §13, §16)
get no row. The registry in `registry.rs` holds exactly these rows; a row without a claim does
not compile.

| # | Function | Mechanism | Neutral value | Worse without it | § |
|---|---|---|---|---|---|
| 1 | Salience | weighted signals → S | all wₖ = 0 | when-to-speak recall, missed help | 2, 3 |
| 2 | Floor cost | `social_cost` | w_sc = 0 | taking others' floors | 2, 15 |
| 3 | Habituation | S / (1 + κh) | κ = 0 | wakes per repeated inconsequential stimulus | 3 |
| 4 | Drive threshold | θ terms a, b, c | a = b = c = 0 | rate against 1/n; budget overruns; overdue promises | 3 |
| 5 | Familiarity reserve | r·(1 − familiarity) | r = 0 | false speaks in new places | 3, 15 |
| 6 | Stochastic go / no-go | seeded σ draw | τ → 0 | distance to human timing | 3 |
| 7 | Commitment breaker | bypass on turn, expectation, goal | off | unanswered allocated turns | 3, 15 |
| 8 | Patience | `wait` expectation | 0 s | double answers | 4 |
| 9 | Report bar on thought | speaking and certainty bars | 1 and 0 | low-value messages | 4 |
| 10 | Activation recall | B + context + importance | similarity only | recall at 7 and 30 days | 5 |
| 11 | Vector fade | release low-A vectors | threshold −∞ | storage and latency | 5 |
| 12 | Prediction and grounding | predictions after acts and chosen silences | none | unnoticed non-uptake | 6, 17, 20 |
| 13 | Prospective memory | typed promise lifecycle | left to reasoning | promise keeping | 6 |
| 14 | Credence | beta counts | frozen | calibration | 7 |
| 15 | Bounded learning | delta rule | `learning_rate` = 0 | no improvement over weeks | 8 |
| 16 | Memory truth | basis; recollection test; event beats gist; `valid_to` | each off | misattribution; confabulation; lures; stale beliefs | 12, 14 |
| 17 | Report control | P_rc against confidence, grain | P_rc = 0 | wrong volunteered answers | 14 |
| 18 | Consolidation and merge | idle gists; merge on people, place, time | none; never | 30-day recall; blended memories | 12, 14 |
| 19 | Perception ledger | per-source cards and self card | absent | false-belief errors | 18 |
| 20 | Agency | effect predictions | no marking | reactions to own echoes | 19 |
| 21 | Event boundaries | surprise spike | fixed interval | boundary F1 | 21 |
| 22 | Backlog chunking | one appraisal per place | per event | bursts after downtime | 15 |

Run 1 expects rows 6, 11 and 22 to be the likeliest to show no effect.

### 19. What is still open

`research/backlog.md` tracks each question's status. Everything below is in research brief 2
(`research/researcher-prompt-2.md`); its findings enter the changes after this one. After run 1:

| Open point | Effect on this change |
|---|---|
| R-06: which annotated corpus supplies the `info_gap` references, and whether one set serves all languages | `info_gap` is built with no reference set and `w_info` = 0; the set arrives with the next change |
| Open notes inside accepted findings (for example: averaging or minimum in R-04; max or noisy-OR in R-14; whether the act vector stands in for the expected reply in R-18; λ per domain in R-20; K in R-21; lesson thresholds in R-22; γ per medium in R-23; J in R-24; teacher weights in R-26) | built as run 1 states; each is a disposition to sweep in the Lab, not a decision of this design |
| With κ = 1, `S_eff = S / (1 + κ·h)` halves even a first stimulus (h = 1), so `S_eff` ≤ 0.5 while θ₀ ranges from 0.35 to 0.85. Run 1 gives κ a neutral value but does not say whether a first stimulus should be reduced | built as run 1 states; raised with the researcher, because at default dispositions an unaddressed event may rarely reach the threshold |
| Whether sleep closes an episode that has no boundary yet | not stated by R-22 or R-23; consolidation covers closed episodes only until a finding says otherwise |
| How reasoning should estimate value and cost for report control (R-15) | the fields exist; their use is measured live |
| Four citation details (R-28) and the corrections to `RESEARCH.md` listed in run 1 | documents only |
| R-29 to R-34 partly answered; R-32, R-35 to R-38, R-40 not reached | block nothing here; needed for the changes that follow |

Nothing in this table is to be settled from a worker's own judgement.

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
- [Rules written for chat may not carry to other bodies: `social_cost`, exchanges] → Run 1
  flags them as chat-centred until graded address (R-29); the device scenario keeps a
  non-chat body in every run.
- [Nothing but cost is restricted, and gate signals read private memories (R-33)] → Lab only;
  stated in the proposal and the README; the leak baseline is measured.
- [A row may show no measurable effect and be removed] → Expected by the architecture.
- [Projections and vectors live in memory] → Fine to about 10⁵ events; question R-40.
- [The researcher could not read this repository] → Run 1 was written without Decision 17 of
  the earlier draft; this design adopts its map as given rather than merging the two.

## Migration Plan

Greenfield. Install Rust stable with `rustup` and pin it; initialize git so the core version
in run records is a commit. Stored data carries format versions from the first event.

## Open Questions

- **License.** Dual MIT / Apache-2.0 is usual for Rust projects aiming at wide adoption;
  needed before a public release, not for this change.
- **Reference model for published results.** Any OpenAI-compatible model runs the live sets;
  which one is quoted can be chosen when results are first published.
- **Sealed scenarios.** The owner writes them; how many and when does not affect the build.
