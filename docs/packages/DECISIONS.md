# Packages (plugins) — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing in this layer may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Packages (plugins)
- A package bundles organs, middleware, installed skills and dependencies. A loader outside the
  core assembles them; the core sees only senses, organs and hooks (D9).
- Name collisions (two packages claiming the same command, organ or state namespace) fail at boot
  (D9, D41).
- A package declares what it brings — organs, middleware, commands, installed skills, standing
  rules, its state namespace — in one manifest (format in FORMATS.md, Draft) (D41).
- Packages are trusted code installed by the owner and run in-process (the Home Assistant model).
  The owner policy holds against packages that follow the contract; it is not a sandbox against
  malicious code, and the docs say so (D32.11).
- Role packages (assistant, customer service, sysadmin) may ship a birth certificate, used only at
  birth; they can never overwrite identity afterwards (D20, D35).
- **Draft**: installed by the owner through the installer or a command; a local catalog;
  semantic versions; stable, versioned contracts for organs, packages and middleware only after
  they have not changed for a while (Phase 9).
