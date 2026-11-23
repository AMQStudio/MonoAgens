# Proposal

## Why

The current `crates/tui` (~32,047 LOC across 47 files) suffers from structural coupling, input/focus bugs, inconsistent mouse/scroll support, noisy technical UI copy, and expensive test churn:
1. **Broken focus isolation and paste routing**: Six independent dispatch chains (`render_to_buffer`, `handle_key_event`, `handle_paste`, `handle_mouse_event`/`ux_mouse`/`settings_wheel`, `text_input_active`, `conv_chat_blocked`) and three conflicting overlay Z-orders allow keystrokes and pastes in sub-editors to leak. Specifically, `handle_paste` routes `ItemEditor` (`Role: Allow`) pastes into `settings.search_query`, `Ctrl+V` is unreachable in every modal, typing on `RbacPage` writes to `search_query`, Setup Wizard paste checks a stale `w.editing` flag, and `KeyEventKind::Release` is not filtered.
2. **Broken/partial mouse and scroll behavior**: `has_subview()` disables all mouse clicks and wheel scrolling on RBAC sub-pages; Setup Wizard and `/why` leak scroll/clicks to chat behind them; `?` Help wheel fails to update scroll offset; `/model`, `/provider`, `/usage`, `/log`, and governance panels lack row-click and scrollbar dragging; `ItemEditor` hints truncate (`77` cols inside a hardcoded `72`-col box); undimmed modal backdrops bleed underlying sidebar/chat text on the left edge; and `normalize_chrome` creates misaligned double spaces.
3. **High-friction RBAC/Settings UX and technical jargon**: Editing role permissions requires typing raw glob strings (`a add`, `p pick`, `d delete`) and navigating multi-layer modal stacks while reading raw internal keys (`agent.converse`, `line2_scope`, `taint_blocking`).
4. **Test and token inefficiency**: Seven separate integration test binaries (7× Cargo linking with copy-pasted helpers) and 35 full-screen `insta` ASCII snapshots (188 KB across multiple viewport sizes) cause cascading multi-file diffs on minor layout adjustments.

## What Changes

- **Unified overlay stack, focus isolation, and shared `TextInput` / paste pipeline**: Replace the six ad-hoc dispatch chains with a single `OverlayStack` + `FocusScope` governing render, keyboard, paste, and mouse Z-order simultaneously; standardize all single-line and multiline inputs on a shared `TextInput` / `PasteTarget` with cursor editing, undo, `Event::Paste`, `Ctrl+V` smart paste (singleton `Mutex` clipboard on Linux), and central paste sanitization; render `Clear` + dimmed backdrops and pin headers/tabs/footers with `Constraint::Length(1)`; remove the post-render `normalize_chrome` pass.
- **Render-recorded `HitMap` and universal mouse/scroll support**: Record widget `Rect`s and row maps during render into `HitMap` so every view, modal, picker, and panel supports left-click selection/activation, wheel scrolling (ignoring horizontal trackpad drift), and interactive scrollbar click/drag with zero modal leak-through.
- **Ergonomic RBAC Simple ON/OFF Toggle Matrix**: Replace modal-in-modal `ItemEditor` lists with a 2-pane `HubFrame` RBAC matrix powered by `capabilities.list` (from `rbac-guest-converse-and-reload`), showing plugin/domain groups with running/stopped status, master group toggles, Simple ON/OFF `[✓]`/`[ ]` checkboxes, locked `[🔒]` owner-only indicators, a live deterministic `RbacConfig::evaluate` Inspector, safe explicit-list materialization when unchecking from a group, and `Custom patterns` via inline footer `TextInput`. Connected TUI reads config/RBAC via `GatewayClient`.
- **Human-friendly labels (`Friendly Label (tech.id)`) and in-place Settings**: Render clear friendly labels paired with dimmed technical IDs across RBAC capabilities, Valve toggles, and Settings rows, with a deterministic Title-Case fallback humanizer for unknown/third-party plugins and search matching both label and ID; replace nested edit popups with in-place sub-editors and pinned footer strips.
- **Modular file structure and consolidated contract test harness**: Split monolithic `state.rs` (2,699L), `runtime.rs` (2,099L), and `render.rs` (1,894L) into cohesive `core/`, `widgets/`, and `views/` modules; consolidate the seven integration test files into one binary (`crates/tui/tests/it/main.rs` + `common/mod.rs`) using deterministic `TestBackend` contract tests instead of brittle full-screen ASCII snapshots.

## Capabilities

### New Capabilities
- `tui-overlay-and-input-core`: Single-stack `OverlayStack`, `FocusScope` isolation, shared `TextInput` / `PasteTarget` sanitization, `Clear` + dimmed backdrop, and fixed-chrome layout without post-render emoji rewriting.
- `tui-mouse-and-scrolling`: Render-recorded `HitMap`, pure viewport math, and universal mouse click, wheel, and scrollbar drag across all TUI surfaces without modal bleed.
- `tui-rbac-toggle-matrix`: Plugin-detected 2-pane `HubFrame` RBAC Simple ON/OFF toggle matrix, group toggles, locked owner-only rows, live Inspector, safe explicit-list materialization, and `GatewayClient`-backed config reads.
- `tui-friendly-labels-and-settings`: Human-friendly `Label (id)` formatting (with deterministic fallback humanizer), dual-field search, and in-place/footer-strip Settings sub-editors.
- `tui-contract-test-harness`: Single consolidated `crates/tui/tests/it/` test binary with injected `Clock`/`Clipboard` traits and `TestBackend` contract tests replacing mass full-screen snapshots.

### Modified Capabilities
<!-- Builds on top of `rbac-guest-converse-and-reload`; does not modify main specs under `openspec/specs/`. -->

## Impact

- Crates affected: `crates/tui` (primary redesign and modularization into `core/`, `widgets/`, `views/`).
- Dependencies: `rbac-guest-converse-and-reload` must land first so `capabilities.list` metadata and `config.reload` exist.
- External dependencies in `crates/tui/Cargo.toml`: add `unicode-normalization` (NFC paste normalization) and `arboard` (or reuse existing clipboard path hardened with a Linux singleton `Mutex`).
- Architecture invariants preserved: TUI communicates with the gateway solely via `GatewayClient` (`config.apply`, `config.reload`, `capabilities.list`, `gov_call`); RBAC UI edits only `[roles]`, `[places]`, `[permission_bases]`, and `[[person]]` configuration and introduces no independent authorization path.
