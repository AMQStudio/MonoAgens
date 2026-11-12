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

**Baseline (before any edit, 2026-11-12, at `e6bdd81`).** `uv run pytest -q` → 336 passed.
`uv run python scripts/replay.py --fake` → `checks: 131/133 passed`, 2 failed, all in scenario
`20_act_r_activation`; the same scenario alone → `3/3 passed`, so it is order-dependent (like 16
and 30 in Task 3's closing note), not a Task 4 regression. Absolute-line violations 0. Report
`reports/2026-11-12-1646-fake-all.md`; `scenarios/README.md` was regenerated by the run and
restored (not committed as part of this task). Rename inventory (1.1): only `monoagens/boot.py`
imports `owner_policy`; `owner_policy.py` imports `limits` and `tools`; no test imports them
directly (`tests/middleware/__init__.py` only names them in its docstring).

A second baseline run (same commit, with the Task 4 code in place) gave `133/133`; the 2
failures of the first run did not reproduce, so scenario 20 is flaky by order, as suspected.

**1.0 (a) Cold start.** `boot.warm()` awaits `SELECT 1` (a broken connection fails before the
model loads) and then one `embedder.embed(["warm-up"])`; `__main__` calls it after `build` and
before the pairing code or any adapter. The slow-embedder test (`SlowModelEmbedder`, 0.6 s load)
measures the load inside `warm` and the whole pairing percept — recorded, `pair.claimed`,
appraisal — in less than the 500 ms hook timeout; no `error` event is written. `warm` lets an
embedder failure propagate: a soul that cannot embed is not served, and the retry would fail
identically.
**(b) Routes.** `identity.route` now falls back to the current place when `to:` names a person
who is in that place's audience. `labels["hold_for"]` stays `None` for anyone but the owner: that
label means "the thread waits for the owner" (`core/threads.py:70`), not "the answer is for this
person", so the plan's assertion on `hold_for` was dropped in favour of the observable `act.out`
(place and `to`).
**(c) Context cost.** 500 events over 50 shared places, 200 memory events, 5 memories on the
desk: `privacy.filter_context` = **96.3 ms** (assertion `< 250 ms`, half the hook timeout).

**1.1 Rename inventory** (grep at `e6bdd81`, unchanged after): `boot.py` imported
`owner_policy`; `owner_policy.py` imported `identity`, `limits`, `privacy`, `tools`;
`limits.py` imported `infra.store`; no test imported `owner_policy`, `limits` or `tools`
directly. Renames: `owner_policy.py` + `limits.py` → `policy.py` (one `install`), `tools.py` →
`registry.py` (`ToolRegistry` unchanged), plus one docstring reference in `infra/budget.py`.
Behavior unchanged: the suite and the fake replay were identical before and after.

**1.2 Format versions.** The shipped files are already current (`config.yaml` version 2,
`soul.yaml` version 1, `prior/manifest.yaml` `format_version: 1`), so a boot emits **no** upcast
note; only a version 1 or unversioned file does, with the two notes the plan names. `config.yaml`
diff: version 2, `plugins:` list → map with `workspace`, `web-search`, `soul-tuning` enabled,
`interaction: {thinking: owner_direct, steps: long, approvals: owner}`, `roles: {fallback:
unverified, unverified: {base: guest, label, context}}`, `people: {}`. A version 1 `config.yaml`
reads as a map in memory and is never rewritten (test asserts the bytes).

**1.3 Registry.** Entries: `ToolEntry` (facts, permission, output cap), `CommandEntry`,
`SkillEntry`; the one output marker is `contract.OUTPUT_MARKER`, and `_cut` keeps
`cap - len(marker)` characters so `text + marker` fits the cap. `/help` lists commands and
**tools with a permission** — talking capabilities are excluded on purpose, which is what makes
the text byte-identical on Telegram and CLI (their `say`/`react` descriptions differ). The
`<self>` block is a **volatile** `Part` with order 5: `core/context/parts.py::_extension_blocks`
refuses a non-volatile extension block. As decided in the plan, the tool index stays in the
core's `act` tool (one line per capability) and `registry.describe` returns the full schema, so
`<self>` carries commands, relevant skills and standing rules only.
Two consequences of "every entry declares a permission" that the plan did not spell out:
- the core's `self.set_identity` capability needed a permission (the core is frozen and its
  `CAPABILITIES` cannot change), so `policy.install` passes `permissions={"recall":
  "self.recall", "set_identity": "identity.set"}` and the middleware catalog declares
  `identity.set` **without a base** (owner only, like `budget.manage`).
