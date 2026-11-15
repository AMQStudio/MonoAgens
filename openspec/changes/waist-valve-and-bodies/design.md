# Design: `waist-valve-and-bodies`

## Context

See [`proposal.md`](proposal.md) for motivation and scope. `ARCHITECTURE.md` fixes Waist 1 (`Core::step`, the six domain facts, three Doors, and the two Absolute Lines) and Waist 2 (actors, places, audiences, percepts, actions, 5-outcome receipts, lifecycle events, answers, and commands). Meanwhile [`core-and-lab`](../core-and-lab/design.md) specifies the internal Core algorithms, SQLite/HNSW store, and cognitive Lab across 22 mechanisms.

Constraints shaping this design:
- `ARCHITECTURE.md` invariants are read-only and immutable; Option A adds only optional `Receipt` metadata (`termination_reason`, `progress`, `delivered_units`) leaving the five core outcomes untouched.
- `monoagens-core` depends solely on `monoagens-protocol` and `serde`. All IO, async orchestration, Cedar policy evaluation, attachment encryption, `EmbeddingGemma 2` inference, JSON-RPC 2.0 transport, and platform adapters stay in outer crates (`valve`, `gateway`, `bodies`, `cli`, `conformance`).
- Target the **latest stable compatible versions** of Rust (`rustup` stable), TypeScript/Python runtimes, and external dependencies resolved into lockfiles at implementation time, checking upstream docs via **`context7` MCP** (`resolve-library-id` -> `query-docs`) before coding.
- No Rust source code exists yet (`cargo` installed via `rustup` during Phase 0).

## Goals / Non-Goals

**Goals:**
- Freeze `monoagens.body.v1` (`<= 12` OpenRPC `1.4.1` / JSON Schema `2020-12` methods) in Phase 0 so `TestBody`, `body-cli`, `body-mcp-bridge`, and third-party adapters share one black-box wire codec from day one (`R-50`).
- Guarantee deterministic replayable authorization (`cedar-policy` + `cedar-policy-symcc`) enforcing Absolute Lines 1 & 2, non-retrying `PendingApproval` + `H_act`, and 3-door taint blocking without word blocklists (`R-41`–`R-43`).
- Isolate raw multimodal bytes and identity-resolution complexity outside Core via keyed-`BLAKE3` attachment custody, `κ(b, a)` `per_` actor hashing, and pre-queue `EmbeddingGemma 2` (`256d` MRL, Apache 2.0) encoding (`A-15`, `R-37`, `R-44`).
- Decouple public black-box compliance (`monoagens-conformance` TCK + `< 200 LOC` SDK codegen) from `monoagens-lab` cognitive ablations (`R-50`).

**Non-Goals:**
- Implementing Core cognitive internals (gate signals, ACT-R memory, sleep consolidation, bounded learning) or cognitive Lab metrics — handled by [`core-and-lab`](../core-and-lab/design.md).
- Executing live paid LLM evaluations automatically in default `cargo test` runs.
- Heuristic cross-body identity guessing (explicitly forbidden: `T_μ = +∞`, `R-44`).

## Decisions

### D1. Downward-only workspace layout paired with `core-and-lab`

```
crates/
  protocol/     monoagens-protocol     Waist 1 & Waist 2 OpenRPC/serde types, Option A Receipt
  core/         monoagens-core         (core-and-lab) sans-IO state machine
  store/        monoagens-store        (core-and-lab) SQLite hot/sealed, usearch, blake3 snapshots
  valve/        monoagens-valve        cedar-policy + symcc (latest stable), H_act, PendingApproval, quiet hours
  gateway/      monoagens-gateway      tokio loop, κ(b,a) identity, BLAKE3 attachments, EmbeddingGemma 2 256d
  cli/          monoagens              chat, log, replay, approvals, link, conformance, inspector
  conformance/  monoagens-conformance  B-MUST-01..07, B-SHOULD-01..03, C-MUST-01..04, SDK codegen
  lab/          monoagens-lab          (core-and-lab) routes TestBody through Waist-2 codec
bodies/
  body-cli/         terminal place + chat.speak + R-47 honest progress cues
  body-mcp-bridge/  MCP 2026-07-28 (2025-11-25 fallback) adapter -> mcp:<server_id>
  plugin-a2a-bridge/ A2A v1.0.1 adapter (Phase 2–3)
  plugin-ros2-bridge/ ROS 2 Lyrical Luth adapter (Phase 2–3)
  body-sim-actuator/ embodied physical/voice test body with <50 ms barge-in (Phase 3)
```

