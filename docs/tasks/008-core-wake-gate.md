# 008 — Core: wake gate and free-text conditions (Track I)
Status: todo | Wave: 2b | Depends on: 005
Read first: [core.md](../core.md) §5, [core-math.md](../core-math.md) §6, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`

## Goal
Cheap waking for Expectations; cosine filters, a classifier judges.

## Tasks
- [ ] T1 Wake gate: still relevant? stale? (quiet hours stay in the owner policy).
- [ ] T2 Free text: cosine below `lo` ignored; every candidate → one recorded classifier call.
- [ ] T3 `hi` band opens only by the rule of three; `lo` learning within bounds and a minimum gray width.

## Done when
A fulfilled promise wakes no one; a question on the same topic never resolves a promise.

## Notes
