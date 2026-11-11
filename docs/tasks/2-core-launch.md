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
- [ ] 1.2 Sleep's private connection rebuilds its DSN from `conn.info`: prove with a test that the DSN and password never reach a log, an event, an error message or a report.

## Part 2 — Replay baseline
Read first: [core-math.md](../core-math.md) §13
May touch: `scenarios/`, `scripts/replay.py`, `tests/`

- [ ] 2.1 Bring the fake replay to 25/25: the 8 scenarios failing since before Task 1 (reminders and `recall` with the scripted FakeLLM) are fixed by recording the model answers they need, not by changing the core.
- [ ] 2.2 Folder layout by brain function: `scenarios/<function>/` (memory, silence, turn-taking, promises, privacy, cross-body, learning, honesty); one scenario tests one behavior; model answers recorded so a fake replay is deterministic.
- [ ] 2.3 From here on, every part below adds its scenarios here, written to fail before the part lands.

## Part 3 — Privacy follows people, sharing, reaching a person, task threads (D45–D47)
Read first: [core.md](../core.md) §8 (absolute line 2), §4 (reaching a person, `self.share`, task threads), §7 (memories: who, place, private)
May touch: `monoagens/hooks.py`, `monoagens/memory.py`, `monoagens/pfc.py`, `monoagens/organs.py`, `monoagens/store.py` (audience lookup only), new `monoagens/reach.py`, new `monoagens/threads.py`, `monoagens/kernel.py` (only calls into the new modules), `tests/`, `scenarios/`

Replaces the per-place privacy filter from Task 1 with the audience rule.

- [ ] 3.1 Audience of a place: the set of person ids present there (after the alias map); for a direct chat, that one person.
- [ ] 3.2 Context-hook filter: a private memory or line enters only if audience(context) ⊆ audience(source), or the context is the owner's direct chat; content from non-private places is unrestricted.
- [ ] 3.3 `recall` obeys the same rule.
- [ ] 3.4 Tests: Fulan's private chat on body A is usable in Fulan's private chat on body B once the accounts are linked; never in a group with anyone else; a private group's content is usable in a direct chat with one of its members; unlinked accounts are different people.
- [ ] 3.5 `self.share(item | episode, to: person | place)` (a place shares with everyone present there): passes the action hook only when `origin_actor` is in the content's audience; writes `share.granted`; the filter then counts the new people as audience.
- [ ] 3.6 `act` offers the capabilities of every running organ; a communicative act may take `to: person` and optionally `via: body`, resolved to the person's direct place (the named body, else the body where last active); no direct place → honest `act.result` failure.
- [ ] 3.7 A communicative act aimed at another place is drafted with a desk built for that place's audience.
- [ ] 3.8 Task threads: a multi-step chain opens a `task` place (parent, parent's audience, own PFC slot), writes `task.opened`/`task.closed`, reports to the parent; `max_task_steps` and `max_open_tasks` from the prior.
- [ ] 3.9 Scenario: the owner, in a Minecraft-like test organ, asks what Budi sent in a file on another body → answered (owner alone there) or moved to a private channel (others present); "send our work to Budi and ask if it fits" → `share.granted`, sent to Budi's direct place, a wait Expectation; Budi's reply resolves it and the owner hears back where the owner is now. The owner cannot share Fulan's private line to Budi.
- [ ] 3.10 Scenario: "finish my code, then send it to Budi via WhatsApp and to the group" → a task thread, a promise Expectation fired by its completion, delivery to Budi on WhatsApp, a share to the group's audience; with two candidate groups the soul asks which one before sending; meanwhile the owner's chat in the game place is answered without waiting for the task's steps.

## Part 4 — ACT-R activation
Read first: [core.md](../core.md) §7, [core-math.md](../core-math.md) §5
May touch: `monoagens/memory.py`, `tests/test_memory.py`, `scenarios/`

- [ ] 4.1 Base-level activation B = ln(n/(1−d)) − d·ln(L) from the injected clock; d from `memory_persistence`.
- [ ] 4.2 Total activation A = B + w_ctx·cos + w_imp·importance; candidates top 40 by vector ∪ top 20 by text; update `use_count`.
- [ ] 4.3 Feeling-of-knowing helper (max activation among candidates, recent messages included) for Part 5.
- [ ] 4.4 Check: frequently used memories outrank newer unused ones; activation is reproducible under a fake clock.

## Part 5 — The five signals and habituation
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §2–3, §15
May touch: `monoagens/cognition/signals.py`, `monoagens/cognition/situation.py`, `tests/test_cognition.py`, `scenarios/`

- [ ] 5.1 `own_name` (attention capture, not obligation).
- [ ] 5.2 `info_gap` from the prior's reference sentences.
- [ ] 5.3 `feeling_of_knowing` using the helper from 4.3.
- [ ] 5.4 `relevance` to active promises and core memory.
- [ ] 5.5 `social_cost`: the floor belongs to someone else, lowered by closeness.
- [ ] 5.6 Habituation per (place, who) with γ from `boredom_with_repeats`.
- [ ] 5.7 Check: "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated stickers fade; signals are pure (no I/O).

## Part 6 — Turn-taking and patience
Read first: [core.md](../core.md) §5–6, [core-math.md](../core-math.md) §4, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`, `scenarios/`

- [ ] 6.1 `wait` Expectation created on a go for a turn not allocated to the soul, window from `patience_seconds`.
- [ ] 6.2 `not_who` cue: someone other than the sender takes the floor → silent.
- [ ] 6.3 Addressed → no wait.
- [ ] 6.4 Check: not addressed — someone else answers → silent, nobody → one reply; addressed → answers without waiting.

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
