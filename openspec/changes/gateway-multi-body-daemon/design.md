# Design: Multi-Body Gateway Daemon, Dynamic Place Routing, and Bidirectional SDKs

## Context

`monoagens` runs external channel plugins (`[[bodies]]` in `monoagens.toml`) over Waist-2 NDJSON (`stdio` or Unix domain sockets). Messaging platforms like WhatsApp and Telegram continuously encounter new group and DM `PlaceId`s at runtime (`wa:group:...@g.us`, `tg:chat:...`) that cannot be enumerated upfront in `body/discover`.

## Key Decisions

1. **Dynamic Place Ownership on `percept/emit` (`crates/gateway/src/waist2.rs`)**
   - When `BodyRouter::handle_percept_emit` validates a `Percept` from an authenticated body (`Some(bid)`), it acquires `self.inner` before running seams or submitting to `self.ingress` and checks `st.place_to_body.get(&percept.place)`:
     - `None`: inserts `(percept.place.clone(), bid.clone())` into `st.place_to_body` and appends the place to `BodyConnection.places` if absent.
     - `Some(owner)` where `owner != bid`: logs `LogLevel::Warn` on `"gateway.waist2"`, leaves `place_to_body` untouched, skips `self.ingress`, and returns a JSON-RPC `RPC_INVALID_PARAMS` error containing `ERR_PLACE_OWNED_BY_OTHER_BODY`.
   - `handle_body_disconnect` already retains only `owner != body_id` in `place_to_body`, automatically releasing all static and dynamic places when a body disconnects so another body can reclaim them later.
   - `BodyRouter::connected_bodies()` and `BodyRouter::owned_places(&BodyId)` expose read-only routing state for daemon diagnostics and tests.

2. **Crash-Isolated Multi-Body Startup (`crates/gateway/src/runtime.rs`)**
   - `GatewayHandle::connect_configured_bodies(&self, bodies: &[BodyPluginConfig]) -> Vec<BodyConnectOutcome>` iterates `[[bodies]]` entries sequentially, calling `connect_stdio_body` or `connect_unix_body` per entry.
   - Any `Err(err)` is logged as `LogLevel::Warn` on `"gateway.body"` with `body.id` and recorded in `BodyConnectOutcome`, never aborting startup or preventing sibling bodies from connecting (`ARCHITECTURE.md` plugin isolation invariant).

3. **`monoagens serve` & Modular CLI (`crates/cli/src/commands/`)**
   - Split command implementations out of monolithic `crates/cli/src/lib.rs` into `crates/cli/src/commands/` modules while preserving existing subcommand behavior.
   - Add `monoagens serve` to load `monoagens.toml` + `secrets.toml`, start `GatewayRuntime`, invoke `connect_configured_bodies`, stay alive until `SIGINT`/`SIGTERM`, and call `runtime.shutdown().await`.

4. **Bidirectional Waist-2 Clients (`sdk/typescript`, `sdk/python`, `sdk/rust`)**
   - Extend each SDK with a bidirectional NDJSON transport loop that correlates outbound `percept/emit` / `action/receipt` / `body/heartbeat` JSON-RPC requests with responses while concurrently dispatching inbound `body/discover`, `action/execute`, `action/cancel`, `state/query`, and `body/shutdown` requests from the Gateway.
