# Specification — `tui-gateway-ownership`

## ADDED Requirements

### Requirement: One Gateway per Data Directory
MonoAgens SHALL hold an exclusive lock `<data_dir>/gateway.lock` for the life of any process that hosts a gateway, and a process that cannot take the lock SHALL NOT boot a second gateway on that data directory.

#### Scenario: Second host is refused or attaches
- **WHEN** `monoagens serve` is running on a data directory and a second `monoagens serve` starts on the same directory
- **THEN** the second process exits non-zero with a message naming the holder's pid, and the first process keeps running with one queue consumer.

#### Scenario: Lock is released on exit
- **WHEN** the lock holder is killed and a new process starts on the same data directory
- **THEN** the new process takes the lock and boots normally.

### Requirement: Owner-Only Admin Socket
The lock holder SHALL serve an admin socket `<data_dir>/gateway.sock` with file mode 0600, speaking NDJSON JSON-RPC, and SHALL replace a stale socket left by a dead holder.

#### Scenario: Socket permissions and stale socket
- **WHEN** a gateway boots with a leftover `gateway.sock` from a dead process
- **THEN** the old socket is replaced, the new one has mode 0600, and a client can call `status` and receive the owner pid and data dir.

### Requirement: Initial Admin Methods
The admin socket SHALL implement `status`, `config.get`, `config.apply`, `plugins.list`, `plugins.start`, `plugins.stop`, `plugins.restart`, `logs.subscribe`, `turn.submit` and `turn.cancel`, and SHALL return a JSON-RPC error for an unknown method.

#### Scenario: Restart a plugin through the socket
- **WHEN** a client calls `plugins.restart` with the id of a running fake plugin
- **THEN** the plugin process is replaced by a new one, `plugins.list` shows it running, and an unknown method returns a JSON-RPC error with code -32601.

### Requirement: Config Apply Hot-Reloads Changes
`config.apply` SHALL write the patch to the configuration file with `toml_edit` after saving a `.bak` copy, and SHALL then reload only what changed: plugin restart for plugin keys and valve reload for valve keys.

#### Scenario: Patch writes a backup and reloads the valve
- **WHEN** a client calls `config.apply` with a changed valve toggle
- **THEN** a `.bak` of the previous file exists, the file contains the new value with unrelated comments preserved, and the next turn runs under the new valve setting without a restart.

### Requirement: One Client Interface with Two Implementations
The TUI SHALL reach the gateway only through a `GatewayClient` trait with an in-process implementation (the TUI hosts the gateway and also serves the socket) and a socket implementation (attached to `monoagens serve` or another TUI).

#### Scenario: Attach to a running serve
- **WHEN** `monoagens serve` is running and the user starts the TUI on the same data directory
- **THEN** the TUI attaches without booting a gateway, shows the daemon's plugins and state, and a message sent in the TUI is answered with its `chat.speak` delivered to the attached TUI.

### Requirement: Detaching Leaves the Daemon Running
Closing an attached TUI SHALL close only its connection; the daemon and its plugins SHALL keep running and a later TUI SHALL be able to attach again.

#### Scenario: Close and reopen
- **WHEN** an attached TUI exits and a new TUI starts on the same data directory
- **THEN** the daemon pid is unchanged, its plugins were never restarted, and the new TUI attaches.

### Requirement: Owner Turn Cancel Recorded as an Event
`turn.cancel` SHALL stop waiting for the in-flight model call and submit `Input::ModelFailure` with reason `cancelled_by_owner`, and SHALL do nothing and report `cancelled: false` when no model call is in flight.

#### Scenario: Cancel a slow scripted model call
- **WHEN** a turn is waiting on a scripted provider that delays its answer and `turn.cancel` is called
- **THEN** the turn ends within 1 s, the event log contains a model failure with reason `cancelled_by_owner`, `monoagens replay` of that log reproduces identical decisions, and a second `turn.cancel` returns `cancelled: false`.
