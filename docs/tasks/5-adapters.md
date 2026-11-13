# Task 5 — Adapters: WhatsApp, Discord and more

Status: todo | Depends on: Task 4
One task = one worker prompt, done by one worker. Work through the parts **in order**. File names follow the layout of Task 3 ([glossary](../glossary.md)). Rules: [README](README.md).

**Goal.** The soul is usable every day: model providers as drivers with a fallback, the loop
and privacy fixes, nothing lost when work continues; then one soul across WhatsApp, Discord and
Telegram with one id per person so privacy follows people, every adapter (Telegram and CLI
included) using everything its platform offers. The task ends with Part 1; voice, Home Assistant
and more provider drivers moved to [later-voice-home.md](later-voice-home.md) (owner, 2026-11-13).

---

## Part 0A — Providers (first)
Read first: [core.md](../core.md) §3 (the model port, D64), [glossary.md](../glossary.md), [project.md](../project.md) §2
May touch: `monoagens/infra/providers/` (new), `monoagens/boot.py`, `monoagens/__main__.py`, `scripts/replay.py`, `.env.example`, `README.md` (the model section), `tests/`; for A.1 only: `monoagens/core/reasoning.py`

All real-model work below depends on this part, so it comes first. V1 (branch `archived`) has
provider code under `monoagens/providers/` (Anthropic, Gemini, OpenAI); port what fits.

- [x] A.1 The model port (D64): `core/reasoning.py` keeps building the neutral messages and tools and calls an injected model port (`decide(messages, tools, *, tool_choice) -> (tool calls, usage)`), as the embedder is injected; the HTTP request, headers and response parsing move to `infra/providers/openai_compatible.py`. A refactor that changes no behavior: the suite and the fake replay pass unchanged, and the core imports nothing from `infra/providers/`.
- [x] A.2 The OpenCode Go driver (`infra/providers/opencode_go.py`): the endpoint from the official Go docs (`https://opencode.ai/inference/go/openai/v1/chat/completions` for DeepSeek V4.1 Flash, model `deepseek-v4.1-flash`); a stable `x-opencode-session` header per soul so the gateway can route and cache the stable prefix; thinking levels `low | high | max` mapped from `LLM_REASONING_EFFORT`; cached tokens read from the usage. Check the endpoint and the header against the live docs before coding; the docs move.
- [x] A.3 Selection and fallback (`.env`): `LLM_PROVIDER` (`openai-compatible` default, `opencode-go`) and `LLM_FALLBACKS` (an ordered list of providers, each with its own key variable); a call that fails for quota, rate limit or a server error moves to the next, and every move is recorded in the Log with the reason; a refusal of the request itself (a bad argument) does not move. Every model call stays a gated, recorded call counted in the budget (AGENTS invariant 5).
- [x] A.4 New baseline: the launch set once with the real model through OpenCode Go (`deepseek-v4.1-flash`, `--effort high`); record checks per scenario and tokens (cached included) under Notes. Later measurements compare with this baseline, not with Task 4's 24/26 on another gateway.

## Part 0B — The loop, brakes and one privacy truth
Read first: [middleware.md](../middleware.md) §3, §4; the Notes of Task 4
May touch: `monoagens/middleware/`, `tests/`, `scenarios/`, `scripts/replay.py`, `config.yaml`; for B.1 only, and only with the Log evidence: the core files that evidence names, reported first

- [x] B.1 The loop in scenario 19 (Task 4 Notes: 467 model calls, 2.05 M tokens for six steps): find the exact cause in the Log (which events wake the soul again and again, and why: for example an expectation whose cue is already past firing at once), write it under Notes with the event ids, and fix it where it lives. A core fix needs the evidence and names its files before any edit.
- [x] B.2 Brakes (middleware.md §4, D63): `max_task_steps` and `max_wakes_per_hour` per chain in `config.yaml` → `owner_settings` (starting values 30 and 20); a held brake stops the chain, is recorded, and tells the owner in the owner's direct chat which work stopped and why, with the choice to continue. Test: a scripted endless loop stops at the limit and the owner gets one message.
- [x] B.3 One privacy truth (middleware.md §3): `privacy.leaks` answers from the audience labels like the context filter, so content visible at the target is never refused as a leak; a text-similarity check only catches content that is not visible there. Test: scenario 18 step 4 quotes the desk line and is sent; quoting the owner's private chat in a group is still refused.
- [x] B.4 Measure: scenario 19 under 30 model calls with the real model, 18 step 4 passes, the launch set stays at or above the new baseline of A.4.

