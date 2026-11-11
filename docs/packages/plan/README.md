# Packages — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder (one per track or phase,
e.g. `A2.md`), tick steps and record findings. They never change DESIGN, DECISIONS, FORMATS or
MATH; a design question stops the work and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Step 3 — Package loader
- Loader for packages (organs + middleware + skills + dependencies) from one manifest
  ([FORMATS.md](../FORMATS.md)), with shared/organ-own scope, the six middleware levers and a
  state namespace per package (D41). Collisions fail at boot.
- Done when: the example mode package installs, runs and uninstalls (its knob offsets and rules
  disappear with it) without any core change.

## Step 9 — Developers (D13)
- Stable, versioned contracts for organs, packages and middleware; examples; contribution guide.
- Starts only after the contracts have not changed for a while.
