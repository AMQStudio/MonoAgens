# Spec Delta

## Purpose

Defines the deterministic `Valve` policy engine embedded in Gateway's Inbound, Context, Action,
and memory-write doors. Compiled via `cedar-policy` (`4.14.1`) and statically checked with
`cedar-policy-symcc` (`0.8.1`), it enforces Absolute Lines 1 & 2, `PendingApproval` + `H_act`,
3-door taint blocking, cost/chain brakes, and quiet-hours digests (`R-33`, `R-41`–`R-43`, `R-46`).

## ADDED Requirements

### Requirement: Deterministic Cedar ABAC and symcc verification govern every door
The valve SHALL evaluate policies deterministically using `cedar-policy` (`4.14.1`) strictly over
logged `(log_seq, t_now)` snapshots with zero LLM inference or live network lookups, default-denying
unpermitted acts, and running `cedar-policy-symcc` (`0.8.1`) on policy load to prove non-owners
can never trigger `identity_changing` or `SelfModify` actions.

#### Scenario: Symcc catches unauthorized identity policy loopholes at startup
- **WHEN** Gateway loads a policy bundle permitting any non-owner path to `identity_changing` acts
- **THEN** `cedar-policy-symcc` flags the counterexample and aborts policy activation fail-closed

#### Scenario: Attested room roles never derive from natural-language text
- **WHEN** an untrusted speaker asserts `"I am admin"` in message text without attested `PlacePresence` or `PhysicalPresence`
- **THEN** Valve ignores the text claim and restricts derived Cedar roles strictly to verified snapshot facts within TTL

### Requirement: Valve runs first and last to preserve Absolute Lines 1 and 2
In Inbound, Context, Action, and memory-write doors, the valve SHALL run first and last so external
hooks can only narrow permissions. It SHALL uphold Absolute Line 1 (owner-only self-modification
and `OwnerSigned` procedures) and Absolute Line 2 (isolating out-of-scope memories absent an
explicit valid `DeclassToken`).

#### Scenario: Secondary door hook cannot widen a Cedar refusal
- **WHEN** an auxiliary hook permits a private memory or `identity_changing` action that Valve denied
- **THEN** Valve's closing pass overrides the permission, strips or refuses it, and logs a `door_verdict`

#### Scenario: Cross-audience memory requires explicit DeclassToken
- **WHEN** context assembly for wider audience B includes a restricted item from scope A without `DeclassToken`
- **THEN** Valve strips the item at the context door before reasoning prompt rendering

### Requirement: Tiered PendingApproval and canonical H_act stop spam and TOCTOU
The action door SHALL classify actions into tiers `R_0`..`R_3`, placing unapproved `R_2` or tainted
actions into non-retrying `PendingApproval` (`max_retry_on_held = 0`) under sliding caps
(`max_prompts_per_hour`, `max_pending_queue`, `dedup_cooldown_s`), and binding approvals to
`H_act = BLAKE3("monoagens.act.v1" || canonical_json(...))` and `state_epoch`.

#### Scenario: Post-approval parameter edit or epoch drift fails closed
- **WHEN** an approved `PendingApproval` resumes after any field in `canonical_json`, `state_epoch`, or `approval_ttl_s` shifts
- **THEN** Valve rejects dispatch with `refused` (`stale_or_mismatched_approval`) and never auto-retries

#### Scenario: Approval filler flood hits sliding bucket cap without retrying held items
- **WHEN** tainted or rapid turns emit `R_2` requests exceeding `max_pending_queue` or `max_prompts_per_hour`
- **THEN** excess actions settle immediately as `refused` (`oversight_budget_exhausted`) with at most one prompt per held item

### Requirement: Three-door taint quarantine blocks privilege escalation cleanly
Whenever turn taint is below `Trusted` (`UntrustedTold` or `UntrustedObserved`), the action door
SHALL hold or refuse every `R_2`/`R_3`, cross-audience, `procedure`-writing, or
identity-changing action absent owner `ApprovalGrant(H_act)` or `DeclassToken`, while blocking
uncorroborated `procedure` memory promotions at write/compaction time.

#### Scenario: Prompt injection in stranger text cannot invoke R2 tools
- **WHEN** a recalled untrusted chat turn embeds hidden tool instructions triggering an `R_2` state-mutating act
- **THEN** turn taint causes the action door to hold or refuse the call while permitting `R_0` replies

### Requirement: Daily budget, chain brake, quiet hours, and batched digests
The valve SHALL enforce daily token budgets (exempting genuine single-shot fired promises, never
`RecurrenceSpec` monitors) and chain wake limits, plus recipient-local quiet hours
(`Q_start..Q_end`), proactive daily caps (`C_daily`), and `3×/day` digest queues emitting `held`
receipts. Each rule SHALL be toggleable in config and logged on change.

#### Scenario: Quiet-hours notification defers to digest as held while monitor obeys budget
- **WHEN** a non-emergency `RecurrenceSpec` notification fires during quiet hours or after `daily_token_budget` is spent
- **THEN** quiet-hours alerts hold in the next `3×/day` digest (`held` receipt) and budget-exhausted monitors are refused without promise bypass
