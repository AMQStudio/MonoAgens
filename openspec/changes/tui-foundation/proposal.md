# Proposal — `tui-foundation`

Part 1 of 5 of the "complete TUI harness" work. Gap audit and owner decisions: [audit.md](audit.md)
(this change covers A1-A6, B1, B3, D1, D2, F1, H1-H3 and decisions D-1 and D-2). Everything in the
other four changes is built and tested on this one.

## Why
The `interactive-tui` change shipped a TUI that looks complete but is not yet a reliable harness.
Checked by hand in tmux on 2026-11-23 (release binary, commit `a91aea0`) and in the code:

- No lock exists on the data directory. The TUI and `monoagens serve` (or two TUIs) each boot a gateway on
  the same store: two queue consumers, against the invariant "one queue, one consumer", and plugins are
  spawned twice (a WhatsApp multi-device session does not survive that).
- The TUI always hosts its own gateway; there is no control channel for a running `serve`.
- Plugins marked **Enable on Startup ON** stay **stopped** after every start: plugins are only spawned from
  the manual Start action (`crates/tui/src/runtime.rs:697`); `enabled = true` is never acted on.
- A crashed plugin is invisible: stderr goes to `Stdio::null()` (`runtime.rs:1505`) and child exit is never
  watched, so the dot can stay green while the bot is dead.
- "Have to send twice": busy is cleared only by an `AssistantReply` (`chat.speak`). `TurnFinished` does
  nothing (`runtime.rs:888`), so a turn where the soul chose not to speak leaves the TUI waiting. After any
  full redraw, all queued keyboard input is discarded (`runtime.rs:425-428`).
- Two loading indicators run at once: the phase line (`render.rs:744`) and the status-bar spinner (`render.rs:767`).
- Verified: `provider = "nonsense"` makes the TUI print the error and exit 1; it never opens.
- Emoji with variation selectors shift columns; there is no handling of Resize/FocusGained or Ctrl+Z.
- There is no runtime log anywhere, and no automated way to drive the real binary in a terminal.
- The gateway has no turn cancel, so an interrupt cannot exist yet.

## What Changes
1. **Gateway ownership (D-1, lock + attach)**: exclusive lock `<data_dir>/gateway.lock`; the holder serves an
   admin socket `<data_dir>/gateway.sock` (mode 0600, NDJSON JSON-RPC like Waist-2). The TUI talks to the
   gateway through one `GatewayClient` trait with an in-process and a socket implementation, so it can attach
   to `monoagens serve` or to another TUI. Closing an attached TUI leaves the daemon and its plugins running.
2. **Turn cancel (D-2)**: admin method `turn.cancel` abandons waiting on the in-flight model call and submits
   the existing core input `Input::ModelFailure { request_id, reason: "cancelled_by_owner" }`.
3. **Runtime reliability**: autostart in dependency order, stderr capture, exit watching, restart backoff,
   per-plugin lock; turn ends on reply / error / finish+grace; queue while busy; never drop input; one busy
   indicator; width-safe rendering; resize/focus/suspend; small-terminal layout; TUI opens on bad config and
   `/doctor` checks the setup.
4. **`/log`** runtime log window with ring buffer and rotating file.
5. **Verification harness**: `scripts/tui-smoke.sh` (tmux), a Rust PTY test driver, `insta` snapshots at three
   sizes, a fake stdio plugin and a fake provider HTTP server.

## Impact
- Crates: `crates/tui` (most of the work), `crates/gateway` (lock, admin socket, `turn.cancel`, shared
  `config.apply`), `crates/cli` (`serve` takes the lock and serves the socket), new `scripts/tui-smoke.sh`,
  dev-dependencies for the PTY driver and snapshots.
- No core change: `turn.cancel` uses the existing `Input::ModelFailure` (`crates/core/src/lib.rs:136`). No
  protocol (Waist-1/Waist-2) change; the admin socket is a gateway-local control channel for the owner.
- Invariants kept (`ARCHITECTURE.md`): *one queue, one consumer* is what the lock enforces; *valve first and
  last in every door* is untouched (admin methods never bypass a door; `turn.submit` enters through the
  inbound door like any body); *protocol changes only additively and by the owner* (none made); *gateway
  generic and never learns* (the admin socket carries opaque ids and config; nothing is remembered in process
  memory across restarts); *core sees only opaque ids* (cancel reason is a code string).
- Config files stay backward compatible; new keys are additive.
- Later changes depend on this one: `model-provider-catalog`, `tui-settings-complete`, `tui-conversation`,
  `tui-governance-observability`.
