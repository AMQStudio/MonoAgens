# Spec Delta

## Purpose

Standardizes human-friendly `Friendly Label (tech.id)` copy across RBAC, Settings, and Valve and replaces nested edit popups with in-place sub-editors and contextual footer strips.

## ADDED Requirements

### Requirement: Human-Friendly Labels (`Friendly Label (tech.id)`) Across RBAC, Settings, and Valve
Every RBAC capability, Valve toggle, and Settings row SHALL display a human-friendly label with its canonical ID in dimmed parentheses (e.g. `Allow Conversation (agent.converse)`, `Allow Guests (valve.guest_access)`). Unknown plugin capabilities SHALL use a deterministic Title-Case humanizer, and search SHALL match both label and ID.

#### Scenario: Searching by either friendly label or technical ID finds the same capability
- **WHEN** the user filters RBAC capabilities by `"send"` or by `"chat.speak"`
- **THEN** `Send Messages (chat.speak)` matches in both cases.

#### Scenario: Unknown third-party capability gets deterministic friendly label
- **WHEN** a plugin registers `pdf.read_page` with no curated label override
- **THEN** it renders as `Read Page (pdf.read_page)`.

### Requirement: In-Place Sub-Editors and Contextual Footer Strips
Settings value editing, enum selection, and RBAC custom pattern input SHALL render in-place inside the active Settings/Hub pane or inside a pinned 1-row contextual footer strip (`FooterMode::InlineInput` / `FooterMode::Chips`), rather than spawning nested modal boxes on top of Settings.

#### Scenario: Editing a text or enum setting stays inside Settings
- **WHEN** the user activates an enum or text setting in `/settings`
- **THEN** the selector or input opens in-place within the Settings frame, `Esc` returns directly to the row without losing scroll position, and pasting works cleanly.
