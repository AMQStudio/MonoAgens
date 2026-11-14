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

- [x] 1.1 `monoagens/contract/`: the types and validators of contract.md §3 and §4 — event
  envelope, feature names, capability descriptor (facts, permission, the closed schema subset with
  V1's keyword validator), hook outcomes (`Continue`, `Replace`, `Stop(reason, held)`), context
  item, action, receipt states, the adapter surface as Protocols (mandatory and optional), percept,
  `place.audience`, lifecycle events, `answer`, `command`, attachment. Pure types; it imports
  nothing from the core, the middleware, adapters or plugins.
- [x] 1.2 `contract.lock`: the hash of `monoagens/contract/`, and `tests/contract/test_lock.py`
  that fails when they differ. The worker writes the first hash here; from 7.3 on only the owner
  changes it.
- [x] 1.3 Architecture tests as a **ratchet** (`tests/architecture/`): every ban of contract.md §7
  as a test over the source tree; today's violations are listed in
  `tests/architecture/baseline.yaml` (file, line, ban). The test fails on a new violation **and**
  on a listed one that no longer exists, so the list can only shrink. It must be empty by 6.4.
- [x] 1.4 The five fake bodies (`tests/bodies/`: chat, robot, workspace app, sensor, game,
  contract.md §8) as adapters on the contract surface only, and the conformance suite
  (`tests/contract/test_conformance.py`) over them: members of §4.1, percepts of §4.2, one receipt
  per action, deduplication by action id. They run through the kernel in Part 6.

**Parts 2 and 3 interleave where one change spans both** (architect, 2026-11-14, from the
worker's report): **2.5 and 3.3 are one item**, done together (the context hook's item type and
the one privacy check over it), and **2.2's label half is done together with 3.2, 3.3 and 3.5**
(a label can leave the core only when the middleware returns the hook outcome instead). Every
commit still keeps `uv run pytest` and the fake replay green; tick both items when the combined
change lands. Order: 2.2 (rendering half), 2.4, 2.6, 2.7, 2.8, 2.9, then 2.5 + 3.3 with 3.1 and
3.2, then 2.2 (label half) with 3.5, then 3.4, 3.6–3.8.

## Part 2 — The core on waist 1
Read first: contract.md §3, [core.md](../core.md) §3–§6, §8–§9
May touch: `monoagens/core/` (all), `monoagens/infra/store.py`, `monoagens/infra/versioning.py`,
`monoagens/versioning.py`, `sql/migrations/` (one new migration), `tests/` (any test that
asserts behavior the contract removes, e.g. `text_like`), `scenarios/`, `scripts/replay.py`;
`monoagens/contract/` and `contract.lock` only for 2.9

- [x] 2.1 Event envelope v2 (core.md §3, §9.2): `occurred_at`, `description`, `features`,
  `backfill`, `imported`, `labels` as envelope fields, `schema_version` 2; version 1 events and
  percepts upcast on read (`sent_at` → `occurred_at`, `addressed` → `features.addressed`,
  `place_kind` dropped).
- [ ] 2.2 The core never reads `payload` or `labels`. Every steer the middleware gives today
  through a label (`words_hidden`, `place_kind`, `audience`, `hold_for`, `delivers`, `hold`) reaches
  the core only as a hook outcome (`Replace`, `Stop`, `Stop(held=True)`). A steer that no outcome
  can express stops the work and is reported.
- [x] 2.3 Features: `own_name` reads `named`, `social_cost` reads `addressed_other`, the talk rate
  may read `audience_size`; the core searches no text for the soul's name.
- [x] 2.4 `cue_event` keeps facts only (kind, who, place, not_who); `text_like` is refused with
  its reason; stored ones are upcast to a free-text condition.
- [ ] 2.5 The context as items with sources (contract.md §3); the context hook receives items;
  the text is rendered after the last hook.
- [x] 2.6 Actions and receipts: deterministic action ids; every `act.out` ends in exactly one
  `act.result` with a state; `refused` wakes reasoning (bounded by the brakes), `held` does not;
  after a restart an action without a receipt goes back to its body with the same id; nothing is
  re-sent by the core on its own. `act.out` records the arguments as the action hook returned
  them (F.10).
- [x] 2.7 Liveness (F.2–F.5): a tool call runs under the job watchdog and a timeout is a
  `failed` receipt; one failing call inside a round becomes its own receipt and the other calls
  still run; a tool result is kept when the same wake also spoke; the loop survives an exception
  in the backlog flush and in `recover()`, and one failing fired Expectation loses no other.
- [x] 2.8 Speaking elsewhere: the intent's content is what the action hook returns; it never
  carries the goal's or the trigger's text; the core reads no label for it.
- [x] 2.9 The contract package holds types and validators only: `upcast_event` and its helpers
  move from `monoagens/contract/envelope.py` to `monoagens/infra/versioning.py`, so a fix to
  upcasting never needs the lock; regenerate `contract.lock` once in this item and say so in the
  commit message.

## Part 3 — The middleware on waist 2
Read first: contract.md §4–§6, [middleware.md](../middleware.md) (all)
May touch: `monoagens/middleware/` (all), `monoagens/core/` (only for the combined items 2.2
and 2.5 above), `monoagens/infra/` (attachment store, budget), `config.yaml`, `tests/` (any test
that asserts behavior the contract removes), `scenarios/`, `scripts/replay.py`

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

### Baseline and decisions (before any edit, at `44f9c1f`)

Line counts: core 3842 · middleware 6144 · adapters 5465 · plugins 2065 · tests 18577
(`find <dir> -type f \( -name '*.py' -o -name '*.yaml' -o -name '*.sql' \) -exec cat {} + | wc -l`).

Known-red at baseline: `tests/core/test_expectations.py::test_a_message_ends_the_step_so_its_result_does_not_wake_again`
(17 `model.call`, expected 3). Not fixed before the parts: it comes from the unfinished 1.12
F.2-F.5 WIP commit `0b1a2e6` (`jobs.apply_calls`, `kernel` guards, `working.step_spoke`), and
Part 2.7 (F.2-F.4) plus 6.1 own that code. The rest of the tree is green (664 passed, 1 skipped)
and `scripts/replay.py --fake` passes 158/158 checks.

Decisions carried from the plan (one line each):

- **D-A** pairing is a command: adapters translate a native command into `command {name, args}`;
  the middleware looks the name up and never strips syntax.
- **D-B** the core reads no `payload` and no `labels`; its own structured data lives in the books
  (`state: action:<id>`, `state: merged:<from>`) or the in-process return value; the one store read
  is `store.merge_source`.
- **D-C** receipts are returned in-process, not parsed from the Log; a `Refusal` lost to a restart
  is re-obtained by handing the same action id to the body again.
- **D-D** context items are the only context; "annotate" is `Replace` with the item's text extended.
- **D-E** action ids are `f"{job_key}:{job_started_event_id}:{round}:{n}"`.
- **D-F** the action ledger lives in `state: action:<id>`; it replaces `sends.py` and every
  `payload->>'communicative'` query.
- **D-G** a held action is recorded (`act.result {state: held}`), never sent; the middleware
  releases it later as a new act.
- **D-H** `Kernel.act_via_hooks(trigger, capability, arguments)` is the one public door command
  answers, approvals and brake prompts use; authority stays in the action hook.
- **D-I** every installed skill is offered in the semi-stable block; no word overlap.
- **D-J** the talk-rate fair share uses `features.audience_size` when > 0, else distinct percept
  speakers + 1.
- **D-K** a stored `text_like` is upcast to `condition.text` in memory at read; new ones are
  refused; stored rows are never rewritten.
- **D-L** `monoagens/types.py` becomes a re-export of the contract types (`Event`,
  `CURRENT_SCHEMA_VERSION`; `StateEntry` stays); `RECENT_KINDS` moves to `monoagens/core/kinds.py`.
- **D-M** one transitional producer path: `Kernel.put`/`_settle` run incoming events through
  `contract.percept.from_legacy_event` until Part 4 deletes the call.
- **D-N** the database default of `events.schema_version` stays 1; `append_event` passes
  `schema_version=SCHEMA_VERSION` explicitly.

### Part 1 — the contract as code, the lock and the ratchet

Line counts after Part 1: core 3842 · middleware 6144 · adapters 5465 · plugins 2065 ·
tests 19949 (baseline 18577). New: `monoagens/contract/` 805, `contract.lock` 13.
Nothing production-side moved yet; the only new test trees are `tests/contract/`,
`tests/architecture/`, `tests/bodies/`.

One home per fix:

- the contract types and validators: `monoagens/contract/` (11 modules, no `monoagens` import);
- the lock: `contract.lock` written by `tests/contract/test_lock.py` (`--write`), compared by
  the same module; only the owner rewrites the hash;
- the seven bans: `tests/architecture/support.py` (one finder per ban), the ratchet test in
  `tests/architecture/test_ratchet.py`, today's 161 findings in `baseline.yaml`;
- the five bodies: `tests/bodies/` on `tests/bodies/base.py`; the one conformance suite in
  `tests/contract/test_conformance.py` (parametrized, later over the four adapters).

Decisions taken while implementing (not questions, recorded so the reader sees them):

- `Answer` and `Command` are defined in `contract/percept.py`: contract.md §4.2 names them as
  percept fields, the plan's module list gave them no home, and they belong to the percept.
- `descriptor_errors(..., require_facts=True)` requires the `facts` **key**, not a non-empty
  list: a talking capability states `facts: ()` honestly (every adapter descriptor does today),
  so "a capability without facts fails boot" means the key. `FLAGS` (`communicative`,
  `long_running`, `identity_changing`) are validated as booleans.
- a "word list" (ban 2) is **one name** bound to a literal of at least three strings; `__all__`
  and a tuple unpacked into several names are name lists, not word lists.
- imports inside `monoagens/contract/` are relative, so `test_contract_package_imports_nothing_from_monoagens`
  and ban 6 hold literally.

Carried audit items: none yet (S.6, E.1-E.4, F.x, O.x belong to Parts 3-5).

### Part 2 — the core on waist 1

#### 2.1 Event envelope v2

Line counts after 2.1: core 3878 (+36) · middleware 6147 (+3) · adapters 5465 · plugins 2065 ·
tests 19992 (baseline 18577). Contract 812.

One home per fix:

- the envelope and the one v1 upcast: `monoagens/contract/envelope.py` (`upcast_event`);
- the envelope's columns: `sql/migrations/005_envelope_v2.sql` + `sql/schema.sql`
  (`occurred_at` is filled from `ts` by the `events_set_occurred_at` trigger, so every existing
  INSERT keeps working, and the immutability trigger now also allows hard forgetting to scrub
  `description` and `features`);
- writing an event: `monoagens/infra/store.py` (`EVENT_COLUMNS`, `append_event`,
  `append_envelope`) — `append_envelope` is the one place the core hands an event's opaque
  fields to the Log, so the core never reads `labels`;
- incoming percepts: `Kernel.put`/`_incoming` (`Percept` → `to_event`, v1 `Event` →
  `from_legacy_event`), `Kernel.act_via_hooks` for the middleware's own acts (D-H);
- the recent kinds: `monoagens/core/kinds.py` (`RECENT_KINDS`).

Deviations and decisions, recorded:

- `monoagens/types.py` re-exports `Event` and `CURRENT_SCHEMA_VERSION` from the contract but keeps
  the version 1 `Action` shape until Part 4: the adapters and the tool registry still build
  `Action(kind=, place=, payload=)` and both are outside Part 2's reach. The re-export of `Action`
  happens with the cutover that makes it true.
- `payload_text` stays in `monoagens/infra/store.py`: `infra/observability.py` (outside every
  part's reach) and four middleware modules read it, and it reads a payload dict, not an event.
  The core's own uses go in 2.2.
- `append_event` derives the fields a caller leaves empty from the payload **through
  `upcast_event`**, one home for the version 1 shape, so the transitional producers need no
  change while the envelope fills up.
- `tests/infra/test_versioning.py` loses its two tests of the version 1 payload upcaster
  (`UPCASTERS`): that mechanism is replaced by `contract.envelope.upcast_event`, and the tests
  asserted its internals. The version column, the kind namespaces and the plugin kinds stay.
- `contract.lock` was rewritten twice while 2.1 was in flight: `upcast_event` now returns the row
  at `schema_version` 2 (a row read at the current version cannot claim version 1) and
  `from_legacy_event` keeps `body=None` as `None`. Both are meaning fixes inside the same
  envelope; the contract does not change again from this commit on, which is what 6.2 tests.
- two stale envelopes had to be healed together with the columns, because the row shape moved
  under them: `monoagens/core/sends.py` and `monoagens/middleware/contract.py` (`LogView`) built
  their own 10-column list, and `memory/retrieval.recent_lines` selected one. All three now use
  `store.EVENT_COLUMNS`.
- the architecture ratchet: `RECENT_KINDS` is a vocabulary of event kinds, not a word list, so
  ban 2 exempts it by name (`KIND_LISTS` in `tests/architecture/support.py`).

#### 2.3 Features and the gate

Line counts after 2.3: core 3880 · middleware 6147 · adapters 5465 · plugins 2065 · tests 19995.

One home per fix:

- `own_name` and `social_cost`: `monoagens/core/gate/signals.py` reads `features.named`,
  `addressed` and `addressed_other`; the regex over event text is gone;
- `Situation.text` is `event.description`, and `_floor_flags` keeps only `soul_answered`
  (`gate/situation.py`);
- `_addressed` and the back-fill age read `features.addressed`, `event.backfill` and
  `event.occurred_at` (`gate/appraise.py`);
- the talk-rate denominator is `features.audience_size` when it is > 0, else distinct percept
  speakers + 1 (`gate/formula.inputs`, D-J);
- the memory query text is `event.description` (`memory/retrieval.candidates`), and a fired
  Expectation now carries `description=about` (`store.fire_expectation`), so the desk and the
  retrieval read the envelope instead of the payload;
- the per-act `step_spoke` rule (F.4) lives in `core/context/working.py`: the act that spoke ends
  its own step and a tool's result is kept. `dispatch` records the `act.out` id as the
  `act.result`'s `caused_by` — without it neither rule can see its act.

The baseline's red test (`test_a_message_ends_the_step_so_its_result_does_not_wake_again`, 17
`model.call`) is green: the WIP's per-act `step_spoke` together with an `act.result` that had no
`caused_by` made every round wake again.

Deviations, recorded:

- `tests/core` speaks in features and descriptions now (test_gate's `_msg`, the
  `own_name`/`social_cost`/back-fill tests, the `payload->>'description'` reads). The adapters
  report `named` and `addressed_other` from Part 4 on; until then those two features stay 0 for
  real traffic.
- exactly one `act.result` per act (contract.md §3) is 2.6's other half and is **not** in this
  commit: `dispatch` still records one only when the tool returns something. Making it universal
  reorders eight pinned event sequences in `tests/core/test_reasoning.py`, and that migration
  belongs with the deterministic action ids and the ledger.

#### 2.2 (rendering half) and 2.6 Actions and receipts

Line counts after this commit: core 3984 (+104) · middleware 6144 · adapters 5465 ·
plugins 2065 · tests 19960 (baseline 18577). Contract 807, `contract.lock` untouched.

The rendering half of 2.2 and 2.6 landed together: the core's run-time reads of `payload` for
the desk's lines and the ledger that replaces them are one change, because a reader without a
writer leaves `<doing>` blank and a writer without ids leaves nothing to resume.

One home per fix:

- the core's desk lines read the envelope: `core/context/parts.py` (`_recent_parts`,
  `_event_text`, `_reminder_text`) and `core/context/working.py` (`chain_root_info`,
  `_chain_events`);