- every adapter's `say`/`react` and every test body now declares `"permission": None`
  explicitly; a non-talking adapter capability (the replay's `dev.run`) declares a permission.
  A talking capability without the explicit `None` fails at boot, as D59 asks.
`roles.py` is born here with the **empty book** (`RoleBook.empty()` = the middleware catalog as
bases, the owner always free, everyone else only the guest-tier permissions) rather than in 1.6:
the registry offers a turn only what its requester may use, so a kernel without a book would
have offered nothing but talking capabilities and broken the fake replay and the core tests.
1.6 adds `RoleBook.build`, the commands and the wildcards on top of the same API.

**1.4 Loader.** `Loader.load(settings)` reads `config.yaml`'s plugins section (already
upcast), every enabled plugin's `plugin.yaml`, and `config.d/<name>.yaml`; `Loader.install`
adds what they brought and never touches the core. Refusals at boot, with the exact messages:
an unknown/missing manifest key, a `state_namespace` that is not the plugin's name, an enabled
plugin without its folder, an unknown `config.d` key or option, a wrong option type, a missing
or cyclic dependency, a tool without facts, a tool whose permission the manifest does not
declare, a command without a permission, two plugins claiming one command, an extension point
opened twice or filled without being opened. `requires:` uses `importlib.util.find_spec` and a
missing module makes the plugin unavailable (its tools are absent) with the exact
`uv pip install -r plugins/<name>/requirements.txt` hint — never a boot failure.
**Reload/uninstall semantics.** `/plugin enable|disable` edit only their own line (byte-compared
in the test), copy `config.d/<name>.yaml` from the manifest the first time, keep a
`config.yaml.bak-<UTC stamp>` backup, and then run the same `reload` path.
`install` swaps middleware through one stable slot per `(hook, "<plugin>-<name>")` — the
pipeline's names and order never change — so `/reload` never collides, and a slot whose plugin
left stops running. It drops every owner's tools, commands and skills first
(`ToolRegistry.discard_owner`), so a changed plugin set applies in place. `/reload` validates
first (`reload refused: <error>` keeps the running configuration); it reports
`restart needed: plugin <name> <old> -> <new>` when a manifest version changed, because the
running process keeps the old modules. Uninstalling is the owner removing the folder and the
`plugins:` entry by hand: the next reload drops its tools/commands/middleware/rules and reports
`config.d/<name>.yaml: plugin <name> is not installed; ignored`; its events stay history.
**`/plugin update`** follows the dpkg rule: config.d is a file the program wrote, so it is
rewritten from the new manifest only where the value still equals the one `/plugin enable`
recorded (`plugin.config_changed` with `key: config.d/<name>.yaml:<section>.<key>`); a value the
owner changed is kept and named. Every change is an event for history only.

**1.5 Adapters.** One folder per adapter split by job: `telegram/{capabilities,format,receive,
send,__init__}.py` and `cli/{same}`. `capabilities.py` holds the descriptors (each with
`facts: ()` and an explicit `permission: None`), `format.py` turns one raw message into one
neutral event and decides "addressed", `receive.py` owns getting input (one `getUpdates` call;
the Unix-socket server), `send.py` owns output (`say`, `react`, `show`, `place_info`) and
`__init__.py` the class, `shows`, `REQUIRES` and `create(options, env)`. `show` and
`place_info` land here because the vocabulary of §9 and the place facts belong to the adapter;
`interaction.py` (1.8) only chooses what to show. The loader builds each enabled `bodies:`
entry: `module` defaults to `monoagens.adapters.<name>`, `REQUIRES` names the variables it
needs (Telegram: `TELEGRAM_BOT_TOKEN`, refused with `<name>: <KEY> is not set`), the required
parts are `name`, `capabilities`, `run` and `act` (`adapter <name>: missing required part
'<part>'`), the optional ones are `shows`, `history`, `place_info`, `get_cursor` and are named
in the boot report; a `shows` value outside `contract.SHOWS` fails at boot. The cursor reader
is set by the loader (`adapter.get_cursor() -> kernel.cursor(name)`) right before
registration, and an adapter name already taken (by a plugin tool or another adapter) fails
with `adapter collision: <name>`. `__main__.py` imports no adapter: build, `warm`, pairing
code, `Loader.load`/`install`, print the report, then `kernel.run`, every `adapter.run` and
the clock; every adapter with `aclose` is closed on shutdown. An adapter outside the package
loads from `config.yaml` alone (`module: tests.fixtures.echo_adapter`, proved in
`tests/adapters/test_loader.py`).

