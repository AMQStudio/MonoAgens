# Design — `cli-plugin-lifecycle-and-ux`

## Approval Separation
- `CommandRegistry::new()` pre-registers only `/help` (`Guest`), `/whoami` (`Guest`), and `/roles` (`Admin`).
- Approval resolution has no separate slash-command permission in `CommandRegistry`: whoever Valve/guard requires to resolve a held action interacts through the adapter's native `approval.prompt` -> `approval/resolve` (Telegram buttons, WhatsApp reactions, or operator CLI `monoagens approvals`), with Absolute Line 1 (`identity_changing`) always remaining `owner`-only in Valve.

## Plugin Secrets in `secrets.toml` (`[env]`)
- `secrets.toml` supports an optional `[env]` table (e.g. `TELEGRAM_BOT_TOKEN = "..."`, `TAVILY_API_KEY = "..."`).
- `load_plugin_env_secrets` reads `[env]` as `BTreeMap<String, SecretString>`.
- `monoagens serve` passes `[env]` entries into `PluginEnvContext.plugin_env` (process environment variables take precedence if already set) and registers every value with `DiagnosticSink` for redaction so secrets never appear in logs or errors.

## Zero-Copy-Paste CLI Subcommands
- `monoagens plugin install <id>...`: resolves known built-in plugin presets (`telegram`, `whatsapp`, `web-search`, `workspace`, `pdf-reader`, `soul-tuning`, `cli`, `mcp-bridge`, `a2a-bridge`, `ros2-bridge`, `voice`) by locating the binary near `current_exe()` or workspace `target/{release,debug}` (and `plugins/whatsapp/target/{release,debug}` for `whatsapp`), or uses explicit `--command` / `--kind` / `--arg` / `--dep` / `--opt-dep` for third-party plugins. Backs up `monoagens.toml` (`.bak`) and updates `[plugins.<id>]` via `toml_edit`.
- `monoagens plugin enable|disable|remove <id>`: format-preserving `toml_edit` mutations after backup.
- `monoagens plugin list`: shows configured plugins + discovered built-in plugins (`--json` supported).
- `monoagens pair <plugin-id> [--phone <number>] [--db <path>]`: looks up `[plugins.<id>]` command (or resolves built-in binary), injects `MONOAGENS_PLUGIN_CONFIG` & `MONOAGENS_PLUGIN_STATE_DIR`, passes `pair --db <state_dir>/session.db` (plus `--phone` if provided) with inherited stdin/stdout/stderr so QR codes render directly in the terminal.
- `monoagens secret list|set|unset`: manages top-level keys (`model_api_key`, `model_endpoint`) and `[env]` keys (`--env`) in `secrets.toml` with `0600` permissions on Unix; `list` prints only key names, never values.
