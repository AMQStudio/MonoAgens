# Spec Delta

## Purpose

The Lab measures the soul. It runs scenarios against the real core through scripted test
bodies, switches mechanisms to their neutral values and reports the difference, so that
nothing is claimed about the soul that a run does not show.

## ADDED Requirements

### Requirement: Scenarios run against the real core
A scenario SHALL be executed by the same core, store and reasoning code that any later program
will use, with the world supplied by scripted test bodies. No scenario SHALL bypass the doors
or write to the log directly.

#### Scenario: A scenario drives two test bodies
- **WHEN** a scenario scripts percepts in two places from two test bodies
- **THEN** the events pass the inbound door, are recorded and are appraised by the core

### Requirement: Test bodies stand for any kind of body
A scenario SHALL be able to declare test bodies with their own places, sources, capabilities,
scripted receipts and scripted percepts that follow an action, so that a chat room, a device
with sensors or a game world can be scripted with the same means.

#### Scenario: A device body
- **WHEN** a scenario declares a body with a "push" capability and a sensor percept that follows it
- **THEN** the soul's act through that body ends in a receipt and the sensor percept answers the act's prediction

#### Scenario: The sensor stays silent
- **WHEN** the scenario scripts no sensor percept after the act
- **THEN** the act's prediction expires and that absence is recorded

### Requirement: Scenarios run in virtual time
A scenario SHALL control the injected clock, including how long model calls take, so that
hours or days pass without real waiting.

#### Scenario: Thirty days in a scenario
- **WHEN** a scenario places two events thirty days apart
- **THEN** the run completes without waiting, and the recorded events are thirty days apart

#### Scenario: Events during a slow model call
- **WHEN** a scenario sets a model latency longer than the gap between its next events
- **THEN** those events are appraised before the model's result is processed

### Requirement: A scenario states its expectations as checks
A scenario SHALL declare checks on the outcome: an action did or did not happen in a place
within a time; a context did or did not contain an item or a label; an expectation, belief or
ledger card reached a given state; a receipt had a given outcome; a decision had a given
outcome or reason; an action's payload does or does not contain given text. A run SHALL report
each check as passed or failed.

#### Scenario: A check fails
- **WHEN** a scenario requires silence in a place and the soul acts there
- **THEN** the run reports that check as failed with the event that violated it, and the exit status signals failure

### Requirement: Scripted runs are deterministic
With the scripted model provider, the scripted embedder and a fixed seed, two runs of the same
scenario SHALL produce identical logs.

#### Scenario: The same scenario twice
- **WHEN** a scripted scenario is run twice with the same seed
- **THEN** the two logs are identical event for event

### Requirement: Live runs use a real model
The Lab SHALL be able to run the same scenarios with a real model provider. Live runs SHALL be
started only on explicit request and SHALL never be part of the default test run. Checks SHALL
be tagged structural or wording, and live results SHALL report the two separately.

#### Scenario: Live credentials are missing
- **WHEN** a live run is requested and no provider settings are present
- **THEN** the run stops before any scenario starts, with a message naming the missing settings

#### Scenario: A live run finishes
- **WHEN** a live run completes
- **THEN** structural checks are reported as passed or failed, wording checks as a pass rate, and the token cost is in the run record

### Requirement: A cost guard bounds every run
The Lab SHALL install at the context door a guard that refuses further wakes once a run's
token limit is reached or a chain's wake limit is reached, and SHALL record each refusal. Both
limits SHALL be adjustable and removable per run.

#### Scenario: A chain keeps waking itself
- **WHEN** a chain reaches the wake limit
- **THEN** its next wake is refused, the refusal is recorded and the run reports it

#### Scenario: The token limit is reached
- **WHEN** a live run reaches its token limit
- **THEN** no further model call is made and the run ends with a report that the limit stopped it

### Requirement: Ablation sets a mechanism to its neutral value
The Lab SHALL be able to run any scenario set with any registered mechanism at its neutral
value. It SHALL run each side with at least five seeds and SHALL report, per metric, the
difference with a confidence interval, and the cost. A difference within seed noise SHALL be
reported as no measured effect.

#### Scenario: Habituation is ablated
- **WHEN** a scenario with a rapidly repeating source is run with habituation active and neutral
- **THEN** the report shows the number of wakes for both, their difference and its confidence interval

#### Scenario: A mechanism changes nothing
- **WHEN** ablating a mechanism changes no reported metric beyond seed noise across the development and validation sets
- **THEN** the report flags that mechanism as having no measured effect

### Requirement: Every mechanism has a scenario that tests its claim
For each registered mechanism, the development set SHALL contain at least one scenario whose
checks or metrics are expected to differ when that mechanism is neutral.

#### Scenario: A mechanism without a scenario
- **WHEN** the Lab lists mechanisms and one has no scenario naming it
- **THEN** the listing marks it as unproven and the coverage command exits with a non-zero status

### Requirement: Every run records what produced it
Each run SHALL write a record containing the core version, the prior version, the model and
embedder, the door rules installed, the mechanism switches, the learning switches, the seed,
the scenario set, the result of every check, the metrics and the cost by purpose.

