# Tasks — `core-and-lab` (Run-2 Architecture, Phases 0–3)

This checklist implements `monoagens-protocol`, `monoagens-core`, `monoagens-store`, and `monoagens-lab` across Phases 0–3 (`R-50`), paired cleanly with [`waist-valve-and-bodies`](../waist-valve-and-bodies/proposal.md). Every mechanism, bugfix, and parameter follows [`RESEARCH.md`](../../../RESEARCH.md) and the Run-2 compilation [`2026-11-16-run-2.md`](../../../research/findings/2026-11-16-run-2.md), preserving owner-locked decisions: Option A additive `Receipt` (`termination_reason`, `progress`, `delivered_units`, preserving `ARCHITECTURE.md` L198) and unified `EmbeddingGemma 2` (`256d` MRL, Apache 2.0, `8K` ctx).

## 1. Phase 0 — Toolchain, workspace, and crate boundaries

- [ ] 1.1 Install Rust stable via `rustup` and add `rust-toolchain.toml` pinning `stable`, `rustfmt`, and `clippy`; verify `cargo --version` and `cargo clippy --version` succeed.
- [x] 1.2 Initialize a git repository with `.gitignore` for `target/`, data directories, and `lab/runs/`.
- [ ] 1.3 Configure Cargo workspace crates (`protocol`, `core`, `store`, `lab`) with edition 2024 and `#![forbid(unsafe_code)]`; verify `store` deps include `rusqlite` (`bundled`, `PRAGMA secure_delete = ON`), `usearch`, `blake3`, and `postcard` or `minicbor`.
- [ ] 1.4 Structure `core` modules (`loop`, `gate`, `recall`, `expectations`, `beliefs`, `consolidation`, `map/`, `privacy/`, `tck_hooks.rs`) and add a `cargo metadata` test failing if `monoagens-core` depends on anything beyond `monoagens-protocol` and `serde`.

## 2. Phase 0 — Waist 1 & Waist 2 protocol (`monoagens-protocol`)

- [ ] 2.1 Define core identity/provenance types (`ActorId`, `PlaceId`, `Audience`, `OriginActor`, `OwnerSigned`, `PrincipalLink`, `DeclassToken`, `VisibleScope`, `H_act`, `taint`) and six-facts validation rejecting percepts without an audience.
- [ ] 2.2 Implement Option A `Receipt` (`outcome` restricted to `{done, refused, held, failed, unknown}` + additive `termination_reason?`, `progress?`, `delivered_units?`) and verify serde round-trip without modifying `ARCHITECTURE.md` invariants.
- [ ] 2.3 Define multimodal and attention wire types (`AttachmentHandle`, `f_modal`, `orient`, `continued_attention`, `(channel, Δ_obs, Δ_pred)` channel prediction tuple).
- [ ] 2.4 Define capability & promise extensions (`capability_schema_hash`, `RecurrenceSpec`, `PromiseMeta` with `conflict_key`/`depends_on`) plus `PendingApproval`.
- [ ] 2.5 Define structured `ThoughtAct` (`think`, `act`, `expect`, `resolve`, `revise`, `consolidate` acts `{gist, lesson, procedure}`) and clean Waist 1 / Waist 2 JSON-RPC framing separation with version compatibility tests.

## 3. Phase 0–1 — Event log, 3-class queue, and startup recovery (`monoagens-core`)

- [ ] 3.1 Define all 29 canonical event kinds (including `withheld`, `action_feedback`, `hard_forget`, `PendingApproval` transitions, and causal provenance fields) with forward/backward upcasting tests.
- [ ] 3.2 Implement `Store` and `Doors` ports plus deterministic in-memory `Store` with record-first guarantee (failed appends abort `step()` and emit zero `Effect`s).
- [ ] 3.3 Implement 3-class bounded priority queue (`Class A Urgent Control = 256`, `Class B Conversational = 512`, `Class C Background/Sleep = 256`) drained in strict `A > B > C` order, plus single-receipt `canceled_by_core` / `preempted_external` invariant (`R-36`).
- [ ] 3.4 Implement `Core::open` crash recovery (`A-01`): load latest `blake3`-verified snapshot and replay unconsumed `hot.db` tail events deterministically before admitting new ingress.
- [ ] 3.5 Implement guarded Petrov base-level activation (`n > k ∧ t_n > t_k`, `A-01`) and `Two-Epoch Replay` anchor preservation across authorized `hard_forget` erasures (`R-34`).

## 4. Phase 1 — Appraisal gate, habituation, and patience

