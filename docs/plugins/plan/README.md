# Plugins — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder, tick steps and record
findings. They never change DESIGN, DECISIONS, FORMATS or MATH; a design question stops the work
and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Step 3 (after the middleware foundation)
- **modes** example plugin (daily / professional): proves the six levers end to end.
- **memory-view & manage** (Draft).
- Done when: both install, work and uninstall without any core change; memory-view never shows
  private content outside its place.

## Step 6 — Tool plugins and multi-step work
- Web search/fetch and file read as tool organs; long-running actions report through
  `act.result`; limits on steps per task.
- Done when: "who is the president now?" triggers a search when the search plugin is installed,
  and an honest "as of my knowledge" answer when it is not.

## Step 9 — Developers (D13)
- Stable, versioned contracts for bodies, plugins and the middleware API; examples; a
  contribution guide. Starts only after the contracts have not changed for a while.
