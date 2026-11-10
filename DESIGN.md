# DESIGN.md — MonoAgens Design

This document describes the target design, written from DECISIONS.md (D1–D23). Each section
names the decisions it implements. If code and this document disagree, this document is the
target; if this document and DECISIONS.md disagree, DECISIONS.md wins and this file is a bug.

## 1. The idea
MonoAgens is a guard who lives with its owner. It writes down everything that happens, knows
where it is and what it is doing, holds Expectations about the world, and only "thinks hard"
when something matters or an Expectation is missed. At night it sleeps and turns notes into
memories and lessons. The LLM is its prefrontal cortex (PFC): called to weigh things and
choose intentions and words, never to stand watch.

LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the brain.

**Stance: a quantized brain.** We keep a handful of brain *functions* as cheap signals the
kernel computes for every event, and wake the PFC only for moments that deserve it.
Human-like behavior must come from combinations of these few signals, never from scenario code.
There is no amygdala: no emotions, moods or simulated feelings (D20). These are engineering
metaphors; never describe the system as conscious.

| Brain function (analogy) | Mechanism | Computed by |
|---|---|---|
| Brainstem | kernel loop | core |
| Senses, motor cortex, cerebellum | senses and organs (fine control inside the organ) | organs |
| Hippocampus | `events` log, sleep consolidation | core / model at night |
| Declarative memory (ACT-R) | memory activation | core (SQL) |
| Predictive cortex | Expectations; forward model after acting | core |
| Metamemory (feeling of knowing) | FOK signal | core |
| Salience network | salience over signals | core |
| Habituation | per-source dampening | core |
| Cognitive "hypothalamus" (drives, no emotion) | dynamic threshold: talk rate, budget, promise debt | core |
| Basal ganglia (go / no-go) | probabilistic gate | core |
| Self-referential network | self-model assembled each wake | core |
| Theory of mind | person cards | model during sleep |
| Default mode network | sleep, reflection, lessons | model at night |
| Prefrontal cortex | inner thought, intentions, words | PFC |
| Values | principles in the stable prompt | PFC |

## 2. Architecture: a brain, organs and an outer ring (D1, D2, D9)
**The core is only a brain.** It knows three generic things:
- **senses** — anything that `put`s events into the queue (a chat app, a clock, a camera);
- **organs** — anything that receives actions via `act` and may send results back as events
  (a chat app, a speaker, a robot arm, a web search client);
- **hooks** — four points where middleware attaches (section 10).

The core has no notion of tool, plugin, skill, command, gateway or platform. One object may be
both a sense and an organ (Telegram hears and speaks).

**Outside the core** a loader assembles **packages**. A package may contain organs, middleware,
skill documents and dependencies on other packages. The **owner policy** (section 10) wraps all
package middleware and cannot be removed by a package.

**Thought acts** are the only acts in the core: `think`, `stay_silent`, `expect`, `resolve`
(and `revise`, pending P1). Every act on the world, including speaking, is an organ capability.
The brain decides intent and content; the organ decides how to carry it out.

## 3. The books (tables)

### events — the Log (append-only, source of truth)
id · ts · kind · who · place · body · payload (jsonb) · caused_by

`who` is a person id after the alias middleware (D10); `place` is a neutral space id;
`body` is the organ name. The core never interprets them.

Kinds: msg.in, act.out (any organ action; communicative ones also count as "speaking"),
act.result, command, decision.silent, decision.note, decision.wake, model.call,
expectation.created, expectation.fired, expectation.resolved, forward.outcome,
classifier.verdict, weights.updated, disposition.updated, guard.stopped, forget.soft,
forget.hard, sleep.done, sleep.failed, error.

INSERT-only, enforced by a trigger. **The single exception** is hard forgetting (section 9):
one dedicated function may replace a payload with `{"forgotten": true}`; the trigger allows
exactly that change and nothing else.

### event_vectors — one embedding per text event
event_id (pk) · embedding vector(384)

