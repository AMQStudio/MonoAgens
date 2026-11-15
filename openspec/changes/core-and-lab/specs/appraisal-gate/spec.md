# Spec Delta

## Purpose

The appraisal gate decides, cheaply and without any model call, whether a moment deserves
reasoning. It is what lets the soul stay silent on purpose and keeps the cost of perceiving
far below the cost of thinking.

## ADDED Requirements

### Requirement: Appraisal uses no model call
The gate SHALL reach its decision for every event without calling a language model.

#### Scenario: A busy place the soul is not part of
- **WHEN** one hundred events arrive in a place and none is allocated to the soul or engages a commitment
- **THEN** one hundred decisions are recorded and the number of model calls caused by decisions with outcome silent or note is zero

### Requirement: Signals are bounded numbers from structural and embedding features
Each signal (`own_name`, `dialogue_act_gap` via frozen contrastive logistic head `(w_gap, b_gap)`,
`own_info_gap` from low belief credence, `feeling_of_knowing` over `VisibleScope`, `relevance`,
and `social_cost`) SHALL evaluate deterministically in `[0, 1]` without word lists or LLMs (`A-14`).

#### Scenario: A feature is missing
- **WHEN** an event arrives without an embedding vector
- **THEN** vector-dependent signals take their neutral value (`0`) while non-vector structural cues still decide

#### Scenario: Topic chatter mimics a question without requesting help
- **WHEN** a declarative statement shares topical keywords with an open thread (`cos` high, `dialogue_act_gap < 0.5`)
- **THEN** the contrastive logistic head suppresses false `info_gap` waking compared to positive-only cosine matching

### Requirement: Salience is a weighted, clamped sum with reachable neutrals
Salience SHALL be `clamp(Σ wₖ·sₖ, 0, 1)` where every disposition map reaches exact neutral `0`
when turned off or at zero restraint (`A-01`).

#### Scenario: A negative weight lowers salience
- **WHEN** social_cost is high and its weight is negative
- **THEN** salience is lower than for the same event with social_cost at zero

### Requirement: Repeated stimuli habituate cleanly per source channel
Habituation SHALL track `h_{(p, k_src)}` partitioned per `(place, source_channel)`, decay by
`γ^Δt`, and compute `S_eff = S / (1 + κ · h_pre)` **before** incrementing `h ← h_pre + 1`
(`A-03`, `R-36`), so stimulus 1 keeps `S_eff = S` and background sensors never mute alarms.

#### Scenario: First stimulus preserves full salience
- **WHEN** a first event arrives from a new source channel in a place (`h_pre = 0`, `κ = 1`)
- **THEN** `S_eff` equals `S` (not `S / 2`) before habituation increments to `1`

#### Scenario: High-rate telemetry leaves separate alarm channel sharp
- **WHEN** a sensor repeats every second on `channel_a` while an alert fires once on `channel_b`
- **THEN** `channel_b` retains full unhabituated `S_eff = S`

#### Scenario: Recovery after a pause and zero neutral
- **WHEN** a source returns after a long pause or when `κ = 0`
- **THEN** effective salience recovers toward `S` (or equals `S` identically at `κ = 0`)

### Requirement: The threshold moves with drives and opportunity costs
`θ` SHALL combine `θ₀`, cold-start shrunk `talk_rate`, `budget_used`, undiluted urgent
`promise_debt`, unfamiliarity reserve, idle cooldown `Δθ_init` (`R-30`), and opportunity-cost
drives (`R-31`), keeping `Δθ_sleep = 0` (`A-04`) and reachable `a = 0`.

#### Scenario: The soul has just spoken a lot
- **WHEN** the soul's recent acts in a place exceed its fair share among the active participants
- **THEN** the threshold for the next event in that place is higher than before those acts

#### Scenario: The budget is nearly spent
- **WHEN** most of the daily budget has been used
- **THEN** the threshold is higher than at the start of the day for the same event

### Requirement: Go or no-go is a seeded draw
The gate SHALL turn effective salience and threshold into a probability and compare it with a
draw seeded from the configured seed and the event's id. The outcome SHALL be go, note or
silent, where a no-go with a probability below the note band is silent.

#### Scenario: The same event twice
- **WHEN** the gate appraises the same event with the same state and seed twice
- **THEN** both decisions have the same probability, the same draw and the same outcome

#### Scenario: Probabilities are honoured over many events
- **WHEN** many distinct events with the same probability of 0.3 are appraised
- **THEN** the share of go outcomes approaches 0.3

