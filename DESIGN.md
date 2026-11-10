# DESIGN.md — MonoAgens Design

## The idea in one paragraph
MonoAgens is a guard who lives with its owner. It writes down everything that happens, knows
where it is and what it is doing, holds a list of Expectations about the world, and only
"thinks hard" when something matters or an Expectation is missed. At night it sleeps and
tidies its notes into memories. The LLM is its prefrontal cortex (PFC): called to weigh things
and choose words, never to stand watch. Bodies (Telegram, CLI, robots) are just eyes and hands.

LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the brain.

## Design stance: a quantized brain
We do not simulate the brain. We keep a handful of its *functions* as cheap signals the kernel
computes for every event, and we wake the PFC only for the few moments that deserve it.
"Human-like" behavior must come from **combinations of these few signals**, never from
scenario-specific code. There is no amygdala: no emotions, moods, or simulated feelings.

| Brain function (analogy) | MonoAgens mechanism | Computed by |
|---|---|---|
| Brainstem | the kernel loop | kernel |
| Senses and motor cortex | bodies | bodies |
| Hippocampus (episodes) | `events` log + sleep consolidation | kernel / LLM at night |
| Declarative memory (ACT-R) | memory activation | kernel (SQL) |
| Predictive cortex + cerebellum | Expectations; forward model after speaking | kernel |
| Metamemory (feeling of knowing) | FOK signal from memory activation | kernel |
| Salience network (insula + dACC) | salience score over signals | kernel |
| Habituation | per-source dampening | kernel |
| Cognitive "hypothalamus" (drives, no emotion) | dynamic threshold: talk rate, budget, promise debt | kernel |
| Basal ganglia (go / no-go) | probabilistic gate | kernel |
| Theory of mind | person cards | LLM during sleep |
| Default mode network | sleep, reflection | LLM at night |
| Prefrontal cortex | inner thought, motivation, confidence, words | LLM |
| Values | constitution (principles) in the stable prompt | LLM |

Never describe this as consciousness. These are engineering metaphors.

Scope (D20): the core targets human cognitive *functions* (perception, attention, memory,
judgement, learning, restraint), not emotions. Roles such as assistant, customer service or
sysadmin come from packages, never from the core. A new soul must work well out of the box
without configuration.

## The books (tables)

### events — the Log (append-only, the source of truth)
id (bigserial) · ts (timestamptz) · kind (text) · who (text, null) · place (text, null) ·
body (text, null) · payload (jsonb) · caused_by (bigint, null)

Kinds: msg.in, msg.out, react.out, decision.silent, decision.note, decision.wake,
expectation.created, expectation.fired, expectation.resolved, forward.outcome,
weights.updated, guard.stopped, sleep.done, sleep.failed, error.
Clock ticks are NOT stored, unless they cause something (referenced via another event's
caused_by).

`who` is a neutral person id (`person:owner`, `person:tg:123`).
`place` is a neutral space id (`space:tg:-100123`, `space:cli`). Bodies build these; the kernel
never interprets them. They are the hooks for RBAC and privacy middleware.

### event_vectors — one embedding per message (append-only)
event_id (bigint, pk, references events) · embedding vector(384)

Written once for every msg.in / msg.out with text, right after the event is recorded and before
appraisal. Every signal and every memory lookup reuses this vector; nothing is embedded twice.
INSERT-only, like events.

### state — the Status Board (a projection; may be dropped and rebuilt)
key (text, pk) · value (jsonb) · updated_by_event (bigint)

Keys: `last_seen:<who>` · `last_spoke:<place>` · `core_memory` · `sleep:last_event` ·
`habit:<place>:<who>` (habituation level and its timestamp) · `cognition:weights` (current
signal weights; starts from soul.yaml) · `budget:<date>` (tokens used that day) ·
`person:<who>` (person card, written during sleep).

