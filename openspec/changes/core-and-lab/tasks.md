# Tasks — `core-and-lab` (Run-2 Architecture, Phases 0–3)

This checklist implements `monoagens-protocol`, `monoagens-core`, `monoagens-store`, and `monoagens-lab` across Phases 0–3 (`R-50`), paired cleanly with [`waist-valve-and-bodies`](../waist-valve-and-bodies/proposal.md). Every mechanism, bugfix, and parameter follows [`RESEARCH.md`](../../../RESEARCH.md) and the Run-2 compilation [`2026-11-16-run-2.md`](../../../research/findings/2026-11-16-run-2.md), preserving owner-locked decisions: Option A additive `Receipt` (`termination_reason`, `progress`, `delivered_units`, preserving `ARCHITECTURE.md` L198) and unified `EmbeddingGemma 2` (`256d` MRL, Apache 2.0, `8K` ctx).

## 1. Phase 0 — Toolchain, workspace, and crate boundaries

- [x] 1.1 Install latest stable Rust via `rustup` (`rustup toolchain install stable`) and add `rust-toolchain.toml` pinning the active stable channel with `rustfmt` and `clippy`; query `context7` MCP (`resolve-library-id` -> `query-docs`) for latest upstream docs (`rusqlite`, `usearch`, `hnsw_rs`, `ort`, `proptest`, `criterion`, `mem0`, `letta-client`, `langgraph`, `graphiti`) and verify `cargo --version` and `cargo clippy --version` succeed.
- [x] 1.2 Initialize a git repository with `.gitignore` for `target/`, data directories, and `lab/runs/`.
- [x] 1.3 Configure Cargo workspace crates (`protocol`, `core`, `store`, `lab`) using latest stable compatible crates in `Cargo.lock` with Edition 2024 and `#![forbid(unsafe_code)]`; verify `store` deps include `rusqlite` (`bundled`, `PRAGMA secure_delete = ON`), `VectorIndex` (`usearch` primary + pure-Rust `hnsw_rs` fallback), `blake3`, and `postcard` or `minicbor`, plus workspace dev-deps `proptest` and `criterion`.
- [x] 1.4 Structure `core` modules (`loop`, `gate`, `recall`, `expectations`, `beliefs`, `consolidation`, `map/`, `privacy/`, `tck_hooks.rs`) and add a `cargo metadata` test failing if `monoagens-core` depends on anything beyond `monoagens-protocol` and `serde`.

## 2. Phase 0 — Waist 1 & Waist 2 protocol (`monoagens-protocol`)

- [x] 2.1 Define core identity/provenance types (`ActorId`, `PlaceId`, `Audience`, `OriginActor`, `OwnerSigned`, `PrincipalLink`, `DeclassToken`, `VisibleScope`, `H_act`, `taint`) and six-facts validation rejecting percepts without an audience.
- [x] 2.2 Implement Option A `Receipt` (`outcome` restricted to `{done, refused, held, failed, unknown}` + additive `termination_reason?`, `progress?`, `delivered_units?`) and verify serde round-trip without modifying `ARCHITECTURE.md` invariants.
- [x] 2.3 Define multimodal and attention wire types (`AttachmentHandle`, `f_modal`, `orient`, `continued_attention`, `(channel, Δ_obs, Δ_pred)` channel prediction tuple).
- [x] 2.4 Define capability & promise extensions (`capability_schema_hash`, `RecurrenceSpec`, `PromiseMeta` with `conflict_key`/`depends_on`) plus `PendingApproval`.
- [x] 2.5 Define structured `ThoughtAct` (`think`, `act`, `expect`, `resolve`, `revise`, `consolidate` acts `{gist, lesson, procedure}`) and clean Waist 1 / Waist 2 JSON-RPC framing separation with version compatibility tests.

## 3. Phase 0–1 — Event log, 3-class queue, and startup recovery (`monoagens-core`)

