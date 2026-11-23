# Tasks

Worker rules:
- Requires `rbac-guest-converse-and-reload` completed first.
- Run `source "$HOME/.cargo/env"` before `cargo`; use `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0` for builds/tests.
- Run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` before editing functions/methods.
- Never read `lab/scenarios/sealed/`; never run paid live models.
- Finish with `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/tui-smoke.sh`, and `node .gitnexus/run.cjs detect-changes --scope all --repo .`.

## 1. Test Harness & Core Primitives (`core/` + `tests/it/`)

- [x] 1.1 Add `unicode-normalization` (and singleton clipboard lock support if needed) to `crates/tui/Cargo.toml`, and scaffold `crates/tui/tests/it/main.rs` + `crates/tui/tests/it/common/mod.rs` with deterministic `TestBackend`, `FakeClock`, and `FakeClipboard` helpers (`cargo test -p monoagens-tui --test it` compiles).
- [x] 1.2 Implement `crates/tui/src/core/overlay.rs` (`OverlayStack`, `Overlay`, `FocusScope`, `EventOutcome`) and `KeyEventKind::Release` filtering, with contract tests in `tests/it/overlay_focus.rs` proving zero key/paste leakage to background composer or search bars.
- [x] 1.3 Implement `crates/tui/src/core/input.rs` (`TextInput`, `PasteTarget`, `sanitize_paste()` decoding tmux CSI-u/xterm controls, CRLF, NFC, tab expansion, C0 stripping, and `ClipboardPort` with Linux singleton `Mutex`), with unit + contract tests in `tests/it/paste_sanitization.rs` covering `Event::Paste` and `Ctrl+V` across modal prompts, wizard, and composer `[Paste #N, +L lines]` collapse.
- [x] 1.4 Implement `crates/tui/src/core/viewport.rs` (pure scroll/thumb math) and `crates/tui/src/core/hitmap.rs` (render-recorded `HitMap`, horizontal trackpad drift ignore, scrollbar drag mapping), plus `crates/tui/src/core/labels.rs` (`Friendly Label (tech.id)` catalog + deterministic Title-Case fallback humanizer); verify with unit tests in each module.

## 2. Reusable Widgets & Clean Chrome (`widgets/` + `core/theme.rs`)

- [x] 2.1 Remove post-render `normalize_chrome` emoji rewriting in favor of direct 1-cell glyph selection per `IconMode` in `core/theme.rs`, and ensure every overlay renders `Clear` + dimmed backdrop; verify on `TestBackend` that no underlying sidebar/chat text bleeds through modal borders.
- [x] 2.2 Implement `widgets/menu_list.rs` (`MenuSelection<T>` + scrollable list), `widgets/tab_bar.rs` (responsive short-label collapse + hit zones), `widgets/scrollbar.rs`, and `widgets/hub_frame.rs` (2-pane `SplitPane` + pinned 1-row `FooterMode::Hints | Chips | InlineInput`); verify `40x12` and `80x24` footer integrity via `TestBackend` tests.

## 3. Migrate Overlays, Chat, Settings, Catalog, Governance & Wizard (`views/`)

- [x] 3.1 Migrate Chat timeline, composer, sidebar, slash menu, `conv` modals (`/why`, `Ctrl+R`, `Ctrl+F`), and `?` Help / `Ctrl+K` Palette onto `OverlayStack`, `TextInput`, and `HitMap` (fixing composer click vs chat drag-select, `/why` mouse bleed, and `?` Help wheel offset); verify via `tests/it/mouse_hit_testing.rs` and `tests/it/conversation.rs`.
- [x] 3.2 Migrate `/log`, Catalog (`/model`, `/provider`, `/usage`), Governance (`/approvals`, `/people`, `/policy`, `/activity`, `/events`, `/status`), and 5-step Setup Wizard onto `OverlayStack`, `TextInput`, `MenuSelection`, and `HitMap` (fixing wizard paste & wheel bleed, adding row-click + scrollbar drag); verify via `tests/it/governance.rs` and `tests/it/mouse_hit_testing.rs`.
- [x] 3.3 Migrate `/settings` onto schema-driven `SettingDef` rows with `Friendly Label (tech.id)` formatting, dual label+ID search, in-place sub-editors, and pinned footer inputs (eliminating truncated modal hints and nested popup stacking); verify via `tests/it/settings_labels.rs`.

## 4. Ergonomic RBAC Simple ON/OFF Toggle Matrix (`views/rbac/`)

- [x] 4.1 Implement the 2-pane `HubFrame` RBAC Role editor powered by `capabilities.list` (`Core` + auto-detected running/stopped plugins + `Custom patterns`), Simple ON/OFF `[✓]`/`[ ]` toggles, `[🔒]` owner-only rows, group master toggles, safe explicit-list materialization on uncheck from wildcard, and live deterministic `RbacConfig::evaluate` Inspector; verify via `tests/it/rbac_toggle_matrix.rs` including TOML round-trip tests.
- [x] 4.2 Migrate RBAC Actors, Places, and Permission Bases onto the 2-pane `HubFrame` UX and route connected TUI config/RBAC reads through `GatewayClient::config_get()` + `config.apply`; verify attached-mode sync in `tests/it/rbac_toggle_matrix.rs`.

## 5. Decommission Legacy God-Files, Snapshot Churn & Final Verification

- [x] 5.1 Remove superseded legacy modules/helpers (`state.rs`, `render.rs`, `settings_ui.rs`, `rbac_editor.rs`, `list_editor.rs`, etc. replaced by `core/`, `widgets/`, `views/`), consolidate the old 7 integration test files + 35 full-screen `.snap` files into `crates/tui/tests/it/`, and ensure no module exceeds maintainable boundaries.
- [x] 5.2 Update `scripts/tui-smoke.sh` for the new RBAC toggle matrix & `/reload` flows and run `cargo test -p monoagens-tui`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/tui-smoke.sh`, `openspec validate tui-architecture-redesign --strict`, and `node .gitnexus/run.cjs detect-changes --scope all --repo .`.
