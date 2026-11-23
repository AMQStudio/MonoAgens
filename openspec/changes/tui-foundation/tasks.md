# Tasks — `tui-foundation`

## Worker rules
- One cargo command at a time, always with `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0`.
- Never build or create worktrees in `/tmp`.
- No live paid model calls: use the `scripted` provider and fakes.
- Before editing a symbol run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` and report callers, processes and risk.
- Finish each group with `cargo test` for the touched crates and `cargo clippy --workspace --all-targets -- -D warnings`.
- Never read `lab/scenarios/sealed/`.
- Read `design.md` first; the shared audit is [audit.md](audit.md).

## 1. Verification harness first (`crates/tui/tests/support`, `scripts/`)
- [ ] 1.1 Add dev-dependencies `portable-pty`, `vt100`, `insta` (latest stable, pinned in `Cargo.lock`; check docs with context7) and the PTY driver `crates/tui/tests/support/pty.rs` with `send_keys`, `wait_for_text`, `screen_text`. Acceptance: a test starts the binary with `scripted` and sees the composer.
- [ ] 1.2 Fake stdio plugin `crates/tui/tests/support/fake_plugin.rs` (env: crash-after ms, stderr lines, startup latency, exit code). Acceptance: unit test that it writes `boom` and exits 1 after ~200 ms.
- [ ] 1.3 Fake provider HTTP server `crates/tui/tests/support/fake_provider.rs` serving `GET /models` and `POST /chat/completions` on `127.0.0.1:0`. Acceptance: test fetches `/models` and gets the configured list.
- [ ] 1.4 `scripts/tui-smoke.sh` skeleton (tmux, home under `target/tui-smoke/`, scripted provider, fake plugin); checks are added by later tasks. Acceptance: script starts, sees the composer, quits cleanly, exits 0.

## 2. Gateway ownership (`crates/gateway`, `crates/cli`)
- [ ] 2.1 Data-dir lock `<data_dir>/gateway.lock` (crate choice per design, confirm in review) taken by `monoagens serve` and by the TUI host path in `crates/gateway`; holder pid in the lock file. Acceptance: test with two lock attempts (second refused with pid); lock released when the holder is killed.
- [ ] 2.2 Admin socket server `<data_dir>/gateway.sock` (0600, NDJSON JSON-RPC, stale-socket replacement) with `status`, unknown-method error. Acceptance: test connects, calls `status`, checks mode 0600 and error code -32601.
- [ ] 2.3 Admin methods `plugins.list/start/stop/restart`, `logs.subscribe` in the owner process. Acceptance: test restarts the fake plugin and sees new pid.
- [ ] 2.4 `config.get` and `config.apply` (toml_edit, `.bak`, reload plugins and valve; provider hook left as a stub). Acceptance: test applies a valve patch, checks `.bak`, preserved comments and next-turn effect.
- [ ] 2.5 `turn.submit` and `turn.cancel` (drop the `spawn_blocking` join, submit `Input::ModelFailure` with `cancelled_by_owner`). Acceptance: scripted provider with delay; replay of the log is bit-identical; second cancel returns `cancelled: false`; a result arriving after cancel is discarded (log has one `ModelFailure` and no `ModelResult` for that request).
- [ ] 2.6 `GatewayClient` trait plus `InProcessClient`; refactor `crates/tui/src/runtime.rs` to call only the trait. Acceptance: existing `monoagens-tui` tests still pass.
- [ ] 2.7 `SocketClient`; `monoagens serve` serves the socket; TUI tries the lock first, else attaches; peer-uid check. Acceptance: test that a status call through `SocketClient` matches `InProcessClient`.
- [ ] 2.8 Attached `body-tui` chat channel through the socket; failed receipt for `cli:terminal` while no TUI is attached. Acceptance: PTY test: serve running, TUI attaches, a scripted message gets its reply; closing the TUI leaves serve running; the in-process TUI also serves the socket and a second TUI attaches to it.

## 3. Runtime log (`crates/tui/src/log.rs`, `render.rs`)
- [ ] 3.1 `LogBuffer` ring (5 000), rotating file `<data_dir>/logs/tui.log` (5 MiB x 3), secret masking, sources; `scroll_window` and `render_scrollbar` helpers. Acceptance: unit tests for ring size, rotation, masking, scroll window edge cases.
- [ ] 3.2 `/log` overlay (level filter 1-4, source `s`, search `/`, follow `f`, `c` copy, scrollbar, wheel, Esc) and error toasts; `logs.subscribe` feeding an attached TUI. Acceptance: `TestBackend` test for filtering and scrollbar; snapshot at three sizes.

## 4. Plugin supervision (`crates/tui/src/runtime.rs`, `crates/gateway`)
- [ ] 4.1 Autostart in dependency order at boot and after `CompleteSetupWizard`; failure isolation. Acceptance: test with plugins a/b/c from the spec scenario.
- [ ] 4.2 stderr capture to log + last-20 buffer; exit watcher; `Failed(reason)`; restart backoff 1/5/30 s with a fake clock; give up after 3 in 10 min; Stop cancels. Acceptance: tests with fake plugin and injected clock; smoke script crash-then-restart check.
- [ ] 4.3 Per-plugin lock `<data_dir>/plugins/<id>.lock` and `RunningElsewhere` state (check what `serve` already does first). Acceptance: test with an externally held lock; no child spawned.

## 5. Turn lifecycle and input (`crates/tui/src/runtime.rs`, `render.rs`)
- [ ] 5.1 Busy ends on reply / error / `TurnFinished` + 1.5 s grace; "chose not to reply" notice; `turn.no_reply` log. Acceptance: `TestBackend` scenario "Silent turn" with fake clock.
- [ ] 5.2 Queue while busy with dim "queued" bubble, ordered submit, `Alt+Up` dequeue; remove input drain from full-redraw (keep after `pair`). Acceptance: tests for queue order, resize while queued, `Alt+Up`.
- [ ] 5.3 Single busy indicator. Acceptance: render test counts spinner glyphs.

## 6. Terminal robustness (`crates/tui`)
- [ ] 6.1 Resize/FocusGained redraw, `Alt+L` reset, `Ctrl+Z` suspend/resume. Acceptance: unit tests for event handling; PTY test for suspend/resume if feasible, else documented manual check.
- [ ] 6.2 Width-safe rendering (grapheme + `unicode-width`, strip U+FE0F in chrome, `Appearance -> Icons`, control-char sanitising). Acceptance: test with `🛡️` and `⚙️` at 80 columns.
- [ ] 6.3 Small-terminal layouts (<80 cols, <20 rows, <40x10). Acceptance: `TestBackend` at 60x15 and 30x8, plus snapshots at 80x24, 120x40, 200x50.

## 7. Startup robustness (`crates/tui`, `crates/cli`)
- [ ] 7.1 Open the TUI on config error: banner, `/settings` on offending key, retry boot. Acceptance: test with `provider = "nonsense"`; PTY test.
- [ ] 7.2 `/doctor` checks per design A3. Acceptance: test with a missing plugin binary; scripted provider skips reachability.

## 8. Finish
- [ ] 8.1 Complete `scripts/tui-smoke.sh` checks (autostart dot, `/log` line, crash-restart colours, silent-turn notice, attach-then-detach). Acceptance: script exits 0 locally.
- [ ] 8.2 `openspec validate tui-foundation --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.
