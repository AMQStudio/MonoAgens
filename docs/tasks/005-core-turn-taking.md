# 005 — Core: turn-taking and patience (Track E)
Status: todo | Wave: 2a | Depends on: 001
Read first: [core.md](../core.md) §5–6, [core-math.md](../core-math.md) §4, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`

## Goal
Patience as the self-selection rule of turn-taking.

## Tasks
- [ ] T1 `wait` Expectation created on a go for a turn not allocated to the soul, window from `patience_seconds`.
- [ ] T2 `not_who` cue: someone other than the sender takes the floor → silent.
- [ ] T3 Addressed → no wait.

## Done when
Not addressed: someone else answers → silent, nobody → one reply; addressed: answers without waiting.

## Notes
