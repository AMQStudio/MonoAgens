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
A provider SHALL enforce strict nullable schemas (`R | null`) and translate output into typed thought acts—including `[0, 20]` anchored confidence, balanced `1–5` motivation, and `consolidate` acts (`gist`, `lesson`, `procedure`)—falling back to one thought on act-less text or reporting malformed output as a failed wake.

#### Scenario: Typed consolidate act and nullable field decode cleanly
- **WHEN** a model emits a typed `consolidate` act or explicit `null` optional fields under `R | null` schema
- **THEN** the provider decodes the typed payload or rejects schema violations as a failed wake with diagnostic reason

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
Vectors SHALL flow through one embedding interface defaulting to unified Apache 2.0 `EmbeddingGemma 2` (`256d` MRL, `8K` context) across text, image, and `audio_voice` modalities—runnable locally via native `ort` (`ONNX Runtime`) / `LiteRT`, local stdio/HTTP sidecar, or remote OpenAI-compatible endpoint—alongside `none`, deterministic offline, scripted Lab, and `EmbeddingGemma-300M` ablation providers plus a frozen `A-14` `dialogue_act_gap` logistic head.

#### Scenario: Local EmbeddingGemma 2 runner swaps between native runtime and sidecar
- **WHEN** settings switch the local `EmbeddingGemma 2` backend between native `ort`/`LiteRT` execution and a local stdio/HTTP sidecar
- **THEN** the provider produces identical normalized `256d` vectors tagged with `(embed_model_ver, dim_d, prompt_regime_id)` without changing core or caller code

#### Scenario: Frozen dialogue-act gap scorer operates independently of none embeddings
- **WHEN** an epistemic information-gap check scores a percept while main embeddings are `none`
- **THEN** the frozen logistic `(w_gap, b_gap)` scorer still produces a bounded gap probability without blocking the loop

#### Scenario: Attachment handle media resolves only after access checks
- **WHEN** a multimodal percept carries an `AttachmentHandle`
- **THEN** `reason` resolves the referenced bytes only after `VisibleScope` projection and door (`Valve`) authorization pass

#### Scenario: No embedder
- **WHEN** no embedding provider is selected
- **THEN** events carry no vector and the loop runs with the vector-dependent mechanisms at their neutral values

#### Scenario: The embedder fails for one event
- **WHEN** the embedding provider fails for an event
- **THEN** the event is processed without a vector and the failure is recorded

### Requirement: Vectors of different embedders are never compared
Every stored vector SHALL record `(embed_model_ver, dim_d, prompt_regime_id)` and apply mandatory task instruction prefixes (`t_clf`, `t_sts`, `t_q`, `t_d`). Similarity search SHALL strictly partition and compare vectors only when all three tuple components match.

#### Scenario: Asymmetric task prefixes partition by version, dimension, and regime
- **WHEN** `t_q` queries search stored `t_d` vectors across mixed embedder versions or dimensions
- **THEN** the index filters strictly to matching `(embed_model_ver, dim_d, prompt_regime_id)` partitions before computing similarity

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
