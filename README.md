# MonoAgens

A tiny kernel for one persistent AI self that lives in many bodies. The kernel records
everything it sees, holds Expectations about the world, and only thinks hard (calls the LLM)
when the cognition layer's appraisal decides an event matters.

- [`docs/constitution.md`](docs/constitution.md) — the rules that never change (read first).
- [`docs/core.md`](docs/core.md) — the core specification (final, frozen); formulas and sources
  in [`docs/core-math.md`](docs/core-math.md).
- [`docs/`](docs/README.md) — one specification per layer: middleware, bodies, plugins, birth,
  project.
- [`docs/tasks/`](docs/tasks/README.md) — work for coding workers, in order.
- [`AGENTS.md`](AGENTS.md) — rules and invariants for coding agents.
- `scripts/replay.py` — scenario replay for tuning the cognition layer.

## Tuning with replay

The replay tool feeds written scenarios through the real kernel on a throwaway database, so
the cognition settings in `soul.yaml` can be tuned without a week of manual chatting.

```sh
# Real LLM: spends tokens, prints the token report at the end.
uv run python scripts/replay.py

# Fake LLM and fake embeddings: free, only checks appraisal decisions.
uv run python scripts/replay.py --fake

# One scenario only.
uv run python scripts/replay.py scenarios/01_group_chitchat_greeting.yaml
```

The replay database comes from `REPLAY_DATABASE_URL` and its name must end with `_replay`;
the tool creates it when missing and rebuilds its schema for every scenario, so it can never
touch the production database. Real mode reads `LLM_BASE_URL`, `LLM_API_KEY`, and `LLM_MODEL`
from `.env` (or the environment).

Every run writes `reports/replay-<YYYYMMDD-HHMMSS>.md`. Each report has, per scenario, the
step-by-step inputs with the appraisal decision and its numbers (`reflex`, `s`, `theta`, `p`,
`u`, `reason`), the emitted replies, the PASS/FAIL list for the scenario's expectations, a
summary table for all scenarios, and the total LLM calls with the prompt, cached, completion,
and total token counts. Compare reports before and after a `soul.yaml` change to see which
decisions moved.

`--fake` never creates Expectations (when woken it always replies "ok" through the fake organ), so reminder
and recall scenarios are expected to FAIL there. It is for checking appraisal for free.

Tuning guide for `soul.yaml`: in this foundation the decision is reflex-driven (addressed
messages and fired promises wake the PFC) and the signal weights are placeholders, so the
report's `s`, `theta`, `p`, and `u` columns stay at their reflex values. As signals and the
dynamic threshold land, tune `cognition.weights` and `cognition.threshold`; the seed in
`cognition.seed` keeps every draw reproducible in replay.
