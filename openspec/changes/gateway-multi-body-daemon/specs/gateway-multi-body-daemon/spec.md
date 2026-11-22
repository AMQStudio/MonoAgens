## ADDED Requirements

### Requirement: Dynamic place ownership registration and anti-hijack guard on percept/emit
`BodyRouter::handle_percept_emit` SHALL dynamically register an unclaimed `percept.place` to the emitting authenticated `BodyId`, SHALL reject cross-body claims with `ERR_PLACE_OWNED_BY_OTHER_BODY` and a `Warn` diagnostic without forwarding to Core, and SHALL release dynamic places on disconnect. `BodyRouter` SHALL expose `connected_bodies()` and `owned_places(&BodyId)`.

#### Scenario: Body discovers empty places and dynamically claims a place via percept/emit
- **WHEN** an authenticated body connects with `places: []` in `body/discover` and emits a valid `percept/emit` for `wa:group:120363@g.us`
- **THEN** `BodyRouter::owned_places` for that body includes `wa:group:120363@g.us` and `BodyRouter::dispatch_action` routes subsequent actions targeting `wa:group:120363@g.us` to that body instead of returning `ERR_UNKNOWN_PLACE`

#### Scenario: Cross-body place hijack attempt is rejected
- **WHEN** Body A owns `wa:group:120363@g.us` and Body B emits `percept/emit` for `wa:group:120363@g.us`
- **THEN** the Gateway returns a JSON-RPC error containing `ERR_PLACE_OWNED_BY_OTHER_BODY`, logs a `Warn` diagnostic on `"gateway.waist2"`, preserves Body A's ownership, and does not submit Body B's percept to Core

#### Scenario: Disconnecting a body releases its dynamically claimed places
- **WHEN** Body A disconnects after claiming `wa:group:120363@g.us`
- **THEN** `wa:group:120363@g.us` is removed from `place_to_body` and a newly connected Body B can claim it via `percept/emit`

### Requirement: Crash-isolated multi-body startup
`GatewayHandle::connect_configured_bodies` SHALL attempt to connect every `BodyPluginConfig` entry (`Stdio` or `UnixSocket`) in order, returning a `Vec<BodyConnectOutcome>` containing each `config_id` and its `Result<BodyId, BodyRpcError>`, and SHALL log a `LogLevel::Warn` diagnostic on `"gateway.body"` when an entry fails without aborting Gateway startup or preventing sibling bodies from connecting.

#### Scenario: One failing configured body does not block a valid sibling body
- **WHEN** `GatewayHandle::connect_configured_bodies` is called with an invalid stdio binary path followed by a valid Unix socket body
- **THEN** the returned outcomes contain `Err` for the invalid entry and `Ok(body_id)` for the valid sibling, a `Warn` diagnostic is recorded on `"gateway.body"`, and the valid body remains connected and operational

### Requirement: Long-lived monoagens serve daemon and modular CLI commands
The CLI SHALL organize subcommand handlers under `crates/cli/src/commands/` and SHALL provide a `monoagens serve` subcommand that starts `GatewayRuntime`, connects all configured `[[bodies]]` via `GatewayHandle::connect_configured_bodies`, runs until `SIGINT`/`SIGTERM`, and shuts down cleanly.

#### Scenario: monoagens serve runs configured bodies until shutdown
- **WHEN** `monoagens serve` starts with a valid configuration containing `[[bodies]]`
- **THEN** it connects configured bodies with crash isolation, keeps the Gateway runtime alive to process percepts and actions, and triggers `GatewayRuntime::shutdown` upon termination signal

### Requirement: Bidirectional Waist-2 NDJSON client support across SDKs
The TypeScript (`sdk/typescript`), Python (`sdk/python`), and Rust (`sdk/rust`) SDKs SHALL each provide a bidirectional Waist-2 NDJSON client capable of handling Gateway-initiated requests (`body/discover`, `action/execute`, `action/cancel`, `state/query`, `body/shutdown`) while concurrently sending body-initiated requests (`percept/emit`, `action/receipt`, `body/heartbeat`) and correlating responses by JSON-RPC `id`.

#### Scenario: SDK client handles inbound action/execute while emitting outbound percept/emit
- **WHEN** an external body built with `sdk/typescript`, `sdk/python`, or `sdk/rust` connects over stdio or Unix socket to the Gateway
- **THEN** it completes `body/discover`, sends `percept/emit` requests awaiting correlated JSON-RPC responses, and handles inbound `action/execute` requests concurrently
