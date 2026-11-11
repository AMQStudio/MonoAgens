# Middleware / Install (plugin manager) — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

- Reads each plugin's manifest ([plugin formats](../../plugins/FORMATS.md)) and assembles its
  organs, middleware, commands, skills, rules, knob-offset bounds and state namespace.
- Checks collisions (commands, organs, state namespaces) and fails at boot on any clash.
- Install and uninstall: uninstalling removes the plugin's organs, middleware, rules and knob
  offsets; its events stay in the log as history.
- The one-command installer (from clone to a talking soul) uses the same module.
