# Tasks — `tui-settings-complete`

## Worker rules
- One cargo command at a time, always with `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0`.
- Never build or create worktrees in `/tmp`.
- No live paid model calls: use the `scripted` provider and fakes.
- Before editing a symbol run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` and report callers, processes and risk.
- Finish each group with `cargo test` for the touched crates and `cargo clippy --workspace --all-targets -- -D warnings`.
- Never read `lab/scenarios/sealed/`.
- Read `design.md` first; the shared audit is [audit.md](../tui-foundation/audit.md).

## 1. Scrolling and navigation (`crates/tui/src/render.rs`, `state.rs`)
- [ ] 1.1 Slash menu above composer, max 10 rows, own scroll offset, scrollbar, wrap, PageUp/PageDown, wheel, click, Tab, stays open per D4; live-state descriptions. Acceptance: `TestBackend` "select the last command" and "plugins row" scenarios; snapshot at 80x24.
- [ ] 1.2 Scrollbars on settings lists, plugin detail and all existing pickers using `scroll_window`/`render_scrollbar`; Esc goes back one level. Acceptance: tests for scrollbar presence and the nested-Esc scenario.

## 2. Field kinds and writes (`crates/tui/src/settings.rs`, `crates/gateway`)
- [ ] 2.1 Field kinds Toggle/Select/Number/Text/Secret/List/Map/Path/Action and the enum selector widget (name, description, preview, options, search). Acceptance: unit tests for number range, list reorder, selector search-and-pick.
- [ ] 2.2 Route every edit through `config.apply` (and a no-echo secret write; method shape proposed in design, confirm in review). Acceptance: test with `SocketClient` attached to a test gateway changing a valve toggle; `.bak` present.
- [ ] 2.3 Coverage test enumerating parseable keys with a documented exclusion list (approach per design, confirm in review). Acceptance: test fails when a key is added without a row.
- [ ] 2.4 Convert every enum row to Select; add Embeddings section and the remaining Model-tab keys; valve toggle descriptions (C2). Acceptance: `output_limit_param` scenario; description footer scenario.

## 3. Interaction infrastructure (`crates/tui`)
- [ ] 3.1 Action table + `keybindings.toml` loader (defaults, conflict warnings to `/log`) + settings editor for bindings. Acceptance: conflict test; rebound key works.
- [ ] 3.2 `?`/`F1` help overlay generated from the action table. Acceptance: test shows a rebound key.
- [ ] 3.3 `Ctrl+K` palette (commands, settings rows, chats). Acceptance: "jump to a setting" scenario.
- [ ] 3.4 First-run wizard rebuilt on provider list, key test, model picker, plugin selection (reuses `model-provider-catalog` screens). Acceptance: wizard scenario against the fake provider server.
- [ ] 3.5 Providers and Model tabs using the `/login` and `/model` screens. Acceptance: switching model from the Model tab applies on the next turn.

## 4. RBAC editor (`crates/tui/src/settings.rs`, `crates/valve` read-only use)
- [ ] 4.1 Roles list and role page (tier, approval incl. inherit, label, context, settings Map), create/delete, `[rbac]` created on first edit, delete refusal with reason. Acceptance: create-role and delete-refused scenarios; the written TOML parses back into `RbacConfig`.
- [ ] 4.2 Allow/deny capability picker from connected bodies' `body.discover` + free text. Acceptance: picker scenario with a fake body.
- [ ] 4.3 Fallback role, actors (seen-actor list + typed id, multi-role), places, permission bases. Acceptance: one test per scenario in `tui-rbac-editor`.

## 5. Plugin management (`crates/tui`, `crates/gateway`)
- [ ] 5.1 Plugin detail page: state+reason, stderr tail, Start/Stop/Restart, enable toggle, View log filter, typed value editors. Acceptance: restart and array-value scenarios with the fake plugin.
- [ ] 5.2 Required-secret warnings from manifest/args with set action. Acceptance: telegram-token scenario.
- [ ] 5.3 In-TUI pairing page (first read how `monoagens pair` and the whatsapp plugin emit the code); proposed `plugins.pair.start/status` admin methods. Acceptance: pairing scenario with a fake pairing plugin.
- [ ] 5.4 Guided install, remove with confirm, build hint. Acceptance: missing-binary and remove-confirm scenarios.

## 6. Finish
- [ ] 6.1 Extend `scripts/tui-smoke.sh`: slash menu reaches the last item, settings scrollbar visible, nested Esc. Acceptance: script exits 0.
- [ ] 6.2 `openspec validate tui-settings-complete --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.