*Alternative rejected:* Putting Cedar, attachment storage, or JSON-RPC inside `monoagens-core`. Rejected because `core` must remain `protocol + serde` only and testable in sub-millisecond in-memory loops.

### D2. Waist 2 (`monoagens.body.v1`) OpenRPC 1.4.1 & 12-method wire contract (`R-45`, `R-50`)

- **Transport & schema:** NDJSON over `stdio` or Unix domain socket generated from single-source `OpenRPC 1.4.1` + `JSON Schema Draft 2020-12` using latest stable `schemars`, `serde_json`, and `jsonrpsee` (verified via Context7).
- **Twelve stateless methods (`<= 12`):** `body/discover`, `body/heartbeat`, `body/shutdown`, `percept/emit`, `action/execute`, `action/cancel`, `action/receipt`, `approval/resolve`, `question/answer`, `state/query`, `blob/put`, `blob/get`.
- **Envelopes & compatibility:** Carry `idempotency_key`, `protocol_version`, reverse-DNS `_meta` extensions, and `traceparent`. Accept `(M_msg == M_gw) && (m_msg <= m_gw)` while ignoring unknown keys outside strict `args`; reject errors cleanly with `-32001 ERR_UNKNOWN_CAPABILITY`, `-32002 ERR_DUPLICATE_IDEMPOTENCY`, `-32003 ERR_INVALID_RECEIPT_TRANSITION`, `-32004 ERR_UNSUPPORTED_VERSION`, `-32005 ERR_HASH_MISMATCH`, `-32006 ERR_UNDECLARED_AUDIENCE`.
- **Single-receipt FSM:** `T_act = min(action_timeout_ms, body_limit)`. Emits at most one terminal/held receipt per `action_id`; timeout or crash resolves to `unknown`; duplicates drop cleanly (`receipt_cardinality == 1`).

*Alternative rejected:* Requiring bodies to be raw `MCP 2026-07-28` servers (`R-45`). Rejected because stateless MCP disallows unsolicited server-initiated percept push, lacks native place/audience witnesses, collapses `refused`/`failed`/`unknown`, and treats `ToolAnnotations` as unverified hints; MCP is wrapped via `body-mcp-bridge` instead.

### D3. Canonical cross-body identity `κ(b, a)` & bilateral linking (`R-44`)

- Length-prefixed encoding `κ(b, a) = u32_le(|b|) || b || u32_le(|a|) || a` eliminates colon collisions in Matrix MXIDs (`@alice:matrix.org`), DIDs (`did:web:...`), and WhatsApp JIDs (`PN-JID`/`LID-JID`).
- Resolution order: `[[person]]` owner config (`C_person`) -> non-revoked log link (`E_link`: `actor_linked` / `actor_unlinked`) -> deterministic `"per_" || hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))`.
- `heuristic_merge_enabled = false` (`T_μ = +∞` from Fellegi–Sunter false-merge cost `C_I -> ∞` under Absolute Lines 1 & 2). Tier-2 platform IDs link only via out-of-band dual-nonce bilateral challenge (`link_challenge_ttl_s = 600 s`, `link_max_attempts = 3`, `>= 128-bit` CSPRNG / `>= 40-bit` SAS) storing nonce hashes (never raw secrets) and enforcing `owner_escalation_guard = true` + `retroactive_merge = false` default.

### D4. Keyed-`BLAKE3` attachment custody & unified `256d` `EmbeddingGemma 2` (`A-15`, `R-37`)

