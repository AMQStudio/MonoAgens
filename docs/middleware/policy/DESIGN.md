# Middleware / Policy — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

The owner policy is the one middleware that is always present. It runs first and last in every
hook, so nothing installed can get around it.
- Enforces the core's two absolute lines deterministically: the `identity_changing` check on
  `origin_actor` in the action hook, and the private-content filter in the context hook.
- Holds the owner settings: the daily model budget and quiet hours.
- Plugins may add rules inside it (for example official roles); they can never remove it.
