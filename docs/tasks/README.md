# Tasks

Work for coding workers, in four files, one per stage. Each file holds several tasks; each task
is one section (`## NNN — …`) with its own status, checklist and notes. Task numbers never
change, so `020` means the same task everywhere. A worker is given one task (or one sub-task)
and works only inside that section.

| File | Stage |
|---|---|
| [1-core-launch.md](1-core-launch.md) | everything needed until the owner uses the soul every day |
| [2-core-mind.md](2-core-mind.md) | depth of mind after launch |
| [3-middleware-plugins.md](3-middleware-plugins.md) | the foundation and the first apps |
| [4-bodies-project.md](4-bodies-project.md) | more bodies, first users, release, evaluation |

## Status
The architect updates this table and each section's `Status:` line; workers never do.

| Task | Title | File | Status | Depends on |
|---|---|---|---|---|
| 001 | Core: align with the frozen contract | (git history) | **done** — replay 17/25; the 8 failing scenarios are fixed in 011 T0 | — |
| 021 | Core: follow-ups from 001 | 1 | todo | 001 |
| 020 | Core: privacy follows people, sharing, reaching a person, task threads | 1 | todo | 021 |
| 011 | Core: replay scenarios and metrics | 1 | todo | 001 (runs alongside every core task) |
| 002 | Core: ACT-R activation | 1 | todo | 020 |
| 003 | Core: the five signals and habituation | 1 | todo | 002 (T3), 020 |
| 005 | Core: turn-taking and patience | 1 | todo | 020 |
| 010 | Core: the desk and recall | 1 | todo | 020 |
| 004 | Core: threshold, gate and circuit breaker | 1 | todo | 003 |
| 006 | Core: inner thought and self-model | 2 | todo | 010 |
| 007 | Core: revise, beliefs, lessons and procedures | 2 | todo | 002 |
| 008 | Core: wake gate and free-text conditions | 2 | todo | 005 |
| 009 | Core: trust, fading and forgetting | 2 | todo | 007 |
| 017 | Core: born from a prior | 2 | deferred | 001 |
| 012 | Middleware: the foundation | 3 | todo | launch |
| 013 | Plugins: modes example and memory-view | 3 | todo | 012 |
| 016 | Plugins: first tool organs | 3 | todo | 013 |
| 019 | Plugins: stable contracts for developers | 3 | todo | 018 |
| 018 | Bodies: identity linking, more bodies, more models | 4 | todo | 012 |
| 014 | Project: first users and the release gate | 4 | todo | 013 |
| 015 | Project: honest human-likeness evaluation | 4 | todo | 014 |

**Order.** 021 → 020 alone (they touch shared files). Then 002, 005, 010 in parallel, 003 after
002, 004 after 003; 011 runs alongside all of them. Stage 2 after launch; stages 3 and 4 after
that.

## Launch path
"Launch" means: the owner uses the soul every day in a direct chat and one group. Everything in
[1-core-launch.md](1-core-launch.md) is on that path. The first number is the **missed-help
rate** (011): of the group messages that should have been answered, how many passed the gate.
The gate is tuned for recall, not precision: a needless wake costs a few tokens and the PFC can
still stay silent; a missed moment is a failure.

**Deferred until weeks of real logs** (the spec stays; only the work waits):
- Upcasting: `monoagens/versioning.py` exists from 001 T11; do not extend it until a payload
  schema first changes.
- The full prior loader (017); the code reads `prior/dispositions.yaml` directly until then.
- Snapshots (009 T5).

## Rules for every task
1. Read [AGENTS.md](../../AGENTS.md), [constitution.md](../constitution.md) and the spec sections
   the task links, before touching code.
2. Touch only the files listed under **May touch**. A needed change elsewhere stops the work and
   goes to the architect.
3. **Never change a specification** (`docs/*.md`). A design question stops the work; write it
   under the task's **Notes** and report.
4. One sub-task = one commit when practical. A task is done only when its **Done when** checks
   pass **and** every earlier test and the replay suite still pass.
5. Every behavior change ships with at least one replay scenario that shows it.
6. Design by brain function, not by scenario: never add a rule for one case to the core.
7. Do only what the sub-task asks; no machinery for later (001 T11 built upcasting that was
   deferred).
8. Workers may tick boxes and write under **Notes** of their own task section; nothing else.

Checklist syntax (as in Spec Kit / Kiro): `- [ ] T1 description` — `[P]` marks a sub-task that
may run in parallel with the others of the same task; `- [x]` when done.

## Template
```markdown
## NNN — Layer: title
Status: todo | Depends on: NNN
Read first: links to spec sections
May touch: files

**Goal.** …

- [ ] T1 …

**Done when.** …

**Notes.**
```
