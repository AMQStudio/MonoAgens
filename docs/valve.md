# Policy Valve (`crates/valve/`)

> Normative architecture: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (Absolute Lines 1 & 2, Three Doors, First-and-Last Rule). Implementation: [`crates/valve/`](../crates/valve/) (`18/18` integration tests passing in `tests/valve_v1.rs` and `tests/valve_v2.rs`).

`Valve` wraps `Core`'s decision loop deterministically over a single `ValveSnapshot` `(log_seq, t_now)` so that no external body, secondary hook, or untrusted prompt can bypass security boundaries.

---

## 1. Absolute Lines & First-and-Last Door Ordering

### Absolute Lines
- **Absolute Line 1 (`SelfModify` / `identity_changing` owner-only)**: Only `Actor::"owner"` may trigger `Action::"SelfModify"` or capabilities with `identity_changing: true` ([`crates/valve/policies/default.cedar`](../crates/valve/policies/default.cedar) lines 2, 26–32, 71–80; classified `RiskTier::R3` with tag `ValveReasonTag::NonOwnerIdentityChange` for non-owners). Verified in `valve_v2::proptest_h_act_place_order_invariant_and_r3_non_owner_identity` and `valve_v1::symcc_proves_unsat_and_aborts_on_hostile_bundle_or_missing_solver`.
- **Absolute Line 2 (`VisibleScope` Isolation, `R-33`)**: No event outside the current `ContextPlan` `VisibleScope` reaches the prompt or cross-audience action without a valid logged `DeclassToken` matching `(event_id, context_audience, target_place, now <= expiry)`. Items outside scope are stripped (`DoorDecision::Strip`, `ValveReasonTag::Line2ScopeStripped`). Verified in `valve_v1::line2_enforces_visible_scope_and_validates_logged_owner_declass_token`.

### Four Doors & First-and-Last Rule
Implemented across [`crates/valve/src/valve.rs`](../crates/valve/src/valve.rs) and [`crates/valve/src/hooks.rs`](../crates/valve/src/hooks.rs):
1. `inbound` (`InboundPercept`): checks declared audience (`UnknownAudience`) and attested/known actor presence.
2. `context` (`ContextAssemble`): enforces `VisibleScope` and evaluates minimum turn taint `tau_turn`.
3. `action` (`InvokeCapability` / `SelfModify`): evaluates Cedar ABAC, Line 1/Line 2, budgets/chain brake, quiet hours/proactive caps, taint quarantine, and `R_0..R_3` `PendingApproval`.
4. `memory_write` (`WriteMemory`): blocks tainted or unsigned-procedure writes (`TaintedProcedureWrite`), allowing owner-signed procedures and trusted non-procedure writes.

**First-and-last ordering**: On every door, `Valve` runs its opening pass, invokes any registered `SecondaryDoorHook`s, and executes its closing pass ([`hooks.rs`](../crates/valve/src/hooks.rs)). Hooks may **only narrow** access (`Allow -> Strip -> Hold -> Refuse`); any widening attempt is overridden, replaced with the stricter decision, and tagged `ValveReasonTag::HookWideningOverridden` (`valve_v1::first_and_last_ordering_overrides_widening_and_logs_across_all_doors`).

---

## 2. Cedar Policy Engine & `cedar-policy-symcc` Proof

- **Schema & Default Policy**: [`crates/valve/policies/default.cedarschema`](../crates/valve/policies/default.cedarschema) declares entities `Role`, `Actor in [Role]`, `Place`, `Capability`, `MemoryItem` and actions `InboundPercept`, `ContextAssemble`, `InvokeCapability`, `SelfModify`, `WriteMemory`. Default policy bundle: [`crates/valve/policies/default.cedar`](../crates/valve/policies/default.cedar).
- **Static SMT Verification (`R-48`)**: `Valve` validates policy bundles on load with `cedar-policy-symcc` using `LocalSolver::cvc5()` (host solver verified at `/usr/bin/cvc5`). Any bundle that fails UNSAT verification for Absolute Line 1/Line 2 properties or lacks the `cvc5` solver aborts startup instead of falling back (`valve_v1::symcc_proves_unsat_and_aborts_on_hostile_bundle_or_missing_solver`).
- **Presence & Role Attestation**: `room_present` and `physically_present` roles are derived exclusively from attested hardware/place records with TTL expiry (`room_presence_ttl_s`, `physical_presence_ttl_s`, `physical_room_radius_m` in [`crates/valve/src/config.rs`](../crates/valve/src/config.rs)), never from message text (`valve_v1::admin_text_spoof_denied_and_presence_ttl_enforced`).

