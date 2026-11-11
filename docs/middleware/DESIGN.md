# Middleware — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../core/DESIGN.md),
[core formats](../core/FORMATS.md), frozen).

## 1. The foundation in the middle
Middleware is the shared foundation between the core and everything outside it. Bodies and
plugins never talk to the core directly; they stand on the middleware, which runs in the core's
three hooks (inbound, context, action) and offers one plugin/body API. It never learns —
learning belongs to the soul.

```
   Plugins (memory-view, modes, roles, tools…)      Bodies (Telegram, WhatsApp, robot…)
                   \                                  /
                    ──────────── Middleware ──────────
                    policy · rbac · commands & tools · install · identity · skills
                                       │  three hooks
                                 Core (the soul, frozen)
```

## 2. Modules
| Module | Folder | What it provides | Status |
|---|---|---|---|
| Policy | [policy/](policy/) | owner policy: the two absolute lines, budget, quiet hours; first and last in every hook | default, always on |
| Commands & tools | [commands-tools/](commands-tools/) | one command registry (`/help` everywhere), the capability registry behind `<self>`, tool-use rules and approvals | Step 3 |
| Install | [install/](install/) | plugin manager: manifest, loader, collisions, life cycle, catalog | Step 3 |
| Skills | [skills/](skills/) | loading installed SKILL.md into the desk | Step 3 |
| Identity | [identity/](identity/) | alias map: one person across bodies | when a second chat app arrives |
| RBAC | [rbac/](rbac/) | roles and permissions; role facts recorded for the soul | later |

## 3. The plugin/body API: six levers
Every plugin and body uses the same levers, so the core never knows the case:
1. **Pass, replace or stop** an item at a door (percepts, the desk, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to the PFC, inside its token budget.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember** only by putting events; state is a projection under `<plugin>:*`.
5. **Sense and act** through its own organs (a timer, a `set_mode` capability, a search tool).
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated`.

Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, or writes the
core books except through events.

## 4. Order in each hook
`[policy — first] → [middleware modules] → [plugin middleware, in install order] → [policy — last]`.
Scope is *shared* (every body that opts in) or *body-own*. Name collisions fail at boot.
