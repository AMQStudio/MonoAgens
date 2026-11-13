# Task 6 — The locked contract

Status: todo | Depends on: Task 5 (stopped at 1.12 P, 2026-11-14)
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules:
[README](README.md).

**Goal.** The code implements [contract.md](../contract.md) (v2, D86–D88) exactly, and then the
contract is locked. Everything shaped by one platform leaves the core and the middleware; every
adapter translates its own world into the six facts; tests speak only through the contract; five
very different fake bodies run without a single change to it. After this task there is no "core
change" or "middleware change" of the contract kind again: new things enter as adapters or
plugins.

**Rules for this task** (on top of [README](README.md) and [AGENTS.md](../../AGENTS.md)):
- This is the one task that may reshape contract-facing code across the core, the middleware,
  the adapters and the plugins, and rewrite or delete tests that touch internals. It happens once.
- No behavior beyond the contract and the audit items named below. Anything the contract does not
  cover stops the work and goes to the owner; never extend the contract yourself.
- Prefer deleting to adding: most of what goes is in [contract.md §10](../contract.md). Record
  the line counts of `monoagens/core/`, `monoagens/middleware/`, `monoagens/adapters/`,
  `plugins/` and `tests/` before you start and after each part, under Notes.
- Every rule has one home (AGENTS.md invariant 13). Note each fix's home under Notes.
- After each part: `uv run pytest` and `uv run python scripts/replay.py --fake` pass; commit, push.
- Audit items carried from Task 5 1.12 keep their ids (S.6, E.1–E.4, F.x, O.x); a carried item
  that the contract removes (F.6–F.8: the middleware no longer shows or splits) is ticked as
  removed, with the commit that removed it.

---

## Part 1 — The contract as code, the lock and the ratchet (first)
Read first: [contract.md](../contract.md) (all), V1's `engine/inbound.py` and `engine/registry.py`
(branch `archived`)
May touch: `monoagens/contract/` (new), `contract.lock` (new), `tests/contract/` (new),
`tests/architecture/` (new), `tests/bodies/` (new), `pyproject.toml` (test markers only)

