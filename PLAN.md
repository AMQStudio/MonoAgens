# PLAN.md — Prototype Steps

> MVP steps are complete. New work is tracked as phases and tracks in `ROADMAP.md`.

One step = one agent session = one commit. Tick a step only when its tests pass.

## [x] 0. Foundation
- uv project, Python 3.13+, dependencies exactly as in AGENTS.md; `asyncio_mode = "auto"`.
- `compose.yaml`: PostgreSQL 17 with pgvector ≥ 0.8.1 (dev and tests).
- `sql/schema.sql`: `vector` and `pg_trgm` extensions, the four tables, a trigger that rejects
  UPDATE/DELETE on events, an index on expectations.next_check_at.
- `scripts/db_init` applies the schema. `.env.example`.
- Empty skeleton: monoagens/{kernel,store,expect,memory,pfc,clock,types,config}.py,
  monoagens/bodies/telegram.py, tests/.
Done when: `uv run pytest` passes with one test proving the schema is installed and an
UPDATE on events is rejected.

## [x] 1. The Log
store: append an event, read the last N events for a place, cursor(body), read/write state.
Done when: tests for ordering, cursor, and rebuilding state from events pass.

## [x] 2. The kernel loop
Event/Action types, the queue, a single consumer, the clock (tick every clock.tick_seconds,
fakeable), the last_seen projection. Fake bodies for tests.
Done when: 100 events from two fake bodies are recorded in order; ticks are not stored.

## [x] 3. Telegram body
Long polling over httpx, update → Event (addressed flag for DM/mention/reply-to-bot),
say/react actions → sendMessage/setMessageReaction, offset from cursor.
Done when: a test with httpx.MockTransport shows a duplicate update is recorded once;
manual check: an incoming message is recorded.

## [x] 4. The PFC
Config from .env, layered prompt (stable → core_memory → volatile), tool calling, actions
written as events then handed to the body. For now every DM wakes the PFC.
Done when: FakeLLM tests pass; manual check: a DM gets a reply; MVP TEST (a): restart in the
middle of a conversation and the next reply continues the topic.

## [x] 5. The gate
Features + weights + bands from soul.yaml, decision.silent / decision.note with the score
breakdown. Note: disable the bot's privacy mode in BotFather (/setprivacy → Disable) so it
receives all group messages.
Done when: MVP TEST (b): 20 ordinary group messages → 0 msg.out and 20 decision.silent;
a mention gets a reply.

## [x] 6. Expectations
expect/resolve tools, time path (tick) and event path, the three operators, once/cooldown/
max_fires policies, required expires_at, recovery of fired expectations at startup.
Done when: MVP TEST (c): "remind me at 7" → exactly one message at 7 with a fake clock,
including a restart around 7; an event_seen expectation that is met never fires.

## [x] 7. Memory and sleep
Embeddings, hybrid retrieval, sleep, core_memory, memories in the prompt.
Add `scripts/embed_check` with 25 sentence pairs in the owner's language (similar/dissimilar).
Done when: MVP TEST (d): after sleep there are new memories, events are untouched, and a
question phrased with different words finds the right memory.

## [x] 8. MVP acceptance
Combine tests (a)–(d) into `tests/test_mvp.py`. The owner uses the bot for a week, notes the
"why so quiet?" and "why did it butt in?" moments, then tunes the weights in soul.yaml.