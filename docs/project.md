# Project — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. Rulings
- Open source under Apache 2.0; public documents mention no hosted service (D8).
- BYOK; one deployment is one soul with its own database; keys only from the environment; the
  embedder is a replaceable port (D7).
- **Who this repository is for**: people who can run their own server. Self-hosted on a VPS or
  a home machine, configured by hand (`.env`, `config.yaml`, `soul.yaml`), BYOK. It ships a
  basic hand-tuned default prior and no role templates; it never ships priors learned from
  other people's souls. A soul still needs no tuning to start: it is born from the default prior
  and adapts (D20).
- Order of users: the owner alone, then one real group the owner knows well (also the honest
  human-likeness evaluation), then developers once contracts are stable (D13).
- **Release gate**: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models
  (D33.6).
- Never described as conscious or as "% human"; claims are per behavior against a human
  baseline.
- **Draft**: README with the brain table, manual install on a server (Docker for Postgres,
  `.env`, `config.yaml`, `soul.yaml`), talk, tune with replay; compressed daily backups with an
  owner-set retention (D14).

## 2. Deployment files
Four files, each with one job; none holds what belongs to another (D7, D35, D36).

| File | Holds | Read | Changed by |
|---|---|---|---|
| `.env` | secrets and endpoints: model keys and URLs, bot tokens, database URL | every start | owner or technician |
| `config.yaml` | this deployment's non-secret settings: timezone, owner settings, runtime, bodies and plugins | every start | owner; owner settings also in chat |
| `soul.yaml` | the birth certificate: name, language, persona, principles ([core.md §9.3](core.md)) | once, at birth | nobody afterwards; identity changes in chat |
| `prior/` | instincts: knobs, calibration, constants ([core.md §9.4](core.md)) | once, at birth | developers, with replay |

**`config.yaml`** (format version 1, **Draft**):

```yaml
timezone: Asia/Jakarta              # where the soul lives; used for <now>
owner_settings:                     # initial values; changes made in chat are events and win
  daily_token_budget: 300000
  quiet_hours: { from: "22:00", to: "06:00" }
runtime:
  tick_seconds: 30
  sleep: { idle_minutes: 30, retry_minutes: 30, hour: 2 }
bodies:                             # non-secret options only; tokens stay in .env
  telegram: { enabled: true }
  cli: { enabled: true }
plugins: []                         # installed plugins, e.g. [work-modes]
```

The rule for every setting: the file gives the starting value; what the soul's books say wins.

## 3. Documents (D32.14)
| File | For | Holds |
|---|---|---|
| `README.md` (root) | people | what it is, install, run, tune with replay |
| `AGENTS.md` (root) | coding agents | rules, stack, invariants, size guide |
| `docs/constitution.md` | everyone | the rules that never change |
| `docs/core.md` | everyone | the core specification (contract frozen) |
| `docs/core-math.md` | scientists | every formula, why it was chosen, sources |
| `docs/middleware.md`, `bodies.md`, `plugins.md`, `project.md` | everyone | one specification per layer |
| `prior/` (root) | developers | default prior values: data, tuned with replay |
| `config.yaml`, `soul.yaml`, `.env.example` (root) | owners | deployment files (§2) |
| `docs/tasks/` | workers | one file per work package, in order |

Rules for documents: one layer = one file, split only when a file grows too long; rulings and
design live together, each rule tagged with its Dxx; formats live in the layer that owns them;
history lives in git, not in the documents.

## 4. Evaluation (later)
Per-behavior parity against a human baseline in the same group: reply timing, messages per hour
vs 1/n, when-to-speak precision/recall against human annotation, silence where it should speak,
interruptions where it should be quiet, promise keeping, recall after 1/7/30 days, blind "which
one is the bot" (target ≈ 50%). Pre-registered metrics. Public claims phrased as "≥ X% of human
baseline on these N behaviors".

**Every scenario carries its own material** (D54). A scenario holds, as synthetic data the worker writes, everything the behavior needs: the facts, the people and their accounts, the artifacts (a file, a piece of work, code and its test output), realistic results from every test organ it uses, and the capabilities the behavior needs (mentioning a file is not enough: reading it and sending it must be capabilities a test organ offers). A failure is called the soul's failure only after a material check shows that what the soul needed was in the Log and reachable on its desk; otherwise the scenario is fixed, never the soul. The material check is written under the task's Notes.

**Scenarios are the capability record** (D48). Each scenario states one capability in one
line and the brain function it belongs to. The replay script writes `scenarios/README.md` from
the latest runs: per capability, whether it holds with the fake model and with the real model
(passes over repeated runs, model name, date). Nothing is claimed in the README or anywhere else
that this record does not show. A capability that fails with the real model is listed as such,
not hidden.

## 5. Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
