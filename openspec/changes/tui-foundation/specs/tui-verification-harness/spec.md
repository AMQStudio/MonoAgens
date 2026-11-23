# Specification — `tui-verification-harness`

## ADDED Requirements

### Requirement: Tmux Smoke Script
The repository SHALL provide `scripts/tui-smoke.sh` that starts the debug binary in tmux with a home directory under `target/tui-smoke/`, the `scripted` provider and the fake plugin, drives it with `tmux send-keys`, asserts with `tmux capture-pane -p`, and makes no paid model call.

#### Scenario: Smoke run passes locally
- **WHEN** the script runs after a debug build
- **THEN** it verifies autostart, `/log` stderr line, crash-then-restart colours, the silent-turn notice and attach-then-detach, and exits 0.

### Requirement: Rust PTY Test Driver
The test support code SHALL provide a PTY driver that spawns the binary in a pseudo-terminal, parses the screen with a terminal emulator, and offers `send_keys`, `wait_for_text` with a timeout and `screen_text`, so screen assertions run under `cargo test`.

#### Scenario: Driver sees the composer
- **WHEN** a test starts the binary with the scripted provider and waits for the composer prompt
- **THEN** `screen_text` contains the status bar and composer, and a timeout produces a failure that prints the last screen.

### Requirement: Snapshot Tests at Three Sizes
Key screens (chat, `/log`, plugin sidebar, config-error banner) SHALL have `insta` snapshot tests rendered with `TestBackend` at 80x24, 120x40 and 200x50.

#### Scenario: Snapshot set exists
- **WHEN** `cargo test -p monoagens-tui` runs
- **THEN** one snapshot per screen per size is compared and a layout change fails the test until the snapshot is reviewed.

### Requirement: Fake Plugin and Fake Provider Server
The test support code SHALL provide a fake stdio plugin whose crash timing, stderr lines and startup latency are set by environment variables, and a fake provider HTTP server on a loopback port serving model listing and chat completion responses.

#### Scenario: Fake plugin crash on demand
- **WHEN** the fake plugin is started with a crash-after setting of 200 ms and stderr line `boom`
- **THEN** it writes `boom` and exits with code 1 about 200 ms after startup.
