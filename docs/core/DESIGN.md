# Core — Design

**Status: FINAL for v1 — frozen (2026-11-11).** The core design in eight sections. Only
inconsistencies, typos, broken references and clarifications that change no behavior may be
edited; everything else belongs to another layer or waits for a v2 opened by the owner
([DECISIONS.md](DECISIONS.md) §0). Rulings: [DECISIONS.md](DECISIONS.md) (cited as Dxx); core
formats: [FORMATS.md](FORMATS.md); formulas and sources: [MATH.md](MATH.md); order of work:
[plan/](plan/README.md); other layers: [docs/README.md](../README.md); rules for coding agents:
[AGENTS.md](../../AGENTS.md). If this file and DECISIONS.md disagree, DECISIONS.md wins and this
file has a bug.

---

## 1. Goals and non-goals

**Goal.** One persistent self (a *soul*) that lives in many bodies — chat apps, a terminal,
later devices and robots — remembers, knows when to stay quiet, keeps its promises, is honest
about what it knows, learns from experience, and works out of the box for non-technical people.

**LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the
brain.**

**Layers.**
| Layer | What it is | Learns? |
|---|---|---|
| Core | the soul: a generic brain (this document, sections 2–3, 5–8) | yes |
| Middleware | the foundation between the core and everything outside it, running in the three hooks: policy, RBAC, commands & tools, install, identity, skills | no |
| Bodies | organs for platforms and devices (Telegram, CLI, later WhatsApp, robots) | no |
| Packages (plugins) | features built on the middleware: tool organs (search, files), memory-view, modes, roles | no |
| Birth inputs | birth certificate (who I am) and prior (my instincts) | — |

**Non-goals.**
- No emotions, moods or simulated feelings in the core. The core imitates cognitive
  *functions*: perception, attention, memory, expectation, judgment, learning, restraint (D20).
  A package may add any behavior on top, emotions included (D41).
- Not a hosted service. One deployment is one soul with its own database; the owner brings
  their own model key (BYOK) (D7, D8).
- No roles or installed skills in the core; they come from packages (D20, D34).
- No rule written for a single situation; behavior comes from brain functions (D39, D40).
- Never described as conscious or as "% human". Brain names are engineering metaphors.

## 2. Mental model

- **The PFC is the LLM**: it weighs, decides intentions and chooses words. It is woken only
  when a moment deserves it.
- **The core is the rest of the brain**: it records, remembers, expects, pays attention, and
  decides cheaply whether the PFC should wake.
- **Organs are the body**: they sense the world and carry out actions in their own way.
- **Hooks are guards at three doors**: they do not think; they apply rules and cannot be talked
  out of them.

The table below is the **brain map** (D39): the closed list of functions the core implements.
Every core decision is a row; formulas and sources are in MATH.md. Scenarios are tests
against the map, never new rows.

| Brain function (analogy) | Mechanism | Where |
|---|---|---|
| Brainstem | the kernel loop | core |
| Senses, motor cortex, cerebellum | organs (fine control inside the organ) | organs |
| Hippocampus | the event log; consolidation during sleep | core / sleep |
| Declarative memory | ACT-R activation | core |
| Predictive cortex | Expectations; `predict` after acting | core |
| Working memory (dorsolateral PFC) | the desk: prompt rebuilt per wake, work thread, `recall` | core |
| Metamemory | feeling of knowing; source monitoring (`basis`); report control | core |
| Recognition (hippocampus) | familiarity vs recollection; pattern separation | core / sleep |
| Reality filter (orbitofrontal) | superseded memories suppressed; absolute time | core |
| Salience network, habituation | salience over signals; per-source dampening | core |
| Ventral attention (circuit breaker) | commitments bypass the draw | core |
| Turn-taking (social) | `addressed`, `social_cost`, patience as self-selection | core |
| Goal maintenance (prefrontal) | active goals and the work thread hold attention | core |
| Context familiarity | reserve in unfamiliar places and company | core |
| Learning from outcomes (dopamine prediction error) | delta rule; α/β credence | core / sleep |
| Cognitive drives (no emotion) | threshold: talk rate, budget, promise debt | core |
| Basal ganglia (go / no-go) | seeded probabilistic gate | core |
| Theory of mind | person cards (closeness, reliability) | sleep |
| Prefrontal cortex | inner thought, intentions, words | PFC |
| Values | principles in the stable prompt | PFC |

