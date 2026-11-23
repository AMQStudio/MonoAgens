# Tasks — `cli-plugin-lifecycle-and-ux`

## 1. Gateway Command Cleanup & Plugin `[env]` Secrets (`crates/gateway`)
- [x] 1.1 Remove `/approve` and `/deny` from `CommandRegistry::new()` in `crates/gateway/src/commands.rs` and update related unit/integration tests.
- [x] 1.2 Add `load_plugin_env_secrets` in `crates/gateway/src/secrets.rs` (reading `[env]` from `secrets.toml` as `BTreeMap<String, SecretString>`), wire `plugin_env` into `PluginEnvContext` & `build_plugin_env` in `crates/gateway/src/plugins.rs` (process env takes precedence, redaction registered in `DiagnosticSink`), and verify `cargo test -p monoagens-gateway` + clippy.

## 2. Zero-Copy-Paste CLI UX (`crates/cli`)
- [x] 2.1 Implement `monoagens plugin <list|install|enable|disable|remove>` (with built-in binary auto-discovery, custom `--command`, and format-preserving `toml_edit` + `.bak` backup) and `monoagens pair <plugin-id> [--phone ...] [--db ...]` in `crates/cli/src/commands/plugin.rs`.
- [x] 2.2 Implement `monoagens secret <list|set|unset>` (with `--env`, `--from-env`, `--value`, `--stdin`, `0600` perms, zero value printing) and wire `load_plugin_env_secrets` into `monoagens serve`.
- [x] 2.3 Add integration tests covering `plugin`, `pair`, `secret`, and `serve` `[env]` forwarding; verify `cargo test -p monoagens-cli`, workspace clippy, `openspec validate cli-plugin-lifecycle-and-ux --strict`, and GitNexus `detect_changes`.