- a chain's step and result lines read the ledger: `core/context/working.py`
  (`_ledger`, `_step_text`, `_result_text`) through `store.actions_of`;
- the action ledger: `monoagens/infra/store.py` (`write_action` writes `action:<id>` and the
  `act:<event id>` pointer in one call, `close_action`, `action_of`/`actions_of`,
  `unresolved_actions`);
- the deterministic action id: `contract.action_id` called once, in `core/dispatch.action_for`
  (`JobRef` from `core/jobs.Job.started_event_id`, round index, index of the act in its round);
- exactly one receipt per act, and the wake rule: `Kernel._settle_action` (held is recorded and
  never wakes; a message that already ended the step records `decision.silent`; the result is
  settled so its best-effort shows still run) with `Kernel._decided` as the idempotence check;
- resume after a restart: `Kernel._resume_actions` over `store.unresolved_actions` (same id,
  `sends.py` deleted with `act.confirmed`);
- a stored `text_like` cue becomes a free-text condition on read: `store._upcast_cue` (D-K),
  needed here because `expectations.matches_cue` stopped reading words;
- the empty answer costs no model call: `Kernel._settle_action`.

Deviations and decisions, recorded:

- `step_spoke` survives in `core/context/working.py` until 2.7 (F.4): it answers the same
  question the ledger will, and 2.6's wake rule is what replaces it.
