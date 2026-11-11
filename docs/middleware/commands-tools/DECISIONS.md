# Middleware / Commands & tools — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Commands & tools
- **Commands are defined once** in a shared registry; bodies translate native syntax into a neutral
  `command` event and may add presentation (menus, buttons) but never logic; `/help` is the same
  everywhere; plugins register commands into the same registry (D9).
- **Tool-use rules and approvals** are optional owner choices in the action hook, e.g. an owner may
  require approval before a learned procedure is used (D19).
- **Draft**: per-person rate limits and steps-per-task limits live here.
