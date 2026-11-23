# Specification — `tui-observability-views`

## ADDED Requirements

### Requirement: Event Log View with CLI Filters
`/events` SHALL list the event log with filters for place, source, kind and time range, page by cursor, and open the explain view on Enter.

#### Scenario: Filter by kind and time
- **WHEN** the user filters kind `speak` and the last hour
- **THEN** only matching events are listed, and Enter shows the same explanation as `monoagens log --explain`.

### Requirement: Trace View
The TUI SHALL list every event of a `traceparent` in order, from input of the id or from a key on an event.

#### Scenario: Trace from an event
- **WHEN** the user presses `t` on an event
- **THEN** all events sharing its trace are listed in order.

### Requirement: Lab-Only Replay and Export-Learning
Replay and export-learning SHALL be available only in Lab mode, run as cancellable background jobs with progress, make no model calls and leave the live event log untouched.

#### Scenario: Replay with progress
- **WHEN** the user starts replay in Lab mode on a recorded log
- **THEN** a progress bar advances, the report matches `monoagens replay`, and the live log is unchanged.

#### Scenario: Not offered outside Lab
- **WHEN** the TUI is in normal chat mode
- **THEN** replay and export-learning are not listed.

### Requirement: Status View
`/status` SHALL show gateway owner and mode, data dir, provider and model, embedder, plugins with state, queue depth and budget used and left.

#### Scenario: Attached status
- **WHEN** the TUI is attached to `monoagens serve`
- **THEN** `/status` shows mode `serve`, the daemon's pid and its plugin states.