**One turn of the loop.**
1. Take one event from the queue (one consumer).
2. Record it and update state in one transaction; release the intake ack; embed its text.
3. **Inbound hook** (alias map, commands, owner policy). A Stop ends the turn (recorded).
4. Check Expectations (time path on ticks, event path on events) through the wake gate.
5. **Appraise** without any model call: the circuit breaker on commitments, or signals →
   salience → threshold → draw.
6. If the PFC wakes: lay out the desk, pass the **context hook**, and start the model call as a
   background job. Its result returns as an event and is handled in a later turn. At most one
   PFC job per place is in flight (D33).
7. For each `act`: pass the **action hook**, record `act.out`, hand it to the organ, create a
   `predict` Expectation.
8. On ticks: maybe sleep.

## 3. Core contract

**Four books.**
- `events` — the log: id · ts · kind · who · place · body · origin_actor · payload · caused_by.
  INSERT-only by trigger; the single exception is hard forgetting (section 7). `who` is a person
  id (after the alias map), `place` a neutral space id, `body` the organ name; `origin_actor` is
  inherited along the causal chain. `event_vectors` (event_id, embedding) is the log's index;
  rows leave only by fading or hard forgetting, each through one dedicated path that writes an
  event.
- **One door, one language, one way to remember** (D41): everything sensed is a `percept`
  carrying a short natural-language description plus optional raw data, whatever the sense; the
  core knows no modalities. Everything that persists, in the core or in a package, persists as
  events; state is their projection.
- `state` — a namespaced key/value projection that can be rebuilt from events: `identity`,
  `last_seen:*`, `last_spoke:*`, `core_memory`, `self_summary`, `habit:*`, `person:*`,
  `alias:*`, `credence:*`, `cognition:weights`, `cognition:dispositions`, `budget:<date>`.
- `expectations` — section 5.
- `memories` — section 7.

**Five thought acts** (D32): `think`, `act`, `expect`, `resolve`, `revise`. A wake that ends
without `act` is silent; its reason lives in `think` and the decision event.

**Three hooks**: **inbound** (after recording), **context** (before any model call),
**action** (before an organ acts). Contract: `async (item, ctx) -> Continue | Replace | Stop`.
Every Stop writes `guard.stopped`; an exception stops the item and writes `error` (fail closed).
The loader builds one ordered list per hook: owner policy first and last, package middleware in
between. Package middleware is *shared* (every organ that opts in) or *organ-own*. Name
collisions fail at boot.

**Middleware levers** (D41) — the same for every case, from RBAC to modes to simulated emotions:
pass, replace or stop an item · annotate it (the note travels with the item to the PFC, inside
its token budget) · declare standing rules (shown in `<self>`) · remember only by putting events
(state under its own namespace) · sense and act through its own organs · steer knobs by a
bounded, temporary offset recorded as `disposition.updated`. It cannot cross the two absolute
lines or add a thought act, signal or book.

**Invariants.**
- Append-only log; record first, then act.
- One queue, one consumer that only records and decides; slow work (model calls, embedding,
  sleep) runs as background jobs whose results return as events.
- All time from the injected clock (replay uses event times); all randomness from a generator
  seeded by hash(seed, event.id).
- Every model call is a gate decision, recorded as `model.call` with its purpose and tokens, and
  counted in the daily budget.
- Complexity budget (D26, D32): 4 books, 5 thought acts, 3 hooks, at most 10 signals, 3 drives,
  one gate formula.

**Schema changes** are numbered SQL migrations with a version table, applied at start after an
automatic backup (D33).

**Event kinds.** percept · command · act.out · act.result · decision.silent · decision.note ·
decision.wake · model.call · budget.exhausted · expectation.created · expectation.fired ·
expectation.resolved · job.started · job.result · classifier.verdict · credence.updated ·
disposition.updated · guard.stopped · forget.soft · forget.hard · fade.done · sleep.done ·
sleep.failed · error.

## 4. Organs and capabilities

An organ is anything that senses (`run(put)`) and/or acts (`act(action)`). It declares its
capabilities: name · JSON schema · description · `communicative` · `long_running` ·
`identity_changing`. The PFC's tools are the five thought acts; `act` offers the capabilities of
the organs present now, listed in a fixed order so the prompt cache holds.

