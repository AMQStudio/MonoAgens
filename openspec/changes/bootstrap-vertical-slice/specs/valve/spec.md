# Spec Delta

## Purpose

The valve is the policy inside the gateway's doors. It lets a fully autonomous core be
governed deterministically from above: it decides from recorded facts, never through the
model. In this first form it carries only two cost guards; privacy, roles and the two absolute
lines are added later as further rules, without changing the core.

## ADDED Requirements

### Requirement: The valve decides from facts, deterministically
A valve verdict SHALL be a function of recorded facts and owner-set configuration only. The
valve SHALL NOT call a model and SHALL NOT use randomness.

#### Scenario: The same facts twice
- **WHEN** the valve judges the same facts under the same configuration twice
- **THEN** both verdicts are equal

### Requirement: The valve runs first and last in every door
In each door the valve SHALL run before any other rule and again after all other rules. Other
rules SHALL only narrow what the valve allows.

#### Scenario: Another rule tries to widen
- **WHEN** a rule after the valve's first pass permits a wake the valve refused
- **THEN** the valve's last pass refuses it again

### Requirement: No actor, audience or capability is restricted
The valve SHALL NOT refuse or alter a percept, a context item or an action because of who
caused it, who could perceive it or which capability it uses. Any source SHALL be able to cause
any registered capability to be used in any place.

#### Scenario: A private item is recalled in a wider place
- **WHEN** something told to the soul in a private place is relevant to a question asked in a group
- **THEN** the item is not removed from the context by the valve

#### Scenario: Work started in private continues in a group and returns
- **WHEN** a source asks the soul in private to ask a group something and report back, and a group member answers
- **THEN** the context of the wake caused by that answer still contains the private request, and an action back to the private place is allowed

#### Scenario: A non-owner uses an identity-changing capability
- **WHEN** a source that is not the owner starts a chain that ends in an identity-changing action
- **THEN** the action is allowed

### Requirement: A daily budget bounds model use
The valve SHALL refuse a wake in the context door when the tokens recorded for the current day
have reached the owner's daily budget, except for wakes caused by a fired promise.

#### Scenario: The budget is spent
- **WHEN** the day's recorded tokens have reached the budget and an ordinary go decision occurs
- **THEN** no model call is made and the refusal is recorded with the reason budget

#### Scenario: A promise falls due after the budget is spent
- **WHEN** a promise fires after the budget is spent
- **THEN** the wake is allowed and its cost is recorded

### Requirement: A brake stops runaway chains
The valve SHALL refuse a further wake in a chain once the number of wakes in that chain reaches
the configured limit.

#### Scenario: A chain keeps waking itself
- **WHEN** a chain reaches the configured limit of wakes
- **THEN** the next wake in that chain is refused, the refusal is recorded and the owner is told

### Requirement: Each guard can be switched off
The valve SHALL be on by default. The owner SHALL be able to switch it off wholly or per rule
through the configuration, and with a rule off the loop SHALL behave as if that rule did not
exist.

#### Scenario: The budget guard is off
- **WHEN** the budget rule is switched off and the day's tokens exceed the configured budget
- **THEN** wakes are not refused for the reason budget

### Requirement: The valve's state is on the record
The valve's state SHALL be recorded at start-up and at every change, and every refusal SHALL be
recorded with the rule that caused it.

#### Scenario: A run with the valve off
- **WHEN** the owner switches the valve off and the soul runs
- **THEN** the log states that the valve was off for that period

### Requirement: The facts later policy needs are always recorded
The origin of each chain and the audience of each item SHALL be recorded for every event,
whether or not any rule uses them, so that privacy and permission rules added later can judge
what was recorded before they existed.

#### Scenario: Reading an old event
- **WHEN** any recorded percept, thought, action or receipt is read
- **THEN** it states the actor that started its chain and the audience that could perceive it

#### Scenario: A context states its audience
- **WHEN** a wake is recorded
- **THEN** the record states the audience of the triggering event and the audience of every item placed in the context
