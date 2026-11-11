# Core — Specification

**Status: FINAL for v1 (2026-11-11).** This one file is the core's design, rulings and formats.
The contract (§3 stability contract, the two absolute lines, the brain map) is frozen; internal
formulas are provisional and change only with evidence from replay and real logs; prior values
are data ([constitution §1](constitution.md)). Every rule carries the decision
numbers it came from (Dxx); the discussion history of D1–D41 is in git (commit 32248b1 and
earlier). Formulas and sources: [core-math.md](core-math.md). Work: [tasks/](tasks/README.md).

Words: *organ* = anything that senses or acts (bodies and the tool organs plugins bring);
*package* = a plugin.

---

## 1. Goals and non-goals

**Goal.** One persistent self (a *soul*) that lives in many bodies — chat apps, a terminal,
later devices and robots — remembers, knows when to stay quiet, keeps its promises, is honest
about what it knows, and learns from experience — starting from a basic default prior, with no
tuning needed to begin.

**LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the
brain.**

| Layer | What it is | Learns? |
|---|---|---|
| Core | the soul: a generic brain, including what it is born with — birth certificate and prior (§9.3–9.4) | yes |
| Middleware | the foundation between the core and everything outside it, running in the three hooks: policy, commands & tools, install, skills, identity, RBAC ([middleware.md](middleware.md)) | no |
| Bodies | organs for platforms and devices ([bodies.md](bodies.md)) | no |
| Plugins (packages) | features built on the middleware: tool organs, memory-view, modes, roles ([plugins.md](plugins.md)) | no |

**Scope** (D1, D35). The core knows three generic things: **senses** (anything that `put`s
events), **organs** (anything that receives `act` and may return results as events) and
**hooks**. It has no notion of tool, plugin, skill, command, gateway or platform. It has a slot
for identity, not an identity.

**Non-goals.**
- No emotions, moods or simulated feelings in the core. It imitates cognitive *functions*:
  perception, attention, memory, expectation, judgment, learning, restraint (D20). A plugin may
  add any behavior on top, emotions included (D41).
- Not a hosted service. One deployment is one soul with its own database; BYOK (D7, D8).
- No roles or installed skills in the core; they come from plugins (D20, D34).
- No rule written for a single situation; behavior comes from brain functions (D39, D40).
- Never described as conscious or as "% human". Brain names are engineering metaphors.

## 2. Mental model

- **The PFC is the LLM**: it weighs, decides intentions and chooses words. It is woken only
  when a moment deserves it (D20).
- **The core is the rest of the brain**: it records, remembers, expects, pays attention, and
  decides cheaply whether the PFC should wake.
- **Organs are the body**: they sense the world and carry out actions in their own way.
- **Hooks are guards at three doors**: they do not think; they apply rules and cannot be talked
  out of them.

The table below is the **brain map** (D39): the closed list of functions the core implements.
Every core rule is a row; formulas and sources are in [core-math.md](core-math.md). Scenarios are
tests against the map, never new rows.

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
   PFC job per place is in flight; if a new event arrived there meanwhile, the result is applied
   and the PFC is woken once more with the newest context (D33).
7. For each `act`: pass the **action hook**, record `act.out`, hand it to the organ, create a
   `predict` Expectation.
8. On ticks: maybe sleep.

## 3. Contract

**Four books** (D26, D32).
- `events` — the log: id · ts · kind · who · place · body · origin_actor · payload · caused_by.
  INSERT-only by trigger; the single exception is hard forgetting (§7). `who` is a person id
  (after the alias map), `place` a neutral space id, `body` the organ name; `origin_actor` is
  inherited along `caused_by`. `event_vectors` (event_id, embedding) is the log's index; rows
  leave only by fading or hard forgetting, each through one dedicated path that writes an event.
- `state` — a namespaced key/value projection that can be rebuilt from events: `identity`,
  `last_seen:*`, `last_spoke:*`, `core_memory`, `self_summary`, `habit:*`, `person:*`,
  `alias:*`, `credence:*`, `cognition:weights`, `cognition:dispositions`, `budget:<date>`, and
  `<plugin>:*` namespaces.
- `expectations` — §5. `memories` — §7.

