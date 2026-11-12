# Task 4 — Middleware and first plugins

Status: todo | Depends on: Task 3
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** The foundation in the middle, in seven files (contract, policy, privacy, identity,
registry, loader, interaction), then the first apps that prove the six levers and give the soul tools. The owner policy
from Task 3 enforces the absolute lines; every other middleware and plugin only narrows.
File names follow the layout of Task 3 ([glossary](../glossary.md)).

**From V1.** Branch `archived` holds V1. Its mechanisms below are adapted, never copied whole:
take the mechanism, leave its roles, approvals, flows and multi-runner leases. Read an ADR with
`git show origin/archived:docs/archive/adr-pre-rebaseline/<path>.md`.

---

## Part 1 — The foundation
Read first: [middleware.md](../middleware.md)
May touch: `monoagens/middleware/`, `monoagens/adapters/`, `monoagens/boot.py`, `monoagens/__main__.py`, `config.yaml`, `monoagens/infra/` store (read-only view only), `scripts/replay.py` (imports only), `tests/`, `scenarios/`

- [ ] 1.0 **Follow-ups from Task 3** (its Notes). (a) A cold start never loses a percept: the boot warms what is slow (the embedding model, the database pool) before any adapter starts, so the first message — often the pairing code — passes its inbound hook inside the watchdog; test with a slow fake embedder. (b) Routes: when `to:` names someone with no direct place who is present in the current place, `identity.route` answers in the current place instead of refusing (scenario 24's "no way to reach tg:7001 yet"). (c) The context hook stays inside its watchdog on a long desk: measure `privacy.filter_context` on the largest scenario and record the time under Notes.
- [ ] 1.1 **Seven files** (middleware.md §1, decided 2026-11-12), before any new feature: merge `owner_policy.py` and `limits.py` into `policy.py`; rename `tools.py` to `registry.py`; update the importers (graphify for callers first). Behavior unchanged: tests and fake replay pass as before. `contract.py`, `privacy.py`, `identity.py` keep their names; `loader.py` and `interaction.py` are born below.
- [ ] 1.2 Policy (`policy.py`, `privacy.py`): privacy rules that only narrow (confirm before forwarding a file, a mode that keeps a topic out); route hints to a person via lever 2; optional approval before a tool call, decided here and shown through 1.6; owner commands for budget and quiet hours.
- [ ] 1.3 Registry (`registry.py`): one registry for tools, commands and skills, every entry naming who uses it (soul or person); identical `/help`; capability index behind `<self>` — the core lines always on the context, each capability a short line, its full schema loaded on request (V1 ADR-0015 `engine/0015-capability-manifest`); every tool declares an output cap and cuts with a "continue from here" marker, never silently (V1 ADR-0021 `engine/0021-tool-output-contract`); skills: names and descriptions of relevant SKILL.md in the context, the body on request.
- [ ] 1.4 Loader (`loader.py`): plugin manager reading the manifest (plugins.md §3; V1 ADR-0028 `0028-plugin-model`: off until enabled, its dependencies live with the plugin); `dependencies` load first, a missing one fails at boot naming both; collisions fail at boot; uninstall removes everything but events.
- [ ] 1.5 Adapters through the same loader (adapters.md §2, D49): Telegram and CLI get a `create(options, env)` factory; `__main__.py` imports no adapter and asks the loader for the enabled ones; a test adapter from outside the package loads from `config.yaml` alone. Adapter contract from V1 ADR-0018 (`engine/0018-gateway-contract`): a small required part and optional parts, a missing capability is visible (never a silent `pass`), checked at boot; one folder per adapter split by job (receive, send, format, what it can do).
- [ ] 1.6 Interaction (`interaction.py`, middleware.md §9): the vocabulary in `contract.py` of what an adapter can show (typing, edit message, buttons, reactions); the defaults (thinking only to the owner in the owner's direct chat, tool steps only for long work, approvals ask the person the policy names); a command to switch them; everything shown passes `privacy.py` first; fallbacks when an adapter lacks a capability; an approval answer comes back as an ordinary event. CLI and Telegram render typing, tool steps and an approval (buttons on Telegram, text on CLI); WhatsApp and Discord rendering is Task 5.
- [ ] 1.7 Read-only view of the log and state for plugins in `contract.py` (core.md §3, D43); the policy rule on data leaving the machine (middleware.md §3).
- [ ] 1.8 Check: private content never reaches someone outside its audience, even when someone tries to talk the soul into it, even with a plugin that tries to widen it, and even through a shown thought or tool step; a reasonable request from a group member is helped; no plugin can remove the policy; `/help` is identical on Telegram and CLI; thinking never shows in a group.

## Part 2 — Plugins: modes and memory-view
Read first: [plugins.md](../plugins.md), [middleware.md](../middleware.md) §2
May touch: a new `plugins/` folder, `tests/`, `scenarios/`

- [ ] 2.1 modes (daily / professional): rules in `<self>`, notes, knob offsets, a `set_mode` capability.
- [ ] 2.2 memory-view & manage: list and correct what the soul remembers; owner-only Markdown export.
- [ ] 2.3 Check: both install, work and uninstall without any core change; memory-view never shows private content to anyone outside its audience.

## Part 3 — Plugins: first tools
Read first: [plugins.md](../plugins.md), [core.md](../core.md) §4
May touch: `plugins/`, `tests/`, `scenarios/`

Multi-step work runs in the core's task threads (Task 2 Part 3); the plugins only offer capabilities.
The working-directory tool's "run" asks the owner first by default (approval of 1.2, shown through 1.6).

- [ ] 3.1 Web search/fetch tool.
- [ ] 3.2 File read tool.
- [ ] 3.3 Working-directory tool (read, write and run inside one folder the owner configures) with `long_running` capabilities. A command runs with a computed environment, never the inherited one, so no secret reaches it (V1 ADR-0024 `engine/0024-tool-input-contract`, the shell part); its output follows the cap of 1.3.
- [ ] 3.3b Scenario: "send our finished work to Budi" where the work is a file in the working folder: the working-directory tool reads it, the soul shares it and sends it through an adapter that can attach files (a test adapter until Task 5), and the forwarding policy of Part 1.2 applies.
- [ ] 3.4 Check: "who is the president now?" triggers a search when the search plugin is installed, and an honest "as of my knowledge" answer when it is not; "finish my code, then send it to Budi" runs as a task thread and keeps its promise.

## Part 3b — Capability targets carried over from Task 2
Read first: [core.md](../core.md) §4, §8 (D45–D48, D55, D56), the Notes of Task 2 Part 12
May touch: `scenarios/`, `scripts/replay.py`, `plugins/`, `monoagens/middleware/`, `tests/`

The core already has the mechanisms; these scenarios need middleware, plugins and real
capabilities around them. A core change here is only for a bug, reported first.

- [ ] 3b.1 Scenario 18 (game adapter, Budi's file, sharing to Budi with consent, Fulan never shared): passes with the real model, its material checked (facts and capabilities).
- [ ] 3b.2 Scenario 19 (task thread, delivery to Budi, ask which group): passes with the real model, with the working-directory tool of 3.3.
- [ ] 3b.3 Scenarios 26 and 28 (a fact from the owner's direct chat: ask the owner, answer after `share.granted`): pass with the real model, with the withheld note (D56).

## Part 4 — Real-model test
- [ ] 4.1 `uv run pytest` and the fake replay pass in full.
- [ ] 4.2 Real-model replay and a live CLI session with the model in `.env` (never read or print it), with both plugins installed: modes switch, memory-view lists and corrects, a search and a working-directory task run end to end, with typing, tool steps and the approval shown. Report what happened and the metrics.

**Done when.** All parts are ticked, tests and both replays reported, no core file changed,
the code readable (AGENTS.md size guide).

**Notes.**
