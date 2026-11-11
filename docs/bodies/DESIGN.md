# Bodies — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. This layer builds on the core and may never change it — the core contract is in
[core/DESIGN.md](../core/DESIGN.md) and [core/FORMATS.md](../core/FORMATS.md) (frozen). Rulings:
[DECISIONS.md](DECISIONS.md).

## 1. What a body is
- A body is an organ for a platform or device: it senses (`run(put)`) and/or acts
  (`act(action)`). To the core, bodies (Telegram, CLI, WhatsApp, a speaker, a robot) and tool
  organs from plugins (search, files) are the same thing. Bodies stand on the middleware like
  plugins do.
- It declares its capabilities with the core's descriptor and senses through the core's
  `percept`. Like a sensory cortex, it turns its native signal into a short description in
  language plus raw data; like a motor cortex, it turns an intention into native actions.

## 2. Duties of every body
- Map native accounts and spaces to ids (through the identity module) and report `addressed`.
- Translate native command syntax into a neutral `command`; presentation only (menus, buttons).
- Ask for `cursor(organ)` on start, so input is never lost or duplicated.
- Report long actions through `act.result`; never block the loop.
- Own its retries, rate limits, reconnects and platform quirks; errors become events, never
  crashes of the loop.
- Optionally offer `history(place, before, limit)` and back-fill on connect (`backfill: true`,
  `sent_at`).
- Mark speaking capabilities `communicative`; mark capabilities that change who the soul is
  `identity_changing`.

## 3. Present and planned bodies
| Body | Kind | Status | Notes |
|---|---|---|---|
| Telegram | body | present | long polling; the Bot API cannot read history, so the soul's own log is its "scroll up" |
| CLI | body | present | Unix socket inside the main process, thin client |
| WhatsApp | body | planned | history and back-fill through a linked device |
| Discord | body | planned | — |
| Voice | body | planned | STT in, TTS out; description = transcript |
| IoT device / robot sim | body | planned | first non-chat body; sensor readings as described percepts |

Tool organs (search, files, email) are listed in the [plugin catalog](../plugins/DESIGN.md).

## 4. Configuration
Tokens and URLs come from `.env`; nothing about a body lives in the core or in `soul.yaml`.
