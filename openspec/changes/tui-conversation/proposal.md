# Proposal — `tui-conversation`

Part 4 of 5. Audit: [audit.md](../tui-foundation/audit.md) (covers B2, B4-B11, chat titles from the old design
D5, and decision D-3).

**Depends on `tui-foundation`** (`turn.cancel`, `turn.submit`, `GatewayClient`, queue-while-busy and turn
lifecycle from B1/B3, PTY driver, fakes), on **`model-provider-catalog`** (streaming support is added to the
dialects defined there; the titler follows the classify model) and on **`tui-settings-complete`** (settings
selectors/Appearance rows, keybinding action table, scrollable lists).

## Why
Verified on 2026-11-23 unless stated:
- There is no way to stop a running turn (B2). The gateway side (`turn.cancel`) exists after `tui-foundation`.
- Auto chat titles never arrive: the titler asks a reasoning model for a title with `max_output_tokens = 32`
  and a 5 s timeout (`crates/tui/src/titler.rs:212`) and drops every failure silently.
- Replies arrive whole via `chat.speak`; there is no live feedback while the model writes (B6).
- No retry (B4), no history search or external editor (B5), no per-turn "what happened" view (B7; only the
  "Reasoning..." phase text), message actions are only `/copy` (B8), chats lack search/pin/export and delete
  does not confirm (B9), nothing notifies when the terminal is unfocused and a reply arrives (B10, the "minimize"
  complaint).

## What Changes
1. **Turn control**: Esc twice interrupts through `turn.cancel`; `F5`/`Alt+R` retries the last failed turn.
2. **Composer**: `Ctrl+R` history search, `Ctrl+G` external editor, recall of a cleared draft.
3. **What happened block** per turn (model calls, capability steps, valve verdicts; `Ctrl+O` expands) and
   **message actions** (copy, quote, `/why`), backed by minimal admin methods `events.query` and
   `events.explain` (filters are extended in `tui-governance-observability`).
4. **Chats**: titles that work (D5), search, pin, delete with confirmation, `/export` to Markdown.
5. **Notifications** when unfocused: bell, OSC 9/777 desktop notification, title badge; a setting.
6. **Optional image preview** where the terminal supports kitty graphics or sixel.
7. **Live draft preview (D-3)**: owner-only, never an event, no protocol change; adapters and every other body
   receive only the final valve-approved `chat.speak`, unchanged.

## Impact
- Crates: `crates/tui` (most), `crates/reason` (streaming variant of provider invoke for `openai_chat`
  and `anthropic_messages`), `crates/gateway` (draft extraction, `preview.subscribe`, `events.query`,
  `events.explain`), `crates/cli` unchanged.
- No core change. No protocol change: previews are a gateway-local owner channel, never placed in the event
  queue, never recorded, never used in a decision.
- Invariants kept:
  - *valve first and last in every door*: a `chat.speak` exists only after the model's full structured output is
    parsed and passes the action door; the preview is not an action and reaches no body;
  - *one queue, one consumer*: previews bypass the queue and never enter it; cancel and retry enter the queue as
    ordinary inputs (`ModelFailure`, a new user message);
  - *record first, then act* and *reproducibility*: previews are not events, so `monoagens replay` is unchanged;
  - *protocol changes only additively and by the owner*: none made;
  - *gateway generic and never learns*: draft extraction works on the structured-output shape, not on a
    platform; no state is kept across restarts;
  - *core sees only opaque ids*: the core is not involved.
- Two absolute lines: the preview goes only to the TUI that submitted the turn (see design), so private content
  is not shown to anyone who was not part of it.
- New setting `Appearance -> Live draft preview` (default on) and notification settings in `tui.toml`.
