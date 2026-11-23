# Design

## Context

See `proposal.md` for the verified audit of `crates/tui` (32k LOC, 6 dispatch chains, 3 Z-orders, paste/mouse/layout bugs, 35 full-screen snapshots, 7 test binaries) and `oh-my-pi` patterns (`/tmp/oh-my-pi/packages/tui`, `packages/coding-agent`, `crates/natives`).
Depends on `rbac-guest-converse-and-reload` for enriched `capabilities.list` metadata and `config.reload`.

## Goals / Non-Goals

**Goals:**
- Unify rendering, keyboard, paste, and mouse routing behind a single `OverlayStack` + `FocusScope` so input/focus leaks are structurally impossible.
- Standardize every text field on one `TextInput` / `PasteTarget` widget + sanitizer (`Event::Paste` + `Ctrl+V` with Linux singleton `Mutex`).
- Provide zero-drift mouse click, wheel, and scrollbar drag on every TUI surface via render-recorded `HitMap` and pure `viewport.rs` math.
- Replace manual RBAC list-editing modals with a 2-pane `HubFrame` Simple ON/OFF Toggle Matrix using `Friendly Label (tech.id)` naming.
- Decompose god-files (`state.rs`, `runtime.rs`, `render.rs`) into cohesive modules under `core/`, `widgets/`, `views/` and consolidate tests into `tests/it/`.

**Non-Goals:**
- Changing Core/Protocol/Waist-2 semantics or adding non-config authorization paths.
- Replacing `ratatui`/`crossterm` with an external UI framework.

## Decisions

### 1. Target Module Architecture (`crates/tui/src/`)
Split the monolithic files into focused modules (<500 LOC target per module):
```text
crates/tui/src/
  lib.rs
  core/
    mod.rs
    overlay.rs     # OverlayStack, OverlayId, FocusScope, EventOutcome
    input.rs       # TextInput, PasteTarget, sanitize_paste(), ClipboardPort, ClockPort
    viewport.rs    # Pure integer scroll math, variable-height windowing, scrollbar thumb
    hitmap.rs      # Render-recorded Rect -> HitTarget map + mouse router
    theme.rs       # ThemePalette & 1-cell IconMode glyph table (no post-render normalize_chrome)
    labels.rs      # Friendly Label (tech.id) catalog + deterministic Title-Case humanizer
  widgets/
    mod.rs
    menu_list.rs   # MenuSelection<T> (fuzzy filter, stable key, 2-step confirm) + scrollable render
    tab_bar.rs     # Responsive TabBar with short-label collapse & click hit zones
    hub_frame.rs   # 2-pane SplitPane + pinned Header/Footer strip (Hints | Chips | InlineInput)
    scrollbar.rs   # Interactive vertical scrollbar widget
  views/
    chat/          # Timeline, composer (with [Paste #N, +L lines] collapse), sidebar, /why, conv
    settings/      # Schema-driven SettingDef table, in-place sub-editors, plugin detail, pairing
    rbac/          # 2-pane RBAC Simple ON/OFF Toggle Matrix, Actors, Places, Bases, Custom patterns
    catalog/       # /model, /provider, /usage built on HubFrame/MenuSelection
    governance/    # /approvals, /people, /policy, /activity, /events, /status
    log/           # /log overlay
    wizard/        # 5-step setup wizard using shared TextInput + MenuSelection
    help_palette.rs# ?/F1 help and Ctrl+K command palette
  client.rs        # GatewayClient (InProcessClient, SocketClient) + config_get/config_reload
  runtime.rs       # TerminalGuard + tokio event loop delegating to AppController
```

### 2. Single `OverlayStack` & `EventOutcome` Contract
- `OverlayStack` holds an ordered `Vec<OverlayEntry>` (`Help`, `Palette`, `Log`, `Catalog`, `Governance`, `Settings`, `Wizard`, `ConvModal`).
- **One source of truth**: `render()` draws bottom-to-top (`Clear` + dimmed backdrop before each overlay); `handle_key()`, `handle_paste()`, and `handle_mouse()` dispatch exclusively to `stack.top_mut()` (except truly fatal `Ctrl+C`/`Ctrl+Q` quit or `Resize`, and `Ctrl+C` while typing cancels/clears or quits per explicit keybinding rule without corrupting state).
- Widgets return `enum EventOutcome { Consumed, Ignored, Close, Action(AppAction) }`. Leaf `TextInput` returns `Ignored` for navigation/Esc/Enter keys it does not own so parent `HubFrame`/`Settings` handles them cleanly.