## Part 0C — Continuing work without losing anything
Read first: [middleware.md](../middleware.md) §4, §10, [plugins.md](../plugins.md) §5.3, [core.md](../core.md) §4 (D52 working memory, task threads)
May touch: `monoagens/middleware/`, `monoagens/infra/` (attachment store), `plugins/workspace/`, `tests/`, `scenarios/`, `scripts/replay.py`

- [ ] C.1 Full outputs (middleware.md §4, D62): an output past its cap is stored whole in the attachment store with the `act.result` as origin; the result carries the cut text and `full: {ref, size}`; `contract.OUTPUT_MARKER` names the ref and the offset; `registry.more(ref, offset, limit)` (permission at tier `guest`, facts `reads`) reads the rest of any tool's output through the attachment privacy check; hard forgetting removes it. Test: a 50 MB `run` output is read back completely in parts; it does not open from a place outside its audience.
- [ ] C.2 Workspace progress note (plugins.md §5.3): the standing rule for work of more than a few steps; `progress.md` in the workspace, updated after each step, read first on resuming.
- [ ] C.3 Scenario "paper over two days" (D54, all material in the scenario): (1) the owner asks to write a paper; (2) asks for a revision; (3) asks to send it to Budi (consent and file forwarding apply); (4) says "continue tonight" (an expectation with a time); **the process is restarted mid-work** before tonight; (5) the next day asks for a new paper in the same format. Checks: the work resumes from the right step after the restart (progress.md and `<doing>`), tonight's expectation fires once, the next day's paper follows the earlier format found in the workspace, nothing is claimed as done without its result, and every long output is reachable through its ref. Fake replay first, then the real model once with B.4's run.

## Part 1 — Identity, places, WhatsApp and Discord
Read first: [adapters.md](../adapters.md) (all), [middleware.md](../middleware.md) §7 (identity), §8 (platform roles), §9 (interaction), §10 (attachments), [core.md](../core.md) §4 (history and import), §8 (two ids become one), §9.2
May touch: `monoagens/adapters/`, `monoagens/middleware/` (identity, interaction, roles, contract), `monoagens/infra/` (attachment store: deferred refs), `config.yaml`, `pyproject.toml` and `uv.lock` (the adapter extras approved in AGENTS.md), `.gitignore`, `.env.example`, `tests/`, `scenarios/`, `scripts/replay.py`; `monoagens/core/` only for `source.merged` (1.1) and `imported` (1.3), and only after the plan and the files it names are written under Notes (`kernel.py` or `store.py` included only if named)

V1 (branch `archived`) already has working WhatsApp (neonize: media, history sync, identity,
directory), Discord (discord.py) and speech provider code under `monoagens/gateways/` and
`monoagens/providers/`. Port it to the adapter contract of Task 4 instead of starting over; leave
its per-gateway policy and roles out, since policy lives in the middleware. Model providers were
ported in Part 0A. Decisions: D65 one person one id, D66 an account shared with a person, D67
native first, D68 places, D69 back-fill and import, D70 what is shown.

Identity comes first: it is what lets a person's private context follow them (D45), so a wrong
link is a leak.

**Every adapter is native** (D67, adapters.md §1, §4). What Part 1 says of WhatsApp and Discord
holds for every adapter, Telegram and CLI included: everything the platform delivers becomes a
percept, everything the platform can show is used, and each gap is declared with its reason. No
adapter is a special case.

- [ ] 1.0 Accounts in, person ids out (D65): adapters report native accounts (`telegram:123`,
  never `person:tg:123`); the inbound hook maps `who`, `data.mentions` and `data.reply_to.who`
  through the account map and registers an unknown account at once (`identity.registered`,
  `person:p_<random>`, one id per account under a unique key); the owner stays `person:owner`.
  Existing development data uses the old ids: the owner decided (2026-11-13) to reset the
  development database, not to convert it. Test: a new account gets one id, two racing
  percepts of it get the same id, a mention is mapped, the core sees no account.
- [ ] 1.1 Linking as a merge (D65, middleware.md §7, core.md §8): `/link` by the owner and by
  code (single use, 10 minutes, three tries), `/unlink`; `identity.merged` into the older id
  (`person:owner` always wins); the middleware puts `source.merged` and the core applies it to
  its own books as core.md §8 says, reads of the log following the merge. Tests: two ids with
  history merge into one card and one memory, `recall` finds both sides, audience labels of old
  lines follow the merge, a wrong or expired code links nothing, `/unlink` gives a new id from
  then on and splits nothing, the soul has no tool that links.
