# Tasks — `tui-conversation`

## Worker rules
- One cargo command at a time, always with `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0`.
- Never build or create worktrees in `/tmp`.
- No live paid model calls: use the `scripted` provider and fakes.
- Before editing a symbol run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` and report callers, processes and risk.
- Finish each group with `cargo test` for the touched crates and `cargo clippy --workspace --all-targets -- -D warnings`.
- Never read `lab/scenarios/sealed/`.
- Read `design.md` first; the shared audit is [audit.md](../tui-foundation/audit.md).

## 1. Titles (`crates/tui/src/titler.rs`, `runtime.rs`)
- [ ] 1.1 Titler uses `output_limits.classify`, 30 s timeout, logs failures, deterministic keyphrase fallback. Acceptance: "provider fails to title" scenario.
- [ ] 1.2 Embedder-based topic shift when `[embed]` is configured; manual-rename lock kept. Acceptance: locked-title and embedder tests with a fake embedder.

## 2. Turn control (`crates/tui`)
- [ ] 2.1 Esc-twice interrupt via `turn.cancel` through `GatewayClient`, prompt timing with fake clock, "Interrupted." notice, `cancelled: false` path, queue kept. Acceptance: the four `tui-turn-control` interrupt scenarios (delayed scripted provider).
- [ ] 2.2 Retry (`F5`/`Alt+R`) enabled only after error/interrupt; action ids in the keybinding table. Acceptance: retry scenario.

## 3. Composer (`crates/tui/src/composer*`, `runtime.rs`)
- [ ] 3.1 `Ctrl+R` history search and cleared-draft recall. Acceptance: tests for both scenarios.
- [ ] 3.2 `Ctrl+G` external editor via file under `<data_dir>/cache/`, raw-mode leave/restore. Acceptance: test with a script editor; PTY test for return to the alternate screen.

## 4. What happened and message actions (`crates/gateway`, `crates/tui`)
- [ ] 4.1 Admin methods `events.query` (by trace) and `events.explain` (by event id); reuse the CLI `log` explain code. Acceptance: test through socket and in-process clients on a recorded scripted turn.
- [ ] 4.2 Collapsed what-happened line and `Ctrl+O` expansion. Acceptance: expand scenario.
- [ ] 4.3 Message selection (keys, mouse), copy, quote, `/why` overlay. Acceptance: quote and why scenarios.

## 5. Chats (`crates/tui`)
- [ ] 5.1 Chat search, pin (`pinned` flag, additive in `tui_chats.json`), delete confirmation. Acceptance: three chat scenarios.
- [ ] 5.2 `/export` Markdown. Acceptance: export scenario.

## 6. Notifications and images (`crates/tui`)
- [ ] 6.1 Focus tracking, bell, OSC 9/777, title badge, `Appearance -> Notifications` setting. Acceptance: tests on written escape sequences for `both` and `off`.
- [ ] 6.2 (Optional, last) kitty/sixel detection and inline preview; fallback card. Acceptance: no-graphics scenario; skip with a note if unstable.

## 7. Draft preview (`crates/reason`, `crates/gateway`, `crates/tui`)
- [ ] 7.1 Streaming variant of provider invoke (additive, default falls back to blocking) for `openai_chat` and `anthropic_messages`; usage recorded as before. Acceptance: fake-server streaming test comparing usage with non-streamed.
- [ ] 7.2 Incremental draft-text extractor over partial structured output (find the speech field first). Acceptance: unit tests splitting a JSON output at every byte offset.
- [ ] 7.3 `preview.subscribe` for the submitting TUI only (in-process channel + socket), never to other bodies, never an event. Acceptance: "adapter sees no draft" and "replay unchanged" scenarios; byte-equal final `chat.speak` test.
- [ ] 7.4 Draft bubble, resolution to reply/verdict/no-reply, removal on cancel/failure, `Appearance -> Live draft preview`. Acceptance: valve-denies and setting-off scenarios.

## 8. Finish
- [ ] 8.1 Extend `scripts/tui-smoke.sh`: interrupt with Esc Esc, `Ctrl+R`, `/export`, draft bubble with a delayed scripted stream. Acceptance: script exits 0.
- [ ] 8.2 `openspec validate tui-conversation --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.
