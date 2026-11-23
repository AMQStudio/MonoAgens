# Specification — `unified-plugins-rbac-approvals`

## ADDED Requirements

### Requirement: Generic Config-Driven RBAC Narrowing in Valve
`crates/valve` SHALL provide a deterministic `RbacConfig` (`Guest < Member < Trusted < Admin < Owner`, named roles with `allow`/`deny` patterns, `ApprovalMode`, `role_settings`, per-place `allow`/`deny`, and permission base tiers) that strictly narrows capability and command authorization without weakening `cvc5`-verified Absolute Lines 1 & 2.

#### Scenario: Tiered role and place rules narrow non-owner capability access
- **WHEN** `RbacConfig` assigns an actor to a `guest` role and a capability requires `member` or is explicitly denied on the target place
- **THEN** `Valve` deterministically refuses the action while continuing to allow `owner` and permitted capabilities, and `ApprovalMode::Strict` holds permitted non-R3 actions as `PendingApproval`.

### Requirement: Unified `[plugins.<id>]` Configuration, Dependencies, and Self-Managed Plugin Configs
Gateway SHALL parse `[plugins.<id>]` (`kind = "adapter" | "capability"`, preserving `[[bodies]]` as an adapter alias), topologically resolve `dependencies` and `optional_dependencies` (including built-in `"rbac"`), skip only plugins with missing required dependencies without stopping sibling plugins, and inject `MONOAGENS_PLUGIN_CONFIG` and `MONOAGENS_PLUGIN_STATE_DIR` into spawned plugin processes.

#### Scenario: Missing dependency isolates only the broken plugin and injects plugin config paths
- **WHEN** Gateway boots plugins where plugin A has a missing required dependency and plugin B has `optional_dependencies = ["rbac"]`
- **THEN** plugin A is skipped with a diagnostic warning, plugin B connects cleanly with `MONOAGENS_PLUGIN_CONFIG` pointing to `<config_dir>/plugins/<id>.toml` and `MONOAGENS_PLUGIN_STATE_DIR` pointing to `<data_dir>/plugins/<id>`, and plugin B owns creating and reading its own config file.

### Requirement: Gateway Command Registry and RBAC-Filtered `/help`
Gateway SHALL maintain a `CommandRegistry` indexing built-in commands (`/help`, `/whoami`, `/roles`, `/approve`, `/deny`) and plugin-declared commands from `discover._meta.commands`, answering `/help` and authorized slash commands without invoking the reasoning model while filtering visible and executable commands by the sender's RBAC tier.

#### Scenario: `/help` indexes built-in and plugin commands filtered by caller role
- **WHEN** a `guest` actor and an `owner` actor each invoke `/help`
- **THEN** Gateway answers both without an LLM call, showing the `guest` only commands permitted for `guest` while showing the `owner` all registered built-in and plugin commands.

### Requirement: Generic Adapter-Rendered Approval Prompts (`approval.prompt` -> `approval/resolve`)
Gateway SHALL support dispatching the generic `approval.prompt` capability to any adapter that declares it so the adapter renders any `PendingApproval` using its platform's native UX and returns a generic `approval/resolve` request verified by Gateway/Valve (`owner` identity, `h_act`, `state_epoch`, TTL, single settlement), with zero hardcoded scenarios in Core, Valve, Gateway, or adapters.

#### Scenario: Telegram renders native inline buttons and WhatsApp renders native emoji-reaction prompts
- **WHEN** Gateway dispatches `approval.prompt` for any held action to `body-telegram` or `body-whatsapp`
- **THEN** `body-telegram` sends native `inline_keyboard` Approve/Deny buttons and translates button `callback_query` events into `approval/resolve`, while `body-whatsapp` sends a prompt message and translates matching inbound emoji reactions into `approval/resolve`, and both bodies still pass `monoagens-conformance body --level should`.