- [ ] 1.2 Places (D68, adapters.md §3): place info with `form`, `parent`, `can_post`, `name`,
  `members` and platform role facts, kept by identity; Telegram reports it first (private chat,
  group, forum topic as `thread`, channel as `broadcast`). Test: a send to a place where
  `can_post` is false is refused with its reason; `/whoami` shows the place's form and parent.
- [ ] 1.3 WhatsApp (neonize, adapters.md §4–§6): one stable form per account (LID and phone
  number are one account); everything in as percepts (text, replies, mentions, edits,
  deletions, reactions, stickers, locations, contact cards, polls, view-once, albums as one
  percept with several attachments, calls as percepts (incoming, missed, ended), group events); out through `say` (attachments, reply_to,
  mentions, sticker, choices, thread, location), `react`, `edit`; the shared account told apart by message ids in the Log
  (D66); full history import with `imported: true` (core: never appraised as a wake, summarized
  by sleep only for places that become active) and back-fill on reconnect; media as deferred
  refs, fetched on open, with a re-upload request when the link expired; WhatsApp Channels as
  `broadcast`, Communities as `parent`. Tests: an import of 2,000 messages in 40 chats makes no
  model call; a message typed on the owner's phone is a percept from the owner and the soul's
  own message is not; an album of three photos is one percept; a deferred photo opens once
  fetched.
- [ ] 1.4 Discord (discord.py, adapters.md §4): servers as `parent`, channels, threads, DMs;
  everything in as on WhatsApp; buttons and menus drawn by the adapter, interactions
  acknowledged within 3 s by deferring inside the adapter; several attachments per message;
  the Discord role ids of the sender reported with each percept.
- [ ] 1.4b Telegram and CLI to native (D67, adapters.md §4, §7): Telegram reads and shows
  everything the Bot API offers (inline buttons and the command menu, edit, delete, reply,
  reactions, polls, stickers, albums, locations, contacts, forum topics, channels, typing), with
  `say`, `react` and `edit` as on the others; the CLI shows what a terminal can (typing and
  progress as status lines, buttons as numbered choices). The coverage declaration and the
  contract test of adapters.md §4 run over every adapter (Telegram, CLI, WhatsApp, Discord);
  each gap is declared with its reason (Telegram: no history, no read receipts). `say`'s
  neutral fields (`choices`, `thread`, `location`) work on every adapter through its best native
  form or the fallback. Platform-only actions (pin, reject a call, Discord role management, …) are adapter-own
  tools, off by default, offered only when listed under `bodies.<adapter>.tools:`; test that an
  unlisted one is not in the prompt.
- [ ] 1.5 What is shown (D70, middleware.md §9): `contract.SHOWS` with the full vocabulary;
  settings from `interaction:` and `bodies.<adapter>.show:`; `/show <name> <value> [adapter]`;
  thinking `off | owner_direct | all` (all still passes `privacy.visible` per thought); typing only
  when the soul is going to speak (replace Task 4's rule that also shows it for long actions);
  read receipts after the decision is recorded, never for imported or back-filled percepts, and
  on an account shared with the owner only where the soul has spoken since; progress by edit or
  new message; the fallbacks in the code and the boot report (approval: buttons → reactions →
  text; choices: buttons → poll → numbered list; thread → reply with the title; location →
  text; sticker → emoji). The decisions stay in `interaction.py`; adapters only draw. Test per
  setting; a silent decision shows no typing.
- [ ] 1.6 Roles from Discord (D68, middleware.md §8): `discord_role:` on a role, by ids; held only
  in places whose `parent` chain reaches that server; a role taken away on Discord is gone at the
  next percept; an unmapped Discord role grants nothing. Test: the same person is a moderator in
  the server's channel and not in a DM or on WhatsApp.
- [ ] 1.7 Check: one conversation continues across WhatsApp, Discord and Telegram; the
  coverage table of every adapter is in the boot report with no undeclared gap; a person's
  private context follows them only after a verified link; a file received on WhatsApp can be
  read and sent on; a sticker received can be sent back.
- [ ] 1.8 Real-model test (this task's one exception to the last-part rule, because WhatsApp is
  needed daily): the replay launch set passes on the new adapters, then a live session on
  WhatsApp in the owner's direct chat and one group (the model in `.env`; never read or print
  it). Report what happened and the metrics. From here the owner uses the soul daily on WhatsApp.

