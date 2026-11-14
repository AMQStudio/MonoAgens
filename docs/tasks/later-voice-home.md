# Later — Voice, Home Assistant and more model providers

Status: todo, not yet numbered | Depends on: Task 6 and the web panel task (its place in the order is decided with the panel)
One task = one worker prompt, done by one worker. Work through the parts **in order**. Rules: [AGENTS.md](../../AGENTS.md).

**Goal.** The soul hears and speaks on every adapter through drivers by protocol, lives in
the home through Home Assistant as its first non-chat body, and can think through more
model providers. Moved out of Task 5 Part 2 unchanged (owner, 2026-11-13); the specification is
final in [spec.md](../spec.md) *Home Assistant as a body* and *Terms*.

## Part 1 — Speech, Home Assistant and more model providers
Read first: [spec.md](../spec.md) *Adapters*
May touch: `monoagens/adapters/`, `monoagens/infra/providers/` (speech and model drivers), `integrations/homeassistant/` (new: the HA custom integration), `monoagens/middleware/` (interaction, for `voice`), `.env.example`, `tests/`, `scenarios/`

- [ ] 1.1 Speech for every adapter ([spec.md](../spec.md) *Speech: hearing and speaking*): drivers by protocol in
  `infra/providers/speech/` — OpenAI-shaped HTTP (ported from V1), Wyoming (new, plain asyncio),
  ElevenLabs and Deepgram (ported) — chosen by `.env`, shared by every adapter, each reporting its
  languages with a boot warning when the soul's language is missing. Voice notes in on WhatsApp,
  Telegram and Discord become percepts with their transcript (imported ones on open; an honest
  description when STT fails); each chat adapter's own speaking capability takes `voice: true`
  and sends the platform's voice note, text as the fallback, the words kept in the Log. Barge-in
  and `spoken_until` in the receipt's result, tested with a fake device. A realtime engine, if set, only transcribes and speaks; its own replies are off,
  and a test proves no utterance reaches a reply without the gate. Tests with fake STT/TTS
  servers for each door. A websocket client, if needed, asks the owner first. Voice calls wait
  for a mature, widely released calling library ([spec.md](../spec.md) *Later ideas (not scheduled)*); until then a call stays a
  percept and the soul answers in chat.
- [ ] 1.2 First non-chat adapter: Home Assistant as a body ([spec.md](../spec.md) *Home Assistant as a body*): the soul as HA's
  conversation agent through a small HA custom integration, satellites' areas as shared places,
  exposed entities as `home.state` and `home.control`, listed entity changes as percepts, the
  reply deadline with a holding line and a later announce. Tests with a fake HA. A robot with its
  own adapter and reflexes comes later.
- [ ] 1.3 More provider drivers in `infra/providers/` (Part 0A's port): native Claude (prompt caching), native Gemini, DeepSeek; optional separate models for sleep and the classifier.
- [ ] 1.4 Check: a non-chat adapter senses and acts through the same contract; switching provider is a `.env` change (`LLM_PROVIDER`).

**Done when.** All parts are ticked; the soul hears and speaks on WhatsApp and through Home
Assistant; switching a speech or model provider is a `.env` change; tests and a real run are
reported; the code is readable (AGENTS.md size guide).

**Notes.**