- [x] 3.1 Define all 29 canonical event kinds (including `withheld`, `action_feedback`, `hard_forget`, `PendingApproval` transitions, and causal provenance fields) with forward/backward upcasting tests.
- [x] 3.2 Implement `Store` and `Doors` ports plus deterministic in-memory `Store` with record-first guarantee (failed appends abort `step()` and emit zero `Effect`s).
- [x] 3.3 Implement 3-class bounded priority queue (`Class A Urgent Control = 256`, `Class B Conversational = 512`, `Class C Background/Sleep = 256`) drained in strict `A > B > C` order, plus single-receipt `canceled_by_core` / `preempted_external` invariant (`R-36`).
- [x] 3.4 Implement `Core::open` crash recovery (`A-01`): load latest `blake3`-verified snapshot and replay unconsumed `hot.db` tail events deterministically before admitting new ingress.
- [x] 3.5 Implement guarded Petrov base-level activation (`n > k ∧ t_n > t_k`, `A-01`) and `Two-Epoch Replay` anchor preservation across authorized `hard_forget` erasures (`R-34`).

## 4. Phase 1 — Appraisal gate, habituation, and patience

- [x] 4.1 Implement the `info_gap` gate signal via the frozen `A-14` logistic `dialogue_act_gap` scorer `(w_gap, b_gap)` plus reachable neutral weights for all signals.
- [x] 4.2 Implement continuous noisy-OR `deg(e, l)` expectation-relevance matching (`R-29`) and breakpoint-aware `Δθ_init` (`R-30`) + opportunity-cost term (`R-31`).
- [x] 4.3 Implement post-evaluation partitioned habituation `h_{(p, k_src)}` (`A-03`, `R-36`) using `S_eff = S / (1 + κ · h_pre)` without pre-evaluation suppression.
- [x] 4.4 Delete obsolete `"or asleep"` short-circuit, enforce `Δθ_sleep = 0` (`A-04`), and implement shrunk quantile patience `q_0.10` (`A-08`).
- [x] 4.5 Record full per-turn gate decision trace (`P`, `θ`, signal breakdown, `withheld` silence markers) for replay verification.

## 5. Phase 1–2 — Memory recall, `VisibleScope`, and cognitive maps

- [x] 5.1 Enforce pre-Step-5 `VisibleScope` projection (`R-33`) BEFORE FOK scoring, vector/text retrieval, and context assembly so restricted memories leak neither content nor counts.
- [x] 5.2 Implement `A-09` dual-process familiarity (`F`) + recollection (`R`) without fixed 30-day blocks and configure `w_B = 0` Row-10 neutral weight.
- [x] 5.3 Build Successor Representation (SR) topological cognitive map edges (`map/`, `R-32`) linking co-occurring places, people, and tasks without unbounded random walks.
- [x] 5.4 Enforce query partitioning by `(embed_model_ver, dim_d, prompt_regime_id)` across `usearch` HNSW and SQLite FTS5 hybrid candidates.

## 6. Phase 1–2 — Expectations, recurrences, and approvals

- [x] 6.1 Extend `Expectation` with plural `places`/`people`, optional `domain?`, `monitor`, and `pending_approval` kinds (`R-42`, `R-46`).
- [x] 6.2 Implement `RecurrenceSpec` scheduling and coalesced startup recovery so downtime produces one catch-up firing rather than an alarm storm.
- [x] 6.3 Track `{kept_late, renegotiated, released, reached}` transitions, `continued_attention` follow-up windows, `(channel, Δ_obs, Δ_pred)`, and undiluted `promise_debt` urgency.
- [x] 6.4 Enforce `PromiseMeta` conflict detection (`conflict_key`), causal `depends_on` chaining, `PendingApproval` pause/resume, and non-retry on `preempted_external` / `canceled_by_core`.

## 7. Phase 1–3 — Context plan, safety anchoring, and reasoning

