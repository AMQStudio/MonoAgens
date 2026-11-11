# 004 — Core: threshold, gate and circuit breaker (Track D)
Status: todo | Wave: 2b | Depends on: 003
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §3, §15
May touch: `monoagens/cognition/appraise.py`, `monoagens/cognition/appraisal.py`, `monoagens/cognition/rng.py`, `tests/test_cognition.py`

## Goal
Dynamic θ, the seeded go/no-go draw and the circuit breaker on commitments.

## Tasks
- [ ] T1 θ = base + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity); familiarity from ACT-R activation of interactions with the place and people present.
- [ ] T2 P = σ((S_eff − θ)/τ) with the seeded draw; every number stored in the decision event.
- [ ] T3 Circuit breaker: `addressed`, an active Expectation fired or answered, or an active goal continued.
- [ ] T4 Knob mapping function (one place) from the prior's knobs to θ, τ, γ and weights.

## Done when
Same events + seed → same decisions; talk rate ≤ 1/n in a busy group; an addressed message always wakes.

## Notes
