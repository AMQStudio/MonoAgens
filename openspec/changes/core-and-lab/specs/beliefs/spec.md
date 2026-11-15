# Spec Delta

## Purpose

Beliefs govern what the soul may assert about the past and the world. Every remembered item
carries how the soul came to know it, how much it trusts it and whether it still holds, so
that the soul can tell knowing from guessing and can say that it does not know.

## ADDED Requirements

### Requirement: Every belief has a basis that the core checks
Each belief SHALL carry a basis: observed (it rests on events the soul witnessed), told (it
rests on what a named source asserted), inferred (the soul concluded it) or self (the result
of the soul's own act). Reasoning SHALL state the basis with the events it rests on, and the
core SHALL downgrade observed to inferred when those links do not reach percepts the soul
witnessed. That a source said something is itself observed; what it said is told.

#### Scenario: A source reports something
- **WHEN** a source tells the soul a fact about a third party
- **THEN** the belief in that fact has the basis told and names that source, while the event of the source saying it is observed

#### Scenario: An unsupported claim of observation
- **WHEN** reasoning records a belief as observed and its links reach no percept the soul witnessed
- **THEN** the belief is stored as inferred and the downgrade is recorded

#### Scenario: The soul concludes something
- **WHEN** reasoning records a conclusion through think or sleep derives one
- **THEN** the belief's basis is inferred

### Requirement: Verbatim and gist are kept in parallel and consolidation verifies conflicts
Recorded events are the verbatim trace and summaries are the gist trace; both SHALL stay intact.
During `consolidate` (`reason`, not inside blind core, `A-01`), details absent from source events
cause the gist to be superseded.

#### Scenario: A summary says more than happened
- **WHEN** consolidation detects a summary stating a detail absent from its linked source events
- **THEN** the summary is superseded and recall presents the source events as recollected

### Requirement: A belief has a validity that can end
A belief SHALL carry the time it is valid from and, once replaced, the time it stopped being
valid and what replaced it. Replacing a belief SHALL NOT remove the old one from the log.

#### Scenario: A fact changes
- **WHEN** a source says their situation changed
- **THEN** the old belief is marked as ended and points to the new one, and both remain readable

### Requirement: Times are stored absolute
Times in memories SHALL be stored as absolute times. Relative wording SHALL be produced only
when an item is shown, against the current time.

#### Scenario: An item from last week
- **WHEN** an item recorded seven days ago is shown
- **THEN** it carries its absolute time, and any relative wording says seven days ago

### Requirement: Credence is counted evidence across inferred, told, and procedural items
Told assertions (initialized from source reliability prior, `A-01`), lessons, procedures, and
inferred beliefs SHALL hold `(α, β)` counts updating on cited outcomes, decaying toward prior at
`λ_domain`, recomputable cleanly on `hard_forget` (`R-34`).

#### Scenario: A told assertion initializes from source prior
- **WHEN** a source tells the soul a claim in a domain where it holds prior `(α₀, β₀)`
- **THEN** the resulting belief starts at that `(α₀, β₀)` count instead of missing credence

#### Scenario: A lesson is confirmed repeatedly
- **WHEN** decisions that cited a lesson have good outcomes five times in a row
- **THEN** its expected credence is higher than at the start

#### Scenario: One failure after much confirmation
- **WHEN** a lesson with many confirmations fails once
- **THEN** its expected credence falls only slightly

#### Scenario: In the context but not cited
- **WHEN** an item was in the context of a decision and the deciding thought did not cite it
- **THEN** the outcome leaves its counts unchanged

#### Scenario: Evidence ages
- **WHEN** no new evidence arrives for a long time
- **THEN** the counts move toward the prior

### Requirement: Source reliability is domain-aligned and corruption-resistant
Source reliability `(α, β)` SHALL track per `(source, domain)` using external domains or
Hungarian-aligned sleep centroids (`A-12`). Uncorroborated single-stranger claims (`< k_corrob`),
untrusted taint (`R-43`), and discounted conventions (`R-32`) SHALL NOT bypass direct observation.

#### Scenario: A source is often wrong about one thing
- **WHEN** several things a source said in one domain are later contradicted by direct observations
- **THEN** new told items from that source in that domain start with lower credence while other domains stay unaffected

#### Scenario: Taint propagation and skeptical corroboration stop memory poisoning
- **WHEN** an untrusted actor asserts a novel rule repeated only once without `k_corrob` independent sources
- **THEN** the claim keeps untrusted `taint`, never upgrades to trusted basis, and never overrides direct observation or safety doors

#### Scenario: No prior is supplied
- **WHEN** a source appears with no prior supplied
- **THEN** its reliability starts from a uniform prior

### Requirement: Credence is recomputable from the log
Every change to an evidence count SHALL be an event, so that all credence and reliability
values can be rebuilt from the log.

#### Scenario: State is rebuilt
- **WHEN** credence state is discarded and rebuilt from the log
- **THEN** every count equals its value before

### Requirement: Inferences about minds are never facts
What the soul concludes about what a source wants or believes SHALL be stored as inferred, with
credence, and SHALL be tested by later predictions.

#### Scenario: A guess about a preference
- **WHEN** reasoning concludes that a source prefers something and a later prediction built on it fails
- **THEN** the belief's credence falls and it remains marked inferred
