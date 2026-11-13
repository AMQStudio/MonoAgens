# Task 5 — Adapters: WhatsApp, Discord and more

Status: todo | Depends on: Task 4
One task = one worker prompt, done by one worker. Work through the parts **in order**. File names follow the layout of Task 3 ([glossary](../glossary.md)). Rules: [README](README.md).

**Goal.** The soul is usable every day: model providers as drivers with a fallback, the loop
and privacy fixes, nothing lost when work continues; then one soul across WhatsApp, Discord and
Telegram with linked identities so privacy follows people; then voice, a non-chat adapter and
more provider drivers.

---

## Part 0A — Providers (first)
Read first: [core.md](../core.md) §3 (the model port, D64), [glossary.md](../glossary.md), [project.md](../project.md) §2
May touch: `monoagens/infra/providers/` (new), `monoagens/boot.py`, `monoagens/__main__.py`, `scripts/replay.py`, `.env.example`, `README.md` (the model section), `tests/`; for A.1 only: `monoagens/core/reasoning.py`

All real-model work below depends on this part, so it comes first. V1 (branch `archived`) has
provider code under `monoagens/providers/` (Anthropic, Gemini, OpenAI); port what fits.

- [ ] A.1 The model port (D64): `core/reasoning.py` keeps building the neutral messages and tools and calls an injected model port (`decide(messages, tools, *, tool_choice) -> (tool calls, usage)`), as the embedder is injected; the HTTP request, headers and response parsing move to `infra/providers/openai_compatible.py`. A refactor that changes no behavior: the suite and the fake replay pass unchanged, and the core imports nothing from `infra/providers/`.
- [ ] A.2 The OpenCode Go driver (`infra/providers/opencode_go.py`): the endpoint from the official Go docs (`https://opencode.ai/inference/go/openai/v1/chat/completions` for DeepSeek V4.1 Flash, model `deepseek-v4.1-flash`); a stable `x-opencode-session` header per soul so the gateway can route and cache the stable prefix; thinking levels `low | high | max` mapped from `LLM_REASONING_EFFORT`; cached tokens read from the usage. Check the endpoint and the header against the live docs before coding; the docs move.
- [ ] A.3 Selection and fallback (`.env`): `LLM_PROVIDER` (`openai-compatible` default, `opencode-go`) and `LLM_FALLBACKS` (an ordered list of providers, each with its own key variable); a call that fails for quota, rate limit or a server error moves to the next, and every move is recorded in the Log with the reason; a refusal of the request itself (a bad argument) does not move. Every model call stays a gated, recorded call counted in the budget (AGENTS invariant 5).
- [ ] A.4 New baseline: the launch set once with the real model through OpenCode Go (`deepseek-v4.1-flash`, `--effort high`); record checks per scenario and tokens (cached included) under Notes. Later measurements compare with this baseline, not with Task 4's 24/26 on another gateway.

## Part 0B — The loop, brakes and one privacy truth
Read first: [middleware.md](../middleware.md) §3, §4; the Notes of Task 4
May touch: `monoagens/middleware/`, `tests/`, `scenarios/`, `scripts/replay.py`, `config.yaml`; for B.1 only, and only with the Log evidence: the core files that evidence names, reported first

- [ ] B.1 The loop in scenario 19 (Task 4 Notes: 467 model calls, 2.05 M tokens for six steps): find the exact cause in the Log (which events wake the soul again and again, and why: for example an expectation whose cue is already past firing at once), write it under Notes with the event ids, and fix it where it lives. A core fix needs the evidence and names its files before any edit.
- [ ] B.2 Brakes (middleware.md §4, D63): `max_task_steps` and `max_wakes_per_hour` per chain in `config.yaml` → `owner_settings` (starting values 30 and 20); a held brake stops the chain, is recorded, and tells the owner in the owner's direct chat which work stopped and why, with the choice to continue. Test: a scripted endless loop stops at the limit and the owner gets one message.
- [ ] B.3 One privacy truth (middleware.md §3): `privacy.leaks` answers from the audience labels like the context filter, so content visible at the target is never refused as a leak; a text-similarity check only catches content that is not visible there. Test: scenario 18 step 4 quotes the desk line and is sent; quoting the owner's private chat in a group is still refused.
- [ ] B.4 Measure: scenario 19 under 30 model calls with the real model, 18 step 4 passes, the launch set stays at or above the new baseline of A.4.