- [ ] 1.1 `monoagens/contract/`: the types and validators of contract.md §3 and §4 — event
  envelope, feature names, capability descriptor (facts, permission, the closed schema subset with
  V1's keyword validator), hook outcomes (`Continue`, `Replace`, `Stop(reason, held)`), context
  item, action, receipt states, the adapter surface as Protocols (mandatory and optional), percept,
  `place.audience`, lifecycle events, `answer`, `command`, attachment. Pure types; it imports
  nothing from the core, the middleware, adapters or plugins.
- [ ] 1.2 `contract.lock`: the hash of `monoagens/contract/`, and `tests/contract/test_lock.py`
  that fails when they differ. The worker writes the first hash here; from 7.3 on only the owner
  changes it.
- [ ] 1.3 Architecture tests as a **ratchet** (`tests/architecture/`): every ban of contract.md §7
  as a test over the source tree; today's violations are listed in
  `tests/architecture/baseline.yaml` (file, line, ban). The test fails on a new violation **and**
  on a listed one that no longer exists, so the list can only shrink. It must be empty by 6.4.
- [ ] 1.4 The five fake bodies (`tests/bodies/`: chat, robot, workspace app, sensor, game,
  contract.md §8) as adapters on the contract surface only, and the conformance suite
  (`tests/contract/test_conformance.py`) over them: members of §4.1, percepts of §4.2, one receipt
  per action, deduplication by action id. They run through the kernel in Part 6.

## Part 2 — The core on waist 1
Read first: contract.md §3, [core.md](../core.md) §3–§6, §8–§9
May touch: `monoagens/core/` (all), `monoagens/infra/store.py`, `monoagens/versioning.py`,
`sql/migrations/` (one new migration), `tests/core/`, `scripts/replay.py`

- [ ] 2.1 Event envelope v2 (core.md §3, §9.2): `occurred_at`, `description`, `features`,
  `backfill`, `imported`, `labels` as envelope fields, `schema_version` 2; version 1 events and
  percepts upcast on read (`sent_at` → `occurred_at`, `addressed` → `features.addressed`,
  `place_kind` dropped).
- [ ] 2.2 The core never reads `payload` or `labels`. Every steer the middleware gives today
  through a label (`words_hidden`, `place_kind`, `audience`, `hold_for`, `delivers`, `hold`) reaches
  the core only as a hook outcome (`Replace`, `Stop`, `Stop(held=True)`). A steer that no outcome
  can express stops the work and is reported.
- [ ] 2.3 Features: `own_name` reads `named`, `social_cost` reads `addressed_other`, the talk rate
  may read `audience_size`; the core searches no text for the soul's name.
- [ ] 2.4 `cue_event` keeps facts only (kind, who, place, not_who); `text_like` is refused with
  its reason; stored ones are upcast to a free-text condition.
- [ ] 2.5 The context as items with sources (contract.md §3); the context hook receives items;
  the text is rendered after the last hook.
- [ ] 2.6 Actions and receipts: deterministic action ids; every `act.out` ends in exactly one
  `act.result` with a state; `refused` wakes reasoning (bounded by the brakes), `held` does not;
  after a restart an action without a receipt goes back to its body with the same id; nothing is
  re-sent by the core on its own. `act.out` records the arguments as the action hook returned
  them (F.10).
- [ ] 2.7 Liveness (F.2–F.5): a tool call runs under the job watchdog and a timeout is a
  `failed` receipt; one failing call inside a round becomes its own receipt and the other calls
  still run; a tool result is kept when the same wake also spoke; the loop survives an exception
  in the backlog flush and in `recover()`, and one failing fired Expectation loses no other.
- [ ] 2.8 Speaking elsewhere: the intent's content is what the action hook returns; it never
  carries the goal's or the trigger's text; the core reads no label for it.

## Part 3 — The middleware on waist 2
Read first: contract.md §4–§6, [middleware.md](../middleware.md) (all)
May touch: `monoagens/middleware/` (all), `monoagens/infra/` (attachment store, budget),
`config.yaml`, `tests/middleware/`, `scripts/replay.py`

- [ ] 3.1 The contract types move out of `middleware/contract.py` into `monoagens/contract/`;
  the middleware imports them; what contract v2 removed is deleted (`SHOWS`, ladders, the neutral
  `say` fields).
- [ ] 3.2 Identity on actors (middleware.md §7): the actor map over `actor` and `involves`;
  `about` resolved to an event id from the Log; place info removed; the `self_actor` line.
- [ ] 3.3 Privacy (middleware.md §3): labels from the latest `place.audience`; **one** check over
  context items in the last context slot, plus attachments on open and forward. Delete the regexes
  over rendered lines, the second filter, `kernel.visible`, `_delegated_share_source`, the
  stopword and topic-word code. `share` takes ids; D80 by `about` or the chain's `wait`.
- [ ] 3.4 Registry (middleware.md §4): each body's own capabilities, routed by place ownership,
  an action for another body's place refused with its reason; no neutral `say`, nothing hidden, no
  permission exception; commands from `command`; a failing handler answers with an error; command
  answers through the action hook (F.12); `registry.more` without a place refused, capped.
- [ ] 3.5 Interaction (middleware.md §9): lifecycle events to each body's `render` as best-effort
  rules with their own timeout; questions through `ask` with `question.asked`, `answered`,
  `expired` events; the `answer` capability with its fact checks; an approved act runs as its
  requester and still runs after a restart (F.9). Delete `yes_words`, typed matching, `split`,
  the sticker fallback.
- [ ] 3.6 Roles (middleware.md §8): person ids only; `platform_role` and platform role facts
  deleted; person cards read the roles at call time.
- [ ] 3.7 Loader and config (O.1, O.2, O.4): `/reload` with adapters running, validation before
  any change, rules never duplicated and removed with their plugin; every config edit parsed back,
  atomic, comments kept; `/plugin update` reads the recorded defaults; a missing requirement or
  dependency fails boot; the boot checks of contract.md §3 and §4.1.
- [ ] 3.8 Budget (O.3): every round's usage recorded, failed ones included; the budget update
  atomic; cached tokens read; a fallback provider uses its own key only.

## Part 4 — Adapters on the contract
Read first: contract.md §4, [adapters.md](../adapters.md) (all), Task 5 1.11 (fakes from the
real thing, recording mode)
May touch: `monoagens/adapters/` (all), `monoagens/__main__.py` (supervision only),
`monoagens/infra/` (a recording helper), `.gitignore`, `scripts/recording.py` (new),
`tests/adapters/`, `tests/fixtures/`

- [ ] 4.1 CLI on the surface of contract.md §4.1, as the reference adapter.
- [ ] 4.2 Telegram, with E.2.
- [ ] 4.3 WhatsApp, with E.1.
- [ ] 4.4 Discord, with E.3.
- [ ] 4.5 Every adapter (E.4): its own capabilities and `prompt_section`; features; `place.audience`
  with its platform's rule folded in (adapters.md §3); `render` from its own `render:` options;
  `ask`; deduplication by action id; splitting and length limits by itself; a fresh client on
  restart; a backoff that does not reset after a short run; the CLI socket created with its mode.
- [ ] 4.6 Fakes from the real thing: `create_autospec` of the neonize and discord.py clients,
  recorded Telegram Bot API responses (errors included); the conformance suite of 1.4 over all four
  adapters.
- [ ] 4.7 Recording mode (`bodies.<adapter>.record: true` into `var/recordings/`, git-ignored)
  and `scripts/recording.py anonymize` to turn a recording into a committed fixture; shadow mode
  (`dry_run: true`: everything sensed, nothing sent, each would-be action recorded with
  `dry_run`).

## Part 5 — Plugins and the remaining audit items
Read first: [plugins.md](../plugins.md), contract.md §4.8
May touch: `plugins/`, `monoagens/core/expectations.py`, `monoagens/core/memory/`,
`monoagens/middleware/lang/`, `tests/plugins/`, `tests/core/`

- [ ] 5.1 S.6: a cancelled run kills its sandbox; a ceiling on the run timeout; a plugin gets only
  the environment variables its manifest declares; `web-fetch` total timeout and size cap;
  `pdf-reader` page and size caps; blocking calls through `asyncio.to_thread`.
- [ ] 5.2 Every plugin on the contract: facts, permissions in its own namespace, no text parsing
  in regulating middleware (a narrowing rule works on facts: place, person, kind).
- [ ] 5.3 O.5: an Expectation's default expiry is never before its next cue; `cooldown`, `once`
  and `max_fires` hold on the event path and for reminders; an Expectation that can never fire is
  refused when made.
- [ ] 5.4 O.6: `LogView.search` escapes `%` and `_`; recall uses the injected settings; the
  newest active Expectations are listed; a retrieval counts a use once; the inbound encode runs
  after the inbound hook; the remaining English strings move to the language catalog.

## Part 6 — Tests through the contract, five bodies, invariants
Read first: contract.md §7–§8, [AGENTS.md](../../AGENTS.md) (Tests)
May touch: `tests/` (all), `scenarios/`, `scripts/replay.py`

- [ ] 6.1 Every test that calls a private function, asserts on an internal payload or on prompt
  wording is rewritten through the contract or deleted; the architecture test refuses private
  imports in `tests/`. This is the only time tests are rewritten in bulk.
- [ ] 6.2 The five fake bodies of 1.4 run through the real core and middleware with the fake
  model: percepts in, audiences, capabilities out, receipts, lifecycle events, questions and
  answers, back-fill — **with no change to `monoagens/contract/`**. A body that needs one stops the
  work and is reported.
- [ ] 6.3 Hypothesis stateful invariants (contract.md §7): no context item outside its audience;
  exactly one receipt per action; nothing sent twice across restarts; every Stop recorded; a merge
  never splits; the owner's direct place sees everything.
- [ ] 6.4 The architecture baseline of 1.3 is empty.
- [ ] 6.5 The fault-injection test (D82): each adapter member, each best-effort rule and each tool
  fails in turn; the soul keeps working and records the error.
- [ ] 6.6 Scenarios on contract v2 facts (`place.audience`, features); scenarios 18 and 19 check
  behavior, not prompt text.

## Part 7 — Lock and real use
- [ ] 7.1 Line counts before and after, per folder, under Notes.
- [ ] 7.2 The real-model replay launch set against Task 5's A.4 baseline (the model in `.env`;
  never read or print it).
- [ ] 7.3 **Stop and report to the owner.** The owner reviews and commits `contract.lock`; from
  then on only the owner changes it (contract.md §9).
- [ ] 7.4 A live session on WhatsApp in the owner's direct chat and one group; the failures seen
  live since 2026-11-13 (Task 5 1.11 step 4) become adapter tests from recordings. From here the
  owner uses the soul daily.

**Done when.** All parts are ticked; the architecture baseline is empty; the five bodies pass with
no contract change; `contract.lock` is committed by the owner; the soul is used daily on WhatsApp.

**Notes.**
