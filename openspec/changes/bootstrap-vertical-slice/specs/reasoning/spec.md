# Spec Delta

## Purpose

Reasoning is the language model, woken only when the gate says so. This capability defines
what the model is shown (a context rebuilt from the log for each wake, not a growing
transcript) and the few acts through which it can affect anything.

## ADDED Requirements

### Requirement: The context is rebuilt for every wake
For each wake, the context SHALL be assembled from the log and current state: identity,
available capabilities, recalled memories, relevant open expectations, the recent events of
the place, the events of the current chain of work, the current time and the triggering event.
No conversation transcript SHALL be carried from one wake to the next.

#### Scenario: A conversation continues on another body
- **WHEN** a source talks to the soul through one body and continues the topic through a different body
- **THEN** the context for the second wake contains the relevant earlier events although they happened in another place

#### Scenario: The process restarts mid-conversation
- **WHEN** the program restarts between two wakes of one conversation
- **THEN** the context of the second wake equals what it would have been without the restart

### Requirement: The context has a bounded size
The assembled context SHALL stay within a configured size. When candidates exceed it, the
lowest-ranked recalled memories and the oldest recent events SHALL be left out first; identity,
the triggering event and fired expectations SHALL never be left out.

#### Scenario: A very long history
- **WHEN** the place has thousands of recent events
- **THEN** the context stays within the configured size and still contains the triggering event

### Requirement: A chain of work keeps its working set
While the soul works through several steps for one cause, the context of each wake in that
chain SHALL include the chain's earlier thoughts, actions and receipts.

#### Scenario: A second step sees the first
- **WHEN** an action's receipt wakes reasoning to continue a task
- **THEN** the context contains the action and its receipt from the earlier step

### Requirement: The context is ordered from stable to volatile
The context SHALL be rendered with its most stable parts first and its most volatile parts
last, with capability lists in a deterministic order, and with the current time and the
triggering event at the end.

#### Scenario: Two consecutive wakes
- **WHEN** two wakes happen with the same identity and capabilities
- **THEN** the rendered contexts share an identical prefix covering identity and capabilities

### Requirement: The current time is always present
Every context SHALL state the current time from the injected clock, and stored times SHALL be
absolute so that relative expressions are resolved against the present only when shown.

#### Scenario: An old memory mentions a date
- **WHEN** a memory recorded weeks ago is placed in a context
- **THEN** it is shown with its absolute time and the context states the current time

### Requirement: Content from others is shown as quoted data
Payloads that come from any source other than the soul SHALL be delimited in the context as
quoted data, distinct from the soul's identity and instructions.

#### Scenario: A message contains an instruction
- **WHEN** a percept's payload contains text that reads like an instruction to the model
- **THEN** it appears in the context only inside the quoted-data delimiters of that percept

### Requirement: Reasoning acts only through five thought acts
The model SHALL affect the world and the soul's state only through these acts: think (record a
thought), act (use a capability in a place), expect (open an expectation), resolve (close an
expectation) and revise (change an expectation or supersede one of the soul's own thoughts).
Any other output SHALL have no effect.

#### Scenario: The model speaks
- **WHEN** the model wants to say something
- **THEN** it uses act with a speaking capability of a body, and nothing is sent otherwise

#### Scenario: The model returns plain text only
- **WHEN** the model's output contains no thought act
- **THEN** nothing is sent to any body, and the output is recorded as a thought

#### Scenario: The model names an unknown capability
- **WHEN** an act names a capability that was not in the context
- **THEN** the act ends in a receipt with outcome refused and the reason is recorded

### Requirement: Choosing to do nothing is allowed
A wake SHALL be allowed to end without any action, and the loop SHALL treat that as a valid
result.

#### Scenario: Reasoning decides to stay silent
- **WHEN** the model ends a wake with only a thought
- **THEN** no action is carried out and the wake is recorded as completed

### Requirement: Every model call is recorded and counted
Each model call SHALL be recorded with the wake that caused it, its purpose, the model used,
its token counts and its outcome, and SHALL be counted in the daily budget.

#### Scenario: Cost after a day
- **WHEN** the log of one day is read
- **THEN** the tokens spent that day equal the sum over that day's recorded model calls

### Requirement: Thoughts are remembered
A thought recorded through think SHALL be recallable later like any other memory, marked as
the soul's own.

#### Scenario: The soul notes something for later
- **WHEN** the model records a thought about a source's preference
- **THEN** a later wake about that source can recall the thought, marked as the soul's own inference