### expectations — the Expectation Book (one primitive for every scenario)
id · who · place · about (human sentence) · cue_time (jsonb) · cue_event (jsonb) ·
expect (jsonb) · intent (text) · weight (real 0..1) · policy (jsonb) · status (text) ·
next_check_at (timestamptz, indexed) · last_fired_at · fire_count · source_event ·
expires_at (REQUIRED) · created_at · origin (text: `pfc` | `patience` | `forward`)

- cue_time — when to check. One of `{"at": iso}`, `{"every": "2d"}`,
  `{"daily": "07:00"}` (timezone from soul.yaml).
- cue_event — an event pattern that also triggers a check:
  `{"kind": "msg.in", "who": "...", "place": "...", "text_like": "workout"}` (all optional).
- expect — the expected condition. Only three operators:
  - `{"none": true}` — no condition; when the cue arrives it is missed (= a reminder).
  - `{"event_seen": {pattern}, "within": "2d"}` — a matching event should happen in that window.
    A pattern may say `"not_who": "..."` to mean "from someone other than this person".
  - `{"state": "key", "op": "eq|gt|lt", "value": x}` — a state value should hold.
- intent — what the PFC should weigh if it is missed ("remind gently").
- policy — `{"once": true}` or `{"cooldown": "6h", "max_fires": 3}`.
- status — active → fired → resolved (satisfied | dropped) · or snoozed.
- origin — who created it. `pfc` = the LLM via the expect tool. `patience` and `forward` are
  created by the cognition layer (see below). Only `pfc` expectations are shown to the PFC as
  promises; the others are internal.

Examples:
- "remind me at 7" → cue_time at 07:00, expect none, policy once.
- "nudge me if I skip workouts for 2 days" → cue_time every 1d, expect event_seen
  {msg.in, who=owner, text_like=workout} within 2d, cooldown 1d.
- "tell me if someone says the server is down" → cue_event {msg.in, place=group,
  text_like="server down"}, expect none, cooldown 1h.

### memories — the Memory Book (produced by sleep)
id · who · place · text · embedding vector(384) · tsv (generated tsvector) · importance ·
valid_from · valid_to (null = still valid) · superseded_by · source_events (bigint[]) ·
created_at · last_accessed_at · use_count (int, default 0)

## One turn of the loop
1. Take one event from the queue (message, tick, anything).
2. If it is not a tick: write it to events and update state (one transaction). Resolve the
   intake ack. If it carries text, write its vector to event_vectors.
3. **Hook 1 — inbound** middleware (see "Middleware hooks"). A middleware may stop the event
   here (e.g. a command it handled). Stops are recorded.
4. Check Expectations (time path on ticks, event path on events). Missed → fired.
5. **Appraise** (cognition layer, no LLM): reflexes, then signals → salience → threshold →
   go/no-go draw. The decision (silent / note / patience / wake) and every number behind it
   are written as one decision event.
6. If wake: build the PFC context, pass it through **hook 2 — context**, call the PFC.
7. Each tool call passes **hook 3 — tool**, then is written as an event; say/react pass
   **hook 4 — outbound**, then go to the body.
8. After a msg.out in a group: create a forward-model Expectation.
9. On a tick: maybe sleep.

## Cognition layer (`monoagens/cognition/`)
The kernel calls exactly one function, `appraise(event) -> Appraisal`. Everything below lives
in the cognition package, never in kernel.py.

