# Spec Delta

## Purpose

Expectations are what the soul holds about the future: promises it made and predictions of
what should follow its acts. Their lifecycle lives in code, not in the model's attention, so a
promise is kept even when nobody mentions it again.

## ADDED Requirements

### Requirement: An expectation is a typed record
An expectation SHALL hold a kind (`effect`, `response`, `wait`, `promise`, `goal`, `monitor`,
`pending_approval`), plural `places` and `people`, optional `domain?`, `RecurrenceSpec` (`R-46`),
`PromiseMeta` (`R-49`), expiry, and origin event, driven in code without model calls.

#### Scenario: A multi-party or cross-place promise is opened
- **WHEN** reasoning commits to ask one person and report back to another place
- **THEN** an open expectation recording plural `places` and `people` and its `PromiseMeta` deadline is stored

### Requirement: Time conditions and startup recovery avoid storms
Time conditions and recurring `RecurrenceSpec` triggers SHALL fire no earlier than scheduled;
missed due items after downtime SHALL coalesce or stagger (`coalesce_one` / `skip_to_next`, once
per place) without re-triggering storm bursts (`R-46`).

#### Scenario: A reminder comes due
- **WHEN** the soul promised a reminder in two hours and two hours pass in silence
- **THEN** reasoning wakes with the promise in its context and no earlier than the due time

#### Scenario: The soul was stopped across due times
- **WHEN** single-shot promises and recurring monitors fell due while stopped and the core restarts
- **THEN** overdue promises wake once per place and `RecurrenceSpec` monitors coalesce cleanly without catch-up bursts

### Requirement: Promises, approvals, and preempted acts handle receipts distinctly
A promise stays open across retries unless `kept`, `kept_late`, `renegotiated`, `released`,
`cancelled`, or `failed` (`R-49`). A `held` receipt SHALL park in non-retrying `PendingApproval`
(`R-42`), and `preempted_external`/`canceled_by_core` receipts (`R-36`) SHALL resolve cleanly
without backoff retry storms.

#### Scenario: An action is held by Valve awaiting human approval
- **WHEN** a capability dispatch returns `Receipt(held)` bound to canonical hash `H_act`
- **THEN** the expectation transitions to `PendingApproval` and does not emit duplicate approval prompts on timers

#### Scenario: A promise is kept, retried, or kept late
- **WHEN** the act fails transiently (`failed`, `normal`), retries with exponential backoff, and succeeds after the due time
- **THEN** completion is recorded as `kept_late` separately from on-time `kept`

#### Scenario: An action is preempted externally
- **WHEN** an in-flight speech or body action ends with `Receipt(failed, termination_reason = preempted_external)`
- **THEN** the expectation stops cleanly with `delivered_units` recorded instead of spamming exponential retries

### Requirement: Acts open channel-aware effect and response predictions
After each action the loop SHALL open an effect prediction carrying `(channel, window, Δ_pred)`
when declared (`R-09`, `R-36`) and a response prediction accepting `done` receipts, addressee
replies, or `continued_attention` (`R-08`).

#### Scenario: The expected reply comes
- **WHEN** the source the soul acted toward responds in that place before the expiry
- **THEN** the response prediction is recorded as answered and the reply wakes reasoning as a commitment

#### Scenario: Nothing comes
- **WHEN** the expiry passes without a response
- **THEN** the response prediction is recorded as expired, as a noticed absence

#### Scenario: The declared effect does not appear
- **WHEN** the soul acts through a body and the declared effect is not sensed within its window
- **THEN** the effect prediction is recorded as an error and reasoning wakes

### Requirement: Only the effects of its own acts are self-caused
An incoming event SHALL be marked self-caused, and SHALL NOT be appraised as a new stimulus,
only when it matches an open effect prediction: it carries the act's correlation id or is
attributed to the soul itself, or it arrives on the declared channel within the declared
window and tolerance. An event that answers a response prediction SHALL NOT be marked
self-caused.

