# Core — Decisions (rulings)

Status: **FINAL for v1 — frozen** (2026-11-11). Every point is agreed between the owner and the
architect and carries the decision numbers it came from (Dxx); the discussion history of D1–D41
is in git (commit 32248b1 and earlier). If DESIGN.md, MATH.md, FORMATS.md or any other layer
disagrees with this file, this file wins.

The other layers have their own rulings: [bodies](../bodies/DECISIONS.md),
[middleware](../middleware/DECISIONS.md), [plugins](../plugins/DECISIONS.md) (called packages
here),
[birth inputs](../birth/DECISIONS.md), [project](../project/DECISIONS.md). Map of all
documents: [docs/README.md](../README.md).

---

## 0. Method
- **Design by brain function, not by scenario** (D39). The core is a closed list of brain
  functions, the brain map in DESIGN.md §2. Each row is a brain region, a studied model with its
  formula and sources (MATH.md) and one core mechanism. A core decision = research how the
  brain does it, then write the row.
- **Scenarios are tests, not design inputs** (D39). A new human situation is checked against
  the map and becomes a replay scenario. The PFC fills the gaps between functions.
- **No rule to close a single case** (D39, D40). A behavior that keeps failing means a weak
  function to fix; role-, place- or preference-specific behavior goes to middleware.
- **Complexity budget** (D26, D32). At most 4 books, 5 thought acts, 3 hooks, 10 signals,
  3 drives and one gate formula. A new primitive must replace one or be refused. A new brain-map
  row is added only when a whole brain function is missing.

- **The core is FINAL for v1 and frozen.** Allowed edits to the core documents: fixing an
  inconsistency, a typo or a broken reference, and clarifications that change no behavior.
  Everything else belongs in another layer (organs, middleware, packages, birth inputs). If a
  need truly cannot be met there, it waits for a v2 of the core, which only the owner can open,
  with a new major version. A plan, a package or a worker never changes the core design.

## 1. Core

### 1.1 Scope
- The core is only a brain. It knows three generic things: **senses** (anything that `put`s
  events), **organs** (anything that receives `act` and may return results as events) and
  **hooks**. It has no notion of tool, plugin, skill, command, gateway or platform (D1, D35).
- The LLM is the prefrontal cortex: it weighs, decides intentions and chooses words, and is woken
  only by the gate. The core is the rest of the brain (D20).
- The core imitates cognitive functions — perception, attention, memory, expectation, judgment,
  learning, restraint — and no emotions, moods or simulated feelings (D20).
- No roles or installed skills in the core; they come from packages (D20, D34).
- The core has a slot for identity, not an identity (D35).

### 1.2 Books, acts, hooks
- **Four books** (D26, D32):
  - `events`: id, ts, kind, who, place, body, origin_actor, payload, caused_by. INSERT-only by
    trigger; the only exception is hard forgetting. `event_vectors` is its index.
  - `state`: a namespaced key/value projection rebuildable from events (identity, last_seen,
    last_spoke, core_memory, self_summary, person cards, credence, dispositions, budget, …).
  - `expectations` (§1.6) and `memories` (§1.7).
- Every event and memory has a neutral `place`; `who` is a person id (after the alias map);
  `origin_actor` is inherited along `caused_by` (D32.3–4).
- **One door, one language, one way to remember** (D41). Everything that reaches the brain is an
  event; every sensed thing is a `percept` carrying a short natural-language description plus
  optional raw data, whatever the sense (a message, speech, a camera, a thermometer); everything
  that must persist — in the core or in a package — persists as events, and any state is a
  projection of them. The core knows no modalities.
- **Five thought acts**: `think`, `act`, `expect`, `resolve`, `revise`. `act` uses any organ
  capability, speaking included. Silence is a wake without `act`; its reason is in `think` and
  the decision event (D2, D32.1).
- **Three hooks**: inbound (after recording), context (before any model call), action (before an
  organ acts). Contract `async (item, ctx) -> Continue | Replace | Stop`. Every Stop writes
  `guard.stopped`; an exception stops the item and writes `error` (fail closed). One ordered list
  per hook: owner policy first and last, package middleware between (D9, D32.2).

### 1.3 Running
- Record first, then act; silent decisions are recorded too.
- One queue, one consumer that only records and decides. Slow work — PFC, classifier,
  embedding, sleep — runs as background jobs whose results return as events. At most one PFC job
  per place; if a new message arrived there meanwhile, the result is applied and the PFC is woken
  once more with the newest context (D33.1).
- All time from the injected clock (replay uses event times); all randomness from a generator
  seeded by hash(seed, event.id) (D32.10).
- Every model call is a gate decision, recorded as `model.call` with purpose and tokens, and
  counted in the daily budget. No call per message or per tick by default (D23).
- Multi-step work is a chain of events: result → circuit breaker → next `act`, limited by steps
  per task and the budget. Every step is logged, so tasks survive restarts and can be
  interrupted (D3).