**Done when.** All parts are ticked; the owner talks to the soul daily on WhatsApp; tests and
the real-model run are reported; the code is readable (AGENTS.md size guide).

**Notes.**

### A.1 Core edit plan
- Exact core file: `monoagens/core/reasoning.py`. No other core file is modified.
- Change: remove `httpx` and HTTP transport/request/response code from `monoagens/core/reasoning.py`; define `ModelPort` (`Protocol` with `decide(messages, tools, *, tool_choice="required") -> tuple[list[ToolCall], dict[str, Any]]` and `aclose() -> None`); refactor `Reasoning` to wrap an injected `ModelPort`. Move the OpenAI-compatible HTTP client (`_http`, `_post`, `aclose`, `_usage`, headers, retries) to `monoagens/infra/providers/openai_compatible.py`.
- Progress A.1: done — 539 passed, 1 skipped (`uv run pytest`); fake replay 133/133 passed; core import invariant clean.
- Progress A.2: done — OpenCode Go live docs (`https://opencode.ai/docs/go/`, checked 2026-11-13) verified (`https://opencode.ai/inference/go/openai/v1/chat/completions`, `x-opencode-session` derived via UUIDv5 from soul id, `medium` mapped to `high` with boot note); 542 passed, 1 skipped (`uv run pytest`); fake replay 133/133 passed.
- Progress A.3: done — `FallbackProvider`, `load_provider_config`, `build_provider`, additive `provider` & `fallbacks` on `model.call` in `budget.py`; 546 passed, 1 skipped (`uv run pytest`); fake replay 133/133 passed.
- Progress A.4: done — OpenCode Go real-model launch set (`deepseek-v4.1-flash`, `--effort high`): **24/26** scenarios in full, **72/75** checks, 94 calls, **260,089** total tokens (223,988 prompt · 136,320 cached · 36,101 completion · 19,720 reasoning); missed-help 0.0%, false-speak 0.0%, absolute-line violations 0, honesty 96.2%.

### A.4 Launch-set baseline (OpenCode Go, `deepseek-v4.1-flash`, `--effort high`, 2026-11-13)
Report: `reports/2026-11-13-1110-real-17-07-11-12-20-23-25-27-29-01-02-03-15-21-04-05-06-14-22-24-08-09-10-13-16-30.md`.
Failing scenarios: `23_recall_episodes` (5/7), `24_gate_threshold` (10/11).

| Scenario | Checks | LLM calls | Prompt | Cached | Completion | Total tokens | Result |
|---|---|---|---|---|---|---|---|
| 17_cross_body_continuity | 1/1 | 2 | 4596 | 2048 | 788 | 5384 | PASS |
| 07_dm_restart_continuity | 2/2 | 3 | 7233 | 6272 | 1013 | 8246 | PASS |
| 11_sleep_memory_consolidation | 2/2 | 3 | 5314 | 4096 | 1137 | 6451 | PASS |
| 12_topic_recall_after_sleep | 2/2 | 3 | 7391 | 2176 | 991 | 8382 | PASS |
| 20_act_r_activation | 3/3 | 18 | 48468 | 26880 | 6220 | 54688 | PASS |
| 23_recall_episodes | 5/7 | 5 | 12700 | 10368 | 1644 | 14344 | FAIL |
| 25_help_place_name | 4/4 | 2 | 2890 | 0 | 817 | 3707 | PASS |
| 27_help_wifi_password | 4/4 | 2 | 2882 | 0 | 1050 | 3932 | PASS |
| 29_help_clinic_hours | 4/4 | 2 | 2892 | 0 | 790 | 3682 | PASS |
| 01_group_chitchat_greeting | 1/1 | 0 | 0 | 0 | 0 | 0 | PASS |
| 02_group_chitchat_lunch | 1/1 | 0 | 0 | 0 | 0 | 0 | PASS |
| 03_group_chitchat_weather | 1/1 | 0 | 0 | 0 | 0 | 0 | PASS |
| 15_urgent_unaddressed_group | 1/1 | 0 | 0 | 0 | 0 | 0 | PASS |
| 21_own_name_capture | 3/3 | 0 | 0 | 0 | 0 | 0 | PASS |
| 04_group_mention | 1/1 | 1 | 2182 | 768 | 394 | 2576 | PASS |
| 05_group_reply_to_bot | 1/1 | 2 | 4482 | 2048 | 1448 | 5930 | PASS |
| 06_dm_conversation | 1/1 | 1 | 2238 | 2048 | 86 | 2324 | PASS |
| 14_busy_group_repeated_mention | 3/3 | 15 | 45604 | 32640 | 4330 | 49934 | PASS |
| 22_patience_turn_taking | 7/7 | 2 | 2859 | 0 | 547 | 3406 | PASS |
| 24_gate_threshold | 10/11 | 4 | 9256 | 4096 | 2179 | 11435 | FAIL |
| 08_reminder_at_time | 1/1 | 3 | 5278 | 4096 | 791 | 6069 | PASS |
| 09_reminder_ask_again | 1/1 | 4 | 7681 | 6272 | 1567 | 9248 | PASS |
| 10_reminder_recurring_nudge | 1/1 | 9 | 19788 | 10880 | 5342 | 25130 | PASS |
| 13_fired_reminder_different_place | 3/3 | 7 | 15764 | 10624 | 1800 | 17564 | PASS |
| 16_watch_for_phrase | 4/4 | 2 | 4661 | 2944 | 1053 | 5714 | PASS |
| 30_same_topic_question | 5/5 | 4 | 9829 | 8064 | 2114 | 11943 | PASS |

