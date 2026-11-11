# DESIGN.md — MonoAgens

The current design, in eight sections. Reasons and history live in DECISIONS.md (cited as Dxx);
file formats in docs/FORMATS.md; steps in ROADMAP.md; rules for coding agents in AGENTS.md.
If this file and DECISIONS.md disagree, DECISIONS.md wins and this file has a bug.

---

## 1. Goals and non-goals

**Goal.** One persistent self (a *soul*) that lives in many bodies — chat apps, a terminal,
later devices and robots — remembers, knows when to stay quiet, keeps its promises, is honest
about what it knows, learns from experience, and works out of the box for non-technical people.

**LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the
brain.**

**Non-goals.**
- No emotions, moods or simulated feelings. The core imitates cognitive *functions*:
  perception, attention, memory, expectation, judgment, learning, restraint (D20).
- Not a hosted service. One deployment is one soul with its own database; the owner brings
  their own model key (BYOK) (D7, D8).
- No roles or skills in the core. Assistant, customer service, sysadmin, and installed skills
  come from optional packages, which do not learn. The soul itself learns (D20, D34).
- Never described as conscious or as "% human". Brain names below are engineering metaphors.

## 2. Mental model

- **The PFC is the LLM**: it weighs, decides intentions and chooses words. It is woken only
  when a moment deserves it.
- **The core is the rest of the brain**: it records, remembers, expects, pays attention, and
  decides cheaply whether the PFC should wake.
- **Organs are the body**: they sense the world and carry out actions in their own way.
- **Hooks are guards at three doors**: they do not think; they apply rules and cannot be talked
  out of them.

| Brain function (analogy) | Mechanism | Where |
|---|---|---|
| Brainstem | the kernel loop | core |
| Senses, motor cortex, cerebellum | organs (fine control inside the organ) | organs |
| Hippocampus | the event log; consolidation during sleep | core / sleep |
| Declarative memory | ACT-R activation | core |
| Predictive cortex | Expectations; `predict` after acting | core |
| Metamemory | feeling-of-knowing signal | core |
| Salience network, habituation | salience over signals; per-source dampening | core |
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
5. **Appraise** without any model call: reflexes, or signals → salience → threshold → draw.
6. If the PFC wakes: assemble the layered context, pass the **context hook**, and start the
   model call as a background job. Its result returns as an event and is handled in a later
   turn (D33). At most one PFC job per place is in flight.
7. For each `act`: pass the **action hook**, record `act.out`, hand it to the organ, create a
   `predict` Expectation.
8. On ticks: maybe sleep.

## 3. Core contract

**Four books.**
- `events` — the log: id · ts · kind · who · place · body · origin_actor · payload · caused_by.
  INSERT-only by trigger; the single exception is hard forgetting (section 7). `who` is a
  person id (after the alias map), `place` a neutral space id, `body` the organ name;
  `origin_actor` is inherited along the causal chain. `event_vectors` (event_id, embedding) is
  the log's index; rows leave only by fading or hard forgetting, each through one dedicated
  path that writes an event.
- `state` — a namespaced key/value projection that can be rebuilt from events:
  `identity`, `last_seen:*`, `last_spoke:*`, `core_memory`, `self_summary`, `habit:*`, `person:*`, `alias:*`,
  `credence:*`, `cognition:weights`, `cognition:dispositions`, `budget:<date>`.
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

**Invariants.** Append-only log · record first, then act · one queue, one consumer that only
records and decides; slow work (model calls, embedding, sleep) runs as background jobs whose
results return as events · all time
from the injected clock (replay uses event times) · all randomness from a generator seeded by
hash(seed, event.id) · every model call is a gate decision, recorded as `model.call` with its
purpose and tokens, and counted in the daily budget · the complexity budget (D26, D32): 4 books,
5 thought acts, 3 hooks, at most 10 signals, 3 drives, one gate formula.

**Schema changes** are numbered SQL migrations with a version table, applied at start after an
automatic backup (D33).

**Event kinds.** msg.in · command · act.out · act.result · decision.silent · decision.note ·
decision.wake · model.call · budget.exhausted · expectation.created · expectation.fired ·
expectation.resolved · job.started · job.result · classifier.verdict · credence.updated · disposition.updated ·
guard.stopped · forget.soft · forget.hard · fade.done · sleep.done · sleep.failed · error.

## 4. Organs and capabilities

An organ is anything that senses (`run(put)`) and/or acts (`act(action)`). It declares its
capabilities: name · JSON schema · description · `communicative` · `long_running` ·
`identity_changing`. The PFC's tools are the five thought acts; `act` offers the capabilities of
the organs present now, listed in a fixed order so the prompt cache holds.