- [x] 7.1 Implement `ContextPlan` with immutable slot-0 `OwnerSigned` `IdentityAnchor` and `PrincipleDescriptor`s (`R-48`), bounded sections, and Spotlighting `[UNTRUSTED_QUOTED]` delimiters (`R-43`).
- [x] 7.2 Implement monotonic `origin_actor` and minimum-trust `taint` propagation across percepts, recalled memories, thoughts, and emitted actions.
- [x] 7.3 Enforce `[ANCHOR_CHECK]`, `PrincipleAudit`, `CommitCheck` (`R-49`), and `CitationLock` before dispatching outbound or promise-mutating acts.
- [x] 7.4 Implement `[0, 20]` anchored confidence + balanced `1–5` motivation parsing, `0/0`-guarded report control `P_rc` (`A-01`), structured `memory.recall` auto-recall loop, and safe `withheld` logging.
- [x] 7.5 Implement `R-38` progressive `Tool RAG` disclosure (`<= 4` static `OwnerSigned` core tools in cached prefix + hybrid BM25/`256d` retrieval of `<= k_cap = 8` capabilities & `<= k_proc = 3` procedures filtered by `ActiveCaps ∩ VisibleScope` and matching `capability_schema_hash`, plus neutral ablation `k_cap = |C_avail|, k_proc = 0`).
- [x] 7.6 Add ablatable `Switches.self_mechanics_guide` (`neutral = false`), typed `CoreSelfSnapshot` + `Doors::context` `ValveSelfSnapshot` attachment slot on `ContextPlan`, exhaustive Rust `match` `[SELF_GUIDE]` prefix renderer (`<= 250` tokens, zero hidden formula leakage) + just-in-time `auto_recall`/`held` working-set cues in `reason/src/render.rs`, and compile/snapshot zero-drift parity tests.

## 8. Phase 1–2 — Durable segmented storage (`monoagens-store`)

- [x] 8.1 Implement WAL-mode `hot.db` SQLite store (`rusqlite` bundled, `PRAGMA secure_delete = ON`) with transactional append and ordered scans.
- [x] 8.2 Implement read-only sealed temporal shards, per-shard FTS5 tables (`'secure-delete'=1`), and pluggable `VectorIndex` trait (`usearch` `i8`/`bf16` in-traversal HNSW primary + pure-Rust `hnsw_rs` and `BruteForceFlat` fallbacks) partitioned by `(embed_model_ver, dim_d, prompt_regime_id)`.
- [x] 8.3 Implement `blake3`-checksummed `postcard` or `minicbor` state snapshots synced with `Two-Epoch Replay` sequence watermarks.
- [x] 8.4 Implement atomic `hard_forget` erasure scrubbing SQLite rows, FTS5 tokens, and `VectorIndex` entries followed by an immediate clean snapshot and raw byte scan check (`R-34`).

## 9. Phase 1–2 — Model providers & unified `256d` `EmbeddingGemma 2`

- [x] 9.1 Define strict `R | null` schema validators for thought acts (`think`, `act`, `expect`, `resolve`, `revise`, `consolidate`) with diagnostic `failed_wake` errors on malformed responses.
- [x] 9.2 Implement `consolidate` structured outputs (`gist`, `lesson`, `procedure`) and `[0, 20]` / `1–5` rubric decoding.
- [x] 9.3 Implement `scripted`, offline deterministic, OpenAI-compatible HTTP model adapters with zero secret leakage in error events.
- [x] 9.4 Integrate unified Apache 2.0 `EmbeddingGemma 2` (`256d` MRL, `8K` ctx, `arXiv:2610.37364`) behind one embedder trait runnable via native `ort` (`ONNX Runtime`) / `LiteRT`, local stdio/HTTP sidecar, or remote endpoint across text, image, and `audio_voice` with mandatory task prefixes (`t_clf`, `t_sts`, `t_q`, `t_d`) and strict `(embed_model_ver, dim_d, prompt_regime_id)` non-mixing, plus `EmbeddingGemma-300M` Lab comparative baseline and `none` fallback.
- [x] 9.5 Verify provider error, HTTP timeout, and offline fallback leave the cognitive loop alive and log recoverable errors.
- [x] 9.6 Build golden fixture tests ensuring identical schema parsing across scripted and mock-HTTP providers.
- [x] 9.7 Implement `A-14` frozen logistic `dialogue_act_gap` head (replacing legacy `w_info = 0` static baseline) and blind `AttachmentHandle` byte resolution strictly after `VisibleScope` and door/`Valve` authorization.

