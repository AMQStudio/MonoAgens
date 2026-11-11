# Project — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing in this layer may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Project
- Open source under Apache 2.0; public documents mention no hosted service (D8).
- BYOK; one deployment is one soul with its own database; keys only from the environment; the
  embedder is a replaceable port (D7).
- Out of the box for non-technical people: no configuration panel; a soul is born from a prior and
  adapts (D20).
- Order of users: the owner alone, then one real group the owner knows well (also the honest
  human-likeness evaluation), then developers once contracts are stable (D13).
- **Release gate**: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models
  (D33.6).
- Never described as conscious or as "% human"; claims are per behavior against a human
  baseline.
- Documents: one folder per layer under `docs/` with DESIGN, DECISIONS, FORMATS where needed and
  `plan/`; the core folder also holds MATH.md; AGENTS.md (rules for coding agents) and README.md
  stay at the root; the map is docs/README.md (D32.14).
- **Draft**: installer (one command from clone to a talking soul); README with the brain table,
  install, talk, tune with replay; compressed daily backups with an owner-set retention (D14).

## 2. Open
- Worker prompts for core Track A2 and the Phase 2 tracks, when the owner resumes implementation.
- Deferred non-core items: identity linking and package suggestions (middleware).