**One door, one language, one way to remember** (D41). Everything that reaches the brain is an
event; everything sensed is a `percept` carrying a short natural-language description plus
optional raw data, whatever the sense (§9.2); the core knows no modalities. Everything that
persists, in the core or in a plugin, persists as events; state is their projection.

**Five thought acts** (D2, D32): `think`, `act`, `expect`, `resolve`, `revise`. `act` uses any
organ capability, speaking included. A wake that ends without `act` is silent; its reason lives
in `think` and the decision event.

**Three hooks** (D9, D32): **inbound** (after recording), **context** (before any model call),
**action** (before an organ acts). Contract: `async (item, ctx) -> Continue | Replace | Stop`.
Every Stop writes `guard.stopped`; an exception stops the item and writes `error` (fail closed).
**Watchdog** (D42): every hook call has a time limit (`hook_timeout_ms`); a hook that exceeds it
counts as Stop and writes `error`, and the loop goes on. Background jobs (PFC, classifier,
embedding, sleep) have their own limit (`job_timeout_seconds`); a job that exceeds it returns
`job.result` with an error. Organs never block the loop: `act` is handed over and answered by
`act.result`. One plugin can fail; it cannot freeze the soul.
One ordered list per hook: owner policy first and last, everything else in between, *shared*
(every organ that opts in) or *organ-own*. Name collisions fail at boot.

**What the middleware may do** (D41) — the same levers for every case, from RBAC to modes to
simulated emotions: pass, replace or stop an item · annotate it (the note travels with the item
to the PFC, inside its token budget, before the owner policy's last pass) · declare standing
rules (shown in `<self>`) · remember only by putting events (state under its own namespace) and
observe the log and state read-only ·
sense and act through its own organs · steer knobs by a bounded, temporary offset recorded as
`disposition.updated`. It cannot cross the two absolute lines or add a thought act, signal or
book.

**Invariants.**
- Append-only log; record first, then act; silent decisions are recorded too.
- One queue, one consumer that only records and decides; slow work (PFC, classifier, embedding,
  sleep) runs as background jobs whose results return as events (D33).
- All time from the injected clock (replay uses event times); all randomness from a generator
  seeded by hash(seed, event.id) (D32.10).
- Every model call is a gate decision, recorded as `model.call` with its purpose and tokens, and
  counted in the daily budget; no call per message or per tick by default (D23).
- Complexity budget (D26, D32): 4 books, 5 thought acts, 3 hooks, at most 10 signals, 3 drives,
  one gate formula.

**Schema changes** are numbered SQL migrations with a version table, applied at start after an
automatic backup; no migration framework (D33).

**Snapshots** (D42; built once real logs show rebuilds are slow). `state` is a projection of the
log; rebuilding it from the first event gets slow over years. The core periodically writes a snapshot of `state` with the id of the last event
it includes (every `snapshot_every_events`); a rebuild loads the latest snapshot and replays only
later events. Snapshots are caches: deleting them loses nothing. Hard forgetting invalidates every
snapshot that contains the forgotten data; a new one is written from the cleaned log.

**Stability contract** (D42). What middleware, bodies and plugins may rely on, like a kernel's
user-space ABI:
- *Stable surfaces*: the percept (§9.2), the capability descriptor (§9.1), the hook contract
  (Continue / Replace / Stop, fail closed, watchdog), the six levers, the read-only view of the
  log and state (for dashboards, debugging, metric exporters; D43), event kinds and their
  payloads, `recall`, the birth certificate (§9.3) and the prior format (§9.4).
- *Within v1, changes are additive only*: new optional fields, new event kinds, new constants.
  Readers ignore fields they do not know. Nothing that exists is renamed, removed or given a new
  meaning.
- *Events are versioned*: every event carries a `schema_version`; when a payload schema first
  changes, readers upgrade old versions on read (upcasting); stored events are never rewritten,
  except by hard forgetting.
- Plugins put their own events under their own namespace (`<plugin>.<kind>`); only the core
  defines unprefixed kinds.
- *Not part of the contract*: code modules and functions, database tables and indexes, prompt
  wording, the values in a prior. They may change at any time.
- A change that breaks a stable surface is a core v2 ([constitution §1](constitution.md)).

**Event kinds** (each with a `schema_version`; plugins use `<plugin>.<kind>`). percept · command · act.out · act.result · decision.silent · decision.note ·
decision.wake · model.call · budget.exhausted · expectation.created · expectation.fired ·
expectation.resolved · job.started · job.result · classifier.verdict · credence.updated ·
disposition.updated · share.granted · guard.stopped · forget.soft · forget.hard · fade.done · sleep.done ·
sleep.failed · error.

## 4. Organs and capabilities

An organ is anything that senses (`run(put)`) and/or acts (`act(action)`). It declares its
capabilities (§9.1): name · JSON schema · description · `communicative` · `long_running` ·
`identity_changing`. The PFC's tools are the five thought acts; `act` offers the capabilities of
every running organ, not only the one the current percept came from, listed in a fixed order so
the prompt cache holds (D2, D32, D46). The soul can act in any body from any body.

- The brain decides intent and content; the organ decides how (chat bubbles, TTS, gestures).
  Fine control stays inside the organ (D2).
- Long actions report progress and completion as `act.result` events; the core never waits (D3).
- Multi-step work is a chain: result → circuit breaker (it continues an active goal) → next
  `act`, limited by steps per task and the budget. Every step is logged, so tasks resume after a
  restart and can be interrupted (D3).
- Organs map native accounts to person ids (alias map) and native command syntax to a neutral
  `command` event; one shared command registry makes `/help` identical everywhere (D9).
- Organs report `addressed` (direct chat, reply, tag) as a neutral event field (D40).
- What an organ senses enters as a `percept`: the organ, like a sensory cortex, turns its signal
  into a short description, with raw data attached (D41).
- On start an organ asks for `cursor(organ)`, so input is never lost or duplicated.
- `communicative` capabilities count as speaking for the talk-rate drive and the forward model.

**Reaching a person** (D46). A communicative act may name a person instead of a place
(`to: person`, optionally `via: body`). The core resolves it to that person's direct place from
the log and the alias map — on the named body, or by default the body where the person was last
active — before the action hook; a middleware hint may suggest another route (lever 2). When
several places fit an instruction ("send it to the group"), the soul asks which one instead of
guessing, because a message sent to the wrong place cannot be taken back. No known direct place → the act fails with
an honest `act.result` ("no way to reach Budi yet"). A communicative act aimed at a place other
than the current one is drafted with a desk built for the target place's audience (§8, absolute
line 2), never with the current desk, so what the soul says to Budi is written as it would be
written in Budi's chat.

