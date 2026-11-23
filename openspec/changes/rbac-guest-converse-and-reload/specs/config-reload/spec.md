# Spec Delta

## Purpose

Provides live configuration and secrets reload from disk via `config.reload` and `/reload`, and clarifies `/retry` behavior between startup errors and active sessions.

## ADDED Requirements

### Requirement: Live `config.reload` Admin RPC and `/reload` Command
`GatewayOwner` and the admin socket SHALL expose `config.reload` to re-read `monoagens.toml` and `secrets.toml`, validate fail-closed, and hot-reload changed Valve/RBAC/Persons, `PluginSupervisor`, and `ModelProvider` routes returning `ReloadReport`. The TUI SHALL expose `/reload` in hosting and attached modes.

#### Scenario: Editing `monoagens.toml` on disk and running `/reload`
- **WHEN** the owner updates `[roles.guest]` in `monoagens.toml` on disk while the gateway is running and executes `/reload` in the TUI
- **THEN** `config.reload` applies the updated RBAC rules live to Valve and `BodyRouter::command_ctx`, refreshes TUI config/plugin/capability state, and shows a confirmation notice without restarting.

#### Scenario: Invalid `monoagens.toml` fails closed on `/reload`
- **WHEN** `monoagens.toml` contains a syntax or validation error and `/reload` is run
- **THEN** `config.reload` returns an error without replacing the running Valve, plugins, or provider routes, and the TUI displays the error cleanly.

### Requirement: `/retry` Retries Failed Boot or Last Turn
In the TUI, `/retry` SHALL trigger `RetryBoot` when `config_error` is active, and SHALL retry the last failed or interrupted user turn when the gateway is already running (showing a clear notice if no prior turn exists).

#### Scenario: `/retry` in active session vs config error
- **WHEN** the gateway is running and the owner runs `/retry` after a failed/interrupted turn
- **THEN** the TUI resubmits that last turn; if no prior turn exists, it shows a non-error toast indicating there is no turn to retry (and suggests `/reload` to reload config).
