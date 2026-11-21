# Proposal

## Why

With Wave A (`2026-11-21-wave-a-chat-foundation-fixes`) archived into `openspec/specs/`, the next highest-leverage chat-agent gaps verified in Research Runs 6 (`R-67`) and 7 (`R-71`) are governance, multi-bot safety, and long-horizon evaluation: `approval/resolve` still returns `-32601` in Gateway while `ApprovalManager` is purely in-memory and lacks out-of-band `I-HITL` cryptographic verification (`C-5`); multi-bot group chats lack flat-room ping-pong brakes and `OpenWait` chorus jitter (`C-14`); `[SELF_GUIDE]` lacks `Verify-Before-Abstain` defense against warning-induced over-abstention (`arXiv:2611.14864`) and `DigestQueue` lacks replannable reconsideration (`arXiv:2610.05159`); and `crates/lab` lacks true multi-checkpoint `trajectory_checkpoint_ece`, stateful 7-day `pm_bench` execution (`C-6`), and multi-day/adversarial offline scenarios (`dev-09..13`, `val-03`).

## What Changes

- **Track 1 — Decoupled Inalienable HITL (`approval/resolve`), Anti-Approval-Fatigue & `Verify-Before-Abstain` (`C-5`, `R-67`):**
  - Wire `approval/resolve` end-to-end across `crates/protocol/src/waist2.rs` (additive `#[serde(default)]` `signature` / attestation metadata + `channel_out_of_band` verification), `crates/gateway/src/{waist2,seams,runtime}.rs` (`ControlCommand::ResolveApproval` into `OwnerTaskContext`), `crates/valve/src/{approvals,valve,quiet_hours}.rs`, and `crates/cli/src/lib.rs` (`monoagens approvals grant/deny` + structured 3-line impact card + real `state_epoch` projection).
  - Event-source `ApprovalManager` and `DigestQueue` state so pending approvals, grants, denials, and digests survive restarts deterministically (`ARCHITECTURE.md:114-115`).
  - Enforce out-of-band **`I-HITL`** attestation (rejecting `approval/resolve` from untrusted/shared chat bodies and replacing `!signature.trim().is_empty()` with deterministic HMAC-BLAKE3 / Ed25519 verification over canonical `(action_id || h_act || state_epoch || approved || resolved_at_ms)`).
  - Partition the `6/h` approval cap against flooding & approval fatigue (reserving `>= 2/6` slots for `Taint::Trusted` owner chains, capping untrusted escalations separately at `<= 2/h`, recording `approval_rubber_stamp_rate`).
  - Add structural provenance framing (`[TRUSTED_VALVE_SNAPSHOT]` vs `[UNTRUSTED_PERCEPT]`) + **Verify-Before-Abstain** rule in Valve's `[SELF_GUIDE]` Context Door (`arXiv:2611.14864`, zero word lists).
  - Add a **reconsider-before-send** cancellation/supersession check on `DigestQueue` (`arXiv:2610.05159`) so intervening turns that cancel or resolve a queued topic drop stale digests before dispatch.
- **Track 2 — Multi-Agent Group Chat Coordination: Anti-Ping-Pong, Anti-Chorus & Expertise Deferral (`C-14`, `R-71`):**
  - Add strictly additive `#[serde(default)]` protocol fields: `ActorFacts::automated_peer: bool` (translated at edge adapters, never NL-parsed in Core) and `Percept::peer_expertise_margin: Option<f32>`.
  - Add deterministic Valve **`PeerLoopBrake`** (`max_consecutive_automated_turns = 4` per `place`, resetting to `0` on any non-automated turn) and **`max_a2a_hops = 3`**, plus `hop_count` / `visited_agents` cycle protection in `bodies/plugin-a2a-bridge/`.
  - Guard `CommitmentInputs::turn_allocated` in `crates/core` so an automated peer repeatedly addressing Soul after `consecutive_automated_turns >= 2` cannot bypass neural restraint, and add deterministic `SeededRng` per-soul jitter scaled by `(1 + 2 * peer_expertise_margin)` to `OpenWait.due_ms` to eliminate homogeneous bot chorus collisions.
- **Track 3 — Multi-Day Chat Evaluation Metrics, Stateful `pm_bench` & 6 Offline Scenarios (`C-6`, `R-67`, `R-71`):**
  - Upgrade `crates/lab/src/metrics.rs` with true multi-checkpoint `trajectory_checkpoint_ece` (binned across normalized horizon checkpoints `0.2..1.0` with early vs late signed calibration error, keeping existing `tc_ece` alias documented), `stale_replan_leak_rate`, `self_repair_patience_rate`, `annoyance_index`, `warning_over_abstention_delta`, `paired_abstention_accuracy`, and HITL fatigue/spoof counters.
  - Add `HistoryMode::LiveSequential` option in `crates/lab/src/datasets/pm_bench.rs` so 7-day Virtual Week timelines preserve `ExpectationBook` and `h_t` across turns instead of resetting between checkpoints.
  - Author and verify 6 new `$0` offline scenarios (zero access to `lab/scenarios/sealed/`): `dev-09-multiday-replan-and-cancel`, `dev-10-multiparty-self-repair`, `dev-11-warning-over-abstention-dos`, `dev-12-approval-fatigue-and-ihitl`, `dev-13-multi-agent-pingpong-and-chorus`, and `val-03-7day-trajectory-calibration`.

## Capabilities

### New Capabilities
- `hitl-governance`: Decoupled out-of-band `approval/resolve` (`I-HITL`), event-sourced approval & digest persistence, anti-approval-fatigue trusted/tainted quota partitioning, `[SELF_GUIDE]` `Verify-Before-Abstain` provenance separation, and replannable digest reconsideration.
- `multi-agent-coordination`: Deterministic multi-bot floor safety (`PeerLoopBrake`, `max_a2a_hops` cycle guard, edge-attested `automated_peer`, scoped `CommitmentInputs` guard, and seeded expertise-scaled `OpenWait` jitter).
- `multiday-chat-lab`: Multi-checkpoint trajectory calibration (`trajectory_checkpoint_ece`), proactive annoyance/replan/abstention/HITL Lab metrics, `HistoryMode::LiveSequential` for `pm_bench`, and 6 multi-day/adversarial offline evaluation scenarios (`dev-09..13`, `val-03`).

### Modified Capabilities

## Impact

- **Crates Modified:** `crates/protocol` (strictly additive `#[serde(default)]` fields), `crates/valve`, `crates/gateway`, `crates/cli`, `crates/core` (`+0` net primitives), `bodies/plugin-a2a-bridge`, `crates/lab`, and `lab/scenarios/{development,validation}/`.
- **Invariants Preserved:** Zero `ARCHITECTURE.md` edits, zero word lists or simulated emotion inside Core/Valve, mandatory `None` neutral fallback, `Delta_NI = 0`, deterministic replay, zero access to `lab/scenarios/sealed/`, and **`0` paid live LLM API calls**.
