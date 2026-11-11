# Middleware / Commands & tools — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

- **Commands**: one shared registry. Bodies only translate native syntax (`/help@bot`, `:help`)
  into a neutral `command` event; the registry decides. `/help` is identical everywhere; plugins
  register commands into it.
- **Tools and capabilities**: the registry of capabilities of all organs present (bodies and
  plugin tools), sorted, which the core shows in `<self>` and the PFC uses through `act`.
- **Tool-use rules and approvals**: optional rules in the action hook, e.g. ask the owner before
  an expensive or irreversible tool call, or before a learned procedure is used.
