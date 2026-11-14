# Tasks

Work for coding workers. **One file = one task = one worker.** Each task is split into parts and
items, numbered in the order they are done (`3.2` = part 3, item 2). Tasks run **one after
another**: one worker takes one task file, works through its parts in order (one commit per item
when practical), verifies its own work, ends with the real-model test in the last part, and
reports to the owner. The next task starts after that report.

Tasks are numbered in the order they are done. Finished tasks stay as history. Tasks written
before Task 3 name files by their earlier names; the [spec.md §9](../spec.md) maps them.

| Task | File | Status | Depends on |
|---|---|---|---|
| 1 | [Core: align with the frozen contract](done/1-core-align.md) | **done** — 181 tests; fake replay 17/25, fixed in Task 2 Part 2 | — |
| 2 | [Core: launch](done/2-core-launch.md) | **done** — 321 tests, fake 133 checks, violations 0; 16 and 30 flaky; 20, 22 run in Task 3 | Task 1 |
| 3 | [Core: clean refactor](done/3-core-refactor.md) | **done** — 336 tests, fake 133/133, violations 0; core imports no middleware; 22/26 real, 09 and 23 flaky, 24 open | Task 2 |
| 4 | [Middleware and first plugins](done/4-middleware-plugins.md) | **done** — 539 tests, fake 133/133, launch set 24/26 (Task 3: 22/26), sandbox 53/53 real, honesty counts claims; 5.1–5.3 moved to Task 6; loop in 19 and leaks vs desk carried to Task 5 0.0 | Task 3 |
| 5 | [Adapters: WhatsApp, Discord and more](done/5-adapters.md) | **stopped** at 1.12 P (2026-11-14) — providers, loop, brakes, full outputs, identity and linking, WhatsApp, Discord, Telegram and CLI native, the audit's S, A, F.1 and P; the rest (1.10 D83, 1.11, 1.12 F/E/O, 1.8) moved to Task 6 | Task 4 |
| 6 | [The locked contract](6-contract.md) | **next** — contract v2 as code and its lock, the core and the middleware on the contract, adapters translating everything, tests through the contract only, five fake bodies, the remaining audit items, then daily use on WhatsApp (D86–D88) | Task 5 |
| 7 | [Core: depth of mind](7-core-mind.md) | todo — after weeks of daily use | Task 6 and some weeks of daily use |
| 8 | [First users and release](8-release.md) | todo | Task 7 |
| — | [Later: voice, Home Assistant, more model providers](later-voice-home.md) | todo — moved out of Task 5 Part 2; its number and place are decided with the web panel task | Task 6 |

The architect updates this table and each file's `Status:` line; workers never do.

## Launch
"Launch" means: the owner uses the soul every day in a direct chat and one group. Task 2 is the
whole path there; Task 3 then cleans the core without changing what it does. The first number is the **missed-help rate**: of the group messages that
should have been answered, how many passed the gate. The gate is tuned for recall, not
precision: a needless wake costs a few tokens and reasoning can still stay silent; a missed
moment is a failure.

**Deferred until weeks of real logs** (the spec stays; only the work waits):
- Extending upcasting: `monoagens/versioning.py` exists from Task 1; leave it as it is until a
  payload schema first changes.
- The full prior loader with its loading rules ([spec.md §3.9.4](../spec.md)); the code reads
  `prior/dispositions.yaml` directly until then.
- Snapshots of `state` ([spec.md §3.3](../spec.md)).

The rules every task follows are in [AGENTS.md](../../AGENTS.md).