**History and catching up** (D37, D40). An organ may offer `history(place, before, limit)` for
messages the log lacks. On connect it back-fills what it missed as `percept` with
`backfill: true` and `sent_at`. A backlog of one place is appraised once, as one percept with
every signal discounted by age (ACT-R decay); an addressed message still trips the circuit
breaker.

**The `self` organ** (D37, D38) is the only organ inside the core. It offers one
non-communicative capability, `recall(query | event_id, place?, from?, to?)`: a deeper search of
the soul's own log and memories — by time range (no query lists the episodes in that range),
along `derived_from` to the original events, and by text search over event payloads, so faded or
unsummarized messages are still found. It obeys absolute line 2 and reports what it searched and
found, "nothing" included. Its second capability, `share(item | episode, to: person | place)` (D46; a place means everyone present there), records
consent to show private content to someone new (§8, absolute line 2); it passes the action hook
only when the chain's `origin_actor` is part of that content's audience.

## 5. Expectations

One schema for every promise and prediction (D15, D32.8):
id · kind · about · who · place · cue_time · cue_event · condition · condition_text ·
condition_vector · intent · weight · policy · status · next_check_at · last_fired_at ·
fire_count · origin_actor · derived_from · expires_at (required) · created_at

| kind | used for | wakes the PFC? |
|---|---|---|
| `promise` | promises, reminders, approvals awaiting the owner | yes, after the wake gate |
| `wait` | patience: does someone else take the floor first? | yes, if nobody does |
| `predict` | forward model after `act` ("a reply within 30 min", "arrived within 30 s") | only on a miss that matters |
| `guard` | an important belief ("Budi lives in Bandung") | never by itself; see §7 |

