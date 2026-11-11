# Organs (bodies and tools) — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing in this layer may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Organs (bodies and tools)
- An organ senses (`run(put)`) and/or acts (`act(action)`) and declares capabilities: name, JSON
  schema, description, flags `communicative`, `long_running`, `identity_changing`. The PFC's
  capability list is assembled from the organs present, in a fixed order (D2, D32).
- The brain decides intent and content; the organ decides how (bubbles, TTS, gestures). Fine
  motor control stays in the organ (D2).
- Long actions report progress and completion as `act.result`; the core never waits (D3).
- `communicative` capabilities count as speaking for talk rate and the forward model (D2).
- On start an organ asks for `cursor(organ)`, so input is never lost or duplicated.
- Organs translate native accounts into neutral ids (through the alias map) and native command
  syntax into a neutral `command` event, adding presentation only (menus, buttons) (D9).
- Organs report `addressed` (direct chat, reply, tag) as a neutral field (D40).
- What an organ senses enters as a `percept`: who, place, `addressed`, a short description in
  language, optional raw data, and `sent_at` when the sense has its own clock. The organ, like a
  sensory cortex, turns its signal into that description (D41).
- A body may offer `history(place, before, limit)`; on connect it back-fills what it missed as
  `percept` with `backfill: true` and `sent_at`. The core appraises a backlog per place once, as
  one age-discounted percept; an addressed message still trips the circuit breaker (D37, D40).
- Tools are organs too (search, files, email) (D35). The only organ inside the core is `self`.
- Present: Telegram (long polling) and CLI (Unix socket, thin client).
- **Draft**: WhatsApp (with history), Discord, voice (STT/TTS), a first non-chat organ (IoT
  device or robot simulation); web search/fetch and file read as the first tool organs. Each
  organ owns its own retries, rate limits and platform quirks.
