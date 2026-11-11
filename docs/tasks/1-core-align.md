# Task 1 — Core: align the code with the core spec
Status: **done** (2026-11-11) | Depends on: —
Kept as history. Report: 181 tests passed on real Postgres; fake replay 17/25 (the 8 failing
scenarios are fixed in Task 2 Part 2); package 2,897 / 3,500 lines. Follow-ups: Task 2 Part 1.
Read first: [core.md](../core.md) §3–§9, [project.md](../project.md) §2
May touch: `monoagens/` (all), `sql/`, `soul.yaml`, `config.yaml` (new), `scripts/replay.py`, `scenarios/`, `tests/`, `prior/`

## Goal
Bring the merged Track A code in line with the frozen core spec. This is the only task allowed to change the shared files (`kernel.py`, `store.py`, `sql/schema.sql`, `soul.yaml`).

## Tasks
- [x] T1 Thought acts `think, act, expect, resolve, revise`; `say`/`react` become Telegram and CLI capabilities (core.md §9.1); silence = a wake without `act`; `msg.in` becomes `percept` with a description and optional data (§9.2); `act.out`/`act.result` replace `msg.out`/`react.out` in code, tests and replay.
- [x] T2 Three hooks (inbound, context, action) with owner-policy slots first and last, shared vs organ-own scope, collision check at boot.
- [x] T3 Provenance and privacy: `origin_actor` on events (inherited along `caused_by`), `identity_changing` check in the action hook; `place` + `private` on memories and the context-hook filter.
- [x] T4 Lineage: `derived_from` on memories, expectations and vectors; replace the `event_vectors` no-delete trigger with the two dedicated paths (fade, hard forget) — stubs that write their events are fine.
- [x] T5 Budget: `model.call` for every model call; the budget term in θ and `budget.exhausted` (no thrift mode).
- [x] T6 Expectation schema with the four kinds (`promise`, `wait`, `predict`, `guard`).
- [x] T7 Injected clock everywhere (no `datetime.now()` in the core); replay uses event times.
- [x] T8 Slow work off the loop: PFC, classifier, embedding and sleep as background jobs whose results return as events; one PFC job per place; re-wake once if a new event arrived meanwhile.
- [x] T9 Numbered SQL migrations with a version table and a backup before migrating; pairing code for the first owner.
- [x] T10 Identity as birth content: `soul.yaml` becomes the birth certificate; cognition numbers move to the default prior in `prior/` as knobs / calibration / constants (core.md §9.4); the rest of today's `soul.yaml` splits by project.md §2: `timezone`, sleep, clock, budget and enabled bodies go to a new `config.yaml`; memory and expectation numbers go to the prior's constants; owner ids are replaced by pairing; secrets stay in `.env`; identity stored in `state: identity` after birth; an `identity_changing` capability for owner changes.
- [x] T11 Stability contract: `schema_version` on every event with upcasting on read; plugin event kinds under `<plugin>.<kind>`; readers ignore unknown fields (core.md §3).
- [x] T12 Watchdog: `hook_timeout_ms` on every hook call (timeout = Stop + `error`), `job_timeout_seconds` on background jobs; a test where a hanging hook does not stop the loop.

## Done when
All tests and replay pass; grep shows no platform name, no world action and no direct clock or random call in the core.

## Notes
- T1: five thought acts only; `act` carries `{capability, arguments}` for the capabilities
  declared by each body (`monoagens/organs.py`); a wake without `act` writes
  `decision.silent`; percepts carry `description` + optional `data`; `act.out`/`act.result`
  replace the old message kinds across code, tests and replay.
- T2: `monoagens/middleware.py` holds `HookRegistry` with `inbound`, `context` and `action`
  pipelines; each `HookPipeline` keeps an owner policy (`set_policy`) that runs first and last,
  three slots (`first`, `middle`, `last`), and shared versus organ-own scope (`organ=None` is
  shared). Duplicate names and the reserved owner-policy name fail at registration. The kernel
  keeps only the registry and routes the action hook before an organ acts; the old `tool` and
  `outbound` hooks are gone.
