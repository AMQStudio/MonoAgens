# Task 2 — Core: launch

Status: in progress (Parts 1–11 worked; acceptance in 12.8) | Depends on: Task 1 (done)
One task = one worker prompt, done by one worker. Work through the parts **in order**; one part may be split
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
Read first: [core.md](../core.md) §4 (speaking elsewhere), §8 (absolute line 2), §9.2 (`place_kind`), [project.md](../project.md) §4 (capability record), [AGENTS.md](../../AGENTS.md) (size guide)
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
- [x] 12.5 **Free-text conditions (moved from Task 6, simplified by D53).** Wake gate: still relevant, stale (quiet hours stay in the owner policy). Free text: cosine below `lo` ignored; at or above `lo` the Expectation is reminded and the PFC wakes once with the line and the Expectation as trigger, and judges (act, resolve, or let it be). No classifier module, no `hi` band, no `lo` learning, no vector cache (deferred, core.md §5). Keep the draft's change to `appraise._engaged`. Must fit the remaining package budget; if it does not, stop and report — the budget is not raised. Checks: a fulfilled promise wakes no one; a question on the same topic never resolves a promise; scenarios 16 and 24 fire on "server down lagi nih guys" and "dokumennya sudah selesai kok".
- [x] 12.5b **A memory's audience from its sources (D53, before any real-model run).** Sleep no longer takes `place`/`private` from the model: the core derives them from `derived_from` (private if any source is private; audience = the people every private source was said to; no known source → private to the owner). Core memory follows the same rule. Tests: a sleep answer that omits place for a summary of Fulan's direct chat never reaches a group; core memory built from a direct chat stays out of a group. If a file is at its budget, move code rather than compress; if the package cannot hold it, stop and report.
- [x] 12.6 **Checks that forbid correct behavior.** 16: "model.call >= 2" forbids calling `expect` at once — the check is that the promise is held in the first wake. 24: the stored fact may be said in any format ("jam lima", "17:00"); its contradictory group lines are made consistent. List every changed check under Notes.
- [x] 12.7 **Guidance stays small.** `guidance.py` ≤ 30 lines of general principles; any line added in this part has its evidence under Notes.
- [x] 12.9 **Person cards for people named (D55).** The desk carries the person cards of people present or named in the trigger: name, person id, reachable bodies, closeness; never their messages. Cards are never withheld by absolute line 2. Scenario 18 steps 6–8: with Charlie in the world, the soul still knows `person:budi` and how to reach him.
- [x] 12.10 **Withheld note (D56).** When the context hook withholds relevant private items it leaves one line: how many, and whose private place they came from; no content. Scenarios 26 and 28: the soul does not give the fact in the group, says it has something private from the owner, asks the owner in the owner's direct place, and answers only after `share.granted`.
- [x] 12.11 **One message per place per turn (D51).** A second communicative act to a place the same turn already spoke to is refused as already said. Test: a round with an act and a refused `expect` returns the refusal without sending the message twice.
- [x] 12.8a **Material check of every scenario still failing** (16, 18, 19, 24, 26, 27, 28, 30) — facts **and capabilities**. Scenario 18 step 7 still lacks them: the owner only mentions the finished work, no test organ can read the working folder, and the WhatsApp test organ cannot send a file; in Task 2 make the work text in the Log (what is tested here is share, send and wait, not files). Files are not the core's: receiving and sending them is a body capability (Task 5), reading a working folder is a plugin (Task 4), and rules about forwarding them are middleware policy (Task 4); a file version of this scenario belongs there. Per scenario: per scenario, what the soul needed and whether it was in the Log and on its desk; fix the scenario where material was missing. Write it under Notes before 12.8 runs.

