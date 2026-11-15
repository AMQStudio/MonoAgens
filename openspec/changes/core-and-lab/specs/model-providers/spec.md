# Spec Delta

## Purpose

Model providers connect the core to a language model and to an embedding model without the
core knowing which. They turn a context plan into a request, a response into thought acts, and
content into vectors, so that any model can be the soul's reasoning.

## ADDED Requirements

### Requirement: Language models are interchangeable
Reasoning SHALL be reached through one provider interface, with at least a provider for
OpenAI-compatible endpoints and a scripted provider that returns prepared outputs. Changing
the provider or the model SHALL require settings only.

#### Scenario: The scripted provider
- **WHEN** the scripted provider is selected
- **THEN** a full loop runs without any network access

#### Scenario: Another model
- **WHEN** the model name is changed in the settings
- **THEN** the same scenario runs against the new model with no other change

### Requirement: A provider's output becomes thought acts or a thought
A provider SHALL translate the model's output into protocol thought acts. Output that contains
no act SHALL become a single thought. Output that cannot be translated SHALL be reported as a
failed wake with the reason.

#### Scenario: Several acts in one output
- **WHEN** the model returns a thought, an action and an expectation together
- **THEN** all three are returned, in the model's order

#### Scenario: Malformed act
- **WHEN** the model returns an act with missing or mistyped fields
- **THEN** the wake is recorded as failed with the field that was wrong

### Requirement: The prompt is rendered from the context plan
The provider side SHALL render the request from the context plan in the plan's order, wrapping
content from sources other than the soul as quoted data and stating times as absolute times.

#### Scenario: The same plan twice
- **WHEN** the same context plan is rendered twice
- **THEN** the two requests are identical

### Requirement: Embeddings are interchangeable and optional
Vectors SHALL be produced through one embedding interface with at least: none, a deterministic
offline embedder, an OpenAI-compatible embedder and a scripted embedder for the Lab. With none
selected the soul SHALL still run.

#### Scenario: No embedder
- **WHEN** no embedding provider is selected
- **THEN** events carry no vector and the loop runs with the vector-dependent mechanisms at their neutral values

#### Scenario: The embedder fails for one event
- **WHEN** the embedding provider fails for an event
- **THEN** the event is processed without a vector and the failure is recorded

### Requirement: Vectors of different embedders are never compared
Each stored vector SHALL be tagged with the embedder that produced it, and similarity SHALL be
computed only between vectors of the same embedder.

#### Scenario: The embedder is changed
- **WHEN** a soul with stored vectors is run with a different embedder
- **THEN** old vectors are not compared with new ones, and the mismatch is reported

### Requirement: Usage and failures are reported for the record
Each provider call SHALL report its model, its token counts including cached tokens when the
provider states them, its latency and its outcome, and failures SHALL be classified
(authentication, rate limit, timeout, server, invalid output) without including any secret.

#### Scenario: Rate limited
- **WHEN** the provider answers with a rate-limit error
- **THEN** the wake is recorded as failed with the class rate limit and no key material

### Requirement: Secrets and endpoints come from the environment
Keys and endpoints SHALL be read from environment variables or a separate secrets file, and
SHALL NOT be accepted from scenario files or soul files.

#### Scenario: A key in a scenario file
- **WHEN** a scenario file contains a provider key
- **THEN** loading it fails with an error that says where secrets belong

### Requirement: Every call has a time limit
Each provider call SHALL run under a time limit, after which it is reported as a timeout.

#### Scenario: The endpoint hangs
- **WHEN** the endpoint does not answer within the limit
- **THEN** the call ends as a timeout and the loop continues
