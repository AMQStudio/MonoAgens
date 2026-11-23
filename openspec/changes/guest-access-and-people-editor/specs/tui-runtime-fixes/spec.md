# Specification — `tui-runtime-fixes`

## ADDED Requirements

### Requirement: TUI Body Is Ready Before the First Turn
The TUI SHALL not submit a turn before body-tui's capabilities (including `chat.speak`) are registered with the gateway; a message typed earlier SHALL be queued and sent once registration completes.

#### Scenario: Message right after start
- **WHEN** the owner sends a message immediately after the TUI starts
- **THEN** the reply is delivered and no `unknown_capability … 'chat.speak' not disclosed` verdict is recorded.

### Requirement: One Terminal Receipt per Action
For every action dispatched to body-tui exactly one terminal receipt SHALL be submitted to the core.

#### Scenario: Two consecutive turns
- **WHEN** the owner sends two messages in a row
- **THEN** both replies render and the log has no `duplicate terminal receipt` error.

### Requirement: Titles Use the Provider's Configured Headers
The titler request SHALL carry `[model.extra_headers]` with placeholders filled for the chat being titled, like any other model request.

#### Scenario: Provider that requires a session header
- **WHEN** `[model.extra_headers]` has `x-opencode-session = "{conversation_id}"` and a chat is titled
- **THEN** the title request carries a non-empty `x-opencode-session` header and the title is set from the model.

### Requirement: Config Migration Is Announced Once
After a configuration migration the TUI SHALL show one notice naming what changed and where the backups are, and SHALL not repeat it on later starts.

#### Scenario: First start after migration
- **WHEN** the legacy provider config was migrated on this start
- **THEN** a notice names the new provider id and the two `.bak` paths, and it does not appear on the next start.
