# Specification — `tui-plugin-management`

## ADDED Requirements

### Requirement: Plugin Detail Page
A plugin's detail page SHALL show its state with the failure reason, its last 5 stderr lines, Start, Stop and Restart actions, an enable-on-startup toggle and a View log action that opens `/log` filtered to that plugin.

#### Scenario: Restart from detail
- **WHEN** the user presses Restart on a running fake plugin
- **THEN** the state passes through starting and returns to running, and View log opens `/log` filtered to `plugin:<id>`.

### Requirement: Required Secret Warnings
The detail page SHALL warn for each required secret of the plugin that is not set, name it, and offer an action to set it.

#### Scenario: Missing token
- **WHEN** the telegram plugin is enabled and `TELEGRAM_BOT_TOKEN` is not set
- **THEN** its detail page shows a warning naming `TELEGRAM_BOT_TOKEN` with a set action, and a failed start shows the plugin's own error text.

### Requirement: Typed Plugin Value Editors
The detail page SHALL edit values from `plugins/<id>.toml` with editors matching their types: boolean as toggle, number as number field, array as list, string as text.

#### Scenario: Array value
- **WHEN** a plugin config has an array value and the user adds an item
- **THEN** the file stores the array with the new item and comments elsewhere are preserved.

### Requirement: In-TUI Pairing Page
Pairing an adapter that supports it SHALL be shown on a page inside the TUI with the QR code or pair code and a live status until paired, without suspending the TUI.

#### Scenario: Pairing completes
- **WHEN** the user opens pairing for the fake pairing plugin that emits a code and then reports paired
- **THEN** the page shows the code, then shows status paired, and the TUI never leaves the alternate screen.

### Requirement: Guided Install and Confirmed Remove
Installing a catalog plugin SHALL run a guided flow (command path detection, kind, enable) and show a build hint when the binary is missing, and removing a plugin SHALL ask for confirmation.

#### Scenario: Missing binary
- **WHEN** the user installs a catalog plugin whose binary is not found
- **THEN** the flow shows the expected path and a build hint instead of enabling it.

#### Scenario: Remove asks first
- **WHEN** the user removes a plugin
- **THEN** a confirmation is shown and the plugin remains if the user declines.
