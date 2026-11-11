# Project — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. Rulings
- Open source under Apache 2.0; public documents mention no hosted service (D8).
- BYOK; one deployment is one soul with its own database; keys only from the environment; the
  embedder is a replaceable port (D7).
- Out of the box for non-technical people: no configuration panel; a soul is born from a prior
  and adapts (D20).
- Order of users: the owner alone, then one real group the owner knows well (also the honest
  human-likeness evaluation), then developers once contracts are stable (D13).
- **Release gate**: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models
  (D33.6).
- Never described as conscious or as "% human"; claims are per behavior against a human
  baseline.
- **Draft**: installer (one command from clone to a talking soul); README with the brain table,
  install, talk, tune with replay; compressed daily backups with an owner-set retention (D14).

## 2. Documents (D32.14)
| File | For | Holds |
|---|---|---|
| `README.md` (root) | people | what it is, install, run, tune with replay |
| `AGENTS.md` (root) | coding agents | rules, stack, invariants, size budget |
| `docs/constitution.md` | everyone | the rules that never change |
| `docs/core.md` | everyone | the core specification (frozen) |
| `docs/core-math.md` | scientists | every formula, why it was chosen, sources |
| `docs/middleware.md`, `bodies.md`, `plugins.md`, `birth.md`, `project.md` | everyone | one specification per layer |
| `docs/tasks/` | workers | one file per work package, in order |

Rules for documents: one layer = one file, split only when a file grows too long; rulings and
design live together, each rule tagged with its Dxx; formats live in the layer that owns them;
history lives in git, not in the documents.

## 3. Evaluation (later)
Per-behavior parity against a human baseline in the same group: reply timing, messages per hour
vs 1/n, when-to-speak precision/recall against human annotation, silence where it should speak,
interruptions where it should be quiet, promise keeping, recall after 1/7/30 days, blind "which
one is the bot" (target ≈ 50%). Pre-registered metrics. Public claims phrased as "≥ X% of human
baseline on these N behaviors".

## 4. Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
