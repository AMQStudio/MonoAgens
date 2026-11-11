# Stage 4 — Bodies and project

More bodies, first users, release, evaluation. Order and status: [README](README.md). Each
section is one task; work only inside yours.

---

## 018 — Bodies: identity linking, more bodies, more models
Status: todo | Depends on: 012
Read first: [bodies.md](../bodies.md), [middleware.md](../middleware.md) §7 (identity), [core.md](../core.md) §4, §8
May touch: `monoagens/bodies/`, `monoagens/middleware.py` (identity part), provider adapters, `tests/`

**Goal.** One soul across many bodies and providers. Linking comes first: it is what lets a
person's private context follow them (D45), so a wrong link is a leak.

- [ ] T1 Identity linking (`/link`): the owner links accounts, or the person confirms with a verification code sent from the second account; every link is an event and can be undone; the soul may suspect and ask, never link silently.
- [ ] T2 [P] WhatsApp with `history` and back-fill on connect.
- [ ] T3 [P] Discord.
- [ ] T4 [P] Voice (STT/TTS).
- [ ] T5 [P] First non-chat body (a game such as Minecraft, an IoT device or a robot simulation) with its own reflexes inside the body.
- [ ] T6 Native provider adapters; optional separate models for sleep and the classifier.

**Done when.** One conversation continues across three bodies; a person's private context
follows them only after a verified link; a non-chat body senses and acts through the same
contract; switching provider is a `.env` change.

**Notes.**

---

## 014 — Project: first users and the release gate
Status: todo | Depends on: 013
Read first: [project.md](../project.md) §1
May touch: `README.md`, `LICENSE`, `scenarios/`

**Goal.** Real use, then a release.

- [ ] T1 The owner alone for a few days with replay checks after each change.
- [ ] T2 One real group the owner knows well, with the owner policy on.
- [ ] T3 Release gate: end-to-end with a real model, a real database, long multi-day conversations and real people; per-model replay report; recommended models.
- [ ] T4 README for technical self-hosters (brain table, manual install on a server, talk, tune with replay; no claims of consciousness or of being for non-technical users); Apache 2.0 LICENSE.

**Done when.** The release gate passes and the report lists recommended models.

**Notes.**

---

## 015 — Project: honest human-likeness evaluation
Status: todo | Depends on: 014
Read first: [project.md](../project.md) §4
May touch: `docs/` results file (new), `scripts/`

**Goal.** Per-behavior parity against a human baseline in the same group.

- [ ] T1 Pre-register the metrics.
- [ ] T2 Measure reply timing, messages per hour vs 1/n, when-to-speak precision/recall, promise keeping, recall after 1/7/30 days, blind "which one is the bot".

**Done when.** Results published as "≥ X% of human baseline on these N behaviors".

**Notes.**