Written once after the event is recorded; reused by every signal and memory lookup. Rows may be
deleted **only** by the retention function (D14) or hard forgetting (D4), each through one
dedicated path that writes an event. No UPDATE.

### state — the Status Board (projection)
key · value (jsonb) · updated_by_event

Keys include `last_seen:<who>`, `last_spoke:<place>`, `core_memory`, `self_summary`,
`sleep:last_event`, `habit:<place>:<who>`, `cognition:weights`, `cognition:dispositions`,
`budget:<date>`, `person:<who>` (person card), `alias:<organ>:<native id>`.

### expectations — the Expectation Book (one primitive for every scenario)
id · who · place · about · cue_time · cue_event · expect · completion_text · completion_vector ·
intent · weight · policy · status · next_check_at · last_fired_at · fire_count · source_event ·
expires_at (required) · created_at · origin (`pfc` | `patience` | `forward` | `approval` |
`guarded_fact`)

- cue_time: `{"at"}`, `{"every"}`, `{"daily"}`. cue_event: an event pattern
  (kind, who, place, text_like, not_who).
- expect: exact fast paths `none`, `event_seen`, `state`; or a free-text completion condition
  (`completion_text`, embedded once) judged as in section 5.2.
- policy: `once`, or `cooldown` + `max_fires`.
- Only `pfc` expectations are shown to the PFC as promises; other origins are internal.

### memories — the Memory Book
id · who · place · kind (`fact` | `episode` | `lesson`) · text · embedding · tsv · importance ·
source_who · trust (`owner` | `known` | `stranger`) · valid_from · valid_to · superseded_by ·
source_events · created_at · last_accessed_at · use_count

Sleep stores "Budi said X", not "X is true" (D6). A stranger's claim never supersedes an
owner's fact.

## 4. One turn of the loop
1. Take one event from the queue.
2. Record it and update state in one transaction; resolve the intake ack; embed text into
   event_vectors.
3. **Hook 1 — inbound.** Alias map (D10), commands, owner policy. A Stop ends the turn
   (recorded).
4. **Expectations**: time path on ticks, event path on events, each through the wake gate
   (5.2).
5. **Appraise** (no model call): reflexes, else signals → salience → threshold → go/no-go.
   One decision event holds every number.
6. If the PFC wakes: assemble the context (self-model, section 8), pass **hook 2 — context**,
   call the model (a `model.call` event is written).
7. Each returned call: thought acts are applied by the core; organ actions pass
   **hook 3 — action** then **hook 4 — outbound**, are recorded as `act.out`, then go to the
   organ.
8. After a communicative act or a world action: create a forward-model Expectation.
9. On ticks: maybe sleep.

## 5. Cognition (`monoagens/cognition/`)
The kernel calls one function, `appraise(event) -> Appraisal`.

### 5.1 Reflexes (wake without a draw)
- addressed events (direct chat, mention, reply to the soul);
- `pfc` expectations that fired and passed the wake gate;
- results of the soul's own actions (`act.result`), so multi-step work continues (D3).

### 5.2 The wake gate for Expectations (D15, D21)
Before an Expectation wakes anyone:
1. **Still relevant?** If its completion condition is already met, resolve it; nobody wakes.
2. **Stale?** Past `expires_at`, drop it.
3. **Allowed now?** Quiet hours, budget, nudge caps (from the owner policy and the drives).
Free-text completion: m = cosine(event vector, completion vector). m < lo → ignore.
m ≥ lo → one classifier call ("does this event satisfy: <completion_text>?"), recorded as
`classifier.verdict` with m. The "clearly yes" band is disabled at start (hi = 1) and opens only
as described in D21. A woken PFC re-judges the situation; it never replays a promise blindly.

### 5.3 Signals
A signal is a pure function `(Situation) -> [0, 1]`; it reads, never writes. `Situation` holds
the event, its vector, recent messages here, memory candidates with activation, relevant state.
Signals are registered by name and must have a weight; adding behavior = adding a signal.
Initial set: `name_mentioned`, `info_gap` (cosine to reference sentences in soul.yaml),
`feeling_of_knowing` (σ((max activation − τ)/s), recent messages included), `relevance`
(cosine to active promises and core memory), `social_cost` (spoke recently, busy place, someone
else already replied).

