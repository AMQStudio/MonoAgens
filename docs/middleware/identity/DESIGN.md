# Middleware / Identity (alias map) — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

- Maps every native account (`telegram:123`, `whatsapp:628…`) to one person id, so one person is
  one person across bodies.
- Runs in the inbound hook; the core never sees raw accounts.