## 10. Phase 2–3 — Beliefs, `told` credences, and cognitive grounding

- [x] 10.1 Implement Beta-Bernoulli (`(α_c, β_c)`) belief & credibility store initializing `told` credences from `(α_0, β_0)` prior cleanly separated from direct evidence.
- [x] 10.2 Move verbatim-vs-gist verification into `consolidate` post-sleep checks with provenance links (`R-35` / Run-2 beliefs spec).
- [x] 10.3 Implement skeptical corroboration `k_corrob` discounting echoed untrusted sources and minimum-trust `taint` infection (`R-43`), plus `R-32` social convention discounting.
- [x] 10.4 Recompute `(α, β)` projections accurately when authorized `hard_forget` erases supporting claims (`R-34`).

## 11. Phase 1–3 — Lab harness, 22 metrics, `Mem0`/`Letta` baselines & Pareto (`monoagens-lab`)

- [x] 11.1 Implement Waist-2 `TestBody` JSON-RPC framing and virtual clock stepping directly to `next_due()` for deterministic multi-week simulation in milliseconds.
- [x] 11.2 Implement all 22 brain-map and safety metrics specified in `specs/lab/spec.md` (`A-02`, including `wakes_per_event`, calls/tokens by purpose, promises `kept`/`kept_late`/repaired, `ECE`/`Brier`, `detail_without_recollection`, `Delta_NI = 0`, safety leakage, `hard_forget` zero-resurrection, and barge-in latency).
- [x] 11.3 Implement standardized `BaselineAdapter` protocol & external harness shims for `Stateless ReAct`, `Full-Transcript Arm F`, `Mem0`, `Letta/MemGPT`, `LangGraph + LangMem`, `Zep/Graphiti`, and `Hermes Agent` enforcing virtual clock injection (`sim_time_rfc3339`), `PerceptView` oracle-stripping, explicit `noop()`/`[SILENT]` token parity, normalized 5-tuple `E_tok`/`C_share`, `restart_epoch` RAM eviction, and `SEALED_MODE=1` isolation, plus `adapter_conformance` self-tests.
- [x] 11.4 Implement `monoagens-lab benchmark` and `pareto` runners across curated 5-domain suites (`LoCoMo` incl. Cat-5, `LongMemEval`, `MemoryAgentBench`, `GroupMemBench`, `STALE`; `PM-Bench`/`PIS`; `PITCH`, `ProACT`, `ProactiveBench`, `CAPA`, `When2Speak`; `Ubuntu IRC`, `Molweni` excluding `MuTual` as multi-party; `FANToM ALL*`, `OpenToM`, `HI-TOM`, `ExploreToM`/`DynToM`), Indonesian (`MultiLexNorm`, `IndoCollex`, `A-15`), `Tool RAG` scaling (`N_caps = 50..200`, `R-38`), and `MPBench` security (`R-43`), reporting paired `S >= 354–471` `Arm T` restart parity (`A-16`), `1/n` `[0.5, 2.0]` calibration band checks, `Holm-Bonferroni` (`d_z`, `r_rb`) statistics, and undominated `Convex Pareto` JSON frontiers (`R-39`, `A-16`).

## 12. Phase 2 — Sleep consolidation: episodes & EM-LLM boundaries

