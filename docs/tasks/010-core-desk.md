# 010 — Core: the desk and recall (Track K)
Status: todo | Wave: 2a | Depends on: 001
Read first: [core.md](../core.md) §4, §8, [birth.md](../birth.md) §3.1 (desk constants)
May touch: `monoagens/pfc.py`, a new `self` organ module inside `monoagens/`, `tests/test_pfc.py`

## Goal
The prompt rebuilt per wake within budgets, the work thread, recall and back-fill.

## Tasks
- [ ] T1 Section token budgets from prior constants; trimming oldest-first per section.
- [ ] T2 Work thread in `<doing>`: goal, one line per step, latest result verbatim, older results as pointers.
- [ ] T3 Middleware notes rendered with their event.
- [ ] T4 `self` organ with `recall(query | event_id, place?, from?, to?)`: time range, lineage, text search on events, scope reported.
- [ ] T5 Back-fill appraised as one age-discounted percept per place.

## Done when
A 15-step task keeps a flat prompt size; "what did we do last month?" lists that month's episodes with details from the original events, also after the vector faded and when sleep failed; 40 unread messages are appraised once.

## Notes