#### Scenario: Comparing two runs
- **WHEN** two run records are compared
- **THEN** every difference in core version, prior, model, door rules, switches or seed between them is visible from the records alone

### Requirement: Scenarios are split into three sets
Scenarios SHALL be grouped into a development set, a validation set and a sealed set. For the
sealed set, the Lab SHALL report only aggregate results and SHALL NOT print scenario contents,
contexts or per-step traces.

#### Scenario: Running the sealed set
- **WHEN** the sealed set is run
- **THEN** the output contains pass counts and metrics only, and the run record marks the run as sealed

### Requirement: The Lab reports behaviour against annotation
A scenario SHALL be able to annotate, per event, whether a reply was warranted. From it the Lab
SHALL report when-to-speak precision and recall, false speaks, missed replies, the soul's share
of messages against one over the number of participants, and the distribution of its reply
delays.

#### Scenario: An annotated group conversation
- **WHEN** a scenario marks five of forty events as warranting a reply and the soul replies to four of those and to one other
- **THEN** the report states precision 0.8 and recall 0.8

### Requirement: The Lab reports memory and honesty metrics
The Lab SHALL report: recall hits at gaps of one, seven and thirty days; promises kept on time;
assertions of detail made without recollection; superseded beliefs presented as current;
correct answers on scripted false-belief questions; tokens and model calls per event and by
purpose; the share of prompt tokens served from cache when the provider states it; and the
number of items from a narrower audience placed in a wider context.

#### Scenario: A promise scenario
- **WHEN** a scenario with three promises runs and two are kept on time
- **THEN** the report states two of three promises kept on time

### Requirement: The foundation is proven end to end
The development set SHALL contain end-to-end scenarios for: recall across places and a
thirty-day gap; a promise kept after silence and after downtime; recorded silence in a group
and a reply when addressed; a multi-step task across two places; a device body with a sensor;
and replay of each resulting log.

#### Scenario: The foundation scenarios pass scripted
- **WHEN** the foundation scenarios are run with the scripted provider
- **THEN** every check passes and every log replays cleanly

### Requirement: Wave 1 is proven end to end
The development set SHALL contain end-to-end scenarios for: yielding to another speaker; a
withheld unallocated turn; a free-text condition met later; a reply that shows an act did not
land; a missed backlog; a stale belief superseded after sleep; a false-belief question; told
against observed; an unreliable source; and bounded learning with rollback.

#### Scenario: The development set passes with the scripted provider
- **WHEN** the development set is run with the scripted provider
- **THEN** every check passes

#### Scenario: The development set is run live
- **WHEN** the development set is run with a real model
- **THEN** every structural check passes, and wording checks are reported with their pass rate

### Requirement: A soul can persist between Lab sessions
The Lab SHALL be able to keep a soul in a directory holding its identity and its log, and to
continue that soul in a later session.

#### Scenario: Continuing yesterday's soul
- **WHEN** a session is started on a soul directory used before
- **THEN** the soul's memories, open promises and learned values are those it had at the end of the earlier session

### Requirement: The owner can probe a soul from the terminal
The Lab SHALL offer a probe in which a person types as one source in one place of a persistent
soul in real time, and sees what the soul sends there, including messages the soul sends on
its own when a promise falls due.

#### Scenario: A reminder arrives unprompted
- **WHEN** the person asked for a reminder in two minutes and types nothing more
- **THEN** the soul's reminder appears after two minutes

### Requirement: Decisions can be explained and logs replayed
The Lab SHALL be able to show a log in order with filters, explain a single decision with all
its numbers, list the mechanisms with their claims, neutral and current values, and replay a
log without a model, reporting whether every decision was reproduced.

#### Scenario: Why did it stay silent
- **WHEN** a decision with outcome silent is explained
- **THEN** the signals, salience, habituation, threshold terms, probability and draw of that decision are printed

#### Scenario: A clean replay
- **WHEN** a log recorded by this version is replayed
- **THEN** the Lab reports that all decisions were reproduced and exits with status zero

### Requirement: Output is usable by programs
Commands that report results SHALL offer a machine-readable output format, and errors SHALL go
to the error stream with a non-zero exit status.

#### Scenario: A script reads Lab results
- **WHEN** a run is made with the machine-readable option
- **THEN** standard output contains only the structured result

### Requirement: Calibrated values are computed from the development set
The Lab SHALL compute the values that the research defines by calibration (the
feeling-of-knowing threshold and scale, and the wake band with its bounds) from the development
set for the embedder in use, and SHALL record them with the run. It SHALL NOT calibrate on the
validation or sealed sets.

#### Scenario: The embedder is changed
- **WHEN** a run uses a different embedder than the last calibration
- **THEN** the values are computed again for that embedder and recorded in the run record

### Requirement: The rebuilt context is compared with a transcript
The Lab SHALL be able to run a scenario with the context replaced by the full transcript of
the place and chain, and SHALL report task success, tokens and cache share for both forms,
including after a restart in the middle of a task.

#### Scenario: A restart in the middle of a task
- **WHEN** a multi-step scenario is run both ways with a restart between two steps
- **THEN** the report shows success and cost for the rebuilt context and for the transcript
