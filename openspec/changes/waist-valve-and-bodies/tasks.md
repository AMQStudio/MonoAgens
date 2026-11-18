# Tasks — `waist-valve-and-bodies` (Phases 0–3)

Checklist implementing `monoagens.body.v1`, `monoagens-valve`, `monoagens-gateway`, reference
bodies/bridges, `monoagens` CLI, and `monoagens-conformance` TCK in lockstep across Phases 0–3
with [`core-and-lab`](../core-and-lab/tasks.md). Every task group carries its own verification.

## 1. Phase 0 — Waist 2 OpenRPC contract & SDK skeleton (`waist2-protocol`)

- [x] 1.1 Query `context7` MCP (`resolve-library-id` -> `query-docs`) for latest stable `schemars`, `serde_json`, `jsonrpsee`, `cedar-policy`/`symcc`, `ort`, TypeScript, and Python SDK docs, then author canonical `OpenRPC 1.4.1` and `JSON Schema Draft 2020-12` definitions using latest stable `schemars` for all twelve stateless `monoagens.body.v1` methods (`body/discover`, `body/heartbeat`, `body/shutdown`, `percept/emit`, `action/execute`, `action/cancel`, `action/receipt`, `approval/resolve`, `question/answer`, `state/query`, `blob/put`, `blob/get`); verify schema validation tests pass cleanly.
- [x] 1.2 Implement NDJSON `stdio` and Unix domain socket RPC envelopes (`idempotency_key`, `protocol_version`, reverse-DNS `_meta`, `traceparent`), Option A `Receipt` extensions (`termination_reason`, `progress`, `delivered_units`), and typed errors `-32001`..`-32006`; verify round-trip serialization, unknown-key ignore, `(M_msg == M_gw && m_msg <= m_gw)` compatibility, and rejection tests for `-32004`/`-32006`.
- [x] 1.3 Generate thin Rust, TypeScript, and Python SDK shims from the OpenRPC schema encapsulating framing, `idempotency_key` dedup, crash journal (`state/query`), SHA-256 blob ops, and heartbeats under `<= 200 LOC` adapter code; verify each SDK compiles and lints cleanly.

## 2. Phase 0–1 — Conformance TCK & Lab codec integration (`conformance-tck`)

- [x] 2.1 Implement standalone `crates/conformance` binary supporting `monoagens-conformance body --level must|should|may --json-report compatibility.json`; verify clean CLI invocation and deterministic report schema.
- [x] 2.2 Implement fault-injection Body TCK tests `B-MUST-01` (`body/discover`), `B-MUST-02` (3x `idempotency_key` replay -> `== 1` exec), `B-MUST-03` (`SIGKILL` + `state/query <= 5000 ms`), `B-MUST-04` (audience/place isolation), `B-MUST-05` (`held` round-trip), `B-MUST-06` (SHA-256 `ERR_HASH_MISMATCH`), `B-MUST-07` (`ERR_UNKNOWN_CAPABILITY`), plus `B-SHOULD-01..03`; verify negative tests fail intentionally broken mock bodies.
- [x] 2.3 Implement Core TCK harness verifying `C-MUST-01` (`Delta_NI = 0.0`), `C-MUST-02` (`hard_forget` zero resurrection), `C-MUST-03` (seeded bit-identical replay), and `C-MUST-04` (10k-tick neutral stability) against `core-and-lab` TCK hooks; verify passing suite.
- [x] 2.4 Wire `TestBody` in `monoagens-lab` strictly through the real `monoagens.body.v1` JSON-RPC codec (`0` in-memory bypass structs); verify `monoagens-lab` dev tests pass with zero codec regressions.

## 3. Phase 1 — Cedar Valve & static owner proofs (`valve`)

- [x] 3.1 Implement `crates/valve` embedding latest stable `cedar-policy` and `cedar-policy-symcc` (`LocalSolver::cvc5()`) verified via `context7` with pure `(log_seq, t_now)` snapshot evaluation across Inbound, Context, Action, and memory-write doors; verify first-and-last pass ordering where secondary hooks can only narrow access.
- [x] 3.2 Run `cedar-policy-symcc` static check on policy load proving `principal != Owner` can never trigger `identity_changing` or `SelfModify` (`UNSAT`, zero counterexamples), and implement dynamic attested role compilation (`room_presence_ttl_s = 300 s`, `physical_presence_ttl_s = 15 s`, `physical_room_radius_m = 4.0 m`); verify text `"I am admin"` spoof attempts remain denied and `< 50 μs` p99 `criterion` latency benchmark holds.
- [x] 3.3 Implement Absolute Line 2 (`VisibleScope` projection enforcement + `DeclassToken` validation) and budget/chain-brake guards (exempting genuine single-shot promises, blocking `RecurrenceSpec` monitors when daily budget is exhausted); verify toggleability and `door_verdict` logging tests.
- [x] 3.4 Attach typed read-only `ValveSelfSnapshot` (`valve_mode`, `quiet_hours`, `PendingApproval`, `C_daily`, `taint_blocking`) inside the `Context` door and stamp canonical `door_verdict`/`Receipt` hold/refusal reason tags synchronized exhaustively with `core-and-lab` `[SELF_GUIDE]`; verify toggle-on/toggle-off zero-drift parity tests pass.

