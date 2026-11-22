# Waist-2 Protocol (`monoagens.body.v1`)

> Normative contract: [`crates/protocol/openrpc/monoagens.body.v1.openrpc.json`](../crates/protocol/openrpc/monoagens.body.v1.openrpc.json) and generated Draft 2020-12 JSON Schemas in [`crates/protocol/jsonschema/`](../crates/protocol/jsonschema/). Byte-identical regeneration and schema validity are checked by [`crates/protocol/tests/it/schema_validation.rs`](../crates/protocol/tests/it/schema_validation.rs).

Waist-2 (`monoagens.body.v1`) is the sole RPC boundary between **Gateway** ([`crates/gateway/`](../crates/gateway/)) and external **Bodies** ([`bodies/`](../bodies/), [`examples/cleanroom-body/`](../examples/cleanroom-body/)). All protocol structs and error codes live in [`crates/protocol/src/waist2.rs`](../crates/protocol/src/waist2.rs) and [`crates/protocol/src/facts.rs`](../crates/protocol/src/facts.rs). Every cross-body invariant follows [`ARCHITECTURE.md`](../ARCHITECTURE.md) and [`RESEARCH.md`](../RESEARCH.md) (`R-36`, `R-37`, `R-38`, `R-42`, `R-43`, `R-44`, `R-49`).

---

## 1. Transport, Framing, and Version Negotiation

- **JSON-RPC 2.0 NDJSON**: One JSON object per newline (`\n`) over `stdio` or a local Unix domain socket (no shared-memory shortcuts). Verified in [`sdk/rust/src/lib.rs`](../sdk/rust/src/lib.rs), [`sdk/typescript/src/index.ts`](../sdk/typescript/src/index.ts), [`sdk/python/monoagens_sdk.py`](../sdk/python/monoagens_sdk.py), and Gateway transport handlers in [`crates/gateway/src/waist2.rs`](../crates/gateway/src/waist2.rs).
- **Headers**: Every request parameter object carries `headers: Waist2Headers`:
  - `idempotency_key: string` — dedup key across retries and crash restarts.
  - `protocol_version: { major: u16, minor: u16 }` — currently `{ major: 1, minor: 0 }` (`CURRENT_PROTOCOL_VERSION`).
  - `traceparent?: string` and `_meta?: Record<string, MetaValue>` — unknown `_meta` keys are ignored (`protocol_tests::unknown_fields_are_ignored`).
- **SemVer Negotiation (`M_msg == M_gw && m_msg <= m_gw`)**:
  Implemented in [`crates/protocol/src/version.rs`](../crates/protocol/src/version.rs) and enforced via `Waist2Headers::validate_version`. Any major mismatch or newer minor version (`m_msg > m_gw`) fails loudly with `-32004 ERR_UNSUPPORTED_VERSION` naming both received and supported versions (`Waist2Error::unsupported_version`).

---

## 2. The Twelve Waist-2 Methods

Schema definitions: [`Waist2Call.schema.json`](../crates/protocol/jsonschema/Waist2Call.schema.json) & [`Waist2Method.schema.json`](../crates/protocol/jsonschema/Waist2Method.schema.json).