- The brain decides intent and content; the organ decides how (chat bubbles, TTS, gestures).
  Fine control stays inside the organ.
- Long actions report progress and completion as `act.result` events; the core never waits.
- Multi-step work is a chain: result → circuit breaker (it continues an active goal) → next
  `act`, limited by steps per task and the budget. Every step is logged, so tasks resume after a
  restart and can be interrupted.
- Organs map native accounts to person ids (alias map) and native command syntax to a neutral
  `command` event; one shared command registry makes `/help` identical everywhere (D9).
- Organs report `addressed` (direct chat, reply, tag) as a neutral event field (D40).
- What an organ senses enters as a `percept` (format in FORMATS.md): the organ, like a
  sensory cortex, turns its signal into a short description, with raw data attached (D41).
- On start an organ asks for `cursor(organ)`, so input is never lost or duplicated.
- `communicative` capabilities count as speaking for the talk-rate drive and the forward model.

**History and catching up** (D37, D40). A body may offer `history(place, before, limit)` for
messages the log lacks. On connect it back-fills what it missed as `percept` with
`backfill: true` and `sent_at`. A backlog of one place is appraised once, as one percept with
every signal discounted by age (ACT-R decay); an addressed message still trips the circuit
breaker.

**The `self` organ** (D37) is the only organ inside the core. It offers one non-communicative
capability, `recall(query | event_id, place?, from?, to?)`: a deeper search of the soul's own
log and memories — by time range (no query lists the episodes in that range), along
`derived_from` to the original events, and by text search over event payloads, so faded or
unsummarized messages are still found. It obeys absolute line 2 and reports what it searched
and found, "nothing" included.

Present organs: Telegram (long polling) and CLI (Unix socket inside the main process, thin
client). Tools such as search, files and email are organs as well.

## 5. Expectations

One schema for every promise and prediction:
id · kind · about · who · place · cue_time · cue_event · condition · condition_text ·
condition_vector · intent · weight · policy · status · next_check_at · last_fired_at ·
fire_count · origin_actor · derived_from · expires_at (required) · created_at

| kind | used for | wakes the PFC? |
|---|---|---|
| `promise` | promises, reminders, approvals awaiting the owner | yes, after the wake gate |
| `wait` | patience: does someone else take the floor first? | yes, if nobody does |
| `predict` | forward model after `act` ("a reply within 30 min", "arrived within 30 s") | only on a miss that matters |
| `guard` | an important belief ("Budi lives in Bandung") | never by itself; see section 7 |

- `cue_time`: at · every · daily. `cue_event`: kind, who, place, text_like, not_who.
- `condition`: exact fast paths (`none`, `event_seen`, `state`) or a free-text
  `condition_text`.
- `policy`: once, or cooldown + max_fires. A fifth kind needs a new decision.

**Wake gate** (no PFC call): *still relevant?* (condition already met → resolve) · *stale?*
(past expiry → drop). Quiet hours are owner policy in the hooks; the budget acts through θ;
repeats are limited by `policy` and habituation (D40).

**Free-text conditions.** m = cosine(event vector, condition vector). m < lo → ignore.
m ≥ lo → one classifier call, recorded as `classifier.verdict` with m. The "clearly yes" band
is off (hi = 1) until zero false resolutions are seen in n ≥ 3/ε verdicts above a proposed hi
(rule of three). `lo` follows the verdicts within bounds (D21, D29, D40).
A woken PFC re-judges the situation; it never replays a promise blindly.

## 6. Cognition

**Circuit breaker** (wake without a draw): the event engages a commitment the soul holds — the
turn was allocated to it (`addressed`), it fires or answers an active Expectation, or it
continues an active goal (a result of its own action) (D40).

**Signals** — pure functions `(Situation) -> [0, 1]`, at most 10 (now 5). `Situation` holds the
event, its vector, recent messages here, memory candidates with activation, and relevant state.
- `own_name` — its name appears (attention capture, not a turn allocation)
- `info_gap` — cosine to reference sentences in the prior (not keywords)
- `feeling_of_knowing` — σ((max activation − τ)/s), recent messages included
- `relevance` — to active promises and core memory
- `social_cost` — the floor belongs to someone else (another person selected, or already
  answered), lowered by closeness to the speaker

