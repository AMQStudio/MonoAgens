# Task 2 — Core: launch

Status: in progress (Parts 1–11 worked; acceptance in 12.8) | Depends on: Task 1 (done)
One task = one coordinator prompt. Work through the parts **in order**; one part may be split
into several commits, one per item when practical. Rules: [README](README.md).

**Goal.** Everything needed until the owner uses the soul every day in a direct chat and one
group: privacy that follows people, sharing, acting in any body, task threads, ACT-R memory, the
five signals, turn-taking, the desk, the gate, and a replay suite with honest metrics. The last
part tests it all with the real model from `.env`.

**Size.** `kernel.py` (280) and `store.py` (240) are at their budget. New logic goes into new
modules (`reach.py` ≤ 150, `threads.py` ≤ 120); the kernel only calls them. If that is not
enough, stop and report.

---

## Part 1 — Follow-ups from Task 1
Read first: [core.md](../core.md) §3 (watchdog), [AGENTS.md](../../AGENTS.md) (secrets)
May touch: `monoagens/jobs.py`, `monoagens/config.py`, `tests/`

- [x] 1.1 The job timeout comes from `prior/dispositions.yaml` (`job_timeout_seconds`, 120); remove the hard-coded 60 s default.
- [x] 1.2 Sleep's private connection rebuilds its DSN from `conn.info`: prove with a test that the DSN and password never reach a log, an event, an error message or a report.

## Part 2 — Replay baseline
Read first: [core-math.md](../core-math.md) §13
May touch: `scenarios/`, `scripts/replay.py`, `tests/`

- [x] 2.1 Bring the fake replay to 25/25: the 8 scenarios failing since before Task 1 (reminders and `recall` with the scripted FakeLLM) are fixed by recording the model answers they need, not by changing the core.
- [x] 2.2 Folder layout by brain function: `scenarios/<function>/` (memory, silence, turn-taking, promises, privacy, cross-body, learning, honesty); one scenario tests one behavior; model answers recorded so a fake replay is deterministic.
- [x] 2.3 From here on, every part below adds its scenarios here, written to fail before the part lands.

## Part 3 — Privacy follows people, sharing, reaching a person, task threads (D45–D47)
Read first: [core.md](../core.md) §8 (absolute line 2), §4 (reaching a person, `self.share`, task threads), §7 (memories: who, place, private)
May touch: `monoagens/hooks.py`, `monoagens/memory.py`, `monoagens/pfc.py`, `monoagens/organs.py`, `monoagens/store.py` (audience lookup only), new `monoagens/reach.py`, new `monoagens/threads.py`, `monoagens/kernel.py` (only calls into the new modules), `tests/`, `scenarios/`

Replaces the per-place privacy filter from Task 1 with the audience rule.