### Requirement: A commitment bypasses the draw
The gate SHALL wake reasoning without a draw when the event engages a commitment: the turn was
allocated to the soul, the event fires or answers an open expectation, or it continues an
active chain of the soul's own work.

#### Scenario: The soul is addressed
- **WHEN** an event arrives whose turn is allocated to the soul
- **THEN** the decision is go with the reason commitment, whatever the salience

#### Scenario: A promise falls due
- **WHEN** an open promise reaches its due time
- **THEN** the decision is go with the reason commitment

### Requirement: Each decision explains itself
Every decision event SHALL contain each signal's value, the salience, the habituation factor,
the threshold with its terms, the probability, the draw, the outcome and the reason.

#### Scenario: Reading a decision
- **WHEN** a decision is read from the log
- **THEN** its outcome can be recomputed from the numbers stored in that same event

### Requirement: Shrunk patience and idle breakpoints regulate timing
An unallocated `go` SHALL open a `wait` priced by a right-censored hierarchical shrinkage
quantile estimator (`A-08`), cancelling as `silent` if a third party answers first; unprompted
idle initiatives SHALL fire only on natural breakpoints and raise `Δθ_init` cooldowns (`R-30`).

#### Scenario: Someone else answers first
- **WHEN** the gate decides go on an unallocated question and another participant answers within the shrunk patience window
- **THEN** no model call is made and the decision is recorded as silent with the reason yielded

#### Scenario: Nobody answers
- **WHEN** the patience time passes with no reply from anyone but the sender
- **THEN** reasoning wakes for that event

#### Scenario: An allocated turn does not wait
- **WHEN** the turn was allocated to the soul
- **THEN** reasoning wakes without a wait

### Requirement: A backlog during downtime is appraised as one chunk
Events arriving after `patience_place` or while the core was truly **stopped/down** (never
merely during background async sleep, `A-04`) SHALL chunk per place via age-discounted event-level
`max` across at most four chunks per wake (`R-14`).

#### Scenario: Live urgent interruption during background sleep consolidation
- **WHEN** background consolidation is running on a snapshot and an urgent event arrives live
- **THEN** the gate appraises the event immediately at normal `θ` (`Δθ_sleep = 0`) without deferring it to a backlog chunk

#### Scenario: Thirty missed messages after downtime
- **WHEN** thirty events of one place arrive together after core downtime, none allocated to the soul
- **THEN** one decision is recorded for the chunk using event-level age-discounted max, and all thirty events remain in the log

#### Scenario: An allocated turn inside a backlog
- **WHEN** one event in a backlog was allocated to the soul
- **THEN** the chunk's decision is go with the reason commitment

#### Scenario: Six places have a backlog
- **WHEN** six places each have a backlog after downtime
- **THEN** the first wake handles the four most salient chunks and the other two are handled afterwards

### Requirement: Closeness uses guarded Petrov activation
Closeness (`σ(B_p)` from exchanges) and place familiarity (`σ(B_ctx)` across present actors)
SHALL evaluate Petrov's hybrid with guard `n > k ∧ t_n > t_k` (1-min floor, neutral `0` on `n = 0`,
pseudo-count shrinkage for cold start `A-07`) and fade when contact stops.

#### Scenario: A large channel with identical first-minute timestamps
- **WHEN** multiple events arrive within the 1-minute floor in a large channel with zero prior exchanges
- **THEN** guarded Petrov activation avoids `0/0` NaN and closeness stays `0`

#### Scenario: A place left long ago
- **WHEN** the soul has not been present in a place for a long time
- **THEN** the familiarity of that place is lower than it was during regular contact

### Requirement: Continuous noisy-OR addressivity drives social floor cost
Core SHALL compute noisy-OR graded addressivity `deg(e, l) ∈ [0, 1]` from mentions, open links,
dyadic momentum, and optional `orient(e, l)` (`R-29`), setting `social_cost = max(clamp(
max_{l≠soul} deg(e, l) − deg(e, soul), 0, 1), answered) · (1 − λ_c · closeness(sender))`.

#### Scenario: Another person holds higher graded addressivity
- **WHEN** an event carries high `deg(e, other)` or third-party reply `answered = 1` from an unfamiliar sender
- **THEN** `social_cost` rises continuously near its maximum without requiring a strict binary addressee exclusion

#### Scenario: A close sender addresses the room broadly
- **WHEN** the same event comes from a close sender across a multi-party thread
- **THEN** `social_cost` is discounted proportionally by `λ_c · closeness(sender)`
