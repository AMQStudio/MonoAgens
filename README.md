# MonoAgens

A tiny kernel for one persistent AI self that lives in many bodies. The kernel records
everything it sees, holds Expectations about the world, and only thinks hard (calls the LLM)
when the gate decides an event matters.

- `DESIGN.md` — the design (read this first).
- `PLAN.md` — the build steps.
- `AGENTS.md` — working rules and invariants.
- `scripts/replay.py` — scenario replay for tuning the gate.

## Tuning with replay

The replay tool feeds written scenarios through the real kernel on a throwaway database, so
gate weights and bands in `soul.yaml` can be tuned without a week of manual chatting.

```sh
# Real LLM: spends tokens, prints the token report at the end.
uv run python scripts/replay.py

# Fake LLM and fake embeddings: free, only checks gate decisions.
uv run python scripts/replay.py --fake

# One scenario only.
uv run python scripts/replay.py scenarios/01_group_chitchat_greeting.yaml
```

The replay database comes from `REPLAY_DATABASE_URL` and its name must end with `_replay`;
the tool creates it when missing and rebuilds its schema for every scenario, so it can never
touch the production database. Real mode reads `LLM_BASE_URL`, `LLM_API_KEY`, and `LLM_MODEL`
from `.env` (or the environment).

Every run writes `reports/replay-<YYYYMMDD-HHMMSS>.md`. Each report has, per scenario, the
step-by-step inputs with the gate decision and its score breakdown (`addressed`, `surprise`,
`cooldown_penalty`, `crowd_penalty`, `score`), the emitted replies, the PASS/FAIL list for the
scenario's expectations, a summary table for all scenarios, and the total LLM calls with the
prompt, cached, completion, and total token counts. Compare reports before and after a
`soul.yaml` change to see which decisions moved.

`--fake` never creates Expectations (it always answers `say("ok")` when woken), so reminder
and recall scenarios are expected to FAIL there. It is for checking the gate for free.

Tuning guide for `soul.yaml`:

- Bot is too quiet: lower `gate.band.wake` (e.g. 0.6 → 0.45), decrease the
  `cooldown_penalty` weight, or decrease `cooldown_minutes`.
- Bot is too chatty: raise `gate.band.wake`, increase the `crowd_penalty` weight
  (e.g. -0.2 → -0.4), or raise `gate.band.silent`.
