# Middleware — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing in this layer may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Middleware (static; runs in the hooks; never learns)
- Middleware adds capabilities and rules the owner chooses; it does not learn (D34).
- **Open to any behavior** (D41). The core never knows the case — modes ("daily" vs
  "professional"), a persona layer, simulated emotions, RBAC, moderation. Every middleware uses
  the same levers:
  1. *Pass, replace or stop* any item at the three doors (percepts, the desk, actions).
  2. *Annotate* an item; the note travels with it to the PFC.
  3. *Declare* standing rules, which appear in `<self>`.
  4. *Remember* only by putting events; its state is a projection under its own namespace
     (`<package>:*`), so replay and hard forgetting still work.
  5. *Sense and act* through organs of its own package (a timer, a `set_mode` capability).
  6. *Steer the knobs* by a bounded, temporary offset recorded as `disposition.updated` with the
     package as source; learning never moves an offset, and removing the package removes it.
  Limits: it cannot cross the two absolute lines (the owner policy runs first and last), add a
  thought act, signal or book, or write core books except through events. The core's "no
  emotions" non-goal is about the core; a package may simulate them.
- **Owner policy** wraps every package middleware (first and last) and cannot be removed by a
  package (D9).
- **Default owner policy**, active out of the box: the two absolute lines, the daily model
  budget, quiet hours. Budget and quiet hours are owner settings; the owner may tighten or loosen
  them; packages may add rules inside it, never remove it (D22, D24, D36).
- Scope: **shared** (every organ that opts in: commands, skills, tool rules) or **organ-own**
  (unique to one platform) (D9).
- **Commands**: one shared registry; `/help` is identical on every organ; packages register
  commands into it (D9).
- **RBAC, roles, hierarchy as permission**: in the hooks only; platform role facts ("admin of
  this group") are recorded as facts about that person for the soul's social understanding
  (D31).
- **Tool-use rules and approvals**: optional; e.g. an owner may require approval before a
  learned procedure is used (D19).
- **Installed skills**: SKILL.md; only names and descriptions of relevant skills enter the
  prompt via the context hook; the body is loaded on request; versioned; use counts feed the
  same ACT-R activation as memories (D11, D34).
- **Alias map** (one person across bodies): every mapping is an event and can be undone; only the
  owner, or the person via a verification code sent from the second account, may link; the soul
  may suspect and ask, never links silently. Needed when a second chat app arrives (D10).
- **Draft — memory-view**: list what the soul remembers about the asker or this place;
  corrections via `revise` with the asker as `origin_actor`; owner-only Markdown export as a
  view, never the source of truth; both absolute lines apply (D37).
- **Draft — package suggestions**: only to the owner, only from the local catalog, at most once
  per missing capability per habituation window, never installs anything (D28).
- **Draft**: per-person rate limits, content filters, scheduled quiet modes — owner choices.
