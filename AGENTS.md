# AGENTS.md — Standing Rules for Coding Agents

This project is **MonoAgens**: an AI agent built as one persistent self that lives in many
apps and devices, with a small core organised like an OS kernel (see README).
Status: prototype. Before touching code read [`docs/spec.md`](docs/spec.md) §1 (principles) and
§2 (the locked contract), the task file you were given in [docs/tasks/](docs/tasks/README.md),
and the spec sections its parts name. Work ONLY on that task. Formulas and their sources are in
[`docs/research.md`](docs/research.md).

## Who writes code
- **One task = one worker**, one task after another. The owner gives the task prompt to the
  worker directly.
- Tools the worker may use when they help: **graphify** (the code graph: callers and
  dependencies, before moving or renaming code) and **Context7** (current library docs:
  psycopg, pgvector, fastembed, httpx, pytest-asyncio). Neither is a project dependency.
- If the same step fails twice, stop and report to the owner what was tried; do not work around
  it.
- The specification (`docs/spec.md`), `docs/research.md` and this file are owned by the architect. Workers may tick
  boxes and write under **Notes** in their own task file in `docs/tasks/`; they never edit a
  specification or the status table.
- **The contract is locked** (`docs/spec.md` §2, D86–D88) and the core's
  formulas change only with evidence (`docs/spec.md` §1.1). No task, worker or plugin
  changes the contract or `contract.lock`; a need the contract cannot meet stops and goes to the
  owner. A new case is classified first (`docs/spec.md` §2.9): adapter fact, principles or
  `prompt_section`, mechanism bug, or a question for the owner. It is never patched into the
  core or the middleware.

## Secrets (never break this)
- Never read, print, cat, grep, copy, or summarize `.env` or any file holding secrets.
  This applies to every worker and every session.
- To learn which variables exist, read `.env.example`. To check that a variable is set,
  test for its presence only (e.g. `test -n "$LLM_API_KEY"`), never print its value.
- Never put a secret in a command line, a log, a test, a commit, a report, or a prompt.
- If a secret is exposed anyway, stop and tell the owner which one, so it can be rotated.

## How to work
- One worker = one task file. Work through its parts in order; one commit per item
  when practical. Do not start the next task. Do not add features that were not asked for.
- An item is done only when all tests pass (`uv run pytest`) and the fake replay passes. Tick it
  in the task file.
- If a design question comes up that the spec does not answer, STOP and write the question
  under the task's **Notes**. Do not guess.
- Every behavior change ships with at least one replay scenario that shows it (scenario rules
  under Tests).
- Tests always use the fake LLM; the real model is used only in a task's last part, through the
  replay script and a live session.

## Language
- Code, identifiers, comments, docstrings, logs, error messages, docs: English.
- Persona text in `soul.yaml` may be in any language.

## Invariants (never break these)
1. **The core is only a brain** (D1, D2, D32). `monoagens/core/` (everything under it) knows only
   percepts, capabilities, hooks and thought acts. It never imports from `monoagens/adapters/` or
   plugins, never mentions a platform, and never hard-codes a world action (speaking included) —
   those are tool capabilities. From Task 3 on (D57) the core also knows only opaque ids, labels
   and numbers: it never imports from `monoagens/middleware/` and never means a person, the owner
   or a group. From Task 5 on (D64) it also knows no model provider: it calls the injected model
   port, and provider drivers live in `monoagens/infra/providers/`. From Task 6 on (D86) it never
   reads an event's `payload` or `labels`: it reads ids, times, `description` and `features`.
2. **The log is immutable.** The `events` table is INSERT-only, enforced by a trigger. The only
   exception is hard forgetting (`docs/spec.md` §3.7), through one dedicated function that the trigger
   recognizes. `event_vectors` rows are deleted only by fading or hard forgetting, each
   through its own dedicated path that writes an event.
3. **One queue, one consumer.** Every source (adapters, clock) only `put`s into the queue.
   Only the kernel loop processes events, one at a time, in order.
4. **Record first, then act.** Incoming events are written to `events` before they are
   processed. Decisions to stay silent are written too.
5. **Every model call goes through a gate and is accounted for** (D23). No model call per
   message or per tick by default; each call is a gate decision, is recorded as an event with
   its purpose and token usage, and counts toward the daily budget.
6. **No scenario features.** No "reminder", "monitor", or "habit tracker" modules. All of them
   are Expectations (see `docs/spec.md` §3.5). If an idea cannot be an Expectation, a state key, or an adapter
   adapter, it does not go in.
7. **Pure async.** No blocking calls inside `async def`. CPU work (embedding) goes through
   `asyncio.to_thread`. Every hook call and background job runs under its watchdog timeout
   (`docs/spec.md` §3.3).
8. **Configuration lives outside code.** The model provider and its fallbacks, models, keys, URLs and the optional reasoning effort come from `.env`; deployment settings
   from `config.yaml` and `config.d/`. Identity comes from the birth certificate (`soul.yaml`); weights and thresholds from the prior in `prior/` (D35).
   No magic numbers in code.
9. **Signals are pure.** A cognition signal only reads its `Situation` and returns a number in
   [0, 1]. No writes, no LLM calls, no I/O beyond what `Situation` already holds.
10. **Decisions are reproducible.** Random draws come only from the seeded generator described
   in `docs/spec.md` §3.3, and all time comes from the injected clock (replay uses event times); never from
   `random`, `datetime.now()` or `time.time()` directly.
