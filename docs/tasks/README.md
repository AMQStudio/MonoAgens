# Tasks

**One file = one task.** A task states the behavior to achieve and how to prove it, in theory:
no files, functions or fields. The owner designs it with the theory architect; the code
architect turns it into work, ticks the items and writes the Notes. Tasks run **one after
another**; parts and items are numbered in the order they are done (`3.2` = part 3, item 2).

## Format of an open task

```
# Task N — Title
Status: todo | doing | stopped | done | Depends on: …

**Goal.** What the soul can do when the task is done, and which spec sections hold the theory.

## Part 1 — Name
*Theory:* the spec and research sections that hold this part's theory, by name.

- [ ] **1.1 What must be true, in one line.**
  - *Function:* what it is for.
  - *Must hold:* the behavior anyone can observe.
  - *Proof:* the condition a test or a sealed scenario checks.
  - *Neutral:* for a mechanism, the setting at which it has no effect.

**Done when.** …

**Notes.** Written by the code architect: results, material checks, theory questions.
```

Short items may keep only the lines they need. Rules:
- Code never adds a behavior rule that the task or the spec does not state. A case the theory
  does not catch is written in **Notes** as a theory question and brought to the design; it is
  never patched in code.
- The theory architect writes the goal, the items and *Done when*; the code architect ticks the
  items, writes Notes, and keeps the `Status` line and the table below up to date.
- Tasks before Task 7 are history and keep the form they were written in; they may name files by
  earlier names, which the spec's *Terms* maps.

| Task | File | Status | Depends on |
|---|---|---|---|
| 1 | [Core: align with the frozen contract](done/1-core-align.md) | **done** — 181 tests; fake replay 17/25, fixed in Task 2 Part 2 | — |
| 2 | [Core: launch](done/2-core-launch.md) | **done** — 321 tests, fake 133 checks, violations 0; 16 and 30 flaky; 20, 22 run in Task 3 | Task 1 |
| 3 | [Core: clean refactor](done/3-core-refactor.md) | **done** — 336 tests, fake 133/133, violations 0; core imports no middleware; 22/26 real, 09 and 23 flaky, 24 open | Task 2 |
| 4 | [Middleware and first plugins](done/4-middleware-plugins.md) | **done** — 539 tests, fake 133/133, launch set 24/26 (Task 3: 22/26), sandbox 53/53 real, honesty counts claims; 5.1–5.3 moved to Task 6; loop in 19 and leaks vs desk carried to Task 5 0.0 | Task 3 |
| 5 | [Adapters: WhatsApp, Discord and more](done/5-adapters.md) | **stopped** at 1.12 P (2026-11-14) — providers, loop, brakes, full outputs, identity and linking, WhatsApp, Discord, Telegram and CLI native, the audit's S, A, F.1 and P; the rest (1.10, 1.11, 1.12 F/E/O, 1.8) moved to Task 6 | Task 4 |
| 6 | [The locked contract](6-contract.md) | **done** (2026-11-14) — contract v2 as code (no hash lock: the owner dropped it, the ratchet and `tests/contract/` hold it), core and middleware on v2, four adapters translating everything with their own `ask`/`render`/`audience`, tests through the contract only, five fake bodies through the real kernel, 684 tests green, fake replay 158/158, ratchet 0; real launch set 113/158 reported, live WhatsApp findings fixed (LID addressing, owner claim, command wake, conversation window) | Task 5 |
| 7 | [Core: depth of mind](7-core-mind.md) | todo — Parts 1–2 now; Parts 3–7 after weeks of daily use | Task 6 |
| 8 | [First users and release](8-release.md) | todo | Task 7 |
| — | [Later: voice, Home Assistant, more model providers](later-voice-home.md) | todo — moved out of Task 5 Part 2; its number and place are decided with the web panel task | Task 6 |

The code architect updates this table and each file's `Status` line.

## Launch
"Launch" means: the owner uses the soul every day in a direct chat and one group. Task 2 is the
whole path there; Task 3 then cleans the core without changing what it does. The first number is the **missed-help rate**: of the group messages that
should have been answered, how many passed the gate. The gate is tuned for recall, not
precision: a needless wake costs a few tokens and reasoning can still stay silent; a missed
moment is a failure.

**Deferred until weeks of real logs** (the spec stays; only the work waits):
- Extending upcasting: it exists since Task 1; it stays as it is until a payload schema first
  changes.
- The full prior loader with its loading rules ([spec.md](../spec.md) *Prior (format version 1)*);
  until then only the dispositions are read.
- Snapshots of `state` ([spec.md](../spec.md) *Books, thought acts and hooks*).

How the code architect and workers work is in [AGENTS.md](../../AGENTS.md).
