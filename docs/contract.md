# Contract — Specification (v2, locked)

**Status: decided by the owner, 2026-11-14 (D86–D88).** This file is the one contract between the
layers. It replaces every earlier description of percepts, capabilities, places, what is shown
and how the soul speaks. The other specifications describe what each layer does *with* this
contract; none of them may add a field, a fact or an event to it. Changes follow §9 only.

## 1. Why one locked contract (D86)
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
- **Waist 1, core ↔ middleware**: events, capabilities, hooks, context items (§3).
- **Waist 2, middleware ↔ adapters and plugins**: actors, places, audiences, percepts, actions
  and receipts, lifecycle events, answers and commands (§4).

## 2. The six facts
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

## 3. Waist 1: core ↔ middleware
The core holds mechanisms only. It reads ids as opaque strings, numbers, times and the
description; it never reads a payload or a label.

**Event envelope** (every event, `schema_version` 2; version 1 is upcast on read, §9):

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
unknown names are stored and ignored. A new name is an additive change (§9).

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

Unchanged and described in [core.md](core.md): the four books, the five thought acts, the gate,
the budget, the injected clock and the seeded generator.

## 4. Waist 2: middleware ↔ adapters and plugins

### 4.1 What an adapter must provide (checked at boot)
A module with one factory `create(options, env)` returning an object with:

| Member | Does |
|---|---|
| `name` | the adapter's name; its capabilities are `<name>.<capability>` |
| `run(put)` / `stop()` | sense and put percepts; stop cleanly |
| `capabilities()` | its own capabilities, with descriptor, facts and permission (§3) |
| `act(action) -> receipt` | carry out one action; deduplicate by `action.id` |
| `prompt_section()` | what reasoning should know about this body, as text, at most 1,200 characters ("messages can quote an earlier message", "you have two arms with a 60 cm reach") |
| `self_actor()` | the body's own actor id, or *unknown* |

A missing member fails boot, naming it. **Optional surfaces**, detected and listed in the boot
report:

| Surface | Does |
|---|---|
| `history(place, before, limit)` | older events of a place, for back-fill and import |
| `fetch(locator)` | the bytes of a deferred attachment |
| `render(lifecycle)` | shows a lifecycle event (§4.5) its own way |
| `ask(place, question, choices) -> ref` | asks a question with choices its own way (§4.6) |

A body without `render` simply shows nothing besides its actions; one without `ask` gets the
question as an ordinary action through its communicative capability.

### 4.2 Percepts
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
| `features` | numbers (§3) |
| `answer` | `{prompt: ref, value}` when the event answers a question (§4.6) |
| `command` | `{name, args}` when a person typed or pressed an operator command (§4.7) |
| `backfill`, `imported` | as in §3 |
| `data` | anything else the adapter wants to keep; opaque to the core and the middleware |

