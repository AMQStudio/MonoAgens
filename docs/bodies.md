# Bodies — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. What a body is
A body is an organ for a platform or device: it senses (`run(put)`) and/or acts (`act(action)`).
To the core, bodies (Telegram, CLI, WhatsApp, a speaker, a robot) and tool organs from plugins
(search, files) are the same thing ([core.md §4](core.md)). Bodies stand on the middleware like
plugins do. Like a sensory cortex, a body turns its native signal into a short description in
language plus raw data; like a motor cortex, it turns an intention into native actions.

## 2. Rules for every body
- Declare capabilities with the core's descriptor ([core.md §9.1](core.md)): name, JSON schema,
  description, `communicative`, `long_running`, `identity_changing`. The PFC's capability list
  is assembled from the organs present, in a fixed order (D2, D32).
- The brain decides intent and content; the body decides how (bubbles, TTS, gestures). Fine
  motor control stays in the body (D2).
- Sense through the core's `percept` ([core.md §9.2](core.md)): who, place, `addressed`, a short
  description, optional raw data, and `sent_at` when the sense has its own clock (D41).
- Report `addressed` (direct chat, reply, tag) as a neutral field (D40).
- Map native accounts and spaces through the identity module; translate native command syntax
  into a neutral `command`, adding presentation only (menus, buttons) (D9).
- Report long actions through `act.result`; never block the loop (D3).
- Mark speaking capabilities `communicative` (they count for talk rate and the forward model,
  D2) and capabilities that change who the soul is `identity_changing`.
- Ask for `cursor(organ)` on start, so input is never lost or duplicated.
- Optionally offer `history(place, before, limit)`; on connect, back-fill what was missed as
  `percept` with `backfill: true` and `sent_at`. The core appraises a backlog per place once, as
  one age-discounted percept; an addressed message still trips the circuit breaker (D37, D40).
- Own retries, rate limits, reconnects and platform quirks; errors become events, never crashes
  of the loop.
- Tokens and URLs come from `.env`; non-secret options (enabled or not, …) from `config.yaml`;
  nothing about a body lives in the core or in `soul.yaml`.

## 3. Present and planned bodies
| Body | Status | Notes |
|---|---|---|
| Telegram | present | long polling; the Bot API cannot read history, so the soul's own log is its "scroll up" |
| CLI | present | Unix socket inside the main process, thin client |
| WhatsApp | **Draft** | history and back-fill through a linked device |
| Discord | **Draft** | — |
| Voice | **Draft** | STT in, TTS out; description = transcript |
| IoT device / robot sim | **Draft** | first non-chat body; sensor readings as described percepts |

Tool organs (search, files, email) are in the [plugin catalog](plugins.md).