- `cue_time`: at · every · daily. `cue_event`: kind, who, place, text_like, not_who.
- `condition`: exact fast paths (`none`, `event_seen`, `state`) or a free-text
  `condition_text`.
- `policy`: once, or cooldown + max_fires. A fifth kind needs a new decision.

**Wake gate** (no PFC call): *still relevant?* (condition already met → resolve) · *stale?*
(past expiry → drop). Quiet hours are owner policy in the hooks; the budget acts through θ;
repeats are limited by `policy` and habituation (D40).

**Free-text conditions** (D21, D29, D40). m = cosine(event vector, condition vector). Cosine only
filters: m < `lo` → ignore; m ≥ `lo` → one classifier call, recorded as `classifier.verdict` with
m. The "clearly yes" band is off (hi = 1) until zero false resolutions are seen in n ≥ 3/ε
verdicts above a proposed hi (rule of three). `lo` moves toward the lowest m with a "yes" minus a
margin, within bounds; the gray band keeps a minimum width.
A woken PFC re-judges the situation; it never replays a promise blindly.

## 6. Cognition

**Circuit breaker** (D40; wake without a draw): the event engages a commitment the soul holds —
the turn was allocated to it (`addressed`), it fires or answers an active Expectation, or it
continues an active goal (a result of its own action).

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
  activation of its interactions with this place and the people present (D33, D40).
- P = σ((S_eff − θ)/τ); draw u from the seeded generator; go if u < P.
- Turn-taking: an addressed soul answers without waiting (possibly "I don't know"). A go on a
  turn not allocated to it first creates a `wait` Expectation: it self-selects only if nobody
  else takes the floor (patience) (D40).
Every number is stored in the decision event.

**Inner thought.** When the soul self-selects, the PFC calls `think(thought, motivation 1..5,
confidence 0..1)` before any communicative `act`; the act passes only at or above
`speaking_bar` and `certainty_bar`. Criteria offered (Inner Thoughts, CHI 2025): relevance,
information gap, expected impact, urgency, coherence, originality, balance, dynamics.

**Budget.** Not a mode (D32.6, D40): as the daily budget is spent θ rises, so only salient events
pass; the circuit breaker still fires; a free-text condition with no affordable classifier call
stays unresolved; `budget.exhausted` is recorded.

**Knobs.** All tunable behavior is about ten named knobs in the prior (talkativeness, patience,
reserve when unfamiliar, …); one function maps them to θ, τ, γ and the weights (D36,
§9.4).

**Learning** (D16, D17, D24, D29, D32.7, D32.9, D33.3, D34, D40).
- The soul learns; skills do not. Procedures and lessons are memories with credence (α/β): the
  start follows the source's trust (owner high, stranger low), outcomes move it; the PFC sees
  provenance and track record and decides to use, try carefully, or ask. No approval rule in the
  core.
- Knobs change slowly: prior, floor, ceiling, small steps; a knob moves only when the 95%
  interval of its evidence excludes "no change"; at most 5% of its range per day; a knob stuck at
  a bound for days freezes and is reported. Teachers: the owner's reactions and classifier
  verdicts only. Other people's reactions tune layer 3 for their place and person. Each change
  is an event storing its own delta; rollback recomputes from the log.
- **Active now**: the wake-gate edge `lo`; α/β credence and reliability. **Deferred until weeks
  of real logs** (§10): learned knobs and signal weights, which come from the prior until then.
  Because the log keeps everything, learning can later run over the whole history.

## 7. Memory, sleep and forgetting

`memories`: id · who · place · private · kind (fact | episode | lesson | procedure) · basis
(observed | told | inferred) · text · embedding · tsv · importance · source_who · valid_from ·
valid_to · superseded_by · derived_from · created_at · last_accessed_at · use_count

- **Activation (ACT-R).** B = ln(n/(1−d)) − d·ln(L); A = B + w_ctx·cos(context, memory) +
  w_imp·importance; L in hours from the injected clock. Candidates: top 40 by vector ∪ top 20 by
  text; ranked by A; use_count updated.
- **Attribution** (D6, D31). Memories say who said it ("Budi said X"). A non-owner's claim never
  supersedes the owner's fact; conflicts stay as conflicting claims.
