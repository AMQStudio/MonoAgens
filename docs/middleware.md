# Middleware — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. The foundation in the middle
The three layers ([constitution §4](constitution.md), D57):
- **Core** — neutral and primitive, only a brain. It knows only opaque ids, labels and numbers;
  it restricts nothing itself.
- **Middleware** — the regulator, like drivers. It decides, it is the only path between the core
  and everything else, and it may limit or add what reaches the core.
- **Adapters and plugins** — talk only to the middleware: they give it data and ask it for data.

The middleware runs in the core's three hooks (inbound, context, action) and offers one API
(§2). It is static and never learns — learning belongs to the soul (D34).

```
   Plugins (Soul Tuning, Web Search, Workspace…)  Adapters (Telegram, WhatsApp, robot…)
                   \                                  /
                    ──────────── Middleware ──────────
  contract · policy · privacy · identity · registry · loader · interaction · roles
                                       │  three hooks
                       Core (ids, labels, numbers — mechanisms only)
```

Eight files, one concept each (decided 2026-11-12, D59). A file that grows becomes a folder of
the same name (`registry/tools.py`, …); the eight concepts stay.

| Module | File | What it provides | When |
|---|---|---|---|
| [Contract](#2-the-api-six-levers-d41) | `contract.py` | the rules of the game for every plugin and adapter: hook outcomes (pass, replace, stop, annotate), the read-only view of the log and state, the vocabulary of tool facts (§4), of what an adapter can show (§9) and of attachments (§10) | Task 3; view and vocabulary in Task 4 |
| [Policy](#3-policy) | `policy.py` | the owner policy, first and last in every hook: the identity line, daily model budget, quiet hours; whether a tool call passes, asks for approval or is refused (roles + tool facts + approval mode, §8); the owner commands that change these settings | Task 3 (as `owner_policy.py` + `limits.py`, merged in Task 4) |
| [Privacy](#3-policy) | `privacy.py` | absolute line 2: audience labels, what a context may hold, the withheld note, `share`, data leaving the machine, stricter rules from plugins | Task 3 |
| [Identity](#7-identity-account-map-owner-person-cards) | `identity.py` | account map (one person id per person, registered automatically), pairing the owner, person cards, place info, routes (`to:`); linking as a merge | Task 3; linking in Task 5 |
| [Registry](#4-registry-tools-commands-skills) | `registry.py` | tools (used by the soul), commands (typed by people), skills (read by the soul): one list of names and short descriptions, the full entry on request, collisions; assembles tool facts; offers a turn only what its requester may use; `/help` | Task 3 (as `tools.py`); commands and skills in Task 4 |
| [Loader](#5-install-plugin-manager) | `loader.py` | reads `config.yaml`, `config.d/` and manifests (with their format versions), loads adapters and plugins in dependency order, collisions and the fail-closed check at boot | Task 4 |
| [Interaction](#9-interaction) | `interaction.py` | what the soul shows people besides its messages (typing, tool steps, thinking) and approvals asked and answered: what, to whom, how much | Task 4; rendering per adapter in Task 5 |
| [Roles](#8-roles-and-permissions) | `roles.py` | what each person may do: roles with a base tier, permissions declared by plugins, settings per role, approval mode per role; always present, empty by default (owner free, everyone else only talks) | Task 4 |

**What the middleware itself offers.** Only commands and tools that regulate the middleware:
`share` (privacy), pairing, `/link` and `/unlink` (identity), budget and quiet-hour settings and
approval answers (policy), `/help`, `/plugin` (registry, loader), `/reload` (loader),
`/whoami` and `/whois` (identity), switching what is shown (interaction), `/role` and
`/roles show` (roles). They are gated by permissions like any tool (§8). Anything
that is a feature or a capability (search, files, `/mode`) is a plugin. Test: if
removing it would leave a rule that can no longer be kept or changed, it belongs to the
middleware; if the soul would only be less able, it is a plugin.

## 2. The API: six levers (D41)
Every plugin and adapter uses the same levers, so the core never knows the case — modes ("daily" vs
"professional"), a persona layer, simulated emotions, RBAC and moderation all fit:
1. **Pass, replace or stop** an item at a door (percepts, the context, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to reasoning, inside its token budget, before the
   policy's last pass.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember and observe**: write only by putting events (state is a projection under
   `<plugin>:*`, so replay and hard forgetting still work); read the log and state read-only,
   for dashboards, debugging or metric exporters (D43). The view returns raw content: a tool
   that shows it to people passes it through `privacy.visible` for the requester's place first.
5. **Sense and act** through its own tools (a timer, a `set_mode` capability, a search tool).
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated` with the
   plugin as source; learning never moves an offset, and uninstalling removes it. The middleware
   (not the plugin) computes the effective knobs: the prior's value (later the learned value)
   plus every active plugin offset (the latest per plugin and knob), clamped to the manifest's
   bounds and [0, 1], recomputed at boot, reload and on every new offset, never from a snapshot.

Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, or writes the
core books except through events. The core's "no emotions" non-goal is about the core; a plugin
may simulate them.

**Order in each hook** (D9): `[policy — first] → [middleware modules] → [plugin middleware, in
install order] → [policy — last]`. Scope is **shared** (every adapter that opts in) or **adapter-own**
(unique to one platform). Name collisions fail at boot.

## 3. Policy
The one middleware that is always present.
- Wraps every module and plugin, first and last, and cannot be removed (D9).
- Enforces the two absolute lines deterministically: the `identity_changing` check on
  `origin_actor` in the action hook, and the private-content rules below in the context and
  action hooks (D24, D32.3–4). The core only carries the ids and labels these rules read and
  refuses to run without this policy (fail-closed, D57).
- **Private content (absolute line 2, D45).** Privacy follows people, not places. In the inbound
  hook the policy labels every line with its audience: the people present where it was said,
  after the account map, fixed at that moment (D50), so a newcomer never receives what was said
  before they arrived. Whether a place is direct or shared comes from the adapter
  (`place_kind`), never from the shape of an id (D48). A private item may enter a context only if
  every person who will see the result is in its audience, or the context is the owner's direct
  chat; content from places that are not private flows freely. One source of truth: the
  audience labels. Every privacy check (the context filter, `leaks` in the action hook,
  `visible` for attachments and shown items) answers from them, so content visible at the
  target is never refused as a leak; a text-similarity check only catches content that is not. A memory's audience comes from the
  labels of the events it was derived from (D53). When the policy withholds relevant items it
  leaves one line without content — how many, and whose private place they came from (D56).
- **Sharing by consent (D46).** The policy offers the `share(item | episode, to)` capability: it
  passes only when the chain's `origin_actor` is in the item's audience, and records
  `share.granted`, after which the new person is in the audience. Each person speaks only for
  what they were part of: the owner can share work done with the soul, not what Fulan said in
  private; the owner's view of everything is a view, not ownership.
- **Asking on someone's behalf (D73).** When the soul sends a message on behalf of a person X
  (the chain's `origin_actor` is X: "ask Budi whether it fits"), the reply that answers it is
  said to X as well: at intake the policy adds X to that reply's audience. The reply is the
  percept that matches that chain's `wait` expectation or replies to that message (`reply_to`);
  only that one reply, never the person's later lines, which keep their own audience. X is then
  part of it and may share it (`share`), and the soul may offer the choice where X is ("Budi
  answered; tell it here or in private?"). Nothing is shared without that explicit act; the
  answer is never said in a shared place before it.
- **Data leaving the machine** (D43): by default nothing does. Data goes out only through a
  plugin the owner installed and configured for it. Such a plugin sends numbers (counts,
  tokens, timings, decisions), not conversation content; sending content outside counts as
  showing it to people who were not part of it and is held to absolute line 2.
- **Privacy rules narrow, never widen.** Other middleware and plugins may only add stricter rules (a mode that keeps health topics out even of the
  person's own chat, an organization rule that a past meeting stays with its board); it can
  never let private content reach someone the owner policy would refuse. It may also ask for more than
  consent before sharing (confirm before forwarding any file; internal documents never go
  outside the organization) (D46).
- **Routes** (D46, D57): the action hook resolves a target that is not a place (`to:`,
  optionally `via:`) to a place — for a person, their direct place on the named adapter, or by
  default the adapter where they were last active — or refuses with its reason; the core only
  delivers to a place. A route rule may change the default (email for work, WhatsApp in the
  evening).
- **Default owner policy**, active out of the box: the two absolute lines, the daily model
  budget, quiet hours. Budget and quiet hours are owner settings in `config.yaml`, changed by
  hand or in chat with `/budget` and `/quiet`, which write their own line (§5); the owner may
  tighten or loosen them;
  plugins may add rules inside the policy, never remove it (D22, D24, D36).

## 4. Registry (tools, commands, skills)

Tools, commands and installed skills follow one pattern, as MCP's tools, prompts and resources
do: a list of names with short descriptions, the full entry on request, one namespace with
collisions failing at boot. Every entry names **who uses it**: the soul (a tool, a skill) or a
person (a command).

- **Commands are defined once** in a shared registry. Adapters only translate native syntax
  (`/help@bot`, `/help`, `:help`) into a neutral `command` event and may add presentation (menus,
  buttons) but never logic. A command is handled in the inbound hook without a model call; the
  core only records it. `/help` is the same everywhere; plugins register commands into the same
  registry (D9).
- **Tool registry**: the capabilities of every adapter and plugin present, sorted. Through the
  context hook it offers a turn the tools it may use (the core puts them in `<self>`); in the
  action hook it routes a call to the adapter or plugin that owns it. The core keeps only the
  dispatch door: check the arguments against the descriptor, record, hand to the action hook,
  record the result.
- **Tool facts.** Every tool states facts about itself in the vocabulary of `contract.py`:
  `reads`, `changes` (writes or deletes something), `runs_code`, `network` (reaches the
  internet). A plugin states them in its manifest, an adapter when it registers its
  capabilities; the registry assembles them next to the descriptor, so the core's descriptor
  never changes. A tool without facts fails at boot, naming it. A tool may add the fact
  `sensitive` to one call (the Workspace guard does, plugins.md §5).
- **Every entry declares a permission** (D59). Only an adapter's talking capabilities (say,
  react, and the like) may have none; every other tool and every command of a plugin or of the
  middleware names one, and a missing one fails at boot. A permission without a `base` is the
  owner's only; there is no "open to everyone" by omission.
- **One output marker** (`contract.OUTPUT_MARKER`) for every cut output.
- **Nothing is lost when an output is cut** (D62). The full output goes to the attachment store
  (§10) with the `act.result` as its origin, so its audience and hard forgetting follow the
  chain; the result carries the cut text plus `full: {ref, size}`, and the marker names the
  ref and the offset to continue from. The middleware capability `registry.more(ref, offset,
  limit)` reads the rest of any tool's output, through the same privacy check as an
  attachment. `<doing>`'s pointers to older results can therefore always be opened again: the
  view is compressed, the record is not.
- **Offered per requester.** In the context hook the registry offers a turn only the tools the
  requester may use (`roles.py`, §8), so the soul never promises what it cannot do. In the action
  hook `policy.py` decides pass, ask or refuse from the requester's permissions, the tool's
  facts and the requester's approval mode (§8). The requester is the chain's `origin_actor`, not
  whoever is present. How an approval is asked and answered is `interaction.py` (§9).
- **Brakes on runaway work** (D63), in `policy.py`, deterministic, no model call: at most
  `max_task_steps` steps per task chain and at most `max_wakes_per_hour` wakes per chain;
  both in `config.yaml` → `owner_settings`. When a brake holds, the chain stops, the event is
  recorded, and the owner is told in the owner's direct chat which work stopped and why, with
  the choice to continue; nothing stops silently. A brake is a fuse: the root cause of a loop
  is still fixed where it lives.
- **Draft**: per-person rate limits live here.

## 5. Install (plugin manager)
- A loader outside the core reads each plugin's manifest ([plugins.md §3](plugins.md)) and
  assembles its tools, middleware, commands, skills, rules, knob-offset bounds and state
  namespace; the core sees only percepts, actions and the three hooks (D9, D41, D57). It
  refuses to start the kernel without the owner policy first and last in every hook.
- Name collisions (two plugins claiming the same command, tool or state namespace) fail at boot
  (D9, D41).
- A manifest may name `dependencies`: other plugins that must load first (as Home Assistant's
  integration manifest does). A missing or failing dependency fails at boot, naming both.
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model).
  The owner policy holds against plugins that follow the contract; it is not a sandbox against
  malicious code, and the docs say so (D32.11).
- The installed plugins are listed in `config.yaml`; each plugin's own settings live in
  `config.d/<plugin>.yaml`, holding only what the owner changed; defaults live in the plugin's
  manifest ([plugins.md §3–4](plugins.md), [project.md §2](project.md)). Off until enabled.
- **Format versions** (D58): every YAML file carries its format version. An older version is
  upcast in memory and the boot report names it; the file is never rewritten. A newer version
  than supported fails at boot. A file without `version:` is read as version 1 with a warning.
- **The files are the source of truth** (D59): no copy in the database, nothing to keep in
  sync. The program writes them only on the owner's command, narrowly, after a backup, and
  applies the change without a restart; every other line is left as it is:
  - `/plugin enable <name>`: `plugins.<name>.enabled: true` in `config.yaml`, and, if it does
    not exist yet, `config.d/<name>.yaml` with the manifest's `permissions` and
    `role_settings`, marked with the plugin's version;
  - `/plugin disable <name>`: `enabled: false` only;
  - `/plugin update <name>`: the lines still equal to the old defaults (below);
  - `/role add|remove <person> <role>`: `people.<id>.roles` only (`roles.py`, §8).
  Uninstalling removes nothing from the owner's files; the boot report names what is unused.
  `/plugin show <name>` prints a plugin's defaults as YAML.
- **`/reload`** re-reads `config.yaml`, `config.d/` and the manifests without a restart (the
  commands above call it after writing). It validates first and applies only if everything is
  valid; otherwise it refuses with a clear message and the running configuration stays (boot,
  in contrast, fails). It names what needs a restart (new plugin code, a new sandbox base) and
  records what changed as history events.
- Every owner setting follows this rule: `/budget`, `/quiet` and the interaction switches
  (§9) write their own line in `config.yaml`; none is kept as an event that overrides the file.
- **History, not source**: every change, by command or by hand (seen at boot or reload), is
  recorded as an event (`plugin.config_changed`, `roles.changed`: who, from what, to what).
- **Plugin updates, the dpkg way**: the values `enable` wrote are recorded. When an update
  changes a default, the boot report compares the old default, the file and the new default:
  a line never touched is offered for `/plugin update`; a line the owner changed is only
  reported, never changed.
- Adapters are loaded by the same module, from the `bodies:` section of `config.yaml` (the key
  keeps its name), through each adapter's `create(options, env)` factory ([adapters.md §2](adapters.md), D49).
- Uninstalling removes a plugin's tools, middleware, rules and knob offsets; its events stay in
  the log as history.
- **Draft**: installed by the owner by listing it in `config.yaml` or with a command; a local
  catalog; semantic versions; stable, versioned contracts only after they have not changed for a
  while.

## 6. Skills
Part of the registry (§4).
- **Installed skills** are static SKILL.md files shipped by plugins. Only the names and
  descriptions of relevant skills enter the prompt via the context hook; the body is loaded on
  request; versioned; use counts feed the same ACT-R activation as memories (D11, D34).
- Skills do not learn; what the soul learns by itself is a core `procedure` or `lesson` memory,
  never a skill (D34).

**Format (SKILL.md).** YAML front matter, then the instructions in Markdown:

```markdown
---
name: shop-customer-service
description: Answer customer questions for a small shop politely and accurately.
version: 1
---
Steps …
```

## 7. Identity (account map, owner, person cards)
**One person, one id** (D65). The core never sees an account, only a person id (D10, D57).
- **The account map** (`state: alias:<account>` → person id): the inbound hook maps every native
  account an adapter reports (`telegram:123`, `whatsapp:628…`) to a person id, in `who` and in
  `data.mentions` / `data.reply_to.who`. No account is an alias of another account.
- **Registration is automatic.** An account seen for the first time — as sender, mentioned, or
  the author of a quoted message — gets a new opaque person id (`person:p_7f3a`) on the spot,
  recorded as `identity.registered {account, person}`. Members of a group who never appear in a
  percept are not registered. One account has one id (a unique constraint), so two percepts
  racing cannot create two. The owner's id is always `person:owner`.
- **Linking is a merge.** Linking two accounts of one person merges their two person ids into
  the **older** one (`person:owner` always wins): every account of the younger id now maps to the
  older, recorded as `identity.merged {from, into}`, and the middleware puts the core's
  `source.merged {from, into}` so the soul's own memory of both becomes one
  ([core.md §8](core.md)). The log is never rewritten; reads follow the merge.
- **Who may link.** The owner (`/link <account|person|name> <account|person|name>`), or the
  person themselves: `/link` from account A gives a six-digit code (single use, valid 10 minutes,
  three tries); the same code sent as `/link <code>` from account B merges B into A's person.
  The owner links his own accounts the same way. The soul may suspect and ask the owner; it has
  no tool that links. Linking makes private context follow the person across adapters (D45), so
  a wrong link is a privacy leak.
- **Unlinking** (`/unlink <account>`, by the owner or the person) gives that account a new person
  id from now on, recorded as `identity.unlinked`. What was merged stays merged: the soul's memory
  is not split again, and what the account already saw cannot be taken back. `/link` says so
  before it merges.
- **Roles after a merge**: the `people:` entry of the younger id keeps applying until the owner
  moves it; the boot report names it and suggests the surviving id (§8).
- **The account shared with the owner** (D66, [adapters.md §6](adapters.md)): when an adapter's
  own account (`self_account`) maps to the owner, identity adds one neutral context line for that
  adapter: speaking in someone else's chat there speaks through the owner's account.
- **Becoming the owner** (D33). On first start the soul shows a short pairing code; the first
  person who sends it from any adapter becomes the owner. Changing the owner later needs the
  current owner. The core never knows who the owner is; the owner policy does.
- **Person cards** (D55): through the context hook it adds, for the people present or named in
  the trigger, their name and where they can be reached, next to what the soul has learned of
  them. Plugins may annotate a card (lever 2); `roles.py` adds each role's and the person's `context`;
  the card is in the volatile part, so the stable prefix stays cached.
- **`/whoami`**: the asker's person id, label and linked accounts (identity), roles with their
  labels, tier and approval mode (roles), and this place: its name, kind and adapter, plus what
  the platform reports ("admin of this group"), from the adapter's optional place-info part
  ([adapters.md §2](adapters.md)); without it, only the place id is shown. In a shared place it
  shows label, roles and place only; linked accounts go to the person's direct chat when there
  is one, never to the group.
- **`/whois <name or account>`**: another person's id, label and linked accounts; permission
  `identity.whois`.
- **Places** (D68): identity keeps the place info each adapter reports
  ([adapters.md §3](adapters.md)) — name, `form`, `parent`, `can_post`, platform role facts — for
  `/whoami`, person cards, routes (`to:` a place where `can_post` is false is refused with its
  reason) and role mapping (§8). The core sees none of it.
- Linking arrives with WhatsApp and Discord (Task 5 Part 1).

## 8. Roles and permissions
`roles.py` answers **what this person may do**; `policy.py` turns that into pass, ask or refuse
(§4). Always present, so no module or plugin has a branch for "no roles". The default
configuration is empty: the owner may do everything, everyone else only talks (D59). Modelled
on Discord roles (several per person, permissions add up, `@everyone` as the floor) and on the
base roles of GitHub and Google Cloud (a custom role starts from a fixed base; new services add
their permissions to the bases automatically).

- **Four fixed tiers**: `guest < member < trusted < admin`; the owner is above all and always
  free. Plugins know only the tiers, never the owner's role names.
- **Permissions are declared by plugins** in the manifest, each with a label and description
  (for a panel later) and the lowest `base` tier that gets it by default; a permission without a
  `base` is the owner's only. Plugins may also declare `role_settings` with a value per tier
  (e.g. a sandbox quota). Wildcards in permission names: `plugin.*` and `*`; the boot report
  and `/roles show` list what a wildcard expands to.
- **Roles are defined by the owner** in `config.yaml` → `roles:`:
  - a **tier role** has a `base` and says how much a person may do;
  - a **group role** has no `base` and only marks where a person belongs (a shop, a team); it
    gives nothing but what it lists;
  - either may add `allow`, `deny`, `settings`, an `approval` mode, a `label` (display name,
    free to change; the key is the stable id) and a `context` (§8, person cards).
  A new role works in every plugin at once, and a newly installed plugin reaches every role
  through its tier. There is no inheritance between roles; a person simply holds several (Budi =
  `member` + `toko-a`). An exception for one person is a small role (`runner: { allow:
  [workspace.run] }`) given to that person.
- **A role's effective tier** is the highest of its `base` and the base tiers of the
  permissions in its `allow` (a permission without a base counts as owner). Giving, editing and
  the "no one grows their own power" rule below compare effective tiers, so a group role that
  carries admin permissions is an admin-tier role.
- **Permission tiers per plugin** live in `config.d/<plugin>.yaml` → `permissions:` (written by
  `/plugin enable` from the manifest): changing `workspace.run: member` gives it to every role
  with `base: member`. `role_settings:` sits next to it.
- **People** (`config.yaml` → `people:`, optional), keyed by the person id of `identity.py`,
  the one stable address across accounts and adapters:
  `p_7f3a: { label: "Budi", roles: [member, toko-a], context: "The owner's brother." }`.
  - `roles` is the **only** record of who holds which role; a person not listed holds the
    fallback role.
  - `label` is for people reading the file or a panel; the soul's name for the person stays in
    identity. `/whois <name or account>` shows the id and its linked accounts.
  - No `allow`, `deny` or `settings` per person: those come only from roles.
  - An unknown id is reported at boot and ignored; after a merge (§7) the younger id's entry
    still applies to the merged person, and the boot report suggests moving it to the
    surviving id.
- **Context on person cards**: through the context hook each person present gets one line per
  role's `context`, then their own `context`, in the volatile part, within a per-card length
  cap. It is for the soul to understand who it is talking with, not something to announce.
- **Which value wins**: all of the person's roles together (a `deny` beats an `allow`), then
  the tiers in `config.d/<plugin>.yaml`, then the manifest's tiers. `fallback:` names the role everyone holds
  (`@everyone`, tier `guest` unless set). A place may override with the same `allow` and `deny`
  (as Discord's channel overrides).
- **Approval modes**, per role, default `default`: `yolo` passes everything except calls marked
  `sensitive`; `default` passes reading and editing and asks before `runs_code` or `changes`
  outside the person's own workspace; `strict` asks before every step. The owner answers, and so
  may anyone with `approval.answer` for someone else's request. An approval covers the call as
  asked; if the retried call differs, it is asked once more, then the owner is told instead.
- **The middleware's own permissions** sit in the same catalog, declared in its code:
  `config.reload`, `plugin.manage`, `roles.manage`, `identity.whois`, `approval.answer`
  (answering an approval for someone else's request) at tier `admin`; `identity.whoami` at
  `guest`. The owner may move them like any permission.
- **No one grows their own power** (as Discord: you assign only roles below your highest):
  `/role add` gives only roles whose effective tier is below the giver's highest tier, so only
  the owner gives an admin-tier role; `/plugin enable` by an admin cannot place a permission above the admin's own
  tier. Owner only, never movable: anything `identity_changing`, a new owner (pairing), editing
  admin-tier roles, and moving the middleware's own permissions.
- `/roles show <role>` prints the merged result across plugins, each line marked with its
  source (manifest, `config.d/` or `config.yaml`).
- Role facts reported by a platform ("admin of this group") are recorded as facts about that
  person, so the soul's social understanding can use them; they grant nothing by themselves
  (D31).
- **Platform roles mapped by the owner** (D68): a role may name the platform roles that give it,
  `moderator: { base: member, discord_role: ["<server id>/<role id>"] }` (ids, not names, since
  names change). The adapter reports the Discord role ids a person holds with each percept;
  `roles.py` reads them when it decides, so a role taken away on Discord is gone at once. A
  mapped role holds **only in places whose `parent` chain reaches that server**, never in a DM
  or on another adapter. A Discord role grants nothing until the owner maps it.
- No permission overrides the two absolute lines or buys dishonesty (D24, D31). Scoping a
  permission to one group ("admin only inside toko A") is not supported; a place override
  covers the common case. The one exception is a mapped platform role, scoped to its server by
  construction.

## 9. Interaction
What the soul shows people besides its messages, and approvals asked and answered. Three parts,
three places:
- **The core records, and changes nothing.** Everything needed is already an event (`job.started`,
  `think`, `act.out`, `act.result`); thinking and tool steps are never plugins.
- **`interaction.py` decides** what is shown, to whom and how much, from the settings below.
  Everything shown passes the audience check of `privacy.py` first: a thought can hold private
  content.
- **The adapter renders**, its own way: WhatsApp's "typing…", an edited Telegram message,
  buttons on Discord and Telegram, reactions on WhatsApp. An approval answer (a click, a
  reaction) comes back as an ordinary event to the middleware.

**Settings** (D70). Defaults live in `interaction.py`; `config.yaml` → `interaction:` sets them
for every adapter; `bodies.<adapter>.show:` overrides them for one adapter. `/show <name>
<value> [adapter]` writes the line (§5); a value the adapter cannot show is refused with the
reason.

```yaml
interaction:                 # for every adapter; /show writes these lines
  thinking: owner_direct     # off | owner_direct (the owner's direct chat only) | all
  steps: long                # off | long (work that takes long) | all
  approvals: owner           # who is asked: the person the policy names
bodies:
  whatsapp:
    enabled: true
    show:                    # optional; overrides interaction: for this adapter only
      typing: on             # on | off
      read_receipts: on      # on | off
      progress: edit         # edit (one message, updated) | new (a message per step)
      stickers: on           # on | off (off → the sticker's emoji)
```

- **Thinking.** `owner_direct` shows thoughts only in the owner's direct chat. `all` shows them
  in every place, but each thought still passes `privacy.visible` for that place: a thought that
  holds content its audience may not see is not shown there.
- **Typing** shows only when the soul is going to speak: from the moment a reasoning result
  carries a communicative act until the last message of it is delivered (the extra wake for
  speaking elsewhere included). Appraisal and a reasoning that ends silent show nothing. Work
  before an answer is shown by `steps`, not by typing. No artificial delay.
- **Read receipts** mark a message read once the soul has processed it (its decision is
  recorded, whether it spoke or stayed silent). Never for imported or back-filled percepts
  ([adapters.md §5](adapters.md)). On an account shared with the owner
  ([adapters.md §6](adapters.md)), only in places where the soul has spoken since the message,
  so the owner keeps the unread marks on the phone.
- **Steps and progress**: with `progress: edit` one message is updated per step; an adapter
  without `edit` falls back to `new`.

**What an adapter can show** is declared at boot in `contract.SHOWS` (typing, read receipts,
edit, delete, reply, reactions, buttons, menus, polls, stickers, embeds, threads;
[adapters.md §4](adapters.md)), like V1 ADR-0018's optional parts and the channel matrix of chat
frameworks. A missing capability is never a silent no-op: `interaction.py` picks the fallback it
knows (choices: buttons → poll → numbered list; thread: new thread → reply with the title;
location: native → text; voice: voice note or speech → text; approval: buttons → reactions → "reply yes or no" as text; progress:
edit → new message; sticker → its emoji; no typing → nothing), so what is lost is a choice, visible in the code and
the boot report.

## 10. Attachments
Files that arrive with a message or leave with one, any number per message. The core never
reads them: they ride in a percept's `data` and in an act's arguments (D61).
- **Vocabulary** (`contract.py`): an attachment is `{ref, name, mime, size}`. A percept lists
  them in `data.attachments`; its `description` sums them up for reasoning ("Budi sent 3 files:
  report.pdf (2 pages), photo.jpg, data.xlsx").
- **Store** (`infra/`, one door in `contract.py`): an adapter calls `put(file) → ref` when it
  receives a file; anyone who opens one calls `open(ref)`, which `privacy.py` checks. An
  attachment carries the audience labels of the events it came with, so a file sent in a private
  chat opens or goes on to someone else only after `share`. The same file sent in several places
  is one stored file: `open` allows it when any non-forgotten event carrying it is visible to the
  requester. The file is deleted when no such event remains (hard forgetting included).
- **Deferred refs** (D69): an adapter may put a file without its bytes (`put_deferred(name,
  mime, size, locator) → ref`) for imported and back-filled media; the first `open` that passes
  the privacy check asks that adapter to fetch it (`fetch(locator)`), stores the bytes and
  returns them. A fetch that fails is an honest error; the ref stays and may be tried again.
- **Stickers** are attachments like any file; `say`'s `sticker: ref` sends one the soul has
  received ([adapters.md §4](adapters.md)), through the same privacy check as any forwarded file.
- **Reading**: Workspace's `read` takes a `ref`, through the same readers as any file
  ([plugins.md §5.3](plugins.md)). Images and stickers are opened only when reasoning chooses to.
- **Sending**: an adapter's send tool takes `attachments: [ref, …]`. The adapter declares how
  many files per message, the largest size and the types it supports; when it can send fewer
  than asked, `interaction.py` splits them into several messages and says so. The policy's
  "confirm before forwarding a file" applies per attachment.
- Whether the soul sees images directly (vision) is a separate, later decision; until then an
  image is read through a reader.