**Gate.**
- S = clamp(Σ wₖ·sₖ); habituation per (place, who): h ← h·γ^Δminutes + 1; S_eff = S/(1+h).
- θ = base + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity). talk_rate is
  the soul's own share of talk against the fair share 1/n; familiarity is σ of the ACT-R
  activation of its interactions with this place and the people present.
- P = σ((S_eff − θ)/τ); draw u from the seeded generator; go if u < P.
- Turn-taking: an addressed soul answers without waiting (possibly "I don't know"). A go on a
  turn not allocated to it first creates a `wait` Expectation: it self-selects only if nobody
  else takes the floor (patience).
Every number is stored in the decision event.

**Inner thought.** When the soul self-selects, the PFC calls `think(thought, motivation 1..5,
confidence 0..1)` before any communicative `act`; the act passes only at or above
`speaking_bar` and `certainty_bar`. Criteria offered (Inner Thoughts, CHI 2025): relevance,
information gap, expected impact, urgency, coherence, originality, balance, dynamics.

**Budget.** Not a mode (D40): as the daily budget is spent θ rises, so only salient events pass;
the circuit breaker still fires; a free-text condition with no affordable classifier call stays
unresolved; `budget.exhausted` is recorded.

**Knobs.** All tunable behavior is about ten named knobs in the prior (talkativeness, patience,
reserve when unfamiliar, …); one function maps them to θ, τ, γ and the weights (D36,
docs/birth/FORMATS.md).

**Learning.** Active now: the wake-gate edge `lo` follows classifier verdicts within bounds;
credence and reliability update as α/β counts. Deferred until weeks of real logs exist: learned
knobs and signal weights, which come from the prior until then. When enabled, only the owner's
reactions and classifier verdicts move knobs, in bounded steps, each storing its own delta; a
knob moves only when the 95% interval of its evidence excludes "no change", at most 5% of its
range per day, and freezes and is reported when stuck at a bound (D29, D32.7, D40). Behavior in
a given place or with a given person adapts from those people's suggestions, criticism and
reactions, stored as layer 3 tuning (D33). Because the log keeps everything, learning can later
run over the whole history.

## 7. Memory, sleep and forgetting

`memories`: id · who · place · private · kind (fact | episode | lesson | procedure) · basis
(observed | told | inferred) · text · embedding · tsv · importance · source_who · valid_from ·
valid_to · superseded_by · derived_from · created_at · last_accessed_at · use_count

- **Activation (ACT-R).** B = ln(n/(1−d)) − d·ln(L); A = B + w_ctx·cos(context, memory) +
  w_imp·importance; L in hours from the injected clock. Candidates: top 40 by vector ∪ top 20 by
  text; ranked by A; use_count updated.
- **Attribution.** Memories say who said it ("Budi said X"). A non-owner's claim never
  supersedes the owner's fact; conflicts stay as conflicting claims.
- **Knowing and misremembering** (D38, MATH §14), modeled on brain functions:
  - source monitoring: every memory carries its `basis`; the model's own knowledge counts as
    `told`, dated at the knowledge cutoff;
  - familiarity vs recollection: details are stated only on recollection (original events with
    matching people, place and time); familiarity alone is said as familiarity;
  - verbatim vs gist: events win over sleep summaries, which are then revised;
  - pattern separation: episodes merge only when people, place and time agree;
  - reality filter: superseded memories are suppressed, stored times are absolute;
  - report control: a criterion that rises with the stakes; below it, a coarser answer or
    "I don't know";
  - absence is weighed by coverage; gaps are filled with `inferred` hypotheses said as guesses.
- **Sleep** (a gated background job, only on new conversational events; failures back off).
  Summarize new events per place into memories with `derived_from`; update person cards
  (closeness from interaction activation; reliability per domain as α/β), core memory and
  `self_summary`; distill lessons and procedures (generic → layer 2 with credence, person- or
  place-specific → layer 3); check `guard` beliefs; drop expired Expectations; fade vectors.
- **Belief maintenance.** A contradiction of a belief is a prediction error weighted by
  precision (the belief's credence × the new source's reliability): relevant now → the PFC sees
  it live; otherwise it is settled in sleep (D40).
