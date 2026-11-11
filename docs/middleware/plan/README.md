# Middleware — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder, tick steps and record
findings. They never change DESIGN, DECISIONS, FORMATS or MATH; a design question stops the work
and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Step 3 — The foundation (in this order)
1. **policy**: default owner policy, active out of the box — the two absolute lines, the daily
   budget, quiet hours.
2. **commands & tools**: one command registry; the capability registry behind `<self>`;
   tool-use rules and approvals.
3. **install**: plugin manager reading the manifest; collisions fail at boot; install and
   uninstall leave the core untouched.
4. **skills**: installed SKILL.md names and descriptions in the desk; body on request.
- Done when: a DM fact is never visible or said in a group, even when someone tries to talk the
  soul into it; a reasonable request from a group member is helped; no plugin can remove the
  policy; `/help` is identical on Telegram and CLI; the example modes plugin installs, works and
  uninstalls through the six levers without any core change.

## Later
5. **identity** (alias map) when a second chat app arrives.
6. **rbac** when the first group needs roles.