- The brain decides intent and content; the organ decides how (chat bubbles, TTS, gestures).
  Fine control stays inside the organ.
- Long actions report progress and completion as `act.result` events; the core never waits.
- Multi-step work is a chain: result → reflex wake → next `act`, limited by steps per task and
  the budget. Every step is logged, so tasks resume after a restart and can be interrupted.
- Organs translate native command syntax into a neutral `command` event; one shared command
  registry makes `/help` identical everywhere (D9).
- On start an organ asks for `cursor(organ)`, so input is never lost or duplicated.
- `communicative` capabilities count as speaking for the talk-rate drive.

Present organs: Telegram (long polling; owner via `owner.telegram_ids`) and CLI (Unix socket
inside the main process, thin client).

## 5. Expectations

One schema for every promise and prediction:
id · kind · about · who · place · cue_time · cue_event · condition · condition_text ·
condition_vector · intent · weight · policy · status · next_check_at · last_fired_at ·
fire_count · origin_actor · derived_from · expires_at (required) · created_at

| kind | used for | wakes the PFC? |
|---|---|---|
| `promise` | promises, reminders, approvals awaiting the owner | yes, after the wake gate |
| `wait` | patience: does someone else answer first? | yes, if nobody answers |
| `predict` | forward model after `act` ("a reply within 30 min", "arrived within 30 s") | only on a miss that matters |
| `guard` | a fact that should stay true ("Budi lives in Bandung") | never; checked during sleep |

- `cue_time`: at · every · daily. `cue_event`: kind, who, place, text_like, not_who.
- `condition`: exact fast paths (`none`, `event_seen`, `state`) or a free-text
  `condition_text`.
- `policy`: once, or cooldown + max_fires. A fifth kind needs a new decision.

**Wake gate** (no PFC call): *still relevant?* (condition already met → resolve) · *stale?*
(past expiry → drop) · *allowed now?* (quiet hours, budget, nudge caps).

**Free-text conditions.** m = cosine(event vector, condition vector). m < lo → ignore.
m ≥ lo → one classifier call, recorded as `classifier.verdict` with m. The "clearly yes" band
is off (hi = 1) until at least 50 verdicts above a proposed hi contain no "no" (D21).
A woken PFC re-judges the situation; it never replays a promise blindly.

## 6. Cognition

**Reflexes** (wake without a draw): addressed events · a due `promise` that passed the wake
gate · `act.result` of the soul's own actions.

**Signals** — pure functions `(Situation) -> [0, 1]`, at most 10 (now 5). `Situation` holds the
event, its vector, recent messages here, memory candidates with activation, and relevant state.
- `name_mentioned`
- `info_gap` — cosine to reference sentences in the prior (not keywords)
- `feeling_of_knowing` — σ((max activation − τ)/s), recent messages included
- `relevance` — to active promises and core memory
- `social_cost` — spoke recently, busy place, someone already replied, closeness to the speaker

**Gate.**
- S = clamp(Σ wₖ·sₖ); habituation per (place, who): h ← h·γ^Δminutes + 1; S_eff = S/(1+h).
- θ = base + a·talk_rate + b·budget_used − c·promise_debt + newcomer(place), where newcomer
  starts small and relaxes to 0 as interactions in that place accumulate (D33).
- P = σ((S_eff − θ)/τ); draw u from the seeded generator; go if u < P.
- A non-reflex go first creates a `wait` Expectation (patience).
Every number is stored in the decision event.

**Inner thought.** On non-reflex wakes the PFC calls `think(thought, motivation 1..5,
confidence 0..1)` before any communicative `act`; the act passes only above `motivation_min`
and `confidence_min`. Criteria (Inner Thoughts, CHI 2025): relevance, information gap, expected
impact, urgency, coherence, originality, balance, dynamics.

**Thrift mode.** When the daily budget is spent: reflexes only, classifier calls skipped and
treated as "no", `budget.exhausted` recorded.

**Knobs.** All tunable behavior is expressed as about ten named knobs in the prior
(talkativeness, patience, shyness in new places, …); one function maps them to θ, τ, γ and the
weights above (D36, docs/FORMATS.md).

**Learning.** Active now: the wake-gate edge `lo` follows classifier verdicts within bounds;
credence and reliability update as α/β counts. Deferred until weeks of real logs exist (D32.9):
learned knobs (dispositions) and signal weights. Until then they come from the prior. When
enabled, only the owner's reactions and classifier verdicts move them, in bounded steps that each
store their own delta, with an evidence minimum, a daily cap, and freeze-and-report at bounds
(D29, D32.7). Behavior in a given place or with a given person adapts from those people's
suggestions, criticism and reactions, stored as layer 3 tuning (D33). Because the log keeps
everything, learning can later run over the whole history.

