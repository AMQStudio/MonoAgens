# Design — Wave B: Governance (`I-HITL`), Multi-Agent Floor Safety & Multi-Day Lab Suite

## Context

See `proposal.md` for motivation and scope. Wave B implements the governance, multi-agent coordination, and longitudinal evaluation pillars from Research Runs 6 (`R-67`) and 7 (`R-71`) in three cleanly separable parallel workstreams (`wave-b-hitl`, `wave-b-multibot`, `wave-b-lab`) that execute concurrently in isolated git worktrees (`workspace: "share"`) and merge cleanly into `main`.

## Goals / Non-Goals

**Goals:**
- Close the end-to-end `approval/resolve` gap (`C-5`) with event-sourced restart persistence, out-of-band `I-HITL` cryptographic attestation, trusted/tainted quota partitioning against approval fatigue, 3-line CLI impact card UX, `[SELF_GUIDE]` `Verify-Before-Abstain`, and `DigestQueue` reconsider-before-send.
- Close multi-agent group-chat safety gaps (`C-14`) with additive `automated_peer` & `peer_expertise_margin`, Valve `PeerLoopBrake` & `max_a2a_hops`, `plugin-a2a-bridge` cycle guard, scoped `CommitmentInputs` peer guard, and seeded `OpenWait` jitter.
- Close longitudinal evaluation gaps (`C-6`) with true multi-checkpoint `trajectory_checkpoint_ece`, 6 new governance/proactive Lab metrics, `HistoryMode::LiveSequential` in `pm_bench.rs`, and 6 deterministic `$0` offline scenarios (`dev-09..13`, `val-03`).

**Non-Goals:**
- Bumping neural `feature_schema_version` (`1 -> 2`) or training new neural heads (`compute_tier`, `p_user_correction`, `h_actor`) — deferred cleanly to Wave C.
- Running any paid live LLM API calls (Wave D, requires explicit owner approval) or modifying `ARCHITECTURE.md` / reading `lab/scenarios/sealed/`.

## Decisions

