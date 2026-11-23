# Specification — `model-picker-login`

## ADDED Requirements

### Requirement: Two-Pane Model Picker
`/model` SHALL open a picker with a left provider pane (configured providers first with a dot and model count, a separator, then unconfigured providers, and a first row "All models") and a right pane with a search line, model list, scrollbar and a detail line for the focused model.

#### Scenario: Search and select
- **WHEN** the user opens `/model`, types `qwen` and presses Enter on a discovered model
- **THEN** only matching models are listed, `[model].model` is saved through `config.apply`, the status bar shows the model, and the next turn uses it.

### Requirement: Unconfigured Provider Asks for a Key First
Choosing a model on a provider with no key SHALL open that provider's key page before selecting the model.

#### Scenario: Key page opens
- **WHEN** the user picks a model under an unconfigured provider
- **THEN** the masked key entry for that provider opens, and after a successful test the model is selected.

### Requirement: Login Provider List
`/login` SHALL list providers with a searchable list showing `key set` or not set per row, open a masked key entry with a connection test on Enter, and return to where the user came from.

#### Scenario: Test connection
- **WHEN** the user enters a key for a provider and runs the test against the fake provider server
- **THEN** the result shows the model count and latency, the key is saved to `secrets.toml [providers.<id>]` with mode 0600, and Esc returns to the previous screen.

### Requirement: Live State in Slash Descriptions
The `/model` menu entry SHALL show the current model in its description.

#### Scenario: Description shows the model
- **WHEN** the slash menu is open with model `deepseek-v4.1-flash` active
- **THEN** the `/model` row description contains `deepseek-v4.1-flash`.

### Requirement: Clear Provider Error Notices
The TUI SHALL show rate-limit, authentication and timeout failures of a model call as a notice containing the provider's message and a pointer to `/log`.

#### Scenario: Rate limited
- **WHEN** the fake provider returns HTTP 429 with a message body
- **THEN** a notice shows the message and "see /log", and the log has a `provider` entry with status 429.