- **Self-correction.** `revise(old, new, reason)` sets `valid_to` and `superseded_by`; nothing
  is deleted. A stranger's correction of an owner fact becomes a conflicting claim. The soul
  never revises its own principles or persona.
- **Fade.** During sleep, per-message vectors with low activation are released; the gist stays
  in memories, the event stays in the log. Deterministic, fixed threshold for now.
- **Soft forgetting** ("forget what I just said"). Related memories get `valid_to`; events are
  excluded from retrieval and `<recent>`; `forget.soft`.
- **Hard forgetting** ("delete my data"). `forget_hard()` replaces payloads with
  `{"forgotten": true}` and deletes everything in their `derived_from` lineage (vectors,
  memories, expectations). Metadata remains (kind, time, actor hash); `forget.hard`. Asked by the
  owner or the person concerned. Owner backups are allowed; old backups and the model provider's
  logs are outside the soul's reach, and the user documentation says so.

**Learned state** has three layers, all in the core: (1) dispositions, (2) generic procedures,
lessons and social intuition, (3) episodic memory with per-place and per-person tuning.
Installed skills are not learned state; they belong to packages (D34). A soul is born from a
**prior** (layers 1–2, a local folder; format in docs/birth/FORMATS.md). This repository only loads a
prior; it never exports or shares learned state (D25).

## 8. Trust, identity and the self-model

**Two absolute lines**, held by the owner policy and never decided by the PFC:
1. **Only the owner changes who the soul is.** Enforced in the action hook: a capability flagged
   `identity_changing` passes only when the chain's `origin_actor` is the owner.
2. **Private content never enters another place.** Enforced in the context hook: private
   memories and lines enter a context only in their own place or in a direct chat with the
   owner.

**Everything else is weighed by the PFC** (D24, D31). Anyone may ask; requests are judged
against the owner's interests and the principles. Learned procedures and lessons carry credence
(α/β from source and track record) that the PFC sees. Claims are weighed by the speaker's
reliability in that domain. Standing never buys dishonesty.

**Identity** (D35). The core has a slot for identity, not an identity. A soul is born from a
birth certificate (`soul.yaml`: name, language, persona, principles; a neutral default if none)
and a prior. After birth, identity lives in `state: identity`; the owner changes it in
conversation through an `identity_changing` capability. Role packages may provide a birth
certificate, used only at birth.

**Becoming the owner.** On first start the soul shows a short pairing code; the first person who
sends it from any organ becomes the owner. Changing the owner later needs the current owner
(D33).

**Default owner policy** (out of the box): the two lines, the daily budget, quiet hours. The
owner may tighten or loosen budget and quiet hours; packages may add rules inside the policy,
never remove it.

**The desk and the self-model** (D12, D37). The prompt is a desk, not the memory: it is rebuilt
from the books at every wake and never compacted. Three layers, ordered for caching:
- *Stable*: who I am, how I work (memories can be incomplete; I can be wrong; I act only through
  organs), honesty principles, knowledge cutoff (from per-model configuration, set slightly
  earlier than claimed), principles, thought acts and capability schemas.
- *Semi-stable*: `<self>` (capabilities, commands, installed skills, packages' standing rules;
  closed-world — what is not listed cannot be done), core memory, person cards of people
  present.
- *Volatile, last*: `<now>` (local time, timezone, time since the last interaction), `<place>`,
  `<why_awake>`, `<doing>`, `<next>`, `<memories>` with basis, source and age, `<recent>`,
  `<event>` with any middleware notes attached to it. Messages from non-owners are delimited
  as quoted data.
Each volatile section has a token budget (prior constants). In a multi-step chain `<doing>`
holds the goal, one line per step and the latest result verbatim; older results are pointers the
soul can `recall`.

**Honesty principles** (D38), few, general, worded by the PFC and never scripted:
- speak as a person about what you know — sure, unsure, don't know, don't remember — without
  describing your own machinery unless asked;
- the more an answer matters, the surer you must be; otherwise be less specific or say you don't
  know; for changing facts use a search organ if present, or answer as of your cutoff;
- your view is partial: absence weighs only as much as you would have seen it;
- do not agree only to please, do not insist when you may not have seen it;
- ask about a gap only when it matters; never claim a capability outside `<self>`.

**Packages are trusted code**, installed by the owner and run in-process. The policy holds
against packages that follow the contract; it is not a sandbox against malicious code.
