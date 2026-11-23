# Specification — `tui-settings-interaction`

## ADDED Requirements

### Requirement: Scrollable Slash Menu Above the Composer
The `/` menu SHALL render above the composer with at most 10 rows, keep the selection visible while scrolling, show a scrollbar when commands exceed its height, wrap on Up/Down, accept PageUp/PageDown, wheel, click and Tab completion, and stay open while the composer starts with `/` and has no space.

#### Scenario: Select the last command
- **WHEN** the user types `/` on a terminal where the menu shows fewer rows than commands and presses Down until the last command
- **THEN** the menu has scrolled so the last command is selected and visible, the scrollbar thumb is at the bottom, and the menu is still open.

### Requirement: Scrollbars on Every List
Every scrollable list in the TUI (settings, plugin detail, pickers, selectors, palette) SHALL show a scrollbar when its content exceeds the view.

#### Scenario: Long settings list
- **WHEN** a settings tab has more rows than the viewport
- **THEN** a right-edge scrollbar is drawn and its thumb moves with the selection.

### Requirement: Esc Goes Back One Level
In settings, Esc SHALL close an open editor first, then leave a sub-page, then clear the search, then close the overlay.

#### Scenario: Nested escape
- **WHEN** the user opens a plugin detail page, starts an edit and presses Esc three times
- **THEN** the edit closes, then the detail page closes, then the settings overlay closes.

### Requirement: Selector Widget
Enter on an enum setting SHALL open a selector showing the name, description, option list with the current value marked, a scrollbar and a type-to-search line; Enter picks an option and Esc returns without a change.

#### Scenario: Pick by search
- **WHEN** the user opens a selector, types two letters and presses Enter
- **THEN** the first matching option is saved and the list view shows it.

### Requirement: Live State in Slash Descriptions
Each slash menu row SHALL show a description with live state where one exists (for example error count for `/log`, running/total for `/plugins`).

#### Scenario: Plugins row
- **WHEN** 4 of 6 plugins are running and the menu is open
- **THEN** the `/plugins` row description contains `4/6 running`.

### Requirement: Per-Screen Help Overlay
`?` or `F1` SHALL open an overlay listing every key bound on the current screen, generated from the same action table as the keybindings.

#### Scenario: Help lists a rebound key
- **WHEN** `keybindings.toml` rebinds an action and the help overlay opens
- **THEN** the overlay shows the new key for that action.

### Requirement: Keybindings File
The TUI SHALL read `keybindings.toml` mapping action ids to keys over built-in defaults, SHALL log unknown ids and conflicts as warnings and ignore them, and SHALL allow editing the bindings from settings.

#### Scenario: Conflict ignored
- **WHEN** the file binds two actions to the same key
- **THEN** the later binding is ignored and the log has a warning naming both actions.

### Requirement: Command Palette
`Ctrl+K` SHALL open a fuzzy palette over commands, settings rows and chats, and Enter SHALL run the command or jump to the settings row or chat.

#### Scenario: Jump to a setting
- **WHEN** the user presses `Ctrl+K`, types `quiet` and presses Enter on a settings row result
- **THEN** `/settings` opens on that row.

### Requirement: First-Run Wizard Uses Pickers
The first-run wizard SHALL walk through provider selection from the catalog, key entry with a connection test, model selection and plugin selection, and SHALL ask for a raw endpoint only for the `custom` provider.

#### Scenario: Wizard with a catalog provider
- **WHEN** a new user chooses `groq` in the wizard against the fake provider server
- **THEN** the wizard asks for a key, tests it, shows the discovered model picker and never shows an endpoint field.
