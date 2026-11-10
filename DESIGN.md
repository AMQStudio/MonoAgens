# DESIGN.md — MonoAgens Design

## The idea in one paragraph
MonoAgens is a guard who lives with its owner. It writes down everything that happens, knows
where it is and what it is doing, holds a list of Expectations about the world, and only
"thinks hard" when something matters or an Expectation is missed. At night it sleeps and
tidies its notes into memories. The LLM is its prefrontal cortex (PFC): called to weigh things
and choose words, never to stand watch. Bodies (Telegram, CLI, robots) are just eyes and hands.

## The four books

### events — the Log (append-only, the source of truth)
id (bigserial) · ts (timestamptz) · kind (text) · who (text, null) · place (text, null) ·
body (text, null) · payload (jsonb) · caused_by (bigint, null)

Kinds: msg.in, msg.out, react.out, decision.silent, decision.note, expectation.created,
expectation.fired, expectation.resolved, sleep.done, error.
Clock ticks are NOT stored, unless they cause something (referenced via another event's
caused_by payload).

`who` is a neutral person id (`person:owner`, `person:tg:123`).
`place` is a neutral space id (`space:tg:-100123`, `space:cli`). Bodies build these; the kernel
never interprets them. They are the hooks for future RBAC and privacy middleware.

### state — the Status Board (a projection; may be dropped and rebuilt)
key (text, pk) · value (jsonb) · updated_by_event (bigint)

MVP keys: `last_seen:<who>` (ts, place, body) · `last_spoke:<place>` (ts) ·
`core_memory` (text, ≤ ~800 tokens, written during sleep) · `sleep:last_event` (id).

### expectations — the Expectation Book (one primitive for every scenario)
id · who · place · about (human sentence) · cue_time (jsonb) · cue_event (jsonb) ·
expect (jsonb) · intent (text) · weight (real 0..1) · policy (jsonb) · status (text) ·
next_check_at (timestamptz, indexed) · last_fired_at · fire_count · source_event ·
expires_at (REQUIRED) · created_at

- cue_time — when to check. One of `{"at": iso}`, `{"every": "2d"}`,
  `{"daily": "07:00"}` (timezone from soul.yaml).
- cue_event — an event pattern that also triggers a check:
  `{"kind": "msg.in", "who": "...", "place": "...", "text_like": "workout"}` (all optional).
- expect — the expected condition. Only three operators:
  - `{"none": true}` — no condition; when the cue arrives it is missed (= a reminder).
  - `{"event_seen": {pattern}, "within": "2d"}` — a matching event should happen in that window.
  - `{"state": "key", "op": "eq|gt|lt", "value": x}` — a state value should hold.
- intent — what the PFC should weigh if it is missed ("remind gently").
- policy — `{"once": true}` or `{"cooldown": "6h", "max_fires": 3}`.
- status — active → fired → resolved (satisfied | dropped) · or snoozed.

Examples:
- "remind me at 7" → cue_time at 07:00, expect none, policy once.
- "nudge me if I skip workouts for 2 days" → cue_time every 1d, expect event_seen
  {msg.in, who=owner, text_like=workout} within 2d, cooldown 1d.
- "ask again if I don't reply" → cue_time at now+1h, expect event_seen
  {msg.in, who=owner, same place} within 1h, once.

### memories — the Memory Book (produced by sleep)
id · who · place · text · embedding vector(384) · tsv (generated tsvector) · importance ·
valid_from · valid_to (null = still valid) · superseded_by · source_events (bigint[]) ·
created_at · last_accessed_at

## One turn of the loop
1. Take one event from the queue (message, tick, anything).
2. If it is not a tick: write it to events and update state (one transaction).
3. Check Expectations:
   - tick → time path: active expectations with next_check_at ≤ now.
   - other events → event path: expectations whose cue_event matches.
   Evaluate `expect` in code. Met → resolved or rescheduled. Missed → fired,
   surprise = weight. Both are written as events.
4. The gate computes a score (no LLM).
5. Score < band.silent → decision.silent event (with the score breakdown). Done.
   Score < band.wake → decision.note event. Done.
   Score ≥ band.wake → wake the PFC.
6. The PFC returns actions through tools. The kernel writes each action as an event
   (msg.out first), then hands it to the matching body.
7. On a tick: if idle ≥ sleep.idle_minutes and there are new events since the last sleep,
   or it is sleep.hour → sleep.

## The gate
Features (all 0..1, weights in soul.yaml):
- addressed: DM, mention, or reply to the bot (flag set by the body) → 1.0
- surprise: weight of a missed expectation
- cooldown_penalty: spoke recently in this place
- crowd_penalty: busy place (many messages in the last few minutes)
score = Σ weight·feature, clamped to 0..1. The breakdown is always stored in the decision
payload so thresholds can be tuned from data.

Two kinds of events skip the score and always wake the PFC:
- addressed events (someone spoke to it);
- fired expectations (it deliberately promised to check something). Their `weight` only orders
  several fired expectations; spam is prevented by the expectation's policy (once, cooldown,
  max_fires), not by the gate. Penalties apply only to the remaining, non-addressed events.