11. **Complexity budget** (D26, D32). The core holds at most 4 books, 5 thought acts
   (think, act, expect, resolve, revise), 3 hooks (inbound, context, action), 10 signals,
   3 drives and one gate formula. A change that needs another core
   primitive stops and goes to the architect.
12. **Generic only** (D86, D87; `docs/spec.md` §2.5). The core and the middleware never name or branch
   on a body, a platform or a device; they translate nothing (adapters and plugins do). No code
   in the core or the middleware tries to understand human language: no word lists, stopwords,
   keyword or substring matching on message text, regexes over rendered text. Decisions not left
   to the model are made from facts (actors, places, audiences, refs, capability facts), never
   from words. The middleware never guesses what the soul means: reasoning states it as an
   argument and the middleware checks it.
13. **One rule, one home** (D87). Each rule is checked in exactly one place (privacy in the
   context hook and on opening or forwarding an attachment; authority in the policy; liveness in
   the core loop). Never add a second check of a rule that is already checked elsewhere; if a
   fix seems to need one, stop and report.
14. **The contract lock** (D88). `monoagens/contract/` is the contract as code and
   `contract.lock` its hash; a test fails when they differ. Never edit either; only the owner
   does, with a decision in `docs/spec.md` §2.

## Stack and versions
- Python 3.13+, managed with `uv` (commit `uv.lock`).
- PostgreSQL 17 with the pgvector extension ≥ 0.8.1 (iterative index scans need ≥ 0.8.0)
  and `pg_trgm`. Use the official `pgvector/pgvector` image for pg17.
- Runtime dependencies (the only ones allowed):
  - `psycopg[binary,pool] >= 3.3, < 3.4`
  - `pgvector` (Python client; use `register_vector_async` on every async connection)
  - `fastembed`
  - `httpx >= 0.28, < 0.29`
  - `pyyaml`
- Dev dependencies: `pytest`, `hypothesis` (property-based and stateful tests of invariants,
  approved by the owner 2026-11-14), `pytest-asyncio >= 1.3` (1.x: set `asyncio_mode = "auto"` in
  `pyproject.toml`; do not use the removed `event_loop` fixture).
- No new dependency, ORM, agent framework, or platform SDK without the owner's approval.
- **Plugin dependencies** live with the plugin (declared in its manifest, installed only when it
  is enabled), never in the core's list. Approved by the owner (2026-11-12): `web-search`:
  `ddgs`; `pdf-reader`: `pypdf`; `workspace`: the system tools `bubblewrap` and `passt`
  (pasta), not Python packages.
- **Adapter dependencies** follow the same rule: an optional extra per adapter in
  `pyproject.toml`, installed only when that adapter is enabled. Approved by the owner
  (2026-11-13): `whatsapp`: `neonize` (with `segno` for the pairing QR, and the `protobuf`
  override neonize needs); `discord`: `discord.py`. Check the current versions against their
  live docs and pin a range.

## Priorities for code
1. **The goal works**: the behavior the task asks for, proven by its scenarios.
2. **Readable and easy to refactor**: the next reader understands it in one pass and can change
   it safely.
3. Small is good only when it serves 1 and 2. Saving lines is never a reason for a design choice.

## Size guide (excluding tests)
Readability comes first. The real limit on the core is the complexity budget in
`docs/spec.md` §1.4 (books, thought acts, hooks, signals, drives, one gate formula), not a
line count. Line numbers are only a signal for review:
- A core module that passes about **300 lines** is a sign it may hold two jobs; split it by
  responsibility when that makes it clearer, not to hit a number.
- When the whole package grows by more than about 10% in one task, say why in the task's Notes.
- Never compress code to save lines (several statements on one line, cryptic names, removed
  docstrings or blank lines, logic moved somewhere odd to stay under a number). That is a
  violation. Write code the next reader understands in one pass.
- Readable code: lines ≤ 100 characters, one statement per line, clear names, blank lines between
  functions and classes, docstrings on public functions, files end with a newline.
- `guidance.py` stays a short list of general principles, never a rule for one case; a new line
  needs evidence in the task's Notes.

## Files a part may touch
- Each part touches only the files its **May touch** line names. Shared files (`kernel.py`,
  `store.py`, `sql/migrations/`, `soul.yaml`) are changed only by a part whose **May touch**
  line says so explicitly; a part that needs a change there otherwise stops and reports.
- Always `git pull --rebase` before starting and before pushing.

## Tests
- **Tests speak through the contract only** (D88, `docs/spec.md` §2.7; from Task 6 Part 6 on). A test
  of the core or the middleware feeds percepts and checks actions, receipts and events. It never
  calls a private function (`_name`), never asserts on an internal payload, and never asserts on
  prompt wording; an architecture test refuses imports of private names in `tests/`. If you feel
  you must edit a contract test, you are changing the contract: stop and report.
- **Invariants over cases**: the rules of the core and the middleware are held by Hypothesis
  stateful tests (`docs/spec.md` §2.7); scenarios are for the soul's behavior, not for one case of a
  rule.
- Tests use a real Postgres (the container from `compose.yaml`), never a mocked database.
- The LLM and HTTP are always fake in tests (scripted FakeLLM, `httpx.MockTransport`). The real
  model from `.env` is used only in a task's last part, through `scripts/replay.py` without
  `--fake` and a live session.
- The clock is always a fake clock that tests can advance.
- Scenarios carry all their material as synthetic data the worker writes: facts, people, artifacts,
  realistic results from every test tool, and the capabilities the behavior needs (if the task
  is "send the file", a test tool must be able to read and send that file). Before reporting a failure as the soul's, check
  that what it needed was in the Log and in its context; if not, fix the scenario (D54).