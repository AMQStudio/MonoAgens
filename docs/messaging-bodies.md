# Integrating Messaging Bodies (WhatsApp, Telegram & Chat Platforms)

This guide explains how to build and run external chat adapters (such as WhatsApp or Telegram bridges) as **Waist-2 (`monoagens.body.v1`)** bodies connected to MonoAgens Gateway (`crates/gateway/`) via `monoagens serve`.

> Protocol reference: [`waist2.md`](waist2.md). Conformance verification: [`conformance.md`](conformance.md).

---

## 1. Canonical `PlaceId` Conventions

Use deterministic, platform-prefixed `PlaceId` strings so memories, topic segmentation, and `VisibleScope` boundaries stay cleanly partitioned across DMs and groups:

| Platform & Room Type | Canonical `PlaceId` Format | Example |
|---|---|---|
| WhatsApp 1:1 DM | `wa:dm:<jid>` | `wa:dm:628123456789@s.whatsapp.net` |
| WhatsApp Group | `wa:group:<jid>` | `wa:group:120363012345678901@g.us` |
| Telegram 1:1 DM | `tg:dm:<chat_id>` | `tg:dm:123456789` |
| Telegram Group / Supergroup | `tg:group:<chat_id>` | `tg:group:-1001234567890` |

Never collapse a DM and a group into the same `PlaceId`, and never omit the platform prefix (`wa:`, `tg:`).

---

## 2. Canonical `ActorId` & Cross-Platform Identity Linking

Gateway resolves every `(body_id, account_id)` pair in [`crates/gateway/src/identity.rs`](../crates/gateway/src/identity.rs) with strict precedence and **heuristic merging permanently disabled** (`HEURISTIC_MERGE_ENABLED = false`):

1. **Static `[[person]]` (`C_person`) in `monoagens.toml`**: Highest precedence. Bind your own WhatsApp, Telegram, and CLI accounts to `actor_id = "owner"` (`owner = true`) so Absolute Line 1 (`SelfModify`) and `PendingApproval` recognize you consistently across platforms.
2. **Active bilateral link (`E_link`)**: Managed at runtime via `monoagens link` (`list`, `resolve`, `initiate`, `verify`, `unlink`).
3. **Deterministic fallback `κ(b, a)`**: Unmapped `(body_id, account_id)` pairs map deterministically to:
   ```text
   per_<hex32(BLAKE3("monoagens.actor.v1" || u32_le(|b|) || b || u32_le(|a|) || a))>
   ```

Example commands to inspect or link identities at runtime:

```sh
monoagens link list
monoagens link resolve --body body:whatsapp --account "628123456789@s.whatsapp.net"
```

---

## 3. Audience & Privacy Rules (`VisibleScope`, `Δ_NI = 0`)

