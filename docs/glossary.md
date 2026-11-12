# Glossary — standard names

The code uses standard engineering names. Older specs and task notes still use the earlier,
biological ones; this table maps them. Task 3 moves the code to these names and the architect
then renames the terms in the specs.

## Terms

| Earlier name | Standard name | Meaning |
|---|---|---|
| body, bodies | **adapter** | Connects the soul to one app, platform or device (Telegram, WhatsApp, CLI, a game, a robot). |
| organ | **tool** | Something the soul can use; what it can do is a **capability**. |
| desk | **context** | What the model sees in one turn. |
| `<doing>` (working memory) | **working memory**, part of the context | The chains in progress. |
| PFC | **reasoning** | One model turn with the five thought acts. |
| cognition, appraise | **gate** | Decides whether an event wakes reasoning (the one gate formula). |
| task thread | **thread** | The main thread is the soul; extra threads do one piece of work and report back. |
| guidance | **principles** | The short list of general principles in the stable prompt. |
| soul | **soul** (kept) | The agent with its lasting identity (`soul.yaml`). |
| prior | **prior** (kept) | The defaults a soul is born with; the standard statistics term. |

## What never changes with a rename

Names stored in data are part of the frozen contract ([constitution.md](constitution.md) §1):
event kinds (`percept`, `decision.wake`, `expectation.reminded`, …), payload fields, `soul.yaml`
keys, the `prior/` format and `config.yaml` keys. A rename touches code only.

## Layout

| Folder | Holds | Changes |
|---|---|---|
| `monoagens/core/` | Mechanisms only: kernel loop, gate, reasoning, context, memory, expectations, threads, the tool door (`dispatch`), the three hooks, identity, the audience labels. | Rarely: evidence, bugs, vulnerabilities. |
| `monoagens/middleware/` | Every decision and the wiring between adapters and plugins: the loader, the tool and command registries, the owner policy (privacy rules, who may share, budget limit, quiet hours), identity (which account is which person), pairing the owner. | Often, as cases need. |
| `monoagens/adapters/` | One folder per adapter. | Per app or device. |
| `monoagens/infra/` | Store, migrations, config, clock, budget counting, embedding, versioning. | With the stack. |
| `plugins/` | Installable features on the middleware. | Per feature. |

The rule between the layers: **the middleware decides, the core records and carries out.**
