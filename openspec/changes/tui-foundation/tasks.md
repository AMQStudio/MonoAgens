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
- [x] 1.1 Add dev-dependencies `portable-pty`, `vt100`, `insta` (latest stable, pinned in `Cargo.lock`; check docs with context7) and the PTY driver `crates/cli/tests/support/pty.rs` (in `crates/cli` because only that crate defines the binary; `insta` lives in `crates/tui`) with `send_keys`, `wait_for_text`, `screen_text`. Acceptance: a test starts the binary with `scripted` and sees the composer.
- [x] 1.2 Fake stdio plugin `crates/tui/tests/support/fake_plugin.rs` (env: crash-after ms, stderr lines, startup latency, exit code). Acceptance: unit test that it writes `boom` and exits 1 after ~200 ms. *(Done in the gateway as `crates/gateway/examples/fake_plugin.rs`; accepts `FAKE_*` env or `--flag value` args. The `crates/tui/tests/support` copy is not needed; the TUI harness reuses the example binary.)*
- [x] 1.3 Fake provider HTTP server `crates/cli/tests/support/fake_provider.rs` serving `GET /models` and `POST /chat/completions` on `127.0.0.1:0`. Acceptance: test fetches `/models` and gets the configured list.
- [x] 1.4 `scripts/tui-smoke.sh` skeleton (tmux, home under `target/tui-smoke/`, scripted provider, fake plugin); checks are added by later tasks. Acceptance: script starts, sees the composer, quits cleanly, exits 0.

## 2. Gateway ownership (`crates/gateway`, `crates/cli`)
- [x] 2.1 Data-dir lock `<data_dir>/gateway.lock` (crate choice per design, confirm in review) taken by `monoagens serve` and by the TUI host path in `crates/gateway`; holder pid in the lock file. Acceptance: test with two lock attempts (second refused with pid); lock released when the holder is killed. *(`monoagens_gateway::acquire_data_dir_lock`, `crates/gateway/src/lock.rs`; `serve` takes it. The TUI host path takes it in 2.6.)*
- [x] 2.2 Admin socket server `<data_dir>/gateway.sock` (0600, NDJSON JSON-RPC, stale-socket replacement) with `status`, unknown-method error. Acceptance: test connects, calls `status`, checks mode 0600 and error code -32601. *(`crates/gateway/src/admin.rs`; `serve` does not serve the socket yet: 2.7.)*
- [x] 2.3 Admin methods `plugins.list/start/stop/restart`, `logs.subscribe` in the owner process. Acceptance: test restarts the fake plugin and sees new pid. *(`GatewayOwner`/`AdminServer` in `crates/gateway`, backed by `PluginSupervisor` and `RunLog`.)*
- [x] 2.4 `config.get` and `config.apply` (toml_edit, `.bak`, reload plugins and valve; provider hook left as a stub). Acceptance: test applies a valve patch, checks `.bak`, preserved comments and next-turn effect.
- [x] 2.5 `turn.submit` and `turn.cancel` (drop the `spawn_blocking` join, submit `Input::ModelFailure` with `cancelled_by_owner`). Acceptance: scripted provider with delay; replay of the log is bit-identical; second cancel returns `cancelled: false`; a result arriving after cancel is discarded (log has one `ModelFailure` and no `ModelResult` for that request).
- [x] 2.6 `GatewayClient` trait plus `InProcessClient`; refactor `crates/tui/src/runtime.rs` to call only the trait. Acceptance: existing `monoagens-tui` tests still pass. *(`GatewayClient` + `InProcessClient` in `crates/tui/src/client.rs`; `runtime.rs` only talks to the trait; plugins come from the gateway supervisor (autostart now also for the TUI); `TuiDoorController` forwards `reload_valve`.)*
- [x] 2.7 `SocketClient`; `monoagens serve` serves the socket; TUI tries the lock first, else attaches; peer-uid check. Acceptance: test that a status call through `SocketClient` matches `InProcessClient`. *(`SocketClient`; `serve` takes the lock, uses `RunLog` + supervisor and serves the admin socket; the hosting TUI serves it too; TUI tries the lock, else attaches. Admin methods added for the TUI: `events.since`, `commands.list/run`, `approvals.resolve`, `cues.subscribe`, `plugins.subscribe` use, `turn.submit` with a full `percept`.)*
- [x] 2.8 Attached `body-tui` chat channel through the socket; failed receipt for `cli:terminal` while no TUI is attached. Acceptance: PTY test: serve running, TUI attaches, a scripted message gets its reply; closing the TUI leaves serve running; the in-process TUI also serves the socket and a second TUI attaches to it. *(`body.attach` turns a socket connection into the Waist-2 `body-tui`; refused while the id or `cli:terminal` is owned, so a second TUI attached to a hosting TUI is a monitor with chat disabled; actions for `cli:terminal` with no TUI attached get a `refused` receipt (`ERR_UNKNOWN_PLACE`), covered by an end-to-end test.)*

