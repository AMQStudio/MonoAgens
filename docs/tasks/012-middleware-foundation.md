# 012 — Middleware: the foundation
Status: todo | Wave: 3 | Depends on: 002–011
Read first: [middleware.md](../middleware.md)
May touch: `monoagens/middleware.py` (and a `middleware/` package if it outgrows its budget), `monoagens/bodies/`, `tests/`

## Goal
Policy, commands & tools, install and skills, in this order.

## Tasks
- [ ] T1 Policy: default owner policy — two absolute lines, daily budget, quiet hours; first and last in every hook.
- [ ] T2 Commands & tools: one command registry (identical `/help`); capability registry behind `<self>`; optional tool-use approvals.
- [ ] T3 Install: plugin manager reading the manifest (plugins.md §3); collisions fail at boot; uninstall removes everything but events.
- [ ] T4 Skills: names and descriptions of relevant SKILL.md in the desk; body on request.

## Done when
A DM fact is never visible or said in a group, even when someone tries to talk the soul into it; a reasonable request from a group member is helped; no plugin can remove the policy; `/help` is identical on Telegram and CLI.

## Notes
