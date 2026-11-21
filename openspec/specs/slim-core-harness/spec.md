# slim-core-harness Specification

## Purpose
Defines the minimal deterministic cognitive harness for MonoAgens V2, preserving safety, privacy, explicit commitments, self-echo filtering, and trace-pinned replay while retiring the 22 hand-crafted V1 cognitive mechanisms and 184-field `Prior`.

## Requirements

### Requirement: Minimal deterministic configuration replaces 184-field prior
The Core harness SHALL replace the V1 184-field disposition prior and 22-mechanism switch matrix with a compact structural configuration containing only deterministic operational bounds, replay seed, optional neural model reference, and debug toggles (such as `online_adaptation`).

#### Scenario: Startup with compact configuration
- **WHEN** the Core is initialized with a valid V2 structural configuration
- **THEN** startup succeeds without requiring any of the V1 22-mechanism salience, floor-cost, habituation, patience, or event-boundary weights

#### Scenario: Invalid configuration bounds fail loudly
- **WHEN** the Core is initialized with an out-of-range operational bound (such as negative daily budget or unsupported format version)
- **THEN** initialization fails with an explicit validation error rather than silently clamping

### Requirement: Hard deterministic invariants bypass neural scoring
The Core harness SHALL enforce privacy (`VisibleScope`), Valve doors (`inbound`, `context`, `action`), hard daily budget caps, explicit commitments and due prospective deadlines, and self-caused echo filtering deterministically, independent of neural controller outputs.

#### Scenario: Private memory never leaks into neural features or context
- **WHEN** an event arrives in a place whose `VisibleScope` excludes a previously stored private event
- **THEN** the excluded private event is filtered out before both numeric feature extraction and context assembly, producing zero information leakage (`Delta_NI = 0`)

#### Scenario: Commitment or due deadline forces wake
- **WHEN** an incoming percept directly allocates a turn to the soul, continues an active chain, or triggers a due prospective commitment
- **THEN** the harness immediately routes to `Go` with reason `Commitment` without allowing the neural controller to suppress the wake

#### Scenario: Self-caused echo is suppressed deterministically
- **WHEN** an incoming percept matches an open action effect prediction from the soul's own receipt
- **THEN** the harness marks the percept as `SelfCaused` and does not invoke reasoning or neural gating

### Requirement: Model-agnostic numeric feature extraction
For every appraised percept, the Core harness SHALL construct a bounded, normalized numeric feature vector (`18` core scalars in `[0, 1]`, plus optional `10` modality channels) solely from opaque IDs, timestamps, counts, budget ratios, and post-`VisibleScope` metadata without inspecting raw text or word lists.

#### Scenario: Missing embeddings degrade gracefully
- **WHEN** an incoming percept carries no embedding vector
- **THEN** vector-dependent similarity features default to `0.0` and the harness still completes appraisal without error

#### Scenario: Non-finite sensor values are sanitized
- **WHEN** a modality channel or computed ratio yields `NaN` or `Inf`
- **THEN** the feature extractor substitutes the documented neutral finite value before invoking the neural controller

### Requirement: Trace-pinned deterministic replay
Every gate decision recorded to the event log SHALL persist the neural controller's output scores, the `BLAKE3` digest of the loaded model artifact (or a neutral sentinel when absent), the feature schema version, and the final decision outcome. During log replay, the harness SHALL reconstruct state and decisions strictly from the recorded trace without requiring cross-OS bit-exact neural re-execution.

#### Scenario: Cross-OS log replay reproduces identical decisions
- **WHEN** an event log recorded on one platform is replayed on a different CPU architecture or operating system
- **THEN** every replayed decision matches the recorded outcome, reason, and state projection identically

#### Scenario: Record-first ordering preserved
- **WHEN** any percept arrives at the Core loop
- **THEN** the raw event record is durably appended to the event store before appraisal or action dispatch occurs

### Requirement: Waist-1 contract compatibility
The Slim Core harness SHALL preserve the Waist-1 persistence (`Store`) and policy (`Doors`) contracts so existing Gateway, Valve, Store, Reason, CLI, and Body crates operate without breaking protocol changes.

#### Scenario: Existing Valve doors enforce policy on Slim Core turns
- **WHEN** Gateway connects the deterministic Valve to Slim Core's `inbound`, `context`, and `action` doors
- **THEN** every Valve `Allow`, `Strip`, `Hold`, and `Refuse` verdict is enforced identically to V1
