# Spec Delta

## Purpose

Reasoning is the language model, woken only when the gate says so. This capability defines
what the model is shown (a context rebuilt from the log for each wake, not a growing
transcript) and the few acts through which it can affect anything.

## ADDED Requirements

### Requirement: The context is rebuilt for every wake
For each wake, the context SHALL be assembled from the log and current state: identity,
available capabilities, the cards of the sources present and of the soul itself, its
commitments and active goal, the working set of the current chain, recalled memories, the
recent events of the place, the current time and the triggering event.
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
chain SHALL include the soul's own earlier thoughts and actions of the chain in full, the most
recent observations in full, and older observations as references that can be recalled.

#### Scenario: A second step sees the first
- **WHEN** an action's receipt wakes reasoning to continue a task
- **THEN** the context contains the action and its receipt from the earlier step

#### Scenario: A long task
- **WHEN** a chain has more observations than the limit for full display
- **THEN** the oldest appear as references, and the soul's own thoughts and actions still appear in full

### Requirement: Context order puts IdentityAnchor at slot 0 and keeps stable-to-volatile prefix
`ContextPlan` SHALL place the immutable `OwnerSigned` `IdentityAnchor` (`[ANCHOR]`, `R-48`) in
slot 0, sorted static core capabilities + schema hashes (and `[SELF_GUIDE]` when enabled), ledgers +
4-pillar `self_card`, commitments, working set, recalled items, place tail, `<now>`, cutoff, and
Spotlighting trigger (`R-11`, `R-43`).

#### Scenario: The place tail shows conversation only
- **WHEN** a wake renders the place tail
- **THEN** it holds only percepts and the soul's actions at the place, never Core bookkeeping (decisions, withholds, expectation records, model calls), so gate internals and embedding vectors never reach the prompt

#### Scenario: Two consecutive wakes
- **WHEN** two wakes happen with the same identity anchor, runtime self-guide snapshot, and capabilities
- **THEN** rendered contexts share an identical static cacheable prefix covering slot-0 anchor, `[SELF_GUIDE]` (when active), and sorted capabilities

### Requirement: Derived self-guide renders active mechanics without drift or threshold leakage
Controlled by `self_mechanics_guide` (neutral `false`), `Core` SHALL emit a content-blind
`CoreSelfSnapshot` while `Doors::context` accepts an optional `ValveSelfSnapshot`. `reason` SHALL
synthesize `[SELF_GUIDE]` (`<= 250` tokens) in the cacheable prefix via exhaustive `match`
templates—never handwritten Core prose or hidden formula thresholds—plus one-line working-set
cues when `auto_recall` or `held`/`refused` receipts occur.

#### Scenario: Neutral switch preserves baseline prompt byte-for-byte
- **WHEN** `self_mechanics_guide` is set to its neutral value `false`
- **THEN** rendered prompts omit `[SELF_GUIDE]` and match baseline contexts byte-for-byte

#### Scenario: Disabled mechanism or Valve rule leaves zero stale prose
- **WHEN** a mechanism switch is neutralized, `Valve` runs in `off` mode, or `quiet_hours` is disabled in config
- **THEN** the next wake's `[SELF_GUIDE]` immediately omits that disabled capability or rule without manual text edits

#### Scenario: Held receipt and auto-recall surface honest just-in-time working-set cues
- **WHEN** `Valve` returns a `held` receipt (`PendingApproval` or `quiet_hours_digest`) or `P_rc` triggers `memory.recall`
- **THEN** the working set renders one explicit factual cue naming the hold reason or recall trigger without revealing tunable formula constants

### Requirement: Progressive capability and procedure disclosure stays within prompt budgets
`ContextPlan` SHALL keep at most four static `OwnerSigned` core capabilities in the cacheable
prefix and select at most `k_cap` (`default = 8`) additional capabilities plus `k_proc` (`default =
3`) active procedures using hybrid BM25 and `256d` vector ranking filtered strictly by `VisibleScope ∩
ActiveCaps` and matching `capability_schema_hash` (`R-38`). Setting `k_cap = |C_avail|` and `k_proc =
0` SHALL act as the exact neutral ablation value.

#### Scenario: Many MCP tools do not bloat prompts and stale procedures stay excluded
- **WHEN** `60` capabilities are registered across connected bodies and one consolidated procedure carries a mismatched `capability_schema_hash`
- **THEN** `ContextPlan` preserves the static core prefix, admits at most `k_cap` relevant dynamic capabilities, and excludes the schema-drifted procedure

### Requirement: The current time is always present
Every context SHALL state the current time from the injected clock, and stored times SHALL be
absolute so that relative expressions are resolved against the present only when shown.

#### Scenario: An old memory mentions a date
- **WHEN** a memory recorded weeks ago is placed in a context
- **THEN** it is shown with its absolute time and the context states the current time

#### Scenario: The model's knowledge ends before the present
- **WHEN** a context is built for a model with a configured knowledge cutoff
- **THEN** the context states that cutoff together with the current time

### Requirement: Untrusted content uses Spotlighting and provenance checks
Non-soul payloads SHALL render inside `[UNTRUSTED_QUOTED]` Spotlighting delimiters distinct from
`[SYSTEM]`/`[ANCHOR]`/`[TRUSTED]` (`R-43`); `CitationLock` (`A-10`) SHALL reject fabricated
`used_ids[]`, and pre-act `[ANCHOR_CHECK]`, `PrincipleAudit` (`R-48`), and `CommitCheck`
(`R-49`) SHALL guard external dispatches.

