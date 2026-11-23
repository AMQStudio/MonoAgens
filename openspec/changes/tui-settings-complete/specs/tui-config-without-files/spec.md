# Specification — `tui-config-without-files`

## ADDED Requirements

### Requirement: Every Configuration Key Is Editable in the TUI
Every key read from `monoagens.toml`, `tui.toml`, `secrets.toml` and `plugins/<id>.toml` SHALL have a `/settings` row, except keys on a documented exclusion list with a reason, and a test SHALL enforce this coverage.

#### Scenario: Coverage test catches a missing key
- **WHEN** a new key is added to the config parser without a settings row or exclusion entry
- **THEN** the settings coverage test fails and names the key.

### Requirement: Typed Field Editors
Settings SHALL edit enums with a searchable selector, numbers with a range check, secrets with a masked editor that reveals on `Ctrl+R`, lists with add/edit/delete/reorder, maps with key/value rows, and paths with an exists check.

#### Scenario: Number out of range
- **WHEN** the user enters a value above a number field's maximum
- **THEN** the edit is rejected with the allowed range and the saved value is unchanged.

#### Scenario: List reorder
- **WHEN** the user moves the second item of a list field up
- **THEN** the saved list has the two items swapped.

### Requirement: Settings Writes Go Through Config Apply
Every settings edit SHALL be applied with the admin method `config.apply`, and secrets SHALL be written without being echoed back to the client, so an attached TUI edits the daemon's configuration the same way a hosting TUI does.

#### Scenario: Attached TUI edits a toggle
- **WHEN** a TUI attached to `monoagens serve` changes a valve toggle
- **THEN** the daemon's `monoagens.toml` changes, a `.bak` is written, and the daemon's next turn uses the new value.

### Requirement: Embeddings and Model Tabs Cover All Keys
The Embeddings section SHALL edit backend, model path, endpoint, dimension and timeout, and the Model tab SHALL edit temperature, output limits, budget, `output_limit_param`, extra headers and usage paths.

#### Scenario: Output limit parameter
- **WHEN** the user selects `max_completion_tokens` for `output_limit_param`
- **THEN** `[model]` stores it and the next request body uses that parameter name.

### Requirement: Valve Toggles Explain Themselves
Each valve toggle row SHALL show a one-line description of what it switches.

#### Scenario: Description footer
- **WHEN** a valve toggle row is focused
- **THEN** the footer under the list shows that toggle's description.
