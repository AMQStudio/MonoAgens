# Spec Delta

## Purpose

Bounded learning lets the soul adjust itself from the outcomes of its own decisions, slowly
and inside limits. Every adjustment is an event, so what the soul has learned can be read,
replayed and rolled back.

## ADDED Requirements

### Requirement: Learning uses the outcome of a prediction
Teaching signals derive from resolved predictions: positive (`r = 1`) for `{good_act, missed_silence}`
and negative (`r = 0`) for `{bad_act, answered_silence}` (repair, miss, failure, negative flag).
Preempted or cancelled receipts (`preempted_external`, `canceled_by_core`) SHALL NOT penalize
learning or capability reliability (`R-36`).

#### Scenario: A reply that landed
- **WHEN** the soul spoke on an unallocated turn and its response prediction was answered in time without a miss
- **THEN** that decision enters learning with target `r = 1`

#### Scenario: A missed judged silence
- **WHEN** the soul stayed silent on a sampled event and its judged-silence prediction confirms a miss
- **THEN** that decision enters learning with target `r = 1` so positive weights on its active signals increase rather than invert

#### Scenario: An externally interrupted speech act
- **WHEN** an in-flight action receives `Receipt(failed, termination_reason = preempted_external)`
- **THEN** weight learning and capability `(α_c, β_c)` record zero failure penalty

#### Scenario: A negative flag
- **WHEN** a body reports a negative flag on the soul's act
- **THEN** that decision enters learning with target `r = 0`

#### Scenario: Nothing came back yet
- **WHEN** a decision's prediction is still open
- **THEN** learning does not use that decision

### Requirement: Signal weights follow the IPS-weighted delta rule
When weight learning is enabled, weights SHALL update via `w_k ← clamp(w_k + α_learn · w_ips ·
teacher_weight · (r − P) · s_k)` using `r = 1` (`good_act`, `missed_silence`), `r = 0` (`bad_act`,
`answered_silence`), and clipped Inverse Propensity Scoring `w_ips = min(M_ips, 1/ρ)` for sampled
silences (`R-19`).

#### Scenario: Speaking that was not welcome
- **WHEN** the gate gave a high probability, the soul spoke, and the outcome was `r = 0`
- **THEN** the weights of the signals that were high for that decision decrease in proportion to teacher weight

#### Scenario: Learning on a sampled missed silence
- **WHEN** a judged silence sampled at probability `ρ = 0.05` yields `missed_silence` (`r = 1`)
- **THEN** active signal weights increase weighted by `w_ips = min(M_ips, 1/ρ)`

#### Scenario: The learning rate is zero
- **WHEN** the learning rate is at its neutral value of zero
- **THEN** no weight changes

### Requirement: Knobs move only on enough evidence
When knob learning is enabled, a disposition SHALL move only when the 95% interval of the
evidence excludes "no change".

#### Scenario: Three observations
- **WHEN** only a handful of outcomes point one way
- **THEN** the disposition does not move

### Requirement: Every step and every day is bounded
Each change to a disposition SHALL be at most the configured share of its range, the total
change in one day SHALL be at most five percent of its range, and no disposition SHALL leave
its floor and ceiling.

#### Scenario: Strong evidence in one day
- **WHEN** a day's evidence would justify a large move
- **THEN** the disposition's total change that day is no more than five percent of its range

### Requirement: A disposition stuck at a bound freezes and is reported
A disposition that sits at its floor or ceiling for the configured number of days SHALL stop
learning and SHALL be reported.

#### Scenario: Pinned at the ceiling
- **WHEN** a disposition has been at its ceiling for the configured number of days
- **THEN** a report event names it, and it no longer changes until released

### Requirement: Global values learn by teacher weight and scoped values from everyone
Each source SHALL have a teacher weight supplied from outside the core. An outcome SHALL move
global weights and dispositions in proportion to the teacher weight of the source involved,
and recorded classification verdicts SHALL count in full. Offsets scoped to one place or one
source SHALL learn from the outcomes of everyone there, by the same bounded rule.

#### Scenario: A stranger's reactions
- **WHEN** a source with teacher weight zero reacts badly to the soul's messages many times
- **THEN** the global values are unchanged, and the offset for that place or source changes

#### Scenario: A teacher's reactions
- **WHEN** a source with a high teacher weight reacts badly many times
- **THEN** the global values move, within their bounds

### Requirement: The lower wake band learns from shrunk quantiles of verdicts
When band learning is enabled, `wake_gate_lo` SHALL track a hierarchical cold-start shrunk lower
quantile (`q_0.10`) of positive classification similarities minus a margin (`A-07`), bounded by
its floor, ceiling, and daily step limit.

#### Scenario: Positive verdicts appear lower than the band expected
- **WHEN** events just above the lower band are repeatedly classified as meeting their condition
- **THEN** the lower band moves down smoothly by bounded steps without collapsing on a single outlier

### Requirement: Each change is an event that carries its own delta
Every learned change SHALL be recorded as an event stating what changed, by how much and on
which evidence, without needing any message content to interpret it.

#### Scenario: Reading what was learned
- **WHEN** the log is filtered for learning events
- **THEN** the current value of every learned weight and disposition equals its prior value plus the recorded deltas

### Requirement: Learned changes can be rolled back
It SHALL be possible to return any learned value to an earlier state by recording a reversing
event, without editing the log.

#### Scenario: Undoing a week of learning
- **WHEN** a rollback to the state of seven days ago is requested
- **THEN** the values equal those of seven days ago and the log contains the rollback event after the changes it reverses

### Requirement: Learning is off by default in measurement
Each kind of learning (weights, knobs, wake band) SHALL have its own switch, and with all
switches off the soul's dispositions SHALL stay exactly at the prior.

#### Scenario: A run with learning off
- **WHEN** a run is made with all learning switches off
- **THEN** the log contains no learning event
