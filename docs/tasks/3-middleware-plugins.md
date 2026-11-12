# Task 3 — Middleware and first plugins

Status: todo | Depends on: Task 2
One task = one coordinator prompt. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** The foundation in the middle (policy, commands & tools, install, skills, read-only
view), then the first apps that prove the six levers and give the soul tools. The absolute lines
are enforced by the core itself; middleware only narrows.

---

## Part 1 — The foundation
Read first: [middleware.md](../middleware.md)
May touch: `monoagens/middleware.py` (and a `middleware/` package if it grows past one responsibility), `monoagens/bodies/`, `monoagens/__main__.py`, `config.yaml`, `monoagens/store.py` (read-only view only), `tests/`, `scenarios/`

- [ ] 1.1 Policy: default owner policy — daily budget, quiet hours; first and last in every hook; privacy rules that only narrow (confirm before forwarding a file, a mode that keeps a topic out); route hints to a person via lever 2.
- [ ] 1.2 Commands & tools: one command registry (identical `/help`); capability registry behind `<self>`; optional tool-use approvals.
- [ ] 1.3 Install: plugin manager reading the manifest (plugins.md §3); collisions fail at boot; uninstall removes everything but events.
- [ ] 1.3b Bodies through the same loader (bodies.md §2, D49): Telegram and CLI get a `create(options, env)` factory; `__main__.py` imports no body and asks the loader for the enabled ones; a test body from outside the package loads from `config.yaml` alone.
- [ ] 1.4 Skills: names and descriptions of relevant SKILL.md in the desk; body on request.
- [ ] 1.5 Read-only view of the log and state for plugins (core.md §3, D43); the policy rule on data leaving the machine (middleware.md §3).
- [ ] 1.6 Check: private content never reaches someone outside its audience, even when someone tries to talk the soul into it and even with a plugin that tries to widen it; a reasonable request from a group member is helped; no plugin can remove the policy; `/help` is identical on Telegram and CLI.

## Part 2 — Plugins: modes and memory-view
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md) §2
May touch: a new `plugins/` folder, `tests/`, `scenarios/`

- [ ] 2.1 modes (daily / professional): rules in `<self>`, notes, knob offsets, a `set_mode` capability.
- [ ] 2.2 memory-view & manage: list and correct what the soul remembers; owner-only Markdown export.
- [ ] 2.3 Check: both install, work and uninstall without any core change; memory-view never shows private content to anyone outside its audience.

## Part 3 — Plugins: first tool organs
Read first: [plugins.md](../plugins.md), [core.md](../core.md) §4
May touch: `plugins/`, `tests/`, `scenarios/`

Multi-step work runs in the core's task threads (Task 2 Part 3); the plugins only offer capabilities.

- [ ] 3.1 Web search/fetch tool organ.
- [ ] 3.2 File read tool organ.
- [ ] 3.3 Working-directory organ (read, write and run inside one folder the owner configures) with `long_running` capabilities.
- [ ] 3.3b Scenario: "send our finished work to Budi" where the work is a file in the working folder: the working-directory organ reads it, the soul shares it and sends it through a body that can attach files (a test organ until Task 5), and the forwarding policy of Part 1.1 applies.
- [ ] 3.4 Check: "who is the president now?" triggers a search when the search plugin is installed, and an honest "as of my knowledge" answer when it is not; "finish my code, then send it to Budi" runs as a task thread and keeps its promise.

## Part 3b — Capability targets carried over from Task 2
Read first: [core.md](../core.md) §4, §8 (D45–D48, D55, D56), the Notes of Task 2 Part 12
May touch: `scenarios/`, `scripts/replay.py`, `plugins/`, `monoagens/middleware.py`, `tests/`

The core already has the mechanisms; these scenarios need middleware, plugins and real
capabilities around them. A core change here is only for a bug, reported first.

- [ ] 3b.1 Scenario 18 (game body, Budi's file, sharing to Budi with consent, Fulan never shared): passes with the real model, its material checked (facts and capabilities).
- [ ] 3b.2 Scenario 19 (task thread, delivery to Budi, ask which group): passes with the real model, with the working-directory organ of 3.3.
- [ ] 3b.3 Scenarios 26 and 28 (a fact from the owner's direct chat: ask the owner, answer after `share.granted`): pass with the real model, with the withheld note (D56).

## Part 4 — Real-model test
- [ ] 4.1 `uv run pytest` and the fake replay pass in full.
- [ ] 4.2 Real-model replay and a live CLI session with the model in `.env` (never read or print it), with both plugins installed: modes switch, memory-view lists and corrects, a search and a working-directory task run end to end. Report what happened and the metrics.

**Done when.** All parts are ticked, tests and both replays reported, no core file changed,
the code readable (AGENTS.md size guide).

**Notes.**