### 5.4 Salience, habituation, threshold, go/no-go
- S = clamp(Σ wₖ·sₖ); habituation per (place, who): h ← h·γ^Δminutes + 1; S_eff = S/(1+h).
- θ = base + a·talk_rate + b·budget_used − c·promise_debt. talk_rate counts communicative acts
  only (D2).
- P = σ((S_eff − θ)/τ); draw u from a generator seeded by hash(seed, event.id); go if u < P.
  Random to users, exactly reproducible in replay.

### 5.5 Patience
A non-reflex "go" creates a `patience` Expectation: someone other than the sender replies here
within `patience_seconds` → silent; nobody does → wake with "open message, nobody answered".

### 5.6 Inner thought
On non-reflex wakes the PFC first calls `think(thought, motivation 1..5, confidence 0..1)`;
communicative acts pass only if motivation ≥ `motivation_min` and confidence ≥
`confidence_min`, otherwise the thought is recorded as silent. Criteria in the prompt
(Inner Thoughts, CHI 2025): relevance, information gap, expected impact, urgency, coherence,
originality, balance, dynamics.

### 5.7 Forward model and learning (D16)
After a communicative act in a group, or any world action, a `forward` Expectation predicts
the outcome (a reply within N minutes; "arrived within 30 s"). Its resolution writes
`forward.outcome`.
Learned values live in state (`cognition:weights`, `cognition:dispositions`); soul.yaml or the
birth snapshot only gives the starting prior. Teachers: classifier verdicts (they move lo, and
later hi, toward the observed boundary) and people's reactions (delta rule
wₖ ← clamp(wₖ + α(r − P)sₖ)). Every value has bounds, the gray band a minimum width, every step
is small, and every change is an event that can be rolled back. The exact update rule is open.

## 6. Organs and capabilities (D2, D3)
An organ declares its capabilities: name, JSON schema, description, `communicative` (bool),
`long_running` (bool). The PFC tool list is assembled from the organs present now, plus the
thought acts. Removing an organ removes its capabilities.
- The organ renders: chunking and formatting for chat, TTS for a speaker, gestures for a robot.
- Fine control stays in the organ; the PFC gives high-level intentions.
- Long-running actions report progress and completion as `act.result` events; the core never
  blocks waiting for an organ.
- Multi-step work: result → reflex wake → next intention, with limits on steps per task and the
  budget. Every step is in the log, so tasks resume after a restart and can be interrupted.
- Organs translate native command syntax into a neutral `command` event (D9).
- On start an organ asks the core for `cursor(organ)` so input is never lost or duplicated.

Current organs: Telegram (long polling; owner recognized via `owner.telegram_ids`), CLI (Unix
socket inside the main process, thin client).

