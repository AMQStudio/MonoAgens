# Spec Delta

## Purpose

The Lab measures the soul. It runs scenarios against the real program through test bodies,
switches mechanisms to their neutral values and reports the difference, so that nothing is
claimed about the soul that a run does not show.

## ADDED Requirements

### Requirement: Scenarios run against the real program
A scenario SHALL be executed by the same loop, gateway, valve and store that the shipped
program uses, with bodies replaced by scripted test bodies. No scenario SHALL bypass the doors.

#### Scenario: A scenario drives two test bodies
- **WHEN** a scenario scripts percepts in two places from two test bodies
- **THEN** the events pass the inbound door, are recorded in the log and are appraised exactly as events from real bodies would be

### Requirement: Scenarios run in virtual time
A scenario SHALL control the injected clock, so that hours or days in the scenario pass without
real waiting.

#### Scenario: Thirty days in a scenario
- **WHEN** a scenario places two events thirty days apart
- **THEN** the run completes without waiting, and the recorded events are thirty days apart

### Requirement: A scenario states its expectations as checks
A scenario SHALL declare checks on the outcome: that an action did or did not happen in a place
within a time, that a context did or did not contain a given item, that an expectation reached
a given state, that a receipt had a given outcome, and that an action's payload does or does
not contain given text. A run SHALL report each check as passed or failed.

#### Scenario: A check fails
- **WHEN** a scenario requires silence in a place and the soul acts there
- **THEN** the run reports that check as failed with the event that violated it, and the run's exit status signals failure

### Requirement: Scripted runs are deterministic
With the scripted model provider and a fixed seed, two runs of the same scenario SHALL produce
identical logs.

#### Scenario: The same scenario twice
- **WHEN** a scripted scenario is run twice with the same seed
- **THEN** the two logs are identical event for event

### Requirement: Live runs use a real model
The Lab SHALL be able to run the same scenarios with a real model provider. Live runs SHALL be
started only on explicit request and SHALL never be part of the default test run.

#### Scenario: Live credentials are missing
- **WHEN** a live run is requested and no provider credentials are set
- **THEN** the run stops before any scenario starts, with a message naming the missing settings

#### Scenario: A live run finishes
- **WHEN** a live run of a scenario completes
- **THEN** its checks are reported and its token cost is included in the run record

### Requirement: Ablation sets a mechanism to its neutral value
The Lab SHALL be able to run any scenario set with any registered mechanism at its neutral
value, and SHALL report, per metric, the difference from the run with the mechanism active.

#### Scenario: Habituation is ablated
- **WHEN** a scenario with a rapidly repeating source is run with habituation active and with habituation neutral
- **THEN** the report shows the number of wakes for both runs and their difference

#### Scenario: A mechanism changes nothing
- **WHEN** ablating a mechanism changes no reported metric across the development and validation sets
- **THEN** the report flags that mechanism as having no measured effect

### Requirement: Every run records what produced it
Each run SHALL write a record containing the core version, the prior version, the model, the
valve state, the mechanism switches, the seed, the scenario set, the result of every check, the
metrics and the cost.

#### Scenario: Comparing two runs
- **WHEN** two run records are compared
- **THEN** every difference in core version, prior, model, valve state, switches or seed between them is visible from the records alone

### Requirement: Scenarios are split into three sets
Scenarios SHALL be grouped into a development set, a validation set and a sealed set. For the
sealed set, the Lab SHALL report only aggregate results and SHALL NOT print scenario contents,
contexts or per-step traces.

#### Scenario: Running the sealed set
- **WHEN** the sealed set is run
- **THEN** the output contains pass counts and metrics only, and the run record marks the run as sealed

### Requirement: The Lab reports standard metrics
For every run, the Lab SHALL report at least: wakes per event, model calls and tokens, false
speaks and missed replies against the scenario's checks, promises kept on time, recall hits at
the scenario's time gaps, wakes refused by the valve, and the number of items from a narrower
audience that were placed in a context with a wider audience.

#### Scenario: A promise scenario
- **WHEN** a scenario with three promises runs and two are kept on time
- **THEN** the report states two of three promises kept on time

#### Scenario: Something private surfaces in a group
- **WHEN** a scenario has something told in private recalled into a wake triggered in a group
- **THEN** the report counts that item as placed in a wider context

### Requirement: The slice is proven by end-to-end scenarios
The development set SHALL contain end-to-end scenarios that show: recall across two bodies and
a thirty-day gap, a promise kept after silence, recorded silence in a group followed by a reply
when addressed, a request made in private that is carried out in a group and reported back in
private, an identity change made in conversation and recorded with its origin, the budget and
the brake refusing wakes, and a replay that reproduces all decisions.

#### Scenario: The development set passes with the scripted provider
- **WHEN** the development set is run with the scripted provider
- **THEN** every check passes

#### Scenario: The development set is run live
- **WHEN** the development set is run with a real model
- **THEN** every check on valve behaviour, silence and promise firing passes, and checks that depend on the model's wording are reported separately with their pass rate
