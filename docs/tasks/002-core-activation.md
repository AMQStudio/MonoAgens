# 002 — Core: ACT-R activation (Track B)
Status: todo | Wave: 2a | Depends on: 001
Read first: [core.md](../core.md) §7, [core-math.md](../core-math.md) §5
May touch: `monoagens/memory.py`, `tests/test_memory.py`

## Goal
Retrieval ranked by ACT-R activation, with use counts and a feeling-of-knowing helper.

## Tasks
- [ ] T1 Base-level activation B = ln(n/(1−d)) − d·ln(L) from the injected clock; d from `memory_persistence`.
- [ ] T2 Total activation A = B + w_ctx·cos + w_imp·importance; candidates top 40 by vector ∪ top 20 by text; update `use_count`.
- [ ] T3 Feeling-of-knowing helper (max activation among candidates, recent messages included) for task 003.

## Done when
Frequently used memories outrank newer unused ones; activation is reproducible under a fake clock.

## Notes
