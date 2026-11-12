# Task 5 — Adapters: WhatsApp, Discord and more

Status: todo | Depends on: Task 4
One task = one worker prompt, done by one worker. Work through the parts **in order**. File names follow the layout of Task 3 ([glossary](../glossary.md)). Rules: [README](README.md).

**Goal.** The soul is usable every day: one soul across WhatsApp, Discord and Telegram first,
with linked identities so privacy follows people; then voice, a non-chat adapter and native
model providers.

---

## Part 1 — Identity linking, WhatsApp and Discord (first)
Read first: [adapters.md](../adapters.md), [middleware.md](../middleware.md) §7 (identity), [core.md](../core.md) §4, §8
May touch: `monoagens/adapters/`, `monoagens/middleware/` (identity part), provider adapters, `tests/`, `scenarios/`

V1 (branch `archived`) already has working WhatsApp (media, history sync, identity), Discord,
Slack and provider (Anthropic, Gemini, OpenAI, speech) code under `monoagens/gateways/` and
`monoagens/providers/`. Port it to the adapter contract of Task 4 instead of starting over; leave
its per-gateway policy and roles out, since policy lives in the middleware.

Linking comes first: it is what lets a person's private context follow them (D45), so a wrong
link is a leak.

- [ ] 1.1 Identity linking (`/link`): the owner links accounts, or the person confirms with a verification code sent from the second account; every link is an event and can be undone; the soul may suspect and ask, never link silently.
- [ ] 1.2 WhatsApp with `history` and back-fill on connect; receiving and sending files as capabilities (a received file is a percept with a description and a reference in `data`).
- [ ] 1.3 Discord.
- [ ] 1.3b Interaction rendering (middleware.md §9): WhatsApp and Discord declare what they can show and render typing, tool steps and approvals their own way (WhatsApp: "typing…", approval by reaction; Discord: typing, buttons); the decisions stay in `interaction.py`.
- [ ] 1.4 Check: one conversation continues across WhatsApp, Discord and Telegram; a person's private context follows them only after a verified link; a file received on WhatsApp can be read and sent on.
- [ ] 1.5 Real-model test (this task's one exception to the last-part rule, because WhatsApp is needed daily): the replay launch set passes on the new adapters, then a live session on WhatsApp in the owner's direct chat and one group (the model in `.env`; never read or print it). Report what happened and the metrics. From here the owner uses the soul daily on WhatsApp.

## Part 2 — More adapters and models
Read first: [adapters.md](../adapters.md)
May touch: `monoagens/adapters/`, provider adapters, `tests/`, `scenarios/`

- [ ] 2.1 Voice (STT/TTS).
- [ ] 2.2 First non-chat adapter (a game such as Minecraft, an IoT device or a robot simulation) with its own reflexes inside the adapter.
- [ ] 2.3 Native provider adapters; optional separate models for sleep and the classifier.
- [ ] 2.4 Check: a non-chat adapter senses and acts through the same contract; switching provider is a `.env` change.

**Done when.** All parts are ticked; the owner talks to the soul daily on WhatsApp; tests and
the real-model run are reported; the code is readable (AGENTS.md size guide).

**Notes.**