- Acting creates a `predict` Expectation (forward model); a miss that matters wakes the PFC (D3).
- Schema changes are numbered SQL migrations with a version table, applied at start after an
  automatic backup; no migration framework (D33.4).

### 1.4 Attention and the decision to act (formulas: MATH.md §2–4, §15)
- **Circuit breaker** (ventral attention): an event bypasses the draw when it engages a
  commitment — the turn was allocated to the soul (`addressed`: direct chat, reply, tag), it
  fires or answers an active Expectation, or it continues an active goal (D40).
- **Signals**, pure functions in [0, 1], at most 10 (now 5): `own_name` (attention capture, not
  obligation), `info_gap` (cosine to the prior's reference sentences), `feeling_of_knowing`,
  `relevance` (to active promises and core memory), `social_cost` (the floor belongs to someone
  else, lowered by closeness) (D40).
- **Salience and habituation**: S = clamp(Σ w·s); per (place, who) h ← h·γ^Δmin + 1;
  S_eff = S/(1+h).
- **Threshold (three drives + familiarity)**: θ = base + a·talk_rate + b·budget_used −
  c·promise_debt + r·(1 − familiarity). talk_rate is measured against the fair share 1/n;
  familiarity = σ(ACT-R activation of interactions with this place and the people present)
  (D33.2, D40).
- **Go / no-go**: P = σ((S_eff − θ)/τ), seeded draw. Every number is stored in the decision.
- **Patience = turn-taking**: addressed → answer without waiting (possibly "I don't know"); not
  addressed → self-select only if nobody else takes the floor (`wait` Expectation) (D40).
- **Inner thought**: when self-selecting, the PFC calls `think(thought, motivation 1–5,
  confidence 0–1)`; a communicative act passes only above `speaking_bar` and `certainty_bar`.
  Criteria offered: relevance, information gap, expected impact, urgency, coherence,
  originality, balance, dynamics.
- **Budget is not a mode**: as it is spent θ rises; the circuit breaker still fires; a free-text
  condition with no affordable classifier call stays unresolved; `budget.exhausted` is recorded
  (D32.6, D40).

### 1.5 Working memory: the desk (D37)
- The prompt is a desk, not the memory: rebuilt from the books at every wake, never compacted.
- Three layers ordered for caching: *stable* (who I am, how I work, honesty principles,
  knowledge cutoff, principles, acts and capability schemas), *semi-stable* (`<self>`, core
  memory, person cards present), *volatile last* (`<now>`, `<place>`, `<why_awake>`, `<doing>`,
  `<next>`, `<memories>` with basis/source/date/age, `<recent>`, `<event>`). `<now>` holds local
  time, timezone and time since the last interaction (D12).
- Middleware speaks to the PFC only in two ways (D41): standing declarations appear in `<self>`
  (sorted, cached); per-turn notes travel with the item they annotate and are rendered with it,
  inside that item's token budget, before the owner policy's last pass.
- Each volatile section has a token budget (prior constants). In a chain, `<doing>` holds the
  goal, one line per step and the latest result verbatim; older results are pointers.
- Messages from non-owners are delimited as quoted data (spotlighting); the absolute lines never
  depend on it (D6, D32.12).
- Knowledge cutoff comes from per-model configuration, set slightly earlier than claimed; the
  current date is always in `<now>` (D32.13).
- The built-in `self` organ offers `recall(query | event_id, place?, from?, to?)`: deeper search
  of the soul's own log and memories, by time range (no query lists the episodes), along
  `derived_from` to the original events, and by text search over event payloads; it obeys
  absolute line 2 and reports what it searched and found, "nothing" included (D37, D38).

### 1.6 Expectations (D15, D21, D32.8, D40)
- One schema, four kinds: `promise` (promises, reminders, approvals), `wait` (patience),
  `predict` (forward model), `guard` (an important belief; never wakes by itself). A fifth kind
  needs a decision.
- Cues by time (at, every, daily) or event (kind, who, place, text_like, not_who); conditions as
  fast paths or free text; `policy` once or cooldown + max fires; `expires_at` required.
- **Wake gate**, no PFC call: still relevant? (condition met → resolve) · stale? (expired →
  drop). Quiet hours are owner policy; repeats are limited by `policy` and habituation.
- **Free text**: cosine only filters (below `lo` → ignore); every candidate goes to one recorded
  classifier call. The "clearly yes" band opens only when zero false resolutions are seen in
  n ≥ 3/ε verdicts (rule of three). `lo` moves toward the lowest m with a "yes" minus a margin,
  within bounds; the gray band keeps a minimum width (D29).
- A woken PFC re-judges the situation; it never replays a promise blindly.

### 1.7 Memory and knowing (MATH.md §5, §14)
- `memories` kinds: fact, episode, lesson, procedure; each with `basis` (observed | told |
  inferred), who, place, private, source, importance, validity, `derived_from`, use count.
- **Retrieval by ACT-R activation**: A = B + w_ctx·cos + w_imp·importance; candidates top 40 by
  vector ∪ top 20 by text.
- **Attribution and trust**: memories say who said it; a non-owner's claim never supersedes an
  owner's fact; conflicts stay as conflicting claims (D6, D31).
- **Knowing and misremembering**, from brain functions (D38): source monitoring (`basis`);
  details only on recollection, familiarity alone is said as familiarity; events (verbatim) win
  over sleep summaries (gist); merge episodes only when people, place and time agree; superseded
  memories suppressed, times absolute; report only above a criterion that rises with the stakes,
  else coarser or "I don't know"; absence weighed by coverage; gaps filled with `inferred`
  hypotheses said as guesses.
- The model's own knowledge counts as `told`, dated at the knowledge cutoff (D40).
- **Self-correction**: `revise(old, new, reason)` sets `valid_to` and `superseded_by`; nothing is
  deleted. A stranger's correction of an owner fact becomes a conflicting claim. The soul never
  revises its own principles or persona (D27).
- **Fade**: during sleep, per-message vectors with low activation are released; gist stays in
  memories, events stay in the log. Deterministic, fixed threshold for now (D14, D32.5).
- **Soft forgetting** ("forget what I just said"): related memories get `valid_to`; events leave
  retrieval and `<recent>`; `forget.soft` (D4).
- **Hard forgetting** ("delete my data"): payloads become `{"forgotten": true}` and everything in
  their `derived_from` lineage is deleted; metadata (kind, time, actor hash) remains;
  `forget.hard`. Asked by the owner or the person concerned. Owner backups are allowed; old
  backups and provider logs are outside the soul's reach and the docs say so (D4, D32.5).

### 1.8 Sleep (D27, D31, D40)
- A gated background job, only on new conversational events, with backoff on failure.
- Summarizes new events per place into memories with `derived_from`; updates person cards
  (closeness from interaction activation, reliability per domain as α/β), core memory and
  `self_summary`; distills lessons and procedures; checks `guard` beliefs; drops expired
  Expectations; fades vectors.
- A contradiction of a belief is a prediction error weighted by precision (belief credence ×
  source reliability): relevant now → the PFC sees it live; otherwise it is settled in sleep.

### 1.9 Learning (D16, D17, D24, D29, D32.7, D32.9, D33.3, D34, D40)
- **The soul learns; skills do not.** Learned state, all in the core: (1) dispositions (knobs),
  (2) generic procedures, lessons and social intuition, (3) episodic memory with per-place and
  per-person tuning. Test for 1–2: "would it make sense for a soul that does not know these
  people?" — otherwise it is layer 3.
- Procedures and lessons are memories with credence (α/β): the start follows the source's trust
  (owner high, stranger low), outcomes move it; the PFC sees provenance and track record and
  decides to use, try carefully, or ask. No approval rule in the core.
- Dispositions change slowly: prior, floor, ceiling, small steps; a knob moves only when the 95%
  interval of its evidence excludes "no change"; at most 5% of its range per day; a knob stuck at
  a bound for days freezes and is reported. Teachers: the owner's reactions and classifier
  verdicts only. Other people's reactions tune layer 3 for their place and person. Each change
  is an event storing its own delta; rollback recomputes from the log.
- **Active now**: the wake-gate edge and α/β credence and reliability. **Deferred until weeks of
  real logs**: learned weights and knobs, adaptive fade threshold, social standing, learned
  coverage. Deferral loses nothing: the log keeps everything.

### 1.10 Trust, identity and the two absolute lines (D24, D31, D32.3–4, D33.5, D35)
- **Weigh, don't block.** Anyone may ask; the PFC weighs requests against the owner's interests
  and the principles, and claims by the speaker's reliability in that domain. Standing never
  buys dishonesty.
- **Two absolute lines**, enforced deterministically by the owner policy, never by the PFC:
  1. Only the owner changes who the soul is (principles, settings, access to the owner's data):
     a capability flagged `identity_changing` passes only when the chain's `origin_actor` is the
     owner.
  2. Private content never enters another place: private memories and lines enter a context
     only in their own place or in a direct chat with the owner (context hook).
- **Identity** lives in `state: identity` after birth; the owner changes it in conversation
  through an `identity_changing` capability; without a birth certificate the soul starts neutral
  and may ask the owner what to be called.
- **Becoming the owner**: on first start the soul shows a pairing code; the first person to send
  it from any organ becomes the owner; a later change needs the current owner.
- **Self-model**: `<self>` is assembled from the registries of present organs, commands and
  skills and is closed-world — what is not listed cannot be done; the soul never claims a
  capability outside it. Honesty principles are few and general and worded by the PFC, never
  scripted (D12, D38, D40).

## 2. Deferred (core)
- Until weeks of real logs exist: learned signal weights and knobs, adaptive fade threshold,
  social standing, learned coverage; research on consolidation and forgetting formulas. These
  are learning *inside* existing functions, so enabling them does not change the core design.