- T3: `events.origin_actor` is written once at intake (explicit, else the `caused_by` parent's
  origin, else `who`) and read back on every `Event`; the kernel's action-hook context carries
  `origin_actor` and the capability's `identity_changing` flag. The default owner policy
  (`owner_policy` in `middleware.py`, installed on all three hooks by the kernel) stops an
  `identity_changing` action unless `origin_actor` is `person:owner`, and filters
  `PFCContext.memories`: a private `MemoryHit` stays only in its own place or an owner direct
  chat. Sleep accepts `place` and `private`; retrieval returns `MemoryHit` (a `str` with
  `place` and `private`).
- T4: `derived_from bigint[]` replaces `memories.source_events` (old databases are migrated and
  the column dropped); expectations record `derived_from = {source.id}`; event vectors record
  their own event id. `event_vectors` UPDATE is still rejected; DELETE is allowed only while
  `monoagens.allowed_vector_delete` is `on`, which only `fade()` and `forget_hard()` set.
  `events` UPDATE is allowed only with `monoagens.forgetting` set, only when the payload becomes
  `{"forgotten": true}` and no metadata changes. `forget_hard()` scrubs payloads and deletes the
  `derived_from` lineage (vectors, memories, expectations), then writes `forget.hard`; `fade()`
  releases vectors older than a caller-given cutoff that no memory or expectation derives from
  and writes `fade.done` (nothing eligible → no event).
- T5: `monoagens/budget.py` owns the accounting. `budget.record_call()` is called right after
  every LLM call (`pfc.decide` in the kernel, `sleep.consolidate` in sleep); it appends
  `model.call` with `{purpose, model, usage}` caused by the prompting event, adds the tokens to
  `budget:<local-date>`, and writes `budget.exhausted` once per local day when the daily limit is
  reached (payload: date, used, budget). `appraise()` reads that state key and sets
  `theta = threshold.budget * min(1, total_tokens / daily_token_budget)`; `daily_token_budget`
  comes from the cognition config, so no budget means no term. Sleep's input query now ignores
  `model.call`/`budget.exhausted` bookkeeping events.
- T6: `expectations.kind` is `promise|wait|predict|guard` (default `promise`, CHECK enforced);
  `expect` was renamed to `condition` and old databases are migrated in place. Expectations
  record `origin_actor` from the source event. `guard` rows are skipped by `due()`, so they never
  fire or wake the PFC. The `expect` tool schema exposes `kind` with the four-value enum.
- T7: the only `datetime.now()` left in the package is `Clock.now()`; `assemble_situation`
  and `memory.retrieve` take a required `now` (no wall-clock fallback), `appraise` falls back
  to `event.ts`, and `CLIBody`/`TelegramBody` accept an optional `Clock` for their percept
  timestamps. `RecordingKernel.put` pins the replay `FakeClock` to each event's `ts`, so
  appraisal, situation assembly, expectations and memory lookups use event times.