- [x] 12.1 Trigger sleep only at `>= N_min` unsealed events over immutable `t_snapshot` while preserving `Δθ_sleep = 0` (`A-04`).
- [x] 12.2 Implement `{open, provisional, sealed}` episode lifecycle per place with `superseded_by` linkage when idle tails reopen (`A-05`).
- [x] 12.3 Implement `[0, 1]` normalized `EM-LLM` boundary segmentation with cold-start shrinkage instead of fixed-count splits (`A-06`).

## 13. Phase 2 — Sleep consolidation: clustering & Petrov fade

- [x] 13.1 Implement domain-partitioned clustering with Hungarian cross-snapshot domain alignment (`A-12`).
- [x] 13.2 Propagate monotonic `min(trust)` taint across consolidated gists and episode summaries.
- [x] 13.3 Implement guarded Petrov archive/fade pruning (`n > k ∧ t_n > t_k`, archiving below `B < -3.29`) without dropping `hard_forget` tombstones (`A-01`).

## 14. Phase 2 — Sleep consolidation: lessons & schema-hashed procedures

- [x] 14.1 Synthesize contrastive `lesson` artifacts comparing succeeded vs. failed chains rather than bare summaries.
- [x] 14.2 Extract capability-bound `procedure` templates stamped with `capability_schema_hash` (`A-11`, `R-38`).
- [x] 14.3 Monitor deployed procedures with CUSUM drift detection and auto-retire invalidated procedures on schema mismatch or accuracy decay (`R-38`).

## 15. Phase 2 — Perception ledger & four-pillar `self_card`

- [x] 15.1 Maintain tick-driven audience perception cards covering both triggering participants and active promisees (`R-35`).
- [x] 15.2 Derive the four-pillar `self_card` strictly from audited log events rather than uncontrolled free-form self-edit.
- [x] 15.3 Trigger deterministic external-door recalibration on `self_card` or capability changes without silently mutating `OwnerSigned` identity.

## 16. Phase 2 — Bounded learning & preemption non-penalty

- [x] 16.1 Fix delta-rule polarity to `r = 1` on `{good_act, missed_silence}` and `r = 0` on `{bad_act, answered_silence}` with sign-consistent `(r − P) · s_k` (`R-19`).
- [x] 16.2 Apply clipped Inverse Propensity Scoring `w_ips = min(M_ips, 1/ρ)` on sampled withheld silences.
- [x] 16.3 Enforce non-penalty guard skipping `(α_c, β_c)` Beta and weight degradation when `Receipt.termination_reason` is `canceled_by_core` or `preempted_external` (`R-36`, Option A).
- [x] 16.4 Implement shrunk quantile calibration (`q_0.10`) for `wake_gate_lo` and event-logged weight rollback.

## 17. Phase 1–2 — Deterministic replay, `proptest` invariants, and `criterion` scale budgets

- [x] 17.1 Implement full log replay and post-erasure `Two-Epoch Replay` verification comparing recomputed decisions bit-for-bit, plus `proptest` randomized property suites covering replay determinism, `VisibleScope` `Δ_NI == 0`, and Petrov hybrid monotonicity.
- [x] 17.2 Add payload-independence tests confirming gate decisions depend strictly on extracted numeric/structural features and not raw opaque bytes.
- [x] 17.3 Enforce suite timing budgets (`monoagens-core < 3 s`, `store`/`reason < 10 s`, scripted `monoagens-lab` dev suite `< 30 s`, combined `dev + validation < 60 s`, zero network IO in default unit suites) and add `criterion` + `R-40` long-log microbenchmarks validating `Core::open < 100 ms` across `10^5–10^7` events and filtered `VectorIndex` (`usearch` vs `hnsw_rs` vs `Flat`) `< 5 ms` p99 at `1%` selectivity (`Recall@10 >= 0.93`).

## 18. Phase 1–2 — Mechanism registry & 22 reachable neutral values