Facts are written only when they are certain (V1's rule). An adapter that is unsure who acted
reports *unknown*; it never guesses.

### 4.3 Audiences
An adapter reports who can perceive a place with the event `place.audience {place, actors,
unknown}`, when it first sees the place and whenever it changes. It folds every platform rule
into this one fact: when people who join later can read the past (a Discord channel open to
anyone), it reports `unknown: true`; when they cannot (a WhatsApp group), it reports the members
now. A body that cannot know reports `unknown: true`. The middleware derives everything else
from this fact: a place whose audience is only the owner is the owner's direct place; one person
is a direct place with that person; more is shared; *unknown* means no private item enters.
There is no `place_kind`, `form`, `parent`, `can_post` or platform role in the contract; a name
or a structure the soul should know about is part of `description` or `prompt_section`.

### 4.4 Capabilities and actions
Each body offers its **own** capabilities, with its own names and schemas. Everything rich a
platform offers (quoting, mentions, stickers, buttons, threads, locations, polls, gestures,
voice) is an argument of that body's capability, explained in its `prompt_section`. The
middleware registry lists every capability of every body in a fixed order, and routes an action
to the body that owns its place; an action for a place another body owns is refused with the
reason ("this place is reached through whatsapp"). A body splits, retries, rate-limits and
falls back by itself, and answers with one receipt. A place where the soul may not post answers
`refused` with its reason; there is no fact for it.

### 4.5 Lifecycle events
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

### 4.6 Questions and answers
The middleware asks with `ask(place, question, choices)`; the body draws it (buttons, a poll,
speech and listening, a dialog) and returns a `ref`. A structured answer comes back as a percept
with `answer: {prompt: ref, value}`. A free answer in words is understood by reasoning, never by
code: reasoning records it with the middleware's `answer` capability, and the policy accepts it
only from facts (the right actor, the right place, the right prompt), so text written by someone
else cannot answer for the owner.

### 4.7 Commands
An adapter translates its native command form (a slash command, a menu, a button, a CLI line)
into `command: {name, args}`. The middleware handles commands by name; it never parses syntax.

### 4.8 Plugins
A plugin declares everything in its manifest ([plugins.md §3](plugins.md)) and uses the six
levers ([middleware.md §2](middleware.md)). Its tools are capabilities as in §3. It sees the
same facts as the middleware and nothing more.

## 5. Absolute rules (D87)
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

## 6. What the middleware does with the facts
Described in [middleware.md](middleware.md); in one line each:
- **identity** maps native actors to one person id, registers new ones, merges on `/link`;
- **privacy** labels each event with its place's audience at that moment and lets an item into
  a context only when every member of the place's audience is in the item's audience;
  *unknown* lets no private item in;
- **policy** holds the two absolute lines, the budget, quiet hours, brakes and approvals;
- **roles** give permissions to person ids;
- **registry** lists capabilities, commands and skills, offers each turn what its requester may
  use, routes actions by place;
- **interaction** emits lifecycle events and asks questions.

## 7. Tests that hold the contract (D88)
- **The contract is code.** Its types live in one package, `monoagens/contract/`, imported by the
  core (waist 1), the middleware (both) and adapters and plugins (waist 2). The core imports
  nothing else from outside itself.
- **The lock.** `contract.lock` holds a hash of `monoagens/contract/`. A test fails when they
  differ. Only the owner commits a new hash, together with a decision in this file (§9).
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
- **One conformance suite** runs over every adapter: every member of §4.1 exists, every
  percept matches §4.2, every action gets exactly one receipt, a repeated action id is
  deduplicated, a lifecycle event never breaks the body.

## 8. Five test bodies
Before the lock and after every change to the contract, five fake bodies in `tests/bodies/` run
through the real core and middleware with the fake model, **with no change to the contract**:
a chat (private and shared places, quotes, files), a robot (a room, people coming and going,
speech, a gesture capability), a workspace app (a document, collaborators, an edit capability
with conflicts), a sensor (no actor, a value changing past a threshold, a read capability) and a
game (a world, players in range, movement). If one of them needs a new field, the contract is
still shaped by another body, and it is fixed before it is locked.

## 9. Changing the contract
- **Additive only**: a new optional field, a new feature name, a new optional surface, a new
  event kind. Readers ignore what they do not know.
- **Experimental first**: a new item is marked *experimental* here and may change; it becomes
  *stable* by an owner decision after it has worked in real use. Stable items never change.
- **Meaning never changes.** A field that would need a new meaning is a new field; the old one
  stays, deprecated. A removed name is never reused.
- **Formats carry their version**; older ones are upcast on read, stored events are never
  rewritten (D58).
- **Who**: only the owner, with a decision recorded here and a new `contract.lock`. A task, a
  worker or a plugin that needs a change stops and reports.
- **A new case is classified before anything is written**: a wrong fact from an adapter → fix
  the adapter; the model misunderstood → the principles or the body's `prompt_section`; a broken
  mechanism → fix it inside, contract and contract tests unchanged; none of these → a question
  for the owner, never a patch.

## 10. What contract v2 removes
For the migration (Task 6). Each item below existed before 2026-11-14 and is gone:

| Removed | Instead |
|---|---|
| `place_kind`, place info (`form`, `parent`, `name`, `members`, `can_post`, `roles`) | `place.audience`; names and structure in `description` / `prompt_section`; `refused` receipts |
| `addressed` as a percept field, `sent_at` | `features.addressed`; `occurred_at` |
| one neutral `say` / `react` / `edit` in the middleware (D83), its fields and fallbacks | each body's own capabilities and schemas |
| `contract.SHOWS`, per-adapter ladders, `show:` in the middleware | lifecycle events; each body's own rendering options |
| splitting attachments and sticker fallback in the middleware | the body does it and answers with one receipt |
| `platform_role`, platform role facts | roles on person ids only; a plugin may assign roles through the role commands |
| `yes_words`, stopwords, `text_like`, topic word filters, command syntax stripping | reasoning understands; `answer` and `command` are structured |
| the core reading `words_hidden`, `place_kind`, `audience`, `hold_for`, `delivers` | the middleware decides in its hooks; the core carries what the hooks return |
| regexes over rendered context lines; a second privacy check (`kernel.visible`) | context items with sources; one check in the context hook |
| `share` without an item and delegation guessing | `share` names item or episode ids, checked against the requester's audience |
| re-sending unconfirmed actions after a restart | deterministic action ids and receipts (§3) |
| the permission exception for talking capabilities | every capability names a permission; bodies declare theirs (base `guest` for talking) |