- **Never omit `audience` or pass `[]`**: Both `percept/emit` and `action/execute` reject empty or missing audiences with `-32006 ERR_UNDECLARED_AUDIENCE`.
- **1:1 DMs**: Always populate `audience` with `[sender_account_id, self_account_id]`. Because `VisibleScope` enforces `Δ_NI == 0`, private DM memories will never leak into group prompts.
- **Groups**: Always populate `audience` with the **current active group members** (including the bot's own account). When a participant leaves a group, exclude them from subsequent `audience` sets so later turns remain isolated from former members.
- **Do not spoof role tags**: Never synthesize `owner`, `room_present`, or `physically_present` from message text; Gateway and Valve resolve canonical roles strictly from `[[person]]`, `E_link`, and attested hardware/place records.

---

## 4. Dynamic Place Routing

Messaging bridges do not know every future DM or group chat at startup:

- In `body/discover`, a dynamic messaging body may return `places: []` (or only pre-pinned rooms).
- When a new DM or group message arrives, emit `percept/emit` with its canonical `PlaceId` (`wa:dm:...`, `wa:group:...`, `tg:dm:...`, `tg:group:...`).
- Gateway dynamically binds `place -> body_id` on the first authenticated percept from that body, prevents other bodies from hijacking an already-owned place, and cleans up dynamic bindings automatically when the body disconnects.

---

## 5. Chat UX Hygiene

- **Self-echo suppression (`R-09`)**: Record outbound platform message IDs (`pid_1..pid_M`) using [`EchoRegistry`](../plugins/kit/src/split_bubble.rs) (`monoagens-body-kit`) or an equivalent bounded map in your bridge, and drop inbound webhook/socket echoes matching your own sent `pid`s so Mono never triggers feedback loops on its own replies.
- **`working_cue` -> native typing indicator**: Gateway and `plugins/kit` enforce `d_art = 0 ms` (zero artificial delay, [`plugins/kit/src/cues.rs`](../plugins/kit/src/cues.rs)). When a genuine turn/action wait crosses `working_cue_ms` (`1_000 ms` default), forward the `working_cue` signal to WhatsApp `composing` presence or Telegram `sendChatAction("typing")`.
- **Split-bubble delivery**: Platform messages have character/byte limits (`L_cap`). Use `deliver_split_bubbles` / `split_text_chunks` in [`plugins/kit/src/split_bubble.rs`](../plugins/kit/src/split_bubble.rs) to split long replies cleanly across whitespace/newline boundaries while tracking `delivered_units`, supporting mid-stream `action/cancel` barge-in, and returning a single combined `Receipt`.
- **Media & attachment handling (`<= 256 KiB` inline vs `blob/put|get`)**: Inline payloads are bounded by `MAX_INLINE_ATTACHMENT_BYTES` (`262_144` bytes / `256 KiB` in [`crates/protocol/src/facts.rs`](../crates/protocol/src/facts.rs)). Any image, voice note, or document exceeding `256 KiB` must be staged out-of-band via `blob/put` / `blob/get` (`60 s` staging TTL, verified by `byte_len` and SHA-256 hex digest; mismatches fail with `-32005 ERR_HASH_MISMATCH`).

---

## 6. Built-In Messaging Plugins (`plugins/telegram` & `plugins/whatsapp`)

MonoAgens ships two pure-Rust Waist-2 messaging adapter plugins under `plugins/`:

- **`plugins/telegram`** (workspace member, binary `body-telegram`) — Telegram Bot API (`reqwest` + `tokio`), passing `monoagens-conformance body --level should` (`10/10`, `capabilities=10`).
- **`plugins/whatsapp`** (standalone excluded crate, binary `body-whatsapp`, powered by `whatsapp-rust 0.7`, isolated from root `libsqlite3-sys 0.40`), passing `monoagens-conformance body --level should` (`10/10`, `capabilities=10`).

### Native Capability Matrix

| Capability | `body-telegram` | `body-whatsapp` |
|---|---|---|
| `chat.speak` | Text, `parse_mode`, `reply_to_message_id`, split-bubble (`4096`), auto `typing`, optional `buttons` | Text, quote-reply (`reply_to_message_id`), `mentions`, split-bubble (`4096`), auto `composing`/`paused` |
| `chat.send_buttons` | Native `inline_keyboard` + inbound `callback_query` auto-ack & percept | — |
| `chat.react` | Native `setMessageReaction` | Native `ReactionMessage` (`""` clears reaction) |
| `chat.typing` | Native `sendChatAction` (`typing`, `upload_photo`, `upload_document`, `record_voice`, `upload_video`, `choose_sticker`) | Native `chatstate` (`composing`, `recording`, `paused`) |
| `chat.send_media` | Single (`sendPhoto`/`sendDocument`/`sendVideo`/`sendAudio`/`sendVoice`) + multi-attachment **`sendMediaGroup`** album | Single & multi-attachment batch (`image`, `video`, `document`, `audio`, `voice`/PTT) |
| `chat.send_sticker` | Native `sendSticker` | Native WebP `StickerMessage` |
| `chat.send_poll` | Native `sendPoll` (regular / quiz) | Native `PollCreationMessage` + encrypted vote decryption |
| `chat.mark_read` | — | Native `mark_as_read` read receipts |
| `chat.edit` | Native `editMessageText` | Native message edit (`edit_message`) |
| `chat.delete` | Native `deleteMessage` | Native revoke/delete (`revoke_message`) |
| `approval.prompt` | Native `inline_keyboard` Approve/Deny buttons (`callback_data` bound to `action_id`) -> `approval/resolve` | Prompt message + configurable emoji reaction watcher (`approve_emojis` / `deny_emojis`) -> `approval/resolve` |

### Building & Pairing

```sh
source "$HOME/.cargo/env"

# Build Telegram body (workspace member)
cargo build --release -p body-telegram -p monoagens-cli

# Build WhatsApp plugin (standalone crate)
cargo build --release --manifest-path plugins/whatsapp/Cargo.toml

# Pair WhatsApp once (QR code in terminal, or --phone 6281234567890 for pairing code)
./plugins/whatsapp/target/release/body-whatsapp pair \
  --db "$HOME/.local/share/monoagens/plugins/body-whatsapp/session.db"
```

### Unified `[plugins.<id>]`, RBAC & Self-Managed Plugin Config (`monoagens.toml`)

MonoAgens keeps `monoagens.toml` lean and internationalized:

- **`[plugins.<id>]`** declares external Waist-2 plugins with `kind = "adapter"` (chat channels like Telegram/WhatsApp) or `kind = "capability"` (tool/skill plugins), optional `enabled`, `dependencies`, and `optional_dependencies`. Legacy `[[bodies]]` entries remain supported as `kind = "adapter"` aliases.
- **Self-managed plugin config**: Gateway injects `MONOAGENS_PLUGIN_CONFIG=<config_dir>/plugins/<id>.toml`, `MONOAGENS_PLUGIN_STATE_DIR=<data_dir>/plugins/<id>`, `MONOAGENS_BODY_STATE_DIR=<data_dir>/plugins/<id>`, and `MONOAGENS_RBAC_ENABLED=1|0` into each spawned stdio plugin. On first run, `body-telegram` and `body-whatsapp` auto-generate commented `plugins/telegram.toml` (e.g. `default_parse_mode`, `poll_timeout_secs`, `approve_button_label`, `deny_button_label`) and `plugins/whatsapp.toml` (e.g. `auto_typing_on_speak`, `auto_mark_read`, `approve_emojis`, `deny_emojis`).
- **Generic RBAC narrowing (`[roles]`, `[places]`, `[[person]].roles`)**: `monoagens serve` and `monoagens chat` pass `[roles]` and `[places]` into `ValveConfig::rbac`, strictly narrowing permissions (`Guest < Member < Trusted < Admin < Owner`) and approval modes (`yolo | default | strict`) without ever weakening `cvc5` Absolute Line 1/2 proofs.
- **Gateway Command Registry (`/help`)**: Gateway indexes built-in commands (`/help`, `/whoami`, `/roles`, `/approve`, `/deny`) alongside plugin-declared `discover._meta.commands`, executing `/help` and built-ins with zero LLM calls and RBAC-filtered visibility per caller.
- **Generic `approval.prompt` -> `approval/resolve`**: When Valve holds an action (`PendingApproval`), Gateway dispatches `approval.prompt` to the target place's adapter and deterministically verifies the resulting `approval/resolve` against `owner`, `h_act`, `state_epoch`, and TTL.

Keep `TELEGRAM_BOT_TOKEN` and LLM `model_api_key` strictly in `secrets.toml` or environment variables — never in `monoagens.toml`:

```toml
version = 1

[soul]
name = "Mono"
seed = 1

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
allow = ["command.help", "identity.whoami"]

[roles.member]
base = "member"
allow = ["chat.*"]
approval = "default"

[roles.trusted]
base = "trusted"
allow = ["chat.*", "tool.read"]
approval = "strict"

[roles.owner]
base = "owner"
allow = ["*"]

[places."tg:group:-1001234567890"]
deny = ["tool.*"]

[plugins.telegram]
kind = "adapter"
command = "/path/to/Mono/target/release/body-telegram"
args = ["--token-env", "TELEGRAM_BOT_TOKEN"]

[plugins.whatsapp]
kind = "adapter"
command = "/path/to/Mono/plugins/whatsapp/target/release/body-whatsapp"
args = ["--live"]

# Built-in capability plugins (`plugins/plugin-*`), each self-managing plugins/<id>.toml
[plugins.web-search]
kind = "capability"
command = "/path/to/Mono/target/release/plugin-web-search"

[plugins.workspace]
kind = "capability"
optional_dependencies = ["rbac"]
command = "/path/to/Mono/target/release/plugin-workspace"

[plugins.pdf-reader]
kind = "capability"
command = "/path/to/Mono/target/release/plugin-pdf-reader"

[plugins.soul-tuning]
kind = "capability"
command = "/path/to/Mono/target/release/plugin-soul-tuning"

[[person]]
actor_id = "owner"
owner = true
roles = ["owner"]
accounts = [
  { body_id = "body-whatsapp", account_id = "wa:user:628123456789@s.whatsapp.net" },
  { body_id = "body-telegram", account_id = "tg:user:123456789" },
]
```

Start the multi-plugin Gateway daemon (filter individual plugins with `--plugin <id>` or legacy `--body <id>`):

```sh
source "$HOME/.cargo/env"
./target/release/monoagens serve --config "$HOME/.config/monoagens/monoagens.toml"
```

---

## 7. Pre-Launch Checklist

- [ ] **Conformance TCK**: Bridge passes `monoagens-conformance body --target <bridge-bin> --level must` (`B-MUST-01..07`) and ideally `--level should` (`B-SHOULD-01..03`).
- [ ] **Canonical `PlaceId`s**: DMs and groups use `wa:dm:<jid>`, `wa:group:<jid>`, `tg:dm:<chat_id>`, `tg:group:<chat_id>`.
- [ ] **Non-empty `audience`**: Every `percept/emit` populates `[sender, self]` for DMs and active members for groups (`-32006` never bypassed).
- [ ] **`[[person]]` Owner Binding & RBAC**: Owner accounts across WhatsApp/Telegram map to `actor_id = "owner"` (`owner = true`, `roles = ["owner"]`) and `[roles]` / `[places]` are configured in `monoagens.toml`.
- [ ] **Self-Managed Plugin Configs & Secret Isolation**: Platform tokens and API keys live only in `secrets.toml` / env vars, while plugin UX settings live in `plugins/<id>.toml` via `MONOAGENS_PLUGIN_CONFIG`.
- [ ] **Self-Echo Suppression**: Outbound `pid`s are recorded and ignored on inbound echo webhooks.
- [ ] **Crash Recovery & Idempotency**: `MONOAGENS_PLUGIN_STATE_DIR` (and `MONOAGENS_BODY_STATE_DIR`) persists `idempotency_key` results and `state/query` answers within `5000 ms` after `SIGKILL`.
- [ ] **Media > `256 KiB`**: Large attachments transfer via `blob/put` / `blob/get` with SHA-256 verification instead of oversized inline payloads.
