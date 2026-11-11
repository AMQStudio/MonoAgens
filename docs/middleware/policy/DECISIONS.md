# Middleware / Policy — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Policy
- **Owner policy** wraps every module and plugin middleware, first and last, and cannot be
  removed (D9).
- **Default owner policy**, active out of the box: the two absolute lines, the daily model
  budget, quiet hours. Budget and quiet hours are owner settings the owner may tighten or loosen;
  plugins may add rules inside it, never remove it (D22, D24, D36).
- The absolute lines themselves are core rulings; this module is where they are enforced
  (D24, D32.3–4).
