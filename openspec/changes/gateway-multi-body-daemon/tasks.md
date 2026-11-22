## 1. Gateway Dynamic Place Routing & Crash-Isolated Multi-Body Startup

- [x] 1.1 Implement dynamic `percept/emit` place ownership registration, `ERR_PLACE_OWNED_BY_OTHER_BODY` anti-hijack guard, disconnect cleanup verification, and `BodyRouter::connected_bodies` / `BodyRouter::owned_places` helpers in `crates/gateway/src/waist2.rs`.
- [x] 1.2 Implement `BodyConnectOutcome` and crash-isolated `GatewayHandle::connect_configured_bodies` in `crates/gateway/src/runtime.rs` and re-export `BodyConnectOutcome` in `crates/gateway/src/lib.rs`.
- [x] 1.3 Add integration tests in `crates/gateway/tests/it/multi_body_tests.rs` covering dynamic place routing, anti-hijack refusal, disconnect cleanup/reclaim, and `connect_configured_bodies` crash isolation.

## 2. Modular CLI & `monoagens serve` Daemon

- [x] 2.1 Refactor `crates/cli/src/lib.rs` command implementations into modular `crates/cli/src/commands/` modules without altering existing subcommand behavior.
- [x] 2.2 Implement `monoagens serve` subcommand wiring `GatewayRuntime::start`, `GatewayHandle::connect_configured_bodies`, signal handling, and graceful `GatewayRuntime::shutdown` plus CLI tests.

## 3. Bidirectional Waist-2 NDJSON SDK Clients

- [x] 3.1 Implement bidirectional Waist-2 NDJSON client transport in `sdk/typescript` with tests.
- [x] 3.2 Implement bidirectional Waist-2 NDJSON client transport in `sdk/python` with tests.
- [x] 3.3 Implement bidirectional Waist-2 NDJSON client transport in `sdk/rust` with tests.