## Part 0C — Continuing work without losing anything
Read first: [middleware.md](../middleware.md) §4, §10, [plugins.md](../plugins.md) §5.3, [core.md](../core.md) §4 (D52 working memory, task threads)
May touch: `monoagens/middleware/`, `monoagens/infra/` (attachment store), `plugins/workspace/`, `tests/`, `scenarios/`, `scripts/replay.py`

- [ ] C.1 Full outputs (middleware.md §4, D62): an output past its cap is stored whole in the attachment store with the `act.result` as origin; the result carries the cut text and `full: {ref, size}`; `contract.OUTPUT_MARKER` names the ref and the offset; `registry.more(ref, offset, limit)` (permission at tier `guest`, facts `reads`) reads the rest of any tool's output through the attachment privacy check; hard forgetting removes it. Test: a 50 MB `run` output is read back completely in parts; it does not open from a place outside its audience.
- [ ] C.2 Workspace progress note (plugins.md §5.3): the standing rule for work of more than a few steps; `progress.md` in the workspace, updated after each step, read first on resuming.
- [ ] C.3 Scenario "paper over two days" (D54, all material in the scenario): (1) the owner asks to write a paper; (2) asks for a revision; (3) asks to send it to Budi (consent and file forwarding apply); (4) says "continue tonight" (an expectation with a time); **the process is restarted mid-work** before tonight; (5) the next day asks for a new paper in the same format. Checks: the work resumes from the right step after the restart (progress.md and `<doing>`), tonight's expectation fires once, the next day's paper follows the earlier format found in the workspace, nothing is claimed as done without its result, and every long output is reachable through its ref. Fake replay first, then the real model once with B.4's run.

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
- [ ] 1.2 WhatsApp with `history` and back-fill on connect; receiving and sending several files per message through the attachment store (middleware.md §10): refs in `data.attachments`, a description that sums them up, declared limits per message.
- [ ] 1.3 Discord, with several attachments per message (middleware.md §10).
- [ ] 1.3b Interaction rendering (middleware.md §9): WhatsApp and Discord declare what they can show and render typing, tool steps and approvals their own way (WhatsApp: "typing…", approval by reaction; Discord: typing, buttons); the decisions stay in `interaction.py`.
- [ ] 1.3c Roles from Discord: map Discord's own roles of a server to roles in `roles.py` (middleware.md §8) in the role definitions (e.g. `discord_role:` on a role), not per person; a Discord role grants nothing until the owner maps it.
- [ ] 1.4 Check: one conversation continues across WhatsApp, Discord and Telegram; a person's private context follows them only after a verified link; a file received on WhatsApp can be read and sent on.
- [ ] 1.5 Real-model test (this task's one exception to the last-part rule, because WhatsApp is needed daily): the replay launch set passes on the new adapters, then a live session on WhatsApp in the owner's direct chat and one group (the model in `.env`; never read or print it). Report what happened and the metrics. From here the owner uses the soul daily on WhatsApp.

## Part 2 — More adapters and models
Read first: [adapters.md](../adapters.md)
May touch: `monoagens/adapters/`, provider adapters, `tests/`, `scenarios/`

- [ ] 2.1 Voice (STT/TTS).
- [ ] 2.2 First non-chat adapter (a game such as Minecraft, an IoT device or a robot simulation) with its own reflexes inside the adapter.
- [ ] 2.3 More provider drivers in `infra/providers/` (Part 0A's port): native Claude (prompt caching), native Gemini, DeepSeek; optional separate models for sleep and the classifier.
- [ ] 2.4 Check: a non-chat adapter senses and acts through the same contract; switching provider is a `.env` change (`LLM_PROVIDER`).

**Done when.** All parts are ticked; the owner talks to the soul daily on WhatsApp; tests and
the real-model run are reported; the code is readable (AGENTS.md size guide).

**Notes.**
