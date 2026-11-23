# Spec Delta

## Purpose

Replaces manual string-list RBAC editing with a plugin-detected Simple ON/OFF 2-pane toggle matrix, locked owner-only rows, live deterministic explanation, and gateway-synchronized config reads.

## ADDED Requirements

### Requirement: Plugin-Detected Simple ON/OFF Toggle Matrix
The Settings -> RBAC role editor SHALL render as a 2-pane `HubFrame` powered by `capabilities.list`, listing Roles and auto-detected plugin/domain groups (`Core`, connected/cached plugins with running/stopped status and counts, plus `Custom patterns`) on the left, Simple ON/OFF toggles (`[✓]`/`[ ]` plus `[🔒]` owner-only) on the right, and a pinned deterministic `RbacConfig::evaluate` Inspector.

#### Scenario: Enabling `agent.converse` and `chat.speak` for `guest` without typing IDs
- **WHEN** the owner opens Settings -> RBAC -> `guest` and toggles `Allow Conversation (agent.converse)` under `Core` and `Send Messages (chat.speak)` under `whatsapp` with `Space` or mouse click
- **THEN** both render `[✓]`, `config.apply` persists them to `[roles.guest].allow`, and the Inspector immediately reflects `ALLOW`.

### Requirement: Safe Explicit-List Materialization & Master Group Toggle
Toggling a plugin group header SHALL switch all non-owner-only capabilities of that plugin ON (`[✓]`) or OFF (`[ ]`), showing `[–]` when partial. Unchecking one capability from a wildcard group SHALL materialize the remaining enabled capabilities explicitly in `roles.<name>.allow` (never writing hidden `deny` rules), while `Custom patterns` edits globs or explicit denies via inline footer `TextInput`.

#### Scenario: Unchecking one capability from a wildcard group materializes explicit allow list
- **WHEN** `roles.guest.allow` contains `chat.*` and the owner unchecks `Delete Messages (chat.delete)` in the `whatsapp` group
- **THEN** `roles.guest.allow` is updated via `config.apply` to the explicit list of remaining `chat.*` capabilities without `chat.delete` and without creating an unexpected `deny` entry.

### Requirement: Actors, Places, and Gateway-Synchronized Config Reads
RBAC Actor role assignments and Place rules SHALL use the same 2-pane `HubFrame` toggle/pattern UX with zero nested popup stacking. When connected to a gateway (`!gateway_link.is_empty()`), the TUI SHALL synchronize configuration reads through `GatewayClient::config_get()` rather than reading `monoagens.toml` directly off disk bypassing the daemon.

#### Scenario: Attached TUI reflects daemon config via `config.get`
- **WHEN** an attached TUI connects to a running `monoagens serve` daemon and opens Settings -> RBAC
- **THEN** the displayed roles and toggles reflect the daemon's `config.get` response.