- An act whose body answers `None` records its `done` receipt but does not wake reasoning: an
  empty answer carries no news, and waking on it spends one model call per react. The receipt is
  still the contract's one answer per act.
- A refusal this door makes itself (invalid arguments, unknown capability, a second message to
  the same place) records its `act.out` and its `act.result` like any other act.
- `Appraisal.payload()` is now `Appraisal.record()`: it builds a decision's payload, it never
  reads one, and the ratchet's ban 3 read the name.
- `types.Action` gains `id` and renames `payload` to `arguments`, so the id reaches the body now;
  the switch to `contract.Action` stays with 3.4.
- `Kernel.enqueue` marks an event that already has an id as recorded, so the consumer wakes on it
  without writing it a second time (the first version inserted the receipt twice).
- tests: the pinned event sequences gained the universal `act.result` (and lost `act.confirmed`);
  `tests/core/test_sends.py` became `tests/core/test_action_ledger.py`; the two chain tests now
  write their ledger rows and read `<doing>` through them; `_rich()` and every hand-built `Event`
  carry their `description`; the `notes` payload branch's test is deleted (the contract removed
  that field).

#### 2.4 Cues keep facts only

Line counts after 2.4: core 3996 · middleware 6144 · adapters 5465 · plugins 2065 · tests 19988.

