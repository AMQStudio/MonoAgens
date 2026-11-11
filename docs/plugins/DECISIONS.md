# Plugins — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing here may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Plugins
- A plugin (a *package* in the core documents) bundles organs, middleware, installed skills and
  dependencies; it stands on the middleware and reaches the core only through it (D9).
- A plugin declares what it brings — organs, middleware, commands, installed skills, standing
  rules, knob-offset bounds, its state namespace — in one manifest ([FORMATS.md](FORMATS.md),
  Draft) (D41).
- Tools (search, files, email) are organs shipped by plugins (D35).
- Role plugins (assistant, customer service, sysadmin) may ship a birth certificate, used only at
  birth; they can never overwrite identity afterwards (D20, D35).
- **Draft — memory-view & manage**: list what the soul remembers about the asker or this place;
  corrections via `revise` with the asker as `origin_actor`; owner-only Markdown export as a
  view, never the source of truth; both absolute lines apply (D37).
- **Draft — plugin suggestions**: only to the owner, only from the local catalog, at most once
  per missing capability per habituation window, never installs anything (D28).
- **Draft**: moderation, content filters and per-person rate limits are owner-chosen plugins.