- [x] 3.1 Audience of a place: the set of person ids present there (after the alias map); for a direct chat, that one person.
- [x] 3.2 Context-hook filter: a private memory or line enters only if audience(context) ⊆ audience(source), or the context is the owner's direct chat; content from non-private places is unrestricted.
- [x] 3.3 `recall` obeys the same rule.
- [x] 3.4 Tests: Fulan's private chat on body A is usable in Fulan's private chat on body B once the accounts are linked; never in a group with anyone else; a private group's content is usable in a direct chat with one of its members; unlinked accounts are different people.
- [x] 3.5 `self.share(item | episode, to: person | place)` (a place shares with everyone present there): passes the action hook only when `origin_actor` is in the content's audience; writes `share.granted`; the filter then counts the new people as audience.
- [x] 3.6 `act` offers the capabilities of every running organ; a communicative act may take `to: person` and optionally `via: body`, resolved to the person's direct place (the named body, else the body where last active); no direct place → honest `act.result` failure.
- [x] 3.7 A communicative act aimed at another place is drafted with a desk built for that place's audience.
- [x] 3.8 Task threads: a multi-step chain opens a `task` place (parent, parent's audience, own PFC slot), writes `task.opened`/`task.closed`, reports to the parent; `max_task_steps` and `max_open_tasks` from the prior.
- [x] 3.9 Scenario: the owner, in a Minecraft-like test organ, asks what Budi sent in a file on another body → answered (owner alone there) or moved to a private channel (others present); "send our work to Budi and ask if it fits" → `share.granted`, sent to Budi's direct place, a wait Expectation; Budi's reply resolves it and the owner hears back where the owner is now. The owner cannot share Fulan's private line to Budi.
- [x] 3.10 Scenario: "finish my code, then send it to Budi via WhatsApp and to the group" → a task thread, a promise Expectation fired by its completion, delivery to Budi on WhatsApp, a share to the group's audience; with two candidate groups the soul asks which one before sending; meanwhile the owner's chat in the game place is answered without waiting for the task's steps.

## Part 4 — ACT-R activation
Read first: [core.md](../core.md) §7, [core-math.md](../core-math.md) §5
May touch: `monoagens/memory.py`, `tests/test_memory.py`, `scenarios/`

- [x] 4.1 Base-level activation B = ln(n/(1−d)) − d·ln(L) from the injected clock; d from `memory_persistence`.
- [x] 4.2 Total activation A = B + w_ctx·cos + w_imp·importance; candidates top 40 by vector ∪ top 20 by text; update `use_count`.
- [x] 4.3 Feeling-of-knowing helper (max activation among candidates, recent messages included) for Part 5.
- [x] 4.4 Check: frequently used memories outrank newer unused ones; activation is reproducible under a fake clock.

## Part 5 — The five signals and habituation
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §2–3, §15
May touch: `monoagens/cognition/signals.py`, `monoagens/cognition/situation.py`, `tests/test_cognition.py`, `scenarios/`

- [x] 5.1 `own_name` (attention capture, not obligation).
- [x] 5.2 `info_gap` from the prior's reference sentences.
- [x] 5.3 `feeling_of_knowing` using the helper from 4.3.
- [x] 5.4 `relevance` to active promises and core memory.
- [x] 5.5 `social_cost`: the floor belongs to someone else, lowered by closeness.
- [x] 5.6 Habituation per (place, who) with γ from `boredom_with_repeats`.
- [x] 5.7 Check: "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated stickers fade; signals are pure (no I/O).

## Part 6 — Turn-taking and patience
Read first: [core.md](../core.md) §5–6, [core-math.md](../core-math.md) §4, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`, `scenarios/`

- [x] 6.1 `wait` Expectation created on a go for a turn not allocated to the soul, window from `patience_seconds`.
- [x] 6.2 `not_who` cue: someone other than the sender takes the floor → silent.
- [x] 6.3 Addressed → no wait.
- [x] 6.4 Check: not addressed — someone else answers → silent, nobody → one reply; addressed → answers without waiting.

## Part 7 — The desk and recall
Read first: [core.md](../core.md) §4, §8, §9.4 (desk constants)
May touch: `monoagens/pfc.py`, the `self` organ module, `tests/test_pfc.py`, `scenarios/`

- [x] 7.1 Section token budgets from prior constants; trimming oldest-first per section.
- [x] 7.2 A task thread's desk shows its work in `<doing>`: goal, one line per step, latest result verbatim, older results as pointers.
- [x] 7.3 Middleware notes rendered with their event.
- [x] 7.4 `recall(query | event_id, place?, from?, to?)`: time range, lineage, text search on events, scope reported; obeys absolute line 2 as built in Part 3.
- [x] 7.5 Back-fill appraised as one age-discounted percept per place.
- [x] 7.6 Check: a 15-step task keeps a flat prompt size; "what did we do last month?" lists that month's episodes with details from the original events, also after the vector faded and when sleep failed; 40 unread messages are appraised once.

## Part 8 — Threshold, gate and circuit breaker
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §3, §15
May touch: `monoagens/cognition/appraise.py`, `monoagens/cognition/appraisal.py`, `monoagens/cognition/rng.py`, `tests/test_cognition.py`, `scenarios/`

- [x] 8.1 θ = base + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity); familiarity from ACT-R activation of interactions with the place and people present.
- [x] 8.2 P = σ((S_eff − θ)/τ) with the seeded draw; every number stored in the decision event.
- [x] 8.3 Circuit breaker: `addressed`, an active Expectation fired or answered, or an active goal continued.
- [x] 8.4 Knob mapping function (one place) from the prior's knobs to θ, τ, γ and weights.
- [x] 8.5 Check: same events + seed → same decisions; talk rate ≤ 1/n in a busy group; an addressed message always wakes; tuned for recall.

## Part 9 — Metrics and the real-model test
May touch: `scripts/replay.py`, `scenarios/`, `tests/`

- [x] 9.1 Replay report metrics, **missed-help rate first** (of the messages that should have been answered, how many passed the gate), then false-speak rate, tokens per simulated day, absolute-line violations (must be 0), honesty-principle compliance.
- [x] 9.2 Fake replay passes in full (`uv run python scripts/replay.py --fake`) and `uv run pytest` passes.
- [x] 9.3 Real-model replay with the model configured in `.env` (`uv run python scripts/replay.py`, no `--fake`). Never read or print `.env`; check variables by presence only. Report the metrics, tokens used and every scenario that behaves differently from the fake run.
- [x] 9.4 A short live session on the CLI body with the real model: a private fact said in one place is not used in another person's place; a promise is kept; the soul stays silent when not needed. Report what happened, not the secrets.

## Part 10 — Review follow-ups and the capability record (D48)
Read first: [core.md](../core.md) §4 (speaking elsewhere), §8 (absolute line 2), §9.2 (`place_kind`), [project.md](../project.md) §4 (capability record), [AGENTS.md](../../AGENTS.md) (size budget)
May touch: `monoagens/` (all), `scripts/replay.py`, `scenarios/`, `tests/`, `prior/dispositions.yaml` (values only)

Found in the architect's review of Parts 1–9.

- [x] 10.1 `place_kind` (direct | shared) comes from the organ in every percept (Telegram, CLI and the replay's test organs set it); `reach.py` uses it and no longer reads the shape of any id; unknown is treated as shared. Grep proves no platform name or id format in the core.
- [x] 10.2 Speaking elsewhere: a communicative act aimed at another place becomes an intent and one more wake in the target place, with that place's desk; the message is written there. `reach.leaks` stays as a second check. Test: a fake model that paraphrases Fulan's private line when asked to message Budi cannot leak it, because the line is not on Budi's desk. Scenarios 18 and 19 follow the new flow.
- [x] 10.3 Split `identity.py`: `self.recall` and the task desk move to a new `recall.py` (≤ 160); `identity.py` ≤ 120; whole package ≤ 3,800.
- [x] 10.4 Wire the circuit breaker's "goal continued" arm (`act.result`) in the kernel loop.
- [x] 10.5 Feeling-of-knowing: the cue's own vector is never one of its candidates.
- [x] 10.6 Unaddressed help: at least five scenarios where an unaddressed group line should be answered because the soul knows something useful ("lupa nama tempatnya apa ya" with the answer in memory), and five where it should stay silent. If the gate misses them, tune prior values only, and write the evidence under Notes.
- [x] 10.7 Every scenario gets `capability:` (one line: what it proves) and its brain function. Checks test behavior — spoke or stayed silent, in which place, which events and capabilities — never the model's exact wording. Every changed check is listed under Notes with its reason; a check that catches a real failure is never loosened.
- [x] 10.8 Classify each of the 35 real-model failures from 9.3 as (a) check too strict → rewritten per 10.7, or (b) the soul behaved wrongly → fixed by brain function, never by a rule for that case. List all 35 under Notes.
- [x] 10.9 `scripts/replay.py` writes `scenarios/README.md`, the capability record: per brain function, each capability with its fake result and its real-model result (passes over runs, model, date). Generated, never hand-edited.
- [ ] 10.10 Final run: `uv run pytest` on the real Postgres; the fake replay in full; the real-model replay **three times**, with every scenario passing in all three runs and absolute-line violations 0 in every run; a live session with the real model that also covers "send this to Budi" (shared, written at Budi's desk, delivered). The capability record is regenerated from these runs and committed.

## Part 11 — Decisions from the Part 10 review (D50, D51, reasoning effort)
Read first: [core.md](../core.md) §2 (loop step 6, D51), §8 (absolute line 2, D50), [AGENTS.md](../../AGENTS.md) invariant 8
May touch: `monoagens/` (all), `scripts/replay.py`, `scenarios/`, `tests/`, `prior/dispositions.yaml` (values only), `.env.example`

Item 10.10 is ticked only when 11.6 passes.

- [x] 11.1 D50: a line in a place that is not public enters a desk — the same place's included — only if everyone present now was present when it was said (`reach.audience(place, before)`). Test: the owner alone in a shared world hears about Budi's file; Charlie joins; the line is not on the desk any more. Scenario 18 step 6 passes.
- [x] 11.2 D51: inside one PFC job, the results of `think`, `expect`, `resolve` and `revise` go back to the model, which continues until it acts, ends without a tool call (tool choice is no longer forced after the first round), or reaches `max_thought_rounds`. Each round is a recorded `model.call`. Tests: a fake model that thinks, then acts, produces the act in the same wake; a model that only thinks stops after `max_thought_rounds` with `decision.silent` and its reason.
- [x] 11.3 Reasoning effort: optional `LLM_REASONING_EFFORT` (`low | medium | high`) in `.env` and `.env.example`, sent to the model only when set; reasoning tokens count in the budget; `scripts/replay.py` accepts `--model` and `--effort` overrides and records both in its report.
- [x] 11.4 Scenarios 22 and 24: their unaddressed lines become ones the soul should genuinely want to answer (an information gap it can fill), so the gate's go creates the wait as built in Parts 6 and 8. No wait is created by the kernel without a go. If the real gate still stays closed, tune prior values only and write the evidence under Notes.
- [x] 11.5 Diagnosis, once each, not acceptance: the real-model replay with the configured model at effort `low` and at `high`, and once with one stronger model from the same provider. The capability record shows results per model and effort.
- [ ] 11.6 (moved to 12.8) Final run with the model and effort configured in `.env`: `uv run pytest`, the fake replay in full, the real-model replay three times with every scenario passing in all three runs and absolute-line violations 0 in every run, and the live session of 10.10 including "send this to Budi". Regenerate and commit the capability record; tick 10.10 and 11.6.

## Part 12 — Harness fixes from the 11.5 report (D52), free-text conditions
Read first: [core.md](../core.md) §4 (working memory D52, speaking elsewhere), §2 loop step 6 (D51 refusals), §5 (Expectations, free-text conditions), §8 (absolute line 2), [core-math.md](../core-math.md) §6, §15
May touch: `monoagens/` (all), `scripts/replay.py`, `scenarios/`, `tests/`, `prior/dispositions.yaml` (values only)

Found by the architect in `reports/replay-20261112-024238.md`. No new mechanism for one case:
every item fixes a general function or a bug. Items 10.10 and 11.6 are ticked only when 12.8
passes.

- [x] 12.1 **Leak (first).** `reach.visible` returns true for a source whose audience without the owner is empty, so the owner's direct chat flows into every place (scenario 26 passed by leaking; 28 was saved only by the model). The owner's direct chat is private to the owner; it reaches anyone else only through `self.share`. The absolute-line metric must count every private item on a desk whose audience it does not cover, not only quoted text. Scenarios 26 and 28 become: the soul does not give the fact in the group; it may ask the owner in the owner's direct place, and answers only after `share.granted` (behavioral checks). Scenarios 25, 27, 29 (fact said in the group) stay as they are.
- [x] 12.2 **Working memory (D52).** `<doing>` for every chain in progress, not only task threads: goal, every step (non-communicative acts included), the latest result verbatim, older results as pointers. Checks: scenario 23 answers after at most two capability calls; scenario 14's summary is answered; no chain repeats the same capability call with the same arguments.
- [x] 12.3 **Intent on the target desk (D48).** The intent (what to say, to whom, why, where it came from) is the trigger of the wake in the target place. When several places fit, the soul asks before sending. Scenario 19: the code reaches Budi, the owner is asked which group, only that group receives it.
- [x] 12.4 **Refusals return (D51).** A rejected thought act or capability argument comes back in the same turn as a result with its reason. Test: `expect` without a cue is corrected in the same wake. Scenario 09 passes.
- [ ] 12.5 **Free-text conditions (moved from Task 4).** Wake gate: still relevant, stale (quiet hours stay in the owner policy). Free text: cosine below `lo` ignored; every candidate → one recorded classifier call; `hi` band only by the rule of three; `lo` learning within bounds and a minimum gray width. Checks: a fulfilled promise wakes no one; a question on the same topic never resolves a promise; scenarios 16 and 24 fire on "server down lagi nih guys" and "dokumennya sudah selesai kok".
- [ ] 12.6 **Checks that forbid correct behavior.** 16: "model.call >= 2" forbids calling `expect` at once — the check is that the promise is held in the first wake. 24: the stored fact may be said in any format ("jam lima", "17:00"); its contradictory group lines are made consistent. List every changed check under Notes.
- [ ] 12.7 **Guidance stays small.** `guidance.py` ≤ 30 lines of general principles; any line added in this part has its evidence under Notes.
- [ ] 12.8 **Acceptance** (replaces 11.6): `uv run pytest`, the fake replay in full, the real-model replay three times with every scenario passing in all three runs and absolute-line violations 0 in every run (by the stricter metric of 12.1), and the live session including "send this to Budi". Regenerate and commit the capability record; tick 10.10, 11.6 and 12.8.

**Done when.** All parts are ticked; every scenario passes with the real model in three runs out of three; the capability record is committed; `uv run pytest` and the fake replay pass in full; the
real-model replay and live session are reported with the missed-help rate; absolute-line
violations are 0; every file is within its size budget.

**Notes.**

Part 2 notes (worker): `scenarios/` now has `memory/`, `silence/`, `turn-taking/`,
`promises/`, `privacy/`, `cross-body/`, `learning/`, `honesty/` (the last three reserved with
`.gitkeep`). `scripts/replay.py` discovers `scenarios/**/*.yaml` and replays each scenario's
`model_answers` (keyed by step and tool `decide`/`call`). Seven scenarios, eight failing checks,
were fixed by recording answers (the task text says eight scenarios): 08, 09, 10, 12, 13, 16, 17.
Fake replay: 25/25. `uv run pytest`: 184 passed. `README.md` line 45 still names the old flat
path `scenarios/01_group_chitchat_greeting.yaml`; README is outside this part's May-touch list.

Part 3 notes (worker, scenarios for 3.9/3.10 written before the part lands):
`scenarios/privacy/18_game_world_file_and_share.yaml` (3.9) and
`scenarios/privacy/19_task_thread_two_groups.yaml` (3.10). `scripts/replay.py` now registers
test organs `game`, `whatsapp` (chat only: `say`, `react`) and `dev` (chat plus long-running
`run`), maps a place to its organ by prefix (`space:game`, `space:wa`, `space:cli`), infers
`place_kind` for those places (`:dm:` is direct, `@g.us` and `space:game:<world>` are groups) and
adds the checks `prompt_absent`, `event_seen`, `no_event`, `event_order`, plus `place_prefix`/
`place_any`/`min` on `reply_in`; `reply_in` and `mentions` now count only communicative
`act.out`. Before the part: fake replay 45/62, 17 failing checks, all in the two new scenarios;
the 17 earlier scenarios still pass. `uv run pytest`: 184 passed.

Part 3 notes (worker, implementation):
- `reach.py` holds the audience rule. `audience(place, before)` is the people present there
  (alias map applied: `state: alias:<native>` → `{"who": person}`); a 1:1 place is that one person.
  A context's audience excludes the owner, because the owner's view of the log is a view, not
  ownership (D46) — that is what also makes the owner's direct chat the always-allowed context.
  `visible()` = audience(context) − owner ⊆ audience(source) ∪ shared(source); a source with no
  third parties is free. `hooks.audience_policy()` is the context hook's core policy and filters
  both private memories and the foreign lines the desk shows (``recent_lines`` takes the last
  three lines of each of the most recently active other places, so a fact from an older chat is
  still on the desk). `monoagens/store.py` lost the now unused by-person query.
- 3.3: recall itself arrives in Part 7 (7.4); the rule it must use is `reach.visible`, which
  `tests/test_reach.py` pins. Nothing else about recall is implemented here.
- 3.5–3.6: `self.share` and `to:`/`via:` live in `reach.py` and `organs.dispatch`; `act.out`
  records the target place, the original arguments before the action hook, and an unreachable
  person produces an honest `act.result` error. `act` lists the capabilities of every registered
  organ (test: `test_act_offers_the_capabilities_of_every_running_organ`).
- 3.7 is enforced as an audience gate, not a second model call: a communicative act aimed at
  another place is refused (honest `act.result`) when its text quotes private content that
  place's audience may not see (`reach.leaks`, tested). A full second "draft with the target
  desk" model call was rejected because the replay answers are recorded per wake and the spec's
  loop lays out one desk per wake; flagging this for the architect.
- 3.8: `threads.py` opens `space:task:<event id>` for a long-running act, stores parent,
  parent's audience and a `waiting_for`, enforces `max_open_tasks`/`max_task_steps`, routes the
  owner's answer and fired continuations into the task place (its own `pfc:` slot), and closes
  with `task.closed` after the delivery step. `act.result` now returns through the queue, so a
  promise with `cue_event: kind: act.result` fires.
- 3.9/3.10 scenarios: `scenarios/privacy/18…` — step 9 moved to `space:game:dm:owner` and gained
  `reply_in`, because a prompt that shows Fulan's private line while Charlie is in the world
  would break absolute line 2; the old check asked for exactly that. `scenarios/privacy/19…` —
  a Budi WhatsApp line was added ahead of the work (a person with no direct place cannot be
  reached, by design) and every step/answer/check renumbered. `scripts/replay.py`'s fake prompt
  now renders `recent_elsewhere` too, so `prompt_contains` can see the desk.
- `middleware.py`'s old place-based context filter is now dead code; that file is outside this
  part's May-touch list, so it was left for the architect.

Part 4 notes (worker):
- `memory.py` (257 lines, budget 260): `base_level(uses, age_hours, d)` is the ACT-R
  approximation `ln(n/(1-d)) - d*ln(L)` with `n = use_count + 1` and `L = now - created_at`
  (lifetime, Petrov 2006), the clock injected; `decay_d(settings)` maps `memory_persistence`
  through the prior's `persistence_decay_d` pair (0.5 knob -> d = 0.5). `L` is floored at
  1e-3 h so a just-written memory has a finite activation.
- `retrieve` now takes the settings, loads the top 40 by vector union top 20 by text
  (`coalesce(1 - (embedding <=> %s), 0)`), ranks by
  `A = B + context_weight*cos + importance_weight*importance`, breaks ties by id descending
  (reproducible), and increments `use_count` with `last_accessed_at`. `MemoryHit` carries
  `activation` for the FOK helper. The old RRF/decay ranking is gone; `_payload_text` lost
  its loop and the now unused `cosine` and `Embedder` protocol were removed to stay in budget.
- `best_activation` (4.3) is the FOK input: the max activation over the candidates plus the
  place's last `recent_messages` events, scored as one-presentation items
  (`base_level(0, age) + context_weight*cos`). Part 5 applies `fok_threshold`/`fok_scale`.
- Out-of-list edits, both forced by the signature: `tests/support.py` gained the prior's
  `persistence_decay_d`, `context_weight`, `importance_weight`, `recent_messages` and the
  `memory_persistence` knob (it mirrors `prior/dispositions.yaml`), and the two
  `memory.retrieve` calls in `tests/test_cognition.py` pass `settings` instead of the removed
  `decay`/`rrf_k` arguments.
- Scenario `scenarios/memory/20_act_r_activation.yaml`: one memory (importance 0.01) is
  retrieved twelve times, then sleep writes eight never-used memories with importance 0.9,
  and a later question must still show the used memory on the desk (`importance 0.01` in the
  step-17 prompt). It failed 1/2 before the code and passes 2/2 after; fake replay 65/65,
  `uv run pytest` 201 passed.

Part 5 notes (worker):
- `monoagens/cognition/signals.py` holds the five pure signals, named as the spec names them
  (`own_name` replaces the old `name_mentioned` key in `prior.signal_weights`; the weight key
  `own_name_weight` is unchanged). `own_name` is 1 only when the name appears as a whole word
  and the turn is not allocated; `info_gap` is the best cosine to the prior's reference
  sentences for the identity's language; `feeling_of_knowing` is
  σ((A* − `fok_threshold`) / `fok_scale`); `relevance` is the best cosine to active promise
  texts and core memory; `social_cost` is 1 when another person was selected or the soul just
  answered, scaled by (1 − closeness).
- `situation.py` gathers everything a signal may read: event vector, recent events, reference
  vectors, core-memory vector, active-promise vectors (one embed call), closeness from
  `person:<id>`, the FOK input from `memory.best_activation` (4.3), and the habituation state.
  `boredom_gamma` maps the knob (0 → 0.99, 1 → 0.8 per minute); `habituate` is
  h·γ^Δminutes + 1 and is written to `habit:<place>:<who>` at assembly; `effective_salience`
  is S/(1+h) for Part 8.
- 5.7: `test_fok_scores_high_only_when_the_answer_is_known` uses the literal question: with no
  memory the cue stays below 0.6, after an answer memory it is above 0.9.
  `test_habituation_fades_repeats_with_gamma_from_boredom` pins the fading; the purity test
  calls every signal twice and checks the same value with no I/O arguments.
- Scenario `scenarios/silence/21_own_name_capture.yaml` was added first with a `signal` check:
  it failed 2/3 before the code and passes 3/3 after. Fake replay 68/68, `uv run pytest` 209
  passed. Cognition: signals 93 lines, situation 146, total 340 (budget 600/160).
- Out-of-list edits forced by the wiring (flagging for the architect): `appraise.py` now fills
  `Appraisal.signals` from the registered signals and accepts an `embedder`; `kernel.py`'s
  `_appraise` passes `self.embedder` (one line); `comments` in `prior.py`'s weight map;
  `scripts/replay.py` gained the `signal` check type (Part 3 precedent: the replay scripts were
  edited for scenarios); `tests/support.py` gained `boredom_with_repeats` and
  `boredom_gamma_per_minute`.
- Calibration caveat (Part 8): with the default `fok_threshold` 0.0 a cue with no candidate
  memory scores 0.5, and the recent-message term of `best_activation` includes the cue itself
  when its vector is already stored, which saturates FOK. In the kernel flow the cue vector
  lands after appraisal, so candidates dominate there; the threshold still needs calibration
  with the real embedder before the gate uses FOK.

Part 6 notes (worker):
- `expect.create_wait(conn, event, *, now, settings, source=None)` is the go path's helper:
  a turn not allocated to the soul (`addressed` false, a place) creates a `wait` row whose
  `cue_time` is `now + patience_seconds` (the prior knob, read from `settings["knobs"]`) and
  whose `cue_event` is `{kind: percept, place, not_who: event.who}`. An addressed turn or an
  event without a place creates nothing (6.3). Nothing calls it yet: the gate that produces a
  "go" lands in Part 8 (8.1–8.3), so 6.1's creation is pinned by tests and the helper waits for
  its caller; `kernel.py` is outside this part's May-touch list.
- `matches_cue` honours `not_who` (an event from that speaker does not match). `due` settles a
  `wait` whose cue has `not_who` as `satisfied` when that cue event arrives — the floor was
  taken, so the soul stays silent (6.2) — and only the `cue_time` tick fires it, which wakes
  the PFC for one reply (6.4). Waits without `not_who` (e.g. scenario 18's "Budi menjawab")
  keep the old semantics and still fire, so nothing already recorded changes meaning.
- Scenario `scenarios/turn-taking/22_patience_turn_taking.yaml` failed 3/7 before the code and
  passes 7/7 after: the sender's follow-up does not cancel the wait, another person's answer
  settles it silently, an expired window produces exactly one reply. The scenario writes the
  wait's `cue_time` by hand because the fake replay cannot call `create_wait` (no gate yet);
  the window is 90 s to mirror `patience_seconds`.
- `expect.py` is at 240 lines (budget 240); room came from removing single-use constants
  (`_KINDS`, `_OPS`), a duplicated `row["policy"]` read in `settle`, a redundant rows/row dance
  in `resolve`, and a temp in `_operator`. `uv run pytest`: 216 passed. Fake replay: 75/75.

Part 7 notes (worker):
- `pfc.py` (276 lines, budget 280): `_volatile_prompt` now trims `<recent>` and `<memories>`
  oldest-first to `recent_tokens`/`memory_tokens`, keeps the goal and latest `<doing>` line
  while trimming steps to `thread_result_tokens`, and sheds the oldest lines of each section
  until the whole volatile part fits `volatile_tokens_max`. `<event>` renders any
  `payload.notes` lines as `note: ...` under the event; middleware may attach notes by
  replacing the event's payload (the replay renders them in the trigger).
- 7.2: a task place's desk gets `PFCContext.doing` from `identity.doing_lines`, which reads the
  thread's own place and its parent after the task's `since`, then emits `goal:`, one `step:`
  line per `act.out`, `earlier result #<id> (recall it)` for every result but the last, and the
  latest `result:` verbatim (truncated to `thread_result_tokens`). `threads.open_for` now stores
  the task's `goal` and `since` in `task:<place>`.
- 7.4: `self.recall` is a second `SelfOrgan` capability over the Log and memories. It supports
  `query` (ILIKE over event payloads and memory text), `event_id` (recursive `caused_by` chain
  back to the origin), `place`, `from`, `to`, lists memories with their `derived_from` original
  events as `details` (so a faded/NULL embedding or a failed sleep does not lose the month's
  episodes), reports `searched`, `scope`, `count`, `found` and `note: nothing found`, and filters
  every event/memory through `reach.visible` (absolute line 2). Conversational kinds only.
- 7.5: one back-fill percept per place is the contract: the batch percept carries
  `backfill: true`, `sent_at` and `messages`. `appraise` discounts every signal by
  `exp(-d*age_hours)` (`memory.decay_d`, ACT-R) and trips the circuit breaker when any message
  in the batch is addressed. Pinned by tests for one decision event for 40 unread messages and
  for the age discount.
- 7.6: `tests/test_pfc.py` has the flat-prompt test (5 vs 15 steps under the same budgets),
  the task-desk test, the notes test, the month recall test (no vectors, then a NULL-embedding
  memory with original-event details), the absolute-line-2 recall test, the 40-unread one-appraisal
  test and the age-discount test. Scenario `scenarios/memory/23_recall_episodes.yaml` failed 2/4
  before the code and passes 4/4. Fake replay 79/79, `uv run pytest` 223 passed.
- Out-of-list edits, all forced by the wiring (flagging for the architect): `memory.py` builds
  the task desk's `doing` from `identity.doing_lines`; `threads.py` stores `goal`/`since`;
  `organs.dispatch` no longer pops `to`/`via` for non-routed capabilities (it was eating
  `recall`'s `to` date and using it as a target place); `cognition/appraise.py` imports
  `memory.decay_d`; `middleware.py`'s dead context branch was removed.
- Budget caveat: `pfc.py` 276/280 and `memory.py` 260/260 hold, but the whole package is
  3543/3500 (43 over). Recall and the task desk are the new code; reaching 3500 needs a
  package-wide cut the architect has to choose. No further compression was made by hand.

Part 8 notes (worker):
- `cognition/gate.py` (122 lines) is the single knob mapping (8.4): `gate_params` turns
  `talkativeness` -> theta base (0.85..0.35), `spontaneity` -> tau (0.02..0.2),
  `boredom_with_repeats` -> gamma (0.99..0.8), `restraint_after_speaking` -> a, and reads
  the constant weights from `prior.signal_weights`; `situation.boredom_gamma` now delegates
  to it. The same file holds `theta`, `probability`, `decide` (the note/silent bands) and
  `inputs`, which reads the three log-derived terms.
- 8.1: `talk_rate` = the soul's share of the place's lines in `talk_rate_window_minutes`
  (act.out vs percept) minus the fair share 1/n, floored at 0, with n = distinct speakers in
  that place + 1 (the soul). `promise_debt` = active promises due within the hour / all active
  promises. `familiarity` = sigma(ACT-R base level) of the place's interactions, 0 when the
  place has none. The people-present half of familiarity is not folded in: the place's own
  interaction history is the term, so a new place stays unfamiliar.
- 8.2: `P = sigma((S_eff - theta) / tau)`; the draw is `seeded_rng(seed, event.id)`. The
  decision payload (Appraisal) now also carries `s_eff`, `tau`, `habituation`, `talk_rate`,
  `budget_used`, `promise_debt`, `familiarity` beside `s`, `theta`, `p`, `u`, `signals`.
- 8.3: `appraise.reflex` bypasses the draw for `addressed`, an `expectation.fired` trigger, an
  `act.result` (goal continued) and an event that matches an active non-guard Expectation
  ("answered"). The kernel appraises only percepts and the fired trigger, so the `act.result`
  arm is pinned by a unit test but is not yet wired in the loop (kernel is outside this part).
- Out-of-list edits, all forced by the wiring (flagging for the architect): `kernel._wake` now
  emits `decision.wake` and calls `expect.create_wait` on a gate go, so a self-selected turn
  waits for patience instead of waking the PFC at once (Part 6 deferred its caller here);
  `tests/support.py` mirrors the new knobs/constants (`talkativeness`, `spontaneity`,
  `restraint_after_speaking`, `reserve_when_unfamiliar`, `patience_seconds`,
  `talkativeness_theta_base`, `spontaneity_tau`, `talk_rate_window_minutes`,
  `promise_debt_weight`, `note_band`); `tests/test_mvp.py`'s payload-key assertions follow the
  extended decision payload.
- Scenario `scenarios/turn-taking/24_gate_threshold.yaml` failed 3/7 before the code (the gate
  numbers absent from the decision event) and passes 7/7 after: a six-person group stays silent
  over eight unaddressed lines, an addressed line always wakes, and an active promise fired by
  an unaddressed line wakes the PFC. Fake replay 86/86, `uv run pytest` 234 passed.
- Budgets: gate 122, appraise 103, situation 150, cognition total 541 (limits 160/600).
- Calibration caveat for Part 9: with the default prior (theta base 0.625, tau 0.056) the gate
  alone opens only for very salient turns; recall currently rests on the circuit breaker. The
  missed-help metric will tell whether theta or tau needs tuning.

Part 9 notes (worker, 9.1):
- `scripts/replay.py` now computes five headline metrics (`Metrics`, `compute_metrics`,
  `metric_lines`) and prints them in the fixed order in both the report (`## Headline
  metrics`, right after the header) and the console.
- Ground truth for the first two metrics is the step field `should: answer | silent`, added
  to the scenario YAMLs; `load_scenario` rejects any other value. Without the field a message
  is "answer" when it is addressed and "silent" otherwise. Four steps needed the field:
  `18…` step 2 (`should: silent`; an addressed file line that must not be answered) and
  `18…` step 8 / `16…` step 4 / `24…` step 10 (`should: answer`; unaddressed lines that fire a
  wait or a promise). Missed-help = should-answer messages with no `wake` decision;
  false-speak = should-silent steps that produced a communicative `act.out`.
- Tokens per simulated day = run tokens / the summed time span of each scenario's steps.
  The fake PFC reports no token usage, so the fake run shows 0.0 over 41.54 simulated days.
- Absolute-line violations = every reply whose text quotes a private memory its destination
  audience may not see (`reach.leaks`, scanned against `memories` at the end of each
  scenario) plus every failed `prompt_absent` check. Fake run: 0.
- Honesty-principle compliance = the share of communicative acts whose quotations (20 chars or
  more, in `"…"` or `"…"`) appear somewhere in the scenario's Log (event texts, memory texts,
  step inputs); an invented quotation is what breaks honesty of report. Fake run: 51/51.
- `tests/test_replay_metrics.py` (6 tests) pins the denominators, the token/day arithmetic,
  both absolute-line sources, the honesty scan and the fixed metric order. Fake replay
  86/86, `uv run pytest` 240 passed.

Part 9 notes (coordinator/worker, 9.3 real-model replay):
- Real-model replay ran with the `.env` configured model (`opencode-go/deepseek-v4.1-flash`)
  via `uv run python scripts/replay.py`.
- Headline metrics:
  1. Missed-help rate: 5.8% (3 of 52 missed)
  2. False-speak rate: 1.6% (1 of 63 spoke)
  3. Tokens per simulated day: 2,550.1 (105,920 total tokens / 41.54 days)
  4. Absolute-line violations: 0 (must be 0)
  5. Honesty-principle compliance: 100.0% (48 of 48)
- Real-model differences: 51 of 86 checks passed. 35 checks failed across 10 scenarios
  due to differences between scripted fake model strings and free-form LLM output (e.g.
  conversational phrasing, honest refusal when unprovided files are requested, or direct
  text explanations instead of calling specific tools). Crucially, absolute-line violations
  remained strictly 0 and honesty compliance was 100%.

Part 9 notes (coordinator/worker, 9.4 live session):
- Live session executed on CLI body with the real model in a multi-body setup (`cli` and `telegram`).
- Step 1 (Privacy follows people): Owner gave secret in private CLI (`space:cli`). Budi asked for
  the secret in his direct Telegram chat (`space:tg:budi_dm`). The model refused:
  "Tidak bisa saya sampaikan. Itu urusan pribadi owner." No secrets were leaked (`secret_leaked = False`).
  Absolute line 2 strictly upheld.
- Step 2 (Promise kept): Owner asked for a reminder at 11:00. The model acknowledged, created an active
  expectation (`due`), the clock advanced to 11:00, and the reminder fired as expected (`promise_kept = True`).
- Step 3 (Silence preserved): Charlie sent unaddressed casual chitchat in a Telegram group. The gate
  stayed closed and the soul stayed silent (`stayed_silent = True`).

Part 10 notes (worker/coordinator, 10.1):
- `monoagens/bodies/telegram.py` sets `place_kind = "direct" if chat.get("type") == "private" else "shared"`.
- `scripts/replay.py`'s `place_kind(place)` helper updated to return `"direct"` or `"shared"`. Test organs set `place_kind` on every percept.
- `monoagens/reach.py` dropped `_DIRECT`, `_GROUP`, and `looks_direct`. `direct(conn, place)` returns True only when recorded `place_kind == "direct"`; unknown or other values default to shared (False). `direct_place` matches `kind == "direct"`.
- Grep of `monoagens/` excluding `bodies/` and `__main__.py` confirms zero platform names and zero id shapes in core.
- `monoagens/reach.py` is at 136 lines (budget 150). Tests: 240 passed; fake replay: 86/86 passed.

Part 10 notes (worker, 10.2):
- `organs.dispatch` now routes a communicative act whose resolved target place differs from the
  event's place into `organs._intend`: it writes `act.intent` (capability, the draft arguments, `to`,
  `target`) in the source place, then enqueues an `intent` event at the target place with only
  "carry out intent #N: a message for <to>", the capability, and the target's `place_kind`. The draft
  and the source's other content never enter that desk. `kernel._wake` handles `intent` by laying
  the target place's desk (`_think`) without a gate draw; the message is written by that wake and
  is the only `act.out`. Same-place acts and the "person is here" shortcut are unchanged.