- [ ] 4.1 Implement the `info_gap` gate signal via the frozen `A-14` logistic `dialogue_act_gap` scorer `(w_gap, b_gap)` plus reachable neutral weights for all signals.
- [ ] 4.2 Implement continuous noisy-OR `deg(e, l)` expectation-relevance matching (`R-29`) and breakpoint-aware `Δθ_init` (`R-30`) + opportunity-cost term (`R-31`).
- [ ] 4.3 Implement post-evaluation partitioned habituation `h_{(p, k_src)}` (`A-03`, `R-36`) using `S_eff = S / (1 + κ · h_pre)` without pre-evaluation suppression.
- [ ] 4.4 Delete obsolete `"or asleep"` short-circuit, enforce `Δθ_sleep = 0` (`A-04`), and implement shrunk quantile patience `q_0.10` (`A-08`).
- [ ] 4.5 Record full per-turn gate decision trace (`P`, `θ`, signal breakdown, `withheld` silence markers) for replay verification.

## 5. Phase 1–2 — Memory recall, `VisibleScope`, and cognitive maps

- [ ] 5.1 Enforce pre-Step-5 `VisibleScope` projection (`R-33`) BEFORE FOK scoring, vector/text retrieval, and context assembly so restricted memories leak neither content nor counts.
- [ ] 5.2 Implement `A-09` dual-process familiarity (`F`) + recollection (`R`) without fixed 30-day blocks and configure `w_B = 0` Row-10 neutral weight.
- [ ] 5.3 Build Successor Representation (SR) topological cognitive map edges (`map/`, `R-32`) linking co-occurring places, people, and tasks without unbounded random walks.
- [ ] 5.4 Enforce query partitioning by `(embed_model_ver, dim_d, prompt_regime_id)` across `usearch` HNSW and SQLite FTS5 hybrid candidates.

## 6. Phase 1–2 — Expectations, recurrences, and approvals

- [ ] 6.1 Extend `Expectation` with plural `places`/`people`, optional `domain?`, `monitor`, and `pending_approval` kinds (`R-42`, `R-46`).
- [ ] 6.2 Implement `RecurrenceSpec` scheduling and coalesced startup recovery so downtime produces one catch-up firing rather than an alarm storm.
- [ ] 6.3 Track `{kept_late, renegotiated, released, reached}` transitions, `continued_attention` follow-up windows, `(channel, Δ_obs, Δ_pred)`, and undiluted `promise_debt` urgency.
- [ ] 6.4 Enforce `PromiseMeta` conflict detection (`conflict_key`), causal `depends_on` chaining, `PendingApproval` pause/resume, and non-retry on `preempted_external` / `canceled_by_core`.

## 7. Phase 1–3 — Context plan, safety anchoring, and reasoning

- [ ] 7.1 Implement `ContextPlan` with immutable slot-0 `OwnerSigned` `IdentityAnchor` and `PrincipleDescriptor`s (`R-48`), bounded sections, and Spotlighting `[UNTRUSTED_QUOTED]` delimiters (`R-43`).
- [ ] 7.2 Implement monotonic `origin_actor` and minimum-trust `taint` propagation across percepts, recalled memories, thoughts, and emitted actions.
- [ ] 7.3 Enforce `[ANCHOR_CHECK]`, `PrincipleAudit`, `CommitCheck` (`R-49`), and `CitationLock` before dispatching outbound or promise-mutating acts.
- [ ] 7.4 Implement `[0, 20]` anchored confidence + balanced `1–5` motivation parsing, `0/0`-guarded report control `P_rc` (`A-01`), structured `memory.recall` auto-recall loop, and safe `withheld` logging.

## 8. Phase 1–2 — Durable segmented storage (`monoagens-store`)

- [ ] 8.1 Implement WAL-mode `hot.db` SQLite store (`rusqlite` bundled, `PRAGMA secure_delete = ON`) with transactional append and ordered scans.
- [ ] 8.2 Implement read-only sealed temporal shards, per-shard FTS5 tables (`'secure-delete'=1`), and `usearch` HNSW vector persistence partitioned by `(embed_model_ver, dim_d, prompt_regime_id)`.
- [ ] 8.3 Implement `blake3`-checksummed `postcard` or `minicbor` state snapshots synced with `Two-Epoch Replay` sequence watermarks.
- [ ] 8.4 Implement atomic `hard_forget` erasure scrubbing SQLite rows, FTS5 tokens, and `usearch` entries followed by an immediate clean snapshot (`R-34`).

## 9. Phase 1–2 — Model providers & unified `256d` `EmbeddingGemma 2`