- [x] 18.1 Implement compile-time `Switches` registry covering all 22 mechanisms from `design.md` D18 with documented `RESEARCH.md` section, causal claim, and reachable neutral value.
- [x] 18.2 Verify neutral ablation parity: running representative scenarios with each individual switch neutralized executes cleanly without divide-by-zero, NaN, or stalled expectations.

## 19. Phase 2–3 — Security, privacy, and robustness Lab suites

- [x] 19.1 Add `VisibleScope` pre-Step-5 leakage suite verifying zero private title/count leakage during FOK, recall, and cross-audience chains.
- [x] 19.2 Add `hard_forget` erasure verification confirming zero reconstruction via SQLite, FTS5, `usearch`, snapshots, or `Two-Epoch Replay`.
- [x] 19.3 Add indirect prompt-injection, Spotlighting bypass, `taint` laundering, and multi-turn `IdentityAnchor` sycophancy drift scenarios.
- [x] 19.4 Add `self_mechanics_guide` ablation suites measuring held/refused explanation truthfulness, `detail_without_recollection` under `R = 0`, zero-drift switch parity, and casual-conversation non-verbosity regression (`>= 5` seeds).

## 20. Phase 3 — Core invariants & Conformance TCK (`C-MUST-01..04`)

- [x] 20.1 Expose deterministic `tck_hooks.rs` test harness entry points inside `monoagens-core` (compiled behind test/TCK features without adding gateway deps).
- [x] 20.2 Verify core loop invariants: single-consumer queue order (`A > B > C`), single-receipt preemption non-penalty (`R-36`), record-first crash recovery via `Core::open`, and pre-Step-5 `VisibleScope` + `taint` isolation.
- [x] 20.3 Verify canonical `R-50` Core TCK checks via `tck_hooks`: `C-MUST-01` privacy non-interference (`Delta_NI = 0.0`), `C-MUST-02` atomic `hard_forget` zero-resurrection (`0-byte / 0-backflow`), `C-MUST-03` seeded bit-identical determinism/replay, and `C-MUST-04` 10k-tick neutral-profile stability soak.

## 21. Research sync & repository docs

- [x] 21.1 Compile Run-2 handbook (`research/findings/2026-11-16-run-2.md`) and update `RESEARCH.md` master formulas/citations (`arXiv:2607.15648`, `arXiv:2502.18658`, `arXiv:2610.37364`, Run-2 terminology).
- [x] 21.2 Author `research/backlog.md` synchronized with Run-2 open calibration items (`design.md` D19) and benchmark status.
- [x] 21.3 Update top-level `README.md` with workspace layout, `core-and-lab` vs `waist-valve-and-bodies` split, build/test commands, and privacy/safety guarantees.

## 22. Verification gates