---

## 3. Risk Tiers (`R_0..R_3`), `PendingApproval`, & `H_act` Binding

Defined in [`crates/valve/src/approvals.rs`](../crates/valve/src/approvals.rs) (`R-42`):

| Tier | Trigger Conditions (from flags/snapshot, never model text) | Valve Disposition |
|---|---|---|
| `R0` | Read-only or communicative non-irreversible capability | Permitted (subject to budgets/quiet hours) |
| `R1` | Reversible, non-identity-changing, budgeted capability | Permitted |
| `R2` | `identity_changing` (owner), `irreversible`, financial/credential, `procedure` write, `DeclassToken` grant, or non-`Trusted` non-communicative action | Held as `PendingApproval` (`MAX_RETRY_ON_HELD = 0`) |
| `R3` | Non-owner `identity_changing`, unauthorized cross-audience act, or bad/stale `H_act` | Hard `Refuse` |

### `H_act` & TOCTOU / Loopjacking Defense
- **Canonical Action Hash `H_act`**: Built over canonicalized JSON payload (`canonical_json_bytes` sorts map keys recursively) plus `(capability_id, sorted_dedup_places, audience, body_id)` — independent of `places` ordering (`valve_v2::proptest_h_act_place_order_invariant_and_r3_non_owner_identity`).
- **TTL, `state_epoch`, & Sliding Caps**:
  - `APPROVAL_TTL_CHAT_S = 900` (`15 min`) for chat/standard places.
  - `APPROVAL_TTL_EMBODIED_S = 30` (`30 s`) for embodied actuators.
  - Approval consumption requires matching `action_id`, matching `h_act`, matching `state_epoch`, and `now <= expires_at_ms`; any mutation, epoch drift, replay, or expiry returns `ValveReasonTag::StaleOrMismatchedApproval` (`valve_v2::loopjacking_and_toctou_mutations_epoch_drift_and_ttl_expiry_refused`).
  - Oversight caps: `MAX_PROMPTS_PER_HOUR = 6`, `MAX_PENDING_QUEUE = 3`, `DEDUP_COOLDOWN_S = 600` (`ValveReasonTag::OversightBudgetExhausted`, `valve_v2::filler_burst_hits_max_pending_queue_and_max_prompts_per_hour`).

---

## 4. Three-Door Taint Quarantine (`R-43`), Budgets, & Quiet Hours (`R-46`)

- **3-Door Taint Quarantine ([`crates/valve/src/taint.rs`](../crates/valve/src/taint.rs))**:
  - Turn taint `tau_turn = min(snapshot_taint, Segments in ContextPlan)` via `Taint::join_min`.
  - Under `tau_turn < Trusted`: `R0` replies remain allowed; `>= R2`, `procedure` writes, or owner `identity_changing` require explicit `ApprovalGrant(H_act)`; cross-audience acts without `DeclassToken` are refused (`valve_v2::three_door_taint_quarantine_prompt_injection_torture_suite`).
- **Token Budget & Chain Brake**:
  - Enforces `daily_token_budget` (`100_000` default) and `max_chain_wakes` (`8` default), blocking excess monitors/wakes (`BudgetExhausted`, `ChainBrake`) while exempting single-shot promises (`valve_v1::budget_exhaustion_blocks_monitors_exempts_single_shot_promises_and_chain_brake_fires`).
