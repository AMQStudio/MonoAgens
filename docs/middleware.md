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
   Plugins (memory-view, modes, search…)        Adapters (Telegram, WhatsApp, robot…)
                   \                                  /
                    ──────────── Middleware ──────────
        owner policy · identity · privacy · limits · tools · commands · loader · skills · rbac
                                       │  three hooks
                       Core (ids, labels, numbers — mechanisms only)
```

| Module | File | What it provides | When |
|---|---|---|---|
| [Policy](#3-policy) | `contract.py`, `owner_policy.py` | hook outcomes; the owner policy, first and last in every hook | Task 3, always on |
| [Privacy](#3-policy) | `privacy.py` | audience labels, what a context may hold, the withheld note, `share` | Task 3 |
| [Limits](#3-policy) | `limits.py` | daily model budget limit, quiet hours | Task 3 |
| [Identity](#7-identity-alias-map-owner-person-cards) | `identity.py` | alias map, pairing the owner, person cards; account linking | Task 3; linking in Task 5 |
| [Tools](#4-commands--tools) | `tools.py` | the tool registry, which tools a turn is offered, routing a call to its owner | Task 3 |
| [Commands](#4-commands--tools) | `commands.py` | one command registry, handled without a model call | Task 4 |
| [Install](#5-install-plugin-manager) | `loader.py` | reads `config.yaml` and manifests, loads adapters and plugins, collisions, fail-closed check | Task 4 |
| [Skills](#6-skills) | — | installed SKILL.md in the context | Task 4 |
| [RBAC](#8-rbac) | — | roles and permissions | when a group needs roles |

## 2. The API: six levers (D41)
Every plugin and adapter uses the same levers, so the core never knows the case — modes ("daily" vs
"professional"), a persona layer, simulated emotions, RBAC and moderation all fit:
1. **Pass, replace or stop** an item at a door (percepts, the desk, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to the PFC, inside its token budget, before the
   policy's last pass.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember and observe**: write only by putting events (state is a projection under
   `<plugin>:*`, so replay and hard forgetting still work); read the log and state read-only,
   for dashboards, debugging or metric exporters (D43).
5. **Sense and act** through its own tools (a timer, a `set_mode` capability, a search tool).
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated` with the
   plugin as source; learning never moves an offset, and uninstalling removes it.

Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, or writes the
core books except through events. The core's "no emotions" non-goal is about the core; a plugin
may simulate them.

**Order in each hook** (D9): `[policy — first] → [middleware modules] → [plugin middleware, in
install order] → [policy — last]`. Scope is **shared** (every body that opts in) or **body-own**
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
  after the alias map, fixed at that moment (D50), so a newcomer never receives what was said
  before they arrived. Whether a place is direct or shared comes from the adapter
  (`place_kind`), never from the shape of an id (D48). A private item may enter a context only if
  every person who will see the result is in its audience, or the context is the owner's direct
  chat; content from places that are not private flows freely. A memory's audience comes from the
  labels of the events it was derived from (D53). When the policy withholds relevant items it
  leaves one line without content — how many, and whose private place they came from (D56).
- **Sharing by consent (D46).** The policy offers the `share(item | episode, to)` capability: it
  passes only when the chain's `origin_actor` is in the item's audience, and records
  `share.granted`, after which the new person is in the audience. Each person speaks only for
  what they were part of: the owner can share work done with the soul, not what Fulan said in
  private; the owner's view of everything is a view, not ownership.
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
  budget, quiet hours. Budget and quiet hours are owner settings (starting values in `config.yaml`, later changes in
  chat as events) the owner may tighten or loosen;
  plugins may add rules inside the policy, never remove it (D22, D24, D36).

## 4. Commands & tools
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
- **Tool-use rules and approvals** are optional owner choices in the action hook, e.g. ask the
  owner before an expensive or irreversible tool call, or before a learned procedure is used
  (D19).
- **Draft**: per-person rate limits and steps-per-task limits live here.

## 5. Install (plugin manager)
- A loader outside the core reads each plugin's manifest ([plugins.md §3](plugins.md)) and
  assembles its tools, middleware, commands, skills, rules, knob-offset bounds and state
  namespace; the core sees only percepts, actions and the three hooks (D9, D41, D57). It
  refuses to start the kernel without the owner policy first and last in every hook.
- Name collisions (two plugins claiming the same command, tool or state namespace) fail at boot
  (D9, D41).
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model).
  The owner policy holds against plugins that follow the contract; it is not a sandbox against
  malicious code, and the docs say so (D32.11).
- The installed plugins are listed in `config.yaml` ([project.md §2](project.md)).
- Adapters are loaded by the same module, from the `bodies:` section of `config.yaml` (the key
  keeps its name), through each adapter's `create(options, env)` factory ([bodies.md §2](bodies.md), D49).
- Uninstalling removes a plugin's tools, middleware, rules and knob offsets; its events stay in
  the log as history.
- **Draft**: installed by the owner by listing it in `config.yaml` or with a command; a local
  catalog; semantic versions; stable, versioned contracts only after they have not changed for a
  while.

## 6. Skills
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

## 7. Identity (alias map, owner, person cards)
- Maps every native account (`telegram:123`, `whatsapp:628…`) to one person id in the inbound
  hook; the core never sees raw accounts (D10).
- Every mapping is an event and can be undone. Only the owner, or the person via a verification
  code sent from the second account, may link two accounts; the soul may suspect and ask the
  owner, never links silently, because linking makes two accounts one person, and private
  context then follows that person across bodies (D10, D45). A wrong link is a privacy leak.
- **Becoming the owner** (D33). On first start the soul shows a short pairing code; the first
  person who sends it from any adapter becomes the owner. Changing the owner later needs the
  current owner. The core never knows who the owner is; the owner policy does.
- **Person cards** (D55): through the context hook it adds, for the people present or named in
  the trigger, their name and where they can be reached, next to what the soul has learned of
  them.
- Linking accounts arrives with the second chat app (Task 5).

## 8. RBAC
- **Hierarchy as permission** (roles, who may do what, "only admins may run /settings") is
  enforced only in the inbound and action hooks; the core never sees it (D31).
- Role facts reported by a platform ("admin of this group") are recorded as facts about that
  person, so the soul's social understanding can use them; the soul's own sense of standing stays
  in its person cards (D31).
- Permission never overrides the two absolute lines and never buys dishonesty (D24, D31).
