# Packages (plugins) — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. This layer builds on the core and may never change it — the core contract is in
[core/DESIGN.md](../core/DESIGN.md) and [core/FORMATS.md](../core/FORMATS.md) (frozen). Rulings:
[DECISIONS.md](DECISIONS.md).

## 1. What a package is
What other projects call a plugin: a folder that bundles organs, middleware, commands,
installed skills, standing rules, knob-offset bounds, a state namespace and dependencies,
declared in one manifest ([FORMATS.md](FORMATS.md)). The loader reads the manifest; the core
only sees the resulting senses, organs and hooks.

## 2. Life cycle
- **Install** by the owner (installer or a command). The loader checks collisions (commands,
  organs, state namespaces) and fails at boot on any clash.
- **Run** in-process as trusted code (the Home Assistant model). The owner policy holds against
  packages that follow the contract; it is not a sandbox against malicious code.
- **Uninstall**: its organs, middleware, rules and knob offsets disappear; its events stay in the
  log (they are history), its state namespace is no longer projected.
- **Update**: semantic versions; a new version never silently changes the core.

## 3. Kinds of packages (examples)
- **Body packages**: an organ for one platform plus its organ-own middleware.
- **Tool packages**: search, files, email organs.
- **Role packages**: assistant, customer service, sysadmin — rules, skills, maybe a birth
  certificate used only at birth.
- **Behavior packages**: modes, persona layers, simulated emotions — through the six levers.
