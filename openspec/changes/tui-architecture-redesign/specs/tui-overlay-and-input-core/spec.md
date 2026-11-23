# Spec Delta

## Purpose

Establishes a single `OverlayStack`, strict `FocusScope` isolation, a unified `TextInput` / `PasteTarget` sanitization pipeline, clean cleared backdrops, and fixed-chrome layouts across `crates/tui`.

## ADDED Requirements

### Requirement: Single OverlayStack Governs Render, Keyboard, Paste, and Mouse Z-Order
`crates/tui` SHALL manage all modals and popups through a single `OverlayStack` and `FocusScope` enum so render order, keyboard dispatch, paste, and mouse routing share one deterministic Z-order. Background inputs SHALL be unreachable while an overlay or sub-editor is active, and `KeyEventKind::Release` SHALL be ignored.

#### Scenario: Typing and pasting inside a sub-editor never leaks to Settings search
- **WHEN** Settings is open and a nested editor (such as an RBAC pattern editor or inline field editor) is active and the user types `'a'`, presses `Backspace`, or pastes text
- **THEN** only the active sub-editor buffer changes and the Settings `? Search:` filter query remains unchanged.

#### Scenario: Key release events do not double-trigger actions
- **WHEN** the terminal emits `KeyEventKind::Release` for any key
- **THEN** the TUI ignores the release event and does not insert duplicate characters or toggle controls twice.

### Requirement: Unified `TextInput` and Sanitized Paste Across Every Input
Every editable text field in `crates/tui` SHALL use the shared `TextInput` primitive supporting cursor movement, word deletion, `Event::Paste`, and `Ctrl+V` smart paste (with a Linux singleton clipboard `Mutex`). Images SHALL attach only when the main chat composer is focused, and large composer pastes (`> 10` lines or `> 1000` chars) SHALL collapse to `[Paste #N, +L lines]`.

#### Scenario: `Ctrl+V` and bracketed paste work inside Setup Wizard and modal text fields
- **WHEN** the user pastes an API key or capability pattern via `Event::Paste` or `Ctrl+V` inside Setup Wizard `StepKey`, Settings inline edit, or RBAC custom pattern input
- **THEN** the sanitized single-line text is inserted at the cursor of the focused field without touching the main chat composer.

### Requirement: Central Paste Sanitization Policy
Paste sanitization SHALL decode tmux re-encoded CSI-u and xterm control sequences, normalize CRLF and Unicode NFC, expand tabs, and strip C0/DEL control bytes (preserving `\n` only for multiline inputs).

#### Scenario: Pasting tmux re-encoded controls or NFD Unicode text
- **WHEN** bracketed paste delivers `\x1b[106;5u` or NFD-decomposed Unicode text into a single-line or multiline `TextInput`
- **THEN** control sequences are decoded cleanly without leaking escape tails, text is NFC-normalized, and single-line inputs strip newlines.

### Requirement: Clean Overlay Backdrop and Fixed-Chrome Layout
Every overlay SHALL render `ratatui::widgets::Clear` and a dimmed backdrop so underlying glyphs never bleed through borders. Headers, tabs, and footers SHALL use `Constraint::Length(1)` slots before `Constraint::Min(...)` bodies, and post-render `normalize_chrome` emoji replacement SHALL be removed in favor of 1-cell glyphs per `IconMode`.

#### Scenario: Modal border and footer integrity at 80x24 and 40x12
- **WHEN** Settings or an RBAC panel renders at `80x24` or `40x12` over a populated chat and sidebar
- **THEN** no underlying sidebar/chat characters appear inside or bleeding across the modal edges, and the bottom hint/footer row remains visible and untruncated.