- **Knowing and misremembering** (D38, D40; core-math.md §14), modeled on brain functions:
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
- **Sleep** (D27, D31; a gated background job, only on new conversational events; failures back
  off). Summarize new events per place into memories with `derived_from`; update person cards
  (closeness from interaction activation; reliability per domain as α/β), core memory and
  `self_summary`; distill lessons and procedures (generic → layer 2 with credence, person- or
  place-specific → layer 3); check `guard` beliefs; drop expired Expectations; fade vectors.
- **Belief maintenance** (D40). A contradiction of a belief is a prediction error weighted by
  precision (the belief's credence × the new source's reliability): relevant now → the PFC sees
  it live; otherwise it is settled in sleep.
- **Self-correction** (D27). `revise(old, new, reason)` sets `valid_to` and `superseded_by`;
  nothing is deleted. A stranger's correction of an owner fact becomes a conflicting claim. The
  soul never revises its own principles or persona.
- **Fade** (D14, D32.5). During sleep, per-message vectors with low activation are released; the
  gist stays in memories, the event stays in the log. Deterministic, fixed threshold for now.
- **Soft forgetting** (D4; "forget what I just said"). Related memories get `valid_to`; events are
  excluded from retrieval and `<recent>`; `forget.soft`.
- **Hard forgetting** (D4, D32.5; "delete my data"). `forget_hard()` replaces payloads with
  `{"forgotten": true}` and deletes everything in their `derived_from` lineage (vectors,
  memories, expectations). Metadata remains (kind, time, actor hash); `forget.hard`. Asked by the
  owner or the person concerned. Owner backups are allowed; old backups and the model provider's
  logs are outside the soul's reach, and the user documentation says so.

**Learned state** (D17, D34) has three layers, all in the core: (1) dispositions (knobs),
(2) generic procedures, lessons and social intuition, (3) episodic memory with per-place and
per-person tuning. Test for 1–2: "would it make sense for a soul that does not know these
people?" — otherwise it is layer 3. Installed skills are not learned state; they belong to
plugins. A soul is born from a **prior** (layers 1–2, §9.4); this repository only
loads a prior and never exports or shares learned state (D25).

## 8. Trust, identity and the self-model

**Two absolute lines** (D24, D32.3–4), held by the owner policy and never decided by the PFC:
1. **Only the owner changes who the soul is** (principles, settings, access to the owner's
   data). Enforced in the action hook: a capability flagged `identity_changing` passes only when
   the chain's `origin_actor` is the owner.
