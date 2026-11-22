# Proposal: `gateway-multi-body-daemon`

## Why

Messaging bodies (WhatsApp, Telegram, Discord) discover dynamic rooms and chats at runtime rather than declaring a static list in `body/discover`. Currently, `BodyRouter` only registers places from `body/discover`, lacks a crash-isolated multi-body startup helper, has no long-lived `monoagens serve` daemon subcommand, and lacks a bidirectional Waist-2 client in the TypeScript, Python, and Rust SDKs. This change closes those four gaps cleanly.

## What Changes

- **Dynamic Place Ownership & Anti-Hijack Guard (`crates/gateway/src/waist2.rs`):** Register unclaimed `percept.place` values dynamically onto the emitting authenticated body during `percept/emit`, refuse cross-body place hijacking with `ERR_PLACE_OWNED_BY_OTHER_BODY` and a `Warn` diagnostic, clean up dynamic places on disconnect, and expose `BodyRouter::connected_bodies` and `BodyRouter::owned_places`.
- **Crash-Isolated Multi-Body Startup (`crates/gateway/src/runtime.rs`, `crates/gateway/src/lib.rs`):** Add `BodyConnectOutcome` and `GatewayHandle::connect_configured_bodies(&[BodyPluginConfig])` so each configured `[[bodies]]` stdio or Unix socket plugin is connected independently without failing sibling bodies or aborting Gateway startup (`ARCHITECTURE.md` invariant).
- **Daemon Subcommand & Modular CLI (`crates/cli/src/`):** Refactor the monolithic CLI command handlers into `crates/cli/src/commands/` and add `monoagens serve` as a long-lived multi-body Gateway daemon driving configured bodies until shutdown signal.
- **Bidirectional Waist-2 SDK Clients (`sdk/typescript`, `sdk/python`, `sdk/rust`):** Provide symmetric NDJSON Waist-2 client helpers across all three SDKs supporting `body/discover`, inbound `action/execute` / `action/cancel` / `state/query` / `body/shutdown` handlers, and outbound `percept/emit` / `action/receipt` / `body/heartbeat` calls.

## Capabilities

### New Capabilities
- `gateway-multi-body-daemon`: Dynamic place routing with cross-body anti-hijack enforcement, crash-isolated `[[bodies]]` startup, `monoagens serve` multi-body daemon mode, and bidirectional Waist-2 NDJSON SDK clients.

### Modified Capabilities
- *(none)*

## Impact

- **Gateway (`crates/gateway`):** Updates `waist2.rs`, `runtime.rs`, `lib.rs`, and adds integration tests in `crates/gateway/tests/it/multi_body_tests.rs`.
- **CLI (`crates/cli`):** Modularizes command handlers under `crates/cli/src/commands/` and adds `monoagens serve`.
- **SDKs (`sdk/typescript`, `sdk/python`, `sdk/rust`):** Adds bidirectional Waist-2 NDJSON client transport support without modifying `crates/core`, `crates/protocol`, `crates/valve`, or `ARCHITECTURE.md`.
