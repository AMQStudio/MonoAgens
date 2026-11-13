# Glossary — standard names

The specs use standard engineering names. The code moves to them in Task 3; until then, and in
the notes of Tasks 1–2, the earlier biological names appear; this table maps them.

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
| — | **provider** (driver) | Where the soul thinks: a model API reached through the core's model port, implemented in `infra/providers/`. Not an adapter: it neither senses nor acts. |
| — | **tier** | One of `guest < member < trusted < admin`; a role's `base`. |
| — | **tier role / group role** | A role with a `base` (how much) / without one (where a person belongs). |
| — | **base** (Workspace) | The one shared, read-only system tree of the sandbox. |
| — | **shared folder** (Workspace) | A work folder several people may use, named by a role setting. |
| account | **actor** / **person id** | Who caused an event as a body reports it (`whatsapp:628…`, `face:3`, or *unknown*) / the one opaque id of a person across actors (`person:p_7f3a`), registered automatically; linking merges two person ids into the older (D65, D86). |
| space | **place** | Where an event happens and an action lands: a chat, a room, a document, a device, a game region (contract.md §2). |
| members | **audience** | Who can perceive what happens in a place, reported by the adapter with `place.audience`; may include *unknown*. The one fact privacy needs (D85, D86). |
| place info, `place_kind` | — | Removed in contract v2 (D86): the audience replaces them; names and structure are part of `description` or `prompt_section`. |
| — | **feature** | A number on an event that the gate reads: `addressed`, `named`, `addressed_other`, `audience_size` (contract.md §3). |
| — | **receipt** | The one `act.result` that ends every action: `done`, `refused`, `held`, `failed` or `unknown` (contract.md §3). |
| — | **lifecycle event** | What the middleware tells a body the soul is doing (`composing`, `seen`, `progress`, `thought`); the body renders it its own way (contract.md §4.5). |
| — | **prompt section** | What a body tells reasoning about itself, as text (contract.md §4.1). |
| — | **contract** | The one locked contract between the layers, `docs/contract.md`, as code in `monoagens/contract/`, held by `contract.lock` (D86–D88). |
| — | **back-fill / import** | Events an adapter missed while offline (`backfill`) / history from before it was connected (`imported`, never a wake) (D69). |
| — | **format version** | `version:` in an owner's YAML file, `manifest_version:` in a plugin manifest (D58). |

## What never changes with a rename

Names stored in data are part of the frozen contract ([constitution.md](constitution.md) §1):
event kinds (`percept`, `decision.wake`, `expectation.reminded`, …), payload fields, `soul.yaml`
keys, the `prior/` format and `config.yaml` keys. A rename touches code only.

## Layout

| Folder | Holds | Changes |
|---|---|---|
| `monoagens/contract/` | The contract as code (D88): event envelope, features, capability descriptor, hooks, context items, actions, receipts, the adapter surface, percepts, lifecycle events. Held by `contract.lock`. | Only by the owner (contract.md §9). |
| `monoagens/core/` | Mechanisms only: kernel loop, gate, reasoning, context, memory, expectations, threads, the tool door (`dispatch`), the three hooks. | Rarely: evidence, bugs, vulnerabilities. |
| `monoagens/middleware/` | Every decision, generic, in seven files ([middleware.md §1](middleware.md)): `policy.py`, `privacy.py`, `identity.py`, `registry.py`, `loader.py`, `interaction.py`, `roles.py`. | Bugs inside; never for one platform or case. |
| `monoagens/adapters/` | One folder per adapter: all translation lives here. | Per app or device. |
| `monoagens/infra/` | Store, migrations, config, clock, budget counting, embedding, versioning, attachments, model providers (`infra/providers/`). | With the stack. |
| `plugins/` | Installable features on the middleware. | Per feature. |

The rule between the layers: **adapters translate, the middleware decides from facts, the core
records and carries out.**
