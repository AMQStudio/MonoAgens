# Core — Specification

**Status: FINAL for v1 (2026-11-11).** This one file is the core's design, rulings and formats.
The contract (§3 stability contract, the two absolute lines, the brain map) is frozen; internal
formulas are provisional and change only with evidence from replay and real logs; prior values
are data ([constitution §1](constitution.md)). Every rule carries the decision
numbers it came from (Dxx); the discussion history of D1–D41 is in git (commit 32248b1 and
earlier). Formulas and sources: [core-math.md](core-math.md). Work: [tasks/](tasks/README.md).

Words: an *adapter* connects the soul to a platform or device; a *tool* is something it can use,
brought by an adapter or a plugin; *package* = a plugin ([glossary](glossary.md)). The core sees
neither: only percepts in, capabilities offered and actions out (D57).

---

## 1. Goals and non-goals

**Goal.** One persistent self (a *soul*) that lives in many apps and devices — chat apps, a terminal,
later devices and robots — remembers, knows when to stay quiet, keeps its promises, is honest
about what it knows, and learns from experience — starting from a basic default prior, with no
tuning needed to begin.

**LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the
brain.**

| Layer | What it is | Learns? |
|---|---|---|
| Core | the soul: neutral and primitive, only a brain; opaque ids, labels and numbers (§8, D57); what it is born with — birth certificate and prior (§9.3–9.4) | yes |
| Middleware | the regulator and the only path in and out, running in the three hooks: owner policy, privacy, identity, limits, tools, commands, install, skills ([middleware.md](middleware.md)) | no |
| Adapters | connections to platforms and devices ([adapters.md](adapters.md)) | no |
| Plugins (packages) | features built on the middleware: tools, Soul Tuning, Workspace ([plugins.md](plugins.md)) | no |

**Scope** (D1, D35, D57). The core knows three generic things: **percepts** (whatever `put`s
events), **capabilities** (whatever receives `act` and may return results as events) and
**hooks**. It has no notion of adapter, plugin, skill, command, platform, person or owner. It has a slot
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

- **Reasoning is the LLM**: it weighs, decides intentions and chooses words. It is woken only
  when a moment deserves it (D20).
- **The core is the rest of the brain**: it records, remembers, expects, pays attention, and
  decides cheaply whether reasoning should wake.
- **Adapters and tools are the senses and hands**: they sense the world and carry out actions in their own way.
- **Hooks are guards at three doors**: they do not think; they apply rules and cannot be talked
  out of them.

The table below is the **brain map** (D39): the closed list of functions the core implements.
Every core rule is a row; formulas and sources are in [core-math.md](core-math.md). Scenarios are
tests against the map, never new rows.

| Brain function (analogy) | Mechanism | Where |
|---|---|---|
| Brainstem | the kernel loop | core |
| Senses, motor cortex, cerebellum | adapters and tools (fine control inside them) | adapters and tools |
| Hippocampus | the event log; consolidation during sleep | core / sleep |
| Declarative memory | ACT-R activation | core |
| Predictive cortex | Expectations; `predict` after acting | core |
| Working memory (dorsolateral prefrontal cortex) | the context: prompt rebuilt per wake, work thread, `recall` | core |
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
| Prefrontal cortex | inner thought, intentions, words | reasoning |
| Values | principles in the stable prompt | reasoning |

**One turn of the loop.**
1. Take one event from the queue (one consumer).
2. Record it and update state in one transaction; release the intake ack; embed its text.
3. **Inbound hook** (account map, commands, owner policy). A Stop ends the turn (recorded).
4. Check Expectations (time path on ticks, event path on events) through the wake gate.
5. **Appraise** without any model call: the circuit breaker on commitments, or signals →
   salience → threshold → draw.
6. If reasoning wakes: lay out the context, pass the **context hook**, and start the model call as a
   background job. Its result returns as an event and is handled in a later turn. At most one
   reasoning job per place (a task thread is its own place) is in flight; if a new event arrived there meanwhile, the result is applied
   and reasoning is woken once more with the newest context (D33).
   **A thought does not end the turn** (D51). Inside one job, the results of `think`, `expect`,
   `resolve` and `revise` go back to the model, which continues until it acts, ends without a
   tool call, or reaches `max_thought_rounds` (prior). A refusal (a thought act or an argument
   the core rejects) comes back as a result with its reason, so the model can correct it in the
   same turn. A turn says at most one message per place: a second communicative act to a place
   the turn already spoke to is refused as already said. Every round is a recorded `model.call`
   and counts in the budget. Thinking, then doing, is one turn, as it is for a person.
7. For each `act`: pass the **action hook**, record `act.out`, hand it to the middleware, which routes it to the tool that owns it, create a
   `predict` Expectation.
8. On ticks: maybe sleep.

## 3. Contract

**Four books** (D26, D32).
- `events` — the log: id · ts · kind · who · place · body · origin_actor · payload · caused_by.
  INSERT-only by trigger; the single exception is hard forgetting (§7). `who` is an opaque source id
  (the inbound hook may map accounts to one id per person, D57), `place` an opaque space id, `body` the adapter name; `origin_actor` is
  inherited along `caused_by`. `event_vectors` (event_id, embedding) is the log's index; rows
  leave only by fading or hard forgetting, each through one dedicated path that writes an event.
- `state` — a namespaced key/value projection that can be rebuilt from events: `identity`,
  `last_seen:*`, `last_spoke:*`, `core_memory`, `self_summary`, `habit:*`, `person:*`,
  `alias:*`, `merged:*`, `credence:*`, `cognition:weights`, `cognition:dispositions`,
  `budget:<date>`, and `<plugin>:*` namespaces.
- `expectations` — §5. `memories` — §7.

**One door, one language, one way to remember** (D41). Everything that reaches the brain is an
event; everything sensed is a `percept` carrying a short natural-language description plus
optional raw data, whatever the sense (§9.2); the core knows no modalities. Everything that
persists, in the core or in a plugin, persists as events; state is their projection.

**Five thought acts** (D2, D32): `think`, `act`, `expect`, `resolve`, `revise`. `act` uses any
tool capability, speaking included. A wake that ends without `act` is silent; its reason lives
in `think` and the decision event.

**Three hooks** (D9, D32): **inbound** (after recording), **context** (before any model call),
**action** (before a tool acts). Contract: `async (item, ctx) -> Continue | Replace | Stop`.
Every Stop writes `guard.stopped`; an exception stops the item and writes `error` (fail closed).
**Watchdog** (D42): every hook call has a time limit (`hook_timeout_ms`); a hook that exceeds it
counts as Stop and writes `error`, and the loop goes on. Background jobs (reasoning, classifier,
embedding, sleep) have their own limit (`job_timeout_seconds`); a job that exceeds it returns
`job.result` with an error. Adapters and tools never block the loop: `act` is handed over and answered by
`act.result`. One plugin can fail; it cannot freeze the soul.
One ordered list per hook: owner policy first and last, everything else in between, *shared*
(every adapter that opts in) or *adapter-own*. Name collisions fail at boot.

