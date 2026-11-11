# Middleware / Install (plugin manager) — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Install (plugin manager)
- A loader outside the core assembles plugins; the core sees only senses, organs and hooks (D9).
- Name collisions (two plugins claiming the same command, organ or state namespace) fail at boot
  (D9, D41).
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model).
  The owner policy holds against plugins that follow the contract; it is not a sandbox against
  malicious code, and the docs say so (D32.11).
- **Draft**: installed by the owner through the installer or a command; a local catalog;
  semantic versions; stable, versioned contracts only after they have not changed for a while.