## 7. Memory, sleep and forgetting

`memories`: id · who · place · private · kind (fact | episode | lesson | procedure) · text ·
embedding ·
tsv · importance · source_who · valid_from · valid_to · superseded_by · derived_from ·
created_at · last_accessed_at · use_count

- **Activation (ACT-R).** B = ln(n/(1−d)) − d·ln(L); A = B + w_ctx·cos(context, memory) +
  w_imp·importance; L in hours from the injected clock. Candidates: top 40 by vector ∪ top 20 by
  text; ranked by A; use_count updated.
- **Attribution.** Memories say who said it ("Budi said X"). A non-owner's claim never
  supersedes the owner's fact; conflicts stay as conflicting claims.
- **Sleep** (a gated model call). Summarize new events per place into memories with
  `derived_from`; update person cards (closeness from interaction activation; reliability per
  domain as α/β), core memory and `self_summary`; distill lessons and procedures ("how I did
  this before"; generic → layer 2 with credence, person- or place-specific → layer 3); check
  `guard` Expectations; drop expired ones. Sleep
  runs only on new conversational events; failures back off.
- **Self-correction.** `revise(old, new, reason)` sets `valid_to` and `superseded_by`; nothing
  is deleted.
- **Fade.** During sleep, per-message vectors with low activation are released; the gist stays
  in memories, the event stays in the log. Fixed threshold for now (adaptive deferred).
- **Soft forgetting** ("forget what I just said"). Related memories get `valid_to`; events are
  excluded from retrieval and `<recent>`; `forget.soft`.
- **Hard forgetting** ("delete my data"). `forget_hard()` replaces payloads with
  `{"forgotten": true}` and deletes everything in their `derived_from` lineage (vectors,
  memories, expectations). Metadata remains (kind, time, actor hash); `forget.hard`. Asked by the
  owner or the person concerned. Owner backups are allowed; old backups and the model provider's
  logs are outside the soul's reach, and the user documentation says so.

**Learned state** has three layers, all in the core: (1) dispositions, (2) generic procedures,
lessons and social intuition, (3) episodic memory with per-place and per-person tuning.
Installed skills are not learned state; they belong to packages (D34). A soul is born from a **prior** (layers 1–2, a local folder;
format in docs/FORMATS.md). This repository only loads a prior; it never exports or shares
learned state (D25).

## 8. Trust and policy

**Two absolute lines**, held by the owner policy and never decided by the PFC:
1. **Only the owner changes who the soul is.** Enforced in the action hook: a capability flagged
   `identity_changing` passes only when the chain's `origin_actor` is the owner.
2. **Private content never enters another place.** Enforced in the context hook: private
   memories and lines enter a context only in their own place or in a direct chat with the
   owner.

**Identity** (D35). The core has a slot for identity, not an identity. A soul is born from a
birth certificate (`soul.yaml`: name, language, persona, principles; a neutral default if none)
and a prior (instincts). After birth, identity lives in `state: identity`; the owner changes it in
conversation through an `identity_changing` capability. Role packages may provide a birth
certificate, used only at birth.

**Becoming the owner.** On first start the soul shows a short pairing code; the first person
who sends it from any organ becomes the owner. Changing the owner later needs the current
owner (D33).

**Default owner policy** (out of the box): the two lines, the daily budget, quiet hours.
Packages may add rules inside it (for example official roles); they can never remove it.

**Everything else is weighed by the PFC** (D24, D31). Requests from other people are judged
against the owner's interests and the principles. Learned procedures and lessons carry credence
(α/β from source and track record) that the PFC sees. Claims are weighed by the speaker's reliability in
that domain. Standing never buys dishonesty.

**Self-model and honesty** (D12). The prompt has three layers, ordered for caching:
- *Stable*: who I am, how I work (memories can be incomplete; I can be wrong; I act only through
  organs), honesty rules, knowledge cutoff (from per-model configuration, set slightly earlier
  than claimed), principles, thought acts and capability schemas.
- *Semi-stable*: `<self>` (capabilities, commands, installed skills; a short "cannot do" list),
  core memory, person cards of people present.
- *Volatile, last*: `<now>` (date and time), `<place>`, `<why_awake>`, `<doing>`, `<next>`,
  `<memories>` with source and age, `<recent>`, `<event>`. Messages from non-owners are delimited
  as quoted data.
Rules: changing facts → use a search organ, or answer "as of my knowledge in <cutoff>" ·
attribute memories · low confidence → ask or say "I don't know" · never claim a capability not in
`<self>`.

**Packages are trusted code**, installed by the owner and run in-process. The policy holds
against packages that follow the contract; it is not a sandbox against malicious code.
