# Specification — `cli-plugin-lifecycle-and-ux`

## ADDED Requirements

### Requirement: Built-In Command Registry Excludes Separate Approval Commands
`CommandRegistry` SHALL pre-register `/help`, `/whoami`, and `/roles` without separate `/approve` or `/deny` slash-command permissions, leaving approval and denial entirely to Valve and native adapter `approval.prompt` -> `approval/resolve` flows.

#### Scenario: `/help` indexes informational built-ins and plugin commands without `/approve` or `/deny`
- **WHEN** `CommandRegistry::new()` is queried for built-in commands or `/help` output
- **THEN** `/help`, `/whoami`, and `/roles` are indexed according to caller RBAC tier, and `/approve` and `/deny` are absent from the built-in command list.

### Requirement: Zero-Copy-Paste Plugin Lifecycle, Pairing, and Secret CLI
The `monoagens` CLI SHALL provide `plugin <list|install|enable|disable|remove>`, `pair <plugin-id>`, and `secret <list|set|unset>` subcommands that manage `monoagens.toml` (via format-preserving `toml_edit` after backup) and `secrets.toml` (`0600` permissions, including `[env]` forwarded into spawned plugins with redaction) without requiring manual TOML editing or `.env` sourcing.

#### Scenario: Installing, toggling, and removing a plugin updates `monoagens.toml` cleanly
- **WHEN** the operator runs `monoagens plugin install`, `disable`, `enable`, `list`, and `remove`
- **THEN** `monoagens.toml` is backed up and updated via `toml_edit` with the corresponding `[plugins.<id>]` table while preserving existing config sections and comments.

#### Scenario: `secrets.toml` `[env]` entries forward into spawned plugins under `monoagens serve`
- **WHEN** a plugin token is stored under `[env]` in `secrets.toml` via `monoagens secret set --env` and `monoagens serve` spawns a stdio plugin
- **THEN** the spawned plugin receives that environment variable without manual shell export and `monoagens secret list` prints only key names without exposing values.