**1.6 Roles.** `RoleBook.build(config, catalog, manifests, config_d, report)` resolves one
pass: bases per permission from `config.d` → manifest → the middleware catalog; roles with
`base`/`allow`/`deny`/`settings`/`approval`/`label`/`context`; people keyed by person id. A
role's **effective tier** is its base or the base of what it allows, and a permission without a
base counts as owner, so a group role that carries one is an owner-tier role. A person's tier is
the highest role held (the fallback included); `person:owner` is above every tier. A person
holds every permission whose base is at or below their tier, plus the `allow`s of their roles,
minus every `deny` (`deny` wins, also at a place). Wildcards (`*`, `workspace.*`) expand in
`allow`/`deny` and as permission keys in `config.d`, never taking a core permission or one
without a base, and each expansion is a boot-report line. `role_settings` resolve per tier
(manifest, then `config.d`), role settings override them, and keys ending in `shared` merge as
lists. `places:` gives allow/deny overrides; a place deny removes what the tier gave.
`/whoami` (guest) shows id, label, roles with labels, tier and approval, and this place with
what the adapter reports (`place_info`); accounts appear only in a direct chat — in a shared
place they are sent to the asker's direct chat on that adapter and the group reply says
"(accounts sent to your direct chat)", while without a direct place they are simply absent.
`/whois` (admin) matches an account, a person id or a label. `/role add|remove` (admin) gives
only roles strictly below the giver's tier (else `only the owner may change <role>`), edits only
`people.<id>.roles` (backup; `people:` and the entry created when absent; the id written as a
quoted YAML key because it holds colons), applies to the running book without a restart and
records `roles.changed`. `/roles show <role>` prints each permission with its base and source
(`middleware`/`manifest`/`config.d`) and the role's label, approval, context and settings from
`config.yaml`. Each role's and person's `context` reaches the person card, one line each, capped
at 200 characters. The book is installed by `Loader.install` (`kernel.roles`); a kernel built
without a loader keeps the empty book, so the owner is free and everyone else only talks.

**1.7 Policy.** The action hook composes, first: `identity.identity_line` → `permission_line` →
`privacy.guard("first")`; last: `privacy.guard("last")` → `quiet_line` → `approval_line`. The
context and inbound hooks keep the policy they had (identity line, the privacy filter, the
approval answer). `permission_line` refuses with `you may not use <capability>` (a Stop, so it
is recorded); `approval_line` decides pass/ask/refuse from the registry facts, the call facts a
plugin may add and the requester's approval mode — `sensitive` always asks, `yolo` passes
otherwise, `strict` always asks, `default` asks only for `runs_code`; a network call is refused
first by the privacy guard if an argument carries a memory this place may not see
(`that would send private content off the machine`); forwarding a file to someone who was not
part of it marks the call `sensitive` with reason `forwarding a file`.
**Approval lifecycle** (events, nothing in memory): asking writes `approval.requested`
(`capability`, `arguments`, `who`, `place`, `reason`, plus `retried_from` when it is the second
question), sets `ctx["refusal"] = "waiting for the owner's approval"` and sends the owner the
question in their direct chat (buttons where the adapter can show them, text otherwise). An
answer is an ordinary percept: a button callback carries `{"approval": id, "answer": bool}`, and
the owner's whole-message `yes|ya|no` answers the newest open request. The answerer must be the
owner or hold `approval.answer`; `approval.answered` records `request`, `allowed`, `who`, `by`.
A `yes` enqueues one `intent` in the request's place carrying `approved` = the request id; the
retry passes when capability **and** arguments equal what was approved, asks exactly once more
when they differ, and a second difference is refused with `the approved call was different; ask
the owner again` (no third question). A second identical call after a retry asks again.
**Quiet hours** are read in the last pass, after the route is resolved: a communicative act
outside the owner's direct chat during the window is refused with
`quiet hours: the owner asked for silence until HH:MM`. `/budget` (owner only) shows today's
total and cap, `/budget <n>` writes `owner_settings.daily_token_budget`; `/quiet 22:00 06:00`
and `/quiet off` write `owner_settings.quiet_hours`; both go through
`loader.edit_config` (backup, only their line) and record `plugin.config_changed`.
**Route hints** (lever 2): the person card's reach line now carries the last-active time
(`telegram at space:tg:1 (last 2026-01-01 03:00)`), so reasoning can pick `via:` without the
middleware choosing for it.
**Narrower rules** in `privacy.py`: `topic_rule(kernel, topics)` / `narrowing(kernel, topics)`
drop recent lines and memories that mention a topic word and add one withheld count labelled
`a topic you keep out of this mode`; they can only remove.

