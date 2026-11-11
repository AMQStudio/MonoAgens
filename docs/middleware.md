# Middleware — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. The foundation in the middle
Middleware is the shared foundation between the core and everything outside it. Bodies and
plugins never talk to the core directly: they stand on the middleware, which runs in the core's
three hooks (inbound, context, action) and offers one API. It is static and never learns —
learning belongs to the soul (D34).

```
   Plugins (memory-view, modes, roles, tools…)      Bodies (Telegram, WhatsApp, robot…)
                   \                                  /
                    ──────────── Middleware ──────────
                    policy · commands & tools · install · skills · identity · rbac
                                       │  three hooks
                                 Core (the soul, frozen)
```

| Module | What it provides | Status |
|---|---|---|
| [Policy](#3-policy) | owner policy; first and last in every hook | default, always on |
| [Commands & tools](#4-commands--tools) | command registry, capability registry, tool-use rules | first |
| [Install](#5-install-plugin-manager) | plugin manager: manifest, loader, collisions, life cycle | first |
| [Skills](#6-skills) | installed SKILL.md in the desk | first |
| [Identity](#7-identity-alias-map) | one person across bodies | when a second chat app arrives |
| [RBAC](#8-rbac) | roles and permissions | when a group needs roles |

## 2. The API: six levers (D41)
Every plugin and body uses the same levers, so the core never knows the case — modes ("daily" vs
"professional"), a persona layer, simulated emotions, RBAC and moderation all fit:
1. **Pass, replace or stop** an item at a door (percepts, the desk, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to the PFC, inside its token budget, before the
   policy's last pass.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember and observe**: write only by putting events (state is a projection under
   `<plugin>:*`, so replay and hard forgetting still work); read the log and state read-only,
   for dashboards, debugging or metric exporters (D43).
5. **Sense and act** through its own organs (a timer, a `set_mode` capability, a search tool).
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
- Enforces the core's two absolute lines deterministically: the `identity_changing` check on
  `origin_actor` in the action hook, and the private-content filter in the context hook
  (D24, D32.3–4).
- **Data leaving the machine** (D43): by default nothing does. Data goes out only through a
  plugin the owner installed and configured for it. Such a plugin sends numbers (counts,
  tokens, timings, decisions), not conversation content; sending content outside counts as
  showing it to people who were not part of it and is held to absolute line 2.
- **Privacy rules narrow, never widen.** The core enforces absolute line 2 on its own (D45).
  Middleware may only add stricter rules (a mode that keeps health topics out even of the
  person's own chat, an organization rule that a past meeting stays with its board); it can
  never let private content reach someone the core would refuse. It may also ask for more than
  consent before sharing (confirm before forwarding any file; internal documents never go
  outside the organization) (D46).
- **Routes to a person** (D46): a hint via lever 2, used only when the owner did not name a body, when the default (the body where the person
  was last active) is wrong, such as email for work, WhatsApp in the evening.
- **Default owner policy**, active out of the box: the two absolute lines, the daily model
  budget, quiet hours. Budget and quiet hours are owner settings (starting values in `config.yaml`, later changes in
  chat as events) the owner may tighten or loosen;
  plugins may add rules inside the policy, never remove it (D22, D24, D36).

## 4. Commands & tools
- **Commands are defined once** in a shared registry. Bodies only translate native syntax
  (`/help@bot`, `/help`, `:help`) into a neutral `command` event and may add presentation (menus,
  buttons) but never logic. `/help` is the same everywhere; plugins register commands into the
  same registry (D9).
- **Capability registry**: the capabilities of all organs present (bodies and plugin tools),
  sorted, from which the core assembles `<self>`.
- **Tool-use rules and approvals** are optional owner choices in the action hook, e.g. ask the
  owner before an expensive or irreversible tool call, or before a learned procedure is used
  (D19).
- **Draft**: per-person rate limits and steps-per-task limits live here.

## 5. Install (plugin manager)
- A loader outside the core reads each plugin's manifest ([plugins.md §3](plugins.md)) and
  assembles its organs, middleware, commands, skills, rules, knob-offset bounds and state
  namespace; the core sees only senses, organs and hooks (D9, D41).
- Name collisions (two plugins claiming the same command, organ or state namespace) fail at boot
  (D9, D41).
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model).
  The owner policy holds against plugins that follow the contract; it is not a sandbox against
  malicious code, and the docs say so (D32.11).
- The installed plugins are listed in `config.yaml` ([project.md §2](project.md)).
- Bodies are loaded by the same module, from the `bodies:` section, through each body's
  `create(options, env)` factory ([bodies.md §2](bodies.md), D49).
- Uninstalling removes a plugin's organs, middleware, rules and knob offsets; its events stay in
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

## 7. Identity (alias map)
- Maps every native account (`telegram:123`, `whatsapp:628…`) to one person id in the inbound
  hook; the core never sees raw accounts (D10).
- Every mapping is an event and can be undone. Only the owner, or the person via a verification
  code sent from the second account, may link two accounts; the soul may suspect and ask the
  owner, never links silently, because linking makes two accounts one person, and private
  context then follows that person across bodies (D10, D45). A wrong link is a privacy leak.
- Needed when a second chat app arrives; deferred until then (D32.9).

## 8. RBAC
- **Hierarchy as permission** (roles, who may do what, "only admins may run /settings") is
  enforced only in the inbound and action hooks; the core never sees it (D31).
- Role facts reported by a platform ("admin of this group") are recorded as facts about that
  person, so the soul's social understanding can use them; the soul's own sense of standing stays
  in its person cards (D31).
- Permission never overrides the two absolute lines and never buys dishonesty (D24, D31).