## The PFC (LLM)
OpenAI-compatible endpoint (chat completions + tool calling) over httpx.
Tools: `say(text)`, `react(emoji)`, `stay_silent(reason)`, `expect(...)`, `resolve(id, status)`.

Prompt order (for prompt caching):
1. Stable: identity + persona + principles from soul.yaml + tool definitions.
2. Semi-stable: core_memory (changes only after sleep).
3. Volatile: current time + time since the last interaction, relevant state, active
   expectations for this who/place, ≤ 5 retrieved memories, ≤ 20 recent events in this place,
   the triggering event.

### PFC payload
Request (chat completions):
- messages[0] system — stable: "You are <name>." + persona + principles + the language to use
  + "You act only through the tools. Staying silent is a valid choice."
- messages[1] system — semi-stable: `<core_memory>…</core_memory>` (empty until the first sleep).
- messages[2] user — volatile, tagged sections in this fixed order:
  `<now>` local time, weekday, timezone, time since the last interaction ·
  `<place>` place id and kind (direct/group) · `<why_awake>` reason and gate score ·
  `<expectations>` active ones for this who/place · `<memories>` retrieved items ·
  `<recent>` last events in this place as "HH:MM name: text" · `<event>` the trigger, last.
- tools: the five tools as full JSON Schema. `tool_choice: "required"` — the model must
  choose an action; silence is the `stay_silent` tool, so the reason is recorded.

Response: one or more tool calls. For each call the kernel validates the arguments, writes an
event (msg.out, react.out, decision.silent, expectation.created, expectation.resolved), and
only then hands say/react to the body. Invalid calls become `error` events.

One wake = one LLM call. Tools are actions, not questions, so there is no
think → tool → think loop. That loop is only added later, for world tools whose results must
be read.

Configuration from `.env`: `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`.

### Failure handling
- The kernel loop never dies because of one event. Any exception while processing an event is
  recorded as an `error` event (with the failing event id) and the loop continues.
- An LLM failure (timeout, HTTP error, invalid JSON) → `error` event, no reply.
- A body whose connection fails retries with backoff and never crashes the process.

## Memory and sleep
- Embeddings: fastembed `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (384d),
  run via asyncio.to_thread. Validate quality for the owner's language before locking it in.
- Retrieval: top-20 by vector + top-20 by text (tsvector/pg_trgm) → Reciprocal Rank Fusion
  1/(60+rank) → × importance × 0.995^(hours since last_accessed_at). Only valid_to IS NULL.
  While data is small, no HNSW index (exact scan). When an HNSW index is added and queries
  filter by who/place, set `hnsw.iterative_scan = relaxed_order`.
- Sleep: take events since sleep:last_event → the LLM summarizes them per place into memory
  items (text, importance, who, place) → embed → store → refresh core_memory → expectations
  past expires_at become dropped → sleep.done event. Raw events are never touched.
- Sleep is due only when there are new conversational events (msg.in, msg.out) since
  sleep:last_event; internal events (decisions, errors) never make sleep due. A failed sleep
  writes `sleep.failed` and the next attempt waits at least sleep.retry_minutes, so a broken
  LLM cannot cause a call on every tick.
- The `<recent>` prompt section shows only conversational events (msg.in, msg.out) and fired
  expectations, never internal bookkeeping events.
- `<recent>` has two parts: the last events in this place, then the owner's (or this person's)
  last events in **other places**, each line prefixed with its place, so a conversation that
  moves from one body to another continues. Limits from soul.yaml: memory.recent_events
  (this place) and memory.recent_elsewhere (other places).

## Bodies
A body has exactly two ports:
- `run(put)`: listen to the world, turn input into Events, call put.
- `act(action)`: perform an action (say/react) in a place it owns.
Each body has a name (`telegram`, `cli`). Actions carry the target body name and place; the
kernel keeps a name → body map and calls `act` on the matching body. Replies go to the body and
place of the triggering event unless the PFC names another place.
On start, a body asks the kernel for `cursor(body)` (e.g. the last recorded update_id), so a
restart neither loses nor duplicates input.

CLI: the CLI body lives **inside the main process** as a local Unix socket server
(path from `.env` `CLI_SOCKET`, file mode 0600, owner only). `python -m monoagens.cli` is a thin
client: it connects to the socket, sends each typed line, and prints what comes back. It never
touches the database or the kernel, so there is still exactly one consumer (invariant 3).
Every CLI line is `msg.in` with who = `person:owner`, place = `space:cli`, addressed = true.
If the main process is not running, the client says so and exits.

Telegram: long polling with getUpdates. Offset = cursor + 1, advanced ONLY after the event is
stored. The owner is recognized by `owner.telegram_ids` in soul.yaml → `person:owner`.

## Guarantees and their limits (prototype)
- Input: never lost, never duplicated (cursor comes from events).
- Output: at most once. msg.out is written before sending. If the process crashes in between,
  a message can be lost but is never sent twice.
- Expectations still in status fired with no resolution at startup → the PFC is woken once more.

## Deliberately not here yet
RBAC, approvals, group privacy (later: middleware between queue and kernel, and between PFC
and bodies) · world tools (shell, files, web) · bodies other than Telegram · images/voice ·
multiple processes.