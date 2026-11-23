# Proposal — `unified-plugins-rbac-approvals`

## Why
As MonoAgens prepares for international third-party plugins and live multi-user WhatsApp/Telegram groups, three architectural needs must be unified cleanly without bloating `monoagens.toml` or hardcoding platform scenarios: (1) an open, unrestricted Core guarded strictly by Valve with config-driven RBAC tiers (`guest < member < trusted < admin < owner`) that narrow Cedar without weakening `cvc5` proofs; (2) a unified `[plugins.<id>]` model (`kind = "adapter" | "capability"`) with dependency resolution (`dependencies`, `optional_dependencies` including `"rbac"`), self-managed per-plugin config files via `MONOAGENS_PLUGIN_CONFIG` / `MONOAGENS_PLUGIN_STATE_DIR`, and a Gateway Command Registry with RBAC-filtered `/help`; and (3) completely generic adapter-rendered approval prompts (`approval.prompt` capability) so Telegram renders native inline buttons and WhatsApp renders native emoji-reaction prompts for any guarded or model-proposed action.

## What Changes
- **`crates/valve`**: Add generic `RbacConfig` (`RoleTier`, `ApprovalMode`, `RoleDefinition`, `PlaceAccessRule`, `role_settings`) integrated as a strict narrowing layer inside `Valve` alongside `cvc5`-verified Cedar policies (zero bypass of Absolute Lines 1 & 2).
- **`crates/gateway`**: Add unified `[plugins.<id>]` config (`kind = "adapter" | "capability"`, preserving `[[bodies]]` alias), topological dependency resolver (`dependencies`, `optional_dependencies` incl. `"rbac"`), automatic `MONOAGENS_PLUGIN_CONFIG` & `MONOAGENS_PLUGIN_STATE_DIR` injection, generic `CommandRegistry` (`/help`, `/whoami`, `/roles`, `/approve`, `/deny` + plugin commands from `discover._meta.commands`, 0-LLM-call execution), and generic `approval.prompt` dispatch + verified owner `approval/resolve`.
- **`bodies/body-telegram` & `bodies/body-whatsapp`**: Auto-generate & load self-managed `telegram.toml` / `whatsapp.toml` via `MONOAGENS_PLUGIN_CONFIG`, default state storage to `MONOAGENS_PLUGIN_STATE_DIR`, and implement the generic `approval.prompt` capability natively (Telegram `inline_keyboard` Approve/Deny buttons -> `approval/resolve`; WhatsApp reaction watcher with configurable emojis -> `approval/resolve`) with zero hardcoded scenarios.
- **`crates/cli`**: Wire `[plugins.*]`, dependency-ordered startup, RBAC config, and plugin config/state paths into `monoagens serve`.

## Impact
- `crates/valve/src/` (new `rbac.rs`, `ValveConfig` / `Valve` integration)
- `crates/gateway/src/` (`config.rs`, new `plugins.rs`, new `commands.rs`, `waist2.rs`, `runtime.rs`)
- `bodies/body-telegram/` and `bodies/body-whatsapp/` (self-managed config + `approval.prompt` capability)
- `crates/cli/src/` (`common.rs`, `commands/serve.rs`)
- Zero changes to `crates/core` or `ARCHITECTURE.md` invariants.