**1.8 Interaction.** The core records it all; `interaction.py` decides what is shown.
Switches: `thinking: owner_direct|off`, `steps: long|off`, `approvals: owner` in
`config.yaml` → `interaction:`, read as `{**DEFAULTS, **settings["interaction"]}`. `narrate`
(inbound) shows each `decision.note` of the chain once (the last shown id lives in
`interaction.shown` events, so it survives a restart) and only in the owner's direct chat; an
`act.result` of a `long_running` call shows `(step) <capability>: <first 200 characters>`. Every
line passes `privacy.visible` first. `present()` applies the fallbacks: the kind the adapter
declares is used through `show`; `typing` missing shows nothing; any other missing kind falls
back to a normal message, never a silent pass. `typing` (action, first pass, after the
permission line) says "typing…" for a communicative act or a long action where the adapter can
show it. `/show <thinking|steps|approvals> on|off` (owner only, `interaction.show`) writes its
one `interaction:` line, applies it at once and records `plugin.config_changed`; `on` restores
the default value for that switch. The boot report names what each adapter declared it can show
and which optional parts it has, so a lost kind is visible (Telegram: typing, edit, buttons,
reactions; CLI: typing).
**The context hook must fit its watchdog** (1.0c): the first full run of the new hook timed out
in `20_act_r_activation` (context hook timed out after 0.5s) because `privacy` resolved each
line's audience with its own queries. `visible`, `memory_visible`, `source_label` and
`withheld_source` now take an optional per-wake cache that `filter_context` fills once per
place, and `identity._direct_places` uses `DISTINCT ON (body)` instead of scanning every
percept of a person; the scenario then passed three runs in a row. Measured on the synthetic
500-event desk: 100 ms (unchanged; there every line comes from a different place, which is the
worst case for the cache).

**1.9 The read-only view.** `contract.LogView` exposes the Log and the state board to plugins
and nothing else: `recent(place, limit, kinds)`, `by_kind(kind, after_id, limit)`,
`state(key)`, `state_prefix(prefix)`, `search(text, limit)` (ILIKE over the payload's
description/text). It holds the connection privately (`_conn`), has no write method and hands
out no connection; unbound it refuses instead of reading somewhere else. The loader creates one
per plugin API and binds it to the kernel's connection at install, so a plugin reads it lazily.
The class docstring carries the rule the plan asks for: any plugin tool that shows a view line
to people passes it through `privacy.visible` (or `memory_visible`) first, and a filtered line
becomes the withheld count, never a shown line. The policy rule on data leaving the machine is
in 1.7 (`privacy.leaving`).

