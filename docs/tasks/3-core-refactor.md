# Task 3 — Core: clean refactor

Status: todo | Depends on: Task 2
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** The same behavior, with everything in its right place. The core keeps only mechanisms;
every decision moves to the middleware, and the core knows only opaque ids, labels and numbers
(D57). **The middleware decides, the core records and carries out**
([constitution.md](../constitution.md) §4). Names become standard engineering terms
([glossary](../glossary.md)).

**How.** Code is moved, not rewritten and not removed. Before moving or renaming anything, use
graphify to find every caller and dependency; use Context7 for any library API in doubt. Each
part ends with `uv run pytest` and the fake replay passing with the same counts as at the start
of the task (write the starting counts under Notes first). Only then start the next part.

**From V1.** Branch `archived` holds V1. Read an ADR with
`git show origin/archived:docs/archive/adr-pre-rebaseline/<path>.md`. Take the mechanism; leave
its roles, approvals, flows and multi-runner leases.

---

## Part 1 — Layout and names
Read first: [glossary](../glossary.md), [AGENTS.md](../../AGENTS.md) (size guide)
May touch: `monoagens/` (all), `tests/`, `scripts/`, `pyproject.toml`, `AGENTS.md` (invariant 1's file list only)

- [x] 1.1 Packages per layer as in the glossary: `core/`, `middleware/`, `adapters/` (from `bodies/`), `infra/`; `plugins/` stays at the top. Tests mirror the same layout.
- [x] 1.2 Standard names (glossary): `pfc` → reasoning, `desk` and `work` → context, `cognition` → gate, `guidance` → principles. `organs` splits in two: the door (check the arguments against the descriptor, record, hand to the action hook, record the result) stays in the core as `dispatch`; the registry of tools present, which tools a turn is offered, and routing a call to the adapter or plugin that owns it go to `middleware/tools.py`. Near-duplicates merge into one module each: `appraisal`/`appraise`, `memory`/`recall`/`forget` (one `memory/` package), `expect`/`condition`. Event kinds, payload fields, `soul.yaml`, `prior/` and `config.yaml` keys keep their names (glossary, "What never changes").
- [x] 1.3 Check: pytest and the fake replay pass unchanged. List every old → new path and every renamed public name under Notes (the specs already use the standard terms; the architect updates any file path they name).

## Part 2 — Decisions out of the core
Read first: [constitution.md](../constitution.md) §4, [core.md](../core.md) §8 (D57), [middleware.md](../middleware.md) §3, §7
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
Read first: [middleware.md](../middleware.md), [core.md](../core.md) §3
May touch: `monoagens/core/`, `monoagens/middleware/`, `monoagens/infra/`, `tests/`, `scenarios/`

- [ ] 3.1 Fail-closed: the kernel refuses to start without the owner policy in all three hooks. The owner policy runs first and last in every hook, no plugin can remove it, and a plugin can only narrow what it allows. Tests for each.
- [ ] 3.2 Context parts (V1 `engine/0034-prompt-parts`): every block of the context is declared once with its order and whether it is stable (cached prefix) or volatile; middleware and plugins add blocks through the context hook without editing the core. Test: the stable prefix hash is the same before and after this item for the same soul.
- [ ] 3.3 Send ledger (V1 `engine/0011-crash-safety`): an outgoing act is recorded before it is sent and a confirming event is appended when the adapter confirms it (events only: no new table, no new book, complexity budget §3), so a restart never sends the same message twice and never loses one silently. Test: a crash between record and confirm resends once, after the restart.

## Part 4 — Real-model parity
- [ ] 4.1 `uv run pytest` and the fake replay pass in full.
- [ ] 4.2 One real-model run at `--effort high` of the launch set of Task 2 (12.8) and a short live CLI session (the model in `.env`; never read or print it). Every scenario that passed at the end of Task 2 still passes; absolute-line violations 0. Scenarios 20 and 22 run with the real model here for the first time; 16 and 30 are flaky (Task 2 closing note). Regenerate and commit the capability record. A scenario that now fails is a refactor bug: find the move that caused it and fix it there.

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
