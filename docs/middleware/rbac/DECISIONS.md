# Middleware / RBAC — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. RBAC
- **Hierarchy as permission** (roles, who may do what) is enforced only in the hooks; the core
  never sees it (D31).
- Platform role facts are recorded as facts about that person for the soul's social
  understanding (D31).
- Permission never overrides the two absolute lines and never buys dishonesty (D24, D31).
