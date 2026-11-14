# MonoAgens — Specification

The one specification of MonoAgens: every rule, format and design decision, each written once.
People start with the [README](../README.md); coding agents with [AGENTS.md](../AGENTS.md), then
the sections their task names. Formulas and their sources are in [research.md](research.md).
Points marked **Draft** state the current intent and may change; everything else is decided.
Only the owner and the architect change this file; *The contract* is locked and changes only as
*Changing the contract* says.

Parts: 1 Principles · 2 The contract · 3 Core · 4 Middleware · 5 Adapters · 6 Plugins ·
7 Operator · 8 Project · 9 Terms.

## 1. Principles

### 1.1 The contract is locked; the internals answer to evidence
Like a kernel: the interface to the outside never breaks, the inside may improve.
- **Locked** — *The contract*, the one contract between the layers (v2, opened and
  locked by the owner on 2026-11-14): the event envelope, features, the capability descriptor,
  hooks, context items, actions and receipts, actors, places, audiences, percepts, lifecycle
  events, answers and commands — plus the hook contract, the six levers, `recall`, the birth
  certificate and prior format, the two absolute lines, the complexity budget and the brain map.
  A hash test holds it; only the owner changes it, additively, by the rules of *Changing the contract*.
- **After the lock there is no "core change" or "middleware change"** of the contract kind.
  What is new — a platform, a device, a feature — enters through an adapter or a plugin.
- **Every file format carries its version** (events *Waist 1: core ↔ middleware*, files *Deployment files*): older versions are
  upcast when read, never rewritten; a newer one fails loudly. This is how a locked contract
  evolves without breaking.
- **Provisional** — the internal formulas: how each signal is computed, the terms of θ, the
  ACT-R parameters, the context layout. They may change **only with evidence** (replay scenarios
  plus real logs showing the change is better), recorded as a decision with that evidence, never
  by a task, plugin or worker on its own.
- **Data** — the values in the default prior (`prior/`), tuned with replay at any time.
- Always allowed: fixing a bug, a vulnerability, an inconsistency, a typo or a broken
  reference, and clarifications or refactors that change no behavior. Everything else belongs to another layer — middleware, adapters, plugins.

### 1.2 Generic, simple, stable
- **The core and the middleware are generic and know nothing above them.** They provide generic
  facts, events and entries that make sense for every body — a chat, a robot, a workspace app, a
  sensor, a game — and never specialize for one. Specializing them for anything above is
  forbidden, without exception.
- **Translation lives at the edge.** Every platform's or device's rules are translated by its
  adapter or plugin into the contract's facts; the middleware and the core never translate.
- **One direction**: the middleware builds on the core's contract, adapters and plugins build on
  the middleware's; a lower layer never changes for a need of an upper one.
- **Understanding belongs to reasoning**; what is not left to it is decided from facts, never
  words (the contract's *Absolute rules*).
- **Simple is king.** Covering scenarios one by one is not design. A case that does not fit is
  classified, never patched.

### 1.3 Design by brain function, not by scenario
- The core is a closed list of brain functions: the brain map in *Mental model*. Each row
  is a brain region, a studied model with its formula and sources ([research.md](research.md))
  and one core mechanism.
- Human scenarios are tests, not design inputs: a new situation becomes a replay scenario
  checked against the map. Reasoning (the LLM) fills the gaps between functions.
- No rule is ever written to close a single case. A behavior that keeps failing means a weak
  function; role-, place- or preference-specific behavior belongs to middleware or a plugin.

### 1.4 Complexity budget
The core holds at most 4 books, 5 thought acts, 3 hooks, 10 signals, 3 drives and one gate
formula. A new primitive must replace one or be refused.

### 1.5 Layers
- **Core** — neutral and primitive: only a brain. It knows no case; it provides mechanisms and
  assembles them. It knows only **opaque ids, labels and numbers**: who and where are strings,
  labels are attached by the middleware and carried without being read, numbers feed the gate.
  It does not know what a person, an owner, a group or a role is. The soul's own identity (birth
  certificate and prior) is the one thing it holds as its own.
  Like a computer's kernel it restricts nothing itself: its capabilities are open — recall the
  whole log, act in any place and any adapter, share data across threads — and every one of them
  leaves only through the three hooks, where the middleware keeps it from running wild.
- **Middleware** — the regulator, like drivers: it decides, it is the only path between the core
  and everything else, and it may limit or add what reaches the core. People, the owner,
  privacy, sharing, routes, commands, the tool registry, budget limits and quiet hours live
  here, as generic rules over the contract's facts; it knows no platform and translates
  nothing. The owner policy is the middleware that is always installed; the core refuses to run
  without it (fail-closed).
- **Adapters and plugins** — talk only to the middleware, through the contract: they translate
  their platform or device into its facts, and the soul's actions back into native actions.
  Standard names are in *Terms*.
- Everything that reaches the brain is a `percept`; everything that persists is an event.
- Middleware and plugins never learn; the soul does. They may add any behavior, emotions
  included, through the six levers; the core itself has no
  emotions.

### 1.6 Two absolute lines
Enforced deterministically by the owner policy, never decided by the model, never removable by a
plugin; another plugin can only narrow them:
1. Only the owner changes who the soul is.
2. Private content never reaches anyone who was not part of it: it follows the people
   it belongs to, across adapters and places, and reaches someone new only when one of those
   people shares it.

### 1.7 Engineering invariants
1. **The core is only a brain.** It knows percepts, capabilities, hooks and thought acts. It
   imports nothing from the middleware, adapters, plugins or model providers, never names a
   platform, and never hard-codes a world action (speaking included): those are capabilities. It
   reads ids, times, `description` and `features`, never an event's `payload` or `labels`.
2. **The log is immutable.** `events` is INSERT-only, enforced by a trigger; the one exception is
   hard forgetting, through one function the trigger recognises. Event vectors leave only by
   fading or hard forgetting, each through its own path that writes an event.
3. **One queue, one consumer.** Every source only puts into the queue; only the kernel loop
   processes events, one at a time, in order.
4. **Record first, then act.** Incoming events are written before they are processed; decisions
   to stay silent are written too.
5. **Every model call is a gate decision**, recorded with its purpose and tokens and counted in
   the daily budget; no model call per message or per tick by default.
6. **No scenario features.** Reminders, monitors and habit trackers are Expectations; what cannot
   be an Expectation, a state key, an adapter or a plugin does not go in.
7. **Pure async.** No blocking call inside `async def`; CPU work goes through
   `asyncio.to_thread`; every hook call and background job runs under its watchdog.
8. **Configuration outside code.** The model provider, models, keys, URLs and reasoning effort
   come from `.env`; deployment settings from `config.yaml` and `config.d/`; identity from the
   birth certificate (`soul.yaml`); weights and thresholds from the prior (`prior/`). No magic
   numbers in code.
9. **Signals are pure**: a signal only reads its situation and returns a number in [0, 1].
10. **Decisions are reproducible**: randomness only from the seeded generator, time only from
    the injected clock (replay uses event times).

### 1.8 Honesty and claims
The soul is honest about what it knows and does not know. MonoAgens is never described as
conscious or as "% human"; claims are per behavior against a human baseline.

### 1.9 Open source
Apache 2.0. BYOK: the owner brings their own model key. One deployment is one soul with its own
database. Public documents mention no hosted service.

## 2. The contract

Decided and locked by the owner on 2026-11-14. This section is the one contract between
the layers; no other section may add a field, a fact or an event to it. It changes only by *Changing the contract*.

### 2.1 Why one locked contract
Systems that last decades share one shape (the hourglass): a small, generic, stable layer in
the middle, with many different things above and below it, and translation only at the edge.
The Internet (IP), Unix (system calls), ROS 2 (rmw), Home Assistant (entities) and Matter
(clusters) all work this way; Linux keeps its user-space interface stable and protobuf grows
only by adding fields. The core and the middleware are that middle: **generic, simple and
stable**. They never specialize for anything above them. Everything specific to a platform, a
device or a feature is translated by its adapter or plugin into the facts below, and nothing
else crosses.

V1's seam (`engine/inbound.py`: `Percept`, `Action`, `Receipt`, capabilities owned by the body,
mandatory and optional surfaces, deterministic action ids, `participants()` or `None`,
`prompt_section`) is the model for this contract; its chat-shaped parts (`Message`, chat hints,
command regex) are not.

Two waists:
- **Waist 1, core ↔ middleware**: events, capabilities, hooks, context items.
- **Waist 2, middleware ↔ adapters and plugins**: actors, places, audiences, percepts, actions
  and receipts, lifecycle events, answers and commands.

### 2.2 The six facts
Every body, present or future, is described with the same six facts. A fact that does not make
sense for every column does not belong in the contract.

| Fact | Means | Chat | Robot | Workspace app | Sensor | Game |
|---|---|---|---|---|---|---|
| **actor** | who caused an event: an opaque id, or *unknown* | an account | a recognised face or voice | a user | none (the world) | a player |
| **place** | where an event happens and where an action lands | a chat | a room, or the robot itself | a document or workspace | the device | a world or region |
| **audience** | who can perceive what happens in that place; may include *unknown* | the members | the people detected in the room | the collaborators | nobody | the players in range |
| **event** | what happened: when, a description for the model, refs to content, numeric features | a message | "Budi walked into the kitchen" | "paragraph 3 changed" | "kitchen at 31 °C" | "a player waved" |
| **capability** | what the body can do, with generic facts | send a message | speak, move, pick up | edit, comment | read, switch on | walk, build |
| **receipt** | the honest outcome of an action | sent / refused | done / blocked | saved / conflict | ok / timeout | ok / failed |

### 2.3 Waist 1: core ↔ middleware
The core holds mechanisms only. It reads ids as opaque strings, numbers, times and the
description; it never reads a payload or a label.

**Event envelope** (every event, `schema_version` 2; version 1 is upcast on read, *Changing the contract*):