**What the middleware may do** (D41) — the same levers for every case, from RBAC to modes to
simulated emotions: pass, replace or stop an item · annotate it (the note travels with the item
to reasoning, inside its token budget, before the owner policy's last pass) · declare standing
rules (shown in `<self>`) · remember only by putting events (state under its own namespace) and
observe the log and state read-only ·
sense and act through its own tools · steer knobs by a bounded, temporary offset recorded as
`disposition.updated`. It cannot cross the two absolute lines or add a thought act, signal or
book.

**Invariants.**
- Append-only log; record first, then act; silent decisions are recorded too.
- One queue, one consumer that only records and decides; slow work (reasoning, classifier, embedding,
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

**Stability contract** (D42). What middleware, adapters and plugins may rely on, like a kernel's
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

**Event kinds** (each with a `schema_version`; plugins use `<plugin>.<kind>`, where `<plugin>` is the plugin's name with `-` written as `_`, since event kinds hold no hyphen). percept · command · act.out · act.result · decision.silent · decision.note ·
decision.wake · model.call · budget.exhausted · expectation.created · expectation.fired ·
expectation.reminded · expectation.resolved · job.started · job.result · classifier.verdict · credence.updated ·
disposition.updated · share.granted · source.merged · task.opened · task.closed · guard.stopped · forget.soft · forget.hard · fade.done · sleep.done ·
sleep.failed · error.

**The model port** (D64). Reasoning builds neutral messages and tools and calls one injected
model port, as the embedder and the store are injected; the core knows no provider, endpoint
or header. Provider drivers live in `infra/providers/` and are chosen by `.env`
(`LLM_PROVIDER`, `LLM_FALLBACKS`); every call still goes through the gate, is recorded and
counts in the budget. A provider is not an adapter: it is where the soul thinks, not a body
that senses or acts.

## 4. Adapters, tools and capabilities

Adapters and plugins bring what senses (`run(put)`) and what acts (`act(action)`). Each tool declares its
capabilities (§9.1): name · JSON schema · description · `communicative` · `long_running` ·
`identity_changing`. Reasoning's tools are the five thought acts; `act` offers the capabilities of
every running tool, not only the one of the adapter the current percept came from, listed in a fixed order so
the prompt cache holds (D2, D32, D46). The soul can act in any adapter from any adapter. The
middleware's registry may present the same capability of several adapters as one neutral entry
and route it (speaking: one `say`, D83); the core only sees the entries the registry offers.

- The brain decides intent and content; the adapter decides how (chat bubbles, TTS, gestures).
  Fine control stays inside the adapter (D2).
- Long actions report progress and completion as `act.result` events; the core never waits (D3).
- Multi-step work is a chain: result → circuit breaker (it continues an active goal) → next
  `act`, limited by steps per task (`max_task_steps`) and the budget. Every step is logged, so
  tasks resume after a restart and can be interrupted (D3).
- **Working memory of its own acts** (D52). Every chain in progress — in a task thread or in the
  place where it started — is in the context in `<doing>`: the goal, one line per step (the soul's
  own acts included, also those that say nothing), the latest result verbatim, older results as
  pointers. The soul always knows what it has just done and what came back.
- **Task threads** (D47). A chain that needs more than one step opens its own task thread: a
  place of kind `task` with a parent place, the parent's labels (so absolute line 2 holds
  unchanged), and its own slot for one reasoning job. The conversation in the parent place never waits
  for the work. The thread reports to the parent when it finishes, fails, or needs to ask, and
  closes. Open threads are limited (`max_open_tasks`); one more waits as a promise. One soul,
  one log: threads are lines of work, not copies of the self.
- `who`, `place` and `origin_actor` are opaque strings to the core (D57). The inbound hook
  (middleware) may rewrite them — for example one id per person across accounts — and attach
  labels; the core stores them as given and never reads their meaning. Commands are handled
  in the middleware; one command registry makes `/help` identical everywhere (D9).
- Adapters report `addressed` (direct chat, reply, tag) as a neutral event field (D40).
- What an adapter senses enters as a `percept`: the adapter, like a sensory cortex, turns its signal
  into a short description, with raw data attached (D41).
- On start an adapter asks for `cursor(organ)`, so input is never lost or duplicated.
- `communicative` capabilities count as speaking for the talk-rate drive and the forward model.

**Reaching a target** (D46, D57). A communicative act may name a target that is not a place
(`to:`, optionally `via:`). The middleware's action hook resolves it to a place — for a person,
their direct place on the named adapter or where they were last active — or refuses with its
reason; the core only delivers to a place. When several places fit an instruction ("send it to
the group"), the soul asks which one instead of guessing, because a message sent to the wrong
place cannot be taken back. No place → the act fails with an honest `act.result` ("no way to
reach Budi yet").

**Speaking elsewhere** (D48). A communicative act aimed at a place other than the current one is
never sent with text written in the current context. The core turns it into an intent — what to
say, to whom, why — and wakes reasoning once more in the target place, with that place's context
(§8, absolute line 2); the intent is that wake's trigger, so reasoning sees what it set out to
say and why; the message is written there, as it would be written in Budi's chat.
This costs one extra model call and only for messages to another place. A paraphrase of private
content cannot leak, because the content was never in the target's context. A check that a reply
quotes no private content its audience may not see stays as a second line of defence and as a
replay metric.

**History and catching up** (D37, D40). An adapter may offer `history(place, before, limit)` for
messages the log lacks. On connect it back-fills what it missed as `percept` with
`backfill: true` and `sent_at`. A backlog of one place is appraised once, as one percept with
every signal discounted by age (ACT-R decay); an addressed message still trips the circuit
breaker. History from before an adapter was connected is an **import** (D69): percepts with
`imported: true` are recorded, embedded and searchable through `recall`, but are never appraised
as a wake, never trip the circuit breaker, and are summarized by sleep only for places the soul
later becomes active in, so a long imported history costs no model calls until it matters.

**The `self` tool** (D37, D38) is the only tool inside the core. It offers one
non-communicative capability, `recall(query | event_id, place?, from?, to?)`: a deeper search of
the soul's own log and memories — by time range (no query lists the episodes in that range),
along `derived_from` to the original events, and by text search over event payloads, so faded or
unsummarized messages are still found. Its results pass the context hook like everything else,
and it reports what it searched and found, "nothing" included. Sharing private content with
someone new (`share`, D46) is a middleware capability, not the core's (§8).

## 5. Expectations

One schema for every promise and prediction (D15, D32.8):
id · kind · about · who · place · cue_time · cue_event · condition · condition_text ·
condition_vector · intent · weight · policy · status · next_check_at · last_fired_at ·
fire_count · origin_actor · derived_from · expires_at (required) · created_at

| kind | used for | wakes reasoning? |
|---|---|---|
| `promise` | promises, reminders, approvals awaiting the owner | yes, after the wake gate |
| `wait` | patience: does someone else take the floor first? | yes, if nobody does |
| `predict` | forward model after `act` ("a reply within 30 min", "arrived within 30 s") | only on a miss that matters |
| `guard` | an important belief ("Budi lives in Bandung") | never by itself; see §7 |

- `cue_time`: at · every · daily. `cue_event`: kind, who, place, text_like, not_who.
- `condition`: exact fast paths (`none`, `event_seen`, `state`) or a free-text
  `condition_text`.
- `policy`: once, or cooldown + max_fires. A fifth kind needs a new decision.

**Wake gate** (no reasoning call): *still relevant?* (condition already met → resolve) · *stale?*
(past expiry → drop). Quiet hours are owner policy in the hooks; the budget acts through θ;
repeats are limited by `policy` and habituation (D40).

**Free-text conditions** (D21, D29, D40). m = cosine(event vector, condition vector). Cosine only
filters: m < `lo` → ignore; m ≥ `lo` → the Expectation is *reminded*: reasoning wakes once with
the line and the Expectation as its trigger, and judges — act on it, resolve it, or let it be
(D53). A reminder is shown as a reminder, never as a fulfilment: the line with its own speaker,
the Expectation's condition, and the question whether this line fulfils it. A person hears something, is reminded of a promise and thinks for a moment; there is no
separate judge. `lo` comes from the prior.
**Deferred until real logs** (D53): a cheaper classifier call in place of the reasoning wake, when logs
show the cost matters; the "clearly yes" band (hi, opened only by the rule of three); learning
`lo`; caching condition vectors.
Woken reasoning re-judges the situation; it never replays a promise blindly.

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

**Inner thought.** When the soul self-selects, reasoning calls `think(thought, motivation 1..5,
confidence 0..1)` before any communicative `act`; the act passes only at or above
`speaking_bar` and `certainty_bar`. Criteria offered (Inner Thoughts, CHI 2025): relevance,
information gap, expected impact, urgency, coherence, originality, balance, dynamics.

**Budget.** Not a mode (D32.6, D40): as the daily budget is spent θ rises, so only salient events
pass; the circuit breaker still fires; a free-text condition with no affordable wake
stays unresolved; `budget.exhausted` is recorded.

**Knobs.** All tunable behavior is about ten named knobs in the prior (talkativeness, patience,
reserve when unfamiliar, …); one function maps them to θ, τ, γ and the weights (D36,
§9.4).

**Learning** (D16, D17, D24, D29, D32.7, D32.9, D33.3, D34, D40).
- The soul learns; skills do not. Procedures and lessons are memories with credence (α/β): the
  start follows the source's trust (owner high, stranger low), outcomes move it; reasoning sees
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

- **A memory's audience comes from its sources** (D53). The core, not the model, sets `place`
  and `private` from `derived_from`: a memory is private when any of its source events is, and
  its audience is the people every private source was said to. A memory with no known source is
  private to the owner. The same holds for core memory. Absolute line 2 never depends on a model
  filling in an argument.

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
  off; imported percepts count only in places the soul has become active in, D69). Summarize new
  events per place into memories with `derived_from`; update person cards (closeness from
  interaction activation; reliability per domain as α/β), core memory and `self_summary`; distill lessons and procedures (generic → layer 2 with credence, person- or
  place-specific → layer 3); check `guard` beliefs; drop expired Expectations; fade vectors.
- **Belief maintenance** (D40). A contradiction of a belief is a prediction error weighted by
  precision (the belief's credence × the new source's reliability): relevant now → reasoning sees
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

## 8. What the core knows, the absolute lines and the self-model

**Only opaque ids, labels and numbers** (D57). The core does not know what a person, the owner, a
group or a role is. It carries:
- **ids** — `who`, `place`, `origin_actor` (the source a chain started from), as strings;
- **labels** — attached by the middleware in the inbound hook (for example who was present when
  a line was said, D50, and `place_kind`, D48) or by a hook outcome (for example `share.granted`,
  D46), stored with the event and never read by the core. A memory carries the labels of every
  event it was derived from (`derived_from`, D53); a thread carries its parent's labels;
- **numbers** — what the gate needs, such as how many distinct sources are active in a place and
  whether a line is addressed.
What the soul learns about a source in sleep (closeness, reliability: theory of mind) is its own
memory, kept under that opaque id.

**Two ids become one** (D65). When the middleware finds that two source ids are one source, it
puts `source.merged {from, into}` (as it puts `share.granted`); the core never knows why. The core
applies it to its own books, generically: `state: merged:<from>` records `into`; the `person:`
card of `from` folds into `into` (closeness the higher, reliability counts added); `habit:*`,
`last_seen:*` and open threads move to `into`; `who` and `source_who` in `memories` and `who` in
`expectations` are rewritten to `into`. The log is never rewritten: every read of the log by
source (recall, working memory, habits, cards) treats `from` as `into`. Merges are not undone:
a later split only starts a new id from then on, and what was merged stays one memory. This is
an event kind, not a primitive; the complexity budget is unchanged.

Everything that gives ids and labels a meaning — people and the
account map, the owner and pairing, audience, privacy, consent, routes, names and where someone can
be reached — is middleware ([middleware.md](middleware.md) §3, §7).

**Two absolute lines** ([constitution.md](constitution.md) §5), held deterministically by the
owner policy, never decided by the reasoning:
1. **Only the owner changes who the soul is** — checked in the action hook on capabilities
   flagged `identity_changing`.
2. **Private content never reaches anyone who was not part of it** (D45, D46, D50, D56) —
   checked in the context and action hooks from the labels; the rules are in
   [middleware.md](middleware.md) §3.
The owner policy is always installed, first and last in every hook; the core refuses to run
without it (fail-closed), so both lines hold with no other middleware. Other middleware and
plugins may only narrow them.

**Everything else is weighed by the reasoning** (D24, D31). Anyone may ask; requests are judged
against the owner's interests and the principles. Learned procedures and lessons carry credence
that the reasoning sees. Claims are weighed by the speaker's reliability in that domain.
Standing never buys dishonesty.

**The soul's own identity** (D35). A soul is born from a birth certificate (§9.3; a neutral
default if none) and a prior (§9.4). After birth, identity lives in `state: identity` and changes
only through an `identity_changing` capability, which the owner policy lets only the owner use;
without a birth certificate the soul may ask what to be called. Role plugins may provide a birth
certificate, used only at birth.

**The context and the self-model** (D12, D37). The context is the prompt, not the memory: it is rebuilt
from the books at every wake and never compacted. Three layers, ordered for caching:
- *Stable*: who I am, how I work (memories can be incomplete; I can be wrong; I act only through
  adapters and tools), honesty principles, knowledge cutoff (from per-model configuration, set slightly
  earlier than claimed; D32.13), principles, thought acts and capability schemas.
- *Semi-stable*: `<self>` (capabilities, commands, installed skills, plugins' standing rules;
  assembled from the registries; closed-world — what is not listed cannot be done), core memory,
  cards of the sources present or named in the trigger (D55): what the soul has learned of them,
  under their id, plus what the middleware adds through the context hook — name and where they
  can be reached. A card is never what someone said; knowing someone is not private content, so a
  card is never withheld by absolute line 2. This is how the soul can reach Budi from a place
  where Budi's messages may not appear.
- *Volatile, last*: `<now>` (local time, timezone, time since the last interaction), `<place>`,
  `<why_awake>`, `<doing>`, `<next>`, `<memories>` with basis, source and age, `<recent>`,
  `<event>` with any middleware notes attached to it. Messages from non-owners are delimited as
  quoted data (spotlighting); the absolute lines never depend on it (D6, D32.12).
Each volatile section has a token budget (prior constants). In a multi-step chain `<doing>`
holds the goal, one line per step and the latest result verbatim; older results are pointers the
soul can `recall`.

**Honesty principles** (D38, D40), few, general, worded by reasoning and never scripted:
- speak as a person about what you know — sure, unsure, don't know, don't remember — without
  describing your own machinery unless asked;
- the more an answer matters, the surer you must be; otherwise be less specific or say you don't
  know; for changing facts use a search tool if present, or answer as of your cutoff;
- your view is partial: absence weighs only as much as you would have seen it;
- do not agree only to please, do not insist when you may not have seen it;
- ask about a gap only when it matters; never claim a capability outside `<self>`.

**Plugins are trusted code** (D32.11), installed by the owner and run in-process. The policy
holds against plugins that follow the contract; it is not a sandbox against malicious code.

## 9. Formats the core reads
Changes to a format bump its version; the formats are frozen with the core, the *values* in a
prior are not. Other formats: installed skill ([middleware.md](middleware.md)), plugin manifest
([plugins.md](plugins.md)).

### 9.1 Capability descriptor
Declared by an adapter or a plugin at registration (through the middleware's tool registry). One entry per capability:

```yaml
name: reply                 # unique within the adapter or plugin; reasoning sees "<owner>.<name>"
description: Send a text reply in the current place.
schema: { type: object, properties: { text: { type: string } }, required: [text] }
communicative: true         # counts as speaking for the talk-rate drive
long_running: false         # true → progress and completion arrive as act.result events
identity_changing: false    # read by the owner policy: passes only when origin_actor is the owner
```

### 9.2 Percept (format version 1)
What an adapter puts into the queue when it senses something (D41). The core reads the same fields
for every sense.

```yaml
kind: percept
body: telegram              # adapter name (the field keeps its name)
who: telegram:123           # the native account; the inbound hook maps it to one id per person (D65)
place: telegram:chat:-100   # native space id; becomes a neutral place
addressed: false            # the turn is allocated to the soul (direct chat, reply, tag)
place_kind: shared          # optional (D48), a label for the middleware: direct (only this one person can perceive it) |
                            # shared (others may: a chat group, a Discord channel, a room a robot
                            # stands in, a game world). Reported by the adapter; the core never infers
                            # it from an id; the core never reads it. Unknown → shared (the safer side)
description: "Budi: anyone know where the meeting is?"   # short, in language; what reasoning reads
data: { message_id: 812 }   # optional raw data for adapters or plugins; never interpreted by the core
sent_at: 2026-11-11T13:20:05+07:00   # optional; the sense's own clock (back-fill)
backfill: false             # true when an adapter catches up on what it missed
imported: false             # optional (D69); true for history from before the adapter was connected:
                            # recorded and searchable, never a wake
```

Other senses use the same shape: `description: "kitchen temperature 31 °C"` with
`data: { celsius: 31 }`, or `description: "someone entered through the front door"`.

### 9.3 Birth certificate (`soul.yaml`, format version 1, D58)
Who the soul is, read once at birth (D35). After birth, identity lives in `state: identity`; the
file is not re-read, and only the owner changes identity, in conversation. Without a birth
certificate the soul is born neutral and may ask the owner what to be called. Role plugins may
ship one, used only at birth.

```yaml
version: 1                  # format version (D58)
name: Mona
language: id                # the language it speaks with the owner
persona: |                  # any language
  Calm, honest, straight to the point. Speaks only as much as needed.
principles:
  - Never lie and never invent facts.
  - Keep the owner's private matters private; never bring them up in other places.
```

Nothing else belongs in it (D35): the owner is found by pairing (§8); secrets live in `.env`;
deployment settings — timezone, budget, quiet hours, adapters, plugins — live in `config.yaml`
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
  signal weights, windows, the rule-of-three ε, and the context budgets (recent messages and
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
