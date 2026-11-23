# Specification — `messaging-bodies-telegram-whatsapp`

## ADDED Requirements

### Requirement: Telegram Waist-2 Body (`body-telegram`)
The system SHALL provide a pure-Rust Waist-2 (`monoagens.body.v1`) body binary `body-telegram` in `bodies/body-telegram` that translates between the Telegram Bot API and Waist-2 facts while passing `monoagens-conformance body --level must` (`B-MUST-01..07`).

#### Scenario: Native Telegram capability discovery and dispatch
- **WHEN** Gateway invokes `body/discover` on `body-telegram`
- **THEN** `body-telegram` returns deterministically sorted `CapabilityDescriptor`s with `capability_schema_hash` covering `chat.speak`, `chat.send_buttons`, `chat.react`, `chat.typing`, `chat.send_media`, `chat.send_sticker`, `chat.send_poll`, `chat.edit`, and `chat.delete`
- **AND** dispatching `chat.send_media` with 2 or more attachments uses Telegram's native `sendMediaGroup` album API while dispatching `chat.send_buttons` sends an `inline_keyboard` and `chat.speak` emits a native `typing` chat action before sending.

#### Scenario: Inbound Telegram updates translate native primitives cleanly
- **WHEN** `body-telegram` receives a Telegram update containing text, multiple media attachments, a sticker, a `callback_query` button press, or a `message_reaction`
- **THEN** it translates the update into a Waist-2 `percept/emit` targeting `tg:dm:<chat_id>`, `tg:group:<chat_id>`, or `tg:group:<chat_id>:topic:<thread_id>` with a non-empty `audience`, suppresses self-echoes, automatically calls `answerCallbackQuery` for button taps, and stages attachments exceeding `256 KiB` via `blob/put`.

#### Scenario: Conformance and offline startup without live credentials
- **WHEN** `body-telegram` is started without `TELEGRAM_BOT_TOKEN` (or with `--offline`)
- **THEN** it runs cleanly in Waist-2 NDJSON `stdio` mode without printing non-JSON output to `stdout` and passes `monoagens-conformance body --level must`.

### Requirement: WhatsApp Waist-2 Body (`body-whatsapp`)
The system SHALL provide a pure-Rust Waist-2 (`monoagens.body.v1`) body binary `body-whatsapp` in `bodies/body-whatsapp` powered by `oxidezap/whatsapp-rust` (`whatsapp-rust 0.7`) that translates between WhatsApp Web E2EE sessions and Waist-2 facts while passing `monoagens-conformance body --level must` (`B-MUST-01..07`).

#### Scenario: Native WhatsApp capability discovery and dispatch
- **WHEN** Gateway invokes `body/discover` on `body-whatsapp`
- **THEN** `body-whatsapp` returns deterministically sorted `CapabilityDescriptor`s with `capability_schema_hash` covering `chat.speak`, `chat.react`, `chat.typing`, `chat.send_media`, `chat.send_sticker`, `chat.send_poll`, `chat.mark_read`, `chat.edit`, and `chat.delete`
- **AND** dispatching `chat.speak` emits `composing` before sending and `paused` afterward, `chat.send_media` supports both single and multiple attachments with `delivered_units` accounting, and `chat.send_sticker` uploads native WebP stickers.

#### Scenario: Inbound WhatsApp event filtering, audience accuracy, and pairing hygiene
- **WHEN** `body-whatsapp` connects to a paired SQLite session
- **THEN** it ignores `HistorySync` and pre-boot historical messages, suppresses self-echoes (`is_from_me` and `EchoRegistry`), normalizes DM (`wa:dm:<jid>`) and group (`wa:group:<jid>`) places with non-empty participant `audience`, translates text/media/stickers/reactions/poll votes into `percept/emit`, and writes all QR/pairing/diagnostic output strictly to `stderr` (or via the dedicated `body-whatsapp pair` subcommand) so `stdout` remains pure NDJSON.

#### Scenario: Conformance and offline startup without paired session
- **WHEN** `body-whatsapp` is started without `--live` / paired SQLite session (or with `--offline`)
- **THEN** it runs cleanly in Waist-2 NDJSON `stdio` mode without printing non-JSON output to `stdout` and passes `monoagens-conformance body --level must`.