One home per fix:

- the refusal: `core/expectations._refuse_words`, called for `cue_event` and for an
  `event_seen` pattern, with the reason
  `cue_event takes facts only (kind, who, place, not_who), not <keys>; use expect: {text: ...}
  for words`;
- matching: `core/expectations.matches_cue` compares kind, who, place and `not_who` only;
  `is_met` reads `not_who` as `who <> %s` and no longer searches text;
- the one upcast: `infra/store._upcast_cue` at `read_expectations`, so a stored `text_like`
  becomes `condition {"text": phrase}` in memory and the row is never rewritten (D-K);
- the wait cue's read: `middleware/privacy` now goes through `store.read_expectations`, so a
  wait's cue is upcast the same way and there is no second reader;
- what the model is told: the `expect` tool's `cue_event` description names the four fact keys
  and says words belong to `expect.text`.

Decisions, recorded: the refusal is an allowlist (any key outside the four facts), not a check
for one name: a cue that carries a second word key would otherwise pass silently. The message
names the offending keys from the call, so no removed field name is written in the core.

#### 2.7 Liveness (F.2-F.5)

Line counts after 2.7: core 4043 · middleware 6136 · adapters 5465 · plugins 2065 · tests 20060.

One home per fix:

- **F.2** the job watchdog: `core/dispatch.carry` runs the body under `asyncio.wait_for(...,
  jobs.timeout)` and answers `Receipt("failed", reason="<capability> timed out after Ns")`;
  `middleware/registry.act` no longer keeps a second timeout (one home for the watchdog);
