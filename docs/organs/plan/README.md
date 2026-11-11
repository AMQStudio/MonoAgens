# Organs — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder (one per track or phase,
e.g. `A2.md`), tick steps and record findings. They never change DESIGN, DECISIONS, FORMATS or
MATH; a design question stops the work and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

## Done
- Telegram (long polling) and CLI (Unix socket, thin client).

## With core Step 1 (Track A2)
- Telegram and CLI declare their capabilities (`reply`, `react`, …) and emit `percept` with
  `addressed` (D40, D41).

## Step 6 — World organs and multi-step work
- First tool organs: web search/fetch and file read. Long-running actions report through
  `act.result`; limits on steps per task.
- Done when: "who is the president now?" triggers a search when a search organ exists, and an
  honest "as of my knowledge" answer when it does not.

## Step 8 — More bodies, more models
- WhatsApp (with `history` and back-fill on connect, D37), Discord, voice (STT/TTS), the first
  non-chat organ (IoT device or robot simulation).
- Native provider adapters; optional separate models for sleep and the classifier.
- Done when: one conversation continues across three bodies and a non-chat organ senses and acts
  through the same contract; switching provider is a `.env` change.