2. **Private content never reaches anyone who was not part of it** (D45). Privacy follows people,
   not places. A private memory or line may enter a context only if every person who will see
   the result was part of its audience — the people present where it was said (after the alias
   map, so one person's linked accounts count as one) — or the context is a direct chat with the
   owner. So what Fulan said in a private chat on one body may be used in Fulan's private chat on
   another body, but never in a group where anyone else is present. Content from places that are
   not private flows freely. Content reaches someone new only by consent (D46): a person in its
   audience shares it (`self.share`, recorded as `share.granted`), and the new person then counts
   as part of its audience. Each person speaks only for what they were part of: the owner can share
   work done with the soul, but cannot share what Fulan said in private; the owner's view of
   everything is a view, not ownership. Enforced by the core itself in the context hook, so it holds with
   an empty middleware pipeline. Middleware decides who is the same person (the alias map) and
   may add stricter rules (narrow); it can never widen what this line allows.

**Everything else is weighed by the PFC** (D24, D31). Anyone may ask; requests are judged
against the owner's interests and the principles. Learned procedures and lessons carry credence
that the PFC sees. Claims are weighed by the speaker's reliability in that domain. Standing never
buys dishonesty.

**Identity** (D35). A soul is born from a birth certificate (§9.3; a neutral default if none)
and a prior (§9.4). After birth, identity lives in
`state: identity`; the owner changes it in conversation through an `identity_changing`
capability; without a birth certificate the soul may ask the owner what to be called. Role
plugins may provide a birth certificate, used only at birth.

**Becoming the owner** (D33). On first start the soul shows a short pairing code; the first
person who sends it from any organ becomes the owner. Changing the owner later needs the current
owner.

**Default owner policy** (D22, D24): the two lines, the daily budget, quiet hours — active out of
the box. The owner may tighten or loosen budget and quiet hours; plugins may add rules inside the
policy, never remove it.

**The desk and the self-model** (D12, D37). The prompt is a desk, not the memory: it is rebuilt
from the books at every wake and never compacted. Three layers, ordered for caching:
- *Stable*: who I am, how I work (memories can be incomplete; I can be wrong; I act only through
  organs), honesty principles, knowledge cutoff (from per-model configuration, set slightly
  earlier than claimed; D32.13), principles, thought acts and capability schemas.
- *Semi-stable*: `<self>` (capabilities, commands, installed skills, plugins' standing rules;
  assembled from the registries; closed-world — what is not listed cannot be done), core memory,
  person cards of people present.
- *Volatile, last*: `<now>` (local time, timezone, time since the last interaction), `<place>`,
  `<why_awake>`, `<doing>`, `<next>`, `<memories>` with basis, source and age, `<recent>`,
  `<event>` with any middleware notes attached to it. Messages from non-owners are delimited as
  quoted data (spotlighting); the absolute lines never depend on it (D6, D32.12).
Each volatile section has a token budget (prior constants). In a multi-step chain `<doing>`
holds the goal, one line per step and the latest result verbatim; older results are pointers the
soul can `recall`.

**Honesty principles** (D38, D40), few, general, worded by the PFC and never scripted:
- speak as a person about what you know — sure, unsure, don't know, don't remember — without
  describing your own machinery unless asked;
- the more an answer matters, the surer you must be; otherwise be less specific or say you don't
  know; for changing facts use a search organ if present, or answer as of your cutoff;
- your view is partial: absence weighs only as much as you would have seen it;
- do not agree only to please, do not insist when you may not have seen it;
- ask about a gap only when it matters; never claim a capability outside `<self>`.

**Plugins are trusted code** (D32.11), installed by the owner and run in-process. The policy
holds against plugins that follow the contract; it is not a sandbox against malicious code.

## 9. Formats the core reads
Changes to a format bump its version; the formats are frozen with the core, the *values* in a
prior are not. Other formats: installed skill ([middleware.md](middleware.md)), plugin manifest
([plugins.md](plugins.md)).

### 9.1 Organ capability descriptor
Declared by an organ at registration. One entry per capability:

```yaml
name: reply                 # unique within the organ; the PFC sees "<organ>.<name>"
description: Send a text reply in the current place.
schema: { type: object, properties: { text: { type: string } }, required: [text] }
communicative: true         # counts as speaking for the talk-rate drive
long_running: false         # true → progress and completion arrive as act.result events
identity_changing: false    # true → passes the action hook only when origin_actor is the owner
```

### 9.2 Percept (format version 1)
What an organ puts into the queue when it senses something (D41). The core reads the same fields
for every sense.

```yaml
kind: percept
body: telegram              # organ name
who: telegram:123           # native id; the alias map turns it into a person id
place: telegram:chat:-100   # native space id; becomes a neutral place
addressed: false            # the turn is allocated to the soul (direct chat, reply, tag)
description: "Budi: anyone know where the meeting is?"   # short, in language; what the PFC reads
data: { message_id: 812 }   # optional raw data for organs or plugins; never interpreted by the core
sent_at: 2026-11-11T13:20:05+07:00   # optional; the sense's own clock (back-fill)
backfill: false             # true when an organ catches up on what it missed
```

Other senses use the same shape: `description: "kitchen temperature 31 °C"` with
`data: { celsius: 31 }`, or `description: "someone entered through the front door"`.

### 9.3 Birth certificate (`soul.yaml`, format version 1)
Who the soul is, read once at birth (D35). After birth, identity lives in `state: identity`; the
file is not re-read, and only the owner changes identity, in conversation. Without a birth
certificate the soul is born neutral and may ask the owner what to be called. Role plugins may
ship one, used only at birth.

```yaml
name: Mona
language: id                # the language it speaks with the owner
persona: |                  # any language
  Calm, honest, straight to the point. Speaks only as much as needed.
principles:
  - Never lie and never invent facts.
  - Keep the owner's private matters private; never bring them up in other places.
```

Nothing else belongs in it (D35): the owner is found by pairing (§8); secrets live in `.env`;
deployment settings — timezone, budget, quiet hours, bodies, plugins — live in `config.yaml`
([project.md §2](project.md)); every behavioral number lives in the prior.

### 9.4 Prior (format version 1)
A soul's instincts: a plain folder with layers 1–2 of learned state, loaded at birth (D25, D30,
D34, D36). It never contains layer 3 (episodic memory, people, places, personal details). End
users never see it; developers tune it with replay reports, not by guessing numbers. The default
prior is [`prior/`](../prior/) in the repository; its values are data, tuned with replay, and are
not part of the frozen design. This repository only loads a prior; it never exports, uploads,
shares or aggregates learned state (D18, D25).

