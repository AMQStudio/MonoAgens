# Proposal — `interactive-tui`

## Why
Running MonoAgens in a terminal today requires either a plain line-by-line REPL or manual TOML editing for settings and plugins. Building a native pure-Rust interactive TUI (`crates/tui` over `ratatui`, `crossterm`, and `tachyonfx`) as the default `monoagens` TTY experience delivers a calm, modern borderless chat app (inspired by OMP and Claude Code) with full mouse and keyboard support, live 3-color plugin status indicators, comprehensive 8-tab settings with dynamic per-plugin TOML editing and interactive pairing, a first-run setup wizard, LLM-powered automatic chat naming with topic-change re-titling, and command-gated `/lab` simulation—all with zero copy-paste and zero duplication of Core, Valve, or Gateway logic.

## What Changes
- **Add `crates/tui` (`monoagens-tui`)** workspace crate (`#![forbid(unsafe_code)]`) built on `ratatui 0.30`, `crossterm 0.29` (with full mouse capture), `tachyonfx 0.25`, and `toml_edit`:
  - **Borderless Chat App Main View**: Dark right-aligned user bubbles (no `You` label in 1:1), clean left-aligned assistant replies (no `Mono` label), dim `┌─ ... └─` capability step blocks, thin 1-line status bar, inline 2-column `/` slash-command autocomplete, and generic inline `▎` Valve Approval cards (`[ Approve ]` / `[ Deny ]`, zero hardcoded scenarios).
  - **Toggleable Responsive Sidebar (`Ctrl+B`)**:
    - `CHATS` list of local terminal chats (NOT disconnected conventional sessions—one persistent Mono soul & memory log), with **background LLM auto-titling on first message and automatic re-titling on topic shift** (respecting manual `/chat rename` locks).
    - `PLUGINS` live list showing `(Adapter)` / `(Capabilities)` kind labels and 3-color status dots: **green** (running), **yellow** (starting/stopping), **red** (stopped/failed).
  - **Command-Gated `/lab` Mode**: No mode tabs in header and no `Code (Coming Soon)` clutter. Typing `/lab` enters Lab simulation mode (enabling `/as`, `/group`, `/dm`, simulated actors in sidebar, and dim brain telemetry strips); `/lab exit` returns cleanly to normal chat.
  - **Full 8-Tab `/settings` Overlay (`F2` / `/settings`)**: 2-pane OMP-style layout (top tab bar, left section index, right scrollable options with type-to-search, 1-line description, live status-line preview) covering `Appearance`, `Soul & Brain`, `Model`, `Valve`, `RBAC`, `Providers` (masked `secrets.toml` + `[env]` editor), `Plugins` (drill-down generic editor for `~/.config/monoagens/plugins/<id>.toml`, start/stop, install/remove, and interactive `Pair Device` with clean raw-mode suspend/resume), and `Runtime`, persisting directly via `toml_edit` with `.bak` backup.
  - **First-Run Setup Wizard**: Interactive centered setup flow when `secrets.toml` / `model_api_key` is missing so new users never edit config files manually.
  - **Full Mouse Support**: Wheel scrolling across chat feed and settings lists, click-to-select chats/plugins/tabs/sections/options/slash-items/Approval buttons, toggleable in `Appearance -> Mouse Support`.
  - **Subtle `tachyonfx` Animations**: Message fade-in, thinking shimmer/braille spinner, modal transition, and status toasts respecting `Motion: full | reduced | off` and `NO_COLOR=1`.
- **Wire `crates/cli`**: Make `Commands` optional in `Cli` so invoking `monoagens` with no subcommand in an interactive TTY launches `monoagens_tui::run(...)`, while all existing subcommands (`serve`, `plugin`, `pair`, `secret`, `chat`, `log`, `replay`, `approvals`, `link`, `conformance`, `inspector`, `export-learning`, `--json`) remain 100% unchanged and headless-safe.

## Impact
- `Cargo.toml` & `Cargo.lock` (adds `crates/tui` workspace member + `ratatui`, `crossterm`, `tachyonfx`).
- `crates/tui/` (new crate).
- `crates/cli/Cargo.toml` & `crates/cli/src/lib.rs` (optional subcommand defaulting to TUI in TTY + explicit `monoagens tui` subcommand).
- Zero changes to `crates/core`, `crates/valve`, `crates/gateway`, or `ARCHITECTURE.md`.