- [x] 22.1 Verify `openspec validate core-and-lab --strict` and `openspec validate waist-valve-and-bodies --strict` exit `0` with zero errors or warnings.
- [x] 22.2 After Rust implementation begins, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`.
- [ ] 22.3 Execute paid live-model Lab benchmarks (`Arm T` vs MonoAgens, `S >= 354–471`, `lab/scenarios/sealed/`) **only upon explicit owner authorization**.

## 23. Reopened — prior loading, `prior_loaded`, and scripted ablation baseline (owner review 2026-11-17, Decision 20)

- [x] 23.1 Add a test that parses `prior/dispositions.json` and fails when it differs from `Prior::default()`; mark `Prior` `#[serde(deny_unknown_fields)]`.
- [x] 23.2 Add one shared host-side prior loader (outside `monoagens-core`) returning the validated `Prior` plus its digest (`BLAKE3` over canonical JSON of `Prior`); errors name the unknown key, the out-of-range disposition and range, or the unreadable path; no fallback to the default when a path is given.
- [x] 23.3 Add the `prior_loaded` event kind (30th, additive, versioned, upcast-tested): `Core::open` appends it before new input when the log has none or the given prior differs from the last recorded; never erased by `hard_forget`.
- [x] 23.4 Make crash recovery, `replay_verify`, `replay_from_checkpoint` and offline replay take the prior from `prior_loaded` events per log position, never from the caller; add a two-prior log replay test.
- [x] 23.5 Add `--prior <path>` to every `monoagens-lab` command that runs the core (`run`, `suite`, `compare`, `replay`, `baseline`, `benchmark`); record format version, content version and digest of the prior actually used (scenario prior wins) in every run record.
- [x] 23.6 Extend `monoagens-lab compare` to accept a scenario set (`--set development|validation`) and an output directory writing one run record per side and seed.
- [ ] 23.7 Run the scripted ablation of all 22 mechanisms over the development set with seeds 1–5 into `lab/runs/<YYYY-MM-DD>-scripted-ablation/`, plus `summary.md` (mechanism × metric delta, interval, verdict, coverage gaps). Scripted model only, no network, never read `lab/scenarios/sealed/`, never write `lab/runs/reference/`.
- [x] 23.8 Verify `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `openspec validate core-and-lab --strict`.

## 24. Reopened — live model-call defects (owner review 2026-11-18, Decision 21)

Machine rules: one cargo command at a time, never build in `/tmp`, targeted tests while iterating. No paid calls in 24.1–24.7. Core changes are allowed only for 24.2.

- [x] 24.1 Diagnose the output-format failures of the 2026-11-18 pilot (`lab/runs/2026-11-18-phase0-deepseek-v4.1-flash/pilot/`, local) without new calls: per purpose and type, the failing field, the value returned, the rule it broke, and whether a strict schema was sent and honoured; fix defects in our prompt or schema; write `diagnosis.md` in that run dir and add the failure-type record (spec "Output-format failures are diagnosed before any retry is added"). Verify with a fixture test per failure type. Do NOT add a parse-error retry.
- [x] 24.2 Fill the trigger and condition sections of Classify plans in `crates/core/src/turn.rs` (SilenceReask, Goal and Condition paths) the way Reason plans are filled; refuse to emit a Classify plan that cannot render both (spec "A classification call carries what it judges"). Verify with core tests on each path and a rendered-request test.
- [x] 24.3 Add the render contract test over reason, classify and consolidate reading the rendered request text (spec "Every rendered model request contains its trigger"). Verify it fails on the pre-24.2 code path and passes after.
- [x] 24.4 Make provider code neutral (spec "Provider code is provider-neutral"): remove `ModelRequest.session_id`, the hard-coded `x-opencode-session` and the OpenCode usage structs from `crates/reason/src/model/openai.rs`; add generic `extra_headers` with literal / env / `{conversation_id}` values and the protocol-header guard; add configurable usage-field JSON paths with OpenAI-compatible defaults and `unknown` when absent; add a source scan test for provider names. Verify with mock-provider tests for each value kind, the guard, default and configured usage paths, and the scan.
- [x] 24.5 Add per-purpose output limits per model profile with the configurable parameter name (default `max_completion_tokens`, product default 16,384), runaway recording per purpose and the "reasoning used the whole limit" error (spec "Every call has an output limit per purpose"). Verify with mock-provider tests for a normal call, a call at the limit, and an empty call at the limit.
- [x] 24.6 Add the Lab render dry run that gates every `--live` run and the benchmark model profile for `deepseek-v4.1-flash` at `high` (specs "A render dry run gates every live run", "Benchmark model profiles set measured output limits"); make the `lab-memory-comparison` metering proxy take header names, session values, limits, parameter name and usage paths only from run configuration. Verify with a fixture where an empty trigger blocks the live run.
- [x] 24.7 Verify `cargo fmt --check` on changed crates (not `sdk/`), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `openspec validate` for `core-and-lab`, `waist-valve-and-bodies`, `lab-datasets` and `lab-memory-comparison` (strict), and GitNexus detect-changes.
