# Spec Delta

## Purpose

Expectations are what the soul holds about the future: promises it made and predictions of
what should follow its acts. Their lifecycle lives in code, not in the model's attention, so a
promise is kept even when nobody mentions it again.

## ADDED Requirements

### Requirement: An expectation is a typed record
An expectation SHALL be a record with a kind (effect, response, wait, promise or goal), a
condition, the place and source it concerns, an expiry and the event that created it. Its
lifecycle SHALL be driven by the loop, not by a model call.

#### Scenario: A promise is opened
- **WHEN** reasoning commits to doing something at a later time
- **THEN** an open promise with that due time, place and source is recorded

### Requirement: A time condition fires on its own
An expectation whose condition is a due time SHALL fire when the injected clock reaches that
time, with no external event needed, and firing SHALL wake reasoning as a commitment.

#### Scenario: A reminder comes due
- **WHEN** the soul promised a reminder in two hours and two hours pass in silence
- **THEN** reasoning wakes with the promise in its context and no earlier than the due time

#### Scenario: The soul was not running at the due time
- **WHEN** promises fell due while the program was stopped and the program starts later
- **THEN** they are marked overdue at start-up and reasoning wakes once for each place that has one

### Requirement: A promise is kept, retried or renegotiated, never dropped
A promise SHALL stay open until it is kept, cancelled or renegotiated. A receipt of failed,
refused, held or unknown SHALL NOT count as keeping it: the promise SHALL be retried with a
growing delay up to a limit, after which reasoning SHALL wake to inform the source or
renegotiate. A promise kept after its due time SHALL be recorded as kept late.

#### Scenario: The attempt fails
- **WHEN** the act that would keep a promise ends in a receipt of failed
- **THEN** the promise is still open, still counts toward promise debt, and is retried after a delay

#### Scenario: A promise is kept
- **WHEN** reasoning carries out the promised act and resolves the promise
- **THEN** the promise is recorded as kept with a reference to the act that kept it

#### Scenario: Retries run out
- **WHEN** the retry limit is reached without success
- **THEN** reasoning wakes as a commitment with the failed promise in its context

#### Scenario: Kept after the due time
- **WHEN** a promise is kept later than its due time
- **THEN** it is recorded as kept late, separately from kept

### Requirement: Every act opens predictions of what should follow
After each action the loop SHALL open an effect prediction when the capability declares a
sensed effect, and a response prediction toward the source and place that a communicative act
was aimed at. Each SHALL have an expiry.

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

### Requirement: Promise debt is visible to the gate
The share of the soul's open promises that fall due soon SHALL be available to the gate as a
number.

#### Scenario: Several promises are due within the hour
- **WHEN** most open promises fall due within the next hour
- **THEN** the promise debt the gate sees is higher than when none is due

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

### Requirement: A goal holds a multi-step task together
Reasoning SHALL be able to open a goal: an expectation with a completion condition, a due time
and an expiry. A receipt or reply linked to the goal's chain, or an event that matches the
goal's open prediction, SHALL be attributed to the goal and SHALL wake reasoning as a
commitment. An event that is only similar to the goal's condition SHALL be attributed after one
classification and SHALL NOT bypass the gate. A goal SHALL end when it is resolved, answered,
expired or revised away.

#### Scenario: A receipt arrives in a busy place
- **WHEN** the receipt of a goal's action arrives among many unrelated events
- **THEN** it is attributed to the goal and wakes reasoning as a commitment, whatever its salience

#### Scenario: A related remark
- **WHEN** an unlinked event is similar to a goal's condition and one classification says it concerns the goal
- **THEN** it is attributed to the goal and appraised by the gate without bypassing it

#### Scenario: A goal expires
- **WHEN** a goal's expiry passes
- **THEN** it is recorded as expired and no later event is attributed to it
