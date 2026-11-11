# Middleware — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

Module rulings: [policy](policy/DECISIONS.md), [commands & tools](commands-tools/DECISIONS.md),
[install](install/DECISIONS.md), [skills](skills/DECISIONS.md),
[identity](identity/DECISIONS.md), [rbac](rbac/DECISIONS.md).

---

## 1. Middleware as a whole
- Middleware is the foundation between the core and bodies and plugins; bodies and plugins
  reach the core only through it (the three hooks and the organ contract).
- It is static and never learns; learning belongs to the soul (D34).
- **Open to any behavior** through one API of six levers (pass/replace/stop, annotate, declare,
  remember via events, sense and act through own organs, bounded knob offsets). The core never
  knows the case: modes, persona layers, simulated emotions, RBAC and moderation all use the same
  levers. The core's "no emotions" non-goal is about the core (D41).
- Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, or writes
  the core books except through events (D41).
- Scope: **shared** (every body that opts in) or **body-own** (unique to one platform) (D9).
- Policy runs first and last in every hook and cannot be removed (D9).