- T8: `monoagens/jobs.py` (`JobRunner`) owns the background tasks under
  `jobs.timeout_seconds` (or `job_timeout_seconds`, default 60 s). PFC, embedding and sleep
  start through it; each writes `job.started` and, when done, enqueues `job.result` (with the
  job name, key and a log-safe result — an embedding vector is not logged). The kernel applies a
  result in the loop (`_finish_job`): PFC calls are applied and `model.call` is recorded with the
  job's captured usage, an embedding is stored, a failed sleep is recorded as `sleep.failed`.
  `job.result` and `job.started` skip the inbound hook. PFC jobs are keyed by place (only one in
  flight); a wake that arrives meanwhile is kept as one pending re-wake with the newest trigger
  and fires after the result is applied. Sleep runs on a private connection (DSN rebuilt from
  `conn.info`, password included) so the shared async connection is never used concurrently.
  put` now returns the stored event; `run_until_idle` also drains in-flight jobs. Retrieval in
  a wake that races its intake encoding embeds its query once more (test updated).
- T9: `monoagens/migrate.py` applies `sql/migrations/NNN_name.sql` in version order; the initial
  schema moved from `sql/schema.sql` to `001_initial.sql`. `schema_version` (version, name,
  applied_at) records each file, each migration runs in one transaction, and a JSON snapshot of
  every book (`backups/before-NNN.json`, `MONOAGENS_BACKUP_DIR`) is written before the first
  pending one. Boot (`__main__`), replay, test bootstrap and `scripts/db_init.py` all go through
  it. `monoagens/pairing.py` creates a `pairing_code` state key on first start (logged), records
  `pair.created`; the inbound hook turns a percept that contains the code into `state: owner`
  (`{"who": ...}`), deletes the code and records `pair.claimed`, and refuses once an owner
  exists. `pair.created`/`pair.claimed` are additive event kinds; the code itself is never
  stored in the Log.
- T10: `soul.yaml` holds only name, language, persona and principles. `monoagens/config.py`
  reads it and `config.yaml`; `monoagens/prior.py` loads `manifest`, `knobs`, `calibration`,
  `constants`, `references`, `lessons` and `procedures` into one runtime view, and only the
  prior supplies signal weights, the gate seed, retrieval numbers and expectation limits.
  `monoagens/identity.py` writes the certificate into `state: identity` once (`identity.born`,
  projected from the Log), reads state afterwards and declares the core's only organ, `self`,
  with `set_identity` (`identity_changing: true`); the action hook's owner policy gates it and
  a successful change records `identity.updated`. `pairing.as_owner` rewrites the paired
  owner's native id to `person:owner` at intake, so no owner ids live in configuration; the
  Telegram body no longer takes them. Embedders moved to `monoagens/embedding.py` to keep
  `memory.py` in budget; the kernel is 280 lines.
- T11: `events.schema_version` (`integer NOT NULL DEFAULT 1`, migration `002_event_schema_version.sql`) and `Event.schema_version` (`CURRENT_SCHEMA_VERSION` in `types.py`); `append_event` writes it and rejects a malformed kind or a version below 1. The immutability trigger now compares `schema_version` too, so hard forgetting can scrub only the payload. `monoagens/versioning.py` holds `upcast()` (a per-version translator chain in `UPCASTERS`, empty at v1; an older version without a translator raises) and the kind rules: every kind is a lower-case dotted name, `plugin_kind(plugin, kind)` composes the required `<plugin>.<kind>` namespace, and `is_namespaced_event_kind` tells a plugin kind from a core one. `_row_to_event` upcasts every event read, so stored rows are never rewritten; readers keep unknown payload fields untouched. `store.py` is 240 lines. Note: `sql/schema.sql` is a leftover duplicate of `001_initial.sql` that nothing reads (the code goes through `sql/migrations/`); it was left untouched.

- T12: the hook machinery (ordered slots, scoped lists, watchdog) moved from
  `middleware.py` to `monoagens/hooks.py`, which the size budget already reserves for it;
  `middleware.py` keeps only the contract (outcomes, `Middleware`, `Registration`,
  `owner_policy`) at 74 lines, `hooks.py` is 102. `HookRegistry(settings)` reads
  `hook_timeout_ms` from the prior constants (default 500 ms) and gives it to every
  `HookPipeline`; `run()` wraps each middleware call in `asyncio.wait_for`, and a timeout
  calls `on_error(TimeoutError)` (the kernel records `error` and `guard.stopped`) and stops
  the item. Background jobs already ran under `job_timeout_seconds` (default 60 s, prior 120 s)
  and returned `job.result` with an error; no change was needed there. New test
  `test_hanging_hook_times_out_as_a_stop_and_the_loop_goes_on` registers a hook that sleeps
  10 s, asserts one `error` + one `guard.stopped` after 10 ms, and shows the next event is
  recorded and processed. `uv run pytest` is 181 passed; replay is 17/25, the same 8 failing
  scenario checks as before T12 (reminders, recall, watch-for, cross-body), unchanged.
