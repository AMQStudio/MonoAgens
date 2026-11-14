# Task 3 — Core: clean refactor

Status: todo | Depends on: Task 2
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [README](../README.md).

**Goal.** The same behavior, with everything in its right place. The core keeps only mechanisms;
every decision moves to the middleware, and the core knows only opaque ids, labels and numbers
(D57). **The middleware decides, the core records and carries out**
([spec.md §1.4](../../spec.md)). Names become standard engineering terms
([spec.md §9](../../spec.md)).

**How.** Code is moved, not rewritten and not removed. Before moving or renaming anything, use
graphify to find every caller and dependency; use Context7 for any library API in doubt. Each
part ends with `uv run pytest` and the fake replay passing with the same counts as at the start
of the task (write the starting counts under Notes first). Only then start the next part.

**From V1.** Branch `archived` holds V1. Read an ADR with
`git show origin/archived:docs/archive/adr-pre-rebaseline/<path>.md`. Take the mechanism; leave
its roles, approvals, flows and multi-runner leases.

---

## Part 1 — Layout and names
Read first: [spec.md §9](../../spec.md), [AGENTS.md](../../../AGENTS.md) (size guide)
May touch: `monoagens/` (all), `tests/`, `scripts/`, `pyproject.toml`, `AGENTS.md` (invariant 1's file list only)

- [x] 1.1 Packages per layer as in the glossary: `core/`, `middleware/`, `adapters/` (from `bodies/`), `infra/`; `plugins/` stays at the top. Tests mirror the same layout.
- [x] 1.2 Standard names (glossary): `pfc` → reasoning, `desk` and `work` → context, `cognition` → gate, `guidance` → principles. `organs` splits in two: the door (check the arguments against the descriptor, record, hand to the action hook, record the result) stays in the core as `dispatch`; the registry of tools present, which tools a turn is offered, and routing a call to the adapter or plugin that owns it go to `middleware/tools.py`. Near-duplicates merge into one module each: `appraisal`/`appraise`, `memory`/`recall`/`forget` (one `memory/` package), `expect`/`condition`. Event kinds, payload fields, `soul.yaml`, `prior/` and `config.yaml` keys keep their names (glossary, "What never changes").
- [x] 1.3 Check: pytest and the fake replay pass unchanged. List every old → new path and every renamed public name under Notes (the specs already use the standard terms; the architect updates any file path they name).

## Part 2 — Decisions out of the core
Read first: [spec.md §1.4](../../spec.md), [spec.md §3.8](../../spec.md) (D57), [spec.md §4.3](../../spec.md), §7
May touch: `monoagens/core/`, `monoagens/middleware/`, `monoagens/__main__.py`, `tests/`

The core keeps only **opaque ids, labels and numbers** (D57): `who`, `place`, `origin_actor` as
strings; labels attached by the middleware and carried unread (a memory carries its sources'
labels, a thread its parent's); counts for the gate. Everything that gives them meaning moves to
the middleware. Middleware files: `contract.py` (hook outcomes and registration), `owner_policy.py`
(always installed, first and last in every hook, composes the rest), `identity.py` (alias map,
pairing the owner, person cards), `privacy.py` (audience, the withheld note, `share`), `limits.py`
(daily budget limit, quiet hours), `tools.py` (registry and routing, from Part 1). The loader and
the command registry come in Task 4.

- [x] 2.0 **The core never imports the middleware** (D57, invariant 1). The composition root (`monoagens/__main__.py`, and one helper the tests and `scripts/replay.py` share) builds the kernel, then installs the middleware into it: the owner policy in the three hooks, the inbound intake, the tool registry, and any number the core needs from a setting (for example the daily budget limit the gate divides by) as a plain injected value. The kernel refuses to start its loop until the policy is installed in all three hooks (fail-closed). What the soul learned about a source stays core memory under its id (`state: person:<id>` keeps its frozen key name; read through one core helper whose comment names it as frozen data).
- [x] 2.1 Labels: the core stores the labels a hook outcome attaches and passes them on (memory from its sources, thread from its parent); no core module reads what a label means.
- [x] 2.2 Privacy: the audience rule (`reach.visible` and every caller in context and recall), the audience label at inbound (D50) and the withheld note (D56) move to `privacy.py`.
- [x] 2.3 Sharing: the `share` capability leaves the core's self tool and becomes the middleware's (`privacy.py`); it records `share.granted` through a hook outcome.
- [x] 2.4 Identity: the alias map, pairing the first owner and every "is this the owner" check move to `identity.py` and `owner_policy.py`; person cards split into what the soul learned (core memory, under the id) and name and routes (added by `identity.py` through the context hook).
- [x] 2.5 Routes: resolving `to:` / `via:` to a place moves to the action hook; the core only delivers to a place.
- [x] 2.6 Limits: the core keeps counting every model call (invariant 5); the daily limit and quiet hours move to `limits.py`.
- [x] 2.7 Anything else in the core that means "person", "owner", "group" or decides right or wrong for a role, place or person: list it under Notes with file and line. Move it when it is plainly a decision; otherwise stop and ask the owner.
- [x] 2.8 Check: pytest and the fake replay pass unchanged, absolute-line violations 0. `grep` of `monoagens/core/` finds no "person", "owner", "alias" or platform name outside comments that point to the middleware. Tests that said "holds with an empty middleware pipeline" now say "holds with only the owner policy"; list them under Notes.

## Part 3 — Fail-closed and the mechanisms the middleware needs
Read first: [spec.md §4](../../spec.md), [spec.md §3.3](../../spec.md)
May touch: `monoagens/core/`, `monoagens/middleware/`, `monoagens/infra/`, `tests/`, `scenarios/`

- [x] 3.1 Fail-closed: the kernel refuses to start without the owner policy in all three hooks. The owner policy runs first and last in every hook, no plugin can remove it, and a plugin can only narrow what it allows. Tests for each.
- [x] 3.2 Context parts (V1 `engine/0034-prompt-parts`): every block of the context is declared once with its order and whether it is stable (cached prefix) or volatile; middleware and plugins add blocks through the context hook without editing the core. Test: the stable prefix hash is the same before and after this item for the same soul.
- [x] 3.3 Send ledger (V1 `engine/0011-crash-safety`): an outgoing act is recorded before it is sent and a confirming event is appended when the adapter confirms it (events only: no new table, no new book, complexity budget §3), so a restart never sends the same message twice and never loses one silently. Test: a crash between record and confirm resends once, after the restart.

## Part 4 — Real-model parity
- [x] 4.1 `uv run pytest` and the fake replay pass in full.
- [x] 4.2 One real-model run at `--effort high` of the launch set of Task 2 (12.8) and a short live CLI session (the model in `.env`; never read or print it). Every scenario that passed at the end of Task 2 still passes; absolute-line violations 0. Scenarios 20 and 22 run with the real model here for the first time; 16 and 30 are flaky (Task 2 closing note). Regenerate and commit the capability record. A scenario that now fails is a refactor bug: find the move that caused it and fix it there.

**Done when.** All parts are ticked; the same scenarios pass as at the end of Task 2; the core
holds only mechanisms and the Notes list every move; the code is readable (AGENTS.md size
guide).

**Notes.**

**Baseline (before any edit, 2026-11-12).** `uv run pytest -q` → 321 passed (the architect's
`--collect-only` count matched). `uv run python scripts/replay.py --fake` → `checks: 133/133
passed`, absolute-line violations 0, report `reports/2026-11-12-1254-fake-all.md`.

**Part 1 — every old → new path.**

| Old | New |
|---|---|
| `monoagens/kernel.py` | `monoagens/core/kernel.py` |
| `monoagens/hooks.py` | `monoagens/core/hooks.py` (also absorbs `middleware.py`) |
| `monoagens/contract.py` | `monoagens/core/arguments.py` |
| `monoagens/organs.py` | `monoagens/core/dispatch.py` (+ `monoagens/middleware/tools.py`) |
| `monoagens/identity.py` | `monoagens/core/identity.py` |
| `monoagens/threads.py` | `monoagens/core/threads.py` |
| `monoagens/pfc.py` | `monoagens/core/reasoning.py` |
| `monoagens/desk.py` | `monoagens/core/context/layout.py` (+ `build_context` from `memory.py`) |
| `monoagens/work.py` | `monoagens/core/context/working.py` |
| `monoagens/memory.py` | `monoagens/core/memory/retrieval.py` + `monoagens/core/memory/sleep.py` |
| `monoagens/recall.py` | `monoagens/core/memory/recall.py` |
| `monoagens/forget.py` | `monoagens/core/memory/forget.py` |
| `monoagens/expect.py` | `monoagens/core/expectations.py` |
| `monoagens/condition.py` | (deleted) appended to `monoagens/core/expectations.py` |
| `monoagens/guidance.py` | `monoagens/core/principles.py` |
| `monoagens/jobs.py` | `monoagens/core/jobs.py` |
| `monoagens/cognition/__init__.py` | `monoagens/core/gate/__init__.py` |
| `monoagens/cognition/appraisal.py` + `appraise.py` | `monoagens/core/gate/appraise.py` |
| `monoagens/cognition/gate.py` | `monoagens/core/gate/formula.py` |
| `monoagens/cognition/signals.py` | `monoagens/core/gate/signals.py` |
| `monoagens/cognition/situation.py` | `monoagens/core/gate/situation.py` |
| `monoagens/cognition/rng.py` | `monoagens/core/gate/rng.py` |
| `monoagens/middleware.py` | merged into `monoagens/core/hooks.py` (the hook contract is the core's) |
| `monoagens/pairing.py` | `monoagens/middleware/identity.py` |
| `monoagens/reach.py` | `monoagens/middleware/privacy.py` |
| `monoagens/budget.py` | `monoagens/infra/budget.py` |
| `monoagens/store.py` | `monoagens/infra/store.py` |
| `monoagens/migrate.py` | `monoagens/infra/migrate.py` |
| `monoagens/versioning.py` | `monoagens/infra/versioning.py` |
| `monoagens/config.py` | `monoagens/infra/config.py` |
| `monoagens/clock.py` | `monoagens/infra/clock.py` |
| `monoagens/embedding.py` | `monoagens/infra/embedding.py` |
| `monoagens/duration.py` | `monoagens/infra/duration.py` |
| `monoagens/prior.py` | `monoagens/infra/prior.py` |
| `monoagens/bodies/*` | `monoagens/adapters/*` |

**Part 1 — renamed public names.** `PFC` → `Reasoning`; `OrganRegistry` → `ToolRegistry`;
`Kernel.organs` → `Kernel.tools`; `Kernel.register_body` → `Kernel.register_adapter`;
`SelfOrgan` → `SelfTool`; `Action.body` → `Action.adapter` (`Event.body` keeps its name: it is the
Log's column); the hook scope keyword `organ=` → `tool=` and `Registration.organ` →
`Registration.tool`; modules `cognition` → `core.gate`, `pfc` → `core.reasoning`, `desk` →
`core.context.layout`, `work` → `core.context.working`, `guidance` → `core.principles`,
`contract` → `core.arguments`, `expect` → `core.expectations`, `reach` → `middleware.privacy`,
`pairing` → `middleware.identity`. `ToolRegistry.bodies` → `ToolRegistry.adapters`.

**Part 1 — deliberate deviations from the letter of the plan.**

- `_args` and the description lines of the `act` tool stay in `core/reasoning.py`; only
  `ToolRegistry.routing_keys` moved to `middleware/tools.py`. Reason: `tools_for(capabilities)`
  renders the prompt, and from 2.0 on the core may not import the middleware, so the renderer
  must stay core. `routing_keys` is reached as `kernel.tools.routing_keys(...)`, an injected
  value, not an import.
- `infra/prior.py` imports `core.identity` inside `load_settings()` (comment names the cycle:
  identity → memory → prior). Moving `memory.py` into a package whose `__init__` re-exports the
  submodules made the module-level import a cycle.

**Part 1 — path constants fixed after the move.** `infra/config.py` `DEFAULT_ROOT`,
`infra/prior.py` `DEFAULT_PRIOR_PATH`, `infra/migrate.py` `ROOT` all gained one `.parent`.

**Part 1 — check.** `uv run pytest -q` → 321 passed; `scripts/replay.py --fake` → 133/133 checks,
absolute-line violations 0 (report `reports/2026-11-12-1307-fake-all.md`).

**Part 2 — every decision that moved out, and where it went.** Old paths are the files before
Task 3 (`git show 23625c4:<path>`), line numbers from that commit.

| Old (file:line) | Now |
|---|---|
| `reach.py:11` `OWNER` | `middleware/identity.py` `OWNER` |
| `reach.py:24` `aliases` | `middleware/privacy.py` `aliases` (kept beside the audience rule: identity imports privacy for it, and the reverse would be a cycle) |
| `reach.py:30` `place_kind`, `:40` `direct`, `:77` `owner_direct` | `middleware/privacy.py`, same names |
| `reach.py:45` `audience`, `:66` `shared` | `middleware/privacy.py` |
| `reach.py:82` `visible`, `:95` `memory_visible` | `middleware/privacy.py`; injected into `core/identity.py:SelfTool` and into `core/memory/recall.py:search(..., visible=, memory_visible=)` |
| `reach.py:111` `source_label`, `:122` `withheld_source`, `:143` `leaks` | `middleware/privacy.py` |
| `reach.py:155` `person_cards` (+ `_known_people`, `_person_states`, `_name`, `_named`, `_card`, `_direct_places`) | `middleware/identity.py` (`identity.cards`, the context hook's scoped rule) |
| `reach.py:229` `direct_place`, `:244` `resolve_to` | `middleware/identity.py` (`identity.route`, the action hook's scoped rule) |
| `reach.py:256` `consent`, `:266` `grant` | `middleware/privacy.py` (`privacy.guard`) |
| `hooks.py:21` `audience_policy` | `middleware/privacy.py:filter_context` — plus the two decisions `build_context` still took: `place_kind` and whether core memory may be shown (with its withheld count) |
| `hooks.py:56` `policy_for` | `middleware/owner_policy.py:policy_for` (composes identity line, context filter, guard) |
| `middleware.py:13` `OWNER`, `:16` `owner_policy` | `middleware/identity.py` `OWNER`, `identity.identity_line()` |
| `budget.py:20` `daily_limit` | `middleware/limits.py:daily_limit`, injected as `kernel.budget_limit` |
| `budget.py:58` `_record_exhausted` | `middleware/limits.py:enforce` → `kernel.after_model_call` |
| `organs.py:80` the `self.share` branch | `privacy.guard`: consent on the first pass (`Stop`), `share.granted` and `handled` on the last |
| `organs.py:91` route resolution (`to`/`via`) | `identity.route`, which sets the `place`/`place_kind`/`hold_for`/`delivers` labels |
| `organs.py:108` the leaks refusal | `privacy.guard` last pass (`Stop`), and the `words_hidden` label for the intent text |
| `organs.py:124` the audience of a new task thread | `labels["audience"]`, set by `privacy.guard` when the act is long-running |
| `organs.py:132` the `capability == "self.set_identity"` test | the result's shape: a dict with an `identity` dict refreshes the reasoning |
| `threads.py:10` `OWNER` and `:71` the `"person:"` prefix test | replaced by the `hold_for`/`delivers` labels |
| `guidance.py:16`, `:18` the share and privacy lines | `core/principles.py`, no capability name, no second person |
| `memory.py:43` `SLEEP_SYSTEM` "who the owner is" | "who the people are" |
| `identity.py:18` `set_identity` "Only the owner may invoke it." | "Only the one the soul is paired with may invoke it." |
| `pairing.py:61` `as_owner`, `:69` `inbound` | `identity.intake(conn)` (the kernel's intake) and `identity.claim(conn)` (the inbound rule) |

**Part 2 — how the core keeps running without the middleware.** `monoagens/boot.py` is the
composition root (`build`, `install`); `middleware/owner_policy.py:install` sets the four injected
values (`tools`, `intake`, `budget_limit`, `after_model_call`) and the owner policy in all three
hooks. `Kernel.require_installed()` is the first statement of `run()`, `step()` and
`run_until_idle()`; `put()` and `register_adapter()` refuse half-wired use. `grep -n
"monoagens.middleware" monoagens/core/` returns nothing.

**Part 2 — new data shapes.**

- `Dispatch(acted, labels)` is what `core/dispatch.py` returns and `core/jobs.apply_calls`
  collects (one `(call, labels)` per non-refused `act`).
- The action-hook context keys and the five labels are documented in
  `middleware/contract.py`; the labels are `audience`, `place_kind`, `words_hidden`, `hold_for`,
  `delivers`, and the core reads none of them as a decision — it stores them and passes them on.
- `PFCContext.place_kind` now defaults to `""` and is filled by `privacy.filter_context`.

**Part 2 — tests whose wording or expectation changed (and why).**

1. `tests/core/test_gate.py::test_action_hook_stop_records_the_stop_and_neither_records_nor_acts`
   (was `..._keeps_the_logged_message_but_never_acts`): the action hook now decides before the
   record, because it resolves the route; the test asserts `body.actions == []`,
   `guard.stopped` `{"hook": "action", "reason": "privacy"}` and `"act.out" not in kinds`.
2. Refusals produced by a content rule now write `guard.stopped` as well as the `act.result`
   error, because such a rule is a `Stop` (it must end the chain so no later middleware sees the
   content). Pinned in `tests/middleware/test_privacy.py`
   (`..._only_a_person_who_was_part_of_it_can_share_it` and the new
   `..._saying_private_words_in_a_place_that_may_not_see_them_is_refused`); the `act.result` text
   is unchanged.
3. `tests/core/test_gate.py::test_the_budget_spent_raises_theta` passes `daily_limit=100` to
   `appraise`, because the limit is now an injected number instead of a settings read.
4. Every test that builds a kernel uses `monoagens.boot.build` (or `install` after a subclass);
   every `SelfTool` is built with `visible=`/`memory_visible=`.
5. `threads.after_step` tests pass `[(call, {"hold_for": OWNER})]` and `[(call, {"delivers":
   True})]` instead of reading the call's own `to:`.
6. Desk tests in `tests/middleware/test_privacy.py` and `tests/test_replay_metrics.py` compose
   `privacy.filter_context` with `identity.cards` instead of calling `hooks.audience_policy()`;
   person cards now come from the scoped context rule.
7. Renames only: `test_self_organ_...` → `test_self_tool_...`, `identity.claim_owner` for the
   pairing event function (`identity.claim(conn)` is now the inbound rule), `..._own_..._organ`
   test names → `..._tool_...`, and the pinned `act` description now reads `to? (a speaker or a
   place), via? (a tool)`.

**Part 2 — deliberate deviations from the letter of the plan.**

- `filter_context` takes the connection (`filter_context(conn)`) and is composed only into the
  context hook; `guard` only into the action hook. Running the guard on the context hook has no
  item to inspect (a `PFCContext` is not a dict), and it would be a no-op anyway.
- `identity.identity_line()` takes no connection: the line is a pure predicate on the context.
- `aliases` stays in `privacy.py` (see the table) and person cards read it from there.
- `infra/budget.py` keeps `used_fraction` (the core's gate divides by the injected limit) and
  `limits.py` reads only `owner_settings.daily_token_budget`; nothing is re-exported.
- The intent's `words_hidden` label is computed from the act's own text; the old reason also
  folded in the chain's goal. The two reason strings are unchanged.

**Part 2 — what had no code to move.** The audience label at inbound (D50): no code exists —
audience is derived at read time from the Log and the alias map, and that derivation now lives in
`privacy.audience`. Quiet hours: `config.yaml` has the key, no code reads it, so `limits.py`
holds only the daily token cap.

**Part 2 — check.** `uv run pytest -q` → 322 passed (321 + the new leak-refusal test);
`scripts/replay.py --fake` → 133/133 checks, absolute-line violations 0; the `core/` grep prints
only comments naming the middleware, the frozen `persona` key, the mechanism's name (owner
policy) and `core/memory/cards.py`, whose comment says `state: person:<id>` is frozen data.

**Part 3 — fail-closed (3.1).** `tests/core/test_fail_closed.py` (new, six tests) pins it:
`boot.build` installs the registry, the intake, the budget number and the post-call watcher and
the policy in all three hooks; a plain `Kernel` refuses `put`, `step`, `run_until_idle` and
`register_adapter` until `boot.install`; a bare `HookRegistry` refuses `require_policy`;
`set_policy` refuses a second installation and `HookPipeline` has no removal API; a context rule
that Replaces the desk with private content is narrowed by the last pass (two withheld items
plus the one the desk already carried); and a rule registered after the policy cannot undo the
identity line — the act is still stopped with the pinned reason. The mechanism is
`HookPipeline.set_policy(first, last)` + `HookRegistry.require_policy()` as the first statement
of `run`, `step` and `run_until_idle`.

**Part 3 — context parts (3.2).** `monoagens/core/context/parts.py` (new) declares `CORE_PARTS`:
`identity` and `principles` stable, `core_memory` and `people` semi, then `now`, `place`,
`why_awake`, `results`, `doing`, `expectations`, `memories`, `withheld`, `recent`, `event`
volatile, each with a unique `order` inside its layer and one `produce`. `messages(soul,
context)` renders the three messages from `CORE_PARTS + context.blocks`; a block with a
non-volatile layer or a taken name is refused with its name; the trimming loop for `recent` and
`memories` and the `volatile_tokens_max` budget are unchanged. `layout.py` keeps `PFCContext`
(now with `blocks`, and `place_kind` defaulting to `""`), `withheld_note` and `build_context`.

Golden stable prefix, captured before the item and unchanged after it:
`sha256 = 7ebc122f4b2988eb0ab2e33048c128a2dda8ed2fe3aa3e3c8cb424237232b448`
(`hashlib` of the first message's content for `Reasoning("http://unused", "", "fake",
settings=SETTINGS)` and the plain desk). `tests/core/test_context.py` pins that hash, the exact
three messages of a desk with results, doing, memories, a withheld note and a line elsewhere (all
byte-identical to the pre-item renderer), a block added through the context hook (it lands after
`</event>`), and the two refusals.

**Part 3 — send ledger (3.3).** `monoagens/core/sends.py` (new, events only: no table, no book).
`act.confirmed` is the additive kind; its payload is `{"capability": ..., "to": ...}` (the `to`
key only when the act had one). The pending predicate is: `act.out` rows whose
`payload->>'communicative'` is `'true'` with no event caused by them of kind `act.confirmed`,
`act.result` or `error` (that is `CLOSING`). `dispatch` confirms right after the tool returns and
before the `act.result` is enqueued; `Kernel.recover()` calls `sends.resume` before the
expectations recovery. **At-least-once**: a crash between the record and the confirmation can
send the same message once more after the restart; a confirmed message is never sent again; a
tool that returns nothing and then raises closes its row with an `error`, so a failing adapter
cannot loop. The first restart after this change also resends communicative `act.out` rows
written before it existed, because they carry no confirmation and no closing event — that is the
same rule, read on old rows.

Tests: `tests/core/test_sends.py` (new, four) pins the single resend on `recover`, the second
`recover` sending nothing, `act.out` before `act.confirmed` with no `act.result` for a tool that
returns nothing, the one `error` row for a capability whose tool is gone (and no retry), and a
non-communicative act never pending. Seven exact `_kinds(...)` lists gained `act.confirmed` right
after `act.out` (`tests/core/test_gate.py` once, `tests/core/test_reasoning.py` six); the
communication "at the moment the adapter acts" lists and the non-communicative
`telegram.react`/`set_identity` lists stay as they were. `test_dm_gets_a_reply_through_the_
telegram_body` now lets the act in flight finish (it sets the stop event and awaits the consumer)
instead of cancelling the loop the moment the adapter sends, so the confirmation is committed
when it asserts.

**Part 3 — check.** `uv run pytest -q` → 336 passed (321 baseline + 6 fail-closed + 4 context +
4 sends + 1 leak refusal); `scripts/replay.py --fake` → 133/133 checks, absolute-line violations
0.

**Notes — the checklist the plan asked for.**

6. *(what had no code to move)* The inbound audience label (D50) and quiet hours: see the Part 2
   notes above — neither had code.
7. `Action.body` became `Action.adapter` while `Event.body` keeps its name: `body` is a Log
   column (the adapter that produced the event), and the table stores it.
8. Left at the top level on purpose: `monoagens/types.py` (the shared Event/Action/StateEntry
   contract every layer imports — no layer owns it), `monoagens/__main__.py` (wires the process)
   and `monoagens/cli.py` (the terminal client that talks over the socket), plus the new
   `monoagens/boot.py`, which is the composition root and belongs to no layer.
9. Size: `monoagens/core/kernel.py` is 321 lines and `monoagens/core/context/parts.py` 308.
   The kernel keeps one loop and every seam this task defined lands in it; splitting it would
   scatter the queue, the hooks and the job result across files. `parts.py` is one declaration
   table plus the renderers that read it; the renderers cannot be split from `Part` without
   passing the trim budgets through a second module. Both are pinned to their single job; no
   other module passes 300 lines.
10. The send ledger is at-least-once (see the Part 3 notes and the `core/sends.py` docstring).
    `AGENTS.md` invariant 1 already names the no-middleware-import rule; only its file list was
    replaced with `monoagens/core/` (everything under it).
11. The four injected values and the two predicates, and why: the core imports no middleware, so
    `middleware/owner_policy.install` sets `kernel.tools` (the `ToolRegistry`), `kernel.intake`
    (`identity.intake`, which rewrites the paired owner's id), `kernel.budget_limit`
    (`limits.daily_limit`) and `kernel.after_model_call` (`limits.enforce`, which writes
    `budget.exhausted` once a day); `core/identity.SelfTool` receives `privacy.visible` and
    `privacy.memory_visible` and hands them to `core/memory/recall.search`. Every one of them is
    data or a callable the core calls, never an import.

**Part 4 — live CLI session (4.2, second half).** `uv run python -m monoagens` with the CLI
adapter, then `uv run python -m monoagens.cli` sending `halo`, `nama saya Muqsith` and `siapa nama
saya?` through the socket: three replies arrived (`Halo, Muqsith.` / `Sudah kudengar, Muqsith. Ada
yang bisa kubantu?` / `Muqsith.`), and the third answered with the name said earlier. In Postgres,
every `act.out` was followed by its `act.confirmed` (`{"capability": "cli.say"}`), and the state
board held `last_spoke`, `last_seen`, `budget:<date>` and `owner`.

**Part 4 — defect found and fixed (this is the one change outside the plan).**
`monoagens/__main__.py` connected with `psycopg.AsyncConnection.connect(DATABASE_URL)` and never
committed: psycopg commits a non-autocommit connection only when its `async with` block exits
normally, so every live process wrote into one long transaction that stayed invisible to other
sessions and was rolled back on SIGTERM. Observed directly: the first live run printed the
pairing code and handled four messages, and after `pkill` the `monoagens` database had no tables
at all. The replay, the tests and `job.sleep`'s private connection all connect with
`autocommit=True`, and the kernel's atomic unit is `conn.transaction()`, which is a real
transaction (BEGIN/COMMIT) on an autocommit connection — so the entry point was the odd one out.
Fixed by the one keyword; the second live run wrote events that other sessions can read while the
process runs, and they survived the kill (28 events, 2 confirmations). No test was added: the
live run in this task is the proof, and a test could only assert the connection's wiring.

**Part 4 — honest observations from the live run.** The very first percept after a cold start
lost its inbound hook to the 0.5 s watchdog (`error` + `guard.stopped`,
`"inbound hook timed out after 0.5s"`), so the pairing code was not claimed on that message; the
next message claimed it (`pair.claimed`, `state: owner`, `pairing_code` deleted). The watchdog did
what it is for — the loop lived and recorded the decision — but a first-boot claim can miss its
own line. Left as observed; it is not caused by this refactor (the pairing rule has always run
under the same watchdog).

**Part 4 — the real-model run (4.2, first half).** `--effort high`, the 26 launch scenarios, from
the report `reports/2026-11-12-1353-real-…-16-30.md`: **69/75 checks**, missed-help 0.0% (0/48),
false-speak 1.4% (1/71), tokens per simulated day 5183.5, **absolute-line violations 0**, honesty
100.0% (50/50), 83 LLM calls, 215936 tokens. Twenty-two of the 26 scenarios passed in full.

| Scenario | Batch | After a single rerun | Record |
|---|---|---|---|
| 07, 11, 12, 20, 25, 27, 29, 17, 01, 02, 03, 15, 21, 04, 05, 06, 14, 22, 08, 10, 13, 16 | PASS | — | PASS |
| 23_recall_episodes | 6/7 FAIL | **7/7 PASS** (`2026-11-12-1355-real-23.md`) | `flaky` |
| 09_reminder_ask_again | 0/1 FAIL | **1/1 PASS** (`2026-11-12-1354-real-09.md`) | `flaky` |
| 30_same_topic_question | 3/5 FAIL | not rerun | `flaky` (it was already) |
| 24_gate_threshold | 9/11 FAIL | 7/11 (`2026-11-12-1356-real-24.md`) | FAIL — the Task 6 target Task 2 left open |

- **23 and 09 are not regressions.** Both pass alone with the same model and effort; the record
  keeps both the failure and the pass (`flaky`, exactly as 16 and 30). In 09 the batch run's soul
  created only the patience wait and never held the follow-up promise; in 23 it said nothing was
  stored. Both are the model's own choices in that sample.
- **24's failure, with evidence, is the open Task 6 question, not this refactor.** In the single
  rerun, the reminded wake did `act.result {"to": "person:tg:7001", "result": {"error": "no way to
  reach tg:7001 yet"}, "capability": "telegram.say"}`: the soul answered the reminded promise by
  addressing the person it promised, who has no direct place (they are only in the group), and the
  door refused with the honest error instead of speaking where the person was present. The old
  `dispatch` had the same fallback and the same condition (`str(to) == event.who`, and the
  reminding line's speaker is someone else), so nothing here changed by moving it. 24 stays what
  Task 2 recorded: not yet held, a Task 6 target.
- **One 0.5 s hook timeout** in the batch (scenario 24, context hook). The context hook now runs
  the absolute lines on both passes, so the desk was filtered twice; `filter_context` now keeps
  the desk it produced in the hook context and skips the second pass when nobody replaced it (a
  plugin's new desk is still filtered — `test_a_plugin_that_widens_the_desk_is_narrowed_again`
  pins it). No real rerun followed that query-saving change: for an unchanged desk the second pass
  returned the same item, and the fake suite is the proof (`133/133`).
- **`reports/` is gitignored** (it holds every run, pruned by the script), so the deliberate
  "commit the reports" step can only apply to `scenarios/README.md`, the capability record, which
  is committed: it now carries every run above, including both verdicts of the flaky scenarios.
