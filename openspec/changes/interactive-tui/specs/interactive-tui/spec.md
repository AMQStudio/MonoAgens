# Specification — `interactive-tui`

## ADDED Requirements

### Requirement: Default Interactive Chat TUI with Borderless Aesthetics and Local `CHATS` Sidebar
Invoking `monoagens` without a subcommand in an interactive TTY (or `monoagens tui`) SHALL launch a pure-Rust `ratatui` + `crossterm` + `tachyonfx` chat TUI with right-aligned dark user bubbles (no `You`/`Mono` labels in 1:1 local chats), left-aligned replies, inline `/` autocomplete, generic Valve approval cards, and a `Ctrl+B` sidebar showing local `CHATS` (one persistent soul) and `PLUGINS`.

#### Scenario: Local chats render with right-aligned dark user bubbles and toggleable sidebar
- **WHEN** the TUI renders a local 1:1 chat exchange on a wide viewport (`>= 120` columns) and then toggles the sidebar with `Ctrl+B`
- **THEN** the user message renders right-aligned in a dark bubble without a `You` label, Mono's reply renders left-aligned without a `Mono` label, the left sidebar displays `CHATS` and `PLUGINS` when open, and `Ctrl+B` hides and restores the sidebar cleanly.

### Requirement: Three-Color Live Plugin Status Indicators and Kind Badges
The `PLUGINS` sidebar and `/settings -> Plugins` view SHALL display every plugin's kind (`Adapter` or `Capabilities`) alongside a three-state color indicator: **green** for running/connected, **yellow** for starting or stopping transitions, and **red** for stopped, disabled, or failed.

#### Scenario: Plugin lifecycle transitions update sidebar status colors accurately
- **WHEN** a plugin transitions from stopped -> starting -> running -> stopping -> stopped (or failed)
- **THEN** the TUI renders red for stopped/failed, yellow for starting/stopping, and green for running alongside its `(Adapter)` or `(Capabilities)` label.

### Requirement: LLM Automatic Chat Naming and Topic-Shift Re-Titling
The TUI SHALL automatically name a local chat using the configured LLM on its first message and automatically update the title when the conversation topic shifts, while respecting manual `/chat rename` locks and degrading gracefully offline without blocking chat turns.

#### Scenario: First message titles a chat and a topic shift updates the title unless locked
- **WHEN** a user sends a first message in an untitled local chat and later shifts to a distinctly different topic
- **THEN** the background titler assigns a concise title from the first message and updates the title after the topic shift, whereas a chat renamed via `/chat rename` remains locked and unchanged.

### Requirement: Command-Gated `/lab` Mode Without Header Mode Tabs
The TUI SHALL NOT display header mode tabs or a `Code (Coming Soon)` placeholder; entering Lab simulation mode SHALL occur exclusively via `/lab`, while `/exit` inside Lab mode returns to normal chat without quitting the harness and `/exit` in normal chat quits MonoAgens.

#### Scenario: `/exit` leaves Lab mode first and exits the harness only from normal mode
- **WHEN** the TUI starts in normal chat mode, enters `/lab`, invokes `/exit`, and later invokes `/exit` again
- **THEN** `/as` and brain telemetry stay hidden initially, activate after `/lab`, deactivate after the first `/exit` without closing the TUI, and the second `/exit` quits the TUI cleanly.

### Requirement: Comprehensive 8-Tab `/settings` Overlay, Generic Plugin Config Editor, Pairing, and Setup Wizard
The TUI SHALL provide an 8-tab 2-pane `/settings` overlay (`Appearance`, `Soul & Brain`, `Model`, `Valve`, `RBAC`, `Providers`, `Plugins`, `Runtime`) with live search, format-preserving `monoagens.toml` + `tui.toml` + `0600` masked `secrets.toml` persistence, a drill-down `Plugins` editor for `plugins/<id>.toml` and raw-mode-safe `Pair Device`, and a first-run Setup Wizard when credentials are missing.

#### Scenario: Settings edits persist cleanly and plugin drill-down edits `plugins/<id>.toml`
- **WHEN** the user toggles a Valve or plugin option in `/settings` or drills into a plugin to edit its `<id>.toml`
- **THEN** changes persist atomically via `toml_edit` (backing up `monoagens.toml`), secrets remain masked in UI/logs, and arbitrary keys in `plugins/<id>.toml` round-trip cleanly.

### Requirement: Full Mouse Support, Bracketed Paste, and Select-to-Auto-Copy
The TUI SHALL enable mouse capture and bracketed paste by default (toggleable in `/settings -> Appearance`), supporting wheel scrolling, left-click interaction across all panes/controls, and mouse drag-selection over chat text that highlights the selection and automatically copies it to the system clipboard on release via OSC 52 (with native clipboard fallback and `/copy`).

#### Scenario: Mouse interactions and drag-select auto-copy work deterministically
- **WHEN** mouse wheel, left-click, or drag-select events target the chat viewport, sidebar entries, settings tabs, or an approval button
- **THEN** scrolling, selection, approval actions, and automatic clipboard copy on drag release update deterministically without requiring keyboard input.

### Requirement: Smart Attachment Cards, Long-Text Paste Collapse, and Plugin-Gated File Paths
The TUI SHALL support `/attach <path>`, `/detach [id]`, `Ctrl+V` clipboard image paste, and bracketed-paste detection turning images (with `WxH` dimensions) and long pasted text (`>= 8` lines or `>= 600` chars) into numbered preview cards (`#1`, `#2`, ...). PDFs SHALL require `pdf-reader` (or `workspace`) to be active, and other host files SHALL import into the soul's sandboxed workspace (`inbox/<file>`) only when `workspace` is active.

#### Scenario: Native media and long text attach directly while PDFs and non-media files respect active plugins
- **WHEN** a user pastes an image, long multi-line text, a PDF, and a `.jar` file with plugins stopped vs. `pdf-reader` and `workspace` running
- **THEN** the image and long text become numbered preview cards in both cases, the PDF attaches as a `📄 #N` card when `pdf-reader` is running (and is refused when neither `pdf-reader` nor `workspace` is running), and the `.jar` is copied into the soul's workspace sandbox and inserted as `inbox/<name>` only when `workspace` is running.
