# Tasks

Work for coding workers. **One file = one work package.** The number gives the order; the wave
says what may run in parallel. A worker takes the next file whose dependencies are done, works
through its checklist, ticks it, and commits.

## Rules for every task
1. Read [AGENTS.md](../../AGENTS.md), [constitution.md](../constitution.md) and the spec sections
   the task links, before touching code.
2. Touch only the files listed under **May touch**. A needed change elsewhere stops the work and
   goes to the architect.
3. **Never change a specification** (`docs/*.md`). The core is frozen. A design question stops
   the work; write it under **Notes** and report.
4. One sub-task = one commit when practical. A task is done only when its **Done when** checks
   pass **and** every earlier test and the replay suite still pass.
5. Every behavior change ships with at least one replay scenario that shows it.
6. Design by brain function, not by scenario: never add a rule for one case to the core.
7. Workers may write findings under **Notes** and tick boxes; nothing else in the task file.

Checklist syntax (as in Spec Kit / Kiro): `- [ ] T1 description` — `[P]` marks a sub-task that
may run in parallel with the others of the same task; `- [x]` when done.

## Order and waves
| Wave | Tasks | Runs |
|---|---|---|
| 1 | [001](001-core-align.md) | alone; touches shared files |
| 2a | [002](002-core-activation.md) · [003](003-core-signals.md) · [005](005-core-turn-taking.md) · [010](010-core-desk.md) | in parallel |
| 2b | [004](004-core-gate.md) (after 003) · [006](006-core-inner-thought.md) (after 010) · [007](007-core-beliefs.md) (after 002) · [008](008-core-wake-gate.md) (after 005) | in parallel |
| 2c | [009](009-core-trust-forgetting.md) (after 007) | |
| 2 (all) | [011](011-core-replay.md) | alongside every wave-2 task |
| 3 | [012](012-middleware-foundation.md), then [013](013-plugins-modes-memory-view.md) | in order |
| 4 | [014](014-project-first-users.md) | |
| 5 | [015](015-project-evaluation.md) | |
| 6 | [016](016-plugins-tools.md) | |
| 7 | [017](017-birth-prior.md) | |
| 8 | [018](018-bodies-more.md) | |
| 9 | [019](019-plugins-developer-contracts.md) | after contracts have been stable a while |

## Template
```markdown
# NNN — Layer: title
Status: todo | Wave: N | Depends on: NNN
Read first: links to spec sections
May touch: files

## Goal
## Tasks
- [ ] T1 …
## Done when
## Notes
```