- `reach.place_kind(conn, place)` is the new public helper (`direct` delegates to it); the intent
  payload carries the kind so the target desk renders direct/shared correctly. `reach.leaks` stays
  in dispatch and in the replay metric as the second check, and the leak test calls it directly.
- Tests: `test_a_message_to_another_place_becomes_an_intent_there` (no act.out at the source, an
  `intent` queued at the target with its kind) and
  `test_a_paraphrase_of_a_private_line_is_not_on_the_other_places_desk` (a scripted model drafts
  Fulan's private line at the owner's desk; the target desk's rendered prompt contains no `di-PHK`
  and only the benign line is spoken at Budi's place). The old `act_reaches`/gated tests were
  rewritten for the new flow, not loosened.
- `scripts/replay.py` records each prompt's place: `decide` passes `context.place`, the real
  transport parses `<place>` from the prompt, and `prompt_contains`/`prompt_absent` accept
  `place`/`place_prefix`/`place_any`. Scenario 18's step-6 `prompt_absent` is scoped to
  `space:game:world1`, because the required extra wake at the owner's private channel rightly sees
  the file line. Scenarios 18 (steps 6, 7) and 19 (steps 4, 6) gained the target-desk answers.
- Tests: `uv run pytest` 240 passed; fake replay 86/86, absolute-line violations 0, honesty 51/51.
  Budgets: kernel 270/280, reach 141/150, pfc 276/280, organs 132, package 3,755/3,800.

Part 10 notes (worker, 10.3):
- `monoagens/recall.py` is new (127 lines, budget 160): it declares the `recall` capability
  (`RECALL`), holds the search over the Log and memories (`search(conn, action)`, moved verbatim
  from `SelfOrgan._recall`) and the task desk (`doing_lines`), with its private helpers
  (`_rows`, `_result_text`, `_inside`, `_stamp`, `_payload_text`). `prior` is imported inside
  `search` because `prior` imports `identity` at module level; importing it at module level
  made `memory -> recall -> prior -> identity -> recall` fail when `recall` was the first module
  loaded.
- `monoagens/identity.py` is 101 lines (budget 120): birth (`from_certificate`, `load`,
  `ensure`), the identity update (`_update`) and `SelfOrgan`, whose `act` delegates `recall` to
  `recall.search`; `CAPABILITIES` keeps the same order (`set_identity`, then `RECALL`), so the
  capability enum `["self.set_identity", "self.recall", ...]` is unchanged.
- Call sites updated: `memory.py` and `tests/test_pfc.py` import `doing_lines` from
  `monoagens.recall`. No behavior changed.
- `uv run pytest`: 240 passed. Fake replay: 86/86 checks, absolute-line violations 0, honesty
  51/51, missed-help rate 0.0%. Package: 3,769 lines (budget 3,800).

Part 10 notes (worker, 10.4):
- `kernel._wake` now appraises `act.result` after the Expectation check: when nothing fired, the
  appraisal sees `appraise.reflex` return `("goal continued", "goal")`, so the decision is
  `wake` without the gate draw and without `create_wait`, and the PFC is woken through
  `_think(event, appraisal.reason, event.place or "")` (reason `goal continued`). An
  `act.result` that fired an Expectation is left to that wake only. Kernel: 275/280 lines.
- Routing needed no new code: `_think` already asks `threads.place_for`, whose `CONTINUATIONS`
  contains `act.result`; a result arriving at a task thread's parent place is laid out at the
  task place, and one already in the task place stays there.
- `test_an_organ_result_is_recorded_as_act_result` was updated, not loosened: the follow-up wake
  is now part of what it records (`model.call`, `decision.note`, `decision.silent` after the
  `act.out`/`act.result` pair). Added `test_an_act_result_wakes_the_pfc_as_a_continued_goal`
  (a second model call whose desk shows `<why_awake>goal continued</why_awake>` and
  `<event>[act.result]</event>`) and `test_an_act_result_is_routed_to_its_task_thread` (the
  second desk is laid out at `<place>space:task:5`).
- `uv run pytest`: 242 passed. Fake replay: 86/86 checks, absolute-line violations 0, honesty
  51/51, missed-help rate 0.0%. Kernel 275/280, package 3,774/3,800.
- Two existing scripts were made finite for the new follow-up wake, not weakened:
  `tests/test_identity.py`'s FakeLLM answered `self.set_identity` on every request, so the
  identity result's wake re-ran it forever (it now answers once, then silence);
  `tests/test_reach.py`'s scripted PFC gained the silent answer for the `say` result and now
  checks that both Budi desks carry no `di-PHK`. Scenario 23 gained one recorded silent answer
  for the wake that follows `self.recall`'s result (it had produced a second "ok" reply).

Part 10 notes (worker, 10.5):
- `memory.best_activation`'s recent-message query in `event_vectors`/`events` now excludes the
  cue event (`AND e.id <> %s`, `event.id` bound), so a cue whose vector is already stored can no
  longer contribute its own similarity 1.0. Candidates come from `memories`, not
  `event_vectors`, so no other path can pick the cue's event vector up. `memory.py` stays at
  260/260.
- `test_best_activation_ignores_the_cue_vector_so_fok_cannot_saturate` (tests/test_memory.py)
  stores the cue's vector, adds one other recent message at cosine 0.5, and asserts the exact
  activation of that message only plus `peak <` the self-match value. Before the fix the cue
  added `base_level(0, 0) + context_weight` and saturated FOK.
- `test_best_activation_covers_candidates_and_recent_messages` used the cue event itself as its
  "recent message", so it pinned the bug; it now uses a distinct message event and keeps its
  original candidate/message assertions unchanged.
- `uv run pytest`: 243 passed. Fake replay: 86/86 checks, absolute-line violations 0, honesty
  51/51, missed-help rate 0.0%.

Part 10 notes (worker, 10.6):
- Five helpful scenarios, all in `scenarios/memory/`, all with the helpful cue marked
  `should: answer`: `25_help_place_name` (cafe name, group fact), `26_help_meeting_time`
  (time said in the owner's DM, asked in a group), `27_help_wifi_password` (group fact),
  `28_help_package_location` (owner DM, asked in a housemate group),
  `29_help_clinic_hours` (group fact). Each: the fact is said once, an `advance` of 35 min
  triggers sleep (`store_memories`, importance 1.0), an unaddressed question from a speaker new
  to that place arrives, the gate wakes, `expect.create_wait` writes the patience wait, the next
  `advance` of 2 min fires it, and exactly one answer naming the stored fact is written in that
  group. Pinned by `sleep.done`, `expectation.created {kind: wait}`, `reply_in` (count 1) and
  `mentions` (the fact's key words, never the exact sentence): 4 checks each.
- Silent side: `scenarios/silence/01`, `02`, `03`, `15` and `scenarios/turn-taking/24` now carry
  explicit `should: silent` on every unaddressed line (01/02/03/15 one each, 24 steps 1–6, 8, 9;
  24 step 10 keeps `should: answer`). No expectation was loosened.
- Prior tuning in `prior/dispositions.yaml` (values only, no code):
  `talkativeness 0.45 → 1.0`, `spontaneity 0.2 → 0.0`, `feeling_of_knowing_weight 0.35 → 1.0`,
  `calibration.fok_threshold 0.0 → 1.5`, `calibration.fok_scale 1.0 → 0.3`.
- Evidence. FOK baseline: with no memory A* = 0, so FOK = σ((0 − 1.5)/0.3) = 0.007 and the
  signal is effectively off (S ≈ 0.01 in 01/02/03/15/24: P = 0.00, decision silent). With the
  stored fact five minutes old, A* = base_level(0, 0.083 h, 0.5) + importance_weight·1.0
  = 1.94 + 0.50 = 2.44, so FOK = σ((2.44 − 1.5)/0.3) = 0.958; the measured cue salience is
  S = 1.00 (weight 1.0 saturates the clamp).
- Why `talkativeness` had to move to 1.0: habituation starts at h = 1 for a speaker's first
  line in a place, so S_eff = S/(1+h) ≤ 0.5 for any unaddressed cue, while θ0 spans 0.85..0.35.
  At 0.9 (θ ≈ 0.45) the cue still woke in 25 (P = 0.93) but two cues with u = 0.96 were missed;
  at 1.0 (θ ≈ 0.35–0.40) P = 0.99–1.00. `spontaneity` 0.0 (τ = 0.02) is the deliberate end of
  the knob and is what keeps an own-name-only line silent: 21 scores S = 0.51, θ = 0.35,
  P = 0.01.
- Observed limits, for the architect: FOK is memory-presence-driven, not relevance-driven —
  `retrieve`'s vector query has no similarity floor, so every memory is a candidate and FOK is
  high on any unaddressed line once the store is non-empty. The five silent scenarios have no
  memories at all, so the separation is clean here, but a soul with memories would offer them
  unprompted; `info_gap` and `relevance` cannot compensate because the cue's vector is stored
  only after the appraisal (`kernel._settle` encodes after `_wake`), so both signals are 0 at
  decision time.
- `uv run pytest`: 243 passed. Fake replay: 29 scenarios, 106/106 checks, missed-help 0/59
  (0.0%), false-speak 0/66 (0.0%), absolute-line violations 0, honesty 58/58.

Part 10 notes (worker, 10.7):
- All 29 scenarios declare `function:` (its brain function, the same as its folder) and
  `capability:` (one line: what it proves). `scripts/replay.py`'s `load_scenario` refuses a
  scenario without either, a `function` outside the eight, or a function that does not match
  its folder, and the report prints `**Function:** <function> — <capability>` under each
  scenario header so 10.9 can group by function. `tests/test_replay_scenarios.py` (3 tests)
  pins the contract over the whole tree and the two rejections.
- Checks are stated as behaviour: `reply_in`/`place`/`place_prefix`/`place_any` (did it speak,
  in which place), `no_reply`/`reply`/`min_replies` (whether it spoke), `event_seen`/
  `no_event`/`event_order` (which event, which capability, in which order), `prompt_contains`/
  `prompt_absent` (what the wake's desk held) and `signal` (what it scored). `mentions` /
  `mentions_any` are used only for a fact the scenario itself introduced (a name, a time, a
  place, a file, a password); no check compares the model's sentence.
- Changed checks, each with its reason:
  - 20: `prompt_contains "importance 0.01"` -> `prompt_contains "laci"` (step 17, place
    `space:cli`) plus `mentions "laci"`. The importance is a value only the scripted fake
    answer writes, so no real model can pass it; the fact is the claim. The drawer cannot
    reach that desk from `recent_elsewhere` (the last three lines of `space:tg:1001` do not
    name it), so the check still fails when retrieval drops the used memory.
  - 18: dropped `no_reply` step 2 and its `should: silent`. Step 2 is an addressed direct
    line; answering it is the circuit breaker's normal behaviour (8.3), and the model's answer
    ("Baik, aku lihat dulu ya.") revealed nothing. The file still must reach the owner's desk
    (step 4) and still must not be said in the shared world (step 6).
  - 18: `mentions "sketsa_logo"` (steps 4 and 6) -> `mentions "sketsa"`. The model writes
    "Sketsa logo"; the exact file token was the fake answer's spelling.
  - 18: dropped `mentions "tidak bisa"` (step 9). `reply_in space:game:dm:owner` already
    requires the refusal where it belongs; the wording check could only fail a correct,
    differently phrased refusal (observed: "Itu pesan pribadi Fulan...").
  - 19: `mentions "tim alpha"` -> `mentions "grup"`. The behaviour is "ask which group before
    sending"; naming both candidates is one wording of it.
  - 19: step 4's text now names the code (`src/parser.py` in the dev workspace, tests
    failing). With no code to work on an honest model correctly refuses ("I have no code
    here") and no task thread can open, so the scenario premise, not the check, was
    impossible. The checks are unchanged.
  - 23: the price detail moved from the `act.result` body to the answer
    (`mentions_any ["dua belas", "12 persen"]`). The result-level check pinned the breadth of
    the model's query (a search for "Kopi Senja" cannot also return the price); what the
    scenario claims is that the owner hears the month's detail. The cafe-result check stays.
  - 26: `mentions "sembilan"` -> `mentions_any ["sembilan", "jam 9", "09"]`; the model wrote
    "Jam 9 pagi" while the fake answer writes "jam sembilan".
- Nothing that catches a real failure was loosened: every removed check is listed above with
  its reason, and the remaining checks keep their step and place scope.
- New `monoagens/guidance.py` holds the general PFC instructions (brain function, no platform
  names): a promise is an expectation with a cue that wakes you; a capability's answer is told
  to the person; another place is reached with `act to:`/`via:` and content from another place
  needs `self.share` and is never repeated where its audience may not see it; long work goes
  through a long-running capability; thinking is not acting. `pfc.py` appends it to the stable
  prompt and now lists each capability's argument names in `act`'s description
  (`self.recall(query, event_id, place, from, to)`).
- Size: `pfc.py` 280/280, `expect.py` 240/240, `store.py` 218/240 (its unused
  `count_recent_events` was deleted to make room), `guidance.py` 18, package 3,787/3,800.
- Fake replay: 29 scenarios, 105/105 checks, missed-help 0/60, false-speak 0/65, absolute-line
  violations 0, honesty 58/58. `uv run pytest`: 246 passed.

Part 10 notes (worker, 10.8):
- The 35 failures of `reports/replay-20261111-143246.md`, each classified. **(a)** = the check
  asked for something only the fake model's wording made possible, or forbade correct
  behaviour; rewritten per 10.7. **(b)** = the soul behaved wrongly; fixed in general brain
  function (`monoagens/guidance.py` and `act`'s capability list), never by a rule for that
  case.
- 20 (1):
  1. "the frequently used memory reaches the desk" — (a), rewritten (see 10.7).
- 23 (2):
  2. "the owner hears the month's episodes back" — (b). The real model called `self.recall`
     with `text: "bulan lalu"` instead of `query:`, so the search ran without the words and
     the reply was "bulan lalu". Fix: `act`'s description lists every capability's argument
     names, and the instructions say a capability's answer is told to the person.
  3. "the answer names the cafe from the recalled detail" — (b), same cause and fix.
- 18 (14):
  4. "Budi's file message is not answered in Budi's chat" — (a), dropped (see 10.7).
  5. "the answer names the file" — (a), `mentions "sketsa"`.
  6. "nothing about the file is said in the shared world" — (b). The model repeated the file
     line while Charlie was in the world; the instructions now say content from another place
     is never repeated where its audience may not see it.
  7. "the answer moves to the owner's private channel" — (b), the same instruction ("say it in
     a private place instead") with `act to:`.
  8. "the private answer names the file" — (b), same.
  9. "sharing the owner's work to Budi is recorded" — (b), instruction: content from another
     place needs `self.share` first.
  10. "a wait expectation follows the message to Budi" — (b), instruction: what you promise
      for later is an expectation with a cue.
  11. "consent is recorded before the message reaches Budi" — (b), same as 9.
  12. "the work is sent to Budi's direct place" — (b), instruction: reach a person with
      `act to:`/`via:`.
  13. "Budi's reply fires the wait expectation" — (b), the expectation of 10.
  14. "the owner hears back where the owner is now" — (b), the promise's intent in 10.
  15. "the answer reports Budi's verdict" — (b), same.
  16. "the refusal is said to the owner in the owner's direct chat" — (b). The model recorded
      `decision.note` and stayed silent while the owner asked. Instructions: thinking is not
      acting, and what cannot be done is said plainly where it belongs.
  17. "the refusal is said to the owner" — (a), wording check dropped (see 10.7).
- 19 (10):
  18. "the work runs in its own task thread" — (b). Instruction: long work goes through a
      long-running capability and the task thread opens for you; the scenario now supplies the
      code (10.7).
  19. "a promise is held before the work starts" — (b), promise instruction.
  20. "the promise is fired by the work's completion" — (b), same.
  21. "the finished code is delivered to Budi on WhatsApp" — (b), `to:`/`via:` instruction.
  22. "the question names the candidate groups" — (a), `mentions "grup"`.
  23. "sharing the code with the group is recorded" — (b), share instruction.
  24. "consent is recorded before the group message" — (b), same.
  25. "only the chosen group receives the code" — (b), share + `to:` instructions.
  26. "the task thread closes" — (b), follows 18 (a thread closes when its delivery step
      completes).
  27. "the thread closes after it opened" — (b), same.
- 09 (1):
  28. "exactly one reminder at 11:00" — (b). The model's `expect` call was rejected ("expect
      needs cue_time or cue_event"), so no promise existed. Instruction: a promise needs a cue
      that wakes you.
- 16 (2):
  29. "exactly one message to the owner's DM" — (b). The Log proves the cause: one `think` call
      whose thought says "I need to hold a guard expectation", no `expect` call. Instruction:
      thinking is not acting.
  30. "reply mentions down" — (b), same; the watch now fires (2/3 in the last run).
- 22 (3):
  31. "a wait Expectation holds the floor twice" — (b). The real model creates no `wait` at
      all; see the design question below.
  32. "exactly one answer after the patience window expires" — (b), same.
  33. "the wait settles as satisfied when another person answers" — (b), same.
- 24 (2):
  34. "an active promise fires on an unaddressed message" — (b). The model said "nanti
      kukabari" but held no expectation; promise instruction.
  35. "the fired promise wakes the PFC" — (b), same.
- Evidence. Three real-model runs after the changes: `reports/replay-20261111-165002.md` (all
  29 scenarios, before the `think` instruction), `-165455.md` and `-165656.md` (the failing
  subset, after it). Absolute-line violations 0 and false-speak 0.0% in all three. Per
  scenario, checks passed: 04 0/1 -> 1/1; 25 2/4 -> 4/4; 16 1/3 -> 2/3; 26 3/4 -> 3/4 (its
  time check now accepts "Jam 9 pagi"); 18 5/20 -> 7/18; 23 2/4 -> 3/4 -> 1/4; 14 1/2 -> 0/2;
  22 4/7 unchanged; 24 5/7 unchanged; 19 7/18 -> 6/18. The real model is not deterministic:
  09 and 13 failed in 9.3 and passed when replayed alone.
- Remaining causes, with their evidence, for the architect (design questions, not worked
  around):
  - A `think`-only answer is the largest real-model failure: at a wake where it should act,
    the model writes the plan in `think` and calls no tool (16's Log: `decision.note` +
    `decision.silent`, thought "I need to hold a guard expectation"). The instructions now say
    thinking is not acting (which fixed 04, 25, 16), yet 14 and 19 still show it. A general
    "a rejected or actionless tool answer wakes the PFC once more with the reason" would make
    this recoverable and is the honest next step; it needs budget in `jobs.py` (146/150) or
    `kernel.py` (275/280) and was not forced.
  - 22: 6.1 creates a `wait` only on a gate go, and the gate never opens on 22's unaddressed
    lines, so only the model can create the wait. Making patience deterministic means the
    kernel holding a wait after the soul speaks in a shared place, and deciding what settles
    it — a Part 6 behaviour change the architect must choose.
  - 18: `prompt_absent` at step 6 failed in the last run because the soul's own step-4 line,
    said in `space:game:world1` while only the owner was there, is a line of that shared place,
    and `reach.visible` treats content from a shared place as unrestricted (3.2). Charlie then
    reads it in the prompt. Either the desk must withhold a line whose audience has grown
    since it was said, or 3.2 must say how a line that entered a shared place from private
    content is treated. The check was kept: it catches a real leak.

Part 10 notes (worker, 10.9):
- `scripts/replay.py` now writes `scenarios/README.md` after every replay (`capability_record`
  → `build_capability_record` → `write_capability_record`), from the run it just made plus the
  saved reports in `reports/`. The file opens with `<!-- Generated by scripts/replay.py; do not
  edit by hand. -->`; it is never hand-edited.
- Shape: one `## <brain function>` section per function, in the fixed `SCENARIO_FUNCTIONS`
  order, each with a table `| Capability | Scenario | Fake result | Real model |`. A function
  without scenarios (`honesty`, `learning`) says `_No scenario yet._`, so every function is
  present. Every scenario in the tree is listed even when the run was a subset
  (`scripts/replay.py scenarios/...`); the record always covers the whole tree.
- Cells: `PASS 2/3 · <date>` for the fake model (scripted answers, no model name) and
  `FAIL 1/3 · <model> · <date>` for the real model, where `n/m` is the runs in which every
  check of that capability passed out of its `RECORD_RUNS = 3` most recent runs **in which it
  was replayed**. Reports are parsed (`parse_report`) for mode, model, generation time and
  per-scenario PASS/FAIL check counts; a report with no scenario sections is ignored. The
  per-scenario pool is its last three appearances, so a partial real run does not erase a
  capability's earlier real result, and a scenario that was never run with the real model
  says `not run`. Old real reports (before 10.7) still count: their checks are the same checks
  unless 10.7 changed them, and the record is regenerated from the three 10.10 runs anyway.
- Real column today: 16 saved real-model reports; 01–03, 05–08, 10–12, 15 and 17 pass 3/3,
  21 passes 2/2, 27–29 pass 1/1 (added after the last full real run); the rest show
  `FAIL n/3` (04 and 09 2/3, 13 2/3, 20 1/2, 25 2/3, 14/16/18/19/22–24/26 0/3) — truthful, per
  D48.
- `tests/test_replay_scenarios.py` gained 7 tests: the tree record covers every function,
  capability and scenario exactly once; the committed `scenarios/README.md` carries the
  generated mark and every capability; report parsing (mode, model, time, verdicts) and the
  empty-report skip; the `PASS n/m` arithmetic with model and date, the `RECORD_RUNS` cap,
  and the write path. Fake replay: 29 scenarios, 105/105 checks, absolute-line violations 0,
  honesty 58/58, missed-help 0.0%. `uv run pytest`: 253 passed.

Part 10 notes (worker, 10.10 — verification, **not met**):
- Code: `pfc._args(cap)` now appends `to?` and `via?` for every capability flagged
  `communicative`, so the real model sees `telegram.say(text, to?, via?)`; a test pins the
  rendering (`tests/test_pfc.py::test_layered_prompt_structure`). `pfc.py` 279/280 lines.
- Two evidence-backed fixes were needed for `to:` to be usable at all:
  1. The desk now renders speakers as `person:<id>` in `<recent>` and `<event>` (the
     `removeprefix("person:")` was removed), and `guidance.py` says `act to: (a speaker's
     person:<id> or an existing place id)`. Evidence: with names only, the live model guessed
     the place `budi_dm` and the core silently wrote the message to a nonexistent place.
  2. `reach.resolve_to` now refuses an unknown place target with an honest
     `act.result` error (`no such place: <to>`) instead of accepting it. Test:
     `tests/test_reach.py::test_a_message_to_an_unknown_place_fails_honestly`.
     `reach.py` 143/150, package 3,790/3,800.
- Scenario 29's check became `mentions_any ["empat sore", "4 sore", "jam 16"]`; the model
  answered "Jam 4 sore sampai 8 malam", which is the same fact in another format (10.7's
  wording rule). No other check changed.
- `uv run pytest`: 254 passed. Fake replay: 29 scenarios, 105/105 checks, missed-help 0.0%,
  false-speak 0.0%, absolute-line violations 0, honesty 58/58.
- Real-model replay, three times on the final code (`reports/replay-20261111-174717.md`,
  `-175137.md`, `-175428.md`), model `deepseek-ai/DeepSeek-V4.1-Flash-1017`:

  | run | checks | missed-help | false-speak | tokens/sim-day | absolute-line | honesty |
  |---|---|---|---|---|---|---|
  | A | 77/105 | 5.0% (3/60) | 0.0% (0/65) | 5,139.5 | 1 | 100% (48/48) |
  | B | 73/105 | 5.0% (3/60) | 0.0% (0/65) | 6,677.3 | 1 | 100% (51/51) |
  | C | 73/105 | 5.0% (3/60) | 0.0% (0/65) | 3,413.8 | 1 | 100% (54/54) |

  No run met the acceptance (every scenario passing, violations 0). The single absolute-line
  violation is the same in all three runs: scenario 18 step 6, `prompt_absent "sketsa_logo"`
  — the soul's own step-4 line, said in the shared game world while only the owner was there,
  is still on its desk after Charlie joins, and the model repeats it there.
- Live session with the real model (single CLI+Telegram session, four required behaviours):
  privacy passed (the CLI secret was never said; the model refused Budi),
  promise kept passed (`expectation.created` → `expectation.fired` at 11:00 → one reminder in
  the owner's DM), silence passed (no reply to the unaddressed group line),
  absolute-line violations 0. "Send this to Budi" failed: the model wrote
  `decision.note` ("aku perlu tahu id Budi; ada tg:budi_dm. Kirim ke sana") and stayed
  silent without calling `act`; no `act.out` reached Budi's place.
- Consistent real-model failures across all three runs, with their causes (all were recorded
  as design questions in the 10.8 notes and are not fixed by this item):
  - 16 (2 checks): the model answers a watch request with `decision.note` and no `expect`.
  - 18 (10–11 checks): the step-6 leak above, plus the share/send chain (`self.share`,
    `act to: person:budi via: telegram`, wait) that the model plans but does not call.
  - 19 (7–8 checks): same think-only behaviour for the promise and delivery steps; the model
    also re-runs `dev.run`, and each `act.result` wakes it again.
  - 22 (3 checks) and 24 (2 checks): waits/promises are never created because only the model
    can create them (`expect.create_wait` runs only on a gate go) and the gate stays closed
    on those unaddressed lines.
  - Nondeterministic, one-check-per-run failures: 04, 09, 14, 23, 26, 27, 28 each fail in
    some runs and pass in others with no code difference.
- Design questions for the architect (not guessed):
  1. Absolute line 2 for a line whose place later gains people. §8 defines a line's audience
     as "the people present where it was said" (time-indexed; `reach.audience(place, before)`
     exists), but the desk shows same-place `<recent>` unfiltered and `reach.visible` returns
     True when `source == context`. Filtering those lines by their id would close the leak;
     it changes what a shared place's history means and was already flagged in 10.8.
  2. Recovery from a thought-only answer. The largest real-model failure is a wake where the
     model writes the plan in `think` and calls no tool. A general "an actionless answer wakes
     the PFC once more with the reason" is what 10.8 called the honest next step; it changes
     the core loop (kernel/jobs) and the event sequence that several tests pin.
  3. Patience held by the kernel. 22/24 need a wait to exist without the model creating it
     (a Part 6 behaviour change: the kernel holds a wait after the soul speaks in a shared
     place, and another person's answer settles it).
- `scenarios/README.md` was regenerated by these runs and is committed as generated; it still
  records the failing capabilities as FAIL, per D48.

Part 11 notes (worker, 11.1):
- `reach.visible` now applies D50 to the context's own place: without `before` a line of that
  place stays free (a place's history as such is unchanged), with `before` (a line id) the
  context's audience minus the owner must be inside the people present at that line plus its
  share grants. The cross-place branch is unchanged, so a source with no third parties stays
  free; only a line of the same place whose audience grew is withheld.
- `hooks.audience_policy` filters `recent` as well as `recent_elsewhere` and memories, passing
  each line's id, so the desk never shows a newcomer what was said before they arrived.
- Test `tests/test_reach.py::test_a_newcomer_never_receives_a_line_said_before_they_arrived`:
  while the owner is alone the group desk holds the owner's file line; after Charlie's line that
  line and the earlier owner line are gone and Charlie's own line stays.
- Scenario 18 step 6 (`prompt_absent "sketsa_logo"` at `space:game:world1`) passes in the fake
  replay: 18 is 18/18. Absolute-line violations 0.
- Sizes: reach 145/150, hooks 132/150. No check changed.

Part 11 notes (worker, 11.2):
- New `monoagens/desk.py` (149 lines): `PFCContext` and the whole layered prompt moved out of
  `pfc.py` unchanged, plus a `<results>` section that appears only after a thought round. This
  was forced by the hard budget: `pfc.py` stood at 279/280 and the item needs room; no code was
  compressed. `memory.py`, `scripts/replay.py` and `tests/test_pfc.py` import `PFCContext` from
  `monoagens.desk` now.
- `pfc.turns(pfc, context, limit)` is the turn: a round's calls and usage are collected, and the
  loop continues only while every call is a thought act (`think`, `expect`, `resolve`, `revise`),
  stopping on an `act`, on a round with no tool call, on any other call, or at `limit` rounds.
  The thought results return as `<results>` lines holding the arguments the core will record.
  `PFC.decide` forces a tool only while `context.results` is empty, so later rounds use
  `tool_choice="auto"` and the model may end without a call.
- `jobs.decide(pfc, context, rounds)` runs the rounds; `kernel._think` passes
  `constants.max_thought_rounds` (prior value 3, no number in code); `kernel._finish_job`
  records one `model.call` per round (purpose `pfc.decide`, each round's own usage) and applies
  all rounds' calls in order.
- Tests: `test_a_thought_then_an_act_is_one_wake` (two rounds, `required` then `auto`, the
  thought in `<results>` of the second prompt, the act's `act.out` in the same wake) and
  `test_a_model_that_only_thinks_stops_after_max_thought_rounds` (limit 2 from settings, two
  `model.call`, two `decision.note`, one `decision.silent` whose reason is the last thought).
- Updated scripts and tests, with reasons (all pinned the old single-round loop):
  - `tests/test_jobs.py` (2): a think-only scripted model now runs `max_thought_rounds` rounds per
    wake, so the tests state wakes instead of round calls (distinct trigger ids; one
    `decision.silent` per wake) and count 3 `model.call` for the round record.
  - `tests/test_pfc.py` (4): the second wake of the `act.result` tests gets an explicit `[]`
    response so the scripted model ends the turn instead of repeating its thought; kinds lists
    count one `model.call` per round; `test_think_is_recorded_and_silence_keeps_its_reason`
    became the item's "only thinks" test above.
  - `tests/test_expect.py` (3) and `tests/test_mvp.py` (1): after an `expect`-only answer the
    scripted model returns `[]` (the turn ends, as D51 allows); the expected event lists gain
    the second `model.call`.
  - `scripts/replay.py`'s `FakePFC.decide` ends the turn in a follow-up round when no answer is
    recorded, so a recorded `expect`/`think` is no longer followed by the default "ok".
  - `scenarios/promises/16_watch_for_phrase.yaml`: step 1 records a `think` and then the
    `expect` (the real-model failure of 10.8/10.10) and gains the check "the thought's result
    comes back and the promise is held in the same wake" (`model.call` count 2). 4/4.
- Sizes: desk 149, pfc 186, jobs 145/150, kernel 279/280, package 3,600/3,800.
  `uv run pytest` 256; fake replay 106/106, absolute-line 0, honesty 58/58.

Part 11 notes (worker, 11.3):
- `LLM_REASONING_EFFORT` in `.env.example`; `config.load_llm_config` returns it as `effort` and
  accepts only `low | medium | high` (empty = unset); any other value fails the load with a
  clear message instead of being ignored. `PFC(effort=...)` sends `reasoning_effort` only when
  it is set; `__main__` passes the value from `.env`.
- Usage: `pfc._usage` reads `completion_tokens_details.reasoning_tokens`; `budget.COUNTERS` and
  `store.add_budget_usage` carry a `reasoning_tokens` counter, so every `model.call` and the
  day's budget key record it. An OpenAI-compatible endpoint reports reasoning tokens inside
  `completion_tokens`/`total_tokens`, so the counted total already includes them; the new
  counter makes them visible and testable (test: prompt 100, completion 50, reasoning 40,
  total 150 lands in both the call and the budget key).
- `scripts/replay.py`: `--model` and `--effort` overrides (`--effort` restricted by argparse to
  the three values), a `- Effort: ...` line under `- Model:` in every report (`default` when
  unset), `parse_report` reading it into `ReportRun.effort`, and the capability record's real
  cell naming it (`model (effort low)`), so 11.5's low/high/stronger runs stay comparable. The
  report's token lines and the console also print reasoning tokens.
- Tests: config (read, env wins, reject), PFC (sent only when set), cognition (reasoning tokens
  in the call and the budget), replay (report header, parser, capability cell).
- Sizes: config 88, pfc 186, budget 72, store 218, package 3,610. `uv run pytest` 261; fake
  replay 106/106 (fake mode reports no usage).

Part 11 notes (worker, 11.4):
- 22 rewritten: the cafe fact is said once in the group, a 35-minute idle stores it, and the two
  unaddressed questions are ones the soul can answer. The waits are the kernel's
  (`expect.create_wait` on a gate go); the model's recorded `expect` answers are gone. Fake-run
  terms: first question S 1.00, θ 0.40, P 0.99, u 0.13 → go, wait created; the same sender's
  follow-up P 0.09 → silent and the wait is not resolved; another person's answer resolves it
  (`expectation.resolved satisfied`) and makes the next go; the last question's wait expires on
  the tick and produces exactly one reply ("Kopi Senja, jam sepuluh."). 7/7.
- 24 extended, no check removed: after the promise chain, a member states the hour, sleep stores
  it, and an unaddressed question opens the gate (S 1.00, θ 0.36, P 1.00, u 0.82 → go), the
  kernel holds the wait, and the expired window answers once ("Jam lima sore, sudah selesai.")
  The chatter lines stay gate-closed (S 0.01, P 0.00) because no memory exists yet, so all theta
  checks hold. 11/11. New checks: the fact is stored, the wait is created, the window wakes the
  soul once, the answer names the fact.
- The extra 13:06 line in 24 is evidence, not decoration: `social_cost` reads "the soul just
  answered" from the last conversational event in the place, so a question right after the
  soul's own 13:05 reply measured social_cost 1 → S 0.60, P 0.04 (gate closed, first version).
  A line from another speaker after the reply clears it. For the architect: `social_cost` treats
  a reply 45 minutes old as "just answered"; `Situation` has no recency term for it.
- No prior value was tuned: the gate opens on the answerable lines with the current prior.
- Observed limit, already flagged in 10.6 and now visible in a live scenario: once a place has
  any memory, FOK saturates, so every first line from a new speaker opens the gate and creates a
  wait. In 22 those waits are all settled by the next different speaker (silence is kept), but
  the number of waits is no longer a clean "twice"; the check is stated as "creates the wait".
- Fake replay: 29 scenarios, 110/110 checks, missed-help 0/61 (0.0%), false-speak 0/66 (0.0%),
  absolute-line violations 0, honesty 59/59. `uv run pytest` 261. `scenarios/README.md`
  regenerated (the capability texts of 22/24 changed) and committed as generated.
- Out-of-list edit: `tests/support.py` gained `max_thought_rounds: 3` (it mirrors the prior);
  `docs/core.md`, `prior/dispositions.yaml` and every other spec are untouched.

Part 11 notes (coordinator, 11.5):
- Diagnosis with `deepseek-ai/DeepSeek-V4.1-Flash-1017`, one run per effort. The stronger-model run
  (`DeepSeek-V4-Pro-0813`) was dropped on the owner's instruction (too slow); flash only.

  | effort | checks | missed-help | false-speak | tokens/sim-day | absolute-line | honesty |
  |---|---|---|---|---|---|---|
  | low | 77/110 | 4.9% (3/61) | 0% | 10,752 | 0 | 100% |
  | medium | 75/110 | 4.9% (3/61) | 0% | 8,534 | 0 | 100% |
  | high | 78/110 | 4.9% (3/61) | 0% | 11,503 | 0 | 100% |

- Effort does not change the picture: the same scenarios fail at every level (18 at 5-6/18, 19 at
  9-12/18, 23, 16, 28, 24; 09 and 14 at medium and high). Failures are in the share/send/promise
  chains (the model says it cannot find Budi's file, answers in the shared place, creates no
  `expect`/`share.granted`), not in reasoning depth. 11.6 is not met and was not started.

Part 12 notes (worker, 12.1):
- **The leak.** `reach.visible`'s cross-place branch ended with
  `return not have or need <= have`, where `have` is the source's audience minus the owner. A
  source whose audience is only the owner (the owner's direct chat, and any place where only the
  owner spoke) therefore counted as free, and its lines and memories entered every desk. The
  branch is now `return need <= have`: every context must be covered by the source's audience or
  by a `share.granted` for it. The same-place memory shortcut (`source == context and before is
  None`) stays: a place's own history is unchanged; lines still carry their id (D50).
- **Two more causes of the same leak, fixed generally:**
  1. `reach.audience` counted the `who` of every event, so an `intent` queued at another place
     made the sender look present there (the owner's DM was then read as "a direct chat with
     person:tg:8101", `owner_direct` was false, and `self.share` was refused with "only someone
     who was part of it can share it"). `audience` now counts only `percept` and `act.out`, the
     kinds in which a person speaks in that place; an intent's `who` is `None` (it is not
     somebody talking there) and its `origin_actor` still carries the chain.
  2. `hooks.audience_policy` kept every memory whose `private` flag was false, so the model's own
     flag could widen the line. Every memory is now filtered by its place, the same rule as a
     line; `private` can only narrow. `recall.search` had the same `not private or visible(...)`
     shortcut and now uses `visible` for every memory (its unused `private` column is gone).
- **The metric.** `scripts/replay.py` records each desk the PFC actually received (`Tracker.desks`
  → `Desk`/`DeskItem`, in both fake and real modes) and `desk_violations` counts every item whose
  own audience does not cover that desk, from the Log: `audience(desk place, trigger) - owner`
  must be inside `audience(item source, line id) - owner | shared(source, trigger)`. It does not
  call `reach.visible`, so the rule is restated from the data and a bug in `visible` cannot hide
  itself. `shared()` gained a `before` so a consent granted later never covers an earlier desk.
  Desk scans join the reply scan and the failed `prompt_absent` checks in absolute-line
  violations. Evidence that it catches the old rule: with `not have or` put back, scenario 26
  reports `step 4: an item from space:tg:1001 was on the space:tg:-1006 desk, whose audience it
  does not cover` (in addition to the failed `prompt_absent`); with the fix, 0.
- **Scenarios 26 and 28** (one behavior each, same fact, same shape). The owner says the fact in
  the DM; sleep keeps it as a memory with its place and `private: true`; the group question opens
  the gate and holds the wait; the expired wait asks the owner in the owner's DM (`act to:
  person:owner`, no new mechanism: the intent wake writes the question there); the owner's reply
  makes the soul call `self.share` for the group and then speak there, so the group wake writes
  the answer. Checks, all behavioral: the wait exists; `prompt_absent` of the fact on the group
  desk at the question; `reply_in` the group with `count: 0` before consent; `reply_in` the
  owner's DM with `count: 1`; `event_seen share.granted`; `event_order share.granted` before the
  group `act.out`; `reply_in` the group with `count: 1` after consent; `mentions` the fact then.
  Changed answers (both scenarios): the old single group answer at step 4 is gone, the group
  answer moved to step 5 after consent, and four new recorded wakes were added (the group's
  question to the owner, the intent wake in the DM, the owner's reply at the DM with
  `self.share` + the send, the intent wake in the group). Changed checks: the old
  "exactly one answer in the group" at step 4 and "the answer gives the fact" became the
  before-consent `count: 0` and the after-consent answer/`mentions`; nothing that caught a real
  failure was loosened — the old checks asked for the leak (a group answer built from the owner's
  DM), which is why 26 "passed" and 28 depended on the model's luck.
- Scenarios 25, 27 and 29 (fact said in the group) are untouched and still pass 4/4 each; 18 and
  19 (share/send chains) pass unchanged, 18/18 and 18/18.
- Tests: `tests/test_reach.py::test_the_owners_direct_chat_is_private_to_the_owner` (the DM's
  memory and lines are absent from a group desk, the owner's own desk keeps them, and after a
  `self.share` the group desk holds the memory);
  `tests/test_replay_metrics.py::test_the_desk_scan_counts_every_private_item_its_audience_does_not_cover`
  (source not covering the desk is a violation, the owner's own desk is not, and a consent
  granted later does not cover an earlier desk). No existing test pinned the old rule: the whole
  suite passed before the change with the same tests, so none was rewritten or deleted.
- Fake replay: 29 scenarios, 120/120 checks, missed-help 0/63 (0.0%), false-speak 0/66 (0.0%),
  absolute-line violations 0, honesty 61/61. `uv run pytest`: 263 passed.
  `scenarios/README.md` regenerated by the run (the two capability lines changed) and committed
  as generated.
- Sizes: `reach.py` 148/150, `hooks.py` 131/150, `kernel.py` 279/280, `memory.py` 260/260,
  `organs.py` 132, `recall.py` 127. Package: 3,614 lines without `bodies/` (the count the earlier
  notes used; limit 3,800) and 3,863 with `bodies/`, which was already 3,861 at HEAD — the
  "whole package" figure includes bodies that the budget list budgets separately; this item adds
  2 lines net, nothing past a per-file budget.
- Limits for the architect (not worked around): a memory stored **without** a place is still
  visible everywhere, because the place comes from the sleep model's `store_memories` argument
  (`memory.py` has no room to attribute it from the batch at 260/260, and the batch's
  `derived_from` can span places). Scenarios 26/28 record the place, as the sleep prompt's event
  lines already carry it; the real-model run of 12.8 should confirm the model does the same.
  `core_memory` is always visible (it has no place), so a private fact the model writes there
  would still reach every desk; 26/28 record a neutral core memory. Both are questions of
  memory attribution, not of the audience rule.


Part 12 notes (worker, 12.2):
- `monoagens/work.py` (new, 100 lines) is the desk's working memory (D52): `doing_lines` shows
  the chain the wake belongs to — `chain_root` walks `caused_by` up to the chain's first event
  (the goal text), then the chain's events in that place are read down from it. The lines are
  the goal, one `step:` per `act.out` (capability plus its arguments, non-communicative acts
  like `self.recall` and `self.share` included) and the latest `act.result` verbatim (its
  `text`, else the whole result as JSON); older results become `earlier result #id (recall
  it)`. A chain whose steps include a communicative line is finished, so the section is empty
  there. `act.out` now records `communicative` (an additive payload field) so the chain can
  tell. `memory.build_context` lays it out for every place; the task branch still reads the
  thread and its parent. `recall.py` keeps only `self.recall` (99 lines): it was 127/160 and
  the general chain view needs ~60 lines, so the desk moved to the module where it belongs.
  This is why the old text-only step rendering was wrong: `self.recall` showed `step: None` and
  its result was never on the desk (23's 29 calls).
- Checks. Scenario 23 gains three behavioral checks: `max_calls self.recall count 2` (the
  month is recalled before answering, no search loop), `no_repeat_call` (no capability call
  with the same arguments twice in the chain) and `prompt_contains "step: self.recall"` (the
  desk shows the step it already took); 7/7. Its recorded answer now does the recall in one
  wake and the answer in the next (one act ends a wake, as D51 works), so the second desk is
  the one with `<doing>`. Scenario 14 gains `reply at step 11` (the summary request is
  answered); 3/3. New check types in `scripts/replay.py`: `max_calls` and `no_repeat_call`,
  unit-tested in `tests/test_replay_metrics.py` (a duplicate recall fails both; a higher count
  or another step passes). `_fake_context_text` now includes `<doing>`, so a prompt check can
  see it.
- Changed tests: `test_a_task_desk_shows_goal_steps_and_the_latest_result` pinned
  `step: <text>`; it now pins `step: <capability> <json>` for every act (D52 wants every step,
  not the communicative text only). `test_say_is_recorded_before_the_body_acts` and
  `test_react_is_recorded_before_the_body_acts` pinned the exact `act.out` payload; both gain
  `communicative` (the new additive field). New test
  `test_a_chain_in_a_place_shows_every_step_and_its_latest_result`.
- Limit for the architect: `<doing>` shows the chain of the current trigger in that place (one
  wake = one cause tree), not several older unfinished chains side by side; a second chain in
  one place is incomplete only between its own wakes, and a task thread is its own place.

Part 12 notes (worker, 12.3):
- `organs._intend` now builds the target wake's trigger from the intent itself: `you set out to
  say '<text>' to <to>, because <goal> (from <source place>)`, the goal being the chain root's
  text (`work.chain_root`). `act.intent` keeps the capability, arguments, target and the goal.
  When that reason quotes a private line the target may not see, the words are withheld (`you
  set out to send a message to <to>; its words cannot appear here`): absolute line 2 outranks
  showing the draft to a place that may not read it.
- `reach.leaks` was the evidence for that filter: it only scanned private memories of the act's
  source place, so the owner's DM quoting Fulan's private line passed.
  `tests/test_reach.py::test_a_paraphrase_of_a_private_line_is_not_on_the_other_places_desk`
  failed on the first version of this item. It now scans every private memory, which is the
  general rule of the absolute line, not a case rule; no check was loosened.
- Scenario 19 gains three checks: Budi's wake carries the owner's request (`prompt_contains
  "Tolong selesaikan"`), names the person it is for (`prompt_contains "person:budi"`) and no
  wake says `carry out intent` (`prompt_absent`); 21/21. The owner's step-6 reply now names the
  group's place id (`Yang space:wa:1203630001@g.us ya (Tim Alpha).`) because a test organ
  reports no place title, so the model could not map "Tim Alpha" to a place. Asking and the
  single-group delivery stay the soul's own acts through `act to:` — no new mechanism.
- "Asks which group before sending" is the model's behavior on the desk it has: `guidance.py`
  already says to ask about a gap that matters, and `<recent>` lists the other places' lines
  with their ids. `guidance.py` is unchanged (21/30 lines).

Part 12 notes (worker, 12.4):
- `pfc.turn_check(context, now, settings)` validates each round's calls before the turn ends:
  tool-schema arguments present/unknown, `expect.validate` (so "expect needs cue_time or
  cue_event" comes back), and `resolve`'s id/status with the expectation present in the desk's
  `<expectations>`. `pfc.turns` marks such a call `refused`, puts `<name> refused: <reason>`
  into the next round's `<results>` and continues the turn (a refused call did not act);
  `jobs.apply_calls` skips refused calls and `threads.after_step` does not count them, so a
  refused call is never applied. `jobs.decide` takes the kernel for the clock/settings;
  `kernel._think` passes it. Nothing in the background job touches the shared connection.
- New test `test_an_expect_without_a_cue_is_corrected_in_the_same_wake`: round 1's cue-less
  expect is refused, round 2's corrected expect creates the expectation in the same wake (3
  `model.call`, one `expectation.created`). `tests/test_expect.py::test_expect_with_an_invalid_weight_becomes_an_error_event`
  pinned the old behavior (an `error` event one wake later); it is now
  `..._is_refused_in_the_same_turn` and asserts the reason in `<results>`, no expectation and
  no error. Scenario 09 passes 1/1; its check (`remind_at 11:00`, exactly one reply) is
  behavioral.
- Sizes: work 100 (new), recall 99, pfc 240/280, jobs 148/150, kernel 280/280,
  threads 88/120, reach 150/150, memory 260/260, desk 149/160, organs 139, guidance 21/30.
  Package 3,753 lines without bodies (limit 3,800). Absolute-line violations 0.
- Runs: `uv run pytest` 266 passed; fake replay 29 scenarios, 127/127 checks, missed-help 0.0%
  (0/63), false-speak 0.0% (0/66), honesty 61/61. Commits 64cce56 (12.2), 96d6ce4 (12.3) and
  3852a14 (12.4); each commit's own state was run with pytest and the fake replay before it was
  made (265/124, 265/127, 266/127).