## 4. Phase 1–2 — Gateway runtime, `κ(b, a)` identity, custody & `EmbeddingGemma 2` (`gateway`)

- [x] 4.1 Implement `crates/gateway` over `tokio` with one dedicated `Core` owner task driving synchronous `Core::step`/`Core::open`, `next_due()` timers, and `hook_timeout_ms`/`action_timeout_ms` bounded tasks; verify crashing plugins resolve pending acts once as `unknown` without stalling sibling bodies.
- [x] 4.2 Implement `monoagens.toml` (`version = 1`, `toml_edit`), `secrets.toml`/env loader rejecting inline keys, stateless OpenAI-compatible HTTP (`store: false`, zero key leaks on error) + `scripted` adapters, and XDG/`--config`/`MONOAGENS_HOME` resolution; verify startup error tests.
- [x] 4.3 Implement canonical `κ(b, a) = u32_le(|b|) || b || u32_le(|a|) || a` resolution (`per_` hash, `heuristic_merge_enabled = false`, `T_μ = +∞`) and append-only dual-nonce bilateral challenge linking (`actor_link_challenge`, `actor_linked`, `actor_unlinked`, `link_challenge_ttl_s = 600 s`, `retroactive_merge = false`, `owner_escalation_guard = true`); verify colon collision `proptest` fuzzing and replay tests.
- [x] 4.4 Implement inline `<= 256 KiB` strip + out-of-band attachment store using scope-keyed `BLAKE3` (latest stable `blake3`, AEAD-at-rest `attachments/<scope_id>/<prefix>/<hex>`) and SQLite `attachment_refs` with synchronous `hard_forget` cascade wipe; verify zero cross-tenant oracle and zero residual bytes after `hard_forget`.
- [x] 4.5 Integrate unified Apache 2.0 `EmbeddingGemma 2` (`256d` MRL, `8K` ctx) in Gateway pre-queue behind one trait supporting native `ort`/`LiteRT` or local stdio/HTTP sidecar execution with `(embed_model_ver, dim_d, prompt_regime_id)` tagging, `dialogue_act_gap` scoring, hybrid `Tool RAG` (`R-38`) capability schema indexing for `50–200+` tools, and graceful degradation to vector-less percepts/BM25 tool selection on outage; verify offline/timeout fallback and `N_caps` scaling tests.

## 5. Phase 1–2 — Canonical CLI & MCP reference bodies (`bodies`, `cli`)

- [x] 5.1 Implement `body-cli` (terminal place, local owner account, `chat.speak`, honest `1 s` `working_cue`, `10 s` `progress_note`, `d_art = 0 ms`, echo correlation) and pass `monoagens-conformance body --level must`.
- [x] 5.2 Implement `body-mcp-bridge` wrapping `MCP 2026-07-28` (`2025-11-25` fallback) into virtual `mcp:<server_id>` places (`input_required -> held`, bounded task poll -> single receipt, `capability_schema_hash` diff updates); verify against mock MCP server and pass `B-MUST-01..07`.
- [x] 5.3 Implement split-bubble `pid_1..pid_M` delivery and mid-stream barge-in cancellation (`delivered_units` accounting, `R-09` echo matching) in generic Chat Body helper; verify 3-bubble abort unit tests.
- [x] 5.4 Implement `monoagens` CLI (`chat` with first-run config bootstrap & non-zero exit when unconfigured, `log --explain --json`, offline `replay`, `approvals` rendering canonical `H_act`, `link`, `conformance`, `inspector` for `traceparent` + active Cedar rules); verify integration tests for every subcommand.

## 6. Phase 2 — Approvals, taint quarantine, & quiet-hours digests (`valve`)