| Field | Read by the core | Meaning |
|---|---|---|
| `id`, `ts` | yes | log id; when it was recorded (injected clock) |
| `occurred_at` | yes | when it happened in the world (the body's clock); equals `ts` when unknown |
| `kind` | yes | the event kind (core kinds unprefixed; `<plugin>.<kind>` otherwise) |
| `who` | as an opaque id | the actor after the middleware's mapping; `unknown` allowed |
| `place` | as an opaque id | where it happened |
| `body` | as an opaque id | the adapter or plugin it came through |
| `caused_by`, `origin_actor` | yes | the chain; `origin_actor` inherited along `caused_by` |
| `description` | yes: rendered and embedded | short natural language for reasoning; any language |
| `features` | yes: numbers only | `{name: number}`, see below |
| `backfill`, `imported` | yes | caught up after being offline / history from before the body was connected (never a wake) |
| `payload` | **never** | opaque data for the middleware, adapters and plugins |
| `labels` | **never** | opaque data attached by the middleware (audience, consent, …) |

**Features** are the only way a fact reaches the gate. The core's signals read the names below;
unknown names are stored and ignored. A new name is an additive change.

| Feature | Value | Set by |
|---|---|---|
| `addressed` | 0 or 1: the turn is given to the soul (a direct place, a reply to the soul, a tag, a wake word) | the adapter |
| `named` | 0 or 1: the soul's name was said without the turn being given to it | the adapter |
| `addressed_other` | 0 or 1: the turn is given to someone else | the adapter |
| `audience_size` | the number of known people in the audience | the middleware |

**Capability descriptor** (declared at registration, through the middleware):
`name` · `description` · `schema` (a closed JSON-schema subset: an unknown keyword fails boot) ·
facts: `communicative` (it speaks to the audience of a place; counts for the talk rate and the
one-message-per-place rule), `long_running`, `identity_changing`, `reads`, `changes`,
`runs_code`, `network` · `permission` (read by the middleware). A capability without facts or
without a permission fails at boot, naming it. There are no exceptions.

```yaml
name: say                   # unique within the adapter or plugin; reasoning sees "<owner>.<name>"
description: Send a message in a place of this body.
schema: { type: object, properties: { text: { type: string } }, required: [text] }  # closed subset
communicative: true         # speaks to a place's audience: talk rate, one message per place per turn
long_running: false         # true → progress and completion arrive as act.result events
identity_changing: false    # read by the owner policy: passes only when origin_actor is the owner
reads: false                # the generic facts; the middleware reads them
changes: false
runs_code: false
network: false
permission: whatsapp.say    # every capability names one; the body declares its base (guest here)
```

**Hooks**: inbound, context, action; `async (item, ctx) -> Continue | Replace | Stop(reason,
held=False)`; every Stop writes `guard.stopped`; an exception or a timeout stops the item and
writes `error`. In the action hook, `Stop(held=True)` means the action waits for someone or
something (an approval, quiet hours) instead of being refused. Everything the middleware decides
reaches the core only through these outcomes; the core reads no label to learn it.

**Context items.** The context is assembled as items, never as text, until the last hook has
run: `{section, text, place, sources: [event ids], kind}` where `section` is one of `recent`,
`elsewhere`, `memories`, `core_memory`, `doing`, `expectations`, `trigger`, `cards`, `notes`.
Hooks pass, drop or annotate items; the core renders the text after the context hook. No hook
ever parses rendered text.

**Actions.** An `act` becomes an action `{id, capability, place, arguments, caused_by}`. The id
is deterministic: derived from the reasoning job and the step (`<job>:<round>:<n>`), so a resume
after a restart derives the same id. `arguments` are opaque to the core except for validation
against the schema.

**Receipts.** Every action ends in exactly one `act.result` with a `state`:
`done` · `refused` (with a reason; never retried; reasoning is told) · `held` (waiting for an
approval or the end of quiet hours; reasoning is not woken; it is released by a later event or
expires) · `failed` (with a reason) · `unknown` (the body cannot tell whether it happened). After a restart, an action without a receipt is handed to
its body again **with the same id**; the body deduplicates by id and answers with the receipt it
already has, or `unknown`. The core never sends anything twice on its own.

Unchanged and described in *Core*: the four books, the five thought acts, the gate,
the budget, the injected clock and the seeded generator.

### 2.4 Waist 2: middleware ↔ adapters and plugins

#### 2.4.1 What an adapter must provide (checked at boot)
A module with one factory `create(options, env)` returning an object with:

| Member | Does |
|---|---|
| `name` | the adapter's name; its capabilities are `<name>.<capability>` |
| `run(put)` / `stop()` | sense and put percepts; stop cleanly |
| `capabilities()` | its own capabilities, with descriptor, facts and permission |
| `act(action) -> receipt` | carry out one action; deduplicate by `action.id` |
| `prompt_section()` | what reasoning should know about this body, as text, at most 1,200 characters ("messages can quote an earlier message", "you have two arms with a 60 cm reach") |
| `self_actor()` | the body's own actor id, or *unknown* |

A missing member fails boot, naming it. **Optional surfaces**, detected and listed in the boot
report:

| Surface | Does |
|---|---|
| `history(place, before, limit)` | older events of a place, for back-fill and import |
| `fetch(locator)` | the bytes of a deferred attachment |
| `render(lifecycle)` | shows a lifecycle event its own way |
| `ask(place, question, choices) -> ref` | asks a question with choices its own way |

A body without `render` simply shows nothing besides its actions; one without `ask` gets the
question as an ordinary action through its communicative capability.

#### 2.4.2 Percepts
What an adapter puts into the queue:

| Field | Meaning |
|---|---|
| `actor` | the native actor id the body observed (`whatsapp:628…`, `face:3`), or *unknown* when it is not certain |
| `place` | the native place id |
| `occurred_at` | when it happened, by the body's clock |
| `description` | short natural language for reasoning, written by the adapter |
| `attachments` | `[{ref, name, mime, size}]` from the attachment store |
| `ref` | the body's own id for this event, so later events can point at it |
| `about` | the body's id of an earlier event this one refers to (a reply, an edit, a reaction, a deletion) |
| `involves` | other actors the event concerns (mentioned, quoted, seen in the image) |
| `features` | numbers |
| `answer` | `{prompt: ref, value}` when the event answers a question |
| `command` | `{name, args}` when a person typed or pressed an operator command |
| `backfill`, `imported` | as in *Waist 1: core ↔ middleware* |
| `data` | anything else the adapter wants to keep; opaque to the core and the middleware |

Facts are written only when they are certain (V1's rule). An adapter that is unsure who acted
reports *unknown*; it never guesses.

```yaml
kind: percept
body: telegram              # adapter name
actor: telegram:123         # the native actor, or unknown; the inbound hook maps it to one id per person
place: telegram:chat:-100   # native place id
occurred_at: 2026-11-11T13:20:05+07:00   # the body's own clock
description: "Budi: anyone know where the meeting is?"   # short, in language; what reasoning reads
features: { addressed: 0, named: 0, addressed_other: 0 }  # numbers for the gate
ref: "812"                  # the body's id of this event
about: null                 # the body's id of an earlier event this one refers to (reply, edit, reaction)
involves: []                # other actors it concerns (mentioned, quoted)
attachments: []             # [{ref, name, mime, size}]
backfill: false             # true when an adapter catches up on what it missed
imported: false             # true for history from before the adapter was connected: recorded, never a wake
data: {}                    # anything else the adapter keeps; opaque to the core and the middleware
```

Other senses use the same shape: `description: "kitchen at 31 °C"` with `actor: unknown`,
`data: { celsius: 31 }`; or `description: "someone came in through the front door"`. Version 1
percepts (`who`, `addressed`, `place_kind`, `sent_at`) are upcast on read: `who` → `actor`,
`addressed` → `features.addressed`, `sent_at` → `occurred_at`; `place_kind` is dropped.

#### 2.4.3 Audiences
An adapter reports who can perceive a place with the event `place.audience {place, actors,
unknown}`, when it first sees the place and whenever it changes. It folds every platform rule
into this one fact: when people who join later can read the past (a Discord channel open to
anyone), it reports `unknown: true`; when they cannot (a WhatsApp group), it reports the members
now. A body that cannot know reports `unknown: true`. The middleware derives everything else
from this fact: a place whose audience is only the owner is the owner's direct place; one person
is a direct place with that person; more is shared; *unknown* means no private item enters.
There is no `place_kind`, `form`, `parent`, `can_post` or platform role in the contract; a name
or a structure the soul should know about is part of `description` or `prompt_section`.

#### 2.4.4 Capabilities and actions
Each body offers its **own** capabilities, with its own names and schemas. Everything rich a
platform offers (quoting, mentions, stickers, buttons, threads, locations, polls, gestures,
voice) is an argument of that body's capability, explained in its `prompt_section`. The
middleware registry lists every capability of every body in a fixed order, and routes an action
to the body that owns its place; an action for a place another body owns is refused with the
reason ("this place is reached through whatsapp"). A body splits, retries, rate-limits and
falls back by itself, and answers with one receipt. A place where the soul may not post answers
`refused` with its reason; there is no fact for it.

#### 2.4.5 Lifecycle events
The middleware tells bodies what the soul is doing, in generic terms; each body renders them
its own way (typing, a read mark, a turned head, a light, a cursor) or not at all. Which ones a
body renders is that body's own option in `config.yaml`; the middleware never knows how.

| Event | When |
|---|---|
| `composing(place)` | a reasoning result is going to speak in that place, until its last receipt |
| `seen(event)` | the soul has taken in an event (recorded and being processed); never for imported or back-filled events |
| `progress(place, step)` | a step of long work: `{capability, state, text}` |
| `thought(place, text)` | a thought, when the owner's interaction setting shows thoughts there and the privacy check passes |

Each lifecycle event passes the privacy check for its place before it is rendered.

#### 2.4.6 Questions and answers
The middleware asks with `ask(place, question, choices)`; the body draws it (buttons, a poll,
speech and listening, a dialog) and returns a `ref`. A structured answer comes back as a percept
with `answer: {prompt: ref, value}`. A free answer in words is understood by reasoning, never by
code: reasoning records it with the middleware's `answer` capability, and the policy accepts it
only from facts (the right actor, the right place, the right prompt), so text written by someone
else cannot answer for the owner.

#### 2.4.7 Commands
An adapter translates its native command form (a slash command, a menu, a button, a CLI line)
into `command: {name, args}`. The middleware handles commands by name; it never parses syntax.

#### 2.4.8 Plugins
A plugin declares everything in its manifest and uses the six
levers. Its tools are capabilities as in *Waist 1: core ↔ middleware*. It sees the
same facts as the middleware and nothing more.

### 2.5 Absolute rules
1. **No code in the core or the middleware tries to understand human language.** No word lists,
   stopwords, keyword matching, substring matching on message text, or regexes over rendered
   text. Understanding is reasoning's job. Strings in code exist only for people to read, and
   come from the language catalog.
2. **Decisions that are not left to the model are made from facts**: actors, places, audiences,
   refs, capability facts, permissions. Never from words.
3. **The core and the middleware never branch on a body, a platform or a device**, and never
   name one. They branch on facts only.
4. **Strict at the edge.** A percept or a declaration that breaks the contract is refused and
   recorded, never patched up. A fact that is not certain is *unknown*.
5. **One rule, one home.** Each rule is checked in exactly one place: privacy in the context
   hook (and on opening or forwarding an attachment), authority in the policy, liveness in the
   core loop. A second check of the same rule elsewhere is refused in review.
6. **The middleware never guesses intent.** What the soul means (which item to share, which
   prompt it answers) is stated by reasoning as an argument and checked against facts.

### 2.6 What the middleware does with the facts
Described in *Middleware*: identity maps actors to person ids, privacy decides what a context may
hold from audiences, the policy holds the absolute lines and the limits, roles give
permissions to person ids, the registry routes actions by place, interaction emits
lifecycle events and asks questions.

### 2.7 Tests that hold the contract
- **The contract is code.** Its types live in one package, `monoagens/contract/`, imported by the
  core (waist 1), the middleware (both) and adapters and plugins (waist 2). The core imports
  nothing else from outside itself.
- **The lock.** `contract.lock` holds a hash of `monoagens/contract/`. A test fails when they
  differ. Only the owner commits a new hash, together with a decision in this file.
- **Tests speak through the contract only.** A test of the core or the middleware feeds percepts
  and checks actions, receipts and events. It never calls a private function (`_name`), never
  asserts on an internal payload, and never asserts on prompt wording. An architecture test
  refuses imports of private names in `tests/`. While the contract is the same, these tests do
  not change.
- **Invariants instead of cases.** Hypothesis stateful tests hold the rules for random places,
  audiences, actors, links, shares and restarts: no context item outside its audience; every
  action has exactly one receipt; nothing is sent twice; every Stop is recorded; a merge never
  splits; the owner's direct place sees everything.
- **Architecture tests** refuse, in the core and the middleware: a body, platform or device
  name; text processing of event text (`lower`, `casefold`, `split`, regexes, word lists); the
  core reading `payload` or `labels`; a capability without facts or permission; the middleware
  importing an adapter or a plugin.
- **One conformance suite** runs over every adapter: every member of *What an adapter must provide (checked at boot)* exists, every
  percept matches *Percepts*, every action gets exactly one receipt, a repeated action id is
  deduplicated, a lifecycle event never breaks the body.

### 2.8 Five test bodies
Before the lock and after every change to the contract, five fake bodies in `tests/bodies/` run
through the real core and middleware with the fake model, **with no change to the contract**:
a chat (private and shared places, quotes, files), a robot (a room, people coming and going,
speech, a gesture capability), a workspace app (a document, collaborators, an edit capability
with conflicts), a sensor (no actor, a value changing past a threshold, a read capability) and a
game (a world, players in range, movement). If one of them needs a new field, the contract is
still shaped by another body, and it is fixed before it is locked.

### 2.9 Changing the contract
- **Additive only**: a new optional field, a new feature name, a new optional surface, a new
  event kind. Readers ignore what they do not know.
- **Experimental first**: a new item is marked *experimental* here and may change; it becomes
  *stable* by an owner decision after it has worked in real use. Stable items never change.
- **Meaning never changes.** A field that would need a new meaning is a new field; the old one
  stays, deprecated. A removed name is never reused.
- **Formats carry their version**; older ones are upcast on read, stored events are never
  rewritten.
- **Who**: only the owner, with a decision recorded here and a new `contract.lock`. A task, a
  worker or a plugin that needs a change stops and reports.
- **A new case is classified before anything is written**: a wrong fact from an adapter → fix
  the adapter; the model misunderstood → the principles or the body's `prompt_section`; a broken
  mechanism → fix it inside, contract and contract tests unchanged; none of these → a question
  for the owner, never a patch.

### 2.10 What contract v2 removes
For the migration (Task 6). Each item below existed before 2026-11-14 and is gone:

| Removed | Instead |
|---|---|
| `place_kind`, place info (`form`, `parent`, `name`, `members`, `can_post`, `roles`) | `place.audience`; names and structure in `description` / `prompt_section`; `refused` receipts |
| `addressed` as a percept field, `sent_at` | `features.addressed`; `occurred_at` |
| one neutral `say` / `react` / `edit` in the middleware, its fields and fallbacks | each body's own capabilities and schemas |
| `contract.SHOWS`, per-adapter ladders, `show:` in the middleware | lifecycle events; each body's own rendering options |
| splitting attachments and sticker fallback in the middleware | the body does it and answers with one receipt |
| `platform_role`, platform role facts | roles on person ids only; a plugin may assign roles through the role commands |
| `yes_words`, stopwords, `text_like`, topic word filters, command syntax stripping | reasoning understands; `answer` and `command` are structured |
| the core reading `words_hidden`, `place_kind`, `audience`, `hold_for`, `delivers` | the middleware decides in its hooks; the core carries what the hooks return |
| regexes over rendered context lines; a second privacy check (`kernel.visible`) | context items with sources; one check in the context hook |
| `share` without an item and delegation guessing | `share` names item or episode ids, checked against the requester's audience |
| re-sending unconfirmed actions after a restart | deterministic action ids and receipts |
| the permission exception for talking capabilities | every capability names a permission; bodies declare theirs (base `guest` for talking) |

## 3. Core

### 3.1 Goals and non-goals

**Goal.** One persistent self (a *soul*) that lives in many apps and devices — chat apps, a terminal,
later devices and robots — remembers, knows when to stay quiet, keeps its promises, is honest
about what it knows, and learns from experience — starting from a basic default prior, with no
tuning needed to begin.

**LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the
brain.**

**Scope**. The core knows three generic things: **percepts** (whatever `put`s
events), **capabilities** (whatever receives `act` and may return results as events) and
**hooks**. It has no notion of adapter, plugin, skill, command, platform, person or owner. It has a slot
for identity, not an identity.

**Non-goals.**
- No emotions, moods or simulated feelings in the core. It imitates cognitive *functions*:
  perception, attention, memory, expectation, judgment, learning, restraint. A plugin may
  add any behavior on top, emotions included.
- No roles or installed skills in the core; they come from plugins.
- Brain names are engineering metaphors.

### 3.2 Mental model

- **Reasoning is the LLM**: it weighs, decides intentions and chooses words. It is woken only
  when a moment deserves it.
- **The core is the rest of the brain**: it records, remembers, expects, pays attention, and
  decides cheaply whether reasoning should wake.
- **Adapters and tools are the senses and hands**: they sense the world and carry out actions in their own way.
- **Hooks are guards at three doors**: they do not think; they apply rules and cannot be talked
  out of them.

The table below is the **brain map**: the closed list of functions the core implements.
Every core rule is a row; formulas and sources are in [research.md](research.md). Scenarios are
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
   and reasoning is woken once more with the newest context.
   **A thought does not end the turn**. Inside one job, the results of `think`, `expect`,
   `resolve` and `revise` go back to the model, which continues until it acts, ends without a
   tool call, or reaches `max_thought_rounds` (prior). A refusal (a thought act or an argument
   the core rejects) comes back as a result with its reason, so the model can correct it in the
   same turn. A turn says at most one message per place: a second communicative act to a place
   the turn already spoke to is refused as already said. Every round is a recorded `model.call`
   and counts in the budget. Thinking, then doing, is one turn, as it is for a person.
7. For each `act`: pass the **action hook**, record `act.out`, hand it to the middleware, which routes it to the tool that owns it, create a
   `predict` Expectation.
8. On ticks: maybe sleep.

### 3.3 Contract

**Four books**.
- `events` — the log, one envelope for every event: id · ts ·
  occurred_at · kind · who · place · body · caused_by · origin_actor · description · features ·
  backfill · imported · payload · labels. INSERT-only by trigger; the single exception is hard
  forgetting. `who` is an opaque actor id (the inbound hook maps native actors to one id per
  person), `place` an opaque place id, `body` the adapter or plugin name; `origin_actor` is
  inherited along `caused_by`. The core reads ids, times, `description` and `features`, and never
  reads `payload` or `labels`. `event_vectors` (event_id, embedding) is the log's index; rows
  leave only by fading or hard forgetting, each through one dedicated path that writes an event.
- `state` — a namespaced key/value projection that can be rebuilt from events: `identity`,
  `last_seen:*`, `last_spoke:*`, `core_memory`, `self_summary`, `habit:*`, `person:*`,
  `alias:*`, `merged:*`, `credence:*`, `cognition:weights`, `cognition:dispositions`,
  `budget:<date>`, and `<plugin>:*` namespaces.
- `expectations` — *Expectations*. `memories` — *Memory, sleep and forgetting*.

**One door, one language, one way to remember**. Everything that reaches the brain is an
event; everything sensed is a `percept` carrying a short natural-language description plus
optional raw data, whatever the sense; the core knows no modalities. Everything that
persists, in the core or in a plugin, persists as events; state is their projection.

**Five thought acts**: `think`, `act`, `expect`, `resolve`, `revise`. `act` uses any
tool capability, speaking included. A wake that ends without `act` is silent; its reason lives
in `think` and the decision event.

**Three hooks**: **inbound** (after recording), **context** (before any model call),
**action** (before a tool acts); their contract is in *Waist 1: core ↔ middleware* (fail closed).
**Watchdog**: every hook call has a time limit (`hook_timeout_ms`); a hook that exceeds it
counts as Stop and writes `error`, and the loop goes on. Background jobs (reasoning, classifier,
embedding, sleep) have their own limit (`job_timeout_seconds`); a job that exceeds it returns
`job.result` with an error. Adapters and tools never block the loop: `act` is handed over and answered by
`act.result`. One plugin can fail; it cannot freeze the soul.
One ordered list per hook: owner policy first and last, everything else in between, *shared*
(every adapter that opts in) or *adapter-own*. Name collisions fail at boot.

The middleware acts on the core only through the six levers.

**Schema changes** are numbered SQL migrations with a version table, applied at start after an
automatic backup; no migration framework.

**Snapshots** (built once real logs show rebuilds are slow). `state` is a projection of the
log; rebuilding it from the first event gets slow over years. The core periodically writes a snapshot of `state` with the id of the last event
it includes (every `snapshot_every_events`); a rebuild loads the latest snapshot and replays only
later events. Snapshots are caches: deleting them loses nothing. Hard forgetting invalidates every
snapshot that contains the forgotten data; a new one is written from the cleaned log.

**Stability**. What middleware, adapters and plugins may rely on is *The contract*, plus the
six levers, the read-only view of the log and state, `recall`, the birth
certificate and the prior format; it changes only by *Changing the contract*. Not part of it:
code modules and functions, database tables and indexes, prompt wording, the values in a prior.
Plugins put their own events under their own namespace (`<plugin>.<kind>`); only the core defines
unprefixed kinds.

**Event kinds** (each with a `schema_version`; plugins use `<plugin>.<kind>`, where `<plugin>` is the plugin's name with `-` written as `_`, since event kinds hold no hyphen). percept · command · act.out · act.result · decision.silent · decision.note ·
decision.wake · model.call · budget.exhausted · expectation.created · expectation.fired ·
expectation.reminded · expectation.resolved · job.started · job.result · classifier.verdict · credence.updated ·
disposition.updated · share.granted · source.merged · place.audience · task.opened · task.closed · guard.stopped · forget.soft · forget.hard · fade.done · sleep.done ·
sleep.failed · error. Every `act.out` ends in exactly one `act.result` with a `state` (`done`,
`refused`, `held`, `failed`, `unknown`; *Waist 1: core ↔ middleware*).

**The model port**. Reasoning builds neutral messages and tools and calls one injected
model port, as the embedder and the store are injected; the core knows no provider, endpoint
or header. Provider drivers live in `infra/providers/` and are chosen by `.env`
(`LLM_PROVIDER`, `LLM_FALLBACKS`); every call still goes through the gate, is recorded and
counts in the budget. A provider is not an adapter: it is where the soul thinks, not a body
that senses or acts.

### 3.4 Adapters, tools and capabilities

Adapters and plugins bring what senses (`run(put)`) and what acts (`act(action)`). Each tool declares its
capabilities: name · JSON schema · description · facts · permission. Reasoning's tools are
the five thought acts; `act` offers the capabilities of every running tool, not only the one of
the adapter the current percept came from, listed in a fixed order so the prompt cache holds. The soul can act in any adapter from any adapter. Each body offers its own
capabilities with its own names and schemas; the middleware's registry routes an action to the
body that owns its place; the core only sees the entries
the registry offers.

- The brain decides intent and content; the adapter decides how (chat bubbles, TTS, gestures).
  Fine control stays inside the adapter.
- Long actions report progress and completion as `act.result` events; the core never waits.
- Multi-step work is a chain: result → circuit breaker (it continues an active goal) → next
  `act`, limited by steps per task (`max_task_steps`) and the budget. Every step is logged, so
  tasks resume after a restart and can be interrupted.
- **Working memory of its own acts**. Every chain in progress — in a task thread or in the
  place where it started — is in the context in `<doing>`: the goal, one line per step (the soul's
  own acts included, also those that say nothing), the latest result verbatim, older results as
  pointers. The soul always knows what it has just done and what came back.
- **Task threads**. A chain that needs more than one step opens its own task thread: a
  place of kind `task` with a parent place, the parent's labels (so absolute line 2 holds
  unchanged), and its own slot for one reasoning job. The conversation in the parent place never waits
  for the work. The thread reports to the parent when it finishes, fails, or needs to ask, and
  closes. Open threads are limited (`max_open_tasks`); one more waits as a promise. One soul,
  one log: threads are lines of work, not copies of the self.
- `who`, `place` and `origin_actor` are opaque strings to the core. The inbound hook
  (middleware) may rewrite them — for example one id per person across accounts — and attach
  labels; the core stores them as given and never reads their meaning. Commands are handled
  in the middleware; one command registry makes `/help` identical everywhere.
- Adapters report `addressed`, `named` and `addressed_other` as features; the core
  reads them as numbers.
- What an adapter senses enters as a `percept`: the adapter, like a sensory cortex, turns its signal
  into a short description, with raw data attached.
- On start an adapter asks for `cursor(organ)`, so input is never lost or duplicated.
- `communicative` capabilities count as speaking for the talk-rate drive and the forward model.

**Reaching a target**. A communicative act may name a target that is not a place
(`to:`, optionally `via:`). The middleware's action hook resolves it to a place or refuses with
its reason (*Policy*, Routes); the core only delivers to a place. When several places fit, the soul
asks which one instead of guessing: a message sent to the wrong place cannot be taken back.

**Speaking elsewhere**. A communicative act aimed at a place other than the current one is
never sent with text written in the current context. The core turns it into an intent — what to
say, to whom, why — and wakes reasoning once more in the target place, with that place's context
(absolute line 2); the intent is that wake's trigger, so reasoning sees what it set out to
say and why; the message is written there, as it would be written in Budi's chat.
This costs one extra model call and only for messages to another place. What the intent carries
is what the action hook returns (the rule is *Policy*, Speaking elsewhere); the core reads no label
for it.

**History and catching up**. An adapter may offer `history(place, before, limit)` for
messages the log lacks. On connect it back-fills what it missed as `percept` with
`backfill: true` and `occurred_at`. A backlog of one place is appraised once, as one percept with
every signal discounted by age (ACT-R decay); an addressed message still trips the circuit
breaker. History from before an adapter was connected is an **import**: percepts with
`imported: true` are recorded, embedded and searchable through `recall`, but are never appraised
as a wake, never trip the circuit breaker, and are summarized by sleep only for places the soul
later becomes active in, so a long imported history costs no model calls until it matters.

**The `self` tool** is the only tool inside the core. It offers one
non-communicative capability, `recall(query | event_id, place?, from?, to?)`: a deeper search of
the soul's own log and memories — by time range (no query lists the episodes in that range),
along `derived_from` to the original events, and by text search over event payloads, so faded or
unsummarized messages are still found. Its results pass the context hook like everything else,
and it reports what it searched and found, "nothing" included. Sharing private content with
someone new (`share`) is a middleware capability, not the core's.

### 3.5 Expectations

One schema for every promise and prediction:
id · kind · about · who · place · cue_time · cue_event · condition · condition_text ·
condition_vector · intent · weight · policy · status · next_check_at · last_fired_at ·
fire_count · origin_actor · derived_from · expires_at (required) · created_at

| kind | used for | wakes reasoning? |
|---|---|---|
| `promise` | promises, reminders, approvals awaiting the owner | yes, after the wake gate |
| `wait` | patience: does someone else take the floor first? | yes, if nobody does |
| `predict` | forward model after `act` ("a reply within 30 min", "arrived within 30 s") | only on a miss that matters |
| `guard` | an important belief ("Budi lives in Bandung") | never by itself; see *Memory, sleep and forgetting* |

- `cue_time`: at · every · daily. `cue_event`: kind, who, place, not_who — facts only; whether a
  line says something is a free-text condition, judged by reasoning.
- `condition`: exact fast paths (`none`, `event_seen`, `state`) or a free-text
  `condition_text`.
- `policy`: once, or cooldown + max_fires. A fifth kind needs a new decision.

**Wake gate** (no reasoning call): *still relevant?* (condition already met → resolve) · *stale?*
(past expiry → drop). Quiet hours are owner policy in the hooks; the budget acts through θ;
repeats are limited by `policy` and habituation.

**Free-text conditions**. m = cosine(event vector, condition vector). Cosine only
filters: m < `lo` → ignore; m ≥ `lo` → the Expectation is *reminded*: reasoning wakes once with
the line and the Expectation as its trigger, and judges — act on it, resolve it, or let it be. A reminder is shown as a reminder, never as a fulfilment: the line with its own speaker,
the Expectation's condition, and the question whether this line fulfils it. A person hears something, is reminded of a promise and thinks for a moment; there is no
separate judge. `lo` comes from the prior.
**Deferred until real logs**: a cheaper classifier call in place of the reasoning wake, when logs
show the cost matters; the "clearly yes" band (hi, opened only by the rule of three); learning
`lo`; caching condition vectors.
Woken reasoning re-judges the situation; it never replays a promise blindly.

### 3.6 Cognition

**Circuit breaker** (wake without a draw): the event engages a commitment the soul holds —
the turn was allocated to it (`addressed`), it fires or answers an active Expectation, or it
continues an active goal (a result of its own action).

**Signals** — pure functions `(Situation) -> [0, 1]`, at most 10 (now 5). `Situation` holds the
event, its vector, recent messages here, memory candidates with activation, and relevant state.
- `own_name` — the `named` feature: its name was said (attention capture, not a turn
  allocation); the adapter reports it, the core never searches text
- `info_gap` — cosine to reference sentences in the prior (not keywords)
- `feeling_of_knowing` — σ((max activation − τ)/s), recent messages included
- `relevance` — to active promises and core memory
- `social_cost` — the floor belongs to someone else (the `addressed_other` feature, or already
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

**Inner thought.** When the soul self-selects, reasoning calls `think(thought, motivation 1..5,
confidence 0..1)` before any communicative `act`; the act passes only at or above
`speaking_bar` and `certainty_bar`. Criteria offered (Inner Thoughts, CHI 2025): relevance,
information gap, expected impact, urgency, coherence, originality, balance, dynamics.

**Budget.** Not a mode: as the daily budget is spent θ rises, so only salient events
pass; the circuit breaker still fires; a free-text condition with no affordable wake
stays unresolved; `budget.exhausted` is recorded.

**Knobs.** All tunable behavior is about ten named knobs in the prior (talkativeness, patience,
reserve when unfamiliar, …); one function maps them to θ, τ, γ and the weights.

**Learning**.
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
  of real logs**: learned knobs and signal weights, which come from the prior until then.
  Because the log keeps everything, learning can later run over the whole history.

### 3.7 Memory, sleep and forgetting

`memories`: id · who · place · private · kind (fact | episode | lesson | procedure) · basis
(observed | told | inferred) · text · embedding · tsv · importance · source_who · valid_from ·
valid_to · superseded_by · derived_from · created_at · last_accessed_at · use_count

- **A memory's audience comes from its sources**. The core, not the model, sets `place`
  and `private` from `derived_from`: a memory is private when any of its source events is, and
  its audience is the people every private source was said to. A memory with no known source is
  private to the owner. The same holds for core memory. Absolute line 2 never depends on a model
  filling in an argument.

- **Activation (ACT-R).** B = ln(n/(1−d)) − d·ln(L); A = B + w_ctx·cos(context, memory) +
  w_imp·importance; L in hours from the injected clock. Candidates: top 40 by vector ∪ top 20 by
  text; ranked by A; use_count updated.
- **Attribution**. Memories say who said it ("Budi said X"). A non-owner's claim never
  supersedes the owner's fact; conflicts stay as conflicting claims.
- **Knowing and misremembering** (see research.md), modeled on brain functions:
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
- **Sleep** (a gated background job, only on new conversational events, failures back
  off, imported percepts count only in places the soul has become active in). Summarize new
  events per place into memories with `derived_from`; update person cards (closeness from
  interaction activation; reliability per domain as α/β), core memory and `self_summary`; distill lessons and procedures (generic → layer 2 with credence, person- or
  place-specific → layer 3); check `guard` beliefs; drop expired Expectations; fade vectors.
- **Belief maintenance**. A contradiction of a belief is a prediction error weighted by
  precision (the belief's credence × the new source's reliability): relevant now → reasoning sees
  it live; otherwise it is settled in sleep.
- **Self-correction**. `revise(old, new, reason)` sets `valid_to` and `superseded_by`;
  nothing is deleted. A stranger's correction of an owner fact becomes a conflicting claim. The
  soul never revises its own principles or persona.
- **Fade**. During sleep, per-message vectors with low activation are released; the
  gist stays in memories, the event stays in the log. Deterministic, fixed threshold for now.
- **Soft forgetting** ("forget what I just said"). Related memories get `valid_to`; events are
  excluded from retrieval and `<recent>`; `forget.soft`.
- **Hard forgetting** ("delete my data"). `forget_hard()` replaces payloads with
  `{"forgotten": true}` and deletes everything in their `derived_from` lineage (vectors,
  memories, expectations). Metadata remains (kind, time, actor hash); `forget.hard`. Asked by the
  owner or the person concerned. Owner backups are allowed; old backups and the model provider's
  logs are outside the soul's reach, and the user documentation says so.

**Learned state** has three layers, all in the core: (1) dispositions (knobs),
(2) generic procedures, lessons and social intuition, (3) episodic memory with per-place and
per-person tuning. Test for 1–2: "would it make sense for a soul that does not know these
people?" — otherwise it is layer 3. Installed skills are not learned state; they belong to
plugins. A soul is born from a **prior** (layers 1–2, *Prior (format version 1)*); this repository only
loads a prior and never exports or shares learned state.

### 3.8 What the core knows, the absolute lines and the self-model

**Only opaque ids, labels and numbers**. The core does not know what a person, the owner, a
group or a role is. It carries:
- **ids** — `who`, `place`, `origin_actor` (the source a chain started from), as strings;
- **labels** — attached by the middleware in the inbound hook (for example the audience when a
  line was said) or by a hook outcome (for example `share.granted`), stored with the
  event and never read by the core. A memory carries the labels of every
  event it was derived from (`derived_from`); a thread carries its parent's labels;
- **numbers** — what the gate needs, as `features`: whether a line is
  addressed or names the soul, how many people the audience holds, how many distinct sources
  are active in a place.
What the soul learns about a source in sleep (closeness, reliability: theory of mind) is its own
memory, kept under that opaque id.

**Two ids become one**. When the middleware finds that two source ids are one source, it
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
be reached — is middleware.

**The two absolute lines** are held deterministically by the owner policy, never by
reasoning: line 1 in the action hook, on capabilities flagged `identity_changing`; line 2 by
construction before the model — the one context check, speaking elsewhere through a wake in the
target place, the attachment audience — with the rules in *Policy*.
The owner policy is always installed, first and last in every hook; the core refuses to run
without it (fail-closed), so both lines hold with no other middleware. Other middleware and
plugins may only narrow them.

**Everything else is weighed by the reasoning**. Anyone may ask; requests are judged
against the owner's interests and the principles. Learned procedures and lessons carry credence
that the reasoning sees. Claims are weighed by the speaker's reliability in that domain.
Standing never buys dishonesty.

**The soul's own identity**. A soul is born from a birth certificate (*Birth certificate*; a neutral
default if none) and a prior. After birth, identity lives in `state: identity` and changes
only through an `identity_changing` capability, which the owner policy lets only the owner use;
without a birth certificate the soul may ask what to be called. Role plugins may provide a birth
certificate, used only at birth.

**The context and the self-model**. The context is the prompt, not the memory: it is rebuilt
from the books at every wake and never compacted. It is assembled as **items** — each with its
section, text, place and source event ids — and passes the context hook as items; the text is
rendered only after the last hook, so no hook ever parses rendered text. Three layers, ordered for caching:
- *Stable*: who I am, how I work (memories can be incomplete; I can be wrong; I act only through
  adapters and tools), honesty principles, knowledge cutoff (from per-model configuration, set slightly
  earlier than claimed), principles, thought acts and capability schemas.
- *Semi-stable*: `<self>` (capabilities, commands, installed skills, plugins' standing rules;
  assembled from the registries; closed-world — what is not listed cannot be done), core memory,
  cards of the sources present or named in the trigger: what the soul has learned of them,
  under their id, plus what the middleware adds through the context hook — name and where they
  can be reached. A card is never what someone said; knowing someone is not private content, so a
  card is never withheld by absolute line 2. This is how the soul can reach Budi from a place
  where Budi's messages may not appear.
- *Volatile, last*: `<now>` (local time, timezone, time since the last interaction), `<place>`,
  `<why_awake>`, `<doing>`, `<next>`, `<memories>` with basis, source and age, `<recent>`,
  `<event>` with any middleware notes attached to it. Messages from non-owners are delimited as
  quoted data (spotlighting); the absolute lines never depend on it.
Each volatile section has a token budget (prior constants). In a multi-step chain `<doing>`
holds the goal, one line per step and the latest result verbatim; older results are pointers the
soul can `recall`.

**Honesty principles**, few, general, worded by reasoning and never scripted:
- speak as a person about what you know — sure, unsure, don't know, don't remember — without
  describing your own machinery unless asked;
- the more an answer matters, the surer you must be; otherwise be less specific or say you don't
  know; for changing facts use a search tool if present, or answer as of your cutoff;
- your view is partial: absence weighs only as much as you would have seen it;
- do not agree only to please, do not insist when you may not have seen it;
- ask about a gap only when it matters; never claim a capability outside `<self>`.

### 3.9 Formats the core reads
Changes to a format bump its version; the formats are frozen with the core, the *values* in a
prior are not. Other formats: installed skill, plugin manifest.

#### 3.9.1 Capability descriptor
In *Waist 1: core ↔ middleware*, with an example.

#### 3.9.2 Percept
In *Percepts*, with an example.

#### 3.9.3 Birth certificate (`soul.yaml`, format version 1)
Who the soul is, read once at birth. After birth, identity lives in `state: identity`; the
file is not re-read, and only the owner changes identity, in conversation. Without a birth
certificate the soul is born neutral and may ask the owner what to be called. Role plugins may
ship one, used only at birth.

```yaml
version: 1                  # format version
name: Mona
language: id                # the language it speaks with the owner
persona: |                  # any language
  Calm, honest, straight to the point. Speaks only as much as needed.
principles:
  - Never lie and never invent facts.
  - Keep the owner's private matters private; never bring them up in other places.
```

Nothing else belongs in it: the owner is found by pairing; secrets live in `.env`;
deployment settings — timezone, budget, quiet hours, adapters, plugins — live in `config.yaml`; every behavioral number lives in the prior.

#### 3.9.4 Prior (format version 1)
A soul's instincts: a plain folder with layers 1–2 of learned state, loaded at birth. It never contains layer 3 (episodic memory, people, places, personal details). End
users never see it; developers tune it with replay reports, not by guessing numbers. The default
prior is [`prior/`](../prior/) in the repository; its values are data, tuned with replay, and are
not part of the frozen design. This repository only loads a prior; it never exports, uploads,
shares or aggregates learned state.

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

**Loading rules**:
- different embedder → `calibration` falls back to the default prior's;
- different LLM family → load, but halve the α and β of procedures and lessons until proven
  again;
- unknown major `format_version` → refuse with a clear message;
- replay scenarios that prove a prior live in the repository's `scenarios/`, not inside it.

### 3.10 Deferred
Until weeks of real logs exist: learned signal weights and knobs, adaptive fade
threshold, social standing, learned coverage; research on consolidation and forgetting formulas.
These are learning *inside* existing functions, so enabling them does not change this design.

## 4. Middleware

### 4.1 The foundation in the middle
The regulator between the core and everything else. It works on the six facts of the
contract and nothing else, under the absolute rules of the contract. In addition:
- **Its own words** for people (an approval question, a brake notice) come from the message
  catalog (`monoagens/middleware/lang/<code>.yaml`, English as the fallback) in the soul's
  language.
- **Only hooks and levers.** It acts through the three hooks and the six levers; it never
  replaces or wraps a core function or attribute.
- **State only through events** (lever 4): anything it must remember across a restart (a pending
  question, a mark of what was seen) is an event or its projection, never a variable in the
  running process.
- Architecture tests hold these rules.

The middleware runs in the core's three hooks and offers one API. It is static and never learns — learning belongs to the soul.

```
   Plugins (Soul Tuning, Web Search, Workspace…)  Adapters (CLI, WhatsApp, a robot, an app…)
                   \                                  /
                    ──────── contract, waist 2 ───────
                                 Middleware
  policy · privacy · identity · registry · loader · interaction · roles
                    ──────── contract, waist 1 ───────
                       Core (ids, numbers — mechanisms only)
```

Seven files, one concept each; the contract types themselves live in `monoagens/contract/`. A file that grows becomes a folder of the same name; the concepts stay.

| Module | File | What it provides |
|---|---|---|
| Policy | `policy.py` | the owner policy, first and last in every hook: the identity line, budget, quiet hours, brakes; whether a call passes, asks or is refused |
| Privacy | `privacy.py` | absolute line 2: audience labels, the one context check, `share`, attachments' audience |
| Identity | `identity.py` | actor map (one person id per person), pairing, linking as a merge, person cards, routes (`to:`) |
| Registry | `registry.py` | capabilities, commands, skills: one list, routing by place, what a requester may use, `/help` |
| Loader | `loader.py` | reads `config.yaml`, `config.d/` and manifests; loads adapters and plugins; the boot checks |
| Interaction | `interaction.py` | lifecycle events (composing, seen, progress, thought) and questions asked and answered |
| Roles | `roles.py` | what each person may do: roles with a base tier, permissions, approval modes |

**What the middleware itself offers.** Only what regulates the middleware: `share` and `answer`
(capabilities for reasoning), pairing, `/link` and `/unlink`, budget and quiet-hour settings,
`/help`, `/plugin`, `/reload`, `/whoami`, `/whois`, `/show`, `/role` and `/roles show`. They are
gated by permissions like any tool. Anything that is a feature (search, files, `/mode`) is a
plugin. Test: if removing it would leave a rule that can no longer be kept or changed, it belongs
to the middleware; if the soul would only be less able, it is a plugin.

### 4.2 The API: six levers
Every plugin and adapter uses the same levers, so the core never knows the case — modes, a
persona layer, simulated emotions, RBAC and moderation all fit:
1. **Pass, replace or stop** an item at a door (percepts, context items, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to reasoning, inside its token budget, before the
   policy's last pass.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember and observe**: write only by putting events (state is a projection under
   `<plugin>:*`, so replay and hard forgetting still work); read the log and state read-only
  . The view returns raw content: a tool that shows it to people passes it through
   `privacy.visible` for the requester's place first.
5. **Sense and act** through its own tools.
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated` with the
   plugin as source; learning never moves an offset, and uninstalling removes it. The middleware
   computes the effective knobs: the prior's value (later the learned value) plus every active
   plugin offset, clamped to the manifest's bounds and [0, 1], recomputed at boot, reload and on
   every new offset.

Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, adds to the
contract, or writes the core books except through events.

**Order in each hook**: `[policy — first] → [middleware modules] → [plugin middleware, in
install order] → [policy — last]`. Name collisions fail at boot.

**What a failure does, by layer.** Every failure is recorded and logged; none is silent,
none is swallowed by a blanket `except`. What happens next depends on what failed, decided in one
place:

| Layer | On failure | Examples |
|---|---|---|
| Guard rule (policy, privacy, identity, roles) | the item stops (fail closed), `guard.stopped` or `error` | the context check, an attachment's audience, a permission |
| Best-effort rule (lifecycle events) | `error` naming the rule; the item **continues** | a body failing to render `composing` or `seen` |
| Adapter | `error` naming it; that adapter alone restarts with backoff | a timeout, a lost connection |
| Tool or plugin call | a receipt with `failed` and the error type, never its text (it may hold a secret) | a search that failed |
| Core loop | `error`, then the next event; never `continue` without a record | a failed insert |

Every rule declares its kind when installed: **guard** (the default) or **best-effort**; the
composer runs best-effort rules in their own wrapper and timeout, so the hook contract (fail
closed) is unchanged. A fault-injection test makes each adapter member, each best-effort rule and
each tool fail in turn and checks that the soul keeps working and records the error.

**A refused act is not the end of the turn.** A refusal returns to reasoning as that act's
receipt (`refused`, with its reason), and reasoning may do something else within the thought-round
limit, the budget and the brakes. Quiet hours and approvals hold the act instead and do not wake
reasoning again.

### 4.3 Policy
The one middleware that is always present.
- Wraps every module and plugin, first and last, and cannot be removed.
- Enforces the two absolute lines deterministically: the `identity_changing` check on
  `origin_actor` in the action hook, and the privacy rule below in the context hook. The core refuses to run without this policy (fail-closed).

**Private content (absolute line 2).** Privacy follows people, not places,
and needs one fact: the audience.
- **Labels.** In the inbound hook every event is labelled with its place's audience at that
  moment, after the actor map: the people who could perceive it then. A newcomer never receives
  what was said before they could perceive it. A memory's audience comes from the events it was
  derived from; an attachment's from the events that carried it.
- **The one check.** A context item may enter the context of a place only if every member of
  that place's audience now is in the item's audience. An audience that includes *unknown*
  lets no private item in. The owner's direct place — a place whose audience is only the owner —
  sees everything. Content from places whose audience was *unknown* when it was said is not
  private to anyone and flows freely.
- **Where it runs.** Once, in the context hook, over every context item (its sources give its
  audience), and when an attachment is opened or forwarded. An item that fails is dropped; when
  relevant items were withheld, one line without content says how many and from whose private
  place. Every lifecycle event that shows content passes the same `visible` call.
- **Before the model, by construction.** The model never sees private content it may not
  use in the place it speaks to, so it has nothing to leak. There is no check after the model and
  no text similarity of any kind.
- **Speaking elsewhere**. An act aimed at another place becomes an intent that wakes
  reasoning in the target place. In the action hook the policy decides
  what the intent carries: the drafted words only if every private item the source context held
  is visible at the target; otherwise only what to do and to whom. The intent never carries the
  goal's or the trigger's text.
- **Sharing by consent.** `share(items | episode, to)`: reasoning names the event ids (or
  an episode id) it means; the policy passes only when the chain's `origin_actor` is in the
  audience of every named item, records `share.granted` for exactly those items, and the new
  person joins their audience. A share that names nothing is refused with a question. Each
  person speaks only for what they were part of; the owner's view of everything is a view, not
  ownership.
- **Asking on someone's behalf.** When the soul sends a message on behalf of a person X,
  the one event that answers it — the event whose `about` is that message, or the one that meets
  that chain's `wait` expectation — is labelled with X in its audience too. Never the person's
  later events. X may then share it.
- **Data leaving the machine**: by default nothing does; only a plugin the owner installed
  for it, sending numbers, not content.
- **Privacy rules narrow, never widen.** Other middleware and plugins may add stricter rules; they
  can never let private content reach someone the owner policy would refuse. They may ask for
  more than consent before sharing (confirm before forwarding any file).
- **Routes**: the action hook resolves a target that is not a place (`to:` a person,
  optionally `via:` a body) to a place: a place of that body whose audience is that person and
  the soul, else the place where the person was last active. Several fitting places → the soul
  asks which; none → a `refused` receipt ("no way to reach Budi yet"). A route rule may change the
  default.

**Default owner policy**, active out of the box: the two absolute lines, the daily model budget,
quiet hours, brakes. Budget and quiet hours are owner settings in `config.yaml`, changed by hand or
with `/budget` and `/quiet`, which write their own line.

**Brakes on runaway work**, deterministic, no model call: at most `max_task_steps` steps per
chain and `max_wakes_per_hour` wakes per chain (`config.yaml` → `owner_settings`). When a brake
holds, the chain stops, it is recorded, and the owner is asked in the owner's direct place whether
to continue; nothing stops silently.

### 4.4 Registry
Capabilities, commands and installed skills follow one pattern, as MCP's tools, prompts and
resources do: a list of names with short descriptions, the full entry on request, one namespace,
collisions failing at boot.

- **Capabilities belong to their body or plugin**. Each adapter and plugin registers its own
  capabilities, with descriptor, facts and permission. The
  registry lists every capability in a fixed order and routes each action: a body's capability to
  that body, for a place it owns; an action for a place another body owns is refused with the
  reason. There is no neutral `say` and no hidden capability.
- **Every capability names a permission.** Adapters declare theirs like plugins (a talking
  capability usually at base `guest`); a capability without facts or without a permission fails
  at boot, naming it. A permission without a `base` is the owner's only.
- **Commands are structured.** An adapter turns its native form into `command: {name, args}`
 ; the registry handles it by name in the inbound hook, without
  a model call. `/help` is the same everywhere. A handler that fails answers with an error. A
  command's answer goes through the action hook like any act.
- **Offered per requester.** In the context hook the registry offers a turn only the capabilities
  the requester may use (`roles.py`); in the action hook `policy.py` decides pass, ask or refuse
  from the requester's permissions, the capability's facts and the requester's approval mode.
  The requester is the chain's `origin_actor`.
- **One output marker** (`contract.OUTPUT_MARKER`) for every cut output; **nothing is lost when
  an output is cut**: the full output goes to the attachment store with its `act.result` as
  origin; the result carries the cut text plus `full: {ref, size}`. `registry.more(ref, offset,
  limit)` reads the rest, capped, through the attachment privacy check; without a place it is
  refused.
- **Draft**: per-person rate limits live here.

### 4.5 Install
- The loader reads each plugin's manifest and each enabled
  adapter's factory, assembles capabilities, middleware, commands, skills, rules, knob-offset
  bounds and state namespaces, and refuses to start without the owner policy first and last in
  every hook.
- **Boot checks**: an adapter missing a member of *What an adapter must provide (checked at boot)*, a capability
  without facts or permission, a schema outside the closed subset, a permission declared outside
  its owner's namespace or redefining a middleware permission, a missing dependency, a name
  collision — each fails boot, naming it. The boot report lists every adapter's optional surfaces.
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model);
  the owner policy holds against plugins that follow the contract, not against malicious code
 .
- The installed plugins are listed in `config.yaml`; each plugin's own settings live in
  `config.d/<plugin>.yaml`, holding only what the owner changed; defaults live in the manifest
 .
- **Format versions**: *Deployment files*.
- **The files are the source of truth**. The program writes them only on the owner's
  command, after a backup, atomically, and each edit is parsed back and checked or refused:
  `/plugin enable|disable|update <name>`, `/role add|remove <person> <role>`, `/budget`, `/quiet`,
  `/show`. Every other line, comments included, stays as it is. Every change, by command or by
  hand, is recorded as a history event.
- **`/reload`** validates the whole new configuration first and applies it only if everything is
  valid; running adapters are not created again; otherwise it refuses with a clear message and
  the running configuration stays. It names what needs a restart.
- **Plugin updates, the dpkg way**: a default the owner never touched is offered for
  `/plugin update`; one the owner changed is only reported.
- Adapters are loaded from `bodies:` in `config.yaml` (the key keeps its name) through each
  adapter's `create(options, env)`.
- Uninstalling removes a plugin's capabilities, middleware, rules and knob offsets; its events stay
  as history.

### 4.6 Skills
Part of the registry.
- **Installed skills** are static SKILL.md files shipped by plugins. Only the names and
  descriptions of relevant skills enter the prompt; the body is loaded on request; versioned; use
  counts feed the same ACT-R activation as memories.
- Skills do not learn; what the soul learns is a core `procedure` or `lesson` memory.

```markdown
---
name: shop-customer-service
description: Answer customer questions for a small shop politely and accurately.
version: 1
---
Steps …
```

### 4.7 Identity
**One person, one id**. The core never sees a native actor, only a person id.
- **The actor map** (`state: alias:<actor>` → person id): the inbound hook maps every native
  actor an adapter reports — the event's `actor` and its `involves` — to a person id. *unknown*
  stays *unknown*.
- **Registration is automatic.** An actor seen for the first time gets a new opaque person id
  (`person:p_7f3a`), recorded as `identity.registered {actor, person}`. One actor has one id (a
  unique constraint). The owner's id is always `person:owner`.
- **Linking is a merge.** Linking two actors of one person merges their person ids into the
  **older** one (`person:owner` always wins), recorded as `identity.merged {from, into}`; the
  middleware puts the core's `source.merged {from, into}`. The log is
  never rewritten; reads follow the merge.
- **Who may link.** The owner (`/link <actor|person|name> <actor|person|name>`), or the person:
  `/link` from actor A gives a six-digit code (single use, 10 minutes); `/link <code>` from actor B
  merges B into A's person. Wrong guesses are counted per guessing actor (three in the code's
  lifetime) and never touch other people's codes; the owner cannot redeem a code someone else
  issued. The soul may suspect and ask the owner; it has no tool that links. A wrong link is a
  privacy leak.
- **Unlinking** (`/unlink <actor>`) gives that actor a new person id from now on
  (`identity.unlinked`). What was merged stays merged.
- **A body's own actor**: when an adapter's `self_actor()` maps to the owner (the soul runs
  on the owner's own account), identity adds one context line for that body: acting there acts as
  the owner. Reasoning weighs it; no rule branches on it.
- **Becoming the owner**: on first start the soul shows a short pairing code; the first
  person who sends it from any body becomes the owner (three wrong tries per actor). Changing the
  owner later needs the current owner.
- **Person cards**: through the context hook, for the people present or involved in the
  trigger, their name and where they can be reached, next to what the soul has learned of them;
  `roles.py` adds each role's and the person's `context`, read at call time.
- **`/whoami`**: the asker's person id, label, linked actors, roles with tier and approval mode,
  and this place's description. In a place whose audience is more than the asker, linked actors
  go to the asker's direct place, never to the group. **`/whois <name or actor>`**: another
  person's id, label and linked actors; permission `identity.whois`.

### 4.8 Roles and permissions
`roles.py` answers **what this person may do**; `policy.py` turns that into pass, ask or refuse.
Always present; empty by default: the owner may do everything, everyone else only talks.
Modelled on Discord roles (several per person, permissions add up, `@everyone` as the floor) and
on the base roles of GitHub and Google Cloud.

- **Four fixed tiers**: `guest < member < trusted < admin`; the owner is above all and always
  free. Plugins and adapters know only the tiers, never the owner's role names.
- **Permissions are declared** by plugins and adapters in their manifest or registration, each
  with a label, a description and the lowest `base` tier that gets it; a permission without a
  `base` is the owner's only. `role_settings` may give a value per tier.
- **Roles are defined by the owner** in `config.yaml` → `roles:`: a **tier role** has a `base`;
  a **group role** has none and gives nothing but what it lists; either may add `allow`, `deny`,
  `settings`, an `approval` mode, a `label` and a `context`. An `allow` gives exactly the listed
  permissions, never a tier.
- **People** (`config.yaml` → `people:`), keyed by person id: `p_7f3a: { label: "Budi", roles:
  [member, toko-a], context: "The owner's brother." }`. `roles` is the only record of who holds
  which role; a person not listed holds the `fallback` role. Roles are given to person ids only;
  nothing a platform reports grants anything. A plugin may give roles through the same role
  operation the owner uses, under its own permission.
- **Which value wins**: all of the person's roles together (a `deny` beats an `allow`), then the
  tiers in `config.d/`, then the manifest's. A place may override with the same `allow` and `deny`.
- **Approval modes**, per role: `yolo` passes everything except calls marked `sensitive`;
  `default` asks before `runs_code` or `changes` outside the person's own workspace; `strict` asks
  before every step. The owner answers, and so may anyone with `approval.answer`. An approval
  is consumed by the act it approved; the approved act runs as the requester, never as the owner.
- **The middleware's own permissions**: `config.reload`, `plugin.manage`, `roles.manage`,
  `identity.whois`, `approval.answer` at tier `admin`; `identity.whoami` at `guest`.
- **No one grows their own power**: `/role add` gives only roles below the giver's highest tier.
  Owner only, never movable: anything `identity_changing`, a new owner, editing admin-tier roles.
- No permission overrides the two absolute lines or buys dishonesty.

### 4.9 Interaction
What the soul shows besides its actions, and questions asked and answered.
- **The core records and changes nothing.** Everything needed is already an event.
- **`interaction.py` decides what and to whom** and emits the lifecycle events of *Lifecycle events* to the
  body of a place: silence shows nothing, `seen` is never sent twice after a restart, and content
  (thoughts, steps) passes `privacy.visible` for that place first.
- **The body renders**, its own way or not at all, from its own options in `config.yaml`
 . The middleware never knows how.

**Settings** in `config.yaml` → `interaction:` (*Deployment files*; `/show <name> <value>` writes the line):
`thinking` is `off`, `owner_direct` (only in a place whose audience is only the owner) or `all`;
`steps` is `off`, `long` (work that takes long) or `all`.

**Questions** (approvals, a brake's "continue?"). `interaction.py` calls the place's body `ask`
with the question and its choices from the message catalog, and records `question.asked {ref,
request, choices}`. An answer counts when:
- a percept carries `answer: {prompt: ref, value}` for that ref, from an actor allowed to answer;
  or
- reasoning, having understood an answer in words, calls the `answer(request, value)` capability,
  and the trigger is from an allowed actor and is either `about` the question or in that actor's
  direct place.

Unanswered questions expire (`question.expired`) after the owner setting `question_ttl`. A
question and its answer are events, so an answer after a restart still counts, and an answered
approval whose act has not run yet runs after the restart. A body without `ask` gets the question
as an ordinary message through its communicative capability.

### 4.10 Attachments
Files that arrive with an event or leave with an action, any number. The core never reads them.
- **Vocabulary** (contract): an attachment is `{ref, name, mime, size}`. A percept lists them in
  `attachments`; its `description` sums them up for reasoning.
- **Store** (`infra/`): an adapter calls `put(file) → ref`; anyone who opens one calls
  `open(ref)`, which `privacy.py` checks. An attachment carries the audience labels of the events
  it came with; a file from a private place goes to someone else only after `share`. One file sent
  in several places is one stored file, deleted when no non-forgotten event carries it.
- **Deferred refs**: an adapter may put a file without its bytes (`put_deferred(name, mime,
  size, locator) → ref`); the first `open` that passes the privacy check calls the body's
  `fetch(locator)`. A fetch that fails is an honest error.
- **Sending**: a body's capability takes attachment refs in its own arguments; the policy checks
  each ref's audience against the target place (the same one check), and "confirm before
  forwarding a file" applies per attachment. How many files fit in one native message is the
  body's business: it splits by itself and answers with one receipt.
- **Reading**: Workspace's `read` takes a `ref`. Images are opened
  only when reasoning chooses to; vision is a separate, later decision.

## 5. Adapters

### 5.1 What an adapter is
An adapter connects the soul to one platform, device or app: a chat app, the CLI, a robot, a
speaker, a workspace app, a sensor, a game. It is where translation happens — the only place. Like a sensory cortex it turns native signals into the contract's facts (actors, places,
audiences, events); like a motor cortex it turns the soul's actions into native actions and
answers with an honest receipt. The core and the middleware never know which adapter something
came from; `body` is only a name they carry.

**Native first**. Every adapter uses everything its platform offers: it reads every kind of
event the platform delivers and acts the best way the platform allows. Rich features are the
adapter's own capabilities and arguments, explained to reasoning in its `prompt_section`; what a
platform lacks is simply not offered. No adapter is a special case: the same contract, the same
conformance suite and the same rules apply to all of them.

### 5.2 Rules for every adapter
The surface, percepts, audiences, capabilities, receipts and commands are defined in *Waist 2: middleware ↔ adapters and plugins*; this
is what an adapter adds to them:
- **One stable form per actor.** When a platform gives one account two forms (WhatsApp's LID and
  phone-number JID), the adapter reports one, so one person never looks like two.
- **Features**: it computes `addressed`, `named` (the middleware tells every adapter the soul's
  current name) and `addressed_other`. The adapter is the only place that reads native text for
  this.
- **Capabilities**: speaking is marked `communicative`; a talking capability usually has its
  permission at base `guest`; native features are explained in `prompt_section`.
- **Acting**: it splits, retries, rate-limits and falls back by itself; `held` is the policy's,
  never a body's; a place where the soul may not post answers `refused` with the reason.
- **Rendering and asking** its own way, from its own options; it never decides what is
  shown or to whom.
- **History**: back-fill and import.
- **Files**: received files go to the attachment store; capabilities that send files take
  refs.
- Own reconnects and platform quirks; errors become events, never crashes of the loop. Tokens and
  URLs come from `.env`; non-secret options from its section under `bodies:` in `config.yaml`.

**Loading an adapter**. An adapter is a module with one factory, `create(options, env)`:
`options` is its own section of `config.yaml`, `env` gives the variables it declares (it reads
only those). `config.yaml` lists the enabled adapters under `bodies:`; an adapter outside this
repository also names its module. The entry point never imports an adapter; it asks the loader.
Adding an adapter means installing its module and adding a few lines under `bodies:` in
`config.yaml` (example in *Deployment files*).

### 5.3 Places and audiences
A place is wherever an event happens and an action lands: a private chat, a group, a channel, a
thread, a room, a document, a device, a game region. The contract knows only its id and its
audience; everything else about it (its name, that it is a thread of a channel of a server,
that only admins may post) is part of a percept's `description` or the adapter's
`prompt_section`, for reasoning to understand.

How adapters fold their platform into the audience:

| Platform | Audience |
|---|---|
| a private chat | the other person |
| a WhatsApp or Telegram group | the members now (newcomers do not see the past) |
| a Discord channel | its members; `unknown` when anyone who joins later can read the history |
| a channel that only broadcasts (WhatsApp Channels, a Telegram channel) | `unknown` |
| a room a robot or speaker is in | the people detected; `unknown` when it cannot be sure |
| a shared document | its collaborators |
| a sensor | nobody |

### 5.4 Native: what goes in and what comes out
**In: everything the platform delivers becomes a percept.** Text, replies and quotes (`about`),
mentions (`involves`), edits and deletions (`about` the earlier event), reactions by others,
stickers, locations, contact cards, polls and votes, view-once media, any number of files, calls
("Budi called, not answered"), and place changes (renamed, joined, left: also a new
`place.audience`). Each becomes a short `description` plus the contract's fields:
- a reaction by someone else on the soul's own message is `addressed`;
- a deletion stays in the log as a person remembers what was said; forgetting stays an explicit
  request;
- a sticker or image is an attachment with a description; it is opened only when reasoning
  chooses to.

**Out: the body's own capabilities.** Each adapter offers what it can do with its own names and
schemas — for a chat app typically `say(text, attachments?, reply_to?, mentions?, sticker?,
buttons?, thread?, location?)`, `react(emoji, message)`, `edit(message, text)`; for a robot
`speak(text)`, `move(to)`, `gesture(name)`; for a document `edit(range, text)`, `comment(range,
text)`. Its `prompt_section` tells reasoning what the arguments mean on this platform. A
platform-only action (pin a message, manage a channel) is **off by default** and offered only
when the owner lists it under `bodies.<adapter>.tools:`.

**Lifecycle rendering** is the adapter's own choice, set in its
`render:` options:

| Event | Chat | Robot | Workspace app | Game |
|---|---|---|---|---|
| `composing` | typing… | turns to the person, a light | a "writing" cursor | faces the player |
| `seen` | read mark (blue ticks) | a nod | — | — |
| `progress` | one message edited per step, or a new message | a spoken "still working" | a status line | — |
| `thought` | a quoted line | — | a side note | — |

An adapter that shares an account with a person marks events seen only when the soul speaks
there, so the person keeps their unread marks.

**Questions** (`ask`, *Questions and answers*): buttons or a menu where the platform has
them, else a poll, else a reaction per choice, else a numbered message; a robot says the question
and listens; an app opens a dialog. The adapter records how each choice will come back and returns
`answer: {prompt, value}` when it does. A reply in words is a normal percept; reasoning
understands it.

### 5.5 History: back-fill and import
- **Back-fill** is what the adapter missed while offline: `backfill: true`, from its optional
  `history(place, before, limit)`.
- **Import** is the history an account already had before the soul was connected (WhatsApp's
  history sync on pairing): all of it, as percepts with `imported: true` and their
  `occurred_at`. What the core does with both is *Adapters, tools and capabilities* (History and catching up).
- **Media are fetched when opened**: an imported or back-filled file is a deferred ref;
  when a platform link has expired, the adapter asks the platform again.

### 5.6 An account shared with a person
Some accounts can be used by a person at the same time (WhatsApp as a linked device on the
owner's own number). The adapter tells the two apart itself: every action's receipt carries the
native id of what it sent, so it is in the Log; an event from the account whose id is in the Log
is the soul's own and not a percept; one whose id is not in the Log was made by the person and is
a percept from that actor. `self_actor()` reports the account; when identity maps it to the owner,
the middleware adds one context line.

### 5.7 Present and planned adapters
| Adapter | Status | Notes |
|---|---|---|
| CLI | present | Unix socket inside the main process, thin client; text, files by path, reply; `composing` and `progress` as status lines; questions as numbered choices |
| Telegram | present | long polling with `allowed_updates` for every kind it reads; inline buttons and the command menu, edit, delete, reply, reactions, polls, stickers, media groups (one percept per group), locations, forum topics; a failed download is an error percept, never a crash loop; no history and no read marks on the Bot API |
| WhatsApp | present | neonize (a linked device); own or owner's number; full history import; files, stickers, polls (votes decrypted), reactions, edits; no buttons, so questions by poll; handles logout and disconnect |
| Discord | present | discord.py (message content intent); servers, channels, threads, DMs; buttons and menus (acknowledged within Discord's 3 s inside the adapter); reactions and edits on uncached messages; `allowed_mentions` set |
| Home Assistant | **Draft**, [later task](tasks/later-voice-home.md) | the first non-chat adapter |
| Voice notes | [later task](tasks/later-voice-home.md) | STT in, TTS out; description = transcript |
| Voice calls | later | waits for a mature calling library |
| Robot (own adapter) | later | microphone and speaker straight to the adapter, with its own reflexes (wake word, voice activity, barge-in, turning to the speaker) |

Tools (search, files, email) are in the plugin catalog. Model providers are not
adapters: they are drivers behind the core's model port, in `infra/providers/`.

### 5.8 Home Assistant as a body (**Draft**)
Home Assistant (HA) already runs a whole voice pipeline — wake word, speech to text and text to
speech over its Wyoming protocol, ESP32 voice satellites — and already controls the home's
devices. HA is one adapter, and the soul is HA's conversation agent.
- **Hearing.** A small HA custom integration (shipped in this repository) forwards each utterance:
  a percept with `addressed: 1` (the wake word was said), the satellite's area as the place, the
  HA user when HA knows who spoke, else *unknown*, and the area's audience as `unknown` unless HA
  knows who is in the room. Nothing private is said aloud in a room.
- **Speaking.** The answer goes back as the agent's reply, which HA speaks on that satellite; a
  silent decision returns an empty reply. Speaking first goes through HA's announce service.
- **The home as capabilities.** Only the entities the owner exposes in HA: `home.state` (facts
  `reads`) and `home.control(entity, action, data?)` (facts `changes`, base tier `trusted`).
- **The home as senses.** State changes of entities listed under `bodies.homeassistant.senses:`
  become percepts with actor *unknown*, never addressed.
- **Latency.** HA waits for the reply; past `reply_seconds` the adapter answers with a short
  holding line and speaks the real answer later. The gate and reasoning are never skipped.
- `.env`: `HA_URL`, `HA_TOKEN`.

### 5.9 Speech: hearing and speaking (**Draft**)
**One soul, one mind.** Every speech engine is only ears and a mouth. Every utterance is a percept
that passes the gate, and every spoken word comes from an action the soul chose.

**Drivers by protocol, not by model** (`infra/providers/speech/`, ported from V1):

| Door | Covers |
|---|---|
| OpenAI-shaped HTTP (`/audio/transcriptions`, `/audio/speech`) | OpenAI, Groq, and local servers that mirror the shape (faster-whisper, Kokoro, …) |
| Wyoming (JSON over TCP) | Home Assistant's local voice services: Whisper, Piper, openWakeWord |
| Native | ElevenLabs, Deepgram |

Chosen by `.env` (`STT_PROVIDER`, `STT_BASE_URL`, `STT_MODEL`, `TTS_PROVIDER`, `TTS_BASE_URL`,
`TTS_VOICE`, `VOICE_REALTIME`, one key variable per cloud provider), shared by every adapter. Each
driver reports its languages at boot; the boot report warns when the soul's language is missing.

- **Hearing.** A voice note is stored and transcribed; the percept's description carries the
  transcript; live speech is segmented by the adapter (wake word, voice activity). A missing STT
  gives an honest description ("a voice note (0:12), not transcribed").
- **Speaking.** A body's speaking capability may take `voice: true` (a voice note on chat) or
  always speak (a device); the words stay in the Log as the action's text.
- **Listening while speaking** (a reflex of the adapter): barge-in stops speech at once, the
  receipt reports what was actually said (`spoken_until`), and the interrupting speech is a
  percept. Echo cancellation belongs to the device.
- **Realtime engines** may be a faster ear and a more expressive mouth, never the one who answers.
- **Cost**: seconds transcribed and characters spoken are recorded with each use.

## 6. Plugins

### 6.1 What a plugin is
A feature built on the middleware (a *package*). A plugin is a folder that bundles any
of: tools, middleware, commands, installed skills, standing rules, knob-offset bounds,
permissions, a state namespace and dependencies, declared in one manifest. It reaches the core only through
the middleware's six levers, and sees only the
contract's facts; like the middleware it never parses message text to
understand it. Every tool names facts and a permission; a plugin's permissions live in its own
namespace and never redefine a middleware permission.
- Tools (search, files, email) are shipped by plugins.
- Role plugins (assistant, customer service, sysadmin) may ship a birth certificate, used only at
  birth; they can never overwrite identity afterwards.
- Installed, run and uninstalled by the install module.
  Uninstalling removes the plugin's tools, middleware, rules and knob offsets; its events stay
  in the log as history.

### 6.2 Catalog
| Plugin | Kind | Status | What it brings |
|---|---|---|---|
| **Soul Tuning** | behavior | Task 4 | modes and example conversations |
| **Web Search** | tool | Task 4 | `web_search`, `web_fetch` over many providers |
| **Workspace** | tool | Task 4, on by default | `read`, `write`, `edit`, `grep`, `glob`, `run` in a sandbox |
| **PDF Reader** | extension of Workspace | Task 4 | a native reader for `.pdf` |
| Word / Spreadsheet readers | extensions of Workspace | planned | the same pattern as PDF Reader |
| simulated emotions | behavior | idea | own state via events, notes, rules, knob offsets |
| plugin suggestions | feature | **Draft** | a note to the owner from the local catalog |
| moderation, content filters | feature | idea | pass/replace/stop |

- Removed: memory-view (2026-11-12). Roles and permissions are not a plugin: they are the
  middleware's `roles.py`, because every plugin brings tools
  and commands that need limits.
- **Plugin suggestions** (Draft): only to the owner, only from the local catalog, at most
  once per missing capability per habituation window, never installs anything.

### 6.3 Manifest
One file at the root of a plugin, `plugin.yaml`, declares everything it brings and every default. The install module reads it; the core never does. The owner never edits it: it is
replaced on update and removed on uninstall.

```yaml
manifest_version: 1                  # format of this file
name: workspace
version: 0.1.0                       # the plugin's release
description: Read, write and run code in a sandboxed folder per person.
dependencies: []                     # must load first; missing or failing fails boot
optional_dependencies: []            # used when present
state_namespace: workspace           # its state keys are workspace:* (projected from its events); its
                                     # event kinds are workspace.<kind> (a `-` in the name becomes `_`)
features:                            # each can be switched off in config.d/
  tools: [read, write, edit, grep, glob, run]
  commands: [workspace]
  middleware: []
tools:                               # facts per tool
  read:  { facts: [reads] }
  write: { facts: [changes] }
  run:   { facts: [runs_code, network] }
settings:                            # schema with defaults; the owner overrides in config.d/
  base: { type: string, default: debian-slim }
  timeout_seconds: { type: int, default: 120 }
permissions:                         # roles.py; base = lowest tier that gets it by default
  workspace.read:  { label: "Read files",        base: guest }
  workspace.write: { label: "Write and edit",    base: member }
  workspace.run:   { label: "Run code",          base: trusted }
role_settings:                       # a value per tier; roles may override
  quota: { guest: 0, member: 1G, trusted: 5G }
extension_points:
  opens: [reader]                    # other plugins may fill it
  fills: []
skills: []
rules: []                            # standing rules shown in <self>, in this order
knob_offsets: {}                     # bounded, temporary; recorded as disposition.updated
```

### 6.4 Configuration
| File | Owner | Holds |
|---|---|---|
| `plugins/<name>/plugin.yaml` | the plugin | every default; never edited by the owner |
| `config.yaml` | the owner | which plugins are on (`plugins:`), `roles:`, `people:` |
| `config.d/<plugin>.yaml` | the owner | one plugin: `features`, `options`, and `permissions` / `role_settings` (written by `/plugin enable`) |

```yaml
# config.d/workspace.yaml — written by /plugin enable from workspace 0.1.0
version: 1
features: { run: true }
options:  { timeout_seconds: 300 }
permissions:                   # lowest tier per permission; edit to move a permission
  workspace.read:  guest
  workspace.write: member
  workspace.run:   trusted
role_settings:
  quota: { guest: 0, member: 1G, trusted: 5G }
```

- **Merge rules** (as Helm values): maps merge, lists replace whole, `[]` switches everything
  in that list off. The boot report warns when a list replaces a default.
- An unknown key or a wrong value fails at boot with a clear message. A `config.d/` file for a
  plugin that is not installed is reported and ignored; it stays, so the settings return if the
  plugin does.
- A feature switched off is removed from the registry, not hidden.
- The files are the source of truth, written only by the owner's commands.
  `/plugin show <name>` prints a plugin's defaults; `/roles show <role>` prints the merged
  permissions per role. Format versions: *Deployment files*.
- Which tier gets a permission is in `config.d/`; which roles exist and who holds them is in
  `config.yaml` (`roles:`, `people:`).

### 6.5 First plugins

#### 6.5.1 Soul Tuning
Tunes expression, never identity; only the owner changes it (absolute line 1).
- **Modes** (daily, professional, …): standing rules in `<self>`, knob offsets within bounds,
  the `/mode` command.
- **Example conversations**: short exchanges in the stable prompt, in a fixed order.
- Who a person is to the soul ("a customer; help with orders") is not here: it is each role's
  and person's `context` in `roles.py`.

#### 6.5.2 Web Search
- Tools `web_search` and `web_fetch`; permissions `web-search.search` and `web-search.fetch`,
  base `guest`.
- Providers in an ordered fallback chain set in `options`: SearXNG, Tavily, Brave, Exa,
  Serper, Google, and DDGS always last. Each normalizes its results to one shape and fails
  with a structured reason, never a crash. Port V1's `engine/tools/web_search.py` (ADR
  `engine/0026-search`).
- `web_fetch` runs in the MonoAgens process, so it refuses internal addresses (SSRF): resolve
  the host, check every address against the same blocked set as the sandbox, connect to
  the checked address only. Port V1's `fetch.py`.

#### 6.5.3 Workspace
On by default through the shipped `config.yaml`; off with one line.
- Tools: `read` (line ranges; also attachments received by adapters, through their
  reference), `write`, `edit` (exact text replacement), `grep`, `glob`, `run`. Output is capped
  with a "continue from here" marker.
- **Progress note for long work** (a standing rule, lever 3): work of more than a few steps
  keeps `progress.md` in the workspace it works in — the goal, what is done, the next step,
  the files involved — updated after each step and read first whenever the work resumes. The
  owner can open and edit it like any file.
- Opens the `reader` extension point: one reader per file type; `read` without a reader for a
  type says so and names the extension it needs, never returns raw bytes.
- Folders: each person's own workspace; **shared folders** named by the setting
  `workspace.shared` on the person's roles (a person in two group roles sees two).

**Sandbox for `run`**. Python drives two native tools, bubblewrap (files) and pasta from
`passt` (network); no Node, no container runtime.
- **Base**: one shared, read-only Debian slim tree (Python, pip/uv, bash, coreutils, git, curl,
  CA certificates), prepared once when the plugin is first enabled and used by everyone.
- **Layer per person**: an overlay on the base where `pip install` and `apt install` land;
  quota from the role setting `workspace.quota` as a soft limit (checked after each run; the
  next run is refused while over); can be reset.
- **Visible inside**: the base and the person's layer, their workspace, shared folders they
  may use, an empty `/tmp` per run, a private `/proc` and `/dev`, copies of `resolv.conf` and
  the CA certificates. The host's system, home, `.env` and database are not visible at all.
- **Network**: outbound to the internet allowed. Blocked: the host's loopback (`--no-map-gw`
  plus `-T none -U none`), private ranges by blackhole routes inside the sandbox's network
  namespace (10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16, fc00::/7, fe80::/10) and the
  server's own public addresses. No inbound (`-t none -u none`).
- **Limits**: a computed environment without secrets (V1 ADR `engine/0024-tool-input-contract`),
  time, CPU, memory and process limits, capped output; the sandbox dies with MonoAgens.
- **Guard**: marks a call `sensitive` (sudo, mount, network scans, download-and-execute, …),
  which asks for approval even in `yolo`. It is a marker, not the wall: patterns can always be
  evaded; the wall is the sandbox and the limits. Port V1's two-stage guard.
- **Fail-closed**: no pasta → no network at all (`--unshare-net`); no bubblewrap or no base →
  `run` unavailable. The boot report says which.
- Shares the host kernel: right for a personal or community soul; a microVM runner can later be
  an extension.

#### 6.5.4 PDF Reader
Fills Workspace's `reader` point for `.pdf`; native (pypdf), no cloud; `dependencies:
[workspace]`. Its library is installed only with it.

## 7. Operator

### 7.1 What the operator layer is
The tools the owner uses to install, run and look after one soul: the `monoagens` command line,
the installer, and **MonoAgens Console**, a web app. They live outside the soul.

- **Not a fourth layer of the soul.** The operator tools are not core, middleware, adapter or
  plugin. The core does not know they exist; every change they make passes through the
  middleware like any owner command.
- **The command line first; Console is a face on it.** Every Console action calls the same code
  as its command. One implementation, two ways to use it.
- **Clean by default.** A fresh install holds the soul, the CLI adapter and the default plugins
  only. Anything else is installed when the owner asks; until then its code is not on the
  machine.
- **Console is a separate process.** It keeps working when the soul has crashed or its
  configuration is broken, so the owner can see why and fix it.
- **One owner.** Console serves the owner of this deployment only; there are no other accounts.

### 7.2 The command line
One global command, `monoagens`, installed in `~/.local/bin`. With the compose stack it
forwards the commands that need the runtime into the container, so every command works the same
on both stacks.

| Command | Does |
|---|---|
| `install`, `init` | install; the setup wizard |
| `update` | backup → pinned, verified release → `uv sync` → migration check → restart; back to the previous version when boot fails; never runs `main` |
| `plugin install\|enable\|disable\|remove [name]` | a plugin's life cycle |
| `adapter install\|enable\|disable\|remove [name]` | an adapter's life cycle |
| `start`, `stop`, `restart`, `status` | the service, through the commands the owner sets |
| `logs [-f]` | the process log |
| `chat` | talk to the soul through the CLI adapter |
| `backup`, `restore <archive>` | one archive: database dump, `config.yaml`, `config.d/`, `soul.yaml` |
| `doctor [--fix]` | health check: database and extensions, migrations, file permissions, provider reachable, adapters, sandbox |
| `config schema` | the schema of `config.yaml` and `config.d/` for Console's form mode |
| `console` | run Console |
| `uninstall [--purge]` | remove the program; data stays unless `--purge` is confirmed |

- **One function, two doors.** `/plugin enable` typed in chat and `monoagens plugin enable` in a
  terminal call the same loader function, which writes only its own line after a backup
 . When the soul runs, the command line asks it to
  `/reload`; when it does not, the change applies at the next start.
- **Look and feel.** Arrow-key choices, a spinner while the soul thinks, status lines for steps,
  colors per speaker, input history, completion of `/commands`, progress bars, a logo. Built on
  Rich and prompt_toolkit (the `cli` extra, installed by default). When the output is not a
  terminal, plain text; `NO_COLOR` is honored.

#### 7.2.1 Service control
How the soul runs as a service belongs to the deployment (OpenRC, systemd, compose). The owner
sets the commands once in `config.yaml` → `operator:`; `start`, `stop`, `restart`, `status` and
Console's buttons call them, and a command left empty hides its button.

```yaml
# config.yaml
operator:
  service:
    start:   "rc-service monoagens start"
    stop:    "rc-service monoagens stop"
    restart: "rc-service monoagens restart"
    status:  "rc-service monoagens status"
    logs:    "tail -n 500 /var/log/monoagens.log"
  console:
    listen: 127.0.0.1:8470
```

#### 7.2.2 The process log
The soul's own output (boot report, pairing code, adapter status, errors). It never holds
conversation content: only ids, numbers and states. A line that names an event carries its id,
so Console links it to the Timeline, where the privacy chip shows who may see it.
`monoagens logs` and Console's Logs page show the same lines.

### 7.3 Installer
Modelled on V1 (branch `archived`, ADR-0007, ADR-0018, ADR-0039), with its lesson: V1's
`install.sh` grew to over 2,000 lines because shell and Python held the same logic twice.

- **`install.sh` is a thin bootstrap.** It installs a pinned `uv` checked against its sha256,
  downloads a pinned release checked against its published checksum, and hands over to
  `monoagens init`. Everything else lives in Python.
- **Two stacks.** `--stack native`: the soul runs on the host, Postgres in a container.
  `--stack compose`: both in containers. The container engine is **Podman or Docker**; an
  external Postgres is given with `--database-url`.
- **The wizard** (`monoagens init`): location first (the folder and what is already there) →
  provider, model and key → database → adapters → plugins → review → write. The left key goes
  back with earlier answers kept; Escape cancels without writing anything. Keys are masked and
  written only to `.env` at mode `0600`, never shown in the review or the log. Every list of
  choices comes from the registries and manifests, never from a list in the installer.
- **What a fresh install holds**: the soul, the CLI adapter (built in: it is the owner's door,
  the pairing code and Console's Chat use it) and the default plugins
 . No other adapter.
- **The read-only database role** for Console is created here, once, with the admin
  credentials the owner gives for the duration of the install. Neither the soul nor Console keeps
  those credentials.
- **Tested, not hoped**: the container image carries the migrations, and the compose stack
  installs the extras of the enabled adapters (both broke in V1).

### 7.4 Plugins and adapters: life cycle and catalog
- **One life cycle for both**: available → installed → enabled ⇄ disabled → removed. Every step
  is an owner action and is recorded as an event.
- **Install** downloads the archive, checks its digest, unpacks it into `plugins/<name>/` or
  `adapters/<name>/` and installs its dependencies there. A digest that does not match writes no
  file and changes no configuration. No automatic updates, no background fetch.
- **Enable** writes the configuration lines; a new plugin's
  or adapter's code needs a restart, and the command says so.
- **Remove** deletes the code and its dependencies; the events stay in the log as history and
  the owner's files are not touched.
- **One repository.** Every plugin and adapter is developed in this repository. The split happens
  on the owner's machine: during development the installer copies only the default parts from
  the checkout; from the first release, each plugin and adapter is its own archive with a
  sha256, attached to the same GitHub release, and listed in a catalog index file (name,
  version, digest, description, dependencies).
- **Adapters move to their own folders** with a manifest, as plugins have, once Task 5 is done
  (its work happens in `monoagens/adapters/`). The CLI adapter stays inside the package.
- Plugins and adapters are trusted code that runs in the soul's process
 : a digest proves the archive is the published one,
  not that the code is safe.

### 7.5 Console: what it shows
A sidebar of pages, grouped:

| Group | Page | Holds |
|---|---|---|
| Main | **Chat** | the CLI adapter in the browser: talk to the soul, type `/commands` |
| | **Timeline** | each percept and what followed: the gate's decision with its numbers (`s`, `theta`, `p`, `u`, reason), thoughts, acts, model calls and tokens; filter by place, person, decision |
| | **Memory** | what the soul remembers, with forget |
| | **Promises** | active expectations: promises, waits, reminders (read only) |
| Bodies and capabilities | **Adapters** | each adapter: on or off, pairing (the WhatsApp QR), its surfaces from the boot report, its `render:` options |
| | **Plugins** | installed and available plugins, on or off, version, "restart needed" |
| | **Skills**, **Tools** | what the registry holds, with facts and permissions |
| | **MCP** | locked until the MCP Connect plugin exists |
| People and rules | **People** | persons, linked accounts, `/link`, the owner pairing code |
| | **Roles** | roles, who holds which, approvals waiting |
| Settings | **Model** | provider, model, fallbacks; tokens and the daily budget |
| | **Keys** | `.env`: write only, never shown |
| | **Config** | `config.yaml` and `config.d/` |
| | **Soul** | the living identity (name, language, persona, principles), editable; `soul.yaml` shown as the birth certificate |
| Footer | — | the soul running or not, adapters up, tokens today; Restart, Update; Logs |

- **Locked pages.** A page that belongs to a plugin or an adapter has its on/off switch at the
  top; when it is off, the rest of the page is grey with the way to turn it on. When the soul is
  not running, everything that needs a command is locked; reading, Config, Logs and Start stay
  open. Pages of the middleware (People, Roles, Config) are never locked by a plugin.
- **Not included**: sessions (the soul has none; it lives on), profiles (one deployment is one
  soul), webhooks, a hosted portal (BYOK, *Rulings*).

### 7.6 Console: how it works

#### 7.6.1 Paths
| Need | Path |
|---|---|
| Timeline, status, tokens, memory | the database, through a **read-only role** (`SELECT` only) |
| Every change the soul applies | a `/command` line to the CLI adapter's socket; the reply is read from the Log (the `act.out` whose `caused_by` is the command) |
| Editing files | *Config editor*: validate in a short-lived process, back up, write, `/reload` |
| Start, stop, restart | the service commands |
| Logs | the process log |

- Console never writes the database. It sends only lines that start with `/`: any other line is
  a message to the soul and would be appraised as one (Chat sends those on purpose).
- Console runs as the same system user as the soul, on the same machine: the socket is `0600`
  and the files are the soul's.
- Console does not import the embedder or plugin code; the validation that needs them runs in a
  short-lived process.

#### 7.6.2 Login and network
- **Network**: Console listens on `127.0.0.1` by default. The owner opens it to a private network
  (Tailscale) by setting the address; TLS comes from that network or a reverse proxy. Console is
  never meant for the open internet: it shows every conversation of every person.
- **First start**: Console prints a one-time **setup code** in the process log. The first page is
  registration, which needs that code; whoever registers is the Console owner, and the code is
  then spent. Only someone who can read the server's log can register.
- **Password**: hashed with scrypt, stored in Console's own state file outside the repository
  (not in `.env`, not in `config.yaml`); a session
  cookie (`HttpOnly`, `Secure` when served over TLS, `SameSite=Strict`) with a CSRF token on
  every change; repeated wrong passwords slow down.

#### 7.6.3 Privacy
The owner may see everything (the owner's view is a view, not ownership). So that he knows
what belongs to others, every line in Timeline and Memory carries a **privacy chip**: its
audience labels, a private place marked as such. Hard
forgotten content shows as forgotten, never reconstructed.

### 7.7 Config editor
- **Two modes**: a **form** generated from `monoagens config schema` (the loader's own knowledge
  of `config.yaml` and the plugin manifests' `settings`): a switch for a boolean, a menu for a
  choice, a bounded number, text, chips for a list; anything a form cannot hold says "edit in
  raw". And **raw**: the file in an editor with YAML highlighting.
- **Comments survive.** The form writes only the lines that changed, with the loader's narrow
  writers; raw saves the text exactly as typed.
- **Saving** is the same in both modes: show the diff → validate (short-lived process) → back up
  → write → `/reload` → show the result (ok, "restart needed", or refused with a button to
  restore the backup). An invalid file is never written; the error names the line. When the soul
  is not running, the file is still saved once valid, marked "applies at start".
- **Conflicts**: Console remembers the file's hash when it opens it; if the file changed on disk
  since (a command in chat, an edit over SSH), saving is refused and the change is shown.
- Switching modes asks to save or discard first.
- Editable: `config.yaml`, `config.d/`. Read only: `prior/`. `.env` only on Keys.
  People and Roles use the same editor on their sections.

#### 7.7.1 The soul's identity
- **After birth** `soul.yaml` is never read again; identity lives in `state: identity`
 . Editing the file would change nothing, so Console never writes it
  after birth and shows it as the birth certificate.
- **The Soul page edits the living identity** with the same two modes: a form (name, language,
  persona, and principles as a list to add, remove and reorder) and raw (the current identity
  as YAML). Saving shows the diff, then sends the owner command `/identity set …`.
- `/identity` calls the core's own `set_identity` capability through the action hook, the same
  door the soul uses when the owner asks in chat: the owner check of absolute line 1 holds, the
  change is recorded as `identity.updated`, and the soul sees the new identity from its next
  turn.
- **Before birth** (a fresh install), `soul.yaml` is edited directly, in the wizard or in
  Console.

### 7.8 Technology and weight
- **Server**: Python, Starlette, uvicorn, Jinja2 (autoescaping on), as the `console` extra.
- **Browser**: HTML rendered on the server, one CSS file, small plain JavaScript, htmx for partial
  updates, Server-Sent Events for live lines. Libraries only on the page that needs them: uPlot
  (charts), CodeMirror (Config), SortableJS (ordering). Every file is served by Console itself:
  no npm, no build step, no CDN.
- **Not used**: Node, React, FastAPI, Django, Flask.
- **Light**: one process (about 40–70 MB), no embedder, no plugin code, one or two database
  connections, every list paged by id, never a whole table.
- **Look**: dark, system fonts, works on a phone (the sidebar folds). Modern through CSS: custom
  properties (the design tokens), grid, container queries, View Transitions, `<dialog>`,
  `popover`, SVG icons, `prefers-reduced-motion` honored.
- **Design hand-off**: the owner designs in FigCraft; what is taken from it is the tokens, the
  components with their states and the screens, never generated framework code.

### 7.9 Changes this needs elsewhere
Written into the operator task, applied when that task runs:
- *Install*: one enable function for chat and the command line; `config schema`; the
  loader accepts the `operator:` section of `config.yaml` and leaves it to the operator tools.
- *Deployment files*: the `operator:` section in the `config.yaml` example.
- *Middleware*: an owner command `/forget` (the core has `forget_hard`; no command reaches it).
- *Middleware*: an owner command `/identity show|set` that calls the core's `set_identity`
  through the action hook.
- *Rules for every adapter*: adapters in their own folders with a manifest; the CLI adapter built in.
- *Terms*: operator, Console, catalog; the layout row for `monoagens/console/`.
- `AGENTS.md`: the `console` and `cli` extras approved by the owner (2026-11-13).

## 8. Project

### 8.1 Rulings
- Keys only from the environment; the embedder is a replaceable port. License, BYOK and one
  soul per deployment: *Open source*.
- **Who this repository is for**: people who can run their own server. Self-hosted on a VPS or
  a home machine, configured by hand (`.env`, `config.yaml`, `soul.yaml`), BYOK. It ships a
  basic hand-tuned default prior and no role templates; it never ships priors learned from
  other people's souls. A soul still needs no tuning to start: it is born from the default prior
  and adapts.
- Order of users: the owner alone, then one real group the owner knows well (also the honest
  human-likeness evaluation), then developers once contracts are stable.
- **Release gate**: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models
 .
- **Draft**: README with the brain table, manual install on a server (Docker for Postgres,
  `.env`, `config.yaml`, `soul.yaml`), talk, tune with replay; compressed daily backups with an
  owner-set retention.

### 8.2 Deployment files
Five files and folders, each with one job; none holds what belongs to another.

| File | Holds | Read | Changed by |
|---|---|---|---|
| `.env` | secrets and endpoints: the model provider and its fallbacks (`LLM_PROVIDER`, `LLM_FALLBACKS`, keys, model), bot tokens, database URL | every start | owner or technician |
| `config.yaml` | this deployment's non-secret settings: timezone, owner settings, runtime, adapters, plugins on, roles | every start | owner; owner settings in chat; `/plugin` and `/role` write their own lines |
| `config.d/` | one file per plugin: its features and options | every start | owner |
| `soul.yaml` | the birth certificate: name, language, persona, principles | once, at birth | nobody afterwards; identity changes in chat |
| `prior/` | instincts: knobs, calibration, constants | once, at birth | developers, with replay |

**`config.yaml`** (format version 2, **Draft**; version 1 had `plugins:` as a list and is upcast
in memory):

```yaml
version: 2
timezone: Asia/Jakarta              # where the soul lives; used for <now>
owner_settings:                     # changed by hand or with /budget and /quiet, which write these lines
  daily_token_budget: 300000
  quiet_hours: { from: "22:00", to: "06:00" }
  question_ttl_minutes: 30          # an unanswered approval or brake question expires
runtime:
  tick_seconds: 30
  sleep: { idle_minutes: 30, retry_minutes: 30, hour: 2 }
bodies:                             # adapters (the key keeps its name); non-secret options only; tokens in .env
  telegram: { enabled: true }       # built in: monoagens.adapters.telegram
  cli: { enabled: true }
  whatsapp:
    enabled: true
    render: { composing: on, seen: on, progress: edit }   # this body's own rendering
    tools: [pin]                    # optional platform-only capabilities; none by default
  minecraft: { enabled: true, module: mono_minecraft, server: localhost:25565 }   # an outside adapter
interaction:                        # what the soul shows, for every body; /show writes these lines
  thinking: owner_direct            # off | owner_direct | all
  steps: long                       # off | long | all
plugins:                            # which plugins are on; their settings live in config.d/<name>.yaml
  workspace:   { enabled: true }
  web-search:  { enabled: true }
  soul-tuning: { enabled: true }
roles:                              # *Roles and permissions*; empty = owner free, everyone else only talks
  fallback: unverified
  unverified: { base: guest,  label: "Unverified", context: "Not known yet; be kind, share nothing about the owner." }
  customer:   { base: guest,  label: "Customer",   context: "A shop customer; help with products and orders." }
  member:     { base: member }
  runner:     { allow: [workspace.run] }             # a small role for an exception
  toko-a:     { label: "Toko A", settings: { workspace.shared: toko-a } }   # a group role: no base
people:                             # optional; keyed by person id; the only record of who holds which role
  p_7f3a:     { label: "Budi", roles: [member, toko-a, runner], context: "The owner's brother." }
```

**`config.d/<plugin>.yaml`**: one per plugin, only what the owner changed.
If `config.yaml` grows too long, `roles:` may move to its own file later; the format stays.

**Format versions**: every YAML file the owner or a developer edits carries its format
version (`config.yaml`, `config.d/*.yaml`, `soul.yaml`; the prior folder through
`format_version` in `prior/manifest.yaml`, since some prior files are plain lists; plugin
manifests through `manifest_version`). Older versions are upcast in memory and named in the
boot report; the file is never rewritten. A newer version than supported fails at boot. A file
without `version:` is read as version 1 with a warning.

The rule for every setting: the deployment files (`config.yaml`, `config.d/`) are the source of
truth for the deployment and are changed by hand or by the few commands that write their own
lines; what the soul learns (knobs, memories, identity) lives in its books, and there the
books win over the starting values in `soul.yaml` and `prior/`.

### 8.3 Stack
- Python 3.13 or newer, managed with `uv` (`uv.lock` is committed).
- PostgreSQL 17 with pgvector 0.8.1 or newer (iterative index scans) and `pg_trgm`, from the
  official `pgvector/pgvector` image.
- Runtime dependencies, the only ones allowed: `psycopg[binary,pool]` 3.3, the `pgvector`
  client, `fastembed`, `httpx` 0.28, `pyyaml`. Development: `pytest`, `pytest-asyncio` 1.x,
  `hypothesis`.
- No ORM, agent framework or platform SDK in the core. A plugin's or an adapter's own dependencies
  live with it — declared in its manifest or as its optional extra in `pyproject.toml`, installed
  only when it is enabled — and each one needs the owner's approval: today `ddgs` (web search),
  `pypdf` (PDF reader), the system tools `bubblewrap` and `passt` (workspace), `neonize` with
  `segno` and its `protobuf` override (WhatsApp), `discord.py` (Discord).

### 8.4 Documents
| File | For | Holds |
|---|---|---|
| `README.md` | people | what it is, install, run, tune with replay |
| `AGENTS.md` | coding agents | how to work, secrets, stack, the invariants, tests, size guide, task rules |
| `docs/spec.md` | agents and the owner | every rule, format and design decision |
| `docs/research.md` | people | every formula, why it was chosen, and its sources |
| `docs/tasks/` | workers | the active task files; finished ones in `docs/tasks/done/` |
| `prior/` | developers | default prior values: data, tuned with replay |
| `config.yaml`, `soul.yaml`, `.env.example` | owners | deployment files |

**Rules for documents.** One fact, one home: nothing is written twice; a section that needs a
fact names the section that holds it, in words, only when the reader would otherwise miss it. No
section numbers in running text and no decision numbers: why something was decided lives in git
history. No new document is created unless another is merged or removed. Formats live in the
section of the layer that owns them.

### 8.5 Evaluation (later)
Per-behavior parity against a human baseline in the same group: reply timing, messages per hour
vs 1/n, when-to-speak precision/recall against human annotation, silence where it should speak,
interruptions where it should be quiet, promise keeping, recall after 1/7/30 days, blind "which
one is the bot" (target ≈ 50%). Pre-registered metrics. Public claims phrased as "≥ X% of human
baseline on these N behaviors".

**Every scenario carries its own material**. A scenario holds, as synthetic data the worker writes, everything the behavior needs: the facts, the people and their accounts, the artifacts (a file, a piece of work, code and its test output), realistic results from every test tool it uses, and the capabilities the behavior needs (mentioning a file is not enough: reading it and sending it must be capabilities a test tool offers). A failure is called the soul's failure only after a material check shows that what the soul needed was in the Log and reachable in its context; otherwise the scenario is fixed, never the soul. The material check is written under the task's Notes.

**Scenarios are the capability record**. Each scenario states one capability in one
line and the brain function it belongs to. The replay script writes `scenarios/README.md` from
the latest runs: per capability, whether it holds with the fake model and with the real model
(passes over repeated runs, model name, date). Nothing is claimed in the README or anywhere else
that this record does not show. A capability that fails with the real model is listed as such,
not hidden.

### 8.6 Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
- **MCP Connect** as a plugin: each MCP server the owner installs (`config.d/mcp.yaml`, `/plugin`)
  becomes a source of tools in the registry, which already follows MCP's pattern of tools,
  prompts and resources. Every MCP tool declares facts and a
  permission like any tool and passes the same policy and approval modes; an outside server never
  bypasses them.
- **Voice calls** on chat adapters, once a mature library for the platform is widely released
  (for WhatsApp, a linked-device calling stack such as meowcaller on top of whatsmeow, which would
  run as a Go sidecar and needs the owner's approval as a dependency): call audio → STT → the soul
  → TTS → call audio, as a `long_running` act.

## 9. Terms

### 9.1 Terms

| Earlier name | Standard name | Meaning |
|---|---|---|
| body, bodies | **adapter** | Connects the soul to one app, platform or device (Telegram, WhatsApp, CLI, a game, a robot). |
| organ | **tool** | Something the soul can use; what it can do is a **capability**. |
| desk | **context** | What the model sees in one turn. |
| `<doing>` (working memory) | **working memory**, part of the context | The chains in progress. |
| PFC | **reasoning** | One model turn with the five thought acts. |
| cognition, appraise | **gate** | Decides whether an event wakes reasoning (the one gate formula). |
| task thread | **thread** | The main thread is the soul; extra threads do one piece of work and report back. |
| guidance | **principles** | The short list of general principles in the stable prompt. |
| soul | **soul** (kept) | The agent with its lasting identity (`soul.yaml`). |
| prior | **prior** (kept) | The defaults a soul is born with; the standard statistics term. |
| — | **provider** (driver) | Where the soul thinks: a model API reached through the core's model port, implemented in `infra/providers/`. Not an adapter: it neither senses nor acts. |
| — | **tier** | One of `guest < member < trusted < admin`; a role's `base`. |
| — | **tier role / group role** | A role with a `base` (how much) / without one (where a person belongs). |
| — | **base** (Workspace) | The one shared, read-only system tree of the sandbox. |
| — | **shared folder** (Workspace) | A work folder several people may use, named by a role setting. |
| account | **actor** / **person id** | Who caused an event as a body reports it (`whatsapp:628…`, `face:3`, or *unknown*) / the one opaque id of a person across actors (`person:p_7f3a`), registered automatically; linking merges two person ids into the older. |
| space | **place** | Where an event happens and an action lands: a chat, a room, a document, a device, a game region. |
| members | **audience** | Who can perceive what happens in a place, reported by the adapter with `place.audience`; may include *unknown*. The one fact privacy needs. |
| place info, `place_kind` | — | Removed in contract v2: the audience replaces them; names and structure are part of `description` or `prompt_section`. |
| — | **feature** | A number on an event that the gate reads: `addressed`, `named`, `addressed_other`, `audience_size`. |
| — | **receipt** | The one `act.result` that ends every action: `done`, `refused`, `held`, `failed` or `unknown`. |
| — | **lifecycle event** | What the middleware tells a body the soul is doing (`composing`, `seen`, `progress`, `thought`); the body renders it its own way. |
| — | **prompt section** | What a body tells reasoning about itself, as text. |
| — | **contract** | The one locked contract between the layers, *The contract* of this file, as code in `monoagens/contract/`, held by `contract.lock`. |
| — | **back-fill / import** | Events an adapter missed while offline (`backfill`) / history from before it was connected (`imported`, never a wake). |
| — | **format version** | `version:` in an owner's YAML file, `manifest_version:` in a plugin manifest. |

### 9.2 What never changes with a rename

Names stored in data are part of the frozen contract:
event kinds (`percept`, `decision.wake`, `expectation.reminded`, …), payload fields, `soul.yaml`
keys, the `prior/` format and `config.yaml` keys. A rename touches code only.

### 9.3 Layout

| Folder | Holds | Changes |
|---|---|---|
| `monoagens/contract/` | The contract as code: event envelope, features, capability descriptor, hooks, context items, actions, receipts, the adapter surface, percepts, lifecycle events. Held by `contract.lock`. | Only by the owner. |
| `monoagens/core/` | Mechanisms only: kernel loop, gate, reasoning, context, memory, expectations, threads, the tool door (`dispatch`), the three hooks. | Rarely: evidence, bugs, vulnerabilities. |
| `monoagens/middleware/` | Every decision, generic, in seven files: `policy.py`, `privacy.py`, `identity.py`, `registry.py`, `loader.py`, `interaction.py`, `roles.py`. | Bugs inside; never for one platform or case. |
| `monoagens/adapters/` | One folder per adapter: all translation lives here. | Per app or device. |
| `monoagens/infra/` | Store, migrations, config, clock, budget counting, embedding, versioning, attachments, model providers (`infra/providers/`). | With the stack. |
| `plugins/` | Installable features on the middleware. | Per feature. |

The rule between the layers: **adapters translate, the middleware decides from facts, the core
records and carries out.**
