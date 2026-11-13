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
Five files and folders, each with one job; none holds what belongs to another (D7, D35, D36).

| File | Holds | Read | Changed by |
|---|---|---|---|
| `.env` | secrets and endpoints: the model provider and its fallbacks (`LLM_PROVIDER`, `LLM_FALLBACKS`, keys, model), bot tokens, database URL | every start | owner or technician |
| `config.yaml` | this deployment's non-secret settings: timezone, owner settings, runtime, adapters, plugins on, roles | every start | owner; owner settings in chat; `/plugin` and `/role` write their own lines |
| `config.d/` | one file per plugin: its features and options | every start | owner |
| `soul.yaml` | the birth certificate: name, language, persona, principles ([core.md §9.3](core.md)) | once, at birth | nobody afterwards; identity changes in chat |
| `prior/` | instincts: knobs, calibration, constants ([core.md §9.4](core.md)) | once, at birth | developers, with replay |

**`config.yaml`** (format version 2, **Draft**; version 1 had `plugins:` as a list and is upcast
in memory):

```yaml
version: 2
timezone: Asia/Jakarta              # where the soul lives; used for <now>
owner_settings:                     # changed by hand or with /budget and /quiet, which write these lines
  daily_token_budget: 300000
  quiet_hours: { from: "22:00", to: "06:00" }
runtime:
  tick_seconds: 30
  sleep: { idle_minutes: 30, retry_minutes: 30, hour: 2 }
bodies:                             # adapters (the key keeps its name); non-secret options only; tokens in .env
  telegram: { enabled: true }
  cli: { enabled: true }
  whatsapp:
    enabled: true
    show: { read_receipts: on, progress: edit }   # optional; overrides interaction: (middleware.md §9)
interaction:                        # what the soul shows, for every adapter; /show writes these lines
  thinking: owner_direct            # off | owner_direct | all
  steps: long                       # off | long | all
  approvals: owner
plugins:                            # which plugins are on; their settings live in config.d/<name>.yaml
  workspace:   { enabled: true }
  web-search:  { enabled: true }
  soul-tuning: { enabled: true }
roles:                              # middleware.md §8; empty = owner free, everyone else only talks
  fallback: unverified
  unverified: { base: guest,  label: "Unverified", context: "Not known yet; be kind, share nothing about the owner." }
  customer:   { base: guest,  label: "Customer",   context: "A shop customer; help with products and orders." }
  member:     { base: member }
  moderator:  { base: member, discord_role: ["<server id>/<role id>"] }   # held only in that server (D68)
  runner:     { allow: [workspace.run] }             # a small role for an exception
  toko-a:     { label: "Toko A", settings: { workspace.shared: toko-a } }   # a group role: no base
people:                             # optional; keyed by person id; the only record of who holds which role
  p_7f3a:     { label: "Budi", roles: [member, toko-a, runner], context: "The owner's brother." }
```

**`config.d/<plugin>.yaml`**: one per plugin, only what the owner changed ([plugins.md §4](plugins.md)).
If `config.yaml` grows too long, `roles:` may move to its own file later; the format stays.

**Format versions** (D58): every YAML file the owner or a developer edits carries its format
version (`config.yaml`, `config.d/*.yaml`, `soul.yaml`; the prior folder through
`format_version` in `prior/manifest.yaml`, since some prior files are plain lists; plugin
manifests through `manifest_version`). Older versions are upcast in memory and named in the
boot report; the file is never rewritten. A newer version than supported fails at boot. A file
without `version:` is read as version 1 with a warning.

The rule for every setting: the deployment files (`config.yaml`, `config.d/`) are the source of
truth for the deployment and are changed by hand or by the few commands that write their own
lines; what the soul learns (knobs, memories, identity) lives in its books, and there the
books win over the starting values in `soul.yaml` and `prior/`.

## 3. Documents (D32.14)
| File | For | Holds |
|---|---|---|
| `README.md` (root) | people | what it is, install, run, tune with replay |
| `AGENTS.md` (root) | coding agents | rules, stack, invariants, size guide |
| `docs/constitution.md` | everyone | the rules that never change |
| `docs/core.md` | everyone | the core specification (contract frozen) |
| `docs/core-math.md` | scientists | every formula, why it was chosen, sources |
| `docs/middleware.md`, `adapters.md`, `plugins.md`, `project.md` | everyone | one specification per layer |
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

**Every scenario carries its own material** (D54). A scenario holds, as synthetic data the worker writes, everything the behavior needs: the facts, the people and their accounts, the artifacts (a file, a piece of work, code and its test output), realistic results from every test tool it uses, and the capabilities the behavior needs (mentioning a file is not enough: reading it and sending it must be capabilities a test tool offers). A failure is called the soul's failure only after a material check shows that what the soul needed was in the Log and reachable in its context; otherwise the scenario is fixed, never the soul. The material check is written under the task's Notes.

**Scenarios are the capability record** (D48). Each scenario states one capability in one
line and the brain function it belongs to. The replay script writes `scenarios/README.md` from
the latest runs: per capability, whether it holds with the fake model and with the real model
(passes over repeated runs, model name, date). Nothing is claimed in the README or anywhere else
that this record does not show. A capability that fails with the real model is listed as such,
not hidden.

## 5. Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
- **MCP Connect** as a plugin: each MCP server the owner installs (`config.d/mcp.yaml`, `/plugin`)
  becomes a source of tools in the registry, which already follows MCP's pattern of tools,
  prompts and resources ([middleware.md §4](middleware.md)). Every MCP tool declares facts and a
  permission like any tool and passes the same policy and approval modes; an outside server never
  bypasses them.
- **Voice calls** on chat adapters, once a mature library for the platform is widely released
  (for WhatsApp, a linked-device calling stack such as meowcaller on top of whatsmeow, which would
  run as a Go sidecar and needs the owner's approval as a dependency): call audio → STT → the soul
  → TTS → call audio, as a `long_running` act ([adapters.md §7](adapters.md)).