- Payloads `<= B_inline = 256 KiB` may arrive inline on `percept/emit` and are immediately stripped before Core queueing; larger media transfer via `blob/put` / `blob/get` (HTTP/UDS/FD pass, `60 s` staging TTL, SHA-256 wire integrity check `-32005 ERR_HASH_MISMATCH`).
- Gateway persists bytes in AEAD-at-rest `attachments/<scope_id>/<prefix>/<hex>` keyed with `blake3::keyed_hash(&scope_salt[scope_id], bytes)` (latest stable `blake3`) and indexed in SQLite `attachment_refs`. Core sees only opaque `AttachmentHandle`, numeric `f_modal` slots (zero emotion labels), and `256d` vectors. Authorized `hard_forget` wipes unreferenced blobs and decoded frame/STT caches before checkpointing.
- Primary Gateway embedder is `google/embeddinggemma-2` (`arXiv:2610.37364`, Apache 2.0, `256d` MRL, `8K` ctx) behind one Gateway embedder port supporting native `ort` (`ONNX Runtime`) / `LiteRT`, local stdio/HTTP sidecar, or remote endpoints with mandatory task instruction prefixes, strict `(embed_model_ver, dim_d, prompt_regime_id)` tagging, `A-14` `dialogue_act_gap` scoring, and hybrid indexing of registered body/MCP `CapabilityDescriptor`s so Core's `Tool RAG` (`k_cap = 8`, `R-38`) scales across `50–200+` external tools without prompt bloat; outages degrade cleanly to vector-less percepts.

### D5. Embedded `cedar-policy` + `cedar-policy-symcc` Valve (`R-41`)

- Evaluates deterministically over `(log_seq, t_now)` snapshots (zero LLM inference, zero live HTTP/DNS) and runs **first and last** in Inbound, Context, Action, and memory-write doors so secondary plugins can only narrow access.
- Derives ephemeral room/co-location Cedar parents strictly from verified `PlacePresence` (`room_presence_ttl_s = 300 s`) or `PhysicalPresence` (`physical_presence_ttl_s = 15 s`, `physical_room_radius_m = 4.0 m`); natural-language self-claims carry zero authority.
- Policy reload invokes latest stable `cedar-policy-symcc` (`CedarSymCompiler::new(LocalSolver::cvc5()?)` over `WellTypedPolicies` and `SymEnv`, verified async against `cvc5` on load) proving UNSAT (`0` counterexamples) that `principal != Owner` can invoke `identity_changing` or `SelfModify` capabilities (`< 50 μs` p99 `criterion` runtime eval target + `proptest` policy/identity invariant fuzzing).

*Alternative rejected:* Embedded Rego (`regorus`, `42–80×` slower, incomplete SMT) or OpenFGA (no static Lean/SMT owner proofs).

### D6. Risk tiers, `PendingApproval` + `H_act`, and 3-door taint (`R-42`, `R-43`)

- Actions classify into `R_0` (read/in-audience allow), `R_1` (reversible/budgeted allow), `R_2` (irreversible write, financial/cred, `DeclassToken`, `procedure` write, tainted tool call -> `PendingApproval`), `R_3` (non-owner `identity_changing` / unauthorized cross-audience / bad `H_act` -> `refused`).
- `PendingApproval` has `max_retry_on_held = 0`, bounded sliding caps (`max_prompts_per_hour = 6`, `max_pending_queue = 3`, `dedup_cooldown_s = 600 s`), and cryptographic `H_act = BLAKE3("monoagens.act.v1" || canonical_json(...))` verified against `state_epoch` and `approval_ttl_s` (`900 s` chat / `30 s` embodied).
- Turn taint `τ_turn = min_{m ∈ ContextPlan} ι(m) < Trusted` holds/refuses `>= R_2`, cross-audience, `procedure`-writing, and `identity_changing` actions absent owner `ApprovalGrant(H_act)` or `DeclassToken`.

### D7. Budgets, chain brakes, quiet hours, & batched digests (`R-46`)

- Daily token budgets exempt genuine single-shot fired promises only (never `RecurrenceSpec` monitors) alongside `max_chain_wakes`.
- Recipient-local quiet hours (`Q_start..Q_end = 22:00..07:00`), proactive daily cap (`C_daily = 8`), urgency cutoffs (`U_imm = 0.60`, `U_quiet = U_cap = 0.90`), and `3×/day` digest queues emit explicit `held` receipts (never silent drops).

### D8. Full-duplex voice barge-in & honest `R-47` delivery (`R-37`, `R-47`)