- **DST-Safe Quiet Hours & Proactive Digests ([`crates/valve/src/quiet_hours.rs`](../crates/valve/src/quiet_hours.rs))**:
  - Uses `jiff::tz::TimeZone` (IANA timezones, no fixed offsets) for local wall-clock evaluation across `[quiet_start_hour, quiet_end_hour)` (`22:00..07:00` default).
  - Non-urgent proactive notifications during quiet hours batch into `3x/day` digest hours `DIGEST_HOURS_LOCAL = [8, 13, 19]` (`ValveReasonTag::QuietHoursDigest`), capped at `PROACTIVE_DAILY_CAP = 8` (`DailyCapReached`).
  - Urgency `>= EMERGENCY_URGENCY_CUTOFF (0.90)` bypasses both quiet hours and the daily cap (`IMMEDIATE_URGENCY_CUTOFF = 0.60` for daytime immediate delivery). Verified in `valve_v2::quiet_hours_dst_transitions_c_daily_digests_and_emergency_override`.

---

## 5. Canonical Reason Tags, `[SELF_GUIDE]`, and Configuration Toggles

### Exhaustive `ValveReasonTag` Table ([`crates/valve/src/reason_tags.rs`](../crates/valve/src/reason_tags.rs))
All 14 canonical `ValveReasonTag` variants map compile-time exhaustively to non-leaky `ValveGuideEntry` cues (no Cedar rules or numeric thresholds exposed in prompts) and feed `build_valve_self_snapshot -> ValveSelfSnapshot` (`valve_v1::valve_self_snapshot_zero_drift_parity_and_exhaustive_reason_guide`):
`PendingApproval`, `quiet_hours_digest`, `stale_or_mismatched_approval`, `oversight_budget_exhausted`, `taint_blocked`, `non_owner_identity_change`, `line2_scope_stripped`, `unknown_audience`, `budget_exhausted`, `chain_brake`, `cedar_denied`, `hook_widening_overridden`, `tainted_procedure_write`, `daily_cap_reached`.

### Configuration Toggles ([`crates/valve/src/config.rs`](../crates/valve/src/config.rs))
Setting `ValveConfig::mode` (`Enforcing`, `Partial`, `Off`) or individual `ValveRuleToggle` booleans toggles only that rule (`valve_v1::toggleability_disables_only_target_rule_and_logs_changes`):

| `ValveConfig` Field | `ValveRuleToggle` | Default | Rule Controlled |
|---|---|---|---|
| `cedar_enabled` | `cedar_abac` | `true` | Cedar policy evaluation |
| `attested_roles_enabled` | `attested_roles` | `true` | Place/physical presence role derivation |
| `line2_enabled` | `line2_scope` | `true` | Absolute Line 2 `VisibleScope` enforcement |
| `budget_guard_enabled` | `budget_guard` | `true` | `daily_token_budget` enforcement |
| `chain_brake_enabled` | `chain_brake` | `true` | `max_chain_wakes` causal chain brake |
| `memory_write_guard_enabled` | `memory_write_guard` | `true` | Tainted/unsigned procedure memory-write block |
| `pending_approval_enabled` | `pending_approval` | `true` | `R_2` `PendingApproval`, TTL, and queue/hourly caps |
| `taint_blocking_enabled` | `taint_blocking` | `true` | 3-door provenance taint quarantine |
| `quiet_hours_enabled` | `quiet_hours` | `false` | Quiet hours `[quiet_start_hour, quiet_end_hour)` & digest batching |
| `proactive_cap_enabled` | `proactive_cap` | `true` | `proactive_daily_cap` (`8/day`) enforcement |

---

## 6. Observed Test & Benchmark Evidence

- **Integration Tests (`cargo test -p monoagens-valve`)**: `12 passed; 0 failed` (`tests/valve_v1.rs`) + `6 passed; 0 failed` (`tests/valve_v2.rs`) = **18/18 passed**.
- **Criterion Microbenchmark (`cargo bench -p monoagens-valve --bench door_verdict -- --warm-up-time 1 --measurement-time 1`)**:
  - Observed `door_verdict` distribution (`100` samples, `76k` iterations): `[12.814 µs, 13.096 µs, 13.454 µs]` (point estimate `13.096 µs`).
  - Note: `< 50 µs` p99 in [`openspec/changes/waist-valve-and-bodies/proposal.md`](../openspec/changes/waist-valve-and-bodies/proposal.md) is the design target; Criterion reports `[min/mean/max CI] = [12.814 µs, 13.096 µs, 13.454 µs]` on [`crates/valve/benches/door_verdict.rs`](../crates/valve/benches/door_verdict.rs).
