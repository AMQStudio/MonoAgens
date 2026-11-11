# Task 2 — Core: launch

Status: todo | Depends on: Task 1 (done)
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

- [ ] 7.1 Section token budgets from prior constants; trimming oldest-first per section.
- [ ] 7.2 A task thread's desk shows its work in `<doing>`: goal, one line per step, latest result verbatim, older results as pointers.
- [ ] 7.3 Middleware notes rendered with their event.
- [ ] 7.4 `recall(query | event_id, place?, from?, to?)`: time range, lineage, text search on events, scope reported; obeys absolute line 2 as built in Part 3.
- [ ] 7.5 Back-fill appraised as one age-discounted percept per place.
- [ ] 7.6 Check: a 15-step task keeps a flat prompt size; "what did we do last month?" lists that month's episodes with details from the original events, also after the vector faded and when sleep failed; 40 unread messages are appraised once.

## Part 8 — Threshold, gate and circuit breaker
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §3, §15
May touch: `monoagens/cognition/appraise.py`, `monoagens/cognition/appraisal.py`, `monoagens/cognition/rng.py`, `tests/test_cognition.py`, `scenarios/`

- [ ] 8.1 θ = base + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity); familiarity from ACT-R activation of interactions with the place and people present.
- [ ] 8.2 P = σ((S_eff − θ)/τ) with the seeded draw; every number stored in the decision event.
- [ ] 8.3 Circuit breaker: `addressed`, an active Expectation fired or answered, or an active goal continued.
- [ ] 8.4 Knob mapping function (one place) from the prior's knobs to θ, τ, γ and weights.
- [ ] 8.5 Check: same events + seed → same decisions; talk rate ≤ 1/n in a busy group; an addressed message always wakes; tuned for recall.

## Part 9 — Metrics and the real-model test
May touch: `scripts/replay.py`, `scenarios/`, `tests/`

- [ ] 9.1 Replay report metrics, **missed-help rate first** (of the messages that should have been answered, how many passed the gate), then false-speak rate, tokens per simulated day, absolute-line violations (must be 0), honesty-principle compliance.
- [ ] 9.2 Fake replay passes in full (`uv run python scripts/replay.py --fake`) and `uv run pytest` passes.
- [ ] 9.3 Real-model replay with the model configured in `.env` (`uv run python scripts/replay.py`, no `--fake`). Never read or print `.env`; check variables by presence only. Report the metrics, tokens used and every scenario that behaves differently from the fake run.
- [ ] 9.4 A short live session on the CLI body with the real model: a private fact said in one place is not used in another person's place; a promise is kept; the soul stays silent when not needed. Report what happened, not the secrets.

**Done when.** All parts are ticked; `uv run pytest` and the fake replay pass in full; the
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
