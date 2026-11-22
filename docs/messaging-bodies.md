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

- **Self-echo suppression (`R-09`)**: Record outbound platform message IDs (`pid_1..pid_M`) using [`EchoRegistry`](../bodies/body-kit/src/split_bubble.rs) (`monoagens-body-kit`) or an equivalent bounded map in your bridge, and drop inbound webhook/socket echoes matching your own sent `pid`s so Mono never triggers feedback loops on its own replies.
- **`working_cue` -> native typing indicator**: Gateway and `body-kit` enforce `d_art = 0 ms` (zero artificial delay, [`bodies/body-kit/src/cues.rs`](../bodies/body-kit/src/cues.rs)). When a genuine turn/action wait crosses `working_cue_ms` (`1_000 ms` default), forward the `working_cue` signal to WhatsApp `composing` presence or Telegram `sendChatAction("typing")`.
- **Split-bubble delivery**: Platform messages have character/byte limits (`L_cap`). Use `deliver_split_bubbles` / `split_text_chunks` in [`bodies/body-kit/src/split_bubble.rs`](../bodies/body-kit/src/split_bubble.rs) to split long replies cleanly across whitespace/newline boundaries while tracking `delivered_units`, supporting mid-stream `action/cancel` barge-in, and returning a single combined `Receipt`.
- **Media & attachment handling (`<= 256 KiB` inline vs `blob/put|get`)**: Inline payloads are bounded by `MAX_INLINE_ATTACHMENT_BYTES` (`262_144` bytes / `256 KiB` in [`crates/protocol/src/facts.rs`](../crates/protocol/src/facts.rs)). Any image, voice note, or document exceeding `256 KiB` must be staged out-of-band via `blob/put` / `blob/get` (`60 s` staging TTL, verified by `byte_len` and SHA-256 hex digest; mismatches fail with `-32005 ERR_HASH_MISMATCH`).

---

## 6. Example `monoagens.toml` & Running `monoagens serve`

Declare external bodies in `[[bodies]]` and static owner/member account bindings in `[[person]]` inside `monoagens.toml` (keep all bot tokens, session keys, and LLM API keys strictly in `secrets.toml` or environment variables — never in `monoagens.toml`):

```toml
version = 1

[soul]
name = "Mono"
seed = 1

[[bodies]]
id = "body:whatsapp"
transport = "stdio"
command = "/usr/local/bin/monoagens-body-whatsapp"
args = ["--state-dir", "/var/lib/monoagens/wa"]

[[bodies]]
id = "body:telegram"
transport = "unix_socket"
socket_path = "/run/monoagens/telegram.sock"

[[person]]
actor_id = "owner"
owner = true
accounts = [
  { body_id = "body:whatsapp", account_id = "628123456789@s.whatsapp.net" },
  { body_id = "body:telegram", account_id = "123456789" },
]
```

Start the multi-body Gateway daemon:

```sh
source "$HOME/.cargo/env"
monoagens serve --config /path/to/monoagens.toml
```

---

## 7. Pre-Launch Checklist

- [ ] **Conformance TCK**: Bridge passes `monoagens-conformance body --target <bridge-bin> --level must` (`B-MUST-01..07`) and ideally `--level should` (`B-SHOULD-01..03`).
- [ ] **Canonical `PlaceId`s**: DMs and groups use `wa:dm:<jid>`, `wa:group:<jid>`, `tg:dm:<chat_id>`, `tg:group:<chat_id>`.
- [ ] **Non-empty `audience`**: Every `percept/emit` populates `[sender, self]` for DMs and active members for groups (`-32006` never bypassed).
- [ ] **`[[person]]` Owner Binding**: Owner accounts across WhatsApp/Telegram map to `actor_id = "owner"` (`owner = true`) in `monoagens.toml`.
- [ ] **Secret Isolation**: Platform tokens and API keys live only in `secrets.toml` / env vars.
- [ ] **Self-Echo Suppression**: Outbound `pid`s are recorded and ignored on inbound echo webhooks.
- [ ] **Crash Recovery & Idempotency**: `MONOAGENS_BODY_STATE_DIR` persists `idempotency_key` results and `state/query` answers within `5000 ms` after `SIGKILL`.
- [ ] **Media > `256 KiB`**: Large attachments transfer via `blob/put` / `blob/get` with SHA-256 verification instead of oversized inline payloads.