### B.1 Log evidence & core edit plan (observed before editing `monoagens/core/`)
- **Live replay Log capture (`REPLAY_DATABASE_URL`, Scenario 19, OpenCode Go `deepseek-v4.1-flash`, events `#1`..`#61`)** + **Task 4 real-model report (`reports/2026-11-13-0457-real-all.md`, 467 calls / 2,047,899 tokens, 22 `task.opened` / 21 `task.closed` reported earlier)**:
  1. **Batch key mismatch inside task threads (`monoagens/core/kernel.py`)**:
     - `#13 percept` (`place=space:game:world1`) ran `#19 dev.run` and opened `#20 task.opened` (`place=space:task:13`).
     - In round `#32 act.result` (`place=space:game:world1`), `_think` ran inside `job.place = "space:task:13"`, emitting two fast acts: `#36 workspace.read` (`caused_by=32`, `place=space:game:world1`) and `#37 self.recall` (`caused_by=32`, `place=space:game:world1`). `_track_batch` keyed `self._batches[(32, "space:task:13")]`.
     - When `#38 act.result` (`caused_by=36`, `place=space:game:world1`) arrived, `_batch_holds` looked up `(32, "space:game:world1")`, missed `(32, "space:task:13")`, returned `False`, and started `job #39` immediately; `#40 act.result` (`caused_by=37`) also missed, found `pfc:space:task:13` busy, and queued `ask_re_wake` (`job #45`).
     - `job #39` emitted `#43 dev.run` + `#44 self.recall` (`#46`, `#47` results -> job `#52`, `#58 decision.note`, `#59 workspace.run` -> `#61`), while parallel `job #45` emitted `#50 dev.run` + `#51 self.recall` (`#53`, `#54` results -> job `#60`), multiplying concurrent branches exponentially on every multi-tool round.
  2. **Closed task chains re-opening from leftover queued/in-flight `act.result` events (`monoagens/core/threads.py`, `monoagens/core/kernel.py`)**:
     - Once an exploding task hit `max_task_steps` (30) and wrote `task.closed` (`status="closed"`), `threads.open_tasks` excluded it, so `threads.place_for` returned `None` and `job.re_wake` / remaining `act.result` events woke `space:game:world1` instead, called `long_running` tools, and `threads.open_for` opened brand-new `space:task:<act.result.id>` threads repeatedly (`task.opened` / `task.closed` cycle in Task 4 report lines 888, 909, 912).
  3. **Batched sibling result hidden behind `earlier result #...` (`monoagens/core/context/working.py`)**:
     - When two tools run in one round (e.g. `#38 workspace.read` + `#40 self.recall`), `working._lines` turns `#38` into `"earlier result #38 (recall it)"` and shows only `#40` (`"nothing found"`), forcing an extra round to recall the sibling result.