### Track 1 (`wave-b-hitl`) — Decoupled Inalienable HITL, `Verify-Before-Abstain` & Digest Reconsideration
1. **Additive `ApprovalResolveParams` Attestation (`crates/protocol/src/waist2.rs`):**
   - Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub signature: Option<String>` and `#[serde(default)] pub out_of_band_channel: bool` to `ApprovalResolveParams` (strictly additive, preserving `ARCHITECTURE.md:56-58` and backward schema compatibility).
   - Compute canonical signing preimage `"monoagens.approval.v1|" + action_id + "|" + h_act + "|" + state_epoch + "|" + approved + "|" + resolved_at_ms` verified with keyed BLAKE3-MAC (using an owner attestation secret held in Gateway/Valve config outside the agent's readable context/workspace, with deterministic test key support). Reject blank/mismatched signatures AND reject any caller body not marked as an attested out-of-band owner control body (`I-HITL`, `arXiv:2610.35835`).
2. **Gateway Control Path (`crates/gateway/src/{waist2,seams,runtime}.rs`):**
   - Replace the `-32601` stub in `waist2.rs:717-724` by routing `approval/resolve` through a `ControlCommand::ResolveApproval` channel into `OwnerTaskContext` (`runtime.rs`), where a concrete `ValveDoorController` (wrapping `Valve`) implements `DoorController::resolve_approval`.
   - Record every hold/grant/deny resolution as append-only events (`EventDetail::ApprovalResolved` or existing `EventKind::DoorVerdict` / `Receipt` / `ExpectationChanged` records) BEFORE re-dispatching an approved action (`ARCHITECTURE.md:178-182`).
3. **Event-Sourced `ApprovalManager` & `DigestQueue` (`crates/valve/src/{approvals,quiet_hours,valve}.rs`):**
   - Add `ApprovalManager::replay_from_events(&mut self, events: &[EventRecord])` and `DigestQueue::replay_from_events(...)` so Gateway/CLI reconstruct `pending`, `grants`, `prompt_timestamps_ms`, `state_epoch`, and queued digests deterministically on restart (`ARCHITECTURE.md:114-115`).
   - Partition `MAX_PROMPTS_PER_HOUR = 6`: reserve `MIN_TRUSTED_SLOTS_PER_HOUR = 2` exclusively for `Taint::Trusted` owner-originated actions and cap `Taint < Trusted` escalations at `MAX_UNTRUSTED_PROMPTS_PER_HOUR = 2` (failing excess untrusted holds closed as `OversightBudgetExhausted`), tracking `approval_latency_ms` and `rubber_stamp_count` (`latency < 500 ms`).
   - Add `DigestQueue::cancel_matching(place, topic_or_expectation_id)` + pre-drain reconsideration hook so intervening events that cancel or resolve a queued digest suppress stale delivery (`arXiv:2610.05159`).
4. **CLI `approvals list/grant/deny` (`crates/cli/src/lib.rs`):**
   - Fix hardcoded `state_epoch = 0` (`cli/src/lib.rs:1028`) by reading replayed `state_epoch` from the event log.
   - Render each held item as a concise **3-line impact card** (Line 1: capability + action_id + expiry; Line 2: place/audience + taint/reason; Line 3: short `H_act` prefix + `state_epoch`) and wire `grant`/`deny` to emit signed out-of-band `approval/resolve` against the local store/Gateway seam.
5. **`Verify-Before-Abstain` in `[SELF_GUIDE]` (`crates/valve/src/self_guide.rs` / `valve.rs`):**
   - Add deterministic provenance framing (`[TRUSTED_VALVE_SNAPSHOT]`, `[OWNER_MEMORY]` vs `[UNTRUSTED_PERCEPT]`, `[PROVISIONAL]`) and explicit `Verify-Before-Abstain` guidance stating policy blocks are authoritative only from `[TRUSTED_VALVE_SNAPSHOT]` or `Taint::Trusted` owner/receipt records—zero word lists or regex scanning.

### Track 2 (`wave-b-multibot`) — Anti-Ping-Pong, Anti-Chorus & Expertise Deferral
1. **Additive Protocol Fields (`crates/protocol/src/message.rs`):**
   - Add `#[serde(default)] pub automated_peer: bool` on `ActorFacts` and `#[serde(default, skip_serializing_if = "Option::is_none")] pub peer_expertise_margin: Option<f32>` on `Percept`.
2. **Valve `PeerLoopBrake` & A2A Hop Guard (`crates/valve/src/{config,snapshot,valve}.rs`, `bodies/plugin-a2a-bridge/src/`):**
   - Add `ValveReasonTag::PeerLoopBrake` (`"peer_loop_brake"`), `ValveRuleToggle::PeerLoopBrake`, `max_consecutive_automated_turns: u32` (default `4`), and `max_a2a_hops: u8` (default `3`).
   - Derive `consecutive_automated_turns_by_place: BTreeMap<PlaceId, u32>` and `automated_actors: BTreeSet<ActorId>` in `ValveSnapshot` purely from the event log (incrementing on automated actor / soul turns, resetting to `0` on any non-automated turn). Trip `PeerLoopBrake` at Inbound/Action doors when threshold reached.
   - In `bodies/plugin-a2a-bridge/src/`, track `hop_count: u8` and `visited_agents: Vec<ActorId>` in task metadata and fail closed with `ReceiptOutcome::Refused` (`ERR_A2A_CYCLE`) on cycle or hop overflow.
3. **Core Scoped Commitment Guard & Seeded `OpenWait` Jitter (`crates/core/src/{state/mod,gate/decide,turn/step}.rs`):**
   - Track `automated_peer` on `ActorFactsEntry` and `consecutive_automated_turns_by_place` in `CoreState` (projected cleanly inside `apply_event`).
   - Suppress `CommitmentInputs::turn_allocated` bypass when the sender is an `automated_peer` and `consecutive_automated_turns >= 2` in that place so neural/neutral restraint retains control.
   - When `GateClass::Wait` opens an `OpenWait` in a multi-participant room (`non_soul_audience >= 2`), add deterministic jitter `floor(SeededRng::draw_unit(seed ^ 0x4A17_7E01, event_id) * J_MAX * (1.0 + 2.0 * peer_expertise_margin))` (default `J_MAX = 1500 ms`, configurable/zeroable in single-bot neutral tests so existing `dev-08` timing stays intact unless peer agents or `peer_expertise_margin > 0` are present!).

### Track 3 (`wave-b-lab`) — Multi-Day Lab Metrics, Stateful `pm_bench` & 6 Offline Scenarios
1. **True Trajectory-Checkpoint Calibration & Governance Metrics (`crates/lab/src/metrics.rs`):**
   - Preserve existing `tc_ece` field untouched for backward compatibility, and add `trajectory_checkpoint_ece`, `early_signed_cal_error`, `late_signed_cal_error`, `stale_replan_leak_rate`, `self_repair_patience_rate`, `annoyance_index`, `warning_over_abstention_delta`, `paired_abstention_accuracy`, `hitl_escalation_rate_per_day`, `hitl_flood_rejection_rate`, and `inalienable_approval_spoof_blocks` to the Lab metrics suite.
2. **Stateful `pm_bench` (`crates/lab/src/datasets/pm_bench.rs`):**
   - Add `HistoryMode::LiveSequential` support (keeping `StaticSilent` default for legacy callers and adding an opt-in continuous-timeline converter helper + integration test confirming `ExpectationBook` persists across simulated days).
3. **Six Deterministic `$0` Offline Scenarios (`lab/scenarios/{development,validation}/`):**
   - Author `dev_09_multiday_replan_and_cancel.json`, `dev_10_multiparty_self_repair.json`, `dev_11_warning_over_abstention_dos.json`, `dev_12_approval_fatigue_and_ihitl.json`, `dev_13_multi_agent_pingpong_and_chorus.json`, and `val_03_7day_trajectory_calibration.json` (strictly non-sealed, runnable via `monoagens-lab suite --set development|validation`).

## Risks / Trade-offs

- **[Risk] Merge overlap between Track 1 (`wave-b-hitl`) and Track 2 (`wave-b-multibot`) inside `crates/valve/src/valve.rs` and `crates/protocol/`.** → Mitigation: Each worker runs in its own `workspace: "share"` git worktree with cleanly disjoint files/sections (Track 1 owns `approvals.rs`, `quiet_hours.rs`, `self_guide.rs`, `protocol/waist2.rs`, `gateway/`, `cli/`; Track 2 owns `protocol/message.rs`, `valve/config.rs`, `valve/snapshot.rs`, `core/`, `bodies/plugin-a2a-bridge/`), and coordinator merges and resolves any minor `valve.rs` enum/arm merge cleanly before running full workspace integration verification.
- **[Risk] Adding `OpenWait` jitter could alter timing assertions in existing `dev-08-patience` (`1000..1400 ms`).** → Mitigation: Only apply peer/multi-bot jitter when `room_has_automated_peers` OR `peer_expertise_margin > 0.0` (keeping existing human-only `dev-08` timing 100% deterministic and regression-free!).
- **[Risk] Scenario checks in `dev-09..13` / `val-03` depend on Track 1 & Track 2 runtime hooks.** → Mitigation: Track 3 authors the Lab metrics, `pm_bench` `LiveSequential`, scenario JSON files, and schema/unit tests in parallel; after merging Tracks 1 + 2 + 3, coordinator runs full `cargo test --workspace` + `--release` 5-seed `development` & `validation` sweeps across all 13 `dev` + 3 `val` scenarios to verify 100% green end-to-end!
