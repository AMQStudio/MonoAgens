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
  running wild. Like the core it is generic: it knows no platform and translates nothing.
- **Adapters and plugins** — Telegram, WhatsApp, Discord, CLI, later devices, robots and apps;
  search, files, modes. Each translates its own world into the contract's facts and talks only
  to the middleware.

One locked contract ([`docs/contract.md`](docs/contract.md)) sits between the layers: six
generic facts (actor, place, audience, event, capability, receipt) that fit a chat, a robot, a
workspace app, a sensor and a game alike. The inside can improve without breaking what is built
on top, and something new enters only as an adapter or a plugin.

> **Note, to avoid confusion.** "Kernel" here is an analogy for how the code is organised, not a
> literal one: MonoAgens is not an operating system, not a scheduler that serves many agents, and
> not an SDK for building other agents. It is one agent that persists. Words borrowed from the
> brain ("memory", "attention", "sleep", "soul") name engineering mechanisms; they are not claims
> of consciousness or feelings. *Agens* is Latin for "the one who acts", the root of *agent*.

- [`docs/constitution.md`](docs/constitution.md) — the rules that never change (read first).
- [`docs/contract.md`](docs/contract.md) — the one locked contract between the layers.
- [`docs/core.md`](docs/core.md) — the core specification (contract frozen); formulas and sources
  in [`docs/core-math.md`](docs/core-math.md).
- [`docs/`](docs/README.md) — one specification per layer: middleware, adapters, plugins, project;
  standard names in [`docs/glossary.md`](docs/glossary.md).
- [`prior/`](prior/) — the default instincts (knob values) a new soul is born with; tune with replay.
- [`docs/tasks/`](docs/tasks/README.md) — work for coding workers, in order.
- [`AGENTS.md`](AGENTS.md) — rules and invariants for coding agents.
- `scripts/replay.py` — scenario replay: tests, metrics and tuning.

## Install

MonoAgens runs as one long-lived process; its database, model keys and adapters come from
`.env` and `config.yaml`.

```sh
git clone <this repo> && cd MonoAgens
uv sync                                  # Python 3.13 and the five runtime packages
docker compose up -d db                  # PostgreSQL 17 with pgvector
cp .env.example .env                     # then fill LLM_*, DATABASE_URL and the adapter keys
uv run python -m monoagens               # the soul
uv run python -m monoagens.cli           # a terminal body, in another terminal
```

Plugin dependencies stay with their plugin: `uv pip install ddgs pypdf` enables web search and
the PDF reader, and the Workspace sandbox needs the system tools `bubblewrap` and `passt`
(`pasta`). Prepare its base once, before the first `workspace.run`:

```sh
uv run python plugins/workspace/prepare.py --root .
```

**Run it as a dedicated system user** (for example `monoagens`), not as your desktop account.
The sandbox bounds a run with `RLIMIT_NPROC`, and that limit counts every task of the account the
process runs under: on a desktop account the browser, the editor and the session itself share the
budget, so the sandbox is limited against them and a busy desktop can make a run fail before the
sandbox limit is reached. Its own account keeps the count to MonoAgens and its runs.

```sh
sudo useradd --system --create-home --home-dir /srv/monoagens --shell /usr/sbin/nologin monoagens
sudo install -d -o monoagens -g monoagens /srv/monoagens
sudo -u monoagens git clone <this repo> /srv/monoagens/app && cd /srv/monoagens/app
```

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
touch the production database. Real mode uses the model provider set in `.env` (`LLM_PROVIDER`:
`openai-compatible` default or `opencode-go`, `LLM_MODEL`, optional `LLM_FALLBACKS` using
`<PROVIDER>_API_KEY` / `<PROVIDER>_MODEL`; see `.env.example`), or the environment.

Every run writes `reports/<YYYY-MM-DD-HHMM>-<real|fake>-<scenarios>.md` and `reports/latest.md`;
the newest ten are kept. Each report has, per scenario, the step-by-step inputs with the gate's
decision and its numbers (`reflex`, `s`, `theta`, `p`, `u`, `reason`), the replies, the PASS/FAIL
list for the scenario's checks, a summary table, the headline metrics (missed help, false
speech, tokens per day, absolute-line violations, honesty) and the token counts. The lasting
record of what holds with the fake and the real model is `scenarios/README.md`.

Weights and thresholds live in the prior (`prior/`), not in code: change a value, replay, and
compare reports. The seed in the prior keeps every draw reproducible.
