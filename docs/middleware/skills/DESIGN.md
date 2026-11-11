# Middleware / Skills — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

- Installed skills are static SKILL.md files shipped by plugins ([FORMATS.md](FORMATS.md)).
- In the context hook, only the names and descriptions of relevant skills enter the desk; the
  body is loaded when the PFC asks for it.
- What the soul learns by itself is a core `procedure` or `lesson` memory, never a skill.
