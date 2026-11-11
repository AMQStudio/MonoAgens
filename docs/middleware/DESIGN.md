# Middleware — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. This layer builds on the core and may never change it — the core contract is in
[core/DESIGN.md](../core/DESIGN.md) and [core/FORMATS.md](../core/FORMATS.md) (frozen). Rulings:
[DECISIONS.md](DECISIONS.md).

## 1. What middleware is
Static rules and capabilities the owner chooses, running in the core's three hooks (inbound,
context, action). It never learns: learning belongs to the soul. It is open to any behavior —
modes, persona layers, simulated emotions, RBAC, moderation — because every middleware uses the
same six levers and the core never knows the case.

## 2. The six levers
1. **Pass, replace or stop** an item at a door (percepts, the desk, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to the PFC, inside its token budget.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember** only by putting events; its state is a projection under `<package>:*`.
5. **Sense and act** through organs of its own package (a timer, a `set_mode` capability).
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated`; removing
   the package removes the offset; learning never moves it.

Limits: it cannot cross the two absolute lines, add a thought act, signal or book, or write the
core books except through events.

## 3. Order in each hook
`[owner policy — first] → [package middleware, in install order] → [owner policy — last]`.
Scope is *shared* (every organ that opts in) or *organ-own*. Name collisions fail at boot.

## 4. Built-in and planned middleware
| Middleware | Hook(s) | Status | What it does |
|---|---|---|---|
| Owner policy | all | default, always on | the two absolute lines, daily budget, quiet hours |
| Commands | inbound | planned (shared) | one registry; identical `/help` everywhere |
| Installed skills | context | planned (shared) | names and descriptions of relevant SKILL.md; body on request |
| Alias map | inbound | later | one person across bodies; linking only by owner or verification code |
| RBAC / roles | inbound, action | later | permission by role; role facts recorded for the soul |
| memory-view | inbound, action | Draft | show and correct what the soul remembers; owner export |
| Modes (example) | context | example | daily / professional: rules in `<self>`, notes, knob offsets |
| Package suggestions | context | Draft | owner only, local catalog, rare, never installs |
