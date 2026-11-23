# Tasks — `messaging-bodies-telegram-whatsapp`

## 1. Telegram Waist-2 Body (`bodies/body-telegram`)
- [x] 1.1 Scaffold `bodies/body-telegram` workspace crate with `TelegramTransport` abstraction (`LiveTelegramTransport` over `reqwest` + offline/mock transport for conformance & tests), `PersistentBodyRuntime`, `EchoRegistry`, and split-bubble delivery.
- [x] 1.2 Implement all native Telegram capabilities (`chat.speak`, `chat.send_buttons`, `chat.react`, `chat.typing`, `chat.send_media` single + `sendMediaGroup` multi-attachment, `chat.send_sticker`, `chat.send_poll`, `chat.edit`, `chat.delete`) and bidirectional `getUpdates` long-polling loop (text, multi-attachments, stickers, `callback_query` auto-ack, `message_reaction`, reply context, forum topics, group audience cache, self-echo suppression, `<= 256 KiB` inline / `blob/put` custody).
- [x] 1.3 Verify unit + mock HTTP Bot API integration tests, `cargo clippy -p body-telegram --all-targets -- -D warnings`, and `monoagens-conformance body --target target/debug/body-telegram --level must` (`B-MUST-01..07`).

## 2. WhatsApp Waist-2 Body (`bodies/body-whatsapp`)
- [x] 2.1 Scaffold standalone `bodies/body-whatsapp` crate (`whatsapp-rust 0.7`, `monoagens-body-kit`, `monoagens-body-sdk`) with `WhatsAppTransport` abstraction (`LiveWhatsAppTransport` + offline/mock transport), `PersistentBodyRuntime`, `EchoRegistry`, split-bubble delivery, and dedicated `body-whatsapp pair [--db <path>] [--phone <number>]` CLI subcommand.
- [x] 2.2 Implement all native WhatsApp capabilities (`chat.speak`, `chat.react`, `chat.typing` `composing`/`recording`/`paused`, `chat.send_media` single + multi-attachment batch, `chat.send_sticker`, `chat.send_poll`, `chat.mark_read`, `chat.edit`, `chat.delete`) and live `whatsapp-rust` event translation (strict `HistorySync`/pre-boot filter, media/sticker/reaction/poll-vote translation, LID/JID normalization, group participant audience lookup, self-echo suppression, `stderr`-only QR/logs).
- [x] 2.3 Verify unit + mock transport tests, `cargo clippy --manifest-path bodies/body-whatsapp/Cargo.toml --all-targets -- -D warnings`, and `monoagens-conformance body --target bodies/body-whatsapp/target/debug/body-whatsapp --level must` (`B-MUST-01..07`).

## 3. Documentation & Integration Verification
- [x] 3.1 Update `docs/messaging-bodies.md` with exact capability tables, CLI flags, `body-whatsapp pair` instructions, and ready-to-use `monoagens.toml` `[[bodies]]` snippets for both `body-telegram` and `body-whatsapp`.
- [x] 3.2 Run `openspec validate messaging-bodies-telegram-whatsapp --strict`, workspace `cargo clippy --workspace --all-targets -- -D warnings`, and GitNexus `detect_changes`.
