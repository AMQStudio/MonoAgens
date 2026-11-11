# 001 — Core: align the code with the core spec (Track A2)
Status: todo | Wave: 1 | Depends on: —
Read first: [core.md](../core.md) §3–§9, [project.md](../project.md) §2
May touch: `monoagens/` (all), `sql/`, `soul.yaml`, `config.yaml` (new), `scripts/replay.py`, `scenarios/`, `tests/`, `prior/`

## Goal
Bring the merged Track A code in line with the frozen core spec. This is the only task allowed to change the shared files (`kernel.py`, `store.py`, `sql/schema.sql`, `soul.yaml`).

## Tasks
- [ ] T1 Thought acts `think, act, expect, resolve, revise`; `say`/`react` become Telegram and CLI capabilities (core.md §9.1); silence = a wake without `act`; `msg.in` becomes `percept` with a description and optional data (§9.2); `act.out`/`act.result` replace `msg.out`/`react.out` in code, tests and replay.
- [ ] T2 Three hooks (inbound, context, action) with owner-policy slots first and last, shared vs organ-own scope, collision check at boot.
- [ ] T3 Provenance and privacy: `origin_actor` on events (inherited along `caused_by`), `identity_changing` check in the action hook; `place` + `private` on memories and the context-hook filter.
- [ ] T4 Lineage: `derived_from` on memories, expectations and vectors; replace the `event_vectors` no-delete trigger with the two dedicated paths (fade, hard forget) — stubs that write their events are fine.
- [ ] T5 Budget: `model.call` for every model call; the budget term in θ and `budget.exhausted` (no thrift mode).
- [ ] T6 Expectation schema with the four kinds (`promise`, `wait`, `predict`, `guard`).
- [ ] T7 Injected clock everywhere (no `datetime.now()` in the core); replay uses event times.
- [ ] T8 Slow work off the loop: PFC, classifier, embedding and sleep as background jobs whose results return as events; one PFC job per place; re-wake once if a new event arrived meanwhile.
- [ ] T9 Numbered SQL migrations with a version table and a backup before migrating; pairing code for the first owner.
- [ ] T10 Identity as birth content: `soul.yaml` becomes the birth certificate; cognition numbers move to the default prior in `prior/` as knobs / calibration / constants (core.md §9.4); the rest of today's `soul.yaml` splits by project.md §2: `timezone`, sleep, clock, budget and enabled bodies go to a new `config.yaml`; memory and expectation numbers go to the prior's constants; owner ids are replaced by pairing; secrets stay in `.env`; identity stored in `state: identity` after birth; an `identity_changing` capability for owner changes.
- [ ] T11 Stability contract: `schema_version` on every event with upcasting on read; plugin event kinds under `<plugin>.<kind>`; readers ignore unknown fields (core.md §3).
- [ ] T12 Watchdog: `hook_timeout_ms` on every hook call (timeout = Stop + `error`), `job_timeout_seconds` on background jobs; a test where a hanging hook does not stop the loop.

## Done when
All tests and replay pass; grep shows no platform name, no world action and no direct clock or random call in the core.

## Notes
