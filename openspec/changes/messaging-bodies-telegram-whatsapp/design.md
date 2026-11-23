# Design — `messaging-bodies-telegram-whatsapp`

## Architectural Alignment
Both bodies follow the Waist-2 edge-translation invariant in `ARCHITECTURE.md`:
- Every platform-specific primitive (Telegram `inline_keyboard` / `callback_query` / `sendMediaGroup` / `setMessageReaction` / `sendSticker` / `sendChatAction`, WhatsApp `ReactionMessage` / `chatstate` / `StickerMessage` / `PollCreationMessage` / `mark_as_read` / `ProtocolMessage`) is translated strictly inside the body binary into Waist-2 `CapabilityDescriptor`s, `Percept`s, `Action`s, `AttachmentHandle`s (`blob/put|get`), and `Receipt`s.
- `stdout` is strictly reserved for NDJSON JSON-RPC 2.0 frames in stdio mode. All operational diagnostics, pairing QR codes, and warnings go to `stderr` (or via dedicated `body-whatsapp pair` subcommand).
- Both binaries wrap `monoagens_body_kit::PersistentBodyRuntime` (`MONOAGENS_BODY_STATE_DIR`) for `idempotency_key` deduplication, `SIGKILL` crash-journal `state/query`, and SHA-256 `blob/put|get`, plus `EchoRegistry` and `deliver_split_bubbles`.
- When launched without live credentials/session (or with `--offline`), both bodies start cleanly in stdio Waist-2 mode with an offline mock/no-op transport so `monoagens-conformance body --level must` (`B-MUST-01..07`) passes deterministically without network access.

## Place & Actor Conventions
- **Telegram (`body-telegram`):**
  - DM: `tg:dm:<chat_id>`
  - Group / Supergroup: `tg:group:<chat_id>`
  - Forum topic: `tg:group:<chat_id>:topic:<message_thread_id>`
  - Actor: `tg:user:<user_id>`
- **WhatsApp (`body-whatsapp`):**
  - DM: `wa:dm:<jid>` (canonical bare user JID)
  - Group: `wa:group:<jid>` (`<group_id>@g.us`)
  - Actor: `wa:user:<normalized_user_id>` (canonical user/LID identifier)

## Crate Isolation Decision
`whatsapp-rust-sqlite-storage 0.7.0` pins `libsqlite3-sys 0.37`, whereas root `Cargo.lock` (`rusqlite 0.43`) links `libsqlite3-sys 0.40`. Cargo forbids two `links = "sqlite3"` crates in the same workspace dependency graph. Placing `bodies/body-whatsapp` in `[workspace] exclude` (matching `sdk/rust`) while depending on `monoagens-body-kit` and `monoagens-body-sdk` via relative path eliminates the link conflict with zero impact on `monoagens serve`, because Waist-2 external bodies execute as independent child processes.

## Native Capability Matrix
| Capability ID | `body-telegram` | `body-whatsapp` |
|---|---|---|
| `chat.speak` | Text, `parse_mode`, `reply_to_message_id`, split-bubble, auto `typing`, optional `buttons` | Text, `reply_to_message_id` quote, `mentions`, split-bubble, auto `composing`/`paused` |
| `chat.send_buttons` | Native `inline_keyboard` + inbound `callback_query` auto-ack & percept | N/A (not natively supported on personal WA accounts) |
| `chat.react` | Native `setMessageReaction` | Native `ReactionMessage` (`""` removes reaction) |
| `chat.typing` | Native `sendChatAction` (`typing`, `upload_photo`, `upload_document`, `record_voice`, `upload_video`, `choose_sticker`) | Native `chatstate` (`composing`, `recording`, `paused`) |
| `chat.send_media` | Single (`sendPhoto`/`sendDocument`/`sendVideo`/`sendAudio`/`sendVoice`) + multi (`sendMediaGroup` album) | Single + multi-attachment batch (`image`, `video`, `document`, `audio`, `voice`/PTT) |
| `chat.send_sticker` | Native `sendSticker` | Native WebP `StickerMessage` |
| `chat.send_poll` | Native `sendPoll` (regular/quiz) | Native `PollCreationMessage` |
| `chat.mark_read` | N/A | Native `mark_as_read` read receipts |
| `chat.edit` | Native `editMessageText` | Native `ProtocolMessage` edit |
| `chat.delete` | Native `deleteMessage` | Native `ProtocolMessage` revoke |

## Safety & Noise Guards
- **Self-echo suppression:** Inbound messages from the bot's own account (`from.id == bot_id` on Telegram, `info.source.is_from_me` on WhatsApp) or matching `EchoRegistry` are suppressed so the soul never triggers on its own utterances.
- **History sync filter (WhatsApp):** `Event::HistorySync` and messages predating process startup (beyond a small clock-skew grace window) are never emitted as live percepts, preventing stale backlog storms on pairing or reconnect.
- **Attachment custody:** Payloads `<= 256 KiB` may travel inline or via `blob/put`; payloads `> 256 KiB` are always staged out-of-band via `blob/put` with SHA-256 verification.
- **Audience non-emptiness:** Both bodies maintain per-group participant caches (seeded from group metadata/admin queries where available and updated from inbound events) so every emitted `Percept` declares a non-empty `audience` (`VisibleScope` `Δ_NI = 0`).