- [x] 6.1 Implement `R_0`..`R_3` classification, non-retrying `PendingApproval` (`max_retry_on_held = 0`), canonical `H_act = BLAKE3("monoagens.act.v1" || canonical_json(...))`, `state_epoch` + `approval_ttl_s` (`900 s` / `30 s`) verification, and sliding caps (`max_prompts_per_hour = 6`, `max_pending_queue = 3`, `dedup_cooldown_s = 600 s`); verify Loopjacking/TOCTOU mutation rejection and filler-burst exhaustion tests.
- [x] 6.2 Enforce 3-door taint quarantine (`τ_turn < Trusted` blocks/holds `>= R_2`, cross-audience, `procedure` write, and `identity_changing` unless `ApprovalGrant(H_act)` or `DeclassToken`) synced with compaction/write taint join; verify zero unauthorized `R_2` execution under prompt-injection torture suite.
- [x] 6.3 Implement recipient-local quiet hours (`Q_start..Q_end = 22:00..07:00`), proactive cap (`C_daily = 8`), emergency cutoffs (`U_quiet`/`U_cap = 0.90`), and `3×/day` digest batching emitting `held` receipts; verify DST-safe replay and zero false quiet-hour alerts.

## 7. Phase 2–3 — Embodied/ecosystem bridges & full interop matrix (`bodies`, `conformance-tck`)

- [ ] 7.1 Implement `plugin-a2a-bridge` (`A2A v1.0.1`), `plugin-ros2-bridge` (`ROS 2 Lyrical Luth`), and Phase-3 `body-sim-actuator` strictly via Waist-2 SDKs; verify each passes `B-MUST-01..07` cleanly.
- [x] 7.2 Implement embodied voice pipeline (`Silero VAD` / `WebRTC VAD` + AEC + causal z-scored `f_modal`) enforcing `< 50 ms` TTS mute (`vad_prob >= 0.70`, `τ_barge >= 120 ms`, `< 30 ms` local mute) and emitting single Option A `Receipt(failed, preempted_external, progress, delivered_units)`; verify `80 ms` backchannel pass vs `200 ms` barge-in preemption test.
- [x] 7.3 Author developer documentation (`docs/waist2.md`, `docs/valve.md`, `docs/conformance.md`) and verify a clean-room external body adapter requires `<= 200 LOC` over the generated SDK and passes `B-MUST-01..07`.

## 8. Integration & release gates

- [x] 8.1 Verify `openspec validate waist-valve-and-bodies --strict` and `openspec validate core-and-lab --strict` exit `0` with zero warnings.
- [ ] 8.2 Execute full Phase-3 `>= 3` Bodies (`body-cli`, `body-mcp-bridge`, `body-sim-actuator`) × `>= 2` Cores conformance matrix at `100% MUST` (`Delta_NI = 0.0`) with `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` passing within latency budgets (`< 50 μs` Valve p99, `< 50 ms` barge-in, `< 15 s` gateway integration suite).
- [ ] 8.3 Run live paid LLM + hardware latency calibration only upon explicit owner request (`S_digest`, `max_prompts_per_hour`, `C_daily`, `L_soft`, local ARM `EmbeddingGemma 2` profiling).

## 9. Reopened — prior configuration and replay (owner review 2026-11-17, D11)

- [x] 9.1 Add optional `[soul] prior` to `monoagens.toml` parsing; `monoagens chat` and the gateway load it through the shared `core-and-lab` prior loader (task 23.2) instead of `Prior::default()` (`crates/cli/src/lib.rs` chat boot), failing start-up on unknown key, out-of-range value or unreadable path.
- [x] 9.2 Make `monoagens replay` take the prior from `prior_loaded` events (task 23.4) and drop its `Prior::default()` use; add an integration test for the two-session prior change scenario.
- [x] 9.3 Verify `cargo test -p monoagens-cli -p monoagens-gateway`, clippy `-D warnings`, and `openspec validate waist-valve-and-bodies --strict`.

## 10. Reopened — provider-neutral headers and output limits (owner review 2026-11-18, core-and-lab Decision 21)

- [x] 10.1 Gateway: send the configured extra headers with `{conversation_id}` filled by a stable id per conversation or place, and the per-purpose output limit of the configured model profile, on every model call; config schema and validation for both (protocol-header guard, default 16,384) (spec "The gateway sends configured headers and output limits on every model call"). Verify with gateway tests using a mock provider: two calls of one chat share the header value, two chats differ, no value is logged, limits per purpose reach the request.

## Workflow follow-up

- Implement Phases 0–3 in lockstep with `core-and-lab` and archive both changes cleanly into `openspec/specs/` after review and conformance verification.
