# Task 4 — Middleware and first plugins

Status: todo | Depends on: Task 3
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [README](README.md).

**Goal.** The foundation in the middle, in eight files (contract, policy, privacy, identity,
registry, loader, interaction, roles), then the first plugins — Soul Tuning, Web Search,
Workspace with its sandbox, PDF Reader — that prove the six levers and give the soul tools. The
owner policy from Task 3 enforces the absolute lines; every other middleware and plugin only
narrows. File names follow the layout of Task 3 ([glossary](../glossary.md)).

**Order and stops.** Part 1 is green (tests and fake replay) before any plugin starts. One commit
per item when practical. If the work cannot finish, stop at the end of a part, push, and report
what is done and what is left; never leave a part half-done.

**From V1.** Branch `archived` holds V1. Its mechanisms below are adapted, never copied whole:
take the mechanism, leave its roles, approvals, flows and multi-runner leases. Read an ADR with
`git show origin/archived:docs/archive/adr-pre-rebaseline/<path>.md`, code with
`git show origin/archived:<path>`.

---

## Part 1 — The foundation
Read first: [middleware.md](../middleware.md), [plugins.md](../plugins.md) §3–4, [project.md](../project.md) §2
May touch: `monoagens/middleware/`, `monoagens/adapters/`, `monoagens/boot.py`, `monoagens/__main__.py`, `monoagens/infra/` (config loading, store read-only view), `config.yaml`, `soul.yaml`, `prior/manifest.yaml`, `config.d/`, `scripts/replay.py` (imports only), `tests/`, `scenarios/`

