# Specification — `tui-activity-feed`

## ADDED Requirements

### Requirement: Read-Only Adapter Activity Feed
`/activity` SHALL list adapter traffic as rows of time, plugin, place, actor label, one-line summary and outcome (replied, held, denied, silent, failed), and SHALL offer no way to send input or actions.

#### Scenario: Adapter message to reply
- **WHEN** a fake adapter delivers a message and the soul replies
- **THEN** one row appears with outcome `replied`, and the feed has no composer.

### Requirement: Rows Link Message, Turn and Outcome
Each row SHALL be built from the events of one trace so that message-in, turn and outcome appear as one row.

#### Scenario: Held action
- **WHEN** an adapter turn ends with a held action
- **THEN** the row shows outcome `held` and Enter opens the explain view for it.

### Requirement: Filters
The feed SHALL filter by plugin, place and outcome.

#### Scenario: Plugin filter
- **WHEN** rows exist for plugins `a` and `b` and the user filters to `a`
- **THEN** only rows of plugin `a` are listed.

### Requirement: Feed Survives Restart
The feed SHALL be rebuilt from the event log on open, with live rows added through `activity.subscribe`, and SHALL NOT depend on in-process memory.

#### Scenario: Reopen after restart
- **WHEN** the gateway restarts and `/activity` is opened
- **THEN** earlier rows are listed.

### Requirement: Adapter Chats Stay Out of CHATS
Adapter conversations SHALL NOT appear in the `CHATS` sidebar.

#### Scenario: Sidebar unchanged
- **WHEN** an adapter conversation occurs
- **THEN** `CHATS` lists only local chats.