- **Core files to edit**:
  1. `monoagens/core/kernel.py`: match `_batch_holds` / `_flush_batches` across parent/task places and direct `act.result.caused_by` triggers (e.g. synchronous approval refusals `#31/#32`); include `long_running` calls when a round emits multiple result-producing acts (mixed fast+slow rounds previously forked into parallel branches); skip `job.re_wake` when a target `space:task:*` is no longer open; silently drop orphaned `act.result` events whose task chain closed with `max_task_steps` or `stopped`.
  2. `monoagens/core/threads.py`: add `is_open` and `closed_reason` (checking both `caused_by` lineage and in-flight acts `[since, closed_event)` on a closed task for the same parent) so stale `act.result` events cannot re-open a closed chain; walk `caused_by` in `_goal_for` so a task opened on an `act.result` keeps its chain's triggering goal instead of `(unknown)`.
  3. `monoagens/core/context/working.py`: in `_lines`, keep all `act.result` lines from the latest round's trigger (`caused_by` of its `act.out`, when non-None) visible so batched tool outputs are not hidden by a sibling result.
- **B.3 & Scenario 18 Step 8 / 8b (D45, D46, D80; D54)**:
  - Unifying `privacy.leaks` with `filter_context` across memories and events (`percept`, `act.out`, `act.result`) exposed a contradiction in Scenario 18 step 8 (`18_game_world_file_and_share.yaml`): Budi replied `"Cocok kok, cuma warnanya agak terlalu terang."` in his private WhatsApp DM (`space:wa:628110001@s.whatsapp.net`) while Charlie was already in `space:game:world1`, yet the old check expected that quote verbatim in `world1` without consent (D54: the old check forbade correct D45 behavior, and `filter_context` already withheld Budi's DM from `world1`'s desk).
  - Implemented **D80** (`docs/middleware.md` §3): at intake (`privacy.delegation` in `hooks.inbound`), a single matching reply to a delegated send (`wait` expectation whose chain sent a communicative `act.out`, or `reply_to` referencing it) adds the delegator `X` (`person:owner`) to that one reply event's audience (`state: delegated:<event_id>`); later unrelated lines from Budi and Fulan's DM keep only their own audience and cannot be shared by the owner. Updated Scenario 18 step 8 to ask the owner without quoting Budi's words (allowing either asking in `world1` or answering in `space:game:dm:`) and added step 8b (`"di sini aja"`, step 9 in the log) proving `share.granted` unlocks Budi's matched reply for `world1` before `act.out`.
- **Progress**: B.1 fixed and verified (Scenario 19 loop eliminated); B.2 brakes (`max_task_steps`, `max_wakes_per_hour` in `config.yaml -> owner_settings`, `brake.held` / `brake.resumed`, owner direct notice with continue prompt) implemented and verified; B.3 unified `privacy.leaks` + D80 delegation rule and Scenario 18 Step 8/8b implemented and verified (`uv run pytest`, `scripts/replay.py --fake` 137/137); B.4 measured (2026-11-13, `deepseek-v4.1-flash`, `--effort high`).
- **B.4 Part 0B real-model measurements (`deepseek-v4.1-flash`, `--effort high`, 2026-11-13)**:
  - **Scenario 19** (`reports/2026-11-13-1239-real-18-19.md`): **16 LLM calls** (down from 467 calls / 2.05 M tokens in Task 4; well under the 30-call cap), **62,103 total tokens** (53,265 prompt · 38,656 cached · 8,838 completion · 6,375 reasoning), 11/21 checks passed (`brake.held` cleanly stopped step 4 at limit).
  - **Scenario 18** (`reports/2026-11-13-1239-real-18-19.md`, `reports/2026-11-13-1243-real-18.md`): 10/23 and 8/23 checks. Step 4's `privacy.leaks` allows quoting the visible desk line (`test_privacy.py` passes and `prompt_contains 'sketsa_logo'` passes on both real runs with 0 `guard.stopped` in `world1` at step 4), while `deepseek-v4.1-flash` itself chose not to quote Budi's WhatsApp file in `world1` (deferring to DM, so step 4's `mentions 'sketsa'` remains a Task 6 model-behavior failure).
  - **Launch set (26 scenarios)** (`reports/2026-11-13-1251-real-17-07-11-12-20-23-25-27-29-01-02-03-15-21-04-05-06-14-22-24-08-09-10-13-16-30.md`): **24/26** scenarios in full, **72/75** checks (matches A.4 baseline), **76 LLM calls** (down from 94), **193,881 total tokens** (162,165 prompt · 90,880 cached · 31,716 completion · 22,191 reasoning; down from 260,089), missed-help **0.0%**, false-speak **0.0%**, absolute-line violations **0**, honesty **98.1%** (`23_recall_episodes` 5/7, `24_gate_threshold` 10/11).
