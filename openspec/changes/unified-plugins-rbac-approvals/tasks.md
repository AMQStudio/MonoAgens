# Tasks — `unified-plugins-rbac-approvals`

## 1. Generic RBAC Narrowing in Valve (`crates/valve`)
- [x] 1.1 Implement `crates/valve/src/rbac.rs` (`RoleTier`, `ApprovalMode`, `RoleDefinition`, `PlaceAccessRule`, `RbacConfig`, `RbacDecision`, glob/prefix matching, `role_settings` lookup) and export from `crates/valve/src/lib.rs`.
- [x] 1.2 Integrate optional `RbacConfig` into `ValveConfig` and `Valve` action/command checks as a strict narrowing layer preserving `cvc5` proofs and all existing Valve tests.

## 2. Self-Managed Config & Generic Native `approval.prompt` in Adapters
- [x] 2.1 Update `bodies/body-telegram` to auto-create/load `telegram.toml` from `MONOAGENS_PLUGIN_CONFIG`, honor `MONOAGENS_PLUGIN_STATE_DIR`, expose `_meta.kind = "adapter"`, and implement generic `approval.prompt` (native `inline_keyboard` buttons -> `approval/resolve`), passing unit tests & `monoagens-conformance --level should`.
- [x] 2.2 Update `bodies/body-whatsapp` to auto-create/load `whatsapp.toml` from `MONOAGENS_PLUGIN_CONFIG` (including configurable `approve_emojis`/`deny_emojis`), honor `MONOAGENS_PLUGIN_STATE_DIR`, expose `_meta.kind = "adapter"`, and implement generic `approval.prompt` (reaction watcher -> `approval/resolve`), passing unit tests & `monoagens-conformance --level should`.

## 3. Unified Plugins, Dependency Resolver, Command Registry & Approval Wiring (`crates/gateway`, `crates/cli`)
- [x] 3.1 Add `[plugins.<id>]` (`kind = "adapter" | "capability"`, `[[bodies]]` alias), `[roles]` / `[places]` / `[[person]].roles` parsing in `crates/gateway/src/config.rs`, topological dependency resolver in `crates/gateway/src/plugins.rs`, and `MONOAGENS_PLUGIN_CONFIG` / `MONOAGENS_PLUGIN_STATE_DIR` / `MONOAGENS_RBAC_ENABLED` spawn env injection.
- [x] 3.2 Implement `crates/gateway/src/commands.rs` (`CommandRegistry` for `/help`, `/whoami`, `/roles`, `/approve`, `/deny` + `discover._meta.commands`, RBAC-filtered, 0-LLM-call) and generic `approval.prompt` dispatch + verified `approval/resolve` handling in `crates/gateway/src/waist2.rs`.
- [x] 3.3 Wire unified plugin boot, RBAC config, and startup summary into `crates/cli` (`monoagens serve`), update `docs/messaging-bodies.md`, and verify all tests, conformance (`--level should`), workspace clippy, `openspec validate --strict`, and GitNexus `detect_changes`.
