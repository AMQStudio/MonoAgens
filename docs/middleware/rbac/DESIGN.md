# Middleware / RBAC — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../../core/DESIGN.md),
[core formats](../../core/FORMATS.md), frozen).

- Roles and permissions per place ("only admins may run /settings"), enforced in the inbound and
  action hooks.
- Role facts reported by a platform ("admin of this group") are recorded as facts about that
  person, so the soul's social understanding can use them; the soul's own sense of standing stays
  in its person cards.
