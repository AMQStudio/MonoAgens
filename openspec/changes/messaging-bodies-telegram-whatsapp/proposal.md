# Proposal — `messaging-bodies-telegram-whatsapp`

## Why
MonoAgens now supports dynamic place routing, multi-body `monoagens serve` supervision, and bidirectional Waist-2 SDKs, but still lacks production messaging adapters for Telegram and WhatsApp. Adding pure-Rust Waist-2 bodies (`body-telegram` and `body-whatsapp` via `oxidezap/whatsapp-rust`) unlocks live cross-channel DM and group presence while exposing each platform's native UX capabilities (inline buttons, reactions, stickers, multi-attachment albums/batches, typing indicators, polls, edit/delete, and read receipts) cleanly at the plugin edge without altering Core or Gateway invariants.

## What Changes
- **Add `bodies/body-telegram`** (workspace member) implementing a pure-Rust Waist-2 (`monoagens.body.v1`) adapter over Telegram Bot API (`reqwest` + `tokio` + `monoagens-body-kit`) with native capabilities: `chat.speak`, `chat.send_buttons`, `chat.react`, `chat.typing`, `chat.send_media` (single + native `sendMediaGroup` multi-attachment albums), `chat.send_sticker`, `chat.send_poll`, `chat.edit`, and `chat.delete`, plus inbound `getUpdates` translation (text, multi-media, stickers, `callback_query`, `message_reaction`, quote replies, forum topics, group audience tracking, and self-echo suppression).
- **Add `bodies/body-whatsapp`** (standalone excluded Rust crate over `whatsapp-rust 0.7` + `monoagens-body-kit` + `monoagens-body-sdk`, isolated from root `libsqlite3-sys 0.40` to avoid linking conflicts with `whatsapp-rust-sqlite-storage`'s `libsqlite3-sys 0.37`) with native capabilities: `chat.speak`, `chat.react`, `chat.typing` (`composing`/`recording`/`paused`), `chat.send_media` (single + multi-attachment batch), `chat.send_sticker` (WebP), `chat.send_poll`, `chat.mark_read`, `chat.edit`, and `chat.delete`, plus live event translation (filtering `HistorySync`/pre-boot backlog, LID/JID normalization, group participant audience lookup, self-echo suppression) and a dedicated `body-whatsapp pair` CLI flow that keeps NDJSON `stdout` strictly clean.
- **Pass Waist-2 Body Conformance (`B-MUST-01..07`)** for both binaries (`monoagens-conformance body --level must`) and document exact configuration & capabilities in `docs/messaging-bodies.md`.

## Impact
- `Cargo.toml` (adds `bodies/body-telegram` member, excludes `bodies/body-whatsapp`).
- `bodies/body-telegram/` (new crate).
- `bodies/body-whatsapp/` (new standalone crate).
- `docs/messaging-bodies.md` (updated capability reference & `monoagens.toml` snippets).
- Zero changes to `crates/core`, `crates/valve`, `crates/gateway`, or `ARCHITECTURE.md` invariants.