- **F.3** one call's failure: `core/jobs.apply_calls` tries every call of every round and
  records `error` for one that raises; the act's own receipt is the `failed` one `carry` returns;
- **F.4** a result is kept when the wake also spoke: `working.step_spoke` (per act) and
  `Kernel._settle_action`'s silent decision;
- **F.5** the loop survives: `Kernel.run` guards `recover()` and the idle-poll backlog flush,
  `run_until_idle` guards its flush, `_flush_backlogs` drops a place's backlog only when there is
  a reasoning to wake, and `expectations.due(on_error=...)` isolates one malformed row:
  `Kernel._wake` passes `partial(self._record_error, event)`.

New test: `test_f5_one_malformed_expectation_loses_no_other_fired_one` (a bad `within` is
reported and the sound row still fires).

#### 2.8 Speaking elsewhere

Line counts after 2.8: core 4045 · middleware 6165 · adapters 5465 · plugins 2065 · tests 20055.

One home per fix:

- the intent's line: `core/dispatch._intend` quotes only what the action hook let through
  (the returned item) and records the arguments reasoning asked for (`asked`), so the Log keeps
  the drafted words while the other place's desk hides them (D45);
- the decision to hide words: `middleware/privacy.guard`'s last pass returns `Replace` without
  `text` when `_words_hidden` holds, in place of the deleted `words_hidden` label;