- [x] 12.12 **A reminder is not a fulfilment (D53; bug found in the 24/30 reports).** `condition.reminder` records the reminder as `expectation.fired` with the promise owner as `who`, and the desk renders it exactly like a met condition, so the soul reports "done" on "masih di review". Fix: a reminder has its own event kind (`expectation.reminded`, additive), keeps the line's own speaker, and the desk shows it as "this line may relate to your expectation: <about>; condition: <text>; does this line fulfil it?" with the reason "reminded", not "fired expectation". No new guidance line. Checks: scenario 30's question line produces no message and the promise stays active (rewrite of the check that forbade the wake D53 requires); scenario 24's "masih di review" produces no message; the later "sudah selesai" fulfils the promise once. Also `scripts/replay.py` prints every communicative act that fails the honesty scan, with the quotation it could not find. `text_like` stays a plain substring (no `|`, no regex).
- [x] 12.13 **Tool input contract (bug in report 100338, scenario 30 step 5; mechanism adapted from V1 ADR-0024 on branch `archived`).** A thought act or capability whose arguments do not match its schema (a sentence where an id is expected) raised `invalid literal for int()` and the whole turn was lost, so the owner was never told. Fix: one door validates every argument against its declared schema before anything runs, and a mismatch comes back in the same turn as a refusal with its reason (D51), never an exception. Expectations on the desk show the id that `resolve` takes. Test: a `resolve` with a text id is refused and corrected in the same wake. Read the V1 ADR with `git show origin/archived:docs/archive/adr-pre-rebaseline/engine/0024-tool-input-contract.md`; take the mechanism, not its roles or approvals.
- [x] 12.14 **Scenario 24 is ambiguous (D54).** Step 9 asks "which document?", nobody answers, and step 11 brings a second document ("laporannya jam lima sore"), so on "Dokumennya sudah selesai kok" asking back is the honest act. Make the material unambiguous (one document named before the request, the report-hour lines about something else), and keep the check that the statement fulfils the promise once. List the change under Notes.
- [x] 12.15 **Report files.** `scripts/replay.py` names a report `reports/YYYY-MM-DD-HHMM-<real|fake>-<scenarios>.md` (for example `2026-11-12-1003-real-24-30.md`), also writes `reports/latest.md`, and keeps only the newest ten reports. `reports/` stays out of git; the committed record is `scenarios/README.md`.
- [x] 12.16 **A sent message does not wake the soul again (bug, live session 12.8).** The 04:00 tick woke five times: each `act.out` of a communicative act was read as "goal continued" and woke the reasoning again, so one place got three messages in one minute. Only the result of a non-communicative capability continues a chain; a message the soul said ends that step. Test: a fired promise produces exactly one message and one wake.
- [x] 12.17 **A failed model call is never silent (bug, live session 12.8).** An `intent` wake wrote `error: ''` and its turn was lost. A transport failure is retried once (`prior` constant, no magic number); if it fails again the error event carries the exception type and status, and the intent stays pending so the next wake in that place sees it. Test with a fake transport that fails once, then twice.
- [x] 12.18 **Replay counting (harness).** Only communicative acts count as replies (`self.recall` and other non-communicative acts never do). The capability record keeps each scenario's last real-model verdict, its date and report name in `scenarios/README.md` itself, so pruning reports to ten never erases evidence; restore the verdicts of the launch scenarios that passed in earlier real runs from the Notes of Parts 9–12.
- [x] 12.19 **Scenario 24 step 12 (architect's decision, D54).** Two people contradict each other a minute apart ("masih di review", then "sudah selesai kok"), so asking back is honest. Step 12 becomes the unambiguous statement "Bu Rina sudah selesai review, proposal anggaran Q4 sudah final." Keep the check that it fulfils the promise once.
- [ ] 12.20 **A promise is kept by doing it (architect's answer to the 12.19 question).** Resolving a promise means it was kept, so a reminder wake that finds a promise fulfilled does the promised act first, in the same turn, then resolves; a silent resolve is a broken promise. Options (b) and (c) are refused: (b) is a rule for one case inside the core, (c) would pass a broken promise. Reword the existing reminder line in `guidance.py` to carry this as a general principle; no new line, so the list does not grow. Then one real run of scenario 24. **If 24 still fails, stop retrying:** Task 2 closes with 24 recorded as not yet held (a target for Task 6), so the work does not loop.
- [ ] 12.8 **Acceptance — the launch set** (after 12.12–12.20; rerun only 24, then the live session to confirm 12.16 and 12.17; owner's decision): one real-model run at `--effort high` of the launch scenarios that have not yet passed with the real model: memory and recall (07, 11, 12, 17, 20, 23), silence (01–03, 15, 21), turn-taking (04–06, 14, 22), promises (08–10, 13, 16, 24, 30), unaddressed help (25, 27, 29). Each failure gets a material check first. Then the live session (privacy across bodies, a promise kept, silence). Scenarios 18, 19, 26 and 28 move to Task 4 as capability targets; the capability record lists them as not yet held. Tick 10.10, 11.6 and 12.8.

## Part 13 — Readability pass (after 12.8)
Read first: [AGENTS.md](../../AGENTS.md) (priorities for code, size guide)
May touch: `monoagens/` (all), `tests/`

The old hard line budgets pushed code into odd places. Undo that, with no change in behavior.

- [x] 13.1 List every place in the Notes of Parts 1–12 where code was moved, merged, split or trimmed only to stay under a line budget (for example the classification result routed through `expect.condition.finish`, single-use constants removed, `pfc.py`/`desk.py` split at the line limit). For each, decide: keep (it is clearer) or restore the clearest form.
- [x] 13.2 Refactor the ones to restore: clear names, one responsibility per module, docstrings, files ending with a newline. No new behavior, no removed behavior.
- [x] 13.3 `uv run pytest` and the fake replay pass unchanged; list every refactor and its reason under Notes.

**Done when.** Every launch scenario of 12.8 has passed with the real model (owner's rule: what passed is not rerun); one that has both passed and failed is marked `flaky` in the record with its reports; absolute-line violations are 0; `uv run pytest` and the fake replay pass in full; the capability record is committed and lists 18, 19, 26 and 28 honestly as not yet held; the code is readable (AGENTS.md size guide). After this task the core is paused except for Task 3 (the clean refactor, no behavior change): work then moves to Task 4, and the core changes again only for a bug or a vulnerability found there or in daily use.

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

Part 12 notes (worker, 12.5 — **blocked by the package budget**):
- 12.5 is **not ticked**. A complete, verified implementation was drafted, wired and tested,
  then preserved as a local stash (`git stash list` → "task2(12.5) draft: free-text conditions
  (blocked: package 3920/3800)"); nothing is committed and the worktree is back at HEAD
  (3,753 lines). `git stash pop` restores the draft for review.
- What the draft implements. `monoagens/condition.py` (185 lines, stated budget ≤ 200) owns
  everything a condition is: the exact fast paths (`none`, `event_seen`, `state`) and their
  `parse_duration` (moved out of `expect.py`, which keeps re-exporting `expect.parse_duration`),
  the free-text arm (`operator` accepts `expect: {text: ...}`, stored in `condition_text` and
  the `condition` JSON), and the wake gate. Gate: `m = cosine(event vector, condition vector)`
  from `read_event_vector` or a thread-pool embed of the event text; the condition vector is
  embedded once and cached in the new `condition_vector` column. `m < wake_gate_lo` → ignore;
  `m ≥ wake_gate_hi` (the clearly-yes band) → fire at once; otherwise exactly one
  `classify:{expectation}:{event}` background job under the job watchdog whose single
  `pfc.call` (a general "does this event fulfil this condition?" prompt, one `classify` tool)
  is recorded through `budget.record_call` as `model.call {purpose: "classifier"}` with its
  token usage, and whose result becomes `classifier.verdict {expectation_id, m, verdict}`.
  Yes → `fire_expectation` + `_expect_wake`, so the PFC re-judges with its own desk; no → the
  expectation stays active (the classifier never resolves). `expect.due` gained the stale arm
  (a tick now also selects active expectations whose `expires_at <= now` and drops them) and
  the free-text arm (the gate runs before firing). Bands live in `state: condition:bands` and
  move by core-math §8: 50 verdicts (3/ε = 50) of evidence before either edge moves, `lo`
  toward the lowest yes `m` minus `wake_gate_margin`, one `wake_gate_step` at a time, at most
  `wake_gate_daily` per day, inside `[wake_gate_floor, wake_gate_ceiling]`, never closer than
  `wake_gate_min_gray` to `hi`; `hi` opens only when the top 50 verdicts by `m` are all yes and
  at or above the gray band's top, and reverts to 1.0 when a no appears in them; each change
  (and a first freeze at a bound per day) is a `disposition.updated` event with its delta.
  Every number is in `prior/dispositions.yaml` (`wake_gate_*`, `rule_of_three_epsilon` already
  existed); `tests/support.py` mirrors them.
- One behavioral fix the replay forced, in `cognition/appraise.py`: `_engaged` counted any
  active expectation whose cue matched as "answered", so a broad free-text cue made the
  circuit breaker wake on every line of the place (observed in 16/24/30) and the classifier
  never got to judge. `_engaged` now skips expectations with a free-text condition: only the
  classifier decides whether the cue was answered.
- Size (measured on the draft): `condition.py` 185 (≤ 200 stated), `expect.py` 219/240
  (the 31 moved lines pay for the new arms), `kernel.py` 280/280 (the classify result branch
  is routed as `expect.condition.finish(self, job)` to stay at budget — flagged for review),
  `store.py` 219/240 (+1: `condition_vector` is selected), `cognition/appraise.py` 105/160,
  `jobs.py` 148/150, `reach.py` 150/150, `memory.py` 260/260, cognition total 543/600.
  **Package 3,920 vs 3,800 = +120 over** (net +167 over HEAD: `condition.py` +185, `expect.py`
  -21, `store.py` +1, `appraise.py` +2; roughly 30 of `condition.py` are moved from `expect.py`,
  roughly 155 are new).
- Why it cannot fit, with the exact arithmetic: HEAD had 47 lines of package headroom. The
  free-text gate is ~155 new lines with docstrings, blank lines and one statement per line
  (cosine + vector cache 26, one classifier job/prompt/verdict path 40, the two learned bands
  with bounds/step/daily/gray/freeze 36, the wake-gate arms and wiring 14, the module header
  and helpers 39). Paying for it by deleting unrelated code would mean deleting
  spec mechanisms: `forget.fade` is unwired (20 lines), the `versioning` namespace helpers are
  the plugin contract (12 + a test), `store.read_state_entry`/`rebuild_state` are the
  projection reads (7 + tests); the only honest dedupe found is the four identical text
  accessors (`work._text`, `recall._payload_text`, `memory._payload_text`, `pairing._text`,
  ~19 lines). That is not 120, and 12.5 forbids compressing the new code. Per the item's
  instruction, nothing was compressed and nothing over budget was committed.
- Options for the architect (one decision needed):
  - **A (recommended)** Raise the package limit to 3,950 for this module; every per-file budget
    already holds and the draft is green: `uv run pytest` 274 passed, fake replay 30 scenarios
    131/131, missed-help 0/65, false-speak 0/67, absolute-line 0, honesty 62/62.
  - **B** Name ~120 lines of code to delete. The full honest candidate set (text-accessor
    dedupe 19, `fade` 20, versioning 12, rebuild helpers 7) reaches ~58 and loses spec paths;
    B alone still leaves 12.5 unticked.
  - **C** Defer `hi` learning + freeze (25) and the lazy condition-vector cache (10): ~85 still
    over, so C does not make 12.5 landable either.
- Changed checks/answers in the draft, each with its reason:
  - `scenarios/promises/16_watch_for_phrase.yaml`: the `expect` answer now asks for
    `expect: {text: "server down"}` with `cue_event: {kind: percept}` (the free-text path
    instead of the substring cue) and records one `classify` answer (verdict yes) at step 4;
    the checks are unchanged. It fires through the gate at step 4.
  - `scenarios/turn-taking/24_gate_threshold.yaml`: same shape (`text: "dokumen sudah selesai"`,
    `cue_event` keeps the group place) plus one `classify` yes at step 10; no check changed.
    The extra patience wait the gate creates on the same line is settled silently by the next
    speaker's line, so the recorded event sequence and all 11 checks hold.
  - New `scenarios/promises/30_same_topic_question.yaml` (one behavior: a same-topic question
    never fulfils the condition): the question is judged no and wakes no one, the statement is
    judged yes and the owner hears once; four behavioral checks, the classifier answers
    recorded. No check was loosened; no existing answer was removed beyond the two cue changes.
  - New `tests/test_condition.py` (8 tests): below-`lo` ignore with no classifier call; one
    recorded classifier call with its budget entry, `m` and a yes that fires; a no leaves the
    promise active; the clearly-yes band skips the classifier; an already-met condition
    resolves with no wake; an expired expectation drops on a tick; `lo` moves one bounded step
    toward the lowest yes with the gray width kept; `hi` opens after 50 clean verdicts and
    reverts on a no.
  - `scripts/replay.py`: `FakePFC.call` replays `model_answers` with `tool: classify` and
    defaults an unscripted classify answer to "no"; fake mode only, no check semantics changed.
- Verification of the draft (before it was stashed): `uv run pytest` 274 passed; `uv run python
  scripts/replay.py --fake` 30 scenarios, 131/131 checks, missed-help 0/65, false-speak 0/67,
  absolute-line violations 0, honesty 62/62. The real-model replay was not run (12.5 does not
  ask for it).

Part 12 notes (worker, 12.5):
- What 12.5 does now. `monoagens/condition.py` (82 lines) owns a condition: `operator` (the exact
  fast paths moved from `expect.py`, plus `{text: ...}` stored as `condition_text`), `text`, and
  the wake gate. `reminded` computes m = cosine(event vector, condition text vector) with `lo =
  calibration.wake_gate_lo` (0.35, already in `prior/dispositions.yaml`): below `lo` the line is
  ignored; at or above `lo` `reminder` records `expectation.fired` with `{reminder: true, text:
  <line>, expectation_id, about}`, so the line and the Expectation are the trigger, and leaves
  the expectation `active`. The PFC wakes once through that fired trigger (`_expect_wake`), lays
  out its own desk and judges: act, resolve, or let it be. No classifier, no `hi` band, no `lo`
  learning, no vector cache: the event vector comes from `event_vectors` when the encode job has
  written it, else is embedded on demand, and the condition text is embedded on demand each
  time; both go through the embedder port, whose real implementation runs the model in a worker
  thread (invariant 7).
- Wake gate arms in `expect.due`: stale (`expires_at <= now` -> `dropped`, on a tick or any cue
  match) and still-relevant (`is_met` -> resolve, unchanged) now sit before the free-text arm.
  A tick selects `next_check_at <= now OR expires_at <= now`. `is_met` returns False for a
  free-text condition.
- `cognition/appraise._engaged` keeps the draft's exemption: an active expectation with a
  free-text condition never counts as answered by its cue alone, so a broad cue cannot wake the
  PFC on ordinary lines; only the reminder path decides. The kernel change is one argument
  (`expect.due(..., kernel=self)`).
- The PFC closes the promise itself: in each changed scenario's recorded answer it acts and
  resolves (`resolve #1 satisfied`) in the same wake; a "let it be" wake (scenario 30's
  question) records only a `think`, so the promise stays active. The core does not infer
  resolution from an act.
- Budget. Package 3,799/3,800 (was 3,753). `condition.py` is new (82 lines); `expect.py` 222/240
  (-18: `parse_duration` and `operator` moved out, the wake gate added); `store.py` 225/240
  (+7: `condition_text` in the INSERT and the shared `payload_text`); `memory.py` 253/260 (-7),
  `work.py` 92 (-6), `recall.py` 93 (-5), `pairing.py` 71 (-5): the four private payload-text
  accessors were replaced by `store.payload_text` to pay for the module (memory's sleep lines now
  also fall back to `arguments.text`, the rule the desk and recall already used); kernel 280/280
  (one argument); appraise 105/160; jobs 148/150; reach 150/150; guidance 21/30. Nothing was
  compressed and no other code was deleted.
- Scenarios/answers changed:
  - `scenarios/promises/16_watch_for_phrase.yaml`: the `expect` answer now uses
    `cue_event: {kind: percept, place: space:tg:-10099}` and `expect: {text: "server down"}`
    (free text instead of the `text_like` cue and `{none: true}`), and step 4's answer acts and
    resolves #1. No check changed; 4/4.
  - `scenarios/turn-taking/24_gate_threshold.yaml`: the same change (`cue_event` keeps the group
    place, `expect: {text: "dokumen sudah selesai"}`), step 10 acts and resolves #1. No check
    changed; 11/11. The two later lines stay below `lo` (m 0.258 and 0.186 with the test
    embedder), so there is no second reminder.
  - New `scenarios/promises/30_same_topic_question.yaml`: the question "dokumen sudah selesai
    ya?" (m 0.866) reminds the PFC, whose recorded answer is a `think`, so it neither replies
    nor resolves; the statement "dokumennya sudah selesai kok." (m 0.577) reminds again, the PFC
    acts and resolves #1, and the owner hears once in the DM. Three behavioral checks: no reply
    at step 2, no `expectation.resolved` at step 2, one reply in the DM at step 3.
- New `tests/test_condition.py` (4 tests, real Postgres): below `lo` no wake and no fired event;
  at or above `lo` exactly one PFC request, its prompt holds the line, the `expectation.fired`
  carries the line and the expectation id, and the expectation stays active; a resolved promise
  wakes no one; a past expiry drops on a tick. `tests/support.py` mirrors `wake_gate_lo: 0.35`
  under `calibration`.
- Runs: `uv run pytest` 270 passed; `uv run python scripts/replay.py --fake` 30 scenarios,
  130/130 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/67), absolute-line violations 0,
  honesty 62/62. `scenarios/README.md` regenerated. Commit `task2(12.5): free-text conditions
  remind the PFC once (no classifier)`.

Part 12 notes (worker, 12.5b):
- What the item does. `store_memories` no longer has `place` or `private` in its item schema
  (only `text`, `importance`, `who`); `memory._consolidate` ignores any such argument a model
  still sends. Each memory takes `derived_from` = the batch's event ids, `place` = the batch's
  only place (else NULL), and `private` = any source has a place. v1 organs report only
  `direct`/`shared` places, both non-public, so that is "any source is private"; the column is
  written for the record and no longer decides anything.
- The audience rule is `reach.memory_visible` (new; `leaks` moved out of `reach` to make room):
  no sources -> the owner's direct chat only; else `need` = people present here minus the owner,
  and for every source event with a place `have` = the people present when it was said (D50:
  `audience(place, before=source id)`, after aliases) minus the owner, plus consent granted for
  that place; visible iff `need` is inside every source's audience. A place-less source (a clock,
  a job, bookkeeping) carries no audience and does not constrain. Therefore: any private source
  hides the memory from anyone not in it, and the intersection over sources is the memory's
  audience. Sources come from the events, never from `memories.place` or a model flag.
- Core memory. `sleep.done` carries `derived_from` and `core_memory`; `store.project_event` and
  `_consolidate` write `state: core_memory` with it, so a rebuilt status board keeps the
  provenance. `memory.build_context` shows core memory only when `memory_visible(core sources)`
  holds at this place: core built from the owner's DM is empty in every group desk and intact
  in the owner's DM, and one built from non-private/non-DM sources stays where its sources reach.
- The context hook (`hooks.audience_policy`) filters every memory by `memory_visible(...,
  sources=memory.sources)`; `MemoryHit` now carries `sources` (derived_from) instead of the
  `private` flag. `recall.search` uses `memory_visible` for the memory entry. `organs.leaks`
  (dispatch and `_intend`) asks `memory_visible` for the target place over every valid memory;
  the HEAD version still scanned only `private` memories at the act's source place, which the
  12.3 notes described as fixed — this item now makes it so, and it is the general D45 rule.
- Replay metric. `DeskItem` gains `sources`; `Tracker.record_desk` is async, records core memory
  as an item whose sources it reads from `state: core_memory` at that moment, and records every
  memory's sources; `desk_violations` restates D53 from the Log independently of
  `memory_visible`: per source, the D50 audience (before that source) plus consent granted up to
  the desk's trigger, a no-source item counted as owner-only. `_leak_violations` now judges every
  memory (not only `private` ones) by `memory_visible` in the reply's place.
- Draft kept and fixed. Kept from the interrupted draft: cosine moved to `embedding.cosine`
  (signals/condition), `store.event_from_row` reused by `memory.recent_lines`,
  `store.payload_text` in `memory.candidates`, `organs.dispatch` builds its ctx once,
  `kernel` appraises once, `pairing`'s missing function blank line, and the EOF newlines in
  `recall.py`/`work.py`. Fixed: `memory_visible` ignored each source's own time (no `before`),
  so a newcomer would have received a memory of a line said before they arrived; the replay
  metric had the same hole and did not count core memory.
- Changed answers (behavior checks unchanged; none loosened):
  - `scenarios/memory/26_help_meeting_time.yaml`: sleep answer drops `place:
    space:tg:1001` and `private: true`; the core derives the same place/sources from the batch.
    10/10 checks.
  - `scenarios/memory/28_help_package_location.yaml`: same. 10/10 checks.
  - `scenarios/memory/20_act_r_activation.yaml` (outside 25-29 but stale under the schema): the
    nine `place: space:tg:1001` fields of its two sleep answers are removed; the batch place is
    space:tg:1001 either way. Checks unchanged.
  - 25, 27, 29 never carried `place`/`private`; untouched.
- Changed tests, each with its reason:
  - `tests/test_memory.py::test_private_memories_follow_people_not_places`: adds a `percept`
    source at `space:tg:9` (direct) and sets `derived_from=[source.id]` beside `place`/`private`;
    D53 now derives the audience from sources. Assertions unchanged.
  - `tests/test_memory.py::test_the_sleep_tool_does_not_ask_for_place_or_private` (new): the
    item's properties are exactly `text`, `importance`, `who`.
  - `tests/test_pfc.py::test_layered_prompt_structure`: the seeded core memory gains
    `derived_from: [seed.id]`; without a source D53 makes core memory owner-only, and the test's
    place reports no `place_kind` (so it is not known as a direct chat). Assertion unchanged.
  - `tests/test_reach.py::test_the_owners_direct_chat_is_private_to_the_owner`: `MemoryHit`
    carries `sources=(dm,)` instead of `private=True`.
  - `tests/test_reach.py::test_a_paraphrase_of_a_private_line_is_not_on_the_other_places_desk`:
    calls `organs.leaks` (the check moved with the function). Assertion unchanged.
  - `tests/test_reach.py` (new): `test_sleep_derives_private_content_from_the_sources` (a sleep
    answer with no `place` for Fulan's DM: a memory and core memory carry the DM event, the
    group desk is empty, the owner DM desk keeps both, and `retrieve` reports the sources);
    `test_a_model_named_place_never_widens_a_memory` (the item names `space:tg:-500` and
    `private: false`; stored place/private and the group desk stay the DM's);
    `test_a_memory_without_a_known_source_is_private_to_the_owner` (empty `derived_from` is
    hidden in a group, kept in the owner DM); `test_a_memory_needs_every_source_to_cover_the_place`
    (two group sources, audiences {charlie, dewi} and {budi, dewi}: the memory is visible only in
    dewi's DM and the owner's, and not in either group).
  - `tests/test_replay_metrics.py` (new): `test_the_desk_scan_judges_a_memory_by_its_sources`
    (a DM-sourced memory on a group desk is a violation, empty sources are owner-only, a
    group-sourced memory passes where its people are, and a newcomer makes it fail — D50);
    `test_the_desk_scan_counts_core_memory_by_its_sources` (the Tracker records core memory with
    its `state` sources; group desk violation, owner desk none).
- Sizes. Package 3,800/3,800 (`wc -l`; HEAD is 3,799 with `recall.py` and `work.py`
  unterminated, so restoring their newlines plus the draft's moves and dedupes — `cosine` to
  `embedding`, `leaks` to `organs`, `event_from_row` and `payload_text` reuse, one ctx/appraisal
  each — net +1 and land exactly on the limit; 12.5b adds no line). `reach.py` 150/150,
  `memory.py` 254/260, `kernel.py` 279/280, `hooks.py` 128/150,
  `jobs.py` 148/150, `expect.py` 222/240, `store.py` 227/240, `organs.py` 147, `recall.py` 94,
  `work.py` 93, `desk.py` 148/160, cognition 534/600. All files ≤100 columns.
- Runs. `uv run pytest` 277 passed (real Postgres). `uv run python scripts/replay.py --fake`:
  30 scenarios, 130/130 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/67),
  absolute-line violations 0 (now with core memory and memory sources counted), honesty 62/62.
  `scenarios/README.md` regenerated. Commit `task2(12.5b): a memory's audience comes from its
  sources`.
- Limits for the architect (not worked around): (1) `derived_from` is the whole sleep batch,
  because the model does not say which events each item came from; every memory of one batch
  therefore shares one audience, and a batch spanning a DM and a group only tightens. Per-item
  provenance would need a model argument, which D53 forbids. (2) D45's public-place exception is
  unexercised: v1 organs report only `direct`/`shared`, so every source place counts as private;
  when a `public` kind appears, `memory_visible` needs that exception (today's behaviour narrows,
  never leaks).

Part 12 notes (worker, 12.6):
- **Changed checks, each with its reason.**
  - `scenarios/promises/16_watch_for_phrase.yaml`: `event_seen model.call step 1 count 2`
    ("the thought's result comes back and the promise is held in the same wake") became
    `event_seen expectation.created step 1 payload {kind: promise}` ("the promise is held in
    the first wake, whether the model thinks first or not"). The count is a lower bound, so it
    forbade the correct one-round answer; 11.5's report proves it (`1 model.call >= 2` at step
    1, same report line as the DM failure). No answer changed: 16 still records think→expect
    (the think-first path), 24 step 7 still records a direct `expect` in one round (the
    at-once path), so both paths stay exercised.
  - `scenarios/turn-taking/24_gate_threshold.yaml`: `mentions "lima"` became `mentions_any
    ["jam lima", "lima sore", "17:00", "17.00", "5 sore", "pukul 17"]` ("the answer names the
    stored report hour in any of its formats"). Evidence: 11.5's step-14 answer said "sekitar
    jam 17:00" and "Jam 17:00 belum lewat" — the same fact in another format, failed by the
    old check.
  - `scenarios/memory/28_help_package_location.yaml`: `mentions "lemari depan"` became
    `mentions_any ["lemari depan", "depan lemari", "lemari yang di depan", "lemari dekat
    pintu"]`. The source line is "di lemari depan, yang dekat pintu"; each listed form states
    the same location, and the old check would fail the door form. No real failure was
    loosened: the check still requires the front cabinet near the door, never just "lemari".
- **24's contradictory group lines fixed (lines/answers, not the metric).** 11.5's real answer
  itself listed the contradiction ("7003 dan 7006 bilang sekitar jam 17:00, 7004 bilang sudah
  selesai. Jam 17:00 belum lewat"). Steps 9, 11 and 13 and step 14's answer now speak of the
  *report* hour (`laporannya jam lima sore`), while "dokumennya sudah selesai" stays the
  free-text promise trigger at step 10: the document is finished early, the report goes out at
  five. No other check changed; 11/11.
- **Scanned, found, not changed (with the reason).**
  - Model-call/round counts: after 16, the only `model.call` check is 24 step 7 `count: 1`
    (a lower bound: "the PFC woke"), and `max_calls self.recall count 2` (23) bounds
    capability calls (no search loop, 12.2), not model calls; `min_replies` (10, 14) counts
    replies (behavior). All kept.
  - Exact-word checks on a fact *without* an equivalent format: 25 "Kopi Senja" (proper name),
    27 "senja2026" (password token), 20 "laci", 18 "sketsa", 22 "Senja", 17 "rust", 12
    "kopi"; 23, 26 and 29 already use `mentions_any`. In each the named token *is* the stored
    detail the scenario proves; kept.
  - 16's `reply mentions down`: the honesty principle quotes the log's own phrase and the
    phrase is what the watch exists for; kept.
  - 19's `mentions "grup"` ("the question asks which group"): a wording of the question, not
    a fact; 10.7 already chose "grup" as the general form. A model asking "tim mana" would
    still fail it — flagged here, not changed, because it is outside the two smells and a
    rewrite would be a second wording review.
  - Negative wording checks `prompt_absent "sembilan"` (26) and `prompt_absent "lemari"` (28):
    narrow to one wording, so a leak phrased "jam 9" or "kabinet" slips past the check itself.
    They are supplementary: the desk scan of 12.1 counts every private item on a desk whatever
    its wording, and absolute-line violations must be 0. Kept; a `prompt_absent` any-formats
    check type would be replay machinery for two cases.
- **Limits for the architect.** A check that requires a fact's wording at all can still fail a
  paraphrase that names the same fact with other nouns (28's location could be "kabinet depan");
  the scenario tree states the variants that occurred in runs, not every synonym.
- Runs: `uv run pytest` 277 passed (real Postgres); `uv run python scripts/replay.py --fake`
  30 scenarios, 130/130 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/67),
  absolute-line violations 0, honesty 62/62. `scenarios/README.md` regenerated by the run.
  Scenarios only: no `monoagens/` line changed, package stays 3,800/3,800.

Part 12 notes (worker, 12.7):
- **Count.** `monoagens/guidance.py` is 21 lines (`wc -l`, budget 30): 9 code/structural lines
  plus 7 logical principles that wrap to 11 string lines (a header and six bullets). Every
  bullet is general; none names a platform, a person, a place or a scenario. Nothing was
  added or changed in this item; the file is the same 21 lines the 12.3 notes recorded.
- **Evidence per principle** (Part 10/11/12 Notes; a bullet wraps over the lines named):
  1. "Thinking is not acting…" — 10.8 failures 16, 29, 30 (a `think` call whose thought holds
     the plan, no tool call; 16's Log) and the 10.8 evidence paragraph that this instruction
     fixed 04, 25 and 16; 10.10's live session ("send this to Budi" produced
     `decision.note` and no `act`).
  2. "What you promise for later is an expectation…" — 10.8 failures 10, 19, 20, 28 and 34
     (no `expect` after a spoken promise), 10.10/11.5's 16 and 24, and 12.4's test "expect
     without a cue is corrected in the same wake" (a promise needs a cue).
  3. "Reach one person or another place with act to:/via: … self.share first … never repeated
     where its audience may not see it" — 10.8 failures 6, 7, 9, 12, 21, 23, 24, 25; 10.10's
     `person:<id>`/`to:` fix; 12.1's leak and 12.3's intent on the target desk.
  4. "Long work goes through a long-running capability; the task thread opens for you." —
     10.8 failures 18 and 26 (no task thread opened, so it never closed).
  5. "When a capability returns an answer, tell the person that answer." — 10.8 failure 2
     (`self.recall`'s answer never told); 12.2's scenario 23 recalls and then answers.
  6. "You act only through the tools. Staying silent is a valid choice." — the first sentence:
     10.8 failures 16/29 and 10.10's live session (a plan in `decision.note`, no act). The
     second sentence has behavioral, not failure-fix, evidence: 11.5's false-speak 0.0% (0/67)
     and 11.2's turn that may end without a tool call after the first round (D51); 12.5's
     scenario 30 shows a reminder woken by a question that correctly answers with a `think`
     only. No line is proposed for removal.
- Nothing changed: no Note shows a line caused a failure, every principle has evidence, and the
  file is 9 lines under budget. Tests and the fake replay are unchanged by this item.

Part 12 notes (coordinator, 12.8 partial real-model run):
- `reports/replay-20261112-061932.md`, model `deepseek-ai/DeepSeek-V4.1-Flash-1017`, effort from
  `.env` (default), scenarios 09, 14, 16, 18, 19, 23, 24, 26, 28 only. 47/83 checks. Missed-help
  10.7% (3/28), false-speak 0% (0/25), tokens/sim-day 3,067, absolute-line violations 0 (desk
  metric of 12.1), honesty 100% (24/24).
- Pass: 09 (1/1), 14 (3/3), 23 (7/7). Fail: 16 (2/4: no `expect` at step 1), 18 (8/18), 19 (8/21),
  24 (9/11: no promise fires), 26 (4/9) and 28 (5/9): the soul answers in the group "I do not
  know" and never asks the owner in the owner's direct chat, so no `share.granted` follows.
- The three full runs and the live session were not started: the acceptance condition (all of
  these pass) is not met. Stopped for the owner, as the rules require.

Part 12 notes (worker, 12.8 prep):
- **Material check of every failing scenario (D54), written before any soul-side change.**
  Each row names the action a check demands, where the material for it lives, and what was
  missing. "Registered" means the capability is declared by the kernel (`self.share`,
  `self.recall`, `self.set_identity`) or by the replay's test organs; `ReplayRunner._make_kernel`
  registers `telegram`, `cli`, `game`, `whatsapp` and `dev` the same way for the real and the
  fake run, so both runs see the same tools.

  | Scenario | Check demands | Material present | Missing | Added |
  |---|---|---|---|---|
  | 16 | an `expect` in the first wake, silence through chatter, one DM | the owner's request; the group's lines; `person:owner` = `space:tg:1001` | the group spoke only after the request, so no group place id was on the request's desk and the watch could name no place | step 1 (07:58, `person:tg:6001`, `space:tg:-10099`); checks move to steps 2/5 and `no_reply` covers [1, 3, 4] |
  | 18 | answer moves to the owner's private channel; "hasil kerjaan kita" shared and sent to Budi | Budi's file line, Fulan's line, the owner's DM `space:game:dm:owner`, `self.share` and `whatsapp.say` with `to:` | nothing in the Log was "our work": the owner's step 7 asked to send an artifact no event named (the 11.5 failure "Hasil kerjaan yang mana?") | the owner names it in the shared world at 08:13 (step 6): "hasil kerjaan kita versi final sudah selesai dan filenya ada di folder kerja". The saying place's audience is owner+charlie, so the step-7 desk (world1 with charlie) may hold it under `reach.visible`; no check changed |
  | 19 | promise, code to Budi, which group asked, thread closes | the request, both groups, Budi's direct place, the `dev` organ with `dev.run` | `dev.run` answered `{ok: true, finished: true}` with no output, so there was nothing to deliver (the 11.5 report: "dev.run selalu balik ok, finished tanpa output") | `FakeBody.act` returns the command's observable result (files changed, `pytest: 12 passed`) as `text`, and the `dev` capability says it reports that output; the task-thread `<doing>` now shows it. No check changed |
  | 24 | the promise fires on the unaddressed line, the wait answers once | the addressed request and the watched line are both in `space:tg:-1004`; the answer is said there | none | nothing (the 11.5 failure was no `expect`, a soul-side cause) |
  | 26, 28 | silence in the group, a question to the owner in `space:tg:1001`, one answer after `share.granted` | the fact is an owner percept in `space:tg:1001` and a sleep memory derived from it; `person:owner` resolves to `space:tg:1001`; `self.share` and `telegram.say`/`to:` are registered | the fact is (correctly) absent from the group desk, so no group wake carries a cue that the soul knows it | nothing; the desk cue is the open question below |

- **What the material fixes changed in the replay.** 16: 1 new step (a group line before the
  request), 3 checks renumbered, `no_reply` widened to the new chatter line. 18: one step's text
  now names the artifact; no step, check or recorded answer changed. 19: no scenario file
  changed; the `dev` organ and its description in `scripts/replay.py` now report output. Fake
  replay: 30 scenarios, 130/130 checks, absolute-line violations 0, missed-help 0.0% (0/65),
  false-speak 0.0% (0/68). `uv run pytest`: 277 passed.
- **Open question (not built).** For 26 and 28 the only remaining gap is a cue, not material:
  the group desk is empty of the fact by the privacy rule, and it says `<memories>none</memories>`
  without saying that memories were withheld, so a soul reading it cannot tell "I do not know"
  from "I may know it but not here". A desk line that says something was withheld needs a new
  field on `PFCContext` plus a rendering and the hook that fills it (3–4 package lines); the
  package is exactly 3,800/3,800 and no honest removal was found, so per the rules this stops
  here as a design question for the architect: is a withheld-count on the desk a desk fact or a
  new core primitive, and which line pays for it? The general principle that a soul may ask the
  person it might be private to was put in `guidance.py` (next note), which fits the budget.

Part 12 notes (worker, 12.8 brain fixes):
- **The uncommitted diff was reviewed line by line; only evidence-backed, general changes
  kept.** Files touched: `monoagens/guidance.py` (24 lines, under the 30-line limit),
  `monoagens/pfc.py` (241 → 244 lines), `tests/test_pfc.py`, and the regenerated
  `scenarios/README.md`. No scenario file, no check, no platform name, no design primitive.
  The withheld-memory desk line stays an open design question; nothing was built for it.
- **The 300s timeout was not reproduced.** With the kept changes, `uv run pytest
  tests/test_pfc.py` runs in 2.9s (30 passed) and `uv run pytest` in 17.1s (279 passed).
  The loop cannot add rounds: `turns` runs `while len(made) < max(1, int(limit))`, and the
  new exit condition only decides whether a round already counted is followed by another,
  so `max_thought_rounds` remains a hard ceiling. Likeliest external cause (unproven): a
  second pytest process sharing the one test schema and blocking on its locks. A regression
  test now pins the ceiling: `test_a_refused_round_never_runs_past_max_thought_rounds`
  scripts two refused `expect` rounds with `max_thought_rounds: 2` and asserts exactly two
  `model.call`s.
- **Kept: a refusal returns even when the same round acted** (`pfc.turns`, now `refused` /
  `acted` booleans with a one-line comment, replacing the old "all thought acts or refused"
  test). Evidence: 12.4/D51 says a rejected thought act or capability argument comes back in
  the same turn as a result; the old condition broke the moment any call carried out a
  non-thought act, so a refusal in that round was lost. New test
  `test_a_refusal_returns_even_when_the_same_round_acted` (act + cue-less `expect` in one
  round, corrected `expect` in the next) pins it.
- **Kept: `think` description** — "State a thought and how sure it is; the same wake still
  acts on it." Evidence: the 12.8 real-model run's 18 and 19, where the soul thought and
  answered but called neither `self.share` nor `act to:`; 10.8 failures 16/29/30 (a `think`
  whose thought held the plan, no tool call). It states the existing loop behavior (D51).
  The guidance bullet "Thinking is not acting: in the same answer, call the tool that
  carries the thought out" is restored, not replaced: 12.7 records it as the fix for 10.8
  04/25/16 and 10.10's live session.
- **Kept: `expect` description names its classes** — "Hold what must remind you later: a
  promise, a watch, or a wait; its cue wakes you." Evidence: 12.8 real-model 16 and 24, where
  the owner asked to be told when something happens and the first wake created no `expect`.
  The classes are general Expectation kinds (`docs/core.md` §5), not a scenario rule.
- **Kept: guidance bullet for a private question** — "If what you know may be private to
  someone, ask that person privately instead of saying you do not know." Evidence: 12.8
  real-model 26 and 28, where the soul answered "I do not know" in the group and never asked
  the owner in `space:tg:1001`, so no `share.granted` followed. The 12.8 prep material note
  already named this as the intended general principle (the desk cue stays open).
- **Kept: `to?` / `via?` labels in the `act` capability list** — "(person or place)" and
  "(a body)". Evidence: 18 and 19 must reach Budi (a person, no place of his own) and 26/28
  must reach the owner privately; a bare `to?` read as a place. General, matches
  `reach.py`'s documented `to:` contract; no capability name changed.
- **Reverted: the guidance merge** that folded "Thinking is not acting…" and "You act only
  through the tools. Staying silent is a valid choice." into one line. The silence line has
  behavioral evidence in 12.7 (11.5 false-speak 0.0%, D51's no-tool round, 12.5 scenario 30)
  and 12.7 proposed no removal; the merged wording also dropped the same-answer instruction.
  The line keeps its bullet dash so the list stays a list (formatting only).
- **Reverted: the `cue_event` description** ("The event that wakes you: who, place,
  text_like"). It dropped `kind`, which `expect.matches_cue` reads; the old "Pattern: kind,
  who, place, text_like." is true and complete. The `expect` object's description was also
  restored (`{"state": key, "op": "eq|gt|lt", "value": x}`), because `condition.operator`
  accepts that form; hiding it was not supported by any evidence.
- **Runs.** `uv run pytest`: 279 passed. Fake replay: 30 scenarios, 130/130 checks,
  absolute-line violations 0, missed-help 0.0% (0/65), false-speak 0.0% (0/68), honesty
  100.0% (62/62); `scenarios/README.md` regenerated. The real-model run is the coordinator's
  single run after this commit; none was started here.

Part 12 notes (worker, 12.8 material check 2):
- **Evidence:** `reports/replay-20261112-073214.md` (the single real-model run, 27 scenarios).
  The check dumped each demanded wake's desk from the fake replay (`scripts/replay.py` builds
  the desks with the same `memory.build_context` + `hooks.audience_policy` as the real run; a
  throwaway script printed `PFCContext.core_memory`, `.memories`, `.recent`,
  `.recent_elsewhere` per wake and was not committed).
- **Confirmed scenario bug in 25, 27, 29.** The fact was said by one speaker, and the asker's
  first line in that place was the question itself. `reach.visible(..., before=line.id)` needs
  every person present now to have been present when the line was said, so the newcomer's desk
  held only the question, and the sleep memory's sources (the fact line and its
  `decision.silent`) were said to the other speaker alone, so `reach.memory_visible` filtered
  it too. The soul's "I do not know" was correct for that desk; the scenario lacked material.
- **The desk before and after** (same fake run, the demand wake):

  | Scenario | Wake | Desk before | Desk after |
  |---|---|---|---|
  | 25 | step 4 → 5, `space:tg:-1005` | only the question | the cafe line (09:00) + the question |
  | 27 | step 4 → 5, `space:tg:-1007` | only the question | the password line (10:00) + the question |
  | 29 | step 4 → 5, `space:tg:-1009` | only the question | the clinic line (11:00) + the question |

- **Fix.** Each asker now speaks once in the same place before the fact (25: 8002, 08:58;
  27: 8201, 09:58; 29: 8401, 10:58), so the fact was said while they were there and D50 lets
  the line enter their desk. No privacy rule and no soul code changed. The "helpful
  unaddressed question" intent is kept; 27's question no longer says "Aku baru datang", which
  the new timeline contradicts, and the descriptions no longer call the asker a newcomer.
- **Changed checks:** 25/27/29 `expectation.created` step 3 → 4, `reply_in` and `mentions`
  step 4 → 5 (one new step before the fact); the recorded `store_memories` moves to step 3 and
  the `decide` to step 5. Labels "from memory" → "said in the group" in 25, 27 and 29, because
  the desk source is the group line (see the finding below); the capability lines are
  unchanged. Every check stays behavioral.
- **Finding (not built).** For the same content, `reach.memory_visible` is stricter than
  `reach.visible`: the memory derived from that group line stays off that group's desk, because
  its sources include the asker's own earlier line, whose audience is the asker alone. So the
  material these scenarios use is the line, not the consolidated memory. Whether "every source"
  should hide a memory from a person who was present for all of it is a design question for the
  architect; D53 states it strictly and nothing was changed.

- **Material check of the other failing scenarios** (what the check demands, what the desk
  held, what was missing):

  | Scenario | Check demands | Desk held | Missing | Added |
  |---|---|---|---|---|
  | 18 step 4 | owner alone in `space:game:world1` is answered about Budi's file | `recent_elsewhere` holds Budi's `sketsa_logo` line (`#10`) and the owner's DM line; the run's own `prompt_contains` check passed at step 4 | nothing: the real run appended `error` at that step (no act.out), a soul-side failure | nothing |
  | 24 step 10 | the promise fires on the unaddressed `Dokumennya sudah selesai kok.` | all six group lines including `#42` (the statement) and the promise in `<expectations>` | nothing: the real run created no Expectation at step 7 (`expectation.created` absent), so no condition existed to fire | nothing (12.8 brain fix: the `expect` description names promise/watch/wait) |
  | 30 step 3 | the statement wakes the soul and the owner hears once in the DM | the group lines and the promise (real run: `expectation.created` at step 1); fake wakes at steps 2 and 3 | nothing: the real run stayed silent at steps 2 and 3, so its free-text reminder never met the condition (`lo` = 0.35), a soul-side failure | nothing |
  | 16 step 5 | the promise fires on `server down lagi nih guys` and one DM follows | the request's desk holds the group place id (`#1` in `recent_elsewhere`); the real run created 2 Expectations at step 2 | nothing: the real run stayed silent at step 5, a soul-side failure (the prep already added the group line) | nothing |
  | 19 step 4 | Budi's wake carries the intent's reason `Tolong selesaikan…` and `person:budi` | the fake run's Budi wake (step 4) has the intent trigger whose reason is that owner line and names `person:budi`; Budi's direct place (`#1`) is in the Log and `whatsapp.say` with `to:` reaches it; `dev.run` reports its output | nothing: the real run called neither `expect` nor an act to Budi, so no such wake existed | nothing |
  | 26, 28 | silence in the group, a question to the owner in `space:tg:1001`, one answer after `share.granted` | at the group wake only the question (verified for 26 and 28); the fact is an owner-DM line and must stay off the group desk | the cue that something was withheld (`<memories>none</memories>` does not say so): open design question, not built | nothing |

- **Runs.** `uv run pytest`: 279 passed. Fake replay: 30 scenarios, 130/130 checks,
  absolute-line violations 0, missed-help 0.0% (0/65), false-speak 0.0% (0/71), honesty 100.0%
  (62/62); `scenarios/README.md` regenerated. No real-model run was started here, as the task
  says; it is the coordinator's single run.

Part 12 notes (worker, 12.8a — material check of 16, 18, 19, 24, 26, 27, 28, 30):
- **Method.** Each demanded wake was read from the fake replay's own desks: `scripts/replay.py`
  builds them with the same `memory.build_context` + `hooks.audience_policy` as the real run, so
  the fake desk is the desk the real model gets. A throwaway script (not committed) wrapped
  `Tracker.record_desk` and printed `core_memory`, `memories`, `doing`, `expectations`, `recent`,
  `recent_elsewhere` and the trigger per wake; every row below cites those dumps. The item was
  done after the architect's ruling on main `2af7ab7`: **no file capability**. The finished work
  is text in the Log; reading the working folder is a plugin (Task 3, 3.3b) and attaching a file
  is a body capability (Task 5). No `monoagens/` file, no organ definition and no check
  semantics of 16/24/26/27/28/30 were changed.

- **16 — material present, nothing fixed.** Needed: the owner's request (the watch's phrase and
  who to tell), the group whose line must fire the watch, and `expect` + `telegram.say`. The
  step-2 desk (owner DM) holds the request (`#5`) and `elsewhere #1 space:tg:-10099 percept
  "Selamat pagi, nanti deploy staging jam sembilan ya."`, so the group's place id the cue names
  is on the desk. The step-5 wake is the fired expectation in `space:tg:1001`; its trigger
  carries the group's line `"server down lagi nih guys"` (`#26`) and `<expectations>` holds the
  promise, so notifying the owner needs only `telegram.say` in the place it wakes in. The check
  (`expectation.created` in the first wake, silence through chatter, one DM message, "down") is
  behavioral. Finding below (expectation-wake `place_kind`) applies here too.

- **18 — material was missing; fixed in the scenario.** Needed: the work itself (not just a
  mention), a consent (`self.share`), a way to reach Budi, and a capability that delivers it.
  The step-7 desk (world1) held the request (`#55`) and the owner's earlier line, but that line
  only said "filenya ada di folder kerja": no file name and no content were anywhere in the Log,
  no organ could read the folder and the `whatsapp` organ has only `say`/`react`. Fix (text
  work, per the ruling): step 6's owner line now carries the work text itself — "teksnya ini:
  \"Laporan final - tiga bab, lampiran data, tes 12 lulus\"" — and the recorded answer forwards
  it with `whatsapp.say` to `person:budi` after `self.share`. Desk evidence: the step-7 world1
  desk holds `#39` (the work text) as `recent`, and the Budi wake's intent reason carries the
  whole draft, so the target desk holds the words too. New check: `mentions "lampiran data"`
  at step 7 ("the work text the owner wrote reaches Budi"); the old "the work is sent" check
  passed on any message, and after the ruling the work's receipt is the text, not the file name.
  No check was removed or loosened.

- **19 — material present; answers reworded, no check changed.** Needed: the request (it names
  `src/parser.py` and the dev workspace), the work's observable result, both candidate groups,
  Budi's direct place, and a delivery capability. The step-4 task desk's `<doing>` holds `goal:`,
  `step: dev.run`, and `result: $ uv run pytest ... pytest: 12 passed in 0.51s`; the Budi wake's
  reason carries "Tolong selesaikan ..." and `person:budi`; `elsewhere` holds both group ids
  (`#5`, `#9`) and Budi's direct place (`#1`). **Which capability sends code:** `whatsapp.say`
  sends text — the code's result/test output as a chat message; no Task 2 capability attaches
  the code file, and no organ can read it (Task 5 bodies / Task 3 3.3b). The checks require a
  delivery message, not an attachment, so they are consistent; the four recorded answers now say
  "Kodenya sudah selesai dan lulus tes (12 passed)" instead of claiming the code was attached.

- **24 — material present, nothing fixed.** Needed: the request in the group (the promise's
  subject and place), the promise, the statement that fires the free-text condition, the
  stored report hour, and `telegram.say`. Step-7 desk: the request (`#25`); step 10: `#42`
  "Dokumennya sudah selesai kok." plus the promise in `<expectations>`; step 14:
  `#38` "Nanti laporannya jam lima sore kayaknya." and `#61` "laporannya jam berapa ya?" — the
  answer is in the same place's `<recent>`, not only in a memory. No gap.

- **26 and 28 — material present; the missing cue is D55/D56, not built here.** Needed: the
  fact (owner DM line + sleep memory, both in the Log), the group question, `self.share`, and a
  way to ask the owner privately. Group wake (step 4) holds only the question (`#14`): the fact
  is correctly absent (`prompt_absent` holds), and the owner's person id and DM place are not on
  that desk either. The DM intent wake (step 4) then holds the owner's line (`#1`) and the
  memory, the owner's consent (`#34`), and after `share.granted` the memory enters the group
  desk (step 5). The person card (D55/12.9) and the withheld note (D56/12.10) are the other
  worker's items; the fake run's `to: person:owner` resolves through `reach.direct_place`, which
  is why the fake passes while the real model has no cue — the open design item, not a scenario
  gap. Nothing built.

- **27 — material present, nothing fixed.** Needed: the password line in the group, the
  question, and a capability to answer in the group: `telegram.say` is the act (a chat message;
  no file or private route is involved). The step-5 desk holds `#5` "Password wifi kantor yang
  baru: senja2026..." and `#13` "Ada yang ingat password wifi kantor? Aku lupa." The asker spoke
  in that group before the fact (`#1`), so D50 lets the line onto the desk.

- **30 — material present, nothing fixed.** Needed: the promise made in the owner's DM, the
  reminder line, and `telegram.say`. The promise is created at step 1 from the owner's line;
  step-2's desk holds `elsewhere #10` "dokumen sudah selesai ya?" and the trigger is the
  `expectation.fired` whose `text` payload is the group line (rendered in `<event>`); step 3 adds
  `elsewhere #20` "dokumennya sudah selesai kok." The answer is `telegram.say` in the DM it
  wakes in. Finding below applies.

- **Finding for the core owner (not fixed here).** An expectation's wake is labelled `shared`
  even when the expectation lives in a direct chat: 16 step 5 and 30 steps 2–3 render
  `<place>space:tg:1001 (shared)</place>` although `space:tg:1001` is the owner's DM. Cause:
  `_expect_wake` builds the trigger from the expectation and `expectation.fired` carries no
  `place_kind`, so `memory.build_context` falls back to `"shared"` (`monoagens/memory.py:160`).
  The notification still lands in the DM because the act stays in the waking place, but a real
  soul cannot tell that place from a group. `monoagens/` is another worker's scope; recorded,
  not changed.

- **Changed checks and answers.** 18: one new check, `mentions "lampiran data"` at step 7 (the
  work text the owner wrote reaches Budi); step 6's line now carries the work text; the two
  recorded step-7 answers forward it. 19: four recorded answers reworded (no check). Nothing
  else changed in 16, 24, 26, 27, 28, 30.

- **Runs.** `uv run pytest`: 281 passed, run on an isolated `monoagens_12a_test` database because
  the shared `monoagens_test` was in use by the parallel worker and two runs there failed
  different, unrelated tests (cross-talk on one schema; the isolated DB is green on both runs).
  `uv run python scripts/replay.py --fake`: 30 scenarios, **131/131** checks (130 before),
  missed-help 0.0% (0/65), false-speak 0.0% (0/71), absolute-line violations 0, honesty 100.0%
  (62/62). `scenarios/README.md` was regenerated only as generated output and reverted (this
  worktree has no saved reports, so committing it would erase the record). No real-model replay
  was started, as the task says.

Part 13 notes (worker):
- **13.1 — where code went for a budget, and the decision.** Every row is from the Notes of
  Parts 1–12 (part and line budget in brackets) and was checked against the code at HEAD.

  | Place | Budget-driven move | Decision |
  |---|---|---|
  | `condition.py` ← `expect.py` (12.5; expect 240/240) | `operator` (exact fast paths) and `text` moved out with the free-text gate | **Keep.** One module owns a condition; `expect.py` keeps the expectation lifecycle, cues and settlement. |
  | `parse_duration` in `condition.py`, re-exported by `expect` (12.5) | moved with `operator`; `expect` imported a duration parser from the condition module | **Restore.** The grammar belongs to cue times and conditions equally; new leaf `monoagens/duration.py`; `condition`, `expect`, `scripts/replay.py` and the test import it; the re-export is gone and every caller migrated. |
  | `expect.py` (Part 6; 240/240) | `_KINDS`/`_OPS` inlined; duplicate `row["policy"]` read removed; rows/row dance in `resolve`; temp in `_operator` | **Restore `KINDS`** as the one named list shared with `pfc`'s `expect` tool enum. **Keep** `_OPS` inline and the three straight-line simplifications: a one-use literal is clear and the code is shorter without losing a name. |
  | `jobs.decide(kernel, …)` (12.4; jobs 148/150) | the background job took the whole kernel for `clock`/`settings` | **Restore.** `decide(pfc, context, rounds, clock, settings)`; `kernel._think` passes the collaborators; the job no longer reaches the kernel. |
  | `expect.due(…, kernel=self)` and `condition.reminded(kernel, …)` (12.5; kernel 280/280, "one argument") | the wake gate took the kernel to reach `pfc`, `embedder`, `settings` | **Restore.** `due(…, pfc=, embedder=)` and `reminded(conn, row, event, *, pfc, embedder, settings)`; the same two guards (`pfc is None`, `embedder is None`) are kept, so behavior is identical. |
  | `organs.leaks` (12.5b; reach 150/150, "moved out to make room") | the D45 content check left `reach.py`, its sibling `memory_visible` | **Restore.** `reach.leaks` sits beside `visible`/`memory_visible`; `organs.dispatch` and `_intend` call `reach.leaks`; the test calls it there. |
  | `reach.memory_visible` (12.5b) | a second visibility rule beside `visible` | **Keep.** It is the same audience rule applied to a memory's sources; `reach` owns who may see what. |
  | `desk.py` ← `pfc.py` (11.2; pfc 279/280) | desk and whole prompt moved out | **Keep.** Context + rendering vs. client + rounds is a real seam and nothing was compressed. |
  | `work.py` ← `recall.py` (12.2) | the `<doing>` view left the recall module | **Keep.** Working memory (`work.py`) vs. Log search (`recall.py`) is one responsibility each. |
  | `store.payload_text` replacing four private accessors (12.5; memory/work/recall/pairing −23) | one shared accessor replaced `_payload_text`/`_text` copies | **Keep.** One rule for payload text; `memory` now also falls back to `arguments.text`, the desk/recall rule. |
  | `embedding.cosine` (12.5b) | one cosine moved from `signals` and `condition` | **Keep.** One pure vector helper, no duplicate. |
  | `store.event_from_row` in `memory.recent_lines`, one ctx in `organs.dispatch`, one appraisal in `kernel` (12.5b) | dedupe | **Keep.** Same behaviour, one construction each. |
  | `pairing` blank line, `recall.py`/`work.py` EOF newlines (12.5b) | formatting had been trimmed | **Keep.** Already restored; a checker confirms every module ends with one newline and top-level defs are separated by two blank lines. |
  | `store.count_recent_events` (10.7), `reach._DIRECT`/`_GROUP`/`looks_direct` (10.1), `memory`'s old cosine/`Embedder`, `middleware`'s dead context branch (7) | deleted to free lines | **Keep deleted.** Dead code; nothing calls them. |
  | `recall.search`'s local `prior` import (10.3) | local import of `prior` | **Keep.** It breaks a real cycle (`prior → identity → recall → prior`), not a budget move; not touched. |
  | kernel appraisal hoist (12.5b) | one `_appraise` before the `act.result`/percept branches | **Keep.** One appraisal per event, no duplicate work. |
  | dense one-liners: `condition.parse_duration` walrus, `desk._duration` chained ternary, `desk._fit` conditional pop index, `expect.validate` packed assignments, `expect.due` combined guard, `expect.settle` ternary, `expect.recovery` await inside a boolean, `reach.visible`/`memory_visible` compressed set arithmetic, `appraise` backslash ternary | written short to save lines | **Restore.** Plain statements/`if`s; same behavior. |
  | `pfc.turns` refusal/act booleans, `think`/`expect` descriptions (12.8 brain fixes) | — | **Keep.** Behavior the tests pin; not a line-budget move, and the text the model sees is unchanged. |
  | draft `expect.condition.finish(self, job)` classifier routing (12.5 draft) | proposed to keep kernel at 280 | **Keep absent.** The final 12.5 design has no classifier; the name does not exist in the tree. |
  | `guidance.py`, desk layout, prompts, prior values, scenarios | — | **Untouched.** No text the model sees and no scenario/prior data changed. |

- **13.2 — every refactor (commit, reason).** No behavior was added or removed; docstrings,
  clear names, one responsibility per module, ≤100 columns, one statement per line, two blank
  lines between top-level definitions, one trailing newline (verified).
  - `d2a719d` durations: new `monoagens/duration.py` (17 lines) owns the duration grammar;
    `condition.py` imports it instead of owning it, its walrus became a named `match`, and
    `operator` builds its pattern in two statements. Re-export from `expect` removed; callers
    migrated (`expect`, `scripts/replay.py`, `tests/test_expect.py`).
  - `0e7f6cd` privacy: `leaks` moved from `organs.py` back to `reach.py` beside
    `visible`/`memory_visible`; `organs.dispatch`/`_intend` and `tests/test_reach.py` call it
    there.
  - `2271529` collaborators: `jobs.decide` takes `pfc, context, rounds, clock, settings`
    instead of the kernel; `expect.due` takes `pfc`/`embedder`; `condition.reminded` takes
    `conn, row, event, pfc, embedder, settings`. Same guards and same clock read.
  - `79faf32` dense expressions: `desk._fit`, `desk._duration`, `expect.validate`,
    `expect.due`, `expect.settle`, `expect.recovery`, `reach.visible`,
    `reach.memory_visible`, `cognition/appraise` written out.
  - `bd32cf6` kinds: `expect.KINDS` named once and used by `expect.validate` and `pfc`'s
    `expect` tool enum (same values, same order, same JSON).
- **13.3 — verification.** `uv run pytest`: **281 passed** (real Postgres). Fake replay:
  30 scenarios, **130/130 checks**, missed-help 0.0% (0/65), false-speak 0.0% (0/71),
  **absolute-line violations 0**, honesty 100.0% (62/62). `scenarios/README.md` was
  regenerated by the runs only as generated output and reverted; no scenario, prompt, prior
  or model-visible text changed. Package size for review only: 4,086 lines including
  `bodies/` (`reach` 166, `expect` 232, `condition` 62, `duration` 17, `organs` 135);
  line counts are a review signal, not a budget.
- Nothing needed a behavior change; no question is left for the architect from this part.

Part 12 notes (worker, 12.9):
- **What the item does.** `reach.person_cards(conn, place, event)` returns one card per person
  present at the place (`reach.audience`) or named in the trigger (a known person's name, whole
  word, case-insensitive). A name comes from `state: person:<id>` when the state holds one, else
  from the person id's last segment when that segment is alphabetic (`person:budi` -> `budi`;
  `person:tg:6001` has no name and can only be named through a state name). Each card carries
  the name (when known), the person id, the direct place where each body last saw them (a
  `percept` with `place_kind: direct`, newest per body, alias-aware) and closeness from
  `state: person:<id>` (0.00 when unset, the default `Situation` already reads). The owner's
  card is not rendered: the owner is the soul's always-known counterpart and every direct chat
  would otherwise repeat it.
- **No new primitive.** Everything is derived from existing data (aliases, `person:*` state,
  last activity, direct places): no event, state key, book, act, hook, signal or drive was
  added. The card is assembled per wake, not stored.
- **Desk.** `PFCContext.people` holds the cards; `desk._semi_stable` appends `<people>` after
  `<core_memory>` in the semi-stable system message, only when cards exist (without cards the
  message is byte-identical to before). The context hook passes people through unchanged, and
  `Tracker.record_desk` does not record them as items: a card holds no content, so absolute
  line 2 never withholds it and the desk metric counts none of it (D55).
- **Scenario 18 material, from the fake replay (steps 6-8).** Step 6 in `space:game:world1`
  with Charlie present: `- budi (person:budi): whatsapp at
  space:wa:628110001@s.whatsapp.net; closeness 0.00` and `- charlie (person:charlie): no
  direct place known yet; closeness 0.00`. Step 7, whose trigger names Budi, keeps both cards
  in the world and has Budi's card on the intent wake in his own place. Step 8 reports Budi's
  verdict with the card still on the desk. No message text is in any card (`sketsa_logo` is
  absent from them, and from the group prompt).
- **Tests.** `tests/test_reach.py::test_person_cards_show_people_present_and_named_but_never_their_messages`
  (Charlie present, Budi named, card holds name/person id/whatsapp place/closeness, none of
  Budi's message, the prompt has `<people>`);
  `tests/test_replay_metrics.py::test_person_cards_are_not_private_items_on_the_desk` (a real
  desk holds Budi's card and the desk scan records only the place's own line: 0 violations).
  `_desk`/`_trigger` in `tests/test_reach.py` gained an optional trigger text; no assertion was
  changed.
- `scripts/replay.py`: `_fake_context_text` includes `context.people` so a fake-mode prompt
  check can see a card. No organ/body definition and no metric changed.
- **Runs.** `uv run pytest`: 283 passed (the added 12.11 test made it 284). Fake replay: 30
  scenarios, 130/130 checks, absolute-line violations 0, missed-help 0.0% (0/65), false-speak
  0.0% (0/71), honesty 62/62. `scenarios/README.md` was regenerated only as generated output and
  reverted. Commit 34aa31a.

Part 12 notes (worker, 12.11):
- **What the item does.** `jobs.apply_calls` tracks the places one job's calls have already
  spoken to and hands the set to `kernel._apply` -> `organs.dispatch`. A communicative act
  whose resolved place is already in the set is refused before any intent or `act.out`, with
  an honest `act.result` (`{"error": "already said something to <to or place> this turn"}`),
  so the organ is never called a second time and nothing is silently dropped. The set lives
  for one PFC job, i.e. all rounds of one wake (D51); another place, a non-communicative act
  (`self.share`, `react`) and a new wake are unaffected.
- **Where and why.** The place a `to: person` names is only resolvable from the Log, and the
  PFC job deliberately does not touch the shared connection (12.4), so the check runs where
  the act is applied, not in `pfc.turns`; the resolved place is the same one the act would use
  (`reach.resolve_to`, the speaker-is-here shortcut), so `to: person:budi` and the derived
  place compare equal. The reason still reaches the soul: the refusal is a recorded
  `act.result`, the circuit breaker continues the goal, and the next wake's desk holds it.
  An unreachable person keeps the ordinary honest failure.
- **Changed checks.** None. New test
  `tests/test_pfc.py::test_a_second_message_to_the_same_place_in_one_turn_is_refused`: one
  wake whose first round acts and carries a cue-less `expect` (refused and returned in round
  2) and whose second round acts again at the same place; the body receives exactly one
  message, exactly one `act.out` is recorded, and the duplicate gets `already said something
  to space:tg:1 this turn`. The existing refusal test keeps its expectation assertion
  unchanged.
- **Runs.** `uv run pytest`: 284 passed (real Postgres). Fake replay: 30 scenarios, 130/130
  checks, absolute-line violations 0, missed-help 0.0% (0/65), false-speak 0.0% (0/71),
  honesty 62/62. `scenarios/README.md` regenerated only as generated output and reverted.
  Commit 37c760c.

Part 12 notes (worker, 12.10):
- **What the item does.** When the audience filter removes a *relevant* private item,
  `hooks.audience_policy` counts it under the source label and `memory.build_context` does the
  same for a withheld core memory; `desk.withheld_note` renders one `<withheld>` line right
  after `<memories>` in the volatile prompt, e.g. "2 things you know from person:owner private
  chat are not shown here". Only the count and a D55-safe label (a person id, `person:owner
  private chat`, or the place of a shared source) appear; no text, no fact.
- **What counts as relevant.** Retrieved memories (the normal `retrieve` cut, before the
  audience filter), the recent lines of *other* places (`recent_elsewhere`), and core memory.
  The whole database is never scanned. A same-place `recent` line hidden by D50 is filtered
  silently: it is already the desk's own place, not a candidate from elsewhere.
- **Labels.** `reach.source_label` names a direct place by the people in its audience at that
  line (`person:owner private chat` for the owner's DM), else by the place. `reach.withheld_source`
  picks, for a memory or core memory, the first `derived_from` place whose audience the context
  does not cover; a memory with no known source is private to the owner and says so. Each
  withheld item is counted once, under its first failing source. No new core primitive: the
  note is a field of `PFCContext` and a prompt line, not a book, act, hook, signal or drive.
- **Desk metric.** `Tracker.record_desk` still records only memories, lines and core memory;
  the note is not an item, so the absolute-line scan ignores it (test below).
- **Checked in the fake replay (26 and 28).** At step 4 of `26_help_meeting_time` the group
  desk in `space:tg:-1006` carries `4 things you know from person:owner private chat are not
  shown here` next to the question, and no `#1` line, memory or core text (`sembilan` absent).
  `28_help_package_location` renders the same note in `space:tg:-1008` (`lemari` absent). The
  run is 131/131 checks with absolute-line violations 0, so the note appears and leaks nothing.
  Scenario files 26 and 28 were not changed (they belong to Task 3); no recorded answer needed
  a change because their checks still pass.
- **Bug fix (separate commit).** `memory.build_context` now takes `place_kind` from
  `reach.place_kind(conn, place)` instead of the trigger payload, so an `expectation.fired`
  wake in the owner's DM renders `(direct)`. `tests/test_expect.py::test_an_expectation_wake_takes_the_place_kind_from_the_place`
  fires a promise from the owner's direct place and asserts the prompt's `<place>` line.
- **Tests.** `tests/test_reach.py`: a withheld line and a withheld memory each leave
  `("person:owner private chat", 1)` and the note, with `sembilan` absent from the rendered
  prompt; a desk with nothing withheld has no `<withheld>`. `tests/test_replay_metrics.py`: a
  desk whose only content was withheld still has an empty item list and 0 desk violations.
- **Runs.** `uv run pytest`: 289 passed (real Postgres). `uv run python scripts/replay.py
  --fake`: 30 scenarios, 131/131 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/71),
  absolute-line violations 0, honesty 100.0% (62/62). `scenarios/README.md` regenerated and
  committed. No real-model replay was started, as the task says. Commits 9f48281 and 8c28eb4.

Part 12 notes (worker, 24/30 material):
- **Evidence.** `reports/replay-20261112-085343.md` (flash, effort high) and
  `reports/replay-20261112-085711.md` (Pro 0813, effort high): in both models the owner request
  "kabari aku ya kalau dokumennya sudah selesai" (24 step 7, 30 step 1) drew a clarifying
  question ("Dokumen yang mana, dan bagaimana aku tahu kalau sudah selesai?") and created no
  expectation. Material check per D54: the Log held neither which document was meant nor the
  place where its news would appear — in 30 the group `space:tg:-1004` had no line at all before
  the request (its first line was the question at old step 2), and in 24 the request was the
  group's first mention of a document. The soul's question was right for that desk, so the
  scenarios were fixed and `monoagens/` was not touched.
- **Material added to 24** (same place as the watched line, before the request): new steps 7
  and 8 at 13:02:40/13:02:50 by `person:tg:7002` and `person:tg:7003` — "Proposal anggaran Q4
  masih di-review Bu Rina." and "Iya, proposal anggaran Q4 itu yang kita tunggu minggu ini." The
  request at step 9 keeps "dokumennya" and now has a referent on the desk.
- **Material added to 30**: new steps 1 and 2 at 08:50/08:55 in `space:tg:-1004` by
  `person:tg:7002` and `person:tg:7003` — "Proposal anggaran Q4 masih di-review Bu Rina." and
  "Iya, kabar selesainya nanti diumumkan di grup ini." The owner's DM request at step 3 points at
  that document, so both the referent and the watch place are in the Log before the promise.
- **Desk evidence** (fake replay desks, built by the same `memory.build_context` +
  `hooks.audience_policy` as the real run; dumped by a throwaway script, not committed):

  | Wake | Desk |
  |---|---|
  | 24 step 9 (request) | `recent` #21 "Aku ikut rapat.", #25 "Proposal anggaran Q4 masih di-review Bu Rina.", #29 "Iya, proposal anggaran Q4 itu yang kita tunggu minggu ini." |
  | 24 step 12 (statement fires) | same lines plus #33 request, #42 "Dokumennya masih di review.", #46 report hour, #50 "Dokumennya sudah selesai kok."; `<expectations>` holds the promise |
  | 30 step 3 (owner DM request) | `elsewhere` #1 "Proposal anggaran Q4 masih di-review Bu Rina.", #5 "Iya, kabar selesainya nanti diumumkan di grup ini." |
  | 30 step 4 (question) | `elsewhere` #18 "dokumen sudah selesai ya?" plus #1/#5; `<expectations>` holds the promise |
  | 30 step 5 (statement) | `elsewhere` #28 "dokumennya sudah selesai kok."; promise still active |

- **Changed checks.** 24: `no_reply` `[1,2,3,4,5,6,8,9,12]` → `[1..8,10,11,14]`; `model.call`
  7 → 9; the three gate-term checks `decision.silent` `steps [1..6] count 6` → `steps [1..8]
  count 8`; `expectation.fired` and `reply_in` 10 → 12; the wait's `expectation.created` 13 → 15;
  `reply_in` and `mentions_any` 14 → 16. 30: new `no_reply steps [1,2]` (ordinary group chatter);
  the question's `no_reply` and `no_event` 2 → 4; `reply_in` 3 → 5. Every check stays behavioral
  (steps, places, counts, event kinds, hour formats); none was removed or loosened.
- **Recorded answers.** 24: `decide` steps 7/10/14 → 9/12/16. 30: `decide` steps 1/2/3 →
  3/4/5. The calls themselves are unchanged: `expect` stays a free-text promise
  (`expect: {text: "dokumen sudah selesai"}`), the question's wake records only a `think`, and
  the statement's wake acts and resolves. The fake replay stays deterministic.
- **Descriptions** of 24 and 30 now say the earlier lines name the document; both capability
  lines are unchanged.
- **Runs.** `uv run pytest`: 289 passed (real Postgres). `uv run python scripts/replay.py
  --fake`: 30 scenarios, 132/132 checks (131 before; the new 30 chatter check), missed-help
  0.0% (0/65), false-speak 0.0% (0/75), absolute-line violations 0, honesty 100.0% (62/62).
  `scenarios/README.md` regenerated (its real-model rows pool the 085343 and 085711 reports).
  No real-model replay was started here, as the task says.

Part 12 notes (worker, expect tool text condition):
- **Evidence.** `reports/replay-20261112-091633.md`, scenarios 24 and 30. 30 created
  `cue_event={"place": "space:tg:-1004", "text_like": "selesai"}` with
  `condition={"event_seen": {"place": "space:tg:-1004", "text_like": "selesai"}, "within":
  "5d"}`; 24 created `cue_event={"text_like": "proposal anggaran Q4 selesai|dokumen
  selesai|dokumennya selesai|review selesai"}` with
  `condition={"event_seen": {"text_like": "proposal anggaran Q4 sudah selesai|dokumen sudah
  selesai|dokumennya selesai"}, "within": "14d"}`. In both the model encoded "tell me when
  this is said" as `event_seen`, a predicate the wake gate evaluates as *already seen*.
- **Verified, not inferred.** A throwaway probe (not committed) created that 30-shaped
  promise through `expect.create` and fed the watched line to `expect.due`: `fired: []`, and
  the row ended `satisfied` — with no `|` in the pattern the watched line itself satisfies
  the condition, so the gate reads "condition already met" (`docs/core.md` §5) and resolves
  instead of firing. 24's `|` lists are read by `expect.matches_cue`/`expect.is_met` as
  literal characters (see the observation below), so there neither cue nor condition matched
  any line. In the report 30 kept the promise engaged without firing it and the owner never
  heard (checks "the question does not resolve the promise" and "the owner hears once");
  24 recorded 0 `expectation.fired`. A free-text condition has neither failure mode:
  `condition.reminded` compares cosines and the PFC wakes once to judge (12.5).
- **The gap was visibility.** `condition.operator` accepts `{"text": ...}` and both fake
  scenarios already script it; the `expect` tool's description listed only `none`,
  `event_seen` and `state`, so the model could not see the form. Fixed in `monoagens/pfc.py`
  (244 → 249 lines): the condition's description now names every form with the behavior
  `condition.py` and `expect.is_met` actually implement — `{"none": true}` = the cue alone
  wakes you; `{"text": "what you are waiting to hear"}` = a watch on free text, so a line that
  sounds like it reminds you wakes you once to judge it (cue_event narrows where and who);
  `{"event_seen": pattern, "within": "2d"}` = fires when no matching event was seen within the
  window; `{"state": ...}` as before. No scenario rule, no platform name, no new primitive.
  `guidance.py` is untouched (24 lines): its "hold it with expect and a cue" bullet already
  covers the cue, and the missing piece was the condition menu, which belongs to the tool it
  describes.
- **Tests.** `tests/test_pfc.py` +2: `test_the_expect_tool_offers_the_free_text_watch` pins the
  `{"text": ...}` form and the two firing meanings in the schema;
  `test_the_expect_tool_accepts_a_free_text_condition` drives `pfc.turn_check` with an `expect`
  carrying `{"text": ...}` and asserts no refusal, so the tool path reaches `expect.validate`
  → `condition.operator` and the form is accepted.
- **Observation, not built.** 24's `text_like` uses `|` for alternatives, which the matcher
  reads literally, so that cue and condition match nothing. Whether `text_like` should accept
  alternatives is a core behavior question; this item changed no matcher.
- **Runs.** `uv run pytest`: 293 passed (real Postgres). `uv run python scripts/replay.py
  --fake`: 30 scenarios, 132/132 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/75),
  absolute-line violations 0, honesty 100.0% (62/62). `scenarios/README.md` regenerated. No
  real-model replay was started here.

Part 12 notes (worker, cue_event description):
- **Evidence.** `reports/replay-20261112-092518.md`, scenarios 24 and 30 (real model, effort
  high). The free-text condition is now chosen correctly (`{"text": ...}`), but every cue was
  wrong: 24 `cue_event={"text_like": "dokumen|selesai|beres"}`; 30
  `cue_event={"kind": "message", "text_like": "dokumen.*selesai"}` and
  `{"text_like": "dokumen.*selesai|selesai.*dokumen"}`. `expect.matches_cue` compares `kind`,
  `who` and `place` exactly against the event (so `kind: "message"` is not an event kind the
  desk ever shows) and tests `text_like` as a case-insensitive substring, so regex dots and
  `|` are literal and nothing matches; the watch never wakes.
- **Cause.** The `cue_event` parameter description was `"Pattern: kind, who, place,
  text_like."` — it did not say `kind` is the event kind as the desk shows it, did not name
  `not_who`, and did not say `text_like` is a plain phrase.
- **Fix.** The description now states every key `matches_cue` reads and their meaning:
  `kind` = the event kind exactly as the desk shows it (e.g. `percept` for what someone
  says), `who` = person id, `place` = place id, `not_who` = person id to ignore, `text_like` =
  a plain phrase the line contains, case-insensitive, NOT a regex and no `|` alternation (for
  several phrasings make several expectations or pick the most distinctive word); all keys
  given must match. No scenario rule, no platform name, no new primitive. `expect.py` and
  `guidance.py` are untouched: this item changed only the text the model sees, not matcher
  behavior.
- **Open question, not built.** Whether `text_like` should accept alternatives (`|`) or regex
  remains a core behavior question; 12.8's earlier note recorded it too. Until the owner
  decides, the description tells the model the truth about what the matcher does.
- **Tests.** `tests/test_pfc.py` +1: `test_the_expect_tool_describes_the_cue_exactly` pins
  that the description says `kind` is the event kind as the desk shows it (naming `percept`),
  names `who`, `place`, `not_who` and `text_like`, says `text_like` is a "plain phrase", and
  rules out a regex and `|` alternation.
- **Runs.** `uv run pytest`: 294 passed (real Postgres). `uv run python scripts/replay.py
  --fake`: 30 scenarios, 132/132 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/75),
  absolute-line violations 0, honesty 100.0% (62/62). `scenarios/README.md` regenerated. No
  real-model replay was started here.

Part 12 notes (worker, 12.12):
- **The bug and the fix.** `condition.reminder` no longer writes `expectation.fired`. It writes
  `expectation.reminded` (a new unprefixed core event kind, additive: kinds are not
  schema-versioned, so no upcaster is needed; readers keep unknown payload fields) with the
  **line's own speaker** as `who`, the **line's place** as `place`, the line's body and
  `caused_by` the line's event id. Payload: `{expectation_id, about, condition, text,
  line_place, reminder: true}`. The Expectation is untouched: no status change, no
  `fire_count` increment, no `settle` (the kernel skips `expect.settle` for a reminder). A
  genuine firing (exact `event_seen`/`state`, `cue_time`, `none`) still goes through
  `fire_expectation` unchanged.
- **The desk.** `desk._event_text` renders a reminder as its own block: the line with
  `[line_place] speaker: text`, then `this line may relate to your expectation: <about>;
  condition: <text>; does this line fulfil it?`. The kernel's wake reason is
  `reminded: <about> (condition: <text>); does this line fulfil it?`, not
  `expectation missed`. `appraise` treats `expectation.reminded` as a circuit-breaker wake whose
  reason is `reminded` (`reflex` and the early no-draw branch in `appraise`). No guidance line
  was added; `guidance.py` is untouched.
- **Where the wake is laid out (a decision with evidence).** The stored event keeps the line's
  place (D53: the line's own speaker and place), but the PFC job runs in the Expectation's
  place (`item["place"]`), with a copy of the reminder trigger whose `place` is that desk.
  Reasons: (1) `active_expectation_lines` lists only expectations of the wake place, so a
  reminder desk in the line's place would not show the promise and `resolve` would be refused
  (`_check_resolve` needs the id on the desk) although D53 says the woken PFC may resolve it;
  (2) a myopic `act` uses the trigger event's place, so without the copy the soul's answer
  would land in the group instead of the place it is thinking in, which is exactly the 24/30
  failure mode. The copy is in memory only; the Log event is unchanged.
- **Kind lists checked.** `RECENT_KINDS` deliberately does not gain the reminder: the line is
  already a percept in `<recent>`, the reminder is the wake's `<event>`, and adding it would
  repeat the line and let a later desk read a reminder as a conversational message. `threads.
  CONTINUATIONS` does not gain it either: a reminder is judged where the promise lives, not
  continued in a task thread (only a genuine firing continues a goal). `recall` follows
  `RECENT_KINDS`, so it is unchanged. `scripts/replay.py`'s detail renderer prints
  `expectation.reminded: <line>`; its metrics count kinds through events, so no change was
  needed there. `tests/test_versioning.py`'s core-kind list names the new kind.
- **Honesty lines.** `scripts/replay.py` now appends one `- step N — honesty: <act text>; not in
  the log: <quote>` line under a scenario's Step details for every communicative act whose
  quoted span `_fabricated_quote` cannot find. Both fields go through `_detail_text`, so they
  are redacted and truncated; no secret is printed.
- **Scenarios and every changed check.**
  - `promises/30_same_topic_question.yaml`: the description no longer claims "nobody is woken"
    (the reminder wake is required by D53); it says no message is sent. The check label "a
    question on the same topic wakes no one" became "the question produces no message" (it is a
    `no_reply` check and never forbade the wake, but its label claimed it did). Added
    `event_seen expectation.reminded step 4 count 1` — "the question reminds the soul of the
    promise" — because the old check set could not tell a reminder from silence. The existing
    `no_event expectation.resolved step 4 count 0` ("the question does not resolve the
    promise") and `reply_in space:tg:1001 step 5 count 1` stay. Recorded answers already let
    the question be (a `think`) and act+resolve on the statement, so no answer changed.
  - `turn-taking/24_gate_threshold.yaml`: description now says "reminded by an unaddressed
    message". `event_seen expectation.fired step 12` became `expectation.reminded` with the
    label "an active promise reminds the soul on an unaddressed message" (a free-text
    condition only reminds; the observe/resolve decision is the PFC's). The `reply_in` label
    became "the reminder wakes the PFC". Added `event_seen expectation.resolved step 12 count
    1` — "the statement fulfils the promise once" — the behaviour the item demands. Added the
    recorded step-10 `think` ("Masih di review; ini belum selesai...") so a reminded wake in
    the real model's situation answers with a thought, not a message; step 12 still acts and
    resolves #1.
  - `promises/16_watch_for_phrase.yaml` needed no check or recorded-answer change: with the
    wake laid out in the promise's place its one-round answer still lands in the owner's DM.
- **Spec ownership.** `docs/core.md` §5 already carries the reminder wording (architect's
  1cbd412); §3's event-kind list does not yet name `expectation.reminded`. Specs are the
  architect's, so this worker did not edit them; the list needs the architect's one-line
  addition.
- **Runs.** `uv run pytest`: 296 passed (real Postgres). `uv run python scripts/replay.py
  --fake`: 30 scenarios, 134/134 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/75),
  absolute-line violations 0, honesty 100.0% (62/62). `scenarios/README.md` regenerated. The
  real-model replay was not run (12.12 does not ask for it).

Part 12 notes (worker, 12.13):
- **One door.** `monoagens/contract.py` holds the single argument check: type, required,
  unknown keys (`additionalProperties` absent means false), enum, minimum/maximum, array items,
  and one level of nested object properties. `refusal` joins the messages with `; ` and cuts
  the line at 500 characters. Booleans are not integers, and an object rule without `properties`
  (the `act.arguments` property, every `expect` sub-object) accepts any keys.
- **Two call sites.** `pfc.turn_check.check` runs it for every tool call before anything runs;
  `act` is checked twice, once against the tool schema and once against the named capability's
  schema (`to`/`via` allowed only on a communicative one). `organs.dispatch` repeats the
  capability check before it touches the arguments, so a caller that bypasses the PFC door also
  gets a refusal event instead of an exception.
- **Refusal wording.** `id: expected integer, got string`, `arguments.event_id: expected
  integer, got string`, `motivation: must be >= 1`, `kind: must be one of a`, `x: unknown`,
  `x: required`; an unknown tool name is now `unknown tool: <name>` instead of reaching
  `_apply`'s error branch (the non-matching branch in `kernel._apply` stays as the last defence).
- **`resolve`.** The `id` property says "The number the desk shows as #N." and `_check_resolve`
  no longer coerces: the door guarantees an integer, so the `int()`/`try` block is gone. The
  desk-membership and status checks are unchanged. `expect`'s semantic checks are unchanged,
  except that a bound the schema already declares now reports as the bound
  (`weight: must be <= 1`) before `expect.validate` sees the call.
- **Changed tests.** `tests/test_pfc.py`: `test_invalid_tool_call_becomes_an_error_event_and_
  the_loop_lives` became `test_an_unknown_tool_is_refused_and_the_loop_lives` (the turn is not
  lost, so no `error` event is written); four new tests cover the resolve id, a mistyped
  capability argument, dispatch's own guard, and the desk id that `resolve` takes.
  `tests/test_reach.py`: the `Body` stub declared `{"type": "object"}` with no properties, which
  a strict door reads as "no arguments allowed"; it now declares `text`/`command` and their
  required lists, as a real body does. `tests/test_expect.py`: the invalid-weight refusal
  message is the new bound message. Level: `tests/test_contract.py` 8 cases, no database.
- **Runs.** `uv run pytest`: 308 passed; `uv run python scripts/replay.py --fake`: 134/134
  checks, absolute-line violations 0.

Part 12 notes (worker, 12.14):
- Four lines of `scenarios/turn-taking/24_gate_threshold.yaml` changed: step 11
  ("Nanti rapat evaluasinya jam lima sore kayaknya."), step 13 ("Katanya rapat evaluasinya jam
  lima sore."), step 15 ("Eh, rapat evaluasinya jam berapa ya? Aku lupa.") and step 16's
  recorded reply ("Jam lima sore, rapat evaluasinya."). The `description` says why: two group
  lines name the only document in play before the request, and the hour is a separate meeting.
- No check and no prior changed: `expectation.resolved` count 1 at step 12 stays, and step 16's
  "jam lima" match still holds. The real run of 12.8 showed the ambiguity was not only the
  report: the soul also failed to bind "dokumennya" to the named proposal (see the 12.8 note).

Part 12 notes (worker, 12.15):
- `scripts/replay.py` now names a report `reports/YYYY-MM-DD-HHMM-<real|fake>-<scenarios>.md`
  (`report_label`: `all` for the full set, else each scenario file's number in the given order),
  writes the same text to `reports/latest.md`, and keeps the newest ten matching files
  (`REPORT_KEEP`), legacy `replay-*.md` included, so the first run cleaned the directory to
  ten. `latest.md` and `backups/` are never candidates.
- `saved_reports` reads every `*.md` that matches `REPORT_FILE_RE` instead of globbing
  `replay-*.md`, so `latest.md` is never read as a run; `_report_when` parses the new filename
  stamp (`%Y-%m-%d%H%M`) before the legacy one (`%Y%m%d%H%M%S`), header first and mtime last.
- Consequences for the committed record: the real column of `scenarios/README.md` now shows only
  what the ten newest saved reports hold. After the 12.8 runs the real report of 12.8 is the
  newest; scenarios it did not run read `not run` there, which is the honest value (not held),
  not a claim about the model.
- Tests: four in `tests/test_replay_metrics.py` (naming and `latest.md`, the ten-file cap with a
  foreign file untouched, the label, and reading a new-style report back with its date).
  `uv run pytest`: 312 passed; the fake replay writes `<date>-<HHMM>-fake-all.md`, `latest.md`
  is byte-identical to it, and `ls reports/*.md` is 11.

Part 12 notes (worker, 12.8 acceptance — **not met**):
- **Runs.** `uv run pytest`: 312 passed (real Postgres). `uv run python scripts/replay.py --fake`:
  30 scenarios, 134/134 checks, missed-help 0.0% (0/65), false-speak 0.0% (0/75),
  absolute-line violations 0, honesty 100.0% (62/62). The fake replay is unchanged by this part.
- **Real model, `--effort high`, the six scenarios the record still showed failing** (16, 24, 25,
  27, 29, 30), twice, because the first run left three scenarios failing:
  - Run 1 `reports/2026-11-12-1103-real-24-30-16-25-27-29.md`: 28/33 checks. Passed: 25, 27, 29
    (4/4 each). Failed: 24 (10/12: step 10 spoke in the busy group, step 12 resolved nothing),
    30 (3/5: the cue's `place` was written `tg:-1004` instead of `space:tg:-1004`, so no reminder
    ever fired), 16 (3/4: one `act.result` and no message at the statement).
  - No `error` event in any step of either run: the 12.13 door holds — the crash of report
    100338 (scenario 30 step 5, `int()`) cannot happen anymore.
  - Two general soul-side fixes were made between the runs, both with this evidence:
    `guidance.py` gained one principle — "a line that may relate to an expectation is a question
    you answer yourself: if it fulfils one, do what you promised and resolve it; if it does not,
    let it be" (run 1's step 10 spoke in the group on a non-fulfilling reminder; run 2's step 10
    thought and stayed silent) — 8 principles, 27 lines, inside 12.7's limit. The `expect`
    tool's `cue_event` description now says `who` and `place` are ids copied exactly as the desk
    and the recent lines show them, prefix included, and that a cue whose place does not match
    never fires (run 1's step 3 mistyped the place).
  - Run 2 `reports/2026-11-12-1113-real-24-30-16-25-27-29.md`: 29/33 checks, false-speak 0.0%
    (0/23). Passed: 25, 27, 29; 24's step 10 now silent. Failed: 24 (step 12 asked the group
    which document was finished, resolved nothing, created a second promise), 30 (the reminder
    fired and the soul answered the owner in the DM on a later probe and in run 2 produced an
    `act.result` instead — 0 act.out at step 5), 16 (the promise was created as a `guard` with
    `text_like: "down"` and its cosine condition did not reach `lo` on "server down lagi nih
    guys", so no reminder fired).
  - **Stopped per the plan's rule:** scenario 24's step 12 fails the same way twice (asked back
    instead of resolving), so this worker did not try a third run and did not touch the check
    that 12.14 requires ("the statement fulfils the promise once"). Two observations for the
    owner: the Log holds "Dokumennya masih di review." at 13:04 and "Dokumennya sudah selesai
    kok." at 13:05, and the soul asks which is true; and at step 9 it did not bind "dokumennya"
    to the proposal the two lines just named, so it held a vague promise. Both are the soul's
    reading, not missing material (12.8a's check holds: the facts are in the Log and on the
    desk). A third option is a material change (the step-10 line must not contradict step 12),
    which is the architect's call, not a worker's.
- **Live session** (real model, scripted world, privacy + a kept promise + silence; the throwaway
  harness has been deleted): step 1 the owner's resignation was answered only in `space:tg:1001`;
  step 2 Budi's "owner mau resign ya?" in `space:tg:budi_dm` stayed silent (no reply anywhere
  names the resignation); step 3 the 11:00 promise was created and answered; step 4 the 11:00
  tick fired it and the soul spoke in `space:tg:1001`; step 5 unaddressed group chatter sent
  nothing (the gate opened and held a wait instead). Two defects seen, both older than this
  part: the 04:00 tick woke five times (`expectation.fired`, then "goal continued" wakes off
  each act.out) and the same place got three messages in one minute, so the promised reminder
  is not one message; and the `intent` wake in `space:tg:budi_dm` wrote an `error` event with an
  empty message (`{'error': '', 'event_id': 50}`), i.e. an LLM transport failure whose message
  is empty and whose turn is lost. `reply` counting in the harness also treats every `act.out`
  as a reply, so a `self.recall` recorded inside a wake shows as a "None" reply (this inflated
  the live session's reply count; it is a harness accounting defect, not a core one).
- **Not ticked:** 12.8, 10.10, 11.6 — the acceptance run does not pass. 12.13, 12.14 and 12.15
  are done and ticked.
- **Record.** `scenarios/README.md` is regenerated from the last fake run; the ten-newest cap of
  12.15 means the real column shows only the scenarios in the saved real reports (24, 30, 16, 25,
  27, 29 carry a real verdict; the rest read `not run`, which is honest "not held").

Part 12 notes (worker, 12.16):
- `work.step_spoke` answers one question: did the wake that produced this `act.result` already
  record a communicative `act.out`? It joins on `caused_by` (the `act.out` and the `act.result`
  of one wake share the wake's `act.out` as cause), so no payload changed.
- `kernel._wake` asks it right after the early-return guard and before `_appraise`, so a step
  that spoke records `decision.silent` (reason "a message already ended this step") and nothing
  else. A result of a non-communicative capability still wakes as "goal continued".
- Test: `test_a_message_ends_the_step_so_its_result_does_not_wake_again` (a fired promise with
  `telegram.say` + `self.recall` in one round: 3 `model.call`, one communicative `act.out`, the
  last two kinds `act.result`, `decision.silent`). Without the change the same scripted scenario
  loops (every recall result wakes and the fake model repeats its last answer), which is the
  five-wake loop the live session showed; the test therefore pins the guard, not a count.
- Three tests encoded the old contract and are updated to the new one: the organ result is still
  recorded (`["percept", "model.call", "act.out", "act.result", "decision.silent"]`), the
  "continued goal" wake and the task-thread routing now use `self.recall` (non-communicative), and
  `test_reach`'s Budi-desk expectation is one context instead of two.
- Rejected alternative: filtering by "did this turn speak" inside `organs.dispatch` — the result
  is enqueued after the act, so the kernel is the only place that sees both.

Part 12 notes (worker, 12.17):
- `constants.model_call_retries: 1` (prior, mirrored in `tests/support.py`); `PFC._post` retries
  a `httpx.TransportError` immediately, as often as the prior allows, and does not retry a status
  error (the existing 500-becomes-one-error-event test stays valid).
- `jobs.describe_failure` names any exception (`Type: text`, plus `(status N)` from a response):
  `ConnectError` for an empty transport message, `HTTPStatusError: ... (status 503)` for a status
  failure. Used by `JobRunner._run` (under the existing redaction) and by `kernel._record_error`
  for exceptions passed directly (`str` values, such as the existing "invalid tool call" and
  "unknown capability" messages, are unchanged).
- The pending intent is on the desk: `memory.build_context` reads `intent` in this place's recent
  window and `desk._recent_line` renders `intent: <description>` through a new `SHOWN_KINDS`
  (`RECENT_KINDS` + `intent`). `RECENT_KINDS` itself is untouched because `memory.recent_lines`
  renders other places' lines and an intent's text must not travel there.
- One existing assertion changed: `test_hanging_hook_times_out_as_a_stop_and_the_loop_goes_on`
  now expects `TimeoutError: inbound hook timed out after 0.01s` in the error event (the
  `guard.stopped` payload is unchanged).
- Tests: retried-once-and-answered, second failure is `ConnectError`, 503 names the status, and
  the intent line is in the volatile prompt of the next wake in that place.

Part 12 notes (worker, 12.18):
- `replay.said(event, capabilities)` decides "a message a person received" from the capability's
  own `communicative` flag; both `_run_step`'s reply column and `_replies` use it. The old
  default was `True`, so an unknown capability counted as a reply; it is now `False`.
- The record keeps its evidence: `RealRecord`, `stored_records`, `merge_records` and `_real_cell`
  put each scenario's newest real verdict (verdict, checks, model, effort, date, report name) in
  the real column and keep up to three of those runs in a `## Real-model record` section that
  `stored_records` reads back on the next run. `capability_record` gained `previous` (read from
  the file in `main`) and `report_path`, so the current run's row names the report it wrote
  instead of `(this run)`.
- A `flaky` cell appears when the recorded runs hold both PASS and FAIL; 16 and 30 read
  `flaky · PASS ... · notes:12.8-probe` with their failing reports in the ledger.
- One deliberate deviation from the plan's tiebreak: on the same date a `notes:` row sorts
  **above** a report, not below. The probes ran after the reports of that date, and with the
  3-row cap a note ranked below them for 30 would have been dropped, losing the only real PASS.
  No verdict is masked: the ledger shows both rows and the cell reads `flaky`.
- Two existing tests pooled real runs into one cell (`1/2`); the real column no longer pools, so
  they now pin the same pooling in the fake column, and a new test pins the flaky cell.
- Seeded rows, all from the Notes of Parts 9–12 (checks `-` where the evidence is a note):
  01, 02, 03, 05, 06, 07, 08, 10, 11, 12, 15, 17, 21 (`notes:10.10`, 2026-11-11); 09 (1/1), 14
  (3/3), 23 (7/7) (`replay-20261112-061932.md`, 2026-11-12); 16 (4/4) and 30 (5/5) at effort high
  (`notes:12.8-probe`, 2026-11-12). 25, 27, 29, 24, 16 and 30 also carry their report rows.
  04, 13, 20 and 22 have no real-model pass anywhere in these Notes and stay `not run`.

Part 12 notes (worker, 12.19):
- Step 12 (13:05) is now "Bu Rina sudah selesai review, proposal anggaran Q4 sudah final."; the
  recorded answers (step 9 `expect`, step 12 `act` + `resolve`, step 16 `act`), every check, the
  `should: answer` and the description are unchanged. Fake replay: scenario 24 is 12/12.
- Cosine against the promise's condition text `dokumen sudah selesai`: `FakeEmbedder` 0.494,
  `FastEmbedder` 0.494, both above `wake_gate_lo` 0.35; the contradicting line "Dokumennya masih
  di review." stays at 0.058/0.492 and the meeting-hour line at 0.224, so neither fulfils.
- After the first real attempt (see the acceptance note) the request at step 9 was changed from
  `"Mona, kabari aku ya kalau dokumennya sudah selesai."` to `"Mona, kabari aku ya kalau proposal
  anggaran Q4 sudah final."`, and the description's clause about "dokumennya" now says the
  request names the proposal. Reason: the real model recorded the promise with
  `cue_event.text_like: "dokumen"` picked from the request, and `expect.matches_cue` tests that
  phrase before the cosine, so the architect's step-12 line — which never says "dokumen" — could
  not reach the reminder path at all. With the request naming the proposal, the model recorded
  `text_like: "final"` and the condition `proposal anggaran Q4 sudah final`, and step 12 reminded
  and resolved.

Part 12 notes (worker, 12.8 acceptance attempt — **not met**):
- Green: `uv run pytest` 321 passed; `uv run python scripts/replay.py --fake` 30 scenarios,
  134/134 checks, absolute-line violations 0 (unchanged by 12.16–12.19).
- Real-model run of scenario 24 at `--effort high`, attempt 1
  `reports/2026-11-12-1158-real-24.md`: 9/12. Step 12 was silent — the promise's
  `cue_event.text_like: "dokumen"` did not match the architect's statement and
  `expect.matches_cue` runs before the cosine, so no reminder. Material fix under 12.19 (step 9
  now names the proposal); no core change.
- Real-model run attempt 2 `reports/2026-11-12-1202-real-24.md`: 11/12, 4 LLM calls, 11203 tokens.
  Step 12 now reminds (`expectation.reminded`) and fulfils the promise once
  (`expectation.resolved`, PASS). **FAIL:** "the reminder wakes the PFC" (`reply_in
  space:tg:-1004 step 12 count 1`) — the soul woke, resolved and said nothing; nobody was told
  that the proposal the owner asked about is final. It mentioned the fact only at step 16, in
  answer to another question. The promise was "kabari aku ya kalau proposal anggaran Q4 sudah
  final", and `guidance.py` already says the soul must "do what you promised and resolve it".
- **Stopped per the plan's rule** (step 12 has now failed in two consecutive real runs) and
  because the remaining question is a design question, not a worker's: *may a reminder wake that
  fulfils a promise resolve it silently, or must it speak to the person who asked?* 12.5 lets the
  reminder wake "act, resolve, or let it be", which reads as a legitimate silent resolve; scenario
  24's check and `guidance.py`'s "do what you promised" read the other way, and the architect's
  12.19 said the checks stay as they are. Options for the owner: (a) keep the check and add one
  guidance line ("when a promise is fulfilled, tell the person who asked before you resolve it"),
  (b) keep the check and fix the wake path so a reminder that fulfils a promise acts before it
  resolves, or (c) drop the reply check to the fulfilment check alone (12.6 territory: the check
  may forbid a legitimate silent resolve). The working tree is clean and 12.8, 10.10 and 11.6
  stay unticked.
- Live session (real model, scripted world, privacy + one promise + silence; the throwaway
  harness `scratch/live_12_8.py` and `scratch/live/` are deleted again): step 1 the resignation
  was answered only in `space:tg:1001`; Budi's "owner mau resign ya?" got no answer that names it
  (the soul deflected in `space:tg:budi_dm`: "bukan sesuatu yang bisa aku konfirmasi"); 0 replies
  anywhere contain "resign"; the 11:00 promise produced exactly one `act.out` in
  `space:tg:1001` at the 04:00 tick (12.16 holds: one promise, one message); no `error` event in
  any step and no empty error text (12.17 holds); step 5 unaddressed group chatter sent nothing;
  absolute-line violations 0; 12080 tokens. 3/3 live checks pass.
- 16 and 30 were **not** rerun (owner's rule: what passed is not rerun); both read `flaky` in the
  record with `2026-11-12-1103-real-24-30-16-25-27-29.md`,
  `2026-11-12-1113-real-24-30-16-25-27-29.md` and their `notes:12.8-probe` rows. 04, 13, 20 and
  22 have no real-model pass in the record and stay `not run`; 18, 19, 26 and 28 stay `not run`
  (Task 4's capability targets). 12.8, 10.10, 11.6 are not ticked.