- [ ] 9.1 Define strict `R | null` schema validators for thought acts (`think`, `act`, `expect`, `resolve`, `revise`, `consolidate`) with diagnostic `failed_wake` errors on malformed responses.
- [ ] 9.2 Implement `consolidate` structured outputs (`gist`, `lesson`, `procedure`) and `[0, 20]` / `1–5` rubric decoding.
- [ ] 9.3 Implement `scripted`, offline deterministic, OpenAI-compatible HTTP model adapters with zero secret leakage in error events.
- [ ] 9.4 Integrate unified Apache 2.0 `EmbeddingGemma 2` (`256d` MRL, `8K` ctx, `arXiv:2610.37364`) across text, image, and `audio_voice` with mandatory task prefixes (`t_clf`, `t_sts`, `t_q`, `t_d`) and strict `(embed_model_ver, dim_d, prompt_regime_id)` non-mixing, plus `EmbeddingGemma-300M` Lab comparative baseline and `none` fallback.
- [ ] 9.5 Verify provider error, HTTP timeout, and offline fallback leave the cognitive loop alive and log recoverable errors.
- [ ] 9.6 Build golden fixture tests ensuring identical schema parsing across scripted and mock-HTTP providers.
- [ ] 9.7 Implement `A-14` frozen logistic `dialogue_act_gap` head (replacing legacy `w_info = 0` static baseline) and blind `AttachmentHandle` byte resolution strictly after `VisibleScope` and door/`Valve` authorization.

## 10. Phase 2–3 — Beliefs, `told` credences, and cognitive grounding

- [ ] 10.1 Implement Beta-Bernoulli (`(α_c, β_c)`) belief & credibility store initializing `told` credences from `(α_0, β_0)` prior cleanly separated from direct evidence.
- [ ] 10.2 Move verbatim-vs-gist verification into `consolidate` post-sleep checks with provenance links (`R-35` / Run-2 beliefs spec).
- [ ] 10.3 Implement skeptical corroboration `k_corrob` discounting echoed untrusted sources and minimum-trust `taint` infection (`R-43`), plus `R-32` social convention discounting.
- [ ] 10.4 Recompute `(α, β)` projections accurately when authorized `hard_forget` erases supporting claims (`R-34`).

## 11. Phase 1–3 — Lab harness, 22 metrics, and Pareto baselines (`monoagens-lab`)

- [ ] 11.1 Implement Waist-2 `TestBody` JSON-RPC framing and virtual clock stepping directly to `next_due()` for deterministic multi-week simulation in milliseconds.
- [ ] 11.2 Implement all 22 brain-map metrics plus extended `tasks.md` Section 10.5 metrics (`NI`, safety leakage, promise fulfillment, `hard_forget` zero-resurrection, barge-in latency).
- [ ] 11.3 Implement paired statistical non-inferiority runner (`S >= 354–471` paired scenarios, `A-16`), `Arm T` transcript baseline comparison, and `1/n` `[0.5, 2.0]` calibration band checks.
- [ ] 11.4 Implement `BaselineAdapter` and 5-domain Pareto frontier scorer (`R-39`) alongside Indonesian & informal register benchmarks (`MultiLexNorm`, `IndoCollex`, `A-15`).

## 12. Phase 2 — Sleep consolidation: episodes & EM-LLM boundaries

- [ ] 12.1 Trigger sleep only at `>= N_min` unsealed events over immutable `t_snapshot` while preserving `Δθ_sleep = 0` (`A-04`).
- [ ] 12.2 Implement `{open, provisional, sealed}` episode lifecycle per place with `superseded_by` linkage when idle tails reopen (`A-05`).
- [ ] 12.3 Implement `[0, 1]` normalized `EM-LLM` boundary segmentation with cold-start shrinkage instead of fixed-count splits (`A-06`).

## 13. Phase 2 — Sleep consolidation: clustering & Petrov fade

- [ ] 13.1 Implement domain-partitioned clustering with Hungarian cross-snapshot domain alignment (`A-12`).
- [ ] 13.2 Propagate monotonic `min(trust)` taint across consolidated gists and episode summaries.
- [ ] 13.3 Implement guarded Petrov archive/fade pruning (`n > k ∧ t_n > t_k`, archiving below `B < -3.29`) without dropping `hard_forget` tombstones (`A-01`).

## 14. Phase 2 — Sleep consolidation: lessons & schema-hashed procedures

- [ ] 14.1 Synthesize contrastive `lesson` artifacts comparing succeeded vs. failed chains rather than bare summaries.
- [ ] 14.2 Extract capability-bound `procedure` templates stamped with `capability_schema_hash` (`A-11`, `R-38`).
- [ ] 14.3 Monitor deployed procedures with CUSUM drift detection and auto-retire invalidated procedures on schema mismatch or accuracy decay (`R-38`).

