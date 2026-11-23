# Proposal — `cli-plugin-lifecycle-and-ux`

## Why
Manual TOML copy-paste and shell `.env` sourcing create poor operator UX, and registering `/approve` and `/deny` as built-in permissioned slash commands conflates command RBAC with Valve's generic approval lifecycle. Moving approval/denial exclusively to Valve + native adapter UX (`approval.prompt` -> `approval/resolve`) and adding first-class `monoagens plugin`, `monoagens pair`, and `monoagens secret` CLI subcommands delivers zero-copy-paste setup suitable for international release.

## What Changes
- **`crates/gateway`**:
  - Remove `/approve` and `/deny` from built-in `CommandRegistry` so built-in commands stay strictly informational/control (`/help`, `/whoami`, `/roles`) while approval/denial is handled generically by Valve + native adapter `approval.prompt` -> `approval/resolve`.
  - Support optional `[env]` table in `secrets.toml` (`load_plugin_env_secrets`), register all loaded values for diagnostic redaction, and forward non-overridden plugin env vars into spawned plugin processes via `PluginEnvContext`.
- **`crates/cli`**:
  - Add `monoagens plugin <list|install|enable|disable|remove>` mutating `[plugins.<id>]` in `monoagens.toml` via format-preserving `toml_edit` after backup, with auto-discovery of built-in plugin binaries (`telegram`, `whatsapp`, `web-search`, `workspace`, `pdf-reader`, `soul-tuning`, `cli`, `mcp-bridge`, `a2a-bridge`, `ros2-bridge`, `voice`) and custom `--command` support.
  - Add `monoagens pair <plugin-id> [--phone <number>] [--db <path>]` delegating interactively to the plugin's `pair` subcommand with auto-injected `MONOAGENS_PLUGIN_CONFIG`, `MONOAGENS_PLUGIN_STATE_DIR`, and default `session.db` path.
  - Add `monoagens secret <list|set|unset>` managing `secrets.toml` (including `[env]` plugin tokens) with `0600` file permissions and zero secret printing.
  - Wire `[env]` plugin secrets from `secrets.toml` into `monoagens serve`.

## Impact
- `crates/gateway/src/` (`commands.rs`, `secrets.rs`, `plugins.rs`, `runtime.rs`, `lib.rs`, tests)
- `crates/cli/src/` (`lib.rs`, `commands/mod.rs`, `commands/plugin.rs`, `commands/serve.rs`, tests)
- Zero changes to `crates/core`, `crates/valve`, or `ARCHITECTURE.md`.
