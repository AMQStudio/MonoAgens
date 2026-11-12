# MonoAgens

MonoAgens is an AI agent built as **one persistent self that lives in many apps and devices** — chat
apps, a terminal, later devices and robots. Most agent frameworks start from "a message
arrived, run the model". MonoAgens starts from "something happened, record it, then decide
whether it is worth thinking about": it remembers across apps and time, keeps its promises,
knows when to stay quiet in a group, and can replay every decision it made.

**Built like the Linux kernel.** Three layers, each with one job:
- **Core** — neutral and primitive, only a brain: perception, attention, memory, expectations,
  a cheap decision to act, and reasoning (the LLM) woken only when a moment deserves it. It knows
  no case and no person, only opaque ids, labels and numbers, and restricts nothing itself.
- **Middleware** — the regulator, like drivers: privacy, identity, the owner policy, limits,
  tools and commands, install. It is the only path in and out of the core and keeps it from
  running wild.
- **Adapters and plugins** — Telegram, WhatsApp, CLI, later devices and robots; search, files,
  modes. They talk only to the middleware.

The core keeps a stable contract with everything around it: it can improve inside without
breaking what is built on top.

> **Note, to avoid confusion.** "Kernel" here is an analogy for how the code is organised, not a
> literal one: MonoAgens is not an operating system, not a scheduler that serves many agents, and
> not an SDK for building other agents. It is one agent that persists. Words borrowed from the
> brain ("memory", "attention", "sleep", "soul") name engineering mechanisms; they are not claims
> of consciousness or feelings. *Agens* is Latin for "the one who acts", the root of *agent*.

- [`docs/constitution.md`](docs/constitution.md) — the rules that never change (read first).
- [`docs/core.md`](docs/core.md) — the core specification (contract frozen); formulas and sources
  in [`docs/core-math.md`](docs/core-math.md).
- [`docs/`](docs/README.md) — one specification per layer: middleware, adapters, plugins, project;
  standard names in [`docs/glossary.md`](docs/glossary.md).
- [`prior/`](prior/) — the default instincts (knob values) a new soul is born with; tune with replay.
- [`docs/tasks/`](docs/tasks/README.md) — work for coding workers, in order.
- [`AGENTS.md`](AGENTS.md) — rules and invariants for coding agents.
- `scripts/replay.py` — scenario replay: tests, metrics and tuning.

## Tuning with replay

The replay tool feeds written scenarios through the real kernel on a throwaway database, so
the values in the prior (`prior/`) can be tuned without a week of manual chatting.

```sh
# Real LLM: spends tokens, prints the token report at the end.
uv run python scripts/replay.py

# Fake LLM and fake embeddings: free; checks the logic with scripted answers.
uv run python scripts/replay.py --fake

# One scenario only.
uv run python scripts/replay.py scenarios/silence/01_group_chitchat_greeting.yaml
```

The replay database comes from `REPLAY_DATABASE_URL` and its name must end with `_replay`;
the tool creates it when missing and rebuilds its schema for every scenario, so it can never
touch the production database. Real mode reads `LLM_BASE_URL`, `LLM_API_KEY`, and `LLM_MODEL`
from `.env` (or the environment).

Every run writes `reports/<YYYY-MM-DD-HHMM>-<real|fake>-<scenarios>.md` and `reports/latest.md`;
the newest ten are kept. Each report has, per scenario, the step-by-step inputs with the gate's
decision and its numbers (`reflex`, `s`, `theta`, `p`, `u`, `reason`), the replies, the PASS/FAIL
list for the scenario's checks, a summary table, the headline metrics (missed help, false
speech, tokens per day, absolute-line violations, honesty) and the token counts. The lasting
record of what holds with the fake and the real model is `scenarios/README.md`.

Weights and thresholds live in the prior (`prior/`), not in code: change a value, replay, and
compare reports. The seed in the prior keeps every draw reproducible.
