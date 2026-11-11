# Project — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder (one per track or phase,
e.g. `A2.md`), tick steps and record findings. They never change DESIGN, DECISIONS, FORMATS or
MATH; a design question stops the work and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Step 4 — First users and the release gate (D13, D33)
- **The owner alone** for a few days with replay checks after each change.
- **One real group** the owner knows well, with the owner policy on.
- Release gate: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models.
- Installer (one command from clone to a talking soul) and README (brain table, install, talk,
  tune with replay; no claims of consciousness or "% human"). Add the Apache 2.0 LICENSE file.

## Step 5 — Honest human-likeness evaluation
- Per-behavior parity against a human baseline in the same group: reply timing, messages per hour
  vs 1/n, when-to-speak precision/recall against human annotation, silence where it should speak,
  interruptions where it should be quiet, promise keeping, recall after 1/7/30 days, blind "which
  one is the bot" (target ≈ 50%). Pre-registered metrics. Public claims phrased as "≥ X% of human
  baseline on these N behaviors".
