# Task 8 — First users and release

Status: todo | Depends on: Task 7
One task = one worker prompt, done by one worker. Work through the parts **in order**. File names follow the layout of Task 3 ([glossary](../glossary.md)). Rules: [README](README.md).

**Goal.** Real users, a release, an honest evaluation, and stable contracts for other
developers.

---

## Part 1 — First users and the release gate
Read first: [project.md](../project.md) §1
May touch: `README.md`, `LICENSE`, `scenarios/`

- [ ] 1.1 The owner alone for a few days with replay checks after each change.
- [ ] 1.2 One real group the owner knows well, with the owner policy on.
- [ ] 1.3 Release gate: end-to-end with a real model, a real database, long multi-day conversations and real people; per-model replay report; recommended models.
- [ ] 1.4 README for technical self-hosters (brain table, manual install on a server, talk, tune with replay; no claims of consciousness or of being for non-technical users); Apache 2.0 LICENSE.

## Part 2 — Honest human-likeness evaluation
Read first: [project.md](../project.md) §4
May touch: `docs/` results file (new), `scripts/`

- [ ] 2.1 Pre-register the metrics.
- [ ] 2.2 Measure reply timing, messages per hour vs 1/n, when-to-speak precision/recall, promise keeping, recall after 1/7/30 days, blind "which one is the bot".
- [ ] 2.3 Publish results as "≥ X% of human baseline on these N behaviors".

## Part 3 — Stable contracts for developers
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md), [adapters.md](../adapters.md)
May touch: `docs/` (contribution guide, examples)

Only after the contracts have not changed for a while.

- [ ] 3.1 Version the contracts for adapters, plugins and the middleware API.
- [ ] 3.2 Examples and a contribution guide.

**Done when.** The release gate passes with recommended models, evaluation results are
published, and the contracts are documented with examples.

**Notes.**