## 7. The PFC and model calls (D12, D23)
OpenAI-compatible chat completions with tool calling over httpx; BYOK from `.env`
(`LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`, plus the model's knowledge cutoff).
Every model call (PFC, classifier, sleep) is a gate decision, writes `model.call` with purpose
and usage, and counts toward `budget:<date>`. `tool_choice: "required"`; silence is
`stay_silent`, so the reason is recorded.

Prompt layers (for caching):
1. **Stable** — who I am, how I work, honesty rules, knowledge cutoff, principles, the thought
   acts and the capability schemas.
2. **Semi-stable** — `<self>` (capabilities, commands, skills present; a short "cannot do"
   list), core memory, person cards of people present.
3. **Volatile** — `<now>`, `<place>`, `<why_awake>` (reason; for non-reflex wakes S, θ, P),
   `<doing>` (own recent actions across places, running tasks), `<next>` (promises due soon),
   `<memories>` (with source and age), `<recent>` (this place, then the same person elsewhere),
   `<event>` last.

## 8. Self-model and honesty (D12)
- Facts that change (office holders, prices, news, versions, schedules): use a search organ if
  present; otherwise answer "as of my knowledge in <cutoff>, … this may have changed".
- Memories are attributed ("Budi said yesterday …"), never stated as plain fact.
- Low confidence → ask back or say "I don't know".
- Never claim a capability not in `<self>`. `/help` and "what can you do?" share one source.

## 9. Memory, sleep, retention and forgetting (D4, D6, D14, D17)
- **Activation (ACT-R)**: B = ln(n/(1−d)) − d·ln(L hours); A = B + w_ctx·cos(context, memory) +
  w_imp·importance. Candidates: top 40 by vector ∪ top 20 by text; rank by A; update use_count.
- **Sleep**: summarize events since the last sleep per place into attributed memories; update
  person cards and core memory; refresh `self_summary`; distill lessons and learned skills
  (layer 2, generic only); drop expired expectations; write `sleep.done`. Raw events untouched.
  Sleep is due only on new conversational events; failures back off by `sleep.retry_minutes`.
- **Retention (D14)**: no day limits. Each per-message vector is scored by activation (use,
  recency, importance, links to promises and active memories). Low activation drops the vector;
  the gist stays in memories; the raw event stays in the log. The threshold adapts to an
  owner-set storage budget. Deterministic.
- **Soft forgetting**: related memories get `valid_to`; events are excluded from retrieval and
  `<recent>`; `forget.soft`.
- **Hard forgetting**: payloads replaced by `{"forgotten": true}`, vectors, derived memories and
  related expectations deleted; `forget.hard`. Requested by the owner or the person the data is
  about. Backups keep data until they expire (documented for users).

## 10. Middleware, packages and the owner policy (D5, D9, D10, D11, D19, D22)
Four hooks: **inbound** (after recording), **context** (before a model call), **action**
(before an organ action is executed), **outbound** (just before `organ.act`).
Contract: `async (item, ctx) -> Continue | Replace(item) | Stop(reason)`. Every Stop writes
`guard.stopped`; an exception in middleware stops the item and writes `error` (fail closed).

Order in every hook: `[owner policy — first] → [package middleware] → [owner policy — last]`.
Package middleware is **shared** (all organs that opt in) or **organ-own** (only its organ).
Name collisions fail at boot.

**Default owner policy (active out of the box, D22):**
- owner-only instructions as soon as the soul is in any group (D5);
- learned skills whose source involves anyone but the owner wait for approval (an `approval`
  Expectation); owner-sourced skills activate at once (D19);
- daily model budget and quiet hours;
- context privacy: direct-chat memories never enter group contexts.

Shared packages planned: **commands** (one registry; `/help` identical everywhere), **alias
map** (D10: only the owner or a verification code links accounts; the soul may only ask),
**skills** (injects names and descriptions of relevant skills through the context hook; loads a
body on request; versions, rollback).

## 11. Learned state and the birth snapshot (D17, D18)
| Layer | Holds | Exportable |
|---|---|---|
| 1 Dispositions | weights, thresholds, band edges, decay, talkativeness | yes |
| 2 Skills | how-to steps, lessons from failures, general social intuition | yes, if generic |
| 3 Episodic memory | conversations, facts about people, per-person tuning | never |
A snapshot holds layers 1–2, generic (no names, places, personal details), reproducible
(replay scenarios per item), model-aware (embedder and LLM recorded). Inheritance gate: passes
replay on a fresh soul, no regressions, owner approval as curator at first.

## 12. Guarantees (prototype)
- Input never lost or duplicated; output at most once.
- Fired promises unresolved at startup wake the PFC once more.
- Decisions reproducible: same events + seed + learned state → same decisions.
- Every model call, guard stop, learning step and forgetting is an event.

## 13. Not here yet
World tools beyond the first organs · bodies beyond Telegram and CLI · images and voice ·
multiple processes · learned gate classifier · snapshot format · track H details (P1).
