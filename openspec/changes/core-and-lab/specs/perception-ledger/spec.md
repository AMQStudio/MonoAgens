# Spec Delta

## Purpose

The perception ledger is the soul's bookkeeping of minds: for each source, the plain facts of
what it witnessed and was told, and for the soul itself, what it said, promised and can do.
Reasoning infers beliefs and wants from these facts instead of tracking them implicitly.

## ADDED Requirements

### Requirement: The ledger holds facts only
For each source the ledger SHALL hold only facts derivable from the log: which events it
witnessed, what the soul told it and whether that was grounded, its open requests to the soul,
and what changed after it last witnessed. It SHALL NOT hold conclusions about what the source
believes or wants.

#### Scenario: Building a card
- **WHEN** a source's card is built
- **THEN** every entry on it points to recorded events

### Requirement: Witnessing follows audiences
A source SHALL count as having witnessed an event exactly when it was in that event's recorded
audience. This SHALL hold for any body: present in a chat, within sensor range, within range in
a world.

#### Scenario: Someone left before it was said
- **WHEN** a source was not in the audience of an event in a place it is usually in
- **THEN** that event is not on the source's card as witnessed

### Requirement: Changes a source did not witness are flagged
When a fact a source witnessed is later superseded by an event that source did not witness, the
ledger SHALL flag it on that source's card as a candidate false belief.

#### Scenario: The object was moved while they were away
- **WHEN** a source witnessed where something was, and it was moved in an event the source was not in the audience of
- **THEN** the source's card flags that the source last witnessed the old location

#### Scenario: They saw the change
- **WHEN** the source was in the audience of the change
- **THEN** nothing is flagged

### Requirement: What the soul told a source is tracked with its grounding
For each communicative act toward a source, the ledger SHALL record what was told and whether
its prediction was answered, expired or missed.

#### Scenario: A message that did not land
- **WHEN** the soul told a source something and the prediction for it ended as a content miss
- **THEN** the card shows that item as told and not grounded

### Requirement: Cards of those present and promisees enter the context
When reasoning wakes, bounded cards for sources in the triggering event's audience **plus** any
active expectation's promisees/targets (`A-01` tick fix) SHALL enter the context.

#### Scenario: A scheduled reminder wakes via internal tick
- **WHEN** a timer tick fires an overdue promise whose trigger carries no external chat audience
- **THEN** the promisee's ledger card (including open promises and candidate false-beliefs) is still included in the wake context

#### Scenario: A question from someone with a stale view
- **WHEN** a source with a flagged candidate false belief addresses the soul
- **THEN** the context of that wake contains the flag

### Requirement: The soul keeps a four-pillar event-derived self card
The soul's `self_card` SHALL expose four event-derived pillars (`R-35`): (1) per-capability
empirical `(α_c, β_c)` & latency, (2) `[0, 20]` Brier/ECE calibration & `meta-d'` gap, (3)
budget/queue headroom, and (4) open claim contradictions, driving deterministic door
recalibration without trusting runtime LLM introspection.

#### Scenario: Asked what it promised or capability changes
- **WHEN** a source asks what was promised or a body capability changes reliability or drops offline
- **THEN** `self_card` reflects current open promises, removed/updated capabilities, and empirical calibration metrics read-only

#### Scenario: Uncalibrated overconfidence meets deterministic recalibration
- **WHEN** historical calibration on `self_card` shows systematic overconfidence in a domain at an external door
- **THEN** confidence passed to gate/report control is recalibrated deterministically from logged Brier/ECE curves

### Requirement: The ledger is a projection
The ledger SHALL be rebuildable from the log alone.

#### Scenario: Rebuild
- **WHEN** the ledger is discarded and rebuilt from the log
- **THEN** every card equals its state before
