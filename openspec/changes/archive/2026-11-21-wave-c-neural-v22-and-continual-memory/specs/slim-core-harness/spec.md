## MODIFIED Requirements

### Requirement: Model-agnostic numeric feature extraction
For every appraised percept, the Core harness SHALL construct a bounded, normalized numeric feature vector (`feature_schema_version = 2`: `32` scalars in `[0, 1]` = `18d` v1 core + `6d` repair + `4d` thread/actor + `4d` peer-agent, with `schema = 1` models slicing `0..18`, plus optional `10` modality channels) solely from opaque IDs, timestamps, counts, budget ratios, and post-`VisibleScope` metadata without inspecting raw text or word lists.

#### Scenario: Missing embeddings degrade gracefully
- **WHEN** an incoming percept carries no embedding vector
- **THEN** vector-dependent similarity features (`max_visible_mem_cos`, repair cosines, `max_active_thread_cos`) default to `0.0` and the harness still completes appraisal without error

#### Scenario: Non-finite sensor values are sanitized
- **WHEN** a modality channel or computed ratio yields `NaN` or `Inf`
- **THEN** the feature extractor substitutes the documented neutral finite value before invoking the neural controller

#### Scenario: Schema v2 repair, thread/actor, and peer-agent channels preserve `Delta_NI = 0`
- **WHEN** `extract_features_v2_from_state` computes the `32d` vector for a group percept while unrelated private DM memories, episodes, or soul replies exist in `CoreState`
- **THEN** all `32` channels match a state without those private items bit-for-bit, and indices `18..32` accurately reflect visible repair, active thread, actor reliability, and automated-peer state

## ADDED Requirements

### Requirement: Streaming 3W multi-topic disentanglement in EpisodeBook and place tail
`EpisodeBook` SHALL maintain up to `M = 6` active thread slots per `(PlaceId, CanonicalAudienceHash)` scope scored deterministically by `w_sem * cos(e_256, centroid) + w_who * actor_overlap + w_when * exp(-dt / tau)` with `reply_to`/`caused_by` fast-path and `EventId` tie-breaking, record `thread_id` on `PlaceHistoryEntry` and `MemoryEntry`, prioritize trigger-thread siblings in `[PLACE_TAIL]`, and update `actor_exchanges` across multi-party rooms (`audience.len() > 2`).

#### Scenario: Interleaved topics in one group room stay separated into distinct threads
- **WHEN** two interleaved discussions with orthogonal embeddings or disjoint participants occur concurrently in the same multi-party room
- **THEN** `EpisodeBook` assigns them to distinct active thread slots within `M <= 6`, stamps each entry's `thread_id`, and `[PLACE_TAIL]` prioritizes messages sharing the trigger's `thread_id`

#### Scenario: Group chat updates actor exchange statistics beyond dyadic audiences
- **WHEN** a percept arrives in a room whose audience has more than `2` non-soul participants
- **THEN** Core updates `actor_exchanges` for the sender so group familiarity and source-card closeness remain accurate

#### Scenario: Multi-slot episode state respects audience isolation and partial hard-forget
- **WHEN** two audiences share a `PlaceId` or `EventKind::HardForget` erases events from one thread slot
- **THEN** thread slots never merge across distinct `CanonicalAudienceHash` keys and erased threads/centroids are scrubbed completely

### Requirement: Continual chat memory recall modes, fade guards, three-timescale preference drift, and SkillForge UCB procedures
The Core harness SHALL support `RecallTimeMode { CurrentOnly, IncludeSuperseded }`, separate `recalled_event_ids` from `cited_event_ids`, enforce `max_fade_fraction` and durable preference anchor protection in `fade.rs` with audience-locked tier-2 rollups, support three-timescale `DomainId`-scoped `BetaCredence` drift preserving immutable `OwnerSigned` invariants, and rank `SkillBook` procedures via Bayesian UCB with a 4-state `SkillForge` lifecycle (`Trial -> Active -> Stable -> Retired`).

#### Scenario: RecallTimeMode controls visibility of superseded memories
- **WHEN** a memory entry has `superseded_by = Some(...)` and `run_recall` executes with `RecallTimeMode::CurrentOnly` versus `RecallTimeMode::IncludeSuperseded`
- **THEN** `CurrentOnly` excludes superseded entries while `IncludeSuperseded` returns them with `is_current = false` strictly post-`VisibleScope`

#### Scenario: Fade guard caps per-cycle pruning and protects durable preference anchors
- **WHEN** `select_fade_candidates` runs over a large set of low-activation memories including durable preference anchors
- **THEN** durable preference anchors are excluded from fading and the number of candidates released in one cycle never exceeds `max_fade_fraction` of eligible memories

#### Scenario: Three-timescale preference drift isolates domains and protects OwnerSigned invariants
- **WHEN** fast (`lambda_fast = 0.80/day`) chat-domain preference evidence arrives alongside slow (`0.98/day`) tool-domain beliefs and `OwnerSigned` `IdentityAnchor`/`principles`
- **THEN** chat-domain drift remains strictly scoped to its `DomainId` without altering tool-domain credences, and `OwnerSigned` `IdentityAnchor` and `principles` remain immutable (`lambda = 1.0`)

#### Scenario: Single-room multi-session chat procedures enter Trial and rank via Bayesian UCB
- **WHEN** a repeating procedure succeeds across `>= 2` distinct episodes within a single room (or across rooms)
- **THEN** `SkillBook` admits it in `ProcedureLifecycleState::Trial`, promotes it through `Active -> Stable` on corroborating receipts (or retires it on CUSUM/schema drift), and `select_tools_for_context` ranks eligible procedures deterministically by Bayesian UCB
