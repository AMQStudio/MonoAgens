# AGENTS.md — Standing Rules for Coding Agents

This project is **MonoAgens**: an AI agent built as one persistent self that lives in many
bodies, with a small core organised like an OS kernel (see README).
Status: prototype. Before touching code read [docs/constitution.md](docs/constitution.md), the
task file you were given in [docs/tasks/](docs/tasks/README.md), and the spec sections its parts link (the core: [docs/core.md](docs/core.md)). Work ONLY on that task.

## Who writes code
- **One task = one worker**, and tasks run one after another. The worker is an **Orca
  Orchestration** agent `omp` with model **`opencode-go/deepseek-v4.1-flash`**, reasoning effort
  **high**, on OpenCode Go. There is no coordinator in between: the worker reads the task, the
  specs and the code itself, implements, verifies its own result (reads its diff, runs
  `uv run pytest` and the fake replay, checks the invariants and the size guide), commits,
  pushes and reports to the owner. The real-model run of the last part is the worker's, once.
- Tools the worker may use when they help: **graphify** (the code graph: callers and
  dependencies, before moving or renaming code) and **Context7** (current library docs:
  psycopg, pgvector, fastembed, httpx, pytest-asyncio). Neither is a project dependency.
- If the same step fails twice, stop and report to the owner what was tried; do not work around
  it.
- Specifications (every `docs/*.md`) and this file are owned by the architect. Workers may tick
  boxes and write under **Notes** in their own task file in `docs/tasks/`; they never edit a
  specification or the status table.
- **The core's contract is frozen and its formulas change only with evidence**
  (`docs/constitution.md` §1). No task, worker or plugin changes the core design; a need the
  core cannot meet stops and goes to the owner.

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
- If a design question comes up, STOP and write the question down. Do not guess.

## Language
- Code, identifiers, comments, docstrings, logs, error messages, docs: English.
- Persona text in `soul.yaml` may be in any language.

## Invariants (never break these)
1. **The core is only a brain** (D1, D2, D32). `monoagens/kernel.py`, `hooks.py`, `jobs.py`,
   `migrate.py`, `store.py`, `expect.py`, `memory.py`, `pfc.py`, `desk.py`, `guidance.py`, `reach.py`, `threads.py`, `recall.py`, `identity.py` and `monoagens/cognition/` (after Task 3: everything under `monoagens/core/`) know only senses, organs, hooks and thought
   acts. They never import from `monoagens/bodies/` or packages, never mention a platform, and
   never hard-code a world action (speaking included) — those are organ capabilities.
2. **The log is immutable.** The `events` table is INSERT-only, enforced by a trigger. The only
   exception is hard forgetting (`docs/core.md` §7), through one dedicated function that the trigger
   recognizes. `event_vectors` rows are deleted only by fading or hard forgetting, each
   through its own dedicated path that writes an event.
3. **One queue, one consumer.** Every source (bodies, clock) only `put`s into the queue.
   Only the kernel loop processes events, one at a time, in order.
4. **Record first, then act.** Incoming events are written to `events` before they are
   processed. Decisions to stay silent are written too.
5. **Every model call goes through a gate and is accounted for** (D23). No model call per
   message or per tick by default; each call is a gate decision, is recorded as an event with
   its purpose and token usage, and counts toward the daily budget.
6. **No scenario features.** No "reminder", "monitor", or "habit tracker" modules. All of them
   are Expectations (see `docs/core.md` §5). If an idea cannot be an Expectation, a state key, or a body
   adapter, it does not go in.
7. **Pure async.** No blocking calls inside `async def`. CPU work (embedding) goes through
   `asyncio.to_thread`. Every hook call and background job runs under its watchdog timeout
   (`docs/core.md` §3).
8. **Configuration lives outside code.** Models, keys, URLs and the optional reasoning effort come from `.env`; deployment settings
   from `config.yaml`. Identity comes from the birth certificate (`soul.yaml`); weights and thresholds from the prior in `prior/` (D35).
   No magic numbers in code.
9. **Signals are pure.** A cognition signal only reads its `Situation` and returns a number in
   [0, 1]. No writes, no LLM calls, no I/O beyond what `Situation` already holds.
10. **Decisions are reproducible.** Random draws come only from the seeded generator described
   in `docs/core.md`, and all time comes from the injected clock (replay uses event times); never from
   `random`, `datetime.now()` or `time.time()` directly.
11. **Complexity budget** (D26, D32). The core holds at most 4 books, 5 thought acts
   (think, act, expect, resolve, revise), 3 hooks (inbound, context, action), 10 signals,
   3 drives and one gate formula. A change that needs another core
   primitive stops and goes to the architect.

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
- Dev dependencies: `pytest`, `pytest-asyncio >= 1.3` (1.x: set `asyncio_mode = "auto"` in
  `pyproject.toml`; do not use the removed `event_loop` fixture).
- No new dependency, ORM, agent framework, or platform SDK without the owner's approval.

## Priorities for code
1. **The goal works**: the behavior the task asks for, proven by its scenarios.
2. **Readable and easy to refactor**: the next reader understands it in one pass and can change
   it safely.
3. Small is good only when it serves 1 and 2. Saving lines is never a reason for a design choice.

## Size guide (excluding tests)
Readability comes first. The real limit on the core is the complexity budget in
`docs/constitution.md` §3 (books, thought acts, hooks, signals, drives, one gate formula), not a
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
- Tests use a real Postgres (the container from `compose.yaml`), never a mocked database.
- The LLM and HTTP are always fake in tests (scripted FakeLLM, `httpx.MockTransport`). The real
  model from `.env` is used only in a task's last part, through `scripts/replay.py` without
  `--fake` and a live session.
- The clock is always a fake clock that tests can advance.
- Scenarios carry all their material as synthetic data the worker writes: facts, people, artifacts,
  realistic results from every test organ, and the capabilities the behavior needs (if the task
  is "send the file", a test organ must be able to read and send that file). Before reporting a failure as the soul's, check
  that what it needed was in the Log and on its desk; if not, fix the scenario (D54).