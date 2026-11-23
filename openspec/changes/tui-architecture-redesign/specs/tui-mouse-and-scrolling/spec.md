# Spec Delta

## Purpose

Provides zero-drift render-recorded hit geometry (`HitMap`), pure viewport scroll math, and complete mouse click, wheel, and scrollbar support across every TUI view and overlay.

## ADDED Requirements

### Requirement: Render-Recorded `HitMap` and Universal Mouse/Scroll Support
Every TUI view, sidebar, tab bar, picker, settings/RBAC panel, Governance panel, `/log`, Catalog overlay, `?` Help, `Ctrl+K` Palette, and Setup Wizard SHALL record its rendered `Rect`s into `HitMap` and route mouse events top-down through `OverlayStack` with left-click selection/activation, wheel scrolling (ignoring horizontal drift), and vertical scrollbar click/drag.

#### Scenario: Clicking and scrolling inside RBAC and Catalog overlays
- **WHEN** the user opens Settings -> RBAC -> a Role page or `/model` and clicks a row or scrolls the mouse wheel
- **THEN** the target row selects/toggles and the list + scrollbar update without leaking events to the chat feed underneath.

### Requirement: Modal Mouse Isolation and Composer Click Safety
While any overlay is open, mouse clicks, drags, and wheel events SHALL NOT scroll, click, or start text selection in the underlying chat or sidebar. Clicking inside the chat composer SHALL NOT start a chat feed drag-selection.

#### Scenario: Setup Wizard and `/why` block background mouse scroll and clicks
- **WHEN** Setup Wizard or `/why` is open and the user scrolls the wheel or clicks inside the modal
- **THEN** the modal handles the scroll/click and `chat_scroll` / `selection` on the chat timeline remain untouched.