#### Scenario: A body reports the soul's own message back
- **WHEN** a body reports as a percept the very message the soul just sent through it
- **THEN** the percept is recorded as self-caused and causes no reasoning

#### Scenario: A sensor changes after the soul's act
- **WHEN** the declared sensor changes within the declared window after the soul's act
- **THEN** the change is recorded as self-caused

#### Scenario: A person replies to the soul
- **WHEN** a source replies to the soul's message
- **THEN** the reply is not marked self-caused and wakes reasoning

#### Scenario: The sensor changes with no act
- **WHEN** the same sensor changes with no open effect prediction
- **THEN** the change is external and is appraised

### Requirement: Expectations can be revised and resolved by reasoning
Reasoning SHALL be able to resolve an open expectation or revise its condition or expiry. Each
change SHALL be recorded with the wake that made it.

#### Scenario: A reminder is moved
- **WHEN** the source asks to move a promised reminder to a later time
- **THEN** the promise's due time is revised, the old due time no longer fires and the revision is recorded

### Requirement: Undiluted urgency promise debt is visible to the gate
`promise_debt` SHALL sum urgency weights over imminent active promises due within `H_debt`,
excluding silent `RecurrenceSpec` monitoring schedules (`R-46`) so low-priority distant
obligations never dilute urgent commitments (`A-01`).

#### Scenario: Adding a distant promise does not dilute an imminent deadline
- **WHEN** one promise is due in ten minutes and ten low-priority promises are scheduled next week
- **THEN** `promise_debt` stays high based on summed urgency weights rather than shrinking by division over total count

### Requirement: A free-text condition is watched cheaply
An expectation SHALL be able to carry a condition in free text. For each event the loop SHALL
compare the event's vector with the condition's vector: below the lower band the event SHALL be
ignored; at or above it, one classification call SHALL decide whether the condition is met,
and its verdict SHALL be recorded with the similarity.

#### Scenario: An unrelated event
- **WHEN** an event's similarity to an open condition is below the lower band
- **THEN** no model call is made for that condition

#### Scenario: A question on the same topic is not the answer
- **WHEN** an event is similar to the condition and the classification says the condition is not met
- **THEN** the expectation stays open and the verdict is recorded

#### Scenario: The condition is met
- **WHEN** the classification says the condition is met
- **THEN** the expectation fires and reasoning wakes as a commitment

### Requirement: No similarity alone fires a condition until evidence allows it
The loop SHALL NOT fire a free-text condition on similarity alone. An upper band above which
no classification is needed SHALL be adopted only after enough recorded verdicts above it with
none negative, by the rule of three.

#### Scenario: Too few verdicts
- **WHEN** fewer verdicts than the rule requires have been recorded above a candidate upper band
- **THEN** every event above the lower band is still classified

### Requirement: A reply that misses is a prediction error by content
For a response prediction the loop SHALL measure how far the reply is from the act and from
any reply reasoning said it expected, and SHALL compare that distance with what is usual for
replies in that place. A reply unusually far SHALL be recorded as a content miss with who
replied, how early or late and how far, and SHALL wake reasoning with those numbers.

#### Scenario: The reply shows the act did not land
- **WHEN** the soul explains something and the source's reply is short, fast and far from the act
- **THEN** the prediction is recorded as a content miss and the wake's context states the miss

#### Scenario: An ordinary reply
- **WHEN** the reply's distance is within the usual range for the place
- **THEN** the prediction is recorded as answered

### Requirement: Chosen silence is judged by what follows
Every note decision, and a seeded fraction of silent decisions, SHALL open a prediction that
someone else responds to the sender before the place's silence window ends. It SHALL be
recorded as missed when nobody else replies to or addresses the sender in that time, or when
the sender raises the matter again and one classification confirms it.

#### Scenario: The question is asked again
- **WHEN** the soul stayed silent on a question and the sender posts it again in other words
- **THEN** after one classification confirms it, the silence's prediction is recorded as missed

#### Scenario: Someone else handled it
- **WHEN** the soul stayed silent and another participant replied to the sender
- **THEN** the silence's prediction is recorded as answered