**1.10 Attachments.** `infra/attachments.py` keeps files on disk under
`var/attachments/<2 hex>/<ref>/<safe name>`, content-addressed: `ref = "att_" + sha256[:24]`,
so the same bytes always get one ref and one copy however many places they are sent to. `put`
refuses a file over `MAX_BYTES` (50 MiB) with `file too large: N`, writes `.tmp` then
`os.replace` (never half a file) and sanitises the name (`safe_filename`: basename, printable,
≤120 chars, `attachment` when empty). **Why the origin is derived from the Log**: nothing about
an attachment is stored in a table — `contract.attachment_origins(ref)` returns every
non-forgotten event whose `payload.data.attachments` carries the ref (oldest first), so hard
forgetting erases the origin as it scrubs the payload, and the audience check is the same
`privacy.visible` every other content check uses. `open` allows a file when **any** such event
is visible to the requester's context (a file sent in several places is one stored file) and
refuses otherwise with `that file came with content this place may not see`. **Sweep trigger**:
at boot (`boot.warm`, after the connection check and the embedder warm-up) and whenever an
inbound `forget.hard` is seen; it deletes a file only when no carrier event remains (and the
empty folders), and returns how many files it removed. Adapters get one `AttachmentDoor`
(`put`, `open`); plugins get `api.open_file(ref, context=...)`.
**CLI**: a line starting with `@` names existing paths joined by `|` with optional text
(`@a.txt|@b.txt|please read`) and becomes one percept with every attachment (a missing path is
reported inside the percept's text, never a crash); `say` writes `[file] <name> <path>` per
attachment. **Telegram**: one message may carry a document, the largest photo, audio, video or
voice; updates that share `media_group_id` are collected for 1 s and become one percept with
every file; uploading uses `sendPhoto` for images and `sendDocument` otherwise; a button press
becomes a percept carrying `{"approval": id, "answer": bool}`.
`FILES = {"per_message": 10, "max_bytes": 50_000_000, "mimes": ()}` on Telegram and
`{"per_message": 1, …}` on CLI, named in the boot report. `interaction.split` splits a
communicative act whose `attachments` exceed the adapter's limit: the first part keeps the text,
the later ones carry `(part i/n)`, one `act.out` per part, and one `interaction.split` event
records the count. `contract.describe_attachments` renders the one line reasoning sees
(`Budi sent 3 files: report.pdf, photo.jpg, data.xlsx`).

**1.11 The twelve checks** (`tests/middleware/test_foundation_check.py`), all passing on a
kernel built with `boot.build`, the empty-ish role book of the gate and a stand-in Workspace
tool (the real plugin arrives in Part 4):
1. a private line from the owner's chat is absent from a group desk, and a say quoting a
   memory from there is refused (`that place may not see this content`), nothing sent;
2. a context middleware that appends a private line is narrowed again by the policy's last
   pass, with a withheld note;
3. a guest is not offered `workspace.run` (the registry's `allowed`) and, if the model asks
   anyway, the act is refused with `you may not use workspace.run` — no promise, no send;
4. an addressed question from a group member wakes reasoning and is answered in that group;
5. `HookPipeline` has no way to remove or replace the policy;
6. `/help` is byte-identical on Telegram and CLI (both record the same `act.out` text);
7. a thought shows in the owner's direct chat and never in a group;
8. a role with only `base: member` holds every member-tier permission of the catalog;
9. an admin cannot grant an admin-tier role (`only the owner may change admin`) and can
   grant a lower one;
10. `/whoami` in a group shows no linked accounts;
11. three attachments in one message are present in `data.attachments`, read through the
    tools (through the one door), and forwarded in one `act.out` carrying all three refs;
12. a file from a private chat is refused by `open_attachment` in a group and allowed after
    `share.granted`.
Part 1 was green at the end of every item: the suite (`339` after 1.0 → `477` after 1.11) and
the fake replay (`133/133`) were run after each of them.

**Part 2 — Soul Tuning (`plugins/soul-tuning/`).** Manifest exactly as planned (`settings:
{mode: daily}`, `permissions: {soul-tuning.mode}` **without a base**, so only the owner has
it, `knob_offsets: {talkativeness: [-0.2, 0.2]}`). `modes.py` holds the two modes and reads
`examples.yaml` (three exchanges per mode, refused if a mode is missing or more than five
exchanges). The context block is **volatile** (order 1, so before the core's `now` at 10),
carrying `mode`, the rule and the examples in file order; the stable prefix is identical with
and without the plugin and after a mode switch (test hashes the first message in all three
states). **Effective knobs, computed by the middleware**: `kernel.prior_knobs` is captured once
at install and never replaced; `Loader.install` sets `kernel.knob_bounds` from the installed
manifests and calls `policy.apply_offsets`, which recomputes from the prior plus the **latest**
`disposition.updated` per (plugin, knob), clamping each offset to its manifest bounds and
[-1, 1], summing, and clamping the total. A knob whose prior value is between 0 and 1 is
normalized and clamped to [0, 1]; another (`patience_seconds` 90, `speaking_bar` 4) only never
goes negative — clamping it to [0, 1] would have changed the gate, which the first measurement
showed and this note records. `/mode` (owner, permission `soul-tuning.mode`; a guest's call is
answered `/mode is not allowed for you`) shows `mode: daily (daily|professional)`, and on a
switch records `soul_tuning.mode_changed` plus one `disposition.updated` per knob and calls
`apply_offsets` at once; a second `/mode professional` changes nothing and `/mode daily` moves
the knob back. Uninstalling (folder and `config.d/` gone, reload) drops the block, the command
and the bounds, and the knobs return to the prior (`0.45` in the test).
**Deviation**: an event kind holds only lower-case words and dots, so the plugin's kind is
`soul_tuning.mode_changed` (the hyphen becomes an underscore); the payload still names the
plugin `soul-tuning` as the source. Ledger: with `soul-tuning` installed,
`talkativeness` = prior + 0.05 (daily) or − 0.05 (professional); two plugins' offsets sum;
removing the second returns prior + the remaining offset.
