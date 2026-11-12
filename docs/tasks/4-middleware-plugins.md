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

- [x] 1.0 **Follow-ups from Task 3** (its Notes). (a) A cold start never loses a percept: the boot warms what is slow (the embedding model, the database pool) before any adapter starts, so the first message — often the pairing code — passes its inbound hook inside the watchdog; test with a slow fake embedder. (b) Routes: when `to:` names someone with no direct place who is present in the current place, `identity.route` answers in the current place instead of refusing (scenario 24's "no way to reach tg:7001 yet"). (c) The context hook stays inside its watchdog on a long desk: measure `privacy.filter_context` on the largest scenario and record the time under Notes.
- [x] 1.1 **Eight files** (middleware.md §1), before any new feature: merge `owner_policy.py` and `limits.py` into `policy.py`; rename `tools.py` to `registry.py`; update the importers (graphify for callers first). Behavior unchanged: tests and fake replay pass as before. `loader.py`, `interaction.py` and `roles.py` are born below.
- [x] 1.2 **Format versions** (D58, project.md §2): `version:` in `config.yaml`, `config.d/*.yaml`, `soul.yaml`; the prior through `prior/manifest.yaml`'s `format_version`; manifests through `manifest_version`. Older is upcast in memory and named in the boot report (`config.yaml` 1 → 2: `plugins:` list → map), never rewritten; newer fails at boot; missing is read as 1 with a warning. Update the shipped `config.yaml` and `soul.yaml`.
- [x] 1.3 **Registry** (`registry.py`, middleware.md §4, §6): one registry for tools, commands and skills, each entry naming who uses it; identical `/help`; tool facts (`reads`, `changes`, `runs_code`, `network`, per-call `sensitive`) in `contract.py`'s vocabulary, assembled next to the descriptor, a tool without facts fails at boot; a turn is offered only what its requester may use; capability index behind `<self>` — core lines always present, one short line per capability, full schema on request (V1 ADR `engine/0015-capability-manifest`); every tool declares an output cap and cuts with a "continue from here" marker (V1 ADR `engine/0021-tool-output-contract`); skills: names and descriptions of relevant SKILL.md in the context, the body on request.
- [x] 1.4 **Loader** (`loader.py`, middleware.md §5, plugins.md §3–4): reads `config.yaml`, `config.d/` and manifests (V1 ADR `0028-plugin-model`: off until enabled, dependencies live with the plugin); `dependencies` load first, `optional_dependencies` when present; merge rules (maps merge, lists replace whole, `[]` off, warning when a list replaces a default); unknown key or wrong value fails at boot; a `config.d/` file for an absent plugin is reported and ignored; features switched off are removed; extension points opened and filled; collisions fail at boot; the files are the source of truth: `/plugin enable|disable|update` writes only its own lines (and creates `config.d/<name>.yaml` with `permissions` and `role_settings` from the manifest), after a backup, applied without a restart; plugin updates follow the dpkg rule (middleware.md §5); every change is recorded as an event for history only; `/plugin show <name>` prints defaults; `/reload` validates first and keeps the running configuration on any error, naming what needs a restart; uninstall removes nothing from the owner's files.
- [x] 1.5 **Adapters through the same loader** (adapters.md §2, D49): Telegram and CLI get a `create(options, env)` factory; `__main__.py` imports no adapter and asks the loader for the enabled ones; a test adapter from outside the package loads from `config.yaml` alone. Adapter contract from V1 ADR-0018 (`engine/0018-gateway-contract`): a small required part and optional parts, a missing capability is visible (never a silent `pass`), checked at boot; adapters declare tool facts and may offer place info (name, member count, platform role facts); one folder per adapter split by job (receive, send, format, what it can do).
- [x] 1.6 **Roles** (`roles.py`, middleware.md §8): four tiers; permissions and `role_settings` from manifests; tier roles and group roles from `config.yaml` with `label` and `context`; `allow`, `deny` (wins), `settings`, `approval`; permission tiers from `config.d/`; `fallback`; place overrides; wildcards `plugin.*` and `*` with their expansion in the boot report; `people:` keyed by person id (`label`, `roles`, `context`) as the only record of who holds which role, written by `/role add|remove` (one entry, backup, no restart, `roles.changed` event for history); `/whoami` (full in a direct chat; label, roles and place only in a shared place) and `/whois`; the middleware's own permissions (`config.reload`, `plugin.manage`, `roles.manage`, `identity.whois`, `approval.answer` at `admin`; `identity.whoami` at `guest`); no one gives a role at or above their own highest tier, owner-only actions are not movable; each role's and person's `context` on person cards within a cap; `/roles show <role>` with each line's source; empty configuration = owner free, everyone else only talks.
- [x] 1.7 **Policy** (`policy.py`, `privacy.py`): pass, ask or refuse from the requester's (`origin_actor`) permissions, the tool's facts and the approval mode (`yolo`, `default`, `strict`; default `default`; `sensitive` always asks); privacy rules that only narrow (confirm before forwarding a file, a mode that keeps a topic out); route hints to a person via lever 2; owner commands for budget and quiet hours.
- [x] 1.8 **Interaction** (`interaction.py`, middleware.md §9): the vocabulary in `contract.py` of what an adapter can show (typing, edit message, buttons, reactions); the defaults (thinking only to the owner in the owner's direct chat, tool steps only for long work, approvals ask the owner); a command to switch them; everything shown passes `privacy.py` first; fallbacks when an adapter lacks a capability; an approval answer comes back as an ordinary event. CLI and Telegram render typing, tool steps and an approval (buttons on Telegram, text on CLI); WhatsApp and Discord are Task 5.
- [x] 1.9 Read-only view of the log and state for plugins in `contract.py` (core.md §3, D43); the policy rule on data leaving the machine (middleware.md §3).
- [x] 1.10 **Attachments** (middleware.md §10): the vocabulary in `contract.py`; the store in `infra/` with `put` and `open`, `open` checked by `privacy.py` against the source event's audience; hard forgetting deletes the file; Telegram and CLI receive and send several attachments per message, declaring their limits; `interaction.py` splits when an adapter sends fewer per message. Port the mechanism of V1's `AttachmentStore` (ADR `engine/0004-core-tools-and-cross-gateway-messaging`).
- [x] 1.11 Check: private content never reaches someone outside its audience, even when someone tries to talk the soul into it, even with a plugin that tries to widen it, and even through a shown thought or tool step; a guest asking for a tool their roles lack is refused and the soul does not promise it; a reasonable request from a group member is helped; no plugin can remove the policy; `/help` is identical on Telegram and CLI; thinking never shows in a group; a new role with only `base: member` works in every plugin; an admin cannot give `admin` or raise a permission above their tier; `/whoami` in a group never shows linked accounts; three files in one message are received, read and forwarded, and a file from a private chat is not opened for someone else without `share`.

## Part 2 — Soul Tuning
Read first: [plugins.md](../plugins.md) §5.1, [middleware.md](../middleware.md) §2, §7
May touch: `plugins/soul-tuning/`, `config.d/`, `tests/`, `scenarios/`

- [x] 2.1 Modes (daily / professional): rules in `<self>`, knob offsets within bounds (recorded as `disposition.updated`; the middleware computes the effective knobs, middleware.md §2 lever 6), `/mode`.
- [x] 2.2 Example conversations in the stable prompt, in a fixed order.
- [x] 2.3 Check: installs, works and uninstalls without any core change; the stable prefix hash does not change when the mode is unchanged; only the owner changes it.

## Part 3 — Web Search
Read first: [plugins.md](../plugins.md) §5.2
May touch: `plugins/web-search/`, `config.d/`, `tests/`, `scenarios/`

- [x] 3.1 `web_search` with the provider chain (SearXNG, Tavily, Brave, Exa, Serper, Google, DDGS last), ported from V1 `monoagens/engine/tools/web_search.py` (ADR `engine/0026-search`): normalized results, structured failure reasons, keys from `.env`.
- [x] 3.2 `web_fetch` ported from V1 `monoagens/engine/tools/fetch.py`: internal addresses refused (resolve, check against the blocked set, connect to the checked address).
- [x] 3.3 Check: "who is the president now?" triggers a search when the plugin is on, and an honest "as of my knowledge" answer when it is off; `web_fetch` of `http://localhost`, a private address and a name resolving to one is refused.

## Part 4 — Workspace and PDF Reader
Read first: [plugins.md](../plugins.md) §5.3–5.4, [core.md](../core.md) §4
May touch: `plugins/workspace/`, `plugins/pdf-reader/`, `config.yaml`, `config.d/`, `tests/`, `scenarios/`; for 4.7 only: `monoagens/core/reasoning.py`, `monoagens/core/jobs.py`, `monoagens/core/kernel.py`, `prior/dispositions.yaml`, `.env.example`

Multi-step work runs in the core's task threads (Task 2 Part 3); the plugins only offer capabilities.

- [x] 4.1 Tools `read`, `write`, `edit`, `grep`, `glob` over the person's workspace and shared folders (`workspace.shared` on their roles); `read` also takes an adapter's attachment reference; the `reader` extension point.
- [x] 4.2 Sandbox for `run` (D60): the shared Debian slim base (prepared once), an overlay layer per person with the soft quota from `workspace.quota`, an empty `/tmp` per run, private `/proc` and `/dev`, nothing of the host visible; a computed environment without secrets (V1 ADR `engine/0024-tool-input-contract`); time, CPU, memory and process limits; capped output.
- [x] 4.3 Network through pasta: outbound allowed; host loopback, private ranges, metadata and the server's own addresses blocked; no inbound. Fail-closed: no pasta → no network, no bubblewrap or base → `run` unavailable, named in the boot report.
- [x] 4.4 The guard (ported from V1's two-stage guard) marks calls `sensitive`; approval follows the person's mode (1.7).
- [x] 4.5 PDF Reader fills `reader` for `.pdf` (pypdf); `read` of a type without a reader says so and names the extension.
- [x] 4.6 Scenario: "send our finished work to Budi" where the work is a file in the workspace: `read` gets it, the soul shares it and sends it through an adapter that can attach files (a test adapter until Task 5), and the forwarding rule of 1.7 applies.
- [x] 4.7 **Batch tool calls**: send `parallel_tool_calls: true` to the provider unless `LLM_PARALLEL_TOOL_CALLS=false` in `.env` (add it to `.env.example`). Scenario "grep three files, then sum them up": record wakes, model calls and tokens. If one batch wakes reasoning more than once, implement the core fix in this item: the results of one round are awaited together, keyed by that round (never a kernel-wide counter; a result from another place or thread never counts), until all arrived or a timeout from the prior (a new constant in `prior/dispositions.yaml`), and wake reasoning once; record the before/after numbers under Notes as the evidence (constitution §1).
- [x] 4.8 Check: from inside `run`, the host's home, `.env` and the database port are unreachable, another person's workspace and `/tmp` are invisible, a private address is unreachable, the internet is reachable; a guest cannot `run`; in `default` mode `run` asks the owner; "finish my code, then send it to Budi" runs as a task thread and keeps its promise.

## Part 5 — Capability targets carried over from Task 2
Read first: [core.md](../core.md) §4, §8 (D45–D48, D55, D56), the Notes of Task 2 Part 12
May touch: `scenarios/`, `scripts/replay.py`, `plugins/`, `monoagens/middleware/`, `tests/`

The core already has the mechanisms; these scenarios need middleware, plugins and real
capabilities around them. A core change here is only for a bug, reported first.

- [ ] 5.1 Scenario 18 (game adapter, Budi's file, sharing to Budi with consent, Fulan never shared): passes with the real model, its material checked (facts and capabilities).
- [ ] 5.2 Scenario 19 (task thread, delivery to Budi, ask which group): passes with the real model, with Workspace.
- [ ] 5.3 Scenarios 26 and 28 (a fact from the owner's direct chat: ask the owner, answer after `share.granted`): pass with the real model, with the withheld note (D56).

## Part 6 — Real-model test
- [x] 6.1 `uv run pytest` and the fake replay pass in full.
- [x] 6.2 Real-model replay and a live CLI session with the model in `.env` (never read or print it), with the four plugins on: a mode switches, a role's `context` reaches a person's card, a search runs, a Workspace task with `run` runs end to end with typing, tool steps and the approval shown. Report what happened and the metrics.

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

**Part 3 — Web Search (`plugins/web-search/`).** `providers.py` keeps the chain in the order the
options name (SearXNG, Tavily, Brave, Exa, Serper, Google, DDGS last by default), one function
per provider and one normalizer per payload shape (`normalize.py` → `{title, url, snippet,
published_at}`). A provider whose key or URL is missing is skipped with `missing_key`; an HTTP
error is `http_<status>`, a timeout `timeout`, a transport failure `transport_error`, any other
exception its class name; a success with no rows is `empty` and the chain goes on; every failure
is returned in `tried`, so `web_search` never crashes and reports exactly why nothing answered.
`ddgs` runs in a worker thread with three retries 1.5 s apart and reports `ddgs_unavailable`
when its package is missing (the plugin keeps its other providers; the manifest declares
`requires: [ddgs]` and the folder ships `requirements.txt`). `web_fetch` (`fetch.py`) runs in a
worker thread: only `http`/`https`, only the allowed ports, the host resolved and **every**
address checked against the middleware's one blocked table, then a pinned connection
(`_Pinned.connect` to the checked address, `Host` header and `server_hostname` the real name) so
DNS rebinding cannot slip through; at most three redirect hops, each parsed, resolved and
checked again (a hop to a private address is refused); only text types, ≤ 1 MB downloaded; the
page extracted with the stdlib parser (scripts, styles, templates dropped, whitespace
collapsed) behind a `fetched: <url> (text, untrusted)` header, and the injection marker when the
text reads like an instruction. Every refusal is a structured result, never a crash. Tests:
`tests/plugins/test_web_search.py` (chain order and reasons, each normalizer, the six refusals,
a name resolving to 192.168.1.10, a public page with its scripts dropped, the injection flag, a
redirect hop refused, a server error) and `tests/plugins/test_web_search_check.py`: with the
plugin a scripted `web-search.web_search` call records a `model.call` and one `act.out`; without
it the capability is absent and the honest "as of my knowledge" reply is the only output.

**Part 4 — Workspace, sandbox and PDF Reader.** `plugins/workspace/`: `plugin.yaml` exactly as
planned (facts and permissions per tool, `role_settings` with the quota and the shared lists,
`extension_points: {opens: [reader]}`), `paths.py` (own workspace `<root>/<person id with ":"→
"_">`, `shared/<name>` for each folder a role names; `resolve` refuses `..`, an absolute path
and a symlink that leaves the folder), `tools.py` (read with an `offset`/`limit` window or an
attachment `ref`, write, edit with exactly-one-match, grep in `matches|count|files` mode sorted
by path and line, glob, run), `guard.py`, `sandbox.py`, `base.py`, `quota.py`, `prepare.py`.
`plugins/pdf-reader/`: `dependencies: [workspace]`, `requires: [pypdf]`, fills `reader` for
`.pdf` with `pages: N` and the text of every page; with pypdf absent the plugin is "unavailable"
(boot note) and `read` of a `.pdf` answers `no reader for .pdf: install one for this file type`.
Every tool answers a refusal as a value (`{"error": …}`), never a crash.
**1. Files.** `read` also reads one attachment through the one door and the `reader` point by
suffix; a suffix with no reader names the extension. A legacy text extension or no suffix reads
as UTF-8; a file with control bytes is `that file is not text`. Ownership: the workspace of
`person:owner` is `var/workspaces/person_owner`, so another person's files are not reachable even
by name.
**2. Sandbox.** `run` (long_running, so a task thread opens and typing shows) is composed by
`sandbox.argv`: pasta with `--config-net --no-map-gw -t none -u none -T none -U none`, then a
shell wrapper that adds the blackhole routes (the blocked table plus this machine's own
addresses) and execs bubblewrap with `--die-with-parent --unshare-all --share-net
--new-session --hostname sandbox --clearenv`, an overlay on the shared base (one layer per
person under `var/sandbox/layers/<person>`), `--bind <own workspace> /work`, a shared folder
`--ro-bind` unless the person holds `workspace.write`, `--tmpfs /tmp --tmpfs /run --proc /proc
--dev /dev`, `--ro-bind /etc/resolv.conf`, a fixed PATH/HOME/LANG/TERM, `--cap-drop ALL` and
`bash -lc`. Limits: `RLIMIT_AS`, `RLIMIT_CPU`, `RLIMIT_NPROC`, `RLIMIT_CORE` in a preexec hook,
`start_new_session`, `wait_for(timeout)` then `killpg(SIGKILL)`; output capped at 8000. The
environment is computed, never inherited (no `LLM_API_KEY`, no `DATABASE_URL`). Fail-closed: no
pasta → `--unshare-net` (no network at all); no bubblewrap, no `ip` or no prepared base → `run`
answers `workspace: run unavailable (<reason>)` and the boot report/rule says why.
**Preparation is a command, not a boot step** (`uv run python plugins/workspace/prepare.py
--root .`): boot must not wait minutes on a download, and `run` stays unavailable until the
marker `base.json` exists. The base comes from `base_rootfs` when given, else `podman create` +
`export` (or docker), else `base_tarball`; the tree is extracted with absolute names, `..`,
links and devices refused, moved into place with `os.replace`, then the one install pass runs
python3/pip/venv, git, curl, CA certificates and iproute2 inside the sandbox with the base
writable.
**4. Guard** (`guard.screen`) is deterministic, two stages: the deny list (container runtime,
mount/namespace, firewall, download-and-execute, `nc -e`, `/dev/tcp/`, decode-and-execute,
`chmod 777 /`, `chown /`, sudo/su, user management, cron/systemd, `kill -9 1`), then the hidden
forms (`$IFS`, `{a,b}`, `$(`, backticks), the allow list of plainly safe heads and a recursive
`shlex` walk that peels shell wrappers and re-screens what they run. `deny` → the act is
refused (`sandbox command blocked: <reason>`, nothing sent); `pending` (a wide or hidden
command) → `sensitive` with reason `the command is not plainly safe`, so the owner is asked even
in `yolo`. **No model call**: a classifier here would bypass the gate and the daily budget
(invariant 5), so stage 2 is deterministic — recorded as the deliberate deviation the plan
allowed.
**4.6 Scenario** (`tests/plugins/test_workspace_scenario.py`): the owner asks to send the
finished work to Budi; the scripted soul reads `laporan.md` from the workspace, shares it with
Budi and sends it through a test adapter that can attach files (`filetest`, 2 per message). The
first send is refused with `waiting for the owner's approval` and `approval.requested` carries
reason `forwarding a file`; after the owner's `yes` one `intent` goes to Budi's place, the send
is carried out there and the recorded `act.out` carries the file's ref. `share.granted` comes
before any send.
**4.7 Batch tool calls.** `LLM_PARALLEL_TOOL_CALLS` (default true; `false`/`0` disables) reaches
`Reasoning.parallel_tool_calls` and the request body (`parallel_tool_calls: true`), named in
`.env.example`. Measurement with a scripted model and the real Workspace tool over three files,
one round of three `workspace.grep` calls: **before the core fix — 3 wakes, 3 model calls, 5
`act.out`; after — 2 wakes, 2 model calls, 4 `act.out`** (one round, then exactly one follow-up
round; the second `act.out` was the duplicated answer). The fix, only in `core/kernel.py` (no
other core file needed): `Kernel._finish_job` records one entry per round keyed by
`(job.trigger.id, job.place)` with the number of results it awaits and a deadline from the new
prior constant `batch_wake_timeout_ms: 2000`; `Kernel._wake` decrements that entry for an
`act.result` (finding the round through `act.out.caused_by`) and returns without appraising
while results are still due; the last one wakes once. A round with a slow
(`long_running`) act or a message (`communicative`) act is never held by it: those wake as they
always did. A late result is flushed by `_flush_batches` when a tick or any event of that place
arrives after the deadline, so nothing can stall a place. `tests/core/test_batch.py` covers one
round/one wake, two places never cross-counting, the timeout flush, an unknown capability not
being counted, and a round with a slow act.
**4.8 Checks** (`tests/plugins/test_workspace_checks.py`): `run` asks the owner in `default`,
passes in `yolo` and asks in `strict`; a guest is not offered `workspace.run` and is refused by
name; "finish my code, then send it to Budi" opens a task thread, runs and delivers. The sandbox
end-to-end checks (host home, `.env`, the database port unreachable; another person's workspace
and the host `/tmp` invisible; the internet reachable) are marked
`skipif(not MONOAGENS_SANDBOX_E2E)`: they need a prepared base, which needs an image export or a
download; the machine has `bwrap 0.13.1`, `pasta`, `ip`, `podman` and `docker`, but the base was
not prepared in this session, so `run` is reported unavailable and the e2e checks were not run.
Note for the owner: `RLIMIT_NPROC` counts the host user's processes, so a busy host can hit it
before the sandbox limit; time, memory and CPU still bound the run and bwrap's PID namespace
isolates the process tree.

**4.7 follow-up.** A round's count must include only calls that really produce a result:
counting a call the middleware handles itself (`self.share` records `share.granted` and
returns nothing) held the round open and its timeout flush woke the soul in a place where
nothing had happened (scenario 18 grew a spurious "ok"). `ToolEntry` now carries
`result: bool` (`False` for `self.share`, declared through `add_capability(..., result=False)`),
and `_track_batch` skips an unknown capability, a slow tool, a message and a handled
capability. Scenario 18 is back to 19/19.

**Part 5 support.** `scripts/replay.py` reads two optional keys per scenario: `plugins:`
(a map of name → settings; the scenario's selection replaces `config.yaml`'s for that run) and
`bodies_extra:` (extra adapters through `module:`), and builds the kernel through the same
`Loader` (the repo's plugin folders, never the repo's `config.yaml`). A scenario without them
behaves exactly as before (the five `FakeBody`s).

**Part 5 — scenarios.** Scenario 18 uses `game.say`, `whatsapp.say` and `self.share` (no `dev`
capability), so its material already matches the plan's mechanism and needed no change; 19 uses
the replay's `dev.run` organ because the Workspace sandbox is not prepared on this machine (a
`workspace.run` there would only prove the "run unavailable" message). Scenarios 26 and 28 now
declare `plugins: {soul-tuning: {enabled: true}}`, so the withheld-note path is exercised with a
plugin and a role book present. The fake replay stays `133/133` with those changes.
The real-model runs of 18, 19, 26 and 28 are reported in Part 6 below.

**Part 6 — real model.** Model `deepseek-ai/DeepSeek-V4.1-Flash-1017` from `.env` (never read or
printed here), effort `high`, report `reports/2026-11-12-1859-real-18-19-26-28.md`.
- **6.1**: `uv run pytest` → **530 passed, 1 skipped** (the sandbox e2e, `MONOAGENS_SANDBOX_E2E`
  not set); `uv run python scripts/replay.py --fake` → **133/133**, absolute-line violations 0.
- **6.2 real replay** (the four scenarios Part 5 names): `checks: 27/58 passed`, failures 31,
  25+65+9+9 LLM calls, tokens prompt 320 882, cached 244 608, completion 57 160 (reasoning
  42 923), total 378 042. Per scenario: `18_game_world_file_and_share` 8/19, `19_task_thread_two_
  groups` 12/21, `26_help_meeting_time` 3/9, `28_help_package_location` 4/9 — all four **FAIL**.
  Metrics: missed-help 11.8 % (2 of 17), false-speak 0 %, absolute-line violations 0, honesty
  100 % (15 of 15). The failures are the soul's, not the plugin layer's: it answered in the
  shared world where the material says to move to the owner's private channel, it sent the work
  to Budi's place without calling `self.share` first (no `share.granted`, and one reply claims
  the delivery), it held no wait expectation after asking Budi, and in 19 it wrote a "sent to
  Budi" line while no share was recorded. 26 and 28 (a fact from the owner's direct chat) show
  the withheld note is delivered but the soul asks/answers before the `share.granted` the
  scenario expects. These are recorded as failing capabilities, not hidden; the middleware
  itself kept every absolute line (0 violations) and the honesty scan stayed at 100 %.
- **6.2 live CLI session** (`uv run python -m monoagens` with the CLI body and the four plugins,
  driven by a socket client; Telegram off in the session because `TELEGRAM_BOT_TOKEN` is empty
  here, restored afterwards). Boot report printed:
  `workspace: run unavailable (no sandbox base (run plugins/workspace/prepare.py --root .))`,
  `adapter cli: shows typing`, `adapter telegram: off`. `/mode professional` → `mode:
  professional`; `/whoami` → `you: person:owner (owner)`, `roles: unverified (Unverified); tier
  owner; approval default`, `place: space:cli — direct, cli`; "cari berita terbaru tentang
  Jakarta" → `[typing…]` then a cited answer (ddgs, after `uv pip install ddgs pypdf`; eight
  Jakarta items with sources) and a follow-up round that said no newer item was found; "halo,
  apa kabar?" → `Halo! Baik kok, siap kerja. Kamu gimana?`. A real Workspace task with `run`
  was not possible (no sandbox base); `workspace.run` answers `workspace: run unavailable (...)`
  as designed, so the "typing, tool steps, approval" part of 6.2 was exercised by 4.8's pytest
  checks instead.
**Deviation found by the live session and fixed**: the CLI adapter never turned a typed `/…` or
`:…` line into a `command` event (only Telegram did), so `/mode` and `/whoami` reached the model
as text. `cli.format.command_event` now does it and `tests/adapters/test_cli.py` covers it; the
transcript above is from the run after the fix.

**Sample `/roles show member`** (a book built from the shipped manifests):
```
help: guest (middleware)
identity.whoami: guest (middleware)
privacy.share: guest (middleware)
registry.describe: guest (middleware)
self.recall: guest (middleware)
web-search.fetch: guest (manifest)
web-search.search: guest (manifest)
workspace.read: guest (manifest)
workspace.write: member (manifest)
workspace.quota: 1G (manifest)
```
**Sample `/whoami`** in the live session: `you: person:owner (owner)`, `roles: unverified
(Unverified); tier owner; approval default`, `place: space:cli — direct, cli`. In a group the
accounts line is absent (or sent to the asker's direct chat), as the group test shows.

### 4.8 for real — the sandbox tested against this machine (2026-11-13)

Setup: Podman Postgres from the repo's `compose.yaml` (`POSTGRES_PORT=5434`, container
`monoagens-sbx-db`, image `docker.io/pgvector/pgvector:pg17`, network `monoagens-sbx_default`
`10.89.11.0/24`, container IP `10.89.11.3`), host LAN `192.168.0.120`. From the host a real
client reached `127.0.0.1:5434` and `192.168.0.120:5434` (`psql` is not installed; a psycopg
connection authenticated and reported `inet_server_addr() = 10.89.11.3`). The **container IP
`:5432` is closed from the host too** (rootless pasta does not route container addresses), so
that row cannot distinguish the sandbox from rootless networking; the published-port paths
are open from the host and closed inside, which is what the check needs. Base:
`plugins/workspace/prepare.py --root .` → `var/sandbox/base`, source
`podman debian:bookworm-slim`, **248 MB**, marker `var/sandbox/base.json`. Canary: a 24-char
random file `~/.monoagens-canary` and `MONOAGENS_CANARY` in the launching process; both absent
from every sandbox view; removed afterwards. Two people (`person:owner`, `person:test:other`
with `secret.txt` in their own workspace) and a shared `toko-a` folder. Harness:
`tests/sandbox_e2e_report.py` (every check goes through the real kernel: role book →
`workspace.run` offered or not → policy pass/ask/refuse → guard → real sandbox);
`tests/sandbox_child.py` starts a run for the SIGKILL check.

**Result: 44/44 checks passed** (elapsed 64 s). Highlights, exact output trimmed:
- `id; whoami` → `uid=1000 gid=1000 groups=1000,65534(nogroup)`; `cat /etc/os-release` →
  `Debian GNU/Linux 12 (bookworm)`.
- `ls -la /home /root` → the base's empty `/home` and `/root`; `cat ~/.monoagens-canary` →
  `No such file`; `find / -name .env` → nothing.
- `env` → `PWD=/work HOME=/work LANG=C.UTF-8 TERM=dumb SHLVL=0 PATH=…` only.
- `/proc/1/environ` → `PWD=/ HOME=/work LANG C.UTF-8 TMPDIR=/tmp TERM=dumb SHLVL=0 PATH=…`,
  process count 4.
- `cat ../person_test_other/secret.txt`, the absolute host path, a symlink in `/work` and a
  symlink to `/etc/shadow` → all `No such file or directory`.
- `echo x > /usr/bin/x; echo x > /etc/x` → present inside, and **absent from the base on the
  host afterwards**.
- `mount`/`unshare`/`sudo`/`su` → `sandbox command blocked: mount or namespace`; with the guard
  bypassed the kernel refuses them (`Operation not permitted`).
- `curl -sSI https://example.com` → `HTTP/2 200`; `pip download requests` → `Saved …whl`.
- `127.0.0.1:5434`, `localhost:5434`, `192.168.0.120:5434`, `10.89.11.3:5432`,
  `169.254.169.254`, `10.0.0.1`, `172.16.0.1`, `192.168.1.1`, `192.168.0.120:22`,
  `localtest.me:5434`, `[::1]:5434`, `[fd00::1]`, `[fe80::1]` → `curl: (7) Failed to connect`
  (`rc=7`); `ip route del blackhole 10.0.0.0/8` → `RTNETLINK answers: Operation not permitted`
  and the routes unchanged; `/dev/tcp/127.0.0.1/5434` → `sandbox command blocked: raw socket`;
  a server started inside is unreachable from the host (`curl: (7) … 127.0.0.1:8099`).
- limits: fork loop → `timeout after 5 seconds` (the host stayed responsive), `x=' '*(4 GiB)` →
  `MemoryError`, `sleep 999` → `timeout after 5 seconds`, `yes | head -c 50 MB` → cut with
  `contract.OUTPUT_MARKER`, `dd 3 MB` into the layer → the next run
  `workspace quota exceeded: 3145732 of 1M; /workspace reset`, `/workspace reset` → `3 files
  removed` and the next run works.
- SIGKILL of the process that started a `sleep 999`: `bwrap before=5 after=0, pasta after=0`.
- guard: `curl … | sh` → `blocked: download and execute`; `nc -e` → `blocked: nc -e`;
  `rm -rf /` → `waiting for the owner's approval` (marked sensitive). A guest
  (`person:test:other`) → `you may not use workspace.run` and `workspace.run` is not offered;
  a member (base trusted) in `default` mode → `waiting for the owner's approval`.
- fail-closed: with pasta off `PATH` the boot report says `workspace: network off (no pasta)`
  and `curl https://example.com` inside fails (`Could not resolve host`); with the base moved
  away `run` answers `workspace: run unavailable (no sandbox base (run
  plugins/workspace/prepare.py --root .))`.

**Defects found and fixed in `plugins/workspace/`** (all found by this test, none by review):
1. `prepare.py` could not import its own package (relative imports) → it loads the plugin the
   way the loader does.
2. `base.extract` skipped every symlink, so the rootfs had no `/bin/sh`; and skipping
   *absolute* symlinks removed the dynamic loader (`/lib64/ld-linux-…`). Symlinks are now
   created explicitly (never followed while extracting, targets validated), hard links only
   when inside and already extracted, devices skipped.
3. Rootless apt cannot switch its sandbox user (`setgroups` is denied in a user namespace) →
   `APT::Sandbox::User=root` for the one install pass.
4. No resolver inside: the host uses a systemd-resolved stub no sandbox network can reach. The
   sandbox now gets its own `resolv.conf` with public resolvers through pasta
   (`--dns-forward 169.254.1.1` also stops pasta's warning polluting every run's output).
5. `RLIMIT_NPROC` counts the host user's **tasks** (processes and threads); 1 577 threads on
   this desktop made the namespace clone fail with `EAGAIN`. The limit is now the user's
   current tasks plus the sandbox allowance.
6. `argv(network=False)` built `bash -lc --unshare-net …` (a bash syntax error): the flag now
   goes where it belongs, inside the bwrap arguments.
7. The sandbox did **not** die with MonoAgens on SIGKILL: pasta resets the kernel's
   parent-death signal. A watchdog in the wrapper polls the MonoAgens pid (and stops when the
   run ends); `pasta --no-pidns` keeps that watchdog in the host PID namespace.
8. `quota.reset` failed on the mode-000 `work` directory bubblewrap leaves in a layer; it
   chmods its own layer before removing.
9. The guard read a whole shell line as one command: `id; whoami` became "not plainly safe"
   (an approval for a read-only command). It now tokenises with `shlex` (quotes and newlines
   inside `python3 -c '…'` stay whole, `2>&1` is not a separator) and screens each segment,
   with the wider safe list the checks needed (`ip`, `id`, `env`, `dd`, `yes`, …) and `curl`
   no longer "sensitive" (a plain fetch is what the sandbox is for; `curl … | sh` stays
   denied).
10. `/proc/1/environ` carried the host's `PWD` (and `OLDPWD`): the wrapper starts with
    `cd / ; unset OLDPWD` and bubblewrap sets `PWD=/work` for the command.
11. The tool pre-sliced output to 8 000 bytes *before* the registry's cap, so the marker never
    appeared; the sandbox now bounds at 200 000 and the registry cuts with the marker.

Suite after these fixes: **530 passed, 1 skipped**; fake replay **133/133**; the base on the
host is unchanged by the overlay writes (`/usr/bin/x` and `/etc/x` absent).