#### Scenario: Most silences are not judged
- **WHEN** a hundred silent decisions occur
- **THEN** only the seeded fraction of them open a prediction, and a replay picks the same ones

### Requirement: Waiting times are learned per place
The patience time, the expiry of a response prediction and the window for judging silence
SHALL be estimated per place from the latencies observed there, starting from the typical
latency the body declares and moving toward the observed values as observations accumulate.

#### Scenario: A slow place
- **WHEN** replies in a place usually take hours
- **THEN** a response prediction there does not expire after minutes

#### Scenario: A new place
- **WHEN** no latency has been observed in a place yet
- **THEN** its durations come from the latency the body declared

### Requirement: Goals unite multi-step tasks and accept reached outcome
Reasoning SHALL open `goal` expectations resolved via `{kept, kept_late, renegotiated, released,
failed, cancelled, answered, reached}` (`A-01`). Linked receipts or replies wake via the circuit
breaker; similar unlinked events classify without bypassing the gate.

#### Scenario: A receipt arrives in a busy place
- **WHEN** the receipt of a goal's action arrives among many unrelated events
- **THEN** it is attributed to the goal and wakes reasoning as a commitment, whatever its salience

#### Scenario: A related remark
- **WHEN** an unlinked event is similar to a goal's condition and one classification says it concerns the goal
- **THEN** it is attributed to the goal and appraised by the gate without bypassing it

#### Scenario: A goal completes via reached outcome
- **WHEN** reasoning resolves an active goal with outcome `reached`
- **THEN** the goal closes cleanly without rejecting `reached` as an invalid enum variant

#### Scenario: A goal expires
- **WHEN** a goal's expiry passes
- **THEN** it is recorded as expired and no later event is attributed to it

### Requirement: Commitment dependencies and conflicts prevent over-promising
`PromiseMeta` SHALL track `depends_on` expectation links and surface conflicting active promises
(`R-49`) before new obligations commit.

#### Scenario: A conflicting commitment is proposed while a hard dependency is open
- **WHEN** a candidate act commits to overlapping exclusive resources against an open `PromiseMeta`
- **THEN** conflict status is flagged to reasoning for renegotiation or refusal prior to emitting the promise

### Requirement: A classification call carries what it judges
Every classification call SHALL render the triggering event's text (speaker, place, time and
content, under the same `VisibleScope` rules as reasoning) and the text of the condition,
expectation or goal it is judged against. A classification plan without both SHALL NOT be sent.

#### Scenario: Classifying an event against an open condition
- **WHEN** an event at or above the lower band triggers a classification for an open condition
- **THEN** the rendered request contains the event's speaker and content and the condition's text, and never renders `trigger: none`

#### Scenario: A classification plan is missing its inputs
- **WHEN** the event or the condition text cannot be rendered for a classification
- **THEN** no model call is made and the failure is recorded with its reason

#### Scenario: Classifying a possible re-ask after a chosen silence
- **WHEN** the sender of a message the soul stayed silent on posts again and a classification is requested
- **THEN** the rendered request contains the earlier message itself (speaker, time, content), the question whether the new message raises that matter again, and the labels; no internal condition tag stands in for it

### Requirement: Every non-reasoning call states its task
Every classification and consolidation call SHALL carry a task (`ContextSections.task`) that
`reason` renders as a `[TASK]` block from fixed templates: the question, the material it is
judged on and the labels Core reads (`met`/`not_met`, `yes`/`no`, `faithful`/`conflict`), or for
consolidation the episode's events in order and the gist to produce. Reasoning wakes SHALL carry no
task. Wording and labels live in `reason`, never in Core prose.

#### Scenario: Verifying a new gist
- **WHEN** sleep produces a gist over source events
- **THEN** its verification is a classification call whose task lists the source events, never a new consolidation call

#### Scenario: Consolidating an episode
- **WHEN** sleep consolidates an episode
- **THEN** the rendered request lists every conversational event of the episode, not only its first
