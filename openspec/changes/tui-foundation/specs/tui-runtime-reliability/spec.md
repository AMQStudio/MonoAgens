# Specification — `tui-runtime-reliability`

## ADDED Requirements

### Requirement: Enabled Plugins Auto-Start in Dependency Order
The lock-holding process SHALL start every plugin with `enabled = true` at gateway boot, ordered by `PluginConfig.dependencies`, and SHALL mark a plugin `Failed` with a reason when its hard dependency is missing or cyclic while still starting the others.

#### Scenario: Dependency order and failure isolation
- **WHEN** plugins `a` (depends on `b`), `b`, and `c` (depends on missing `x`) are enabled
- **THEN** `b` starts before `a`, `c` is shown red with a reason naming `x`, and `a` and `b` run.

### Requirement: Plugin Crashes Are Visible and Recovered
The supervisor SHALL capture each plugin's stderr into the runtime log as `plugin:<id>`, mark an unexpected exit `Failed("exited <code>: <last stderr line>")`, and restart with backoff 1 s, 5 s, 30 s, giving up at the 4th failure in 10 minutes (so the 1 s, 5 s and 30 s delays are all used); a user Stop SHALL cancel the restart loop.

#### Scenario: Crash then recovery
- **WHEN** the fake plugin writes `boom` to stderr and exits with code 1
- **THEN** its dot turns red with reason `exited 1: boom`, the log has `boom` under `plugin:fake`, and it is green again after the first backoff.

#### Scenario: Give up after repeated crashes
- **WHEN** the fake plugin exits four times within 10 minutes
- **THEN** no further restart is attempted and its dot stays red until the user starts it.

### Requirement: A Plugin Held Elsewhere Is Not Spawned
Before spawning a plugin the supervisor SHALL take `<data_dir>/plugins/<id>.lock`, and if another process holds it SHALL show state `RunningElsewhere` and spawn nothing.

#### Scenario: Held lock
- **WHEN** an external process holds the lock for plugin `whatsapp` and the gateway boots with it enabled
- **THEN** no `whatsapp` child is spawned and the sidebar shows it as running in another MonoAgens.

### Requirement: Every Turn Ends the Busy State
The TUI SHALL end the busy state on the first of an assistant reply, a turn error, or turn completion followed by a 1.5 s grace window without a reply, and in the last case SHALL show the muted notice "Mono chose not to reply to this message." and log `turn.no_reply`.

#### Scenario: Silent turn
- **WHEN** a scripted turn finishes without calling `chat.speak`
- **THEN** busy stops within 1.5 s of turn completion, the notice appears under the user message, and the next message starts its own turn.

### Requirement: Messages Sent While Busy Are Queued and Input Is Never Lost
The TUI SHALL queue a message sent while busy as a dim "queued" bubble, submit queued messages in order when busy ends, let `Alt+Up` pull the last queued message back into the composer, and SHALL NOT discard keys typed during a redraw.

#### Scenario: Typing during a turn
- **WHEN** the user types and presses Enter while a turn runs and the terminal is resized meanwhile
- **THEN** the text is kept, the message shows as queued, it is submitted after the running turn ends, and `Alt+Up` before that returns the text to the composer.

### Requirement: Exactly One Busy Indicator
The TUI SHALL show one busy spinner at a time, in the status bar with elapsed time, and the phase line SHALL show phase text without its own spinner.

#### Scenario: Single spinner
- **WHEN** a turn is running with a phase text
- **THEN** the rendered screen contains exactly one spinner glyph and the elapsed time.

### Requirement: Resize, Focus and Suspend Are Handled
The TUI SHALL redraw on Resize and FocusGained, SHALL restore raw mode and the alternate screen after `Ctrl+Z` suspend and resume, and SHALL perform a full display reset on `Alt+L`.

#### Scenario: Suspend and resume
- **WHEN** the user presses `Ctrl+Z` and later resumes the process with `fg`
- **THEN** the TUI redraws in the alternate screen and typed keys reach the composer.

### Requirement: Width-Safe Rendering
The TUI SHALL fit every row by display width (grapheme clusters and `unicode-width`), SHALL render chrome icons as single-width symbols by default, and SHALL sanitise tabs and control characters.

#### Scenario: Emoji with a variation selector
- **WHEN** a status label containing a U+FE0F emoji is rendered in an 80-column buffer
- **THEN** no row exceeds 80 columns and no stray character appears at the right edge.

### Requirement: Defined Small-Terminal Layout
Below 80 columns the TUI SHALL hide the sidebar and collapse the status bar to one line, below 20 rows it SHALL hide the header and hint line, and below 40x10 it SHALL show only a "terminal too small" message.

#### Scenario: 60x15 terminal
- **WHEN** the TUI renders in a 60x15 buffer
- **THEN** the sidebar is not drawn, the status bar is one line, and the composer and chat remain usable.

### Requirement: TUI Opens on Bad Configuration
When the configuration fails to parse or validate, the TUI SHALL still open, show the error with the offending key when known, open `/settings` on that key, and offer a retry of the gateway boot instead of exiting.

#### Scenario: Nonsense provider
- **WHEN** `monoagens.toml` has `provider = "nonsense"` and the TUI starts
- **THEN** the TUI opens with a banner containing the error text, chat input is disabled, and fixing the value then choosing retry boots the gateway.

### Requirement: Doctor Command
The `/doctor` command SHALL check config parsing, secrets file mode, provider reachability (skipped for `scripted`), plugin binaries, embedder files, data-dir writability and lock owner, and SHALL list each check with pass or fail and a fix hint.

#### Scenario: Missing plugin binary
- **WHEN** an enabled plugin's command path does not exist and `/doctor` runs
- **THEN** that check shows fail with the path and a hint, and the other checks still run.

### Requirement: Runtime Log Window
The TUI SHALL keep a runtime log (last 5 000 entries in memory and a rotating file under the data dir, 5 MiB x 3) with level, source and message, SHALL never write secret values to it, and SHALL open it with `/log` as a scrollable overlay with level filter, source filter, search, follow-tail, scrollbar and mouse wheel.

#### Scenario: Filter by plugin source
- **WHEN** a plugin writes a line to stderr and the user runs `/log` and filters the source to that plugin
- **THEN** the overlay lists the line with timestamp and level, hides other sources, shows a scrollbar when entries exceed the view, and the line is also in `<data_dir>/logs/tui.log`.

#### Scenario: Secrets are masked
- **WHEN** a log message would contain an API key value from `secrets.toml`
- **THEN** the key text appears masked in both the overlay and the file.

### Requirement: Error Toast Points to the Log
The TUI SHALL show a toast "<message> - /log for details" for every log entry of level Error.

#### Scenario: Error entry
- **WHEN** an Error entry is logged
- **THEN** a toast with that message and the `/log` hint is shown.
