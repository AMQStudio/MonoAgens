# ROADMAP.md — From MVP to the Final Goal

The final goal: **one persistent self that lives in many bodies, talks to many people safely,
runs on any model, grows through plugins, and installs with one command.**

LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the brain.

## The rule for every phase
1. A phase is done only when its "done when" checks pass **and** every earlier test still passes.
2. **The kernel does not change to make a feature fit.** New capabilities arrive as an
   Expectation, a state key, a body, a tool, or middleware. If a phase seems to need a kernel
   change, stop and bring it to the architect first; the feature design is the likely problem.
3. One phase at a time. Nothing from a later phase sneaks into an earlier one.

## Phase 0 — MVP (done)
Telegram body, the four books, the gate, the PFC, Expectations, memory and sleep.
Tests (a)–(d) in `tests/test_mvp.py`.

## Phase 1 — Launch week
| Day | Work | Done when |
|---|---|---|
| 1 | Scenario replay with a real LLM (`scripts/replay.py`, `scenarios/`) | First report exists; failures understood |
| 2 | Tune gate weights from replay reports; clean-ups (drop `PFC_*` aliases) | Replay passes ≥ 90% of expectations |
| 3–4 | **CLI body** + recent context **per person**, not only per place | Start a topic on Telegram, continue it on the CLI; an expectation created on Telegram can be fulfilled on the CLI |
| 5 | **Installer**: one script for Postgres (compose), schema, `.env` from `.env.example`, `soul.yaml` | A clean machine goes from clone to a talking bot with one command |
| 6 | README and usage docs | A new user can install and talk to it without asking anyone |
| 7 | Buffer for bugs | All tests and the replay pass |

Open decision before day 5: **public launch or owner-only?** If public, an owner-only guard
(only the owner can give instructions; everyone else is just someone it may talk to) ships with
the launch, as the first piece of Phase 4.

## Phase 2 — Fully proactive
- Passive expectations from facts: important facts about people create an Expectation that the
  fact stays true; a contradicting event revises the memory (`valid_to`, `superseded_by`).
- Simple sensors as bodies (e.g. a server ping body emitting events).
- Done when: "server down" → one message, not sixty; a stale fact is revised after a
  contradiction; STALE-style replay scenarios pass.

## Phase 3 — Plugins and world tools
- One plugin mechanism for three kinds of extension: **bodies**, **tools** for the PFC, and
  **middleware**. Plugins register from outside; the kernel only sees the registry.
- First world tools: web search/fetch, file read. Tool results are read by the PFC, so this
  phase introduces the think → tool → think loop, with a hard limit on rounds per wake.
- Done when: a plugin installed without touching `monoagens/` adds a working tool.

## Phase 4 — The guard post (multi-user safety)
- Middleware between the queue and the kernel, and between the PFC and bodies.
- Roles (who may instruct what), approvals for dangerous actions, group privacy
  (what may be said in which place), all reading the `who` / `place` columns that exist today.
- Personal mode keeps the pipeline empty: zero friction.
- Done when: the same soul serves the owner fully in a DM and safely in a public group, and all
  Phase 0–3 tests pass with the guard installed.

## Phase 5 — More bodies
- WhatsApp, Discord, voice (STT/TTS), and the first non-chat body (IoT device or robot sim).
- Done when: one conversation continues across three bodies, and a non-chat body both senses
  (events in) and acts (actions out) through the same two ports.

## Phase 6 — Native providers
- Native adapters next to the OpenAI-compatible one (e.g. Anthropic, for its prompt caching).
- Optional separate model for sleep (`SLEEP_MODEL`, falls back to `LLM_MODEL`).
- Done when: switching provider is a `.env` change, and replay results stay within tolerance.

## Phase 7 — Product layer
- Operator CLI (start/stop/status/logs/doctor), presets ("apps": personal assistant,
  sysadmin, community bot = a soul + bodies + guard settings), upgrade path for the schema.
- Done when: a preset can be installed and running in minutes.

## Later ideas (not scheduled)
- Learned gate: a small model trained on `decision.silent` vs "why so quiet?" moments.
- Skills: reusable procedures learned from repeated work (the basal ganglia of the system).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
