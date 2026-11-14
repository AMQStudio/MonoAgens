# Later — Voice, Home Assistant and more model providers

Status: todo, not yet numbered | Depends on: Task 6 and the web panel task (its place in the order is decided with the panel)

**Goal.** The soul hears and speaks on every adapter, lives in the home through Home Assistant as
its first non-chat body, and can think through more model providers. Moved out of Task 5 Part 2
unchanged in substance (owner, 2026-11-13); the theory is in the spec (*Home Assistant as a body*,
*Speech: hearing and speaking*, *Adapters*).

---

## Part 1 — Speech, Home Assistant and more model providers

- [ ] **1.1 Speech for every adapter.**
  - *Function:* hearing and speaking as drivers by protocol, shared by every adapter and chosen by
    configuration: an OpenAI-shaped HTTP driver, Wyoming, ElevenLabs and Deepgram. Each reports
    its languages, with a warning at start when the soul's language is missing.
  - *Must hold:* a voice note in on WhatsApp, Telegram or Discord becomes a percept with its
    transcript (imported ones when opened; an honest description when transcription fails). Each
    chat adapter's speaking capability can send a voice note, with text as the fallback, and the
    words stay in the log. Barge-in is supported, and the receipt says how far the soul got. A
    realtime engine, if used, only transcribes and speaks: no utterance reaches a reply without
    the gate. Voice calls wait for a mature, widely released calling library; until then a call
    stays a percept and the soul answers in chat.
  - *Proof:* fake speech servers for each protocol; a fake device for barge-in; a test that no
    utterance bypasses the gate. A new network dependency is asked of the owner first.
- [ ] **1.2 Home Assistant as the first non-chat body.**
  - *Must hold:* the soul is Home Assistant's conversation agent; satellites' areas are shared
    places; exposed entities are sensed as home state and acted on as home control; listed entity
    changes are percepts; when a reply is late, a holding line is said and the answer announced
    later. A robot with its own adapter and reflexes comes later.
  - *Proof:* a fake Home Assistant runs the scenarios.
- [ ] **1.3 More model providers.**
  - *Must hold:* native Claude (with prompt caching), native Gemini and DeepSeek; optional separate
    models for sleep and the classifier.
- [ ] **1.4 One contract for every body.**
  - *Proof:* the non-chat body senses and acts through the same contract as the chat adapters;
    switching a speech or model provider is a configuration change only.

**Done when.** All parts are ticked; the soul hears and speaks on WhatsApp and through Home
Assistant; switching a speech or model provider is a configuration change; tests and a real run
are reported; the code is readable.

**Notes.**
