# Task 3 — Core: clean refactor

Status: todo | Depends on: Task 2
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** The same behavior, with everything in its right place. The core keeps only mechanisms;
every decision about right and wrong, about who is who and about what may be shared moves to
the middleware. **The middleware decides, the core records and carries out**
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
- [ ] 1.2 Standard names (glossary): `pfc` → reasoning, `desk` and `work` → context, `organs` → tools, `cognition` → gate, `guidance` → principles. Near-duplicates merge into one module each: `appraisal`/`appraise`, `memory`/`recall`/`forget` (one `memory/` package), `expect`/`condition`. Event kinds, payload fields, `soul.yaml`, `prior/` and `config.yaml` keys keep their names (glossary, "What never changes").
- [ ] 1.3 Check: pytest and the fake replay pass unchanged. List every old → new path and every renamed public name under Notes; the architect renames the spec terms from that list.

## Part 2 — Decisions out of the core
Read first: [core.md](../core.md) §8 (absolute lines, D45, D46, D50, D55, D56), [middleware.md](../middleware.md)
May touch: `monoagens/core/`, `monoagens/middleware/`, `monoagens/__main__.py`, `tests/`

The core keeps the **labels**: who was present when a line was said (D50), `derived_from`,
`share.granted`, `place_kind`, the person id on every event. The middleware owns every **rule**
that reads them. The owner policy is the middleware component that is always installed.

- [ ] 2.1 Privacy: the audience rule (`reach.visible` and every caller in context and recall) becomes the owner policy in the context hook. The withheld note (D56) and person cards (D55) are produced there. The core only hands the policy each item with its labels and drops what the policy drops.
- [ ] 2.2 Identity: deciding which account is which person (the alias map) and pairing the first owner run in the middleware's inbound hook, before the event is recorded; the core records the person id it is given.
- [ ] 2.3 Sharing: whether a person may share an item (each person speaks only for what they were part of, D46) is checked by the owner policy in the action hook; the core records `share.granted` when the policy allows it.
- [ ] 2.4 Budget and quiet hours: the core keeps counting every model call (invariant 5); the daily limit and quiet hours are the owner policy.
- [ ] 2.5 Anything else in the core that decides right or wrong for a role, place or person: list it under Notes with file and line. Move it only when it is plainly policy; otherwise stop and ask the owner.
- [ ] 2.6 Check: pytest and the fake replay pass unchanged, absolute-line violations 0. Tests that said "holds with an empty middleware pipeline" now say "holds with only the owner policy"; list them under Notes.

## Part 3 — Fail-closed and the mechanisms the middleware needs
Read first: [middleware.md](../middleware.md), [core.md](../core.md) §3
May touch: `monoagens/core/`, `monoagens/middleware/`, `monoagens/infra/`, `tests/`, `scenarios/`

- [ ] 3.1 Fail-closed: the kernel refuses to start without the owner policy in all three hooks. The owner policy runs first and last in every hook, no plugin can remove it, and a plugin can only narrow what it allows. Tests for each.
- [ ] 3.2 Context parts (V1 `engine/0034-prompt-parts`): every block of the context is declared once with its order and whether it is stable (cached prefix) or volatile; middleware and plugins add blocks through the context hook without editing the core. Test: the stable prefix hash is the same before and after this item for the same soul.
- [ ] 3.3 Send ledger (V1 `engine/0011-crash-safety`): an outgoing act is recorded before it is sent and a confirming event is appended when the adapter confirms it (events only: no new table, no new book, complexity budget §3), so a restart never sends the same message twice and never loses one silently. Test: a crash between record and confirm resends once, after the restart.

## Part 4 — Real-model parity
- [ ] 4.1 `uv run pytest` and the fake replay pass in full.
- [ ] 4.2 One real-model run at `--effort high` of the launch set of Task 2 (12.8) and a short live CLI session (the model in `.env`; never read or print it). Every scenario that passed at the end of Task 2 still passes; absolute-line violations 0. A scenario that now fails is a refactor bug: find the move that caused it and fix it there.

**Done when.** All parts are ticked; the same scenarios pass as at the end of Task 2; the core
holds only mechanisms and the Notes list every move; the code is readable (AGENTS.md size
guide).

**Notes.**
