# AGENTS.md

MonoAgens is one persistent AI agent (a soul) that lives in many apps and devices: a generic
core, a generic middleware, and adapters and plugins that translate their own world into one
locked contract. This file is how you work here; the design is in `docs/spec.md`.

## Read first
- `docs/spec.md`: *Principles* and *The contract*, always; then the sections your task names.
- Your task file in `docs/tasks/`. Work only on that task.

## Never
1. **Never read, print, copy or summarise `.env`** or any file holding secrets. Learn the
   variable names from `.env.example`; check that one is set by presence only
   (`test -n "$LLM_API_KEY"`). Never put a secret in a command, a log, a test, a commit, a report
   or a prompt. If one is exposed anyway, stop and tell the owner which one.
2. **Never change the contract**: `monoagens/contract/` and `contract.lock` belong to the owner.
3. **Never touch a file** outside your part's **May touch** line.
4. **Never add a dependency** the owner has not approved; the approved ones are those in
   `pyproject.toml`.
5. **Never edit** `docs/spec.md`, `docs/research.md` or this file. In your own task file you may
   tick boxes and write under **Notes**.

## How to work
- Work through the task's parts in order, one commit per item when practical.
  `git pull --rebase` before you start and before you push.
- Write all of the task's code first; the tests come at the end. Then run `uv run pytest` and
  `uv run python scripts/replay.py --fake`, collect every failure, and fix them together until
  both pass. The task is done only then.
- Group the failures by root cause and fix each cause once, in its one home; when one failure
  repeats across many tests, fix the rule that produced it, never each test.
- A behavior change ships with a replay scenario that carries all its material as synthetic data
  (facts, people, files, realistic tool results). A failure is the soul's only after you checked
  that what it needed was in the Log and in its context.
- A case the code does not handle is classified before anything is written: a wrong fact from an
  adapter, the principles or a body's prompt section, a broken mechanism — or a question for the
  owner. Never patch a single case into the core or the middleware.
- A question the spec answers is not a question: decide from the spec and name the source under
  Notes. Stop only for something the contract does not cover, or a step that failed twice after
  two different fixes.

## Tests
- A real Postgres (the container from `compose.yaml`); a fake model, fake HTTP
  (`httpx.MockTransport`) and a fake clock. The real model only in a task's last part, through
  `scripts/replay.py` without `--fake`.
- Tests speak through the contract: they feed percepts and check actions, receipts and events.
  No private functions, no internal payloads, no prompt wording.
- The architecture tests, the contract lock and the ratchet hold the generic rules. When one
  fails, fix the code — never the test, the lock or the baseline.

## Code
- Python 3.13 with `uv`. Code, comments, logs and docs in English; persona text in `soul.yaml`
  may be in any language.
- Readable first: lines of at most 100 characters, one statement per line, clear names,
  docstrings on public functions. Never compress code to save lines. A module past about 300
  lines may be holding two jobs.
- The engineering invariants are in `docs/spec.md` (*Principles → Engineering invariants*).
- Gotchas: call `register_vector_async` on every async connection; pytest-asyncio 1.x runs with
  `asyncio_mode = "auto"` and has no `event_loop` fixture.
- Use GitNexus to see a change's impact (callers, dependencies) before you move code. Revert any
  edit it makes to `AGENTS.md` or `CLAUDE.md`, and never commit its index. Context7 gives current
  library docs.
