# Middleware — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder (one per track or phase,
e.g. `A2.md`), tick steps and record findings. They never change DESIGN, DECISIONS, FORMATS or
MATH; a design question stops the work and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Step 3 — Default owner policy and shared middleware (with the package loader)
- Default owner policy, active out of the box: the two absolute lines, the daily budget, quiet
  hours (D22, D24). Everything else is weighed by the PFC.
- Shared middleware: **commands** (one registry, identical `/help`) and **skills** (installed
  SKILL.md only; static; D34).
- **memory-view** (Draft, D37): list what the soul remembers about the asker or this place,
  corrections through `revise`, owner-only Markdown export.
- Done when: a DM fact is never visible or said in a group, even when someone tries to talk the
  soul into it; a reasonable request from a group member is helped; a package cannot remove the
  policy; `/help` is identical on Telegram and CLI; an example mode package (daily /
  professional) works through the six levers without touching the core.

## Later
- Alias map when a second chat app arrives (D10); RBAC and role packages; package suggestions
  (Draft, D28).