- the intent's payload: no `place_kind`, and `Kernel._wake` reads `event.description`;
- the goal is not read at all: `working.chain_root` is no longer called by the door, and the
  `act.intent` event carries no `why`.

Decisions, recorded: the core builds the neutral line when the hook's item carries no words, so
"its words cannot appear here" is now the else branch of a fact (the returned item), not a label;
`middleware/privacy.intent_words` (an inbound rule) is the safety net for an intent whose source
was only known after the act — it replaces the line on the way in, and the immutable Log keeps
the row the core wrote.

#### 2.9 The contract holds types and validators only

Line counts after 2.9: core 4045 · middleware 6165 · adapters 5465 · plugins 2065 · tests 20055.
Contract 749 (-58); `monoagens/infra/versioning.py` 72 -> 132.

One home per fix:

- the one version 1 -> 2 upcast: `monoagens/infra/versioning.py` (`upcast_event`,
  `_v1_description`, `_as_time`), outside the lock, so a fix to how an old row is read never
  needs the owner;
- the envelope: `monoagens/contract/envelope.py` keeps `Event`, `SCHEMA_VERSION`, `FEATURES` and
  the one `__post_init__` rule; `monoagens/contract/__init__.py` no longer exports the upcast;
- the store's reads: `monoagens/infra/store.py` imports it from `monoagens.infra.versioning`.

`contract.lock` was regenerated **once**, here, because `upcast_event` left the package:
`sha256:cadfbadac4a03cda5e0ab0d8c25d95b3167c8304fcf9383a1539f2d5dcdcb72f`.
From this commit on the contract does not move again (6.2's promise).

#### DEC-2: one hook vocabulary

`monoagens/core/hooks.py` now re-exports `Continue`, `Replace`, `Stop`, `HookOutcome`,
`Middleware`, `Registration`, `SLOTS`, `OWNER_POLICY` and `RULE_KINDS` from
`monoagens.contract.hooks`, so the core and the middleware speak one set of names
(`middleware/contract.py` imports the same ones). `Registration.middleware` is the contract's
`rule`, `HookPipeline.run` calls `on_stop(reason, held)`, and `guard.stopped` carries
`{"hook", "reason", "held"}` — the one place a held action is visible in the Log.

### Where the work stands (session of 2026-11-14)

Green and pushed: Part 1 (1.1-1.4), Part 2 items 2.1 and 2.3. `uv run pytest -q` is **687 passed,
1 skipped, 0 failed** and `scripts/replay.py --fake` is **158/158** at commit `b47d615`. The
baseline's one red test is fixed.

Not started, in the order the plan asks for them, with what each one meets:

- **2.2** the remaining payload reads in the core are `context/parts.py` (rendering),
  `context/working.py` (chain lines), `expectations.py:269` (`reminded`/`reminder`), and
  `identity.py:97`, plus the label reads in `jobs.py:187` and `dispatch.py`/`threads.py`
  (`audience`, `hold_for`, `delivers`, `words_hidden`, `place_kind`). The label half cannot be
  done inside Part 2's May-touch: the middleware must return `Replace`/`Stop(held=True)` first
  (3.2, 3.3, 3.5). Do the rendering half (parts/working/expectations/identity/recall) first; it
  needs `_emit(..., description=...)` at every core site that carries words today.
- **2.4** reads `text_like` in `expectations.matches_cue`, `is_met` and `validate`, and the tests
  that assert it are **not all in Part 2's scope**: `tests/core/test_expectations.py`,
  `test_gate.py`, `test_reasoning.py` are, but `tests/test_mvp.py:235` and
  `tests/test_replay_scenarios.py:221-274` are not. Either widen 2.4's May-touch to those two
  files by a line in this task file, or land 2.4 together with 6.1.
- **2.5** changes the type the context hook receives; the middleware's context rules
  (`privacy.py` 44 tests) parse the rendered lines, so 2.5 and 3.3 are one change, not two.
- **2.6-2.8**: 2.6 needs `registry.act` to answer with a receipt (3.4) and the eight sequence
  tests in `tests/core/test_reasoning.py` rewritten to the universal `act.result`.

The audit items carried by Parts 2-5 (S.6, E.1-E.4, F.2-F.5, F.9-F.12, O.1-O.6) are untouched.
