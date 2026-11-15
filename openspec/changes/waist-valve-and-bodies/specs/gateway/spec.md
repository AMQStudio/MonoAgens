# Spec Delta

## Purpose

The `monoagens-gateway` is the host runtime bridging external bodies and Core without platform
logic inside Core. It drives `Core::step` asynchronously over `tokio`, enforces canonical `κ(b, a)`
identity isolation (`R-44`), manages scope-keyed `BLAKE3` attachment custody (`R-37`), runs
pre-queue `EmbeddingGemma 2` (`256d` MRL, Apache 2.0), and reports honest in-flight lifecycle cues (`R-47`).

## ADDED Requirements

### Requirement: Single-writer async runtime drives synchronous Core
The gateway SHALL run on `tokio` with one dedicated owner task calling synchronous `Core::step` and
`Core::open`, spawning bounded-timeout (`hook_timeout_ms`, `action_timeout_ms`) async tasks for
models, body RPCs, and `next_due()` timers. Crashing or hanging external plugins SHALL settle
pending actions once without halting the loop.

#### Scenario: Crashing plugin yields unknown receipt while siblings run
- **WHEN** an external body crashes or hangs past `action_timeout_ms` mid-action
- **THEN** pending actions settle once as `unknown`, the failure is logged for the owner, and unaffected bodies continue operating

#### Scenario: Slow model wake does not starve urgent percept ingress
- **WHEN** an external LLM wake takes several seconds
- **THEN** incoming percepts continue queuing normally without blocking Gateway IO tasks

### Requirement: Canonical cross-body identity hashing prevents delimiter collisions
The gateway SHALL resolve `(body_id, account_id)` to a configured person or deterministic
`per_<hex32(BLAKE3("monoagens.actor.v1" || κ(b, a)))>` via length-prefixed `κ(b, a) =
u32_le(|b|) || b || u32_le(|a|) || a` with `heuristic_merge_enabled = false`. Cross-body account
linking SHALL proceed only via config or append-only bilateral challenges logged without raw
secrets.

#### Scenario: Colons in platform identifiers never cause hash collisions
- **WHEN** Matrix MXIDs or DIDs containing colons arrive across multiple bodies
- **THEN** length-prefixed `κ(b, a)` produces collision-free deterministic `per_` actor IDs across restarts

#### Scenario: Bilateral challenge links actors prospectively without retro-leaks
- **WHEN** two Tier-2 accounts verify dual-nonce challenges within `link_challenge_ttl_s`
- **THEN** Gateway logs `actor_linked` prospectively without exposing prior private history when `retroactive_merge` is false

### Requirement: Keyed BLAKE3 Attachment Store isolates media and cascades erasure
The gateway SHALL persist stripped inline and out-of-band blobs inside an AEAD-at-rest scope-keyed
`BLAKE3` store (`attachments/<scope_id>/<prefix>/<hex>`) tracked in SQLite `attachment_refs`,
passing Core only opaque `AttachmentHandle`s and numeric `f_modal` slots. Authorized `hard_forget`
commands SHALL wipe zero-ref files and decoded caches synchronously.

#### Scenario: Hard-forget scrubs media without leaking across tenant scopes
- **WHEN** `scope_A` and `scope_B` store identical bytes and `hard_forget` erases `scope_A`'s actor
- **THEN** `scope_A`'s file and `attachment_refs` rows are overwritten and unlinked while `scope_B` remains intact

### Requirement: Pre-queue unified 256d EmbeddingGemma 2 runs locally or remotely and degrades cleanly
Before enqueueing percepts, the gateway SHALL compute unified Apache 2.0 `EmbeddingGemma 2`
(`256d` MRL, `8K` ctx) vectors—via native `ort`/`LiteRT`, local sidecar, or remote endpoint—across
text, image, and `audio_voice` tagged with `(embed_model_ver, dim_d, prompt_regime_id)` plus
`dialogue_act_gap` scores. Embedder outages SHALL allow vector-less percepts to enter cleanly.

#### Scenario: Embedder outage leaves cognitive loop operational
- **WHEN** the embedding provider times out or returns an error during percept ingestion
- **THEN** the gateway records the diagnostic error and forwards the percept without a vector

### Requirement: Gateway indexes capability catalogs for Tool RAG progressive disclosure
The gateway SHALL maintain a hybrid BM25 and `256d` vector index over registered body and MCP
`CapabilityDescriptor` metadata stamped with `capability_schema_hash`, supporting Core's bounded
`Tool RAG` disclosure (`R-38`) and falling back cleanly to lexical BM25 matching when embeddings are
unavailable.

#### Scenario: Large MCP capability catalog indexes for progressive disclosure
- **WHEN** connected bodies register `50` or more capabilities via `body/discover`
- **THEN** the gateway updates its hybrid BM25/`256d` capability index stamped with `capability_schema_hash` so Core selects bounded `Tool RAG` subsets without shipping every full tool schema on every turn

### Requirement: Versioned config, isolated secrets, and honest lifecycle cues
The gateway SHALL validate versioned `monoagens.toml`, reject newer config versions or inline API
keys (`secrets.toml` / env vars only), invoke models via `scripted` or stateless (`store: false`)
OpenAI-compatible adapters redacting keys in logs, and emit honest `R-47` `working_cue` (`1 s`)
and `progress_note` (`10 s`) notifications.

#### Scenario: Inline key halts startup and genuine waits emit honest cues
- **WHEN** `monoagens.toml` contains an inline secret key or an action genuinely runs past `10 s`
- **THEN** inline secrets abort startup cleanly and genuine waits emit `working_cue` at `1 s` and `progress_note` at `10 s`