### Reflexes (always wake, no draw)
- addressed events (DM, mention, reply to the bot);
- fired `pfc` expectations (promises), once they have passed the wake gate (see "Waking,
  learning and the birth snapshot").
Spam from promises is prevented by the expectation's policy and the wake gate, not by the
draw.

### Signals
A signal is a **pure function** `(Situation) -> float in [0, 1]`. It reads, never writes.
`Situation` is assembled once per event and holds: the event, its vector, the last messages
in this place, the top memory candidates with their activation, and the relevant state values.
Signals are registered by name; each name must have a weight in `cognition.weights`
(unknown or missing names fail at boot). Adding behavior = adding a signal, not editing the
kernel.

Initial signals:
- `name_mentioned` — the soul's name appears in the text without a tag.
- `info_gap` — the message reads as an open question or something forgotten: cosine of its
  vector to a small set of reference sentences in the owner's language (in soul.yaml), not
  keywords.
- `feeling_of_knowing` — σ((max memory activation among candidates − τ) / s): "I think I know
  this". Candidates include recent messages in this place, not only sleep memories.
- `relevance` — cosine to what the owner cares about now (active `pfc` expectations and
  core_memory, embedded once after each sleep).
- `social_cost` — rises when it spoke recently here, when the place is busy, and when someone
  else already replied after the message.

### Salience and habituation
S = clamp(Σ wₖ·sₖ, 0, 1) with weights from state `cognition:weights`.
Habituation per (place, who): h ← h·γ^(minutes since last) + 1; S_eff = S / (1 + h).
Repeated stimuli from the same source fade by themselves.

### Threshold (drives, no emotions)
θ = base + a·talk_rate + b·budget_used − c·promise_debt
- talk_rate — its own messages here in the last 10 minutes ÷ max(1, active participants).
- budget_used — tokens used today ÷ `cognition.daily_token_budget`.
- promise_debt — share of its `pfc` expectations due within the next hour.

### Go / no-go
P = σ((S_eff − θ) / τ). Draw u ∈ [0, 1); "go" if u < P.
The draw comes from a generator seeded by hash(cognition.seed, event.id), so a decision is
random to the user but **exactly reproducible** in replay. S, θ, P, u and every signal value
are stored in the decision payload.

Outcomes: below `band.note` of P → decision.silent · otherwise and no go → decision.note ·
go → patience (or decision.wake for reflexes).

### Patience
A non-reflex "go" does not wake the PFC at once. It creates an Expectation with origin
`patience`: cue_time now + `cognition.patience_seconds`, expect event_seen {msg.in, same place,
not_who = the original sender}. If someone else answers, it is satisfied and the decision ends
as silent. If nobody answers, it fires and wakes the PFC with the reason "open message, nobody
answered".

### Inner thought (PFC side)
On non-reflex wakes the PFC must first call `think(thought, motivation 1..5, confidence 0..1)`.
The kernel lets `say` through only if motivation ≥ `cognition.motivation_min` and
confidence ≥ `cognition.confidence_min`; otherwise the thought is recorded as
decision.silent. The prompt lists the motivation criteria from the Inner Thoughts study:
relevance, information gap, expected impact, urgency, coherence, originality, balance,
dynamics. On reflex wakes `think` is optional; low confidence means hedge or ask.

### Forward model and learning
After a msg.out in a group, the cognition layer creates an Expectation with origin `forward`:
expect event_seen {msg.in, same place, not_who = self} within
`cognition.forward_window_minutes`. Its resolution writes `forward.outcome`
(responded | ignored).
For proactive messages only, weights learn by a delta rule:
wₖ ← clamp(wₖ + α·(r − P)·sₖ), r = 1 if responded, 0 if ignored, α = `cognition.learning_rate`.
Each change is a `weights.updated` event, so learning can be audited and rolled back.
Learning can be switched off with α = 0.

## Waking, learning and the birth snapshot
Added from D15–D20 (2026-11-10). The rest of this file predates D1–D14 and is pending a
rewrite; where they differ, DECISIONS.md wins.

### The wake gate (D15)
Waking is not thinking. When an Expectation comes due (time path) or an incoming event might
answer it (event path), the kernel runs a gate that makes no PFC call:
1. **Still relevant?** If the completion condition is already met, resolve the Expectation.
   Nobody is woken.
2. **Stale?** Past `expires_at`, drop it.
3. **Allowed now?** Quiet hours, the daily budget (`budget_used`) and nudge caps. These reuse
   the drives that already shape θ; there is no separate schedule panel.
Only an Expectation that passes all three wakes the PFC. The wake reason tells the PFC how old
the promise is and what changed since, so it re-judges instead of replaying.

### Free-text completion condition
An Expectation may carry a completion condition written as a plain sentence, next to the three
existing operators (`none`, `event_seen`, `state`), which stay as exact fast paths. Optional
hints (`who`, `place`) use the same event-pattern fields as `event_seen` and only narrow the
candidates. The sentence is embedded once, when the Expectation is created.

For each incoming event in scope, the match score m ∈ [0, 1] is the cosine between the event's
vector (already in `event_vectors`) and the sentence's vector. Two band edges, lo < hi:
- m < lo — clearly no: ignore, the Expectation keeps waiting.
- m ≥ hi — clearly yes: resolve it.
- lo ≤ m < hi — gray: one call to a small classifier model ("does this message answer what is
  being waited for?"). The verdict is written as an event with m, so it is auditable and
  replayable.
This is the only LLM call outside a PFC wake; see the open question on invariant 5 in
DECISIONS.md.

### Learned dispositions (D16, D17)
The values that shape behavior live in state under `cognition:dispositions`: lo, hi, the
speak/silent base θ, decay rates, general talkativeness. soul.yaml and the birth snapshot only
provide the starting prior.
- Teacher: gray-case verdicts are labelled examples (m, verdict). They move lo and hi toward the
  observed boundary by a small step, so the gray band narrows and gray calls become rarer.
- Corrector: how people react to proactive messages (the `forward.outcome` already recorded:
  responded | ignored), plus explicit "stop" style feedback, using the delta rule already
  described under "Forward model and learning".
- Bounds: every value has a floor and a ceiling, the gray band has a minimum width, and each
  step is small. One day of unusual behavior must not change the character.
- Each change is a `disposition.updated` event (same audit and rollback story as
  `weights.updated`). Same events + same seed + same prior → same dispositions.
The exact update rule is open.

### Three layers of learned state (D17)
| Layer | Holds | Exportable |
|---|---|---|
| 1 Dispositions | thresholds, band edges, decay, talkativeness | yes |
| 2 Skills | how-to steps, lessons from failures, general social intuition | yes, if generic |
| 3 Episodic memory | conversations, facts about people, relationships, per-person tuning | never |
A learned skill has two parts: the steps that worked, and the lessons (what failed and why).
Skills are versioned and active at once (D19); a bad version is rolled back by activating the
previous one. Use counts feed the same ACT-R activation as memories.

### Birth snapshot (D18)
A snapshot is layers 1–2 only. It is:
- **generic** — names, places and personal details are removed; what remains is the pattern;
- **reproducible** — each item carries the replay scenarios (`scripts/replay.py` format) that
  show it still works;
- **model-aware** — it records the embedder and LLM it was learned with, because band edges
  and θ do not transfer across models.
A new soul starts from a snapshot with empty layer 3 and keeps learning from its own users.
Inheritance gate: an item is included only if it passes replay on a fresh soul, no previously
passing scenario regresses, and (at first) the owner approves it as curator. The owner's review
is also the privacy backstop while automatic scrubbing is unproven.

## Middleware hooks
The kernel exposes four hook points. Each holds an ordered list of middleware; in personal mode
all lists are empty.

1. **inbound** — after an event is recorded, before expectations and appraisal.
   Command handler, "who may instruct" checks.
2. **context** — on the assembled PFC context, before the LLM call.
   Privacy: which memories, recent lines and expectations may appear in this place.
3. **tool** — on each tool call, before it is executed.
   Tool permissions, approvals.
4. **outbound** — on each say/react, before body.act.
   Last check, e.g. never quote group A in group B.

Contract: `async middleware(item, ctx) -> Continue | Replace(item) | Stop(reason)`.
Every Stop is written as a `guard.stopped` event with the hook name and reason. Middleware
never fails silently: an exception inside middleware is an `error` event and the item is
stopped (fail closed).

- Commands (`/status`, `/forget`, `/pause`) are reflexes: an inbound middleware that handles
  the command, emits its reply through ctx, and stops the event so cognition is skipped.
- Approvals are Expectations: the tool middleware creates one ("the owner answers yes or no
  within 1h") and stops the call; a matching "yes" re-issues it, a miss drops it.

## The PFC (LLM)
OpenAI-compatible endpoint (chat completions + tool calling) over httpx.
Tools: `think`, `say`, `react`, `stay_silent`, `expect`, `resolve`.

### PFC payload
Request (chat completions):
- messages[0] system — stable: "You are <name>." + persona + principles (the constitution) +
  the language to use + "You act only through the tools. Staying silent is a valid choice."
- messages[1] system — semi-stable: `<core_memory>…</core_memory>`.
- messages[2] user — volatile, tagged sections in this fixed order:
  `<now>` · `<place>` · `<why_awake>` (reason, and for non-reflex wakes S, θ, P) ·
  `<people>` person cards of who is present (from sleep) · `<expectations>` active `pfc` ones ·
  `<memories>` · `<recent>` · `<event>` the trigger, last.
- `tool_choice: "required"`.

Response: tool calls. For each call the kernel validates, passes hook 3, writes an event, and
only then hands say/react (after hook 4) to the body. Invalid calls become `error` events.

One wake = one LLM call, except that a `think` call followed by `say` in the same response is
one wake. The think → world tool → think loop arrives with world tools (ROADMAP).

Every LLM response's usage (prompt, cached, completion tokens) is added to `budget:<date>`.

Configuration from `.env`: `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`.

### Failure handling
- The kernel loop never dies because of one event. Any exception while processing an event is
  recorded as an `error` event and the loop continues.
- An LLM failure → `error` event, no reply.
- A body whose connection fails retries with backoff and never crashes the process.

## Memory and sleep
- Embeddings: fastembed `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (384d),
  run via asyncio.to_thread.
- **Activation (ACT-R)** replaces the old 0.995^hours decay:
  B = ln(n / (1 − d)) − d·ln(L), with n = use_count + 1, L = age in hours (at least 1),
  d = `memory.decay` (0.5).
  A = B + `memory.context_weight`·cos(context, memory) + `memory.importance_weight`·importance.
  Candidates: top 40 by vector ∪ top 20 by text; ranked by A; top `memory.retrieve_k` returned;
  their use_count and last_accessed_at are updated. Only valid_to IS NULL.
  Effect: memories used often stay alive for a long time; rarely used ones fade, as in people.
- Sleep: take events since sleep:last_event → the LLM summarizes them per place into memory
  items (text, importance, who, place), updates person cards (`person:<who>`: what they know,
  care about, and promised) and core_memory → embed → store → drop expired expectations →
  sleep.done. Raw events are never touched.
- Sleep is due only on new conversational events; a failed sleep waits sleep.retry_minutes.
- `<recent>` shows only msg.in, msg.out and expectation.fired: this place first, then the same
  person's other places, each line prefixed with its place.

## Bodies
A body has exactly two ports:
- `run(put)`: listen to the world, turn input into Events, call put.
- `act(action)`: perform an action (say/react) in a place it owns.
Each body has a name; actions carry the target body name and place; the kernel keeps a
name → body map. On start, a body asks the kernel for `cursor(body)`.

CLI: a Unix socket server inside the main process (`CLI_SOCKET`, mode 0600, owner only);
`python -m monoagens.cli` is a thin client that never touches the database or the kernel.

Telegram: long polling with getUpdates. Offset = cursor + 1, advanced only after the event is
stored. The owner is recognized by `owner.telegram_ids` in soul.yaml.

## Guarantees and their limits (prototype)
- Input: never lost, never duplicated.
- Output: at most once.
- Fired expectations with no resolution at startup → the PFC is woken once more.
- Decisions are reproducible: same events + same seed + same weights → same decisions.

## Deliberately not here yet
World tools (shell, files, web) · full RBAC roles and group privacy policies (the hooks exist;
the policies come later) · bodies beyond Telegram and CLI · images/voice · multiple processes ·
learned classifiers for the gate.
