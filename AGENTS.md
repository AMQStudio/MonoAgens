# AGENTS.md — Standing Rules for Coding Agents

This project is **MonoAgens**: a tiny kernel for one persistent AI self that lives in many bodies.
Status: prototype. Read `DESIGN.md` before touching code. Work ONLY on the step or track you were given
from `PLAN.md` or `ROADMAP.md`.

## Who writes code
- All implementation work is delegated to an **Orca Orchestration** worker running
  agent `omp` with model **`opencode-go/deepseek-v4.1-flash`**, effort **low**, on OpenCode Go.
- The session that receives a PLAN.md step acts only as **coordinator**: it hands the step
  to the worker, then verifies the result (reads the diff, runs `uv run pytest`, checks the
  invariants and the size budget) before committing and reporting.
- The coordinator does not write implementation code itself. If the worker fails twice on
  the same step, stop and report to the owner instead of taking over.
- Architecture and design documents (`DESIGN.md`, `DECISIONS.md`, `PLAN.md`, `ROADMAP.md`, this file) are owned by the
  architect. Workers may only tick steps in PLAN.md and never edit design decisions.

## Secrets (never break this)
- Never read, print, cat, grep, copy, or summarize `.env` or any file holding secrets.
  This applies to the coordinator and to every worker.
- To learn which variables exist, read `.env.example`. To check that a variable is set,
  test for its presence only (e.g. `test -n "$LLM_API_KEY"`), never print its value.
- Never put a secret in a command line, a log, a test, a commit, a report, or a prompt.
- If a secret is exposed anyway, stop and tell the owner which one, so it can be rotated.

## How to work
- One session = one PLAN.md step = one commit.
- Do not start the next step. Do not add features that were not asked for.
- A step is done only when all tests pass (`uv run pytest`). Tick the step in PLAN.md.
- If a design question comes up, STOP and write the question down. Do not guess.

## Language
- Code, identifiers, comments, docstrings, logs, error messages, docs: English.
- Persona text in `soul.yaml` may be in any language.

## Invariants (never break these)
1. **The kernel is body-blind.** `monoagens/kernel.py`, `store.py`, `expect.py`, `memory.py`,
   `pfc.py` and everything in `monoagens/cognition/` must not import anything from
   `monoagens/bodies/` or mention "telegram", "cli", etc.
2. **The log is immutable.** The `events` table is INSERT-only. No UPDATE, no DELETE.
   Enforced by a database trigger.
3. **One queue, one consumer.** Every source (bodies, clock) only `put`s into the queue.
   Only the kernel loop processes events, one at a time, in order.
4. **Record first, then act.** Incoming events are written to `events` before they are
   processed. Decisions to stay silent are written too.
5. **The LLM is only called through the gate.** No LLM call per message or per tick.
6. **No scenario features.** No "reminder", "monitor", or "habit tracker" modules. All of them
   are Expectations (see DESIGN.md). If an idea cannot be an Expectation, a state key, or a body
   adapter, it does not go in.
7. **Pure async.** No blocking calls inside `async def`. CPU work (embedding) goes through
   `asyncio.to_thread`.
8. **Configuration lives outside code.** Models, keys, URLs come from `.env`. Weights,
   thresholds, persona come from `soul.yaml`. No magic numbers in code.
9. **Signals are pure.** A cognition signal only reads its `Situation` and returns a number in
   [0, 1]. No writes, no LLM calls, no I/O beyond what `Situation` already holds.
10. **Decisions are reproducible.** Random draws come only from the seeded generator described
   in DESIGN; never from `random` or the clock directly.

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

## Size budget (excluding tests)
kernel.py ≤ 280 lines · store.py ≤ 240 · expect.py ≤ 240 · memory.py ≤ 260 · pfc.py ≤ 280 ·
gate.py ≤ 120 (removed once cognition replaces it) · bodies/telegram.py ≤ 180 ·
bodies/cli.py ≤ 120 · cli.py ≤ 60 · cognition/ ≤ 600 in total, each file ≤ 160 ·
middleware.py ≤ 150 · whole package ≤ 2,600 lines.
Counted on readable code: lines ≤ 100 characters, one statement per line, blank lines between
functions and classes, docstrings on public functions. Compressing code to fit the budget is a
violation, not a fix.
Going over budget means the design is wrong. Stop and report; do not force it.

## Parallel tracks
- A track touches only the files its ROADMAP row names. Shared files (`kernel.py`,
  `store.py`, `sql/schema.sql`, `soul.yaml`) are changed only by Track A, or by a track that
  says so explicitly; other tracks that need a change there stop and report.
- Always `git pull --rebase` before starting and before pushing.

## Tests
- Tests use a real Postgres (the container from `compose.yaml`), never a mocked database.
- The LLM and HTTP are always fake in tests (scripted FakeLLM, `httpx.MockTransport`).
- The clock is always a fake clock that tests can advance.