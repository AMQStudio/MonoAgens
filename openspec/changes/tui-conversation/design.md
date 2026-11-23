# Design — `tui-conversation`

Audit: [../tui-foundation/audit.md](../tui-foundation/audit.md).

## Context (facts checked on 2026-11-23)
- `ModelProvider::invoke` is blocking (`crates/reason/src/model/mod.rs:312`), called from `spawn_blocking`.
- Titler: `crates/tui/src/titler.rs` (request at ~L212, `clean_generated_title`, `extract_topic_ngrams`,
  `should_request_title_with_embeddings`, n-gram Jaccard rule). Chats persist in `tui_chats.json`.
- Local chat features that exist: multiline composer with history, `/copy`, new/rename/delete chats, capability
  step blocks (`┌─ ... └─`), "Reasoning..." phase text.
- From `tui-foundation`: `turn.cancel` records `Input::ModelFailure { request_id, reason: "cancelled_by_owner" }`;
  busy ends on reply/error/finish+grace; queue-while-busy.

## B2. Interrupt
- Esc pressed twice within 1.5 s while busy (first press shows "press Esc again to interrupt") calls
  `turn.cancel`. The busy state ends on the cancel result; a muted notice "Interrupted." is shown under the
  user message. If `cancelled: false` (no model call in flight, for example waiting on a capability) the notice
  says the turn could not be stopped and finishes on its own.
- Queued messages stay queued and are submitted after the cancel (proposed - confirm in review).
- Esc keeps its other meanings (closing overlays) when not busy or when an overlay is open.

## B4. Retry
- `F5` or `Alt+R` after a failed or interrupted turn re-submits the same user text as a new turn (a new
  event; nothing is deleted). Enabled only when the last turn ended in error or interrupt. Action ids join the
  keybinding table of `tui-settings-complete`.

## B5. Composer
- `Ctrl+R`: reverse incremental search over the composer history (type to filter, Ctrl+R again for older,
  Enter accepts, Esc cancels).
- `Ctrl+G`: write the draft to a temp file under `<data_dir>/cache/` (not `/tmp`), run `$VISUAL` or `$EDITOR`
  after leaving raw mode and the alternate screen, read it back on exit, restore the TUI. No editor set: toast.
- `Ctrl+C` clears the draft and remembers it; Up with an empty composer recalls the cleared draft first.

## B7/B8. What happened and message actions
- Each turn gets a collapsed "what happened" line under the reply (count of model calls, capability steps,
  valve verdicts); `Ctrl+O` expands it into rows. Data comes from the event log for the turn's trace via
  the admin method `events.query` (this change adds it with a by-trace filter only; `tui-governance-observability`
  adds place/source/kind/time filters). Exact event kinds to show: verify during implementation against the CLI
  `log` output.
- Message selection: keys (`Ctrl+Up`/`Ctrl+Down` through messages; `Alt+Up` is already the queue dequeue from `tui-foundation`; key choice proposed - confirm in review) or mouse click selects a message; actions: `c` copy,
  `q` quote into composer (prefixed `> `), `w` or `/why` explain = the valve explanation that `monoagens log
  --explain` prints, via admin method `events.explain` (by event id). Output shape: reuse the CLI's explain
  renderer data; verify during implementation.

## B9. Chats
- Titles fixed (D5 below). Chat search: `Ctrl+F` in the sidebar or `/chat search <text>` filters by title and
  message text. Pin: `/chat pin` toggles; pinned chats sort first (`pinned` flag in `tui_chats.json`, additive).
  Delete asks to confirm (`y`/`n`). `/export [path]` writes the chat as Markdown (default
  `<data_dir>/exports/<title>-<date>.md`; message text only, attachments as filenames, no secrets).

## B10. Notifications
- Enable terminal focus reporting (`EnableFocusChange`); track focused/unfocused. When unfocused and a reply or
  an approval prompt arrives: ring the bell (`\x07`), send OSC 9 (`ESC ] 9 ; <text> BEL`) and OSC 777
  (`ESC ] 777 ; notify ; <title> ; <body> ST`), and set the terminal title to a badge such as `(1) MonoAgens`
  until focus returns. Which terminals honour OSC 9 vs 777: verify during implementation (send both; both are
  harmless when ignored). Setting: `Appearance -> Notifications: off | bell | desktop | both`.

## B11. Image preview (optional, last)
- Detect kitty graphics support (query or `TERM`/`KITTY_WINDOW_ID`; sixel via device attributes) and render
  attachment image previews inline; otherwise the existing `📷 #N WxH` card stays. Detection approach and crate
  (for example `ratatui-image`) : verify during implementation with context7. Skip if it destabilises rendering.

## D-3. Live draft preview (owner-only)
- ARCHITECTURE: the valve runs first and last in every door; a `chat.speak` exists only after the model's full
  structured output is parsed and passes the action door. So streaming is a **draft preview**, not a streamed
  `chat.speak`.
- Provider: a streaming variant of invoke (proposed: an additive method with a default implementation that
  falls back to blocking `invoke`) using SSE (`stream: true`) for `openai_chat` and the Anthropic Messages
  streaming events when the dialect supports it. Token usage under streaming (for example
  `stream_options.include_usage`) must still be recorded exactly as before: verify during implementation.
- Gateway: incrementally extracts the draft speech text from the partial structured output (a tolerant
  incremental JSON string reader over the field that carries the speech text; the field path comes from the
  structured-output schema in `crates/reason`: verify during implementation) and publishes it on
  `preview.subscribe` to the TUI that submitted the turn (`turn.submit` carries the subscription). The in-process
  client uses a channel; the socket client uses the owner-only admin socket (0600). Adapters, robots and all other
  bodies are never given a preview channel.
- Scope: previews are published only for turns submitted by the TUI itself (proposed - confirm in review), so a
  draft addressed to another person's place is never displayed ahead of the valve's decision.
- Display: a dim "draft" bubble that grows while streaming. When the turn ends it is replaced by the real reply,
  or by the valve verdict (held/denied) or by "Mono chose not to reply". A cancel or provider failure removes it.
- Never recorded as events, never placed on the queue, never read by any decision. Setting
  `Appearance -> Live draft preview` (default on); off = no `stream: true` request and no preview.
- The final `chat.speak` is byte-for-byte what it is today.

## D5. Chat titles
- Titler request uses `max_output_tokens = model.output_limits.classify` (user config: 1024) and
  `timeout_ms = 30_000`; it accepts the title from `Verdict.label` or the raw text, then `clean_generated_title`.
- Any failure -> log `titler` warn with the reason, then deterministic fallback: the first message's top
  2-5 content words from `extract_topic_ngrams`, title-cased, max 40 chars. A chat is never left as
  "New Chat" after its first message.
- Topic shift: when `[embed]` is configured, use `should_request_title_with_embeddings` (cosine against the
  chat's stored centroid); otherwise the existing n-gram Jaccard rule. The thresholds already in `titler.rs`
  stay as they are. Manual `/chat rename` still locks the title. The titler uses the classify model when
  `model-provider-catalog` configures one.