```
prior/
  manifest.yaml       # format_version, created, embedder (name + version), llm_family, language
  dispositions.yaml   # knobs, calibration, constants
  references.yaml     # per language: reference sentences for the info_gap signal
  lessons.yaml        # list of {text, credence: {alpha, beta}}
  procedures.yaml     # list of {about, steps, lessons, credence: {alpha, beta}}
```

`dispositions.yaml` has three sections, from "tune this" to "leave this alone":
- **knobs** — about ten values named by behavior; the only values a developer normally changes.
  Each is in [0, 1] unless stated, has a floor and a ceiling, and is what learning moves
  (`disposition.updated talkativeness 0.45 → 0.44`).
- **calibration** — measured per embedder and model by tools (`fok_threshold`, `fok_scale`,
  `wake_gate_lo`, `wake_gate_hi`); never set by hand.
- **constants** — engineering values: how each knob maps to the gate (the two ends of its range),
  signal weights, windows, the rule-of-three ε, and the desk budgets (recent messages and
  tokens, memory candidates and tokens, verbatim thread result, the whole volatile part, recall
  results). Changed only with replay evidence.
One function in the code maps knobs to θ, τ, γ and the weights; nothing else reads knobs
directly. The daily model budget and quiet hours are owner settings (owner policy), not knobs.

| Knob | What it means | Drives (internally) | If the soul is… |
|---|---|---|---|
| `talkativeness` | how easily it speaks without being addressed | θ base (higher knob → lower θ) | too chatty → lower; too quiet → raise |
| `restraint_after_speaking` | how much having just spoken holds it back | talk-rate weight `a` and `social_cost` weight | dominates a conversation → raise |
| `spontaneity` | how varied its decisions are in similar moments | τ (higher knob → more variation) | robotic and predictable → raise; erratic → lower |
| `helpfulness` | how strongly a question it can answer pulls it in | `info_gap` and `feeling_of_knowing` weights | misses chances to help → raise; answers what nobody asked → lower |
| `boredom_with_repeats` | how fast repeated stimuli fade | habituation γ (higher knob → faster fading) | reacts to every sticker or repeat → raise |
| `patience_seconds` | how long it waits for someone else to answer first | the `wait` Expectation window | jumps in too fast → raise; answers too late → lower |
| `reserve_when_unfamiliar` | how reserved it is with unfamiliar places and people | r in θ, times (1 − familiarity) | too forward in new company → raise; too stiff with new people → lower |
| `speaking_bar` | how strong a thought must be before saying it (1..5) | minimum motivation in `think` | says weak things → raise |
| `certainty_bar` | how sure it must be before stating something | minimum confidence; base of the report criterion | states shaky things → raise; hedges too much → lower |
| `memory_persistence` | how long unused memories stay strong | ACT-R decay d (higher knob → slower decay) | forgets too fast → raise; drags up old trivia → lower |

**Loading rules** (D30):
- different embedder → `calibration` falls back to the default prior's;
- different LLM family → load, but halve the α and β of procedures and lessons until proven
  again;
- unknown major `format_version` → refuse with a clear message;
- replay scenarios that prove a prior live in the repository's `scenarios/`, not inside it.

## 10. Deferred
Until weeks of real logs exist (D32.9): learned signal weights and knobs, adaptive fade
threshold, social standing, learned coverage; research on consolidation and forgetting formulas.
These are learning *inside* existing functions, so enabling them does not change this design.
