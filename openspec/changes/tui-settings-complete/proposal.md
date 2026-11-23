# Proposal — `tui-settings-complete`

Part 3 of 5. Audit: [audit.md](../tui-foundation/audit.md) (covers G1-G7, C2, C3, D3, D4, D6).

**Depends on `tui-foundation`** (admin socket, `config.apply`, `plugins.*`, `GatewayClient`, `scroll_window` and
`render_scrollbar` helpers, `/log`, PTY driver) and on **`model-provider-catalog`** (provider list, key
status, `/model` and `/login` screens that the Providers and Model tabs reuse, and `/usage`).

## Why
The requirement of the owner (2026-11-23): everything must be configurable inside the TUI without touching any
config file. Verified on 2026-11-23:
- RBAC is read-only: roles show as a summary string, Enter does nothing; actors, places and permission bases
  cannot be edited (C3).
- Settings lists have no scrollbar and Esc closes the whole overlay instead of going back one level (G3).
- The `/` menu renders only `area.height` rows with no scroll offset (`crates/tui/src/render.rs:1052`), so Down
  moves the selection off-screen and the menu looks frozen.
- No coverage guarantee exists that every config key is editable (G1); no selector widget (G2); only one hint
  line instead of per-screen help (G4); no keybinding file (G5); no command palette (G6); the first-run wizard
  asks for a raw endpoint (G7).
- Pairing suspends the TUI and runs a subprocess (D3); install is a bare key on a catalog row (D4); required
  secrets of a plugin are not shown (D6); valve toggles do not say what they do (C2).

## What Changes
1. **Field kinds and coverage (G1)**: `Toggle`, `Select`, `Number`, `Text`, `Secret`, `List`, `Map`, `Path`,
   `Action`; a coverage test that fails when a config key has no settings row or documented exclusion.
2. **Selectors, help, keybindings, palette, wizard (G2, G4-G7)** following omp patterns.
3. **Scrolling everywhere (G3)**: slash menu above the composer with scroll and scrollbar, scrollbars on every
   list, Esc goes back one level.
4. **RBAC editor (C3)** and **valve toggle explanations (C2)**.
5. **Plugin management (D3, D4, D6)**: detail page, in-TUI pairing page, guided install/remove, required secrets.
6. **All settings writes go through admin `config.apply`** so an attached TUI works the same as a hosting one.

## Impact
- Crates: `crates/tui` (most), `crates/gateway` (`config.apply` validation for RBAC patches, optional
  `plugins.pair.*` admin methods, a config key enumeration used by the coverage test).
- No core change. The valve stays deterministic and in the doors; the TUI only edits `[rbac]` and valve
  configuration through `config.apply`, and the valve reloads it. The editor can only narrow-by-config what the
  valve already supports; it adds no permission path.
- Invariants kept: *valve first and last in every door* and *other rules may only narrow what it allows* (RBAC
  edits go to the same `[rbac]` the valve reads; deleting a referenced role is refused); *gateway generic*
  (pairing is rendered from output the plugin emits, no platform code in the gateway); *protocol additive only
  by the owner* (no Waist-2 change; proposed admin methods are gateway-local); *core sees only opaque ids*.
- Config files stay backward compatible; new keys are additive (`keybindings.toml`, `tui.toml` keys).