#### Scenario: A message contains an instruction
- **WHEN** a percept's payload contains text that reads like an instruction to the model
- **THEN** it appears solely inside `[UNTRUSTED_QUOTED]` delimiters and taint propagation blocks privileged state mutation

#### Scenario: Model cites non-existent evidence ids
- **WHEN** reasoning returns `used_ids[]` referencing ids absent from the snapshot's `VisibleScope`
- **THEN** `CitationLock` rejects the fabricated reference cleanly before belief/memory writes

### Requirement: Reasoning acts through typed five-act and consolidate contracts
`reason` SHALL emit strict nullable acts (`think`, `act`, `expect`, `resolve`, `revise`, `A-10`);
`consolidate` SHALL additionally emit typed `gist`, `lesson`, and `procedure` acts (`A-01`,
`A-11`). `revise` SHALL NOT alter recorded events, `OwnerSigned` identity, credences, or knobs.

#### Scenario: The model speaks with commitment check
- **WHEN** the model uses `act` with a speaking capability making a promise
- **THEN** `CommitCheck` verifies no hard conflict with active `PromiseMeta` obligations before dispatch

#### Scenario: The model returns plain text only
- **WHEN** the model's output contains no thought act
- **THEN** nothing is sent to any body, and the output is recorded as a thought

#### Scenario: The model names an unknown capability
- **WHEN** an act names a capability that was not in the context
- **THEN** the act ends in a receipt with outcome refused and the reason is recorded

#### Scenario: revise aims at a recorded event or immutable identity
- **WHEN** a `revise` names a recorded event, `OwnerSigned` identity/anchor, evidence count, or disposition
- **THEN** nothing changes and the refusal is recorded

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

### Requirement: Unallocated turns require balanced 1-5 motivation and 0-20 calibrated confidence
On an unallocated turn, reasoning SHALL emit balanced `1..5` motivation and verbally anchored
integer confidence `0..20` (`A-10`, converted to `c_raw = k/20` and domain-calibrated).
Communicative acts below `speaking_bar` or `certainty_bar` SHALL be recorded as `withheld`.

#### Scenario: Low motivation
- **WHEN** reasoning on an unallocated turn states a motivation below the speaking bar and attempts to speak
- **THEN** nothing is sent and the act is recorded as `withheld` with both numbers

#### Scenario: An allocated turn
- **WHEN** the turn was allocated to the soul
- **THEN** the bars do not withhold its reply

### Requirement: Report control guards 0/0, specifies auto-recall, and withholds safely
Report control SHALL compute `P_rc = clamp(cost / (value + cost), certainty_bar, 0.99)` (defaulting
`0/0` to `certainty_bar`, `A-01`). Below `P_rc`, core SHALL run explicit `memory.recall` with
synthesized claim-entity query up to `n_recall` times (`A-01`), then request coarser grain
(chat-only) or record `withheld`.

#### Scenario: High stakes and missing zero guard
- **WHEN** reasoning gives cost `3` and value `1`, or omits/zeroes both `(value, cost)`
- **THEN** `P_rc` computes `0.75` (bounded by `certainty_bar`) or safely falls back to `certainty_bar` with no NaN

#### Scenario: Not sure enough triggers explicit recall query
- **WHEN** calibrated confidence sits below `P_rc`
- **THEN** `memory.recall` fires automatically using a structured query synthesized from the claim/trigger before waking reasoning again

#### Scenario: A coarser answer clears the criterion in chat
- **WHEN** after allowed recalls in a chat body reasoning gives a coarser answer whose calibrated confidence clears `P_rc`
- **THEN** the coarser answer is sent

#### Scenario: Still unsure or embodied action
- **WHEN** the coarser answer stays below `P_rc` or the body is embodied
- **THEN** the act is recorded as `withheld` with raw `[0,20]` and calibrated scores without guessing

### Requirement: Each remembered item shows how the soul knows it
Every remembered item in a context SHALL be shown with its basis (observed, told, inferred or
self-caused), whether it is recollected or only familiar, whether it is verbatim or gist, its
credence when it has one, and its validity.

#### Scenario: Something a source claimed
- **WHEN** a recalled item is something a source told the soul
- **THEN** the context shows it as told, by that source, with its credence

### Requirement: Model calls state their purpose
Every model call SHALL be made for one of the purposes reason, classify or consolidate, and
SHALL be recorded with that purpose.

#### Scenario: Cost by purpose
- **WHEN** the log of a run is read
- **THEN** tokens can be totalled separately for reasoning, classification and consolidation

### Requirement: Every rendered model request contains its trigger
For every purpose (reason, classify, consolidate) the rendered request SHALL contain the content
that triggered it and the capabilities registered for its place. Tests of this rule SHALL read
the rendered request text, not only the emitted effect.

#### Scenario: A contract test over all purposes
- **WHEN** the contract test drives a percept that wakes reasoning, one that triggers a classification and one that triggers consolidation
- **THEN** each rendered request contains its trigger content and the registered capabilities

### Requirement: Output-format failures are diagnosed before any retry is added
Model outputs that fail to parse SHALL be recorded per failure type (field and reason) with
whether a strict output schema was sent. Defects in the prompt or schema SHALL be fixed. A retry
that sends the parse error back to the model SHALL NOT be added without the owner's decision.

#### Scenario: Reading the failure types of a run
- **WHEN** the failures of a live run are listed
- **THEN** each failure shows its purpose, the failing field, the reason, and whether a strict schema was requested
