# Stage 3 — Middleware and plugins

The foundation, then the first apps. Order and status: [README](README.md). Each section is one
task; work only inside yours.

---

## 012 — Middleware: the foundation
Status: todo | Depends on: launch (stage 1)
Read first: [middleware.md](../middleware.md)
May touch: `monoagens/middleware.py` (and a `middleware/` package if it outgrows its budget), `monoagens/bodies/`, `monoagens/store.py` (read-only view only), `tests/`

**Goal.** Policy, commands & tools, install and skills, in this order. The absolute lines are
enforced by the core itself; middleware only narrows.

- [ ] T1 Policy: default owner policy — daily budget, quiet hours; first and last in every hook; privacy rules that only narrow (confirm before forwarding a file, a mode that keeps a topic out); route hints to a person via lever 2.
- [ ] T2 Commands & tools: one command registry (identical `/help`); capability registry behind `<self>`; optional tool-use approvals.
- [ ] T3 Install: plugin manager reading the manifest (plugins.md §3); collisions fail at boot; uninstall removes everything but events.
- [ ] T4 Skills: names and descriptions of relevant SKILL.md in the desk; body on request.
- [ ] T5 Read-only view of the log and state for plugins (core.md §3, D43); the policy rule on data leaving the machine (middleware.md §3).

**Done when.** Private content never reaches someone outside its audience, even when someone
tries to talk the soul into it and even with a plugin that tries to widen it; a reasonable
request from a group member is helped; no plugin can remove the policy; `/help` is identical
on Telegram and CLI.

**Notes.**

---

## 013 — Plugins: modes example and memory-view
Status: todo | Depends on: 012
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md) §2
May touch: a new `plugins/` folder, `tests/`

**Goal.** Prove the six levers end to end with two plugins.

- [ ] T1 modes (daily / professional): rules in `<self>`, notes, knob offsets, a `set_mode` capability.
- [ ] T2 memory-view & manage: list and correct what the soul remembers; owner-only Markdown export.

**Done when.** Both install, work and uninstall without any core change; memory-view never shows
private content to anyone outside its audience.

**Notes.**

---

## 016 — Plugins: first tool organs
Status: todo | Depends on: 013
Read first: [plugins.md](../plugins.md), [core.md](../core.md) §4
May touch: `plugins/`, `tests/`

**Goal.** Web search/fetch, file read and a working directory as tool organs. Multi-step work
runs in the core's task threads (020); the plugins only offer capabilities.

- [ ] T1 [P] Web search/fetch tool organ.
- [ ] T2 [P] File read tool organ.
- [ ] T3 [P] Working-directory organ (read, write and run inside one folder the owner configures) with `long_running` capabilities.

**Done when.** "who is the president now?" triggers a search when the search plugin is
installed, and an honest "as of my knowledge" answer when it is not; "finish my code, then send
it to Budi" runs as a task thread and keeps its promise.

**Notes.**

---

## 019 — Plugins: stable contracts for developers
Status: todo | Depends on: 018
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md), [bodies.md](../bodies.md)
May touch: `docs/` (contribution guide, examples)

**Goal.** Versioned, stable contracts for bodies, plugins and the middleware API.

- [ ] T1 Version the contracts.
- [ ] T2 Examples and a contribution guide.

**Done when.** Contracts have not changed for a while and are documented with examples.

**Notes.**
