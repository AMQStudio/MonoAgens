# Specification — `runtime-hotfixes`

## ADDED Requirements

### Requirement: Model Changes Apply Live in a TUI-Hosted Gateway
When the hosting TUI applies a model, provider or purpose change through `config.apply`, the gateway SHALL rebuild the affected providers before the next model request, exactly as `monoagens serve` does.

#### Scenario: Switch model in the hosting TUI
- **WHEN** the owner selects another model in `/model` in a TUI that hosts the gateway
- **THEN** the next model request uses the new model id and the log has no "provider rebuild hook not wired" line.

### Requirement: Telegram Body Handshakes Immediately and Honours Rate Limits
The Telegram body SHALL answer `body.discover` before any Telegram network call, and SHALL wait at least the `retry_after` seconds Telegram returns before the next `getUpdates` or `getMe`.

#### Scenario: Rate-limited bot token
- **WHEN** Telegram answers `getUpdates` with 429 and `retry_after = 14`
- **THEN** the next `getUpdates` is sent no sooner than 14 s later and the gateway handshake has already succeeded.

### Requirement: Intentional Plugin Stops Are Not Logged as Crashes
A stop or restart requested by the owner or by `config.apply` SHALL be logged once at info level as stopped, and SHALL NOT produce an error-level "disconnected/crashed" entry.

#### Scenario: Restart from the plugin page
- **WHEN** the owner presses Restart for `whatsapp`
- **THEN** the log has one "stopping" and one "starting" info entry and no error entry for that plugin.