- [ ] 1.0 **Follow-ups from Task 3** (its Notes). (a) A cold start never loses a percept: the boot warms what is slow (the embedding model, the database pool) before any adapter starts, so the first message — often the pairing code — passes its inbound hook inside the watchdog; test with a slow fake embedder. (b) Routes: when `to:` names someone with no direct place who is present in the current place, `identity.route` answers in the current place instead of refusing (scenario 24's "no way to reach tg:7001 yet"). (c) The context hook stays inside its watchdog on a long desk: measure `privacy.filter_context` on the largest scenario and record the time under Notes.
- [ ] 1.1 **Eight files** (middleware.md §1), before any new feature: merge `owner_policy.py` and `limits.py` into `policy.py`; rename `tools.py` to `registry.py`; update the importers (graphify for callers first). Behavior unchanged: tests and fake replay pass as before. `loader.py`, `interaction.py` and `roles.py` are born below.
- [ ] 1.2 **Format versions** (D58, project.md §2): `version:` in `config.yaml`, `config.d/*.yaml`, `soul.yaml`; the prior through `prior/manifest.yaml`'s `format_version`; manifests through `manifest_version`. Older is upcast in memory and named in the boot report (`config.yaml` 1 → 2: `plugins:` list → map), never rewritten; newer fails at boot; missing is read as 1 with a warning. Update the shipped `config.yaml` and `soul.yaml`.
- [ ] 1.3 **Registry** (`registry.py`, middleware.md §4, §6): one registry for tools, commands and skills, each entry naming who uses it; identical `/help`; tool facts (`reads`, `changes`, `runs_code`, `network`, per-call `sensitive`) in `contract.py`'s vocabulary, assembled next to the descriptor, a tool without facts fails at boot; a turn is offered only what its requester may use; capability index behind `<self>` — core lines always present, one short line per capability, full schema on request (V1 ADR `engine/0015-capability-manifest`); every tool declares an output cap and cuts with a "continue from here" marker (V1 ADR `engine/0021-tool-output-contract`); skills: names and descriptions of relevant SKILL.md in the context, the body on request.
- [ ] 1.4 **Loader** (`loader.py`, middleware.md §5, plugins.md §3–4): reads `config.yaml`, `config.d/` and manifests (V1 ADR `0028-plugin-model`: off until enabled, dependencies live with the plugin); `dependencies` load first, `optional_dependencies` when present; merge rules (maps merge, lists replace whole, `[]` off, warning when a list replaces a default); unknown key or wrong value fails at boot; a `config.d/` file for an absent plugin is reported and ignored; features switched off are removed; extension points opened and filled; collisions fail at boot; the files are the source of truth: `/plugin enable|disable|update` writes only its own lines (and creates `config.d/<name>.yaml` with `permissions` and `role_settings` from the manifest), after a backup, applied without a restart; plugin updates follow the dpkg rule (middleware.md §5); every change is recorded as an event for history only; `/plugin show <name>` prints defaults; `/reload` validates first and keeps the running configuration on any error, naming what needs a restart; uninstall removes nothing from the owner's files.
- [ ] 1.5 **Adapters through the same loader** (adapters.md §2, D49): Telegram and CLI get a `create(options, env)` factory; `__main__.py` imports no adapter and asks the loader for the enabled ones; a test adapter from outside the package loads from `config.yaml` alone. Adapter contract from V1 ADR-0018 (`engine/0018-gateway-contract`): a small required part and optional parts, a missing capability is visible (never a silent `pass`), checked at boot; adapters declare tool facts and may offer place info (name, member count, platform role facts); one folder per adapter split by job (receive, send, format, what it can do).
- [ ] 1.6 **Roles** (`roles.py`, middleware.md §8): four tiers; permissions and `role_settings` from manifests; tier roles and group roles from `config.yaml` with `label` and `context`; `allow`, `deny` (wins), `settings`, `approval`; permission tiers from `config.d/`; `fallback`; place overrides; wildcards `plugin.*` and `*` with their expansion in the boot report; `people:` keyed by person id (`label`, `roles`, `context`) as the only record of who holds which role, written by `/role add|remove` (one entry, backup, no restart, `roles.changed` event for history); `/whoami` (full in a direct chat; label, roles and place only in a shared place) and `/whois`; the middleware's own permissions (`config.reload`, `plugin.manage`, `roles.manage`, `identity.whois`, `approval.answer` at `admin`; `identity.whoami` at `guest`); no one gives a role at or above their own highest tier, owner-only actions are not movable; each role's and person's `context` on person cards within a cap; `/roles show <role>` with each line's source; empty configuration = owner free, everyone else only talks.
- [ ] 1.7 **Policy** (`policy.py`, `privacy.py`): pass, ask or refuse from the requester's (`origin_actor`) permissions, the tool's facts and the approval mode (`yolo`, `default`, `strict`; default `default`; `sensitive` always asks); privacy rules that only narrow (confirm before forwarding a file, a mode that keeps a topic out); route hints to a person via lever 2; owner commands for budget and quiet hours.
- [ ] 1.8 **Interaction** (`interaction.py`, middleware.md §9): the vocabulary in `contract.py` of what an adapter can show (typing, edit message, buttons, reactions); the defaults (thinking only to the owner in the owner's direct chat, tool steps only for long work, approvals ask the owner); a command to switch them; everything shown passes `privacy.py` first; fallbacks when an adapter lacks a capability; an approval answer comes back as an ordinary event. CLI and Telegram render typing, tool steps and an approval (buttons on Telegram, text on CLI); WhatsApp and Discord are Task 5.
- [ ] 1.9 Read-only view of the log and state for plugins in `contract.py` (core.md §3, D43); the policy rule on data leaving the machine (middleware.md §3).
- [ ] 1.10 **Attachments** (middleware.md §10): the vocabulary in `contract.py`; the store in `infra/` with `put` and `open`, `open` checked by `privacy.py` against the source event's audience; hard forgetting deletes the file; Telegram and CLI receive and send several attachments per message, declaring their limits; `interaction.py` splits when an adapter sends fewer per message. Port the mechanism of V1's `AttachmentStore` (ADR `engine/0004-core-tools-and-cross-gateway-messaging`).
- [ ] 1.11 Check: private content never reaches someone outside its audience, even when someone tries to talk the soul into it, even with a plugin that tries to widen it, and even through a shown thought or tool step; a guest asking for a tool their roles lack is refused and the soul does not promise it; a reasonable request from a group member is helped; no plugin can remove the policy; `/help` is identical on Telegram and CLI; thinking never shows in a group; a new role with only `base: member` works in every plugin; an admin cannot give `admin` or raise a permission above their tier; `/whoami` in a group never shows linked accounts; three files in one message are received, read and forwarded, and a file from a private chat is not opened for someone else without `share`.

## Part 2 — Soul Tuning
Read first: [plugins.md](../plugins.md) §5.1, [middleware.md](../middleware.md) §2, §7
May touch: `plugins/soul-tuning/`, `config.d/`, `tests/`, `scenarios/`

- [ ] 2.1 Modes (daily / professional): rules in `<self>`, knob offsets within bounds (recorded as `disposition.updated`; the middleware computes the effective knobs, middleware.md §2 lever 6), `/mode`.
- [ ] 2.2 Example conversations in the stable prompt, in a fixed order.
- [ ] 2.3 Check: installs, works and uninstalls without any core change; the stable prefix hash does not change when the mode is unchanged; only the owner changes it.

## Part 3 — Web Search
Read first: [plugins.md](../plugins.md) §5.2
May touch: `plugins/web-search/`, `config.d/`, `tests/`, `scenarios/`

- [ ] 3.1 `web_search` with the provider chain (SearXNG, Tavily, Brave, Exa, Serper, Google, DDGS last), ported from V1 `monoagens/engine/tools/web_search.py` (ADR `engine/0026-search`): normalized results, structured failure reasons, keys from `.env`.
- [ ] 3.2 `web_fetch` ported from V1 `monoagens/engine/tools/fetch.py`: internal addresses refused (resolve, check against the blocked set, connect to the checked address).
- [ ] 3.3 Check: "who is the president now?" triggers a search when the plugin is on, and an honest "as of my knowledge" answer when it is off; `web_fetch` of `http://localhost`, a private address and a name resolving to one is refused.

## Part 4 — Workspace and PDF Reader
Read first: [plugins.md](../plugins.md) §5.3–5.4, [core.md](../core.md) §4
May touch: `plugins/workspace/`, `plugins/pdf-reader/`, `config.yaml`, `config.d/`, `tests/`, `scenarios/`; for 4.7 only: `monoagens/core/reasoning.py`, `monoagens/core/jobs.py`, `monoagens/core/kernel.py`, `prior/dispositions.yaml`, `.env.example`

Multi-step work runs in the core's task threads (Task 2 Part 3); the plugins only offer capabilities.

- [ ] 4.1 Tools `read`, `write`, `edit`, `grep`, `glob` over the person's workspace and shared folders (`workspace.shared` on their roles); `read` also takes an adapter's attachment reference; the `reader` extension point.
- [ ] 4.2 Sandbox for `run` (D60): the shared Debian slim base (prepared once), an overlay layer per person with the soft quota from `workspace.quota`, an empty `/tmp` per run, private `/proc` and `/dev`, nothing of the host visible; a computed environment without secrets (V1 ADR `engine/0024-tool-input-contract`); time, CPU, memory and process limits; capped output.
- [ ] 4.3 Network through pasta: outbound allowed; host loopback, private ranges, metadata and the server's own addresses blocked; no inbound. Fail-closed: no pasta → no network, no bubblewrap or base → `run` unavailable, named in the boot report.
- [ ] 4.4 The guard (ported from V1's two-stage guard) marks calls `sensitive`; approval follows the person's mode (1.7).
- [ ] 4.5 PDF Reader fills `reader` for `.pdf` (pypdf); `read` of a type without a reader says so and names the extension.
- [ ] 4.6 Scenario: "send our finished work to Budi" where the work is a file in the workspace: `read` gets it, the soul shares it and sends it through an adapter that can attach files (a test adapter until Task 5), and the forwarding rule of 1.7 applies.
- [ ] 4.7 **Batch tool calls**: send `parallel_tool_calls: true` to the provider unless `LLM_PARALLEL_TOOL_CALLS=false` in `.env` (add it to `.env.example`). Scenario "grep three files, then sum them up": record wakes, model calls and tokens. If one batch wakes reasoning more than once, implement the core fix in this item: the results of one round are awaited together, keyed by that round (never a kernel-wide counter; a result from another place or thread never counts), until all arrived or a timeout from the prior (a new constant in `prior/dispositions.yaml`), and wake reasoning once; record the before/after numbers under Notes as the evidence (constitution §1).
- [ ] 4.8 Check: from inside `run`, the host's home, `.env` and the database port are unreachable, another person's workspace and `/tmp` are invisible, a private address is unreachable, the internet is reachable; a guest cannot `run`; in `default` mode `run` asks the owner; "finish my code, then send it to Budi" runs as a task thread and keeps its promise.

## Part 5 — Capability targets carried over from Task 2
Read first: [core.md](../core.md) §4, §8 (D45–D48, D55, D56), the Notes of Task 2 Part 12
May touch: `scenarios/`, `scripts/replay.py`, `plugins/`, `monoagens/middleware/`, `tests/`

The core already has the mechanisms; these scenarios need middleware, plugins and real
capabilities around them. A core change here is only for a bug, reported first.

- [ ] 5.1 Scenario 18 (game adapter, Budi's file, sharing to Budi with consent, Fulan never shared): passes with the real model, its material checked (facts and capabilities).
- [ ] 5.2 Scenario 19 (task thread, delivery to Budi, ask which group): passes with the real model, with Workspace.
- [ ] 5.3 Scenarios 26 and 28 (a fact from the owner's direct chat: ask the owner, answer after `share.granted`): pass with the real model, with the withheld note (D56).

## Part 6 — Real-model test
- [ ] 6.1 `uv run pytest` and the fake replay pass in full.
- [ ] 6.2 Real-model replay and a live CLI session with the model in `.env` (never read or print it), with the four plugins on: a mode switches, a role's `context` reaches a person's card, a search runs, a Workspace task with `run` runs end to end with typing, tool steps and the approval shown. Report what happened and the metrics.

**Done when.** All parts are ticked, tests and both replays reported, no core file changed except by 4.7 with its evidence,
the code readable (AGENTS.md size guide).

**Notes.**