| Method | Direction | Purpose | Schema |
|---|---|---|---|
| `body/discover` | Gateway -> Body | Handshake returning `body_id`, `protocol_version`, declared `places`, and deterministically sorted `capabilities` (`sort_capabilities`) with `capability_schema_hash`. | [`BodyDiscoverParams`](../crates/protocol/jsonschema/BodyDiscoverParams.schema.json) / [`BodyDiscoverResult`](../crates/protocol/jsonschema/BodyDiscoverResult.schema.json) |
| `body/heartbeat` | Gateway -> Body | Liveness ping carrying `body_id`, `BodyStatus` (`ready`, `busy`, `degraded`, `stopped`), and monotonic `timestamp_ms`. | [`BodyHeartbeatParams`](../crates/protocol/jsonschema/BodyHeartbeatParams.schema.json) / [`BodyHeartbeatResult`](../crates/protocol/jsonschema/BodyHeartbeatResult.schema.json) |
| `body/shutdown` | Gateway -> Body | Graceful shutdown request before stdio close. | [`BodyShutdownParams`](../crates/protocol/jsonschema/BodyShutdownParams.schema.json) / [`BodyShutdownResult`](../crates/protocol/jsonschema/BodyShutdownResult.schema.json) |
| `percept/emit` | Body -> Gateway | Emits an inbound sensation for a declared `place` and non-empty `audience` (`-32006` if absent/empty), with actor, payload, and optional `FModal` / `AttachmentHandle`s. | [`PerceptEmitParams`](../crates/protocol/jsonschema/PerceptEmitParams.schema.json) / [`PerceptEmitResult`](../crates/protocol/jsonschema/PerceptEmitResult.schema.json) |
| `action/execute` | Gateway -> Body | Dispatches an authorized action (`action_id`, `capability_id`, `places`, non-empty `audience`, `payload`, optional `timeout_ms`, `h_act`). | [`ActionExecuteParams`](../crates/protocol/jsonschema/ActionExecuteParams.schema.json) / [`ActionExecuteResult`](../crates/protocol/jsonschema/ActionExecuteResult.schema.json) |
| `action/cancel` | Gateway -> Body | Preempts an in-flight action; returns `{ cancelled, receipt }` (`TerminationReason::CanceledByCore` / `PreemptedExternal`). | [`ActionCancelParams`](../crates/protocol/jsonschema/ActionCancelParams.schema.json) / [`ActionCancelResult`](../crates/protocol/jsonschema/ActionCancelResult.schema.json) |
| `action/receipt` | Body -> Gateway | Delivers asynchronous terminal/held `Receipt` for an accepted action; recorded once. | [`ActionReceiptParams`](../crates/protocol/jsonschema/ActionReceiptParams.schema.json) / [`ActionReceiptResult`](../crates/protocol/jsonschema/ActionReceiptResult.schema.json) |
| `approval/resolve` | Gateway -> Body / OOB Control -> Gateway | Settles a `Held` action once (`{ action_id, h_act, state_epoch, approved, resolver, resolved_at_ms }`); returns `{ applied }`. Inbound calls to Gateway (`handle_approval_resolve` in [`crates/gateway/src/waist2.rs`](../crates/gateway/src/waist2.rs)) reject in-band chat bodies (`from_in_band_body`) and accept only attested out-of-band control bodies registered via `register_out_of_band_control_body`. | [`ApprovalResolveParams`](../crates/protocol/jsonschema/ApprovalResolveParams.schema.json) / [`ApprovalResolveResult`](../crates/protocol/jsonschema/ApprovalResolveResult.schema.json) |
| `question/answer` | Bidirectional | Exchanges clarifying question answers. | [`QuestionAnswerParams`](../crates/protocol/jsonschema/QuestionAnswerParams.schema.json) / [`QuestionAnswerResult`](../crates/protocol/jsonschema/QuestionAnswerResult.schema.json) |
| `state/query` | Gateway -> Body | Crash-recovery state probe returning `{ action_id, status }` where `status` is `in_flight`, `terminal { receipt }`, or `unknown`. | [`StateQueryParams`](../crates/protocol/jsonschema/StateQueryParams.schema.json) / [`StateQueryResult`](../crates/protocol/jsonschema/StateQueryResult.schema.json) |
| `blob/put` | Bidirectional | Stages out-of-band attachment bytes (`<= 256 KiB` inline limit for messages) verified by SHA-256; returns `{ blob_ref, sha256, expires_at_ms }`. | [`BlobPutParams`](../crates/protocol/jsonschema/BlobPutParams.schema.json) / [`BlobPutResult`](../crates/protocol/jsonschema/BlobPutResult.schema.json) |
| `blob/get` | Bidirectional | Fetches staged blob bytes with optional `expected_sha256` verification (`-32005` on mismatch). | [`BlobGetParams`](../crates/protocol/jsonschema/BlobGetParams.schema.json) / [`BlobGetResult`](../crates/protocol/jsonschema/BlobGetResult.schema.json) |

---

## 3. Idempotency, Crash Recovery, Blobs, and Error Codes

### Dynamic Place Ownership (`percept/emit`)
- Bodies may declare static rooms in `body/discover.places` or return `places: []` for dynamic chat platforms (such as WhatsApp or Telegram).
- When an authenticated connected body sends `percept/emit` for a `place` not already present in `place_to_body`, Gateway dynamically binds `place -> body_id` on the first percept so subsequent `action/execute` and `action/cancel` requests route back to that body.
- Cross-body hijacking is rejected: if a `place` is already bound to a different active `body_id`, another body cannot take over its ownership.
- On disconnect or crash (`handle_body_disconnect` in [`crates/gateway/src/waist2.rs`](../crates/gateway/src/waist2.rs)), Gateway removes all static and dynamic place bindings belonging to that `body_id` (`st.place_to_body.retain(|_, owner| owner != body_id)`).

### Idempotency & `SIGKILL` Crash Recovery
- Bodies cache `(method, headers.idempotency_key)` responses on disk (`MONOAGENS_BODY_STATE_DIR` in [`bodies/body-kit/src/runtime.rs`](../bodies/body-kit/src/runtime.rs) and [`examples/cleanroom-body/src/main.rs`](../examples/cleanroom-body/src/main.rs)) so replaying `action/execute` returns the identical result and receipt without re-executing side effects.
- After an abrupt crash (`SIGKILL`), Gateway restarts the body (`body/discover`) and issues `state/query` within the `5000 ms` TCK recovery bound (`B-MUST-03`), receiving `InFlight`, `Terminal { receipt }`, or `Unknown`. Gateway also records `Unknown` once on timeout/crash and rejects late conflicting receipts (`gateway_tests::crash_and_timeout_settle_once_as_unknown_while_sibling_runs_and_late_receipt_rejected`).

### Inline Media Strip (`<= 256 KiB`) & `blob/put` / `blob/get`
- `MAX_INLINE_ATTACHMENT_BYTES` in [`crates/protocol/src/facts.rs`](../crates/protocol/src/facts.rs) bounds inline payloads at `256 KiB` (`262_144` bytes).
- Larger payloads are stripped before entering `Core` and transferred via `blob/put` / `blob/get` with `60 s` staging TTL (`STAGING_TTL_MS = 60_000` in [`crates/gateway/src/attachments.rs`](../crates/gateway/src/attachments.rs)).
- Both ends verify `byte_len` and SHA-256 hex digest before storing or referencing a blob in a `percept/emit`. Any mismatch fails immediately with `-32005 ERR_HASH_MISMATCH` (`attachment_tests::out_of_band_blob_put_get_hash_mismatch_and_60s_staging_ttl`, `B-MUST-06`).

### Typed Error Codes (`-32001 .. -32006`)

Defined in [`crates/protocol/src/waist2.rs`](../crates/protocol/src/waist2.rs) (`Waist2ErrorCode`):

| Code | Symbol | Meaning |
|---|---|---|
| `-32001` | `ERR_UNKNOWN_CAPABILITY` | Action names a capability not declared by the target body (`B-MUST-07`). |
| `-32002` | `ERR_DUPLICATE_IDEMPOTENCY` | Conflicting duplicate `idempotency_key`. |
| `-32003` | `ERR_INVALID_RECEIPT_TRANSITION` | Malformed parameters or invalid receipt state transition. |
| `-32004` | `ERR_UNSUPPORTED_VERSION` | Protocol version outside `(M_msg == M_gw && m_msg <= m_gw)`. |
| `-32005` | `ERR_HASH_MISMATCH` | SHA-256 or `byte_len` check failure on `blob/put` or `blob/get` (`B-MUST-06`). |
| `-32006` | `ERR_UNDECLARED_AUDIENCE` | `percept/emit` or `action/execute` omits audience or supplies `[]` (`B-SHOULD-03`). |

---

## 4. Option A Receipts & Delivery Timing

Every [`Receipt`](../crates/protocol/src/facts.rs) carries one of the five canonical outcomes (`done`, `refused`, `held`, `failed`, `unknown`) plus Option A additive metadata (`R-36`, `R-37`):
- `termination_reason?: "normal" | "canceled_by_core" | "preempted_external" | "timed_out"`
- `progress?: f32` validated within finite `[0.0, 1.0]` (`Receipt::validate`)
- `delivered_units?: u64`
- `h_act?: HAct`, `reason_code?: string`, `detail?: string`, `timestamp_ms: Time`

Honest delivery timing (`bodies/body-kit/src/cues.rs`): Gateway/body cue trackers enforce `d_art_ms = 0` (zero artificial typing delay), emit a non-verbal `working_cue` after `1000 ms`, and a `progress_note` at `10_000 ms` (`gateway_tests::lifecycle_cues_respect_1s_and_10s_boundaries_with_virtual_clock`, `B-SHOULD-02`).

---

## 5. Gateway Pre-Queue Subsystems (`crates/gateway/`)

Gateway is organized around a single `Core`-owner loop ([`crates/gateway/src/runtime.rs`](../crates/gateway/src/runtime.rs)) backed by three G2 subsystems:

1. **Cross-Body Identity & Bilateral Linking (`4.3`, `R-44`)** — [`crates/gateway/src/identity.rs`](../crates/gateway/src/identity.rs):
   - Precedence: configured `[[person]]` (`C_person`) -> active append-only link (`E_link`) -> length-prefixed `per_<hex32(BLAKE3("monoagens.actor.v1" || kappa(b, a)))>` where `kappa(b, a) = u32_le(|b|) || b || u32_le(|a|) || a`.
   - `HEURISTIC_MERGE_ENABLED = false` (`T_mu = +inf`): heuristic cross-body merging is disabled permanently. Verified by 6 tests in [`crates/gateway/tests/it/identity_tests.rs`](../crates/gateway/tests/it/identity_tests.rs).
2. **Attachment Custody (`4.4`, `R-37`)** — [`crates/gateway/src/attachments.rs`](../crates/gateway/src/attachments.rs):
   - Strips `> 256 KiB` inline media, stages `blob/put`/`blob/get` with `60 s` TTL, stores encrypted blobs at rest (`XChaCha20Poly1305`, scope-bound keys in [`crates/gateway/src/secrets.rs`](../crates/gateway/src/secrets.rs)), indexes SQLite `attachment_refs`, and performs synchronous `hard_forget` zero-residue scrub (`5` tests in [`crates/gateway/tests/it/attachment_tests.rs`](../crates/gateway/tests/it/attachment_tests.rs)).
3. **Pre-Queue Enrichment & Tool RAG (`4.5`, `R-38`)** — [`crates/gateway/src/prequeue.rs`](../crates/gateway/src/prequeue.rs):
   - Tags percept vectors with `embed_model_ver` (`EmbeddingGemma 2` HTTP/stdio sidecars, `HashEmbedder`, or `NoneEmbedder`), computes bounded `dialogue_act_gap` before queue insertion, and indexes discovered tools into hybrid BM25 + `256d` vector Tool RAG with deterministic BM25 fallback (`4` tests in [`crates/gateway/tests/it/prequeue_tool_rag_tests.rs`](../crates/gateway/tests/it/prequeue_tool_rag_tests.rs)).

---

## 6. Minimal Adapter Examples (`sdk/`)

All three SDK shims implement NDJSON framing, version checks, `idempotency_key` deduplication, `state/query`, `-32006` audience guards, and `blob/put`/`blob/get` SHA-256 verification. External bodies adding `SIGKILL` persistence read `MONOAGENS_BODY_STATE_DIR` (see [`examples/cleanroom-body/src/main.rs`](../examples/cleanroom-body/src/main.rs), 181 LOC, passing `B-MUST-01..07`).

### Rust (`sdk/rust` — [`monoagens_body_sdk`](../sdk/rust/src/lib.rs))

```rust
use monoagens_body_sdk::*;

struct EchoBody;
impl BodyHandler for EchoBody {
    fn discover(&mut self, _: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> { /* sorted caps */ todo!() }
    fn execute(&mut self, p: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> { /* check cap/place/aud */ todo!() }
    fn cancel(&mut self, p: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> { todo!() }
    fn resolve_approval(&mut self, p: ApprovalResolveParams) -> Result<ApprovalResolveResult, Waist2Error> { todo!() }
}
// Use BodyRuntime::run_stdio for in-memory NDJSON or add MONOAGENS_BODY_STATE_DIR persistence for SIGKILL durability.
```

### TypeScript (`sdk/typescript` — [`BodyAdapter`](../sdk/typescript/src/index.ts))

```ts
import { BodyAdapter, PROTOCOL_VERSION } from "../sdk/typescript/src/index.js";

const adapter = new BodyAdapter({
  discover: () => ({ body_id: "body:ts-demo", protocol_version: PROTOCOL_VERSION, places: ["demo:0"], capabilities: [] }),
  execute: async (p) => ({ accepted: true }),
  cancel: async () => ({ cancelled: true }),
  resolveApproval: async () => ({ applied: false }),
});
void adapter.runStdio();
```

### Python (`sdk/python` — [`monoagens_sdk.py`](../sdk/python/monoagens_sdk.py))

```python
from monoagens_sdk import BodyAdapter, PROTOCOL_VERSION

class DemoBody:
    def discover(self, params):
        return {"body_id": "body:py-demo", "protocol_version": PROTOCOL_VERSION, "places": ["demo:0"], "capabilities": []}
    def execute(self, params):
        return {"accepted": True}
    def cancel(self, params):
        return {"cancelled": True}
    def resolve_approval(self, params):
        return {"applied": False}

BodyAdapter(DemoBody()).run_stdio()
```
