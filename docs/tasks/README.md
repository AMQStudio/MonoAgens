# Tasks

Work for coding workers. **One file = one task = one coordinator prompt.** Each task is split
into parts and items, numbered in the order they are done (`3.2` = part 3, item 2). The
coordinator works through the parts in order, one commit per item when practical, and ends with
the real-model test in the last part.

| Task | File | Status | Depends on |
|---|---|---|---|
| 0 | Core: align with the frozen contract (git history) | **done** — 181 tests; fake replay 17/25, fixed in Task 1 Part 2 | — |
| 1 | [Core: launch](1-core-launch.md) | todo | Task 0 |
| 2 | [Core: depth of mind](2-core-mind.md) | todo | Task 1 and some weeks of daily use |
| 3 | [Middleware and first plugins](3-middleware-plugins.md) | todo | Task 1 |
| 4 | [Bodies, first users and release](4-bodies-project.md) | todo | Task 3 |

The architect updates this table and each file's `Status:` line; workers never do.

## Launch
"Launch" means: the owner uses the soul every day in a direct chat and one group. Task 1 is the
whole path there. The first number is the **missed-help rate**: of the group messages that
should have been answered, how many passed the gate. The gate is tuned for recall, not
precision: a needless wake costs a few tokens and the PFC can still stay silent; a missed
moment is a failure.

**Deferred until weeks of real logs** (the spec stays; only the work waits):
- Extending upcasting: `monoagens/versioning.py` exists from Task 0; leave it as it is until a
  payload schema first changes.
- The full prior loader with its loading rules (core.md §9.4); the code reads
  `prior/dispositions.yaml` directly until then.
- Snapshots of `state` (core.md §3).

## Rules for every task
1. Read [AGENTS.md](../../AGENTS.md), [constitution.md](../constitution.md) and the spec sections
   each part links, before touching code.
2. In each part, touch only the files listed under **May touch**. A needed change elsewhere
   stops the work and goes to the architect.
3. **Never change a specification** (`docs/*.md`). A design question stops the work; write it
   under the task's **Notes** and report.
4. An item is done only when `uv run pytest` and the fake replay pass. Tests always use the fake
   LLM; the real model is used only in the last part, through the replay script and a live
   session.
5. Every behavior change ships with at least one replay scenario that shows it.
6. Design by brain function, not by scenario: never add a rule for one case to the core.
7. Do only what the item asks; no machinery for later.
8. Workers may tick boxes and write under **Notes**; nothing else in the task file.
