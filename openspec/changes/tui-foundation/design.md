# Design — `tui-foundation`

Audit and owner decisions: [audit.md](audit.md).

## Context for implementers (facts checked on 2026-11-23)
- TUI event loop: `crates/tui/src/runtime.rs` (`run_event_loop` ~L360, `handle_app_action` ~L500,
  `handle_runtime_event` ~L833, `spawn_single_plugin` ~L1483, TUI body `ActionExecute` -> `chat.speak` ~L1275).
- `GatewayHandle::submit_and_wait` returns `TurnOutcome { effects_spawned, next_due }` after the turn's
  owner step; `chat.speak` reaches the TUI separately as a body `ActionExecute`.
- `ModelProvider::invoke` is **blocking** (`crates/reason/src/model/mod.rs:312`); call it from `spawn_blocking`.
- Config: `crates/gateway/src/config.rs` (`ModelConfig` L172, `ModelProviderKind` L37, `EmbedConfig` L321,
  `PluginConfig` L105). Secrets: `crates/reason/src/secrets.rs` (`ProviderSecrets`).
- Existing core input used for cancel: `Input::ModelFailure { request_id: EventId, reason: String }`
  (`crates/core/src/lib.rs:136`, verified 2026-11-23). Model calls run in `crates/gateway/src/provider.rs`
  (`spawn_blocking(move || provider.invoke(&req))` ~L273, result mapped to `Input::ModelResult` ~L339).
- ratatui 0.30 scrollbar: `Scrollbar::new(ScrollbarOrientation::VerticalRight)` rendered with
  `render_stateful_widget(scrollbar, area, &mut ScrollbarState::new(total).position(offset).viewport_content_length(visible))`.
  `content_length` must be set or the bar renders blank.
- `Cargo.lock` has `unicode-width` and `unicode-segmentation` already (via ratatui). It has no file-lock crate,
  no `portable-pty`, no `vt100`, no `insta` today.

## D-1. Gateway ownership: lock + attach
- **Lock**: `<data_dir>/gateway.lock`, exclusive and non-blocking, held for the process life. Decided 2026-11-23: no crate; use
  std `File::try_lock` (stable since Rust 1.89; toolchain here is 1.100). The lock file
  may hold the holder's pid for the error message.
- **Admin socket**: `<data_dir>/gateway.sock`, unix socket, mode 0600, NDJSON JSON-RPC framed like Waist-2
  (reuse its framing code). Created only by the lock holder, removed on clean exit, stale sockets (no lock
  holder) are replaced.
- **Initial admin methods** (from the owner's decision D-1): `status`, `config.get`, `config.apply(patch)`,
  `plugins.list/start/stop/restart`, `logs.subscribe`, `events.query`, `events.explain`, `approvals.list/resolve`,
  `models.discover`, `turn.submit`, `turn.cancel`, `activity.subscribe`, `preview.subscribe`.
  This change implements the socket framework plus: `status`, `config.get`, `config.apply`, `plugins.*`,
  `logs.subscribe`, `turn.submit`, `turn.cancel`. The rest are added by the change that needs them
  (`models.discover`: `model-provider-catalog`; `events.*`: `tui-conversation` then `tui-governance-observability`;
  `approvals.*`, `activity.subscribe`: `tui-governance-observability`; `preview.subscribe`: `tui-conversation`).
  Request/response JSON shapes: decided as below (2026-11-23).
- **`config.apply(patch)`**: the owner process writes the file with `toml_edit` plus a `.bak` backup, then
  hot-reloads what changed: provider rebuild, plugin restart, valve reload. Both client impls call it, so an
  attached TUI and an in-process TUI behave the same. Provider rebuild itself is implemented in
  `model-provider-catalog`; this change wires reload for plugins and valve and leaves the provider hook.
- **`GatewayClient` trait** (in `crates/tui`, or `crates/gateway` if the CLI needs it - verify during
  implementation): async methods mirroring the admin methods plus a stream of runtime events and the
  `body-tui` chat channel. Impls: `InProcessClient` (TUI won the lock, hosts the gateway, and also serves the
  socket so another TUI can attach) and `SocketClient` (attached to `monoagens serve` or another TUI).
  The TUI code never matches on which impl it has.
- **Attached chat body**: the attached TUI's `body-tui` is served through the same socket so its `chat.speak`
  actions reach it (the owner process routes actions for the TUI place to the socket connection).
- **No TUI attached**: actions routed to the TUI place (`cli:terminal`) while no TUI is connected get an
  honest failed receipt (body offline), exactly like any other disconnected body; nothing is queued for later.
- **Socket access**: besides mode 0600, the server checks the peer's uid (`SO_PEERCRED` /
  `UnixStream::peer_cred`) and refuses other users.
- **Detach**: closing an attached TUI closes only its connection; daemon and plugins keep running. Closing the
  hosting TUI stops the gateway as today (and any attached TUI shows "owner went away" and offers to retry).
- **CLI**: `monoagens serve` takes the same lock and serves the same socket. Starting `serve` while a TUI
  holds the lock refuses with the holder's pid and a hint to attach.
- Invariant check: the lock keeps one gateway and therefore one queue and one consumer per data dir.

### Shapes (decided 2026-11-23)
- Request `{"jsonrpc":"2.0","id":n,"method":"plugins.restart","params":{"id":"whatsapp"}}`; result
  `{"ok":true}` or a JSON-RPC error `{code,message}`. Subscriptions (`logs.subscribe`) answer with a
  subscription id then push `{"method":"logs.entry","params":{...LogEntry}}` notifications.
- `status` result: `{owner:{pid,mode:"tui"|"serve"}, data_dir, provider, model, embedder, plugins:[{id,state,reason}], queue_depth}`.

## D-2. Turn cancel in the gateway
- `turn.cancel` aborts waiting on the in-flight model call (drop the `spawn_blocking` join handle; the blocking
  HTTP call is abandoned and finishes on its own thread without effect) and submits
  `Input::ModelFailure { request_id, reason: "cancelled_by_owner" }` so the cancel is recorded as an event
  and `monoagens replay` stays bit-identical. No core change.
- If no model call is in flight, `turn.cancel` returns `{"cancelled":false}` and submits nothing.
- **Late results are discarded.** The provider task keeps a per-`request_id` cancel flag; once cancelled, a
  result that still arrives from the abandoned blocking call is dropped (logged `provider` info
  "late result discarded") and never submitted, so the log never holds both a `ModelFailure` and a
  `ModelResult` for one request. The cancel and the normal completion race through one atomic
  compare-and-set: exactly one of them submits.
- The cancel UI (Esc twice) is built in `tui-conversation`.

## D1. Plugin supervision
- At gateway boot (`boot_tui_gateway` and after `CompleteSetupWizard`) the TUI computes start order from
  `PluginConfig.dependencies` (topological; a cycle or missing hard dependency marks the plugin `Failed` with a
  reason, others still start) and spawns every `enabled = true` plugin. State goes `Starting` (yellow) first.
- stderr is piped; a reader task forwards each line to the log as `source = plugin:<id>` and keeps the last
  20 lines per plugin.
- A watcher task awaits `child.wait()`. Exit while not stopping -> `Failed("exited <code>: <last stderr line>")`,
  then restart with backoff 1 s, 5 s, 30 s; after 3 failures inside 10 minutes stop retrying and stay red.
  A user Stop cancels the restart loop. A body RPC disconnect is treated like an exit.
- **One owner per plugin**: with the data-dir lock, only the lock holder spawns plugins. The per-plugin
  advisory lock `<data_dir>/plugins/<id>.lock` (held for the child's life) stays as a second guard for a plugin
  started outside the gateway (for example a standalone `monoagens plugin` run). If another process holds it,
  state is `RunningElsewhere` (green ring `◉`, label "running in another MonoAgens") and nothing is spawned.
  Check first whether `monoagens serve` already uses such a lock and reuse it - verify during implementation.
- Sidebar click on a plugin opens its detail page (the page itself is built in `tui-settings-complete`; until
  then it opens the existing drill-down).
- Under D-1 the supervisor lives in the lock-holding process; an attached TUI reads plugin state through
  `plugins.list` and `logs.subscribe`.

## D2. Turn lifecycle
- Busy starts on `SendUserMessage`. It ends on the **first** of: `AssistantReply`, `TurnError`, or
  `TurnFinished` followed by a 1.5 s grace window with no `chat.speak` (replies can arrive just after the
  owner step returns). If the grace window expires with no reply, push a muted `SystemNotice`
  "Mono chose not to reply to this message." and log `turn.no_reply` with `effects_spawned`.
- While busy the composer stays editable; Enter queues the message and the queue is submitted when busy ends
  (shown as a dim "queued" bubble; `Alt+Up` pulls it back to edit - audit B3). No input is ever dropped: remove
  the `key_rx`/`crossterm` drain from the full-redraw path; keep a drain only right after returning from the
  interactive `pair` subprocess, where the child consumed the terminal.
- Exactly one busy indicator: keep the status-bar spinner + elapsed time; the phase line shows the phase text
  without its own spinner glyph, or is removed when the phase text equals the status text.

## A3. Startup with bad config and `/doctor`
- A config parse or validation error no longer exits. The TUI opens in a "config error" state: a banner shows
  the error text, `/settings` opens on the offending key when it can be named, and chat is disabled until the
  gateway can boot (a "Retry boot" action reloads the config). Today's behaviour (verified): print and exit 1.
- `/doctor` runs checks and lists each with pass/fail and a fix hint: config parses, secrets file present and
  mode 0600, provider reachable (one cheap request; skipped for `scripted`), plugin binaries exist and are
  executable, embedder files exist, data dir writable, lock owner. Reuse `monoagens` CLI check code if any
  exists - verify during implementation. No paid model call.

## A5/A6. Resize, focus, suspend, small terminals
- `Event::Resize` and `Event::FocusGained` force a redraw (no state change). `Alt+L` forces a full display reset.
- `Ctrl+Z` suspends: leave alternate screen and raw mode, `SIGTSTP` self; on `SIGCONT` re-enter raw mode and
  alternate screen and redraw. Exact signal handling API: verify during implementation (context7 for `crossterm`).
- Layout below 80 columns: sidebar hidden and cannot open; status bar collapses to one line. Below 20 rows:
  header and hint line hidden, composer 1-3 lines, overlays use full height. Below 40x10 draw only
  "terminal too small (needs 40x10)".

## D3. Runtime log (`/log`)
- `crates/tui/src/log.rs`: `LogBuffer` ring of 5 000 `LogEntry { ts_ms, level: Debug|Info|Warn|Error, source, message }`,
  shared via `Arc<Mutex<..>>` or a channel into `TuiAppState`. In an attached TUI, entries from the daemon
  arrive through `logs.subscribe` and local entries are merged.
- Also appended to `<data_dir>/logs/tui.log` (rotate at 5 MiB, keep 3). Secrets are never logged; reuse the
  masking helpers used for `secrets.toml` display.
- Sources: `turn`, `plugin:<id>`, `provider`, `discovery`, `titler`, `settings`, `gateway`.
- `/log` opens a full-height overlay: level filter (1-4 keys), source filter (`s` cycles), live search (`/`),
  follow-tail toggle (`f`, default on), scrollbar, mouse wheel, `c` copies the visible selection, Esc closes.
- Toast on every `Error` entry: "... - /log for details".
- Shared helper `scroll_window(selected, offset, visible, total) -> offset` and
  `render_scrollbar(area, total, offset, visible)` are introduced here for `/log`; `tui-settings-complete`
  applies them to every other list and the slash menu.

## D8. Rendering width
- Compute display width with `unicode-width` on grapheme clusters (`unicode-segmentation`) and strip U+FE0F
  from UI chrome labels, or replace emoji icons in chrome with single-width symbols when `TERM`/settings say so
  (`Appearance -> Icons: emoji | symbols`, default `symbols` for chrome, emoji stays in chat content).
  Every row is width-fit by display width and tabs/control characters are sanitised.

## D9 / H1-H3. Verification harness plan
- **`scripts/tui-smoke.sh`** (tmux): starts `target/debug/monoagens` with `MONOAGENS_HOME` pointing at a temp
  config dir on disk (not `/tmp` tmpfs: use `target/tui-smoke/`), the `scripted` provider and the fake stdio
  plugin; drives it with `tmux send-keys`; asserts with `tmux capture-pane -p`: autostart dot green, slash menu
  scrolls to the last item (after `tui-settings-complete`; until then the check is skipped), `/log` shows the
  plugin's stderr line, killing the fake plugin turns the dot red then green after restart, a scripted turn with
  no speak ends busy with the notice, second TUI attaches to the first (or to `serve`) and closing it leaves the
  daemon running. No paid model calls.
- **Rust PTY driver** (`crates/tui/tests/support/pty.rs`): `portable-pty` spawns the binary in a PTY, `vt100`
  parses the screen, helpers `send_keys`, `wait_for_text(timeout)`, `screen_text`. Lets screen assertions run in
  `cargo test`. Crate versions: latest stable via context7 before adding - verify during implementation.
- **Snapshots**: `insta` snapshots of key screens (chat, settings, `/log`, plugin sidebar) at 80x24, 120x40 and
  200x50 using `TestBackend`.
- **Fakes**: `crates/tui/tests/support/fake_plugin.rs` (a small stdio plugin binary or script speaking
  Waist-2 with env-configurable crash-after-N-ms, stderr lines, startup latency) and
  `fake_provider.rs` (a tiny HTTP server on `127.0.0.1:0` serving `/models` and `/chat/completions`, used by
  `model-provider-catalog` tests).

## Decisions taken 2026-11-23 (owner delegated the choice)
- Lock: std `File::try_lock`, no crate.
- Admin JSON shapes: as in "Shapes" above.
- Prices: `[model.prices."<provider>/<model>"]` (model-provider-catalog).
- Message selection: `Ctrl+Up/Down` (tui-conversation).
- Queued messages survive an interrupt (tui-conversation).
- All other items labelled "proposed — confirm in review" in the five changes are accepted as written;
  implementers may refine them and must report what they changed.