## 3. Runtime log (`crates/tui/src/log.rs`, `render.rs`)
- [x] 3.1 `LogBuffer` ring (5 000), rotating file `<data_dir>/logs/tui.log` (5 MiB x 3), secret masking, sources; `scroll_window` and `render_scrollbar` helpers. Acceptance: unit tests for ring size, rotation, masking, scroll window edge cases. *(Gateway half done: `crates/gateway/src/runlog.rs` has the ring, rotating `<data_dir>/logs/gateway.log`, masking and subscription; `scroll_window`/`render_scrollbar` and the TUI file remain.)* *(TUI half done: `LogView` (filters, search, follow) over the gateway `RunLog`; the hosting TUI writes `<data_dir>/logs/tui.log`, `serve` writes `gateway.log`; `scroll_window` and `render_scrollbar` are public helpers in `render.rs`.)*
- [x] 3.2 `/log` overlay (level filter 1-4, source `s`, search `/`, follow `f`, `c` copy, scrollbar, wheel, Esc) and error toasts; `logs.subscribe` feeding an attached TUI. Acceptance: `TestBackend` test for filtering and scrollbar; snapshot at three sizes. *(`/log` overlay with level 1-4, `s`, `/`, `f`, `c`, scrollbar, wheel, Esc; error toast `<message> — /log for details`; `logs.subscribe` feeds an attached TUI; `turn.no_reply` is logged.)*

## 4. Plugin supervision (`crates/tui/src/runtime.rs`, `crates/gateway`)
- [x] 4.1 Autostart in dependency order at boot and after `CompleteSetupWizard`; failure isolation. Acceptance: test with plugins a/b/c from the spec scenario. *(Moved to `crates/gateway/src/supervisor.rs`; the TUI adopts it in 2.6.)*
- [x] 4.2 stderr capture to log + last-20 buffer; exit watcher; `Failed(reason)`; restart backoff 1/5/30 s with a fake clock; give up at the 4th failure in 10 min; Stop cancels. Acceptance: tests with fake plugin and injected clock; smoke script crash-then-restart check. *(Moved to the gateway; the smoke-script crash-then-restart check stays in 8.1.)*
- [x] 4.3 Per-plugin lock `<data_dir>/plugins/<id>.lock` and `RunningElsewhere` state (check what `serve` already does first). Acceptance: test with an externally held lock; no child spawned. *(`serve` had no per-plugin lock; the supervisor now takes it.)*

## 5. Turn lifecycle and input (`crates/tui/src/runtime.rs`, `render.rs`)
- [x] 5.1 Busy ends on reply / error / `TurnFinished` + 1.5 s grace; "chose not to reply" notice; `turn.no_reply` log. Acceptance: `TestBackend` scenario "Silent turn" with fake clock.
- [x] 5.2 Queue while busy with dim "queued" bubble, ordered submit, `Alt+Up` dequeue; remove input drain from full-redraw (keep after `pair`). Acceptance: tests for queue order, resize while queued, `Alt+Up`.
- [x] 5.3 Single busy indicator. Acceptance: render test counts spinner glyphs.

## 6. Terminal robustness (`crates/tui`)
- [x] 6.1 Resize/FocusGained redraw, `Alt+L` reset, `Ctrl+Z` suspend/resume. Acceptance: unit tests for event handling; PTY test for suspend/resume if feasible, else documented manual check.
- [x] 6.2 Width-safe rendering (grapheme + `unicode-width`, strip U+FE0F in chrome, `Appearance -> Icons`, control-char sanitising). Acceptance: test with `🛡️` and `⚙️` at 80 columns.
- [x] 6.3 Small-terminal layouts (<80 cols, <20 rows, <40x10). Acceptance: `TestBackend` at 60x15 and 30x8, plus snapshots at 80x24, 120x40, 200x50.

## 7. Startup robustness (`crates/tui`, `crates/cli`)
- [x] 7.1 Open the TUI on config error: banner, `/settings` on offending key, retry boot. Acceptance: test with `provider = "nonsense"`; PTY test. *(TUI opens on a config error: banner, chat disabled, `/settings` opens the offending key, `/retry` or `Ctrl+R` re-boots.)*
- [x] 7.2 `/doctor` checks per design A3. Acceptance: test with a missing plugin binary; scripted provider skips reachability. *(`/doctor` in `crates/tui/src/doctor.rs`; provider reachability is a TCP connect to the endpoint (no HTTP request, no model call).)*

## 8. Finish
- [x] 8.1 Complete `scripts/tui-smoke.sh` checks (autostart dot, `/log` line, crash-restart colours, silent-turn notice, attach-then-detach). Acceptance: script exits 0 locally. *(`scripts/tui-smoke.sh` runs autostart dot, crash red/green, `/log` line, silent-turn notice, reply, attach-then-detach with `serve`; uses a python3 fake provider.)*
- [x] 8.2 `openspec validate tui-foundation --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.
