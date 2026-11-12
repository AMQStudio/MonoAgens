# Task 5 — Bodies, first users and release

Status: todo | Depends on: Task 4
One task = one coordinator prompt. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** One soul across many bodies and providers, then real users, a release, an honest
evaluation, and stable contracts for other developers.

---

## Part 1 — Identity linking, more bodies, more models
Read first: [bodies.md](../bodies.md), [middleware.md](../middleware.md) §7 (identity), [core.md](../core.md) §4, §8
May touch: `monoagens/bodies/`, `monoagens/middleware.py` (identity part), provider adapters, `tests/`, `scenarios/`

Linking comes first: it is what lets a person's private context follow them (D45), so a wrong
link is a leak.

- [ ] 1.1 Identity linking (`/link`): the owner links accounts, or the person confirms with a verification code sent from the second account; every link is an event and can be undone; the soul may suspect and ask, never link silently.
- [ ] 1.2 WhatsApp with `history` and back-fill on connect; receiving and sending files as capabilities (a received file is a percept with a description and a reference in `data`).
- [ ] 1.3 Discord.
- [ ] 1.4 Voice (STT/TTS).
- [ ] 1.5 First non-chat body (a game such as Minecraft, an IoT device or a robot simulation) with its own reflexes inside the body.
- [ ] 1.6 Native provider adapters; optional separate models for sleep and the classifier.
- [ ] 1.7 Check: one conversation continues across three bodies; a person's private context follows them only after a verified link; a non-chat body senses and acts through the same contract; switching provider is a `.env` change.

## Part 2 — First users and the release gate
Read first: [project.md](../project.md) §1
May touch: `README.md`, `LICENSE`, `scenarios/`

- [ ] 2.1 The owner alone for a few days with replay checks after each change.
- [ ] 2.2 One real group the owner knows well, with the owner policy on.
- [ ] 2.3 Release gate: end-to-end with a real model, a real database, long multi-day conversations and real people; per-model replay report; recommended models.
- [ ] 2.4 README for technical self-hosters (brain table, manual install on a server, talk, tune with replay; no claims of consciousness or of being for non-technical users); Apache 2.0 LICENSE.

## Part 3 — Honest human-likeness evaluation
Read first: [project.md](../project.md) §4
May touch: `docs/` results file (new), `scripts/`

- [ ] 3.1 Pre-register the metrics.
- [ ] 3.2 Measure reply timing, messages per hour vs 1/n, when-to-speak precision/recall, promise keeping, recall after 1/7/30 days, blind "which one is the bot".
- [ ] 3.3 Publish results as "≥ X% of human baseline on these N behaviors".

## Part 4 — Stable contracts for developers
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md), [bodies.md](../bodies.md)
May touch: `docs/` (contribution guide, examples)

Only after the contracts have not changed for a while.

- [ ] 4.1 Version the contracts for bodies, plugins and the middleware API.
- [ ] 4.2 Examples and a contribution guide.

**Done when.** The release gate passes with recommended models, evaluation results are
published, and the contracts are documented with examples.

**Notes.**