### 3. Unified `TextInput`, Paste Sanitization & Clipboard Singleton
- **Decision**: Introduce `TextInput` (`value`, `cursor` byte/grapheme-safe, `multiline`, `masked`, `undo_stack`, `paste_text(&mut self, raw: &str)`).
- `sanitize_paste(raw, multiline)`:
  1. Decode tmux extended-keys re-encoded controls (`\x1b[<cp>;5u` and `\x1b[27;5;<cp>~`).
  2. Normalize `\r\n` / `\r` -> `\n`; for single-line, strip `\n` (or replace with space).
  3. Normalize Unicode to NFC (`unicode-normalization` crate) to prevent NFD width drift.
  4. Expand `\t` -> spaces and strip C0/DEL control chars (keeping `\n` iff `multiline`).
- `Ctrl+V` routes to the focused `PasteTarget` via injected `ClipboardPort`; production implementation keeps a `static CLIPBOARD: Mutex<Option<arboard::Clipboard>>` singleton (matching `oh-my-pi/crates/natives/src/clipboard.rs`) so X11 clipboard ownership survives.
- Main composer attaches clipboard images *only* when no overlay is open; all other inputs paste text only.

### 4. RBAC Simple ON/OFF Matrix & Explicit-List Materialization
- Left rail lists:
  - **Roles** section (`guest`, `member`, ..., `+ New role`) + quick role actions (`[b] Base tier`, `[a] Approval`, `[d] Delete`).
  - **Domains & Plugins** section populated from `capabilities.list` `groups`: `Core` (`agent.converse`), each plugin (`whatsapp`, `web-search`, `workspace`, `pdf-reader`, `soul-tuning`, `telegram`, etc.) with `●`/`○` status + `[on/total]` badge, plus `Custom patterns`.
- Right pane shows Simple ON/OFF toggles (`[✓]` / `[ ]`) and `[🔒]` locked rows (`owner_only`).
- **Wildcard/Group Uncheck Rule**:
  - Checking a group header writes `<prefix>.*` if all capabilities share one prefix and none are `owner_only`, OR writes all non-owner-only IDs explicitly.
  - Unchecking one capability from a group that currently matches via a wildcard (`chat.*`) replaces that wildcard in `roles.<name>.allow` with the explicit list of remaining enabled capabilities in that group. This guarantees zero hidden `deny` entries and zero surprise permissions if a plugin adds new capabilities later.
  - Live Inspector bottom pane evaluates `RbacConfig::evaluate` directly on `(role, selected_cap)` so the UI verdict is mathematically identical to Valve.

### 5. Friendly Labels & Humanizer (`core/labels.rs`)
- Table of curated friendly labels for built-in capabilities (`agent.converse` -> `Allow Conversation`, `chat.speak` -> `Send Messages`, etc.) and Valve/Settings keys (`valve.guest_access` -> `Allow Guests`, `line2_scope` -> `Privacy Boundary`, `taint_blocking` -> `Untrusted Input Shield`, etc.).
- Fallback humanizer for any unknown capability `<ns>.<action_name>`: strips namespace prefix, splits on `_`/`-`/`.`, converts to Title Case (`read_page` -> `Read Page`), formatted as `Read Page (pdf.read_page)`.

### 6. Consolidated Contract Test Harness (`crates/tui/tests/it/`)
- Replace the 7 separate `crates/tui/tests/*.rs` binaries and 35 full-screen `.snap` files with `crates/tui/tests/it/main.rs` + `common/mod.rs` + domain submodules.
- Tests render into `ratatui::backend::TestBackend` buffers with injected `FakeClock` & `FakeClipboard`, asserting semantic invariants (text presence in exact `Rect`, zero bleed-through outside overlay `Rect`, footer visible on last row at `40x12` and `80x24`, paste isolation, mouse click/wheel/scrollbar drag, RBAC TOML round-trip).

## Risks / Trade-offs

- **Large internal refactor of `crates/tui`** → Mitigated by keeping `GatewayClient` and `TuiLaunchOptions` public APIs intact, migrating views wave-by-wave (Core primitives -> Widgets -> Views -> Tests), and keeping `scripts/tui-smoke.sh` updated and passing as end-to-end PTY verification.
- **Replacing 35 `.snap` files** → Any existing snapshot assertions covering genuine regressions (such as slash menu scroll, RBAC role layout, narrow viewports) must be converted into explicit `TestBackend` contract assertions before removing redundant `.snap` files so zero behavioral coverage is lost.