- Audio pipeline uses `Silero VAD` (`WebRTC VAD` fallback) + AEC + causal z-scored prosody `f_modal`; non-AEC speech (`vad_prob >= 0.70`, sustained `>= 120 ms`) mutes local TTS in `< 30 ms` (`< 50 ms` total cutoff) and emits one Option A `Receipt(failed, preempted_external, progress, delivered_units)`.
- Delivery enforces `d_art = 0 ms`, honest `working_cue` (`1000 ms`) & `progress_note` (`10000 ms`), and Chat Body `pid_1..pid_M` chunk tracking for `R-09` echo precision `1.0`.

### D9. Configuration, secrets, and stateless LLM adapters

- `monoagens.toml` (`version = 1`, `toml_edit` safe updates after backup) with secrets strictly in `secrets.toml` (`0600`) or environment variables; startup aborts on inline secrets or newer config versions.
- OpenAI-compatible adapters force stateless calls (`store: false`, no server-side `previous_response_id` state bypassing `ContextPlan`).

### D10. Standalone `monoagens-conformance` TCK & `< 200 LOC` SDKs (`R-50`)

- Splits black-box wire TCK (`B-MUST-01..07`, `B-SHOULD-01..03`, `C-MUST-01..04`, `--level must|should|may`, JSON `compatibility.json`) out of `monoagens-lab`.
- `OpenRPC 1.4.1` codegen produces thin Rust, TypeScript, and Python SDKs (`< 200 LOC` user body adapter) implementing framing, idempotency cache, crash journal, SHA-256 blob ops, and heartbeats.

## Risks / Trade-offs

- [Scope spans Gateway, Cedar Valve, CLI, Bridges, and TCK] -> Phased 0–3 delivery mirrors `core-and-lab` D17 so Phase 0–1 unblocks real CLI & MCP testing without waiting for Phase-3 ROS-2 or simulated robotics.
- [Static `symcc` checks add load-time startup latency on complex Cedar bundles] -> Policy bundles are concise and checked once on load/reload (`core` loop stays `< 50 μs` p99 per turn).
- [`EmbeddingGemma 2` multimodal variants (`439M`–`740M`) on low-power CPU gateways] -> Text-only `271M` / quantized `256d` MRL or remote endpoint options share one interface, and embedder downtime degrades gracefully to vector-less mode.

## Migration Plan (Phases 0–3 Interleaved with `core-and-lab`)

- **Phase 0:** Lock `monoagens.body.v1` OpenRPC `1.4.1` schema (`waist2-protocol`), wire `TestBody` in `monoagens-lab` to the real Waist-2 JSON-RPC codec (`0` shortcut structs), and scaffold `monoagens-conformance`.
- **Phase 1:** Ship thin `monoagens-gateway` (`κ(b, a)`, `BLAKE3` attachment custody, `EmbeddingGemma 2` `256d`), `monoagens-valve` (`cedar-policy` + `symcc`, budget/brake, basic taint), `body-cli`, `body-mcp-bridge` (MCP `2026-07-28`), `monoagens` CLI, and `< 200 LOC` Rust/TS/Python SDKs passing `B-MUST-01..07` alongside neutral-profile `Core::step`.
- **Phase 2:** Activate `PendingApproval` + `H_act`, full 3-door taint quarantine, bilateral `actor_linked` challenges, quiet-hours/digests, honest `R-47` chunked chat delivery, and `monoagens inspector`.
- **Phase 3:** Add `plugin-a2a-bridge`, `plugin-ros2-bridge`, `body-sim-actuator` (`< 50 ms` Silero/VAP audio barge-in), and complete `>= 3` Bodies × `>= 2` Cores `100% MUST` interop matrix (`C-MUST-01..04` + `B-MUST-01..07`).

## Open Questions

Non-blocking calibration constants intentionally deferred to empirical Lab / human UX trials:
- `max_prompts_per_hour` ceiling (starts `6/h`, `[2, 20]`) and `C_daily` proactive notification cap (starts `8/day`, `[3, 25]`).
- Chat Body optional soft chunk length `L_soft` and inter-bubble pace `d_gap` beyond hard platform limits (`d_art = 0 ms` fixed).
- Phase-3 optional remote transport parity (WebSocket / HTTP+JSON / gRPC) beyond core NDJSON `stdio` and Unix domain sockets.