## 15. Phase 2 — Perception ledger & four-pillar `self_card`

- [ ] 15.1 Maintain tick-driven audience perception cards covering both triggering participants and active promisees (`R-35`).
- [ ] 15.2 Derive the four-pillar `self_card` strictly from audited log events rather than uncontrolled free-form self-edit.
- [ ] 15.3 Trigger deterministic external-door recalibration on `self_card` or capability changes without silently mutating `OwnerSigned` identity.

## 16. Phase 2 — Bounded learning & preemption non-penalty

- [ ] 16.1 Fix delta-rule polarity to `r = 1` on `{good_act, missed_silence}` and `r = 0` on `{bad_act, answered_silence}` with sign-consistent `(r − P) · s_k` (`R-19`).
- [ ] 16.2 Apply clipped Inverse Propensity Scoring `w_ips = min(M_ips, 1/ρ)` on sampled withheld silences.
- [ ] 16.3 Enforce non-penalty guard skipping `(α_c, β_c)` Beta and weight degradation when `Receipt.termination_reason` is `canceled_by_core` or `preempted_external` (`R-36`, Option A).
- [ ] 16.4 Implement shrunk quantile calibration (`q_0.10`) for `wake_gate_lo` and event-logged weight rollback.

## 17. Phase 1–2 — Deterministic replay & performance budgets

- [ ] 17.1 Implement full log replay and post-erasure `Two-Epoch Replay` verification comparing recomputed decisions bit-for-bit.
- [ ] 17.2 Add payload-independence tests confirming gate decisions depend strictly on extracted numeric/structural features and not raw opaque bytes.
- [ ] 17.3 Enforce suite timing budgets (`monoagens-core < 3 s`, scripted `monoagens-lab` dev suite `< 30 s`, zero network IO in default unit suites).

## 18. Phase 1–2 — Mechanism registry & 22 reachable neutral values

- [ ] 18.1 Implement compile-time `Switches` registry covering all 22 mechanisms from `design.md` D18 with documented `RESEARCH.md` section, causal claim, and reachable neutral value.
- [ ] 18.2 Verify neutral ablation parity: running representative scenarios with each individual switch neutralized executes cleanly without divide-by-zero, NaN, or stalled expectations.

## 19. Phase 2–3 — Security, privacy, and robustness Lab suites

- [ ] 19.1 Add `VisibleScope` pre-Step-5 leakage suite verifying zero private title/count leakage during FOK, recall, and cross-audience chains.
- [ ] 19.2 Add `hard_forget` erasure verification confirming zero reconstruction via SQLite, FTS5, `usearch`, snapshots, or `Two-Epoch Replay`.
- [ ] 19.3 Add indirect prompt-injection, Spotlighting bypass, `taint` laundering, and multi-turn `IdentityAnchor` sycophancy drift scenarios.

## 20. Phase 3 — Core Conformance TCK (`C-MUST-01..04`)

- [ ] 20.1 Expose deterministic `tck_hooks.rs` test harness entry points inside `monoagens-core` (compiled behind test/TCK features without adding gateway deps).
- [ ] 20.2 Verify `C-MUST-01` single-consumer queue order (`A > B > C`) & single-receipt preemption invariant.
- [ ] 20.3 Verify `C-MUST-02` record-first crash recovery via `Core::open`, `C-MUST-03` pre-Step-5 `VisibleScope` + `taint` isolation, and `C-MUST-04` non-penalty on preemption.

## 21. Research sync & repository docs

- [x] 21.1 Compile Run-2 handbook (`research/findings/2026-11-16-run-2.md`) and update `RESEARCH.md` master formulas/citations (`arXiv:2607.15648`, `arXiv:2502.18658`, `arXiv:2610.37364`, Run-2 terminology).
- [ ] 21.2 Author `research/backlog.md` synchronized with Run-2 open calibration items (`design.md` D19) and benchmark status.
- [ ] 21.3 Update top-level `README.md` with workspace layout, `core-and-lab` vs `waist-valve-and-bodies` split, build/test commands, and privacy/safety guarantees.

## 22. Verification gates

- [ ] 22.1 Verify `openspec validate core-and-lab --strict` and `openspec validate waist-valve-and-bodies --strict` exit `0` with zero errors or warnings.
- [ ] 22.2 After Rust implementation begins, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`.
- [ ] 22.3 Execute paid live-model Lab benchmarks (`Arm T` vs MonoAgens, `S >= 354–471`, `lab/scenarios/sealed/`) **only upon explicit owner authorization**.
