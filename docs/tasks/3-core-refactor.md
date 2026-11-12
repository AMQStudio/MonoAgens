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

- [ ] 1.1 Packages per layer as in the glossary: `core/`, `middleware/`, `adapters/` (from `bodies/`), `infra/`; `plugins/` stays at the top. Tests mirror the same layout.
- [ ] 1.2 Standard names (glossary): `pfc` → reasoning, `desk` and `work` → context, `cognition` → gate, `guidance` → principles. `organs` splits in two: the door (check the arguments against the descriptor, record, hand to the action hook, record the result) stays in the core as `dispatch`; the registry of tools present, which tools a turn is offered, and routing a call to the adapter or plugin that owns it go to `middleware/tools.py`. Near-duplicates merge into one module each: `appraisal`/`appraise`, `memory`/`recall`/`forget` (one `memory/` package), `expect`/`condition`. Event kinds, payload fields, `soul.yaml`, `prior/` and `config.yaml` keys keep their names (glossary, "What never changes").
- [ ] 1.3 Check: pytest and the fake replay pass unchanged. List every old → new path and every renamed public name under Notes (the specs already use the standard terms; the architect updates any file path they name).

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

- [ ] 2.0 **The core never imports the middleware** (D57, invariant 1). The composition root (`monoagens/__main__.py`, and one helper the tests and `scripts/replay.py` share) builds the kernel, then installs the middleware into it: the owner policy in the three hooks, the inbound intake, the tool registry, and any number the core needs from a setting (for example the daily budget limit the gate divides by) as a plain injected value. The kernel refuses to start its loop until the policy is installed in all three hooks (fail-closed). What the soul learned about a source stays core memory under its id (`state: person:<id>` keeps its frozen key name; read through one core helper whose comment names it as frozen data).
- [ ] 2.1 Labels: the core stores the labels a hook outcome attaches and passes them on (memory from its sources, thread from its parent); no core module reads what a label means.
- [ ] 2.2 Privacy: the audience rule (`reach.visible` and every caller in context and recall), the audience label at inbound (D50) and the withheld note (D56) move to `privacy.py`.
- [ ] 2.3 Sharing: the `share` capability leaves the core's self tool and becomes the middleware's (`privacy.py`); it records `share.granted` through a hook outcome.
- [ ] 2.4 Identity: the alias map, pairing the first owner and every "is this the owner" check move to `identity.py` and `owner_policy.py`; person cards split into what the soul learned (core memory, under the id) and name and routes (added by `identity.py` through the context hook).
- [ ] 2.5 Routes: resolving `to:` / `via:` to a place moves to the action hook; the core only delivers to a place.
- [ ] 2.6 Limits: the core keeps counting every model call (invariant 5); the daily limit and quiet hours move to `limits.py`.
- [ ] 2.7 Anything else in the core that means "person", "owner", "group" or decides right or wrong for a role, place or person: list it under Notes with file and line. Move it when it is plainly a decision; otherwise stop and ask the owner.
- [ ] 2.8 Check: pytest and the fake replay pass unchanged, absolute-line violations 0. `grep` of `monoagens/core/` finds no "person", "owner", "alias" or platform name outside comments that point to the middleware. Tests that said "holds with an empty middleware pipeline" now say "holds with only the owner policy"; list them under Notes.

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
