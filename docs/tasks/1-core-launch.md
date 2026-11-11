# Stage 1 — Core: launch

Everything needed until the owner uses the soul every day in a direct chat and one group.
Order and status: [README](README.md). Each section is one task; work only inside yours.

---

## 021 — Core: follow-ups from 001
Status: todo | Depends on: 001
Read first: [core.md](../core.md) §3 (watchdog), [AGENTS.md](../../AGENTS.md) (secrets)
May touch: `monoagens/jobs.py`, `monoagens/config.py`, `tests/`

**Goal.** Close the small gaps found in the review of 001.

- [ ] T1 The job timeout comes from `prior/dispositions.yaml` (`job_timeout_seconds`, 120); remove the hard-coded 60 s default.
- [ ] T2 Sleep's private connection rebuilds its DSN from `conn.info`: prove with a test that the DSN and password never reach a log, an event, an error message or a report.

**Done when.** Both tests pass, with all earlier tests.

**Notes.**

---

## 020 — Core: privacy follows people, sharing, reaching a person, task threads (D45–D47)
Status: todo | Depends on: 021
Read first: [core.md](../core.md) §8 (absolute line 2), §4 (reaching a person, `self.share`, task threads), §7 (memories: who, place, private)
May touch: `monoagens/hooks.py`, `monoagens/memory.py`, `monoagens/pfc.py`, `monoagens/organs.py`, `monoagens/store.py` (audience lookup only), new `monoagens/reach.py` (audience, consent, resolving a person, cross-place drafting), new `monoagens/threads.py` (task threads), `monoagens/kernel.py` (only the calls into the new modules), `tests/`, `scenarios/`

**Goal.** Replace the per-place privacy filter written in 001 T3 with the audience rule: a private
item may enter a context only if everyone who will see the result was part of the item's
audience, or the context is a direct chat with the owner; content reaches someone new only by
consent; the soul can act in any body from any body and address a person instead of a place;
long work runs in its own task thread.

`kernel.py` (280) and `store.py` (240) are at their size budget. New logic goes into
`reach.py` and `threads.py`; the kernel only calls them. If that is not enough, stop and report.

- [ ] T1 Audience of a place: the set of person ids present there (after the alias map); for a direct chat, that one person.
- [ ] T2 Context-hook filter: allow a private memory or line only if audience(context) ⊆ audience(source), or the context is the owner's direct chat; content from non-private places is unrestricted.
- [ ] T3 `recall` obeys the same rule.
- [ ] T4 Tests: Fulan's private chat on body A is usable in Fulan's private chat on body B once the accounts are linked; never in a group with anyone else; a private group's content is usable in a direct chat with one of its members; unlinked accounts are different people.
- [ ] T5 `self.share(item | episode, to: person | place)` (a place shares with everyone present there): passes the action hook only when `origin_actor` is in the content's audience; writes `share.granted`; the filter then counts the new people as audience.
- [ ] T6 `act` offers the capabilities of every running organ; a communicative act may take `to: person` and optionally `via: body`, resolved to the person's direct place (the named body, else the body where last active); no direct place → honest `act.result` failure.
- [ ] T7 A communicative act aimed at another place is drafted with a desk built for that place's audience.
- [ ] T8 Task threads: a multi-step chain opens a `task` place (parent, parent's audience, own PFC slot), writes `task.opened`/`task.closed`, reports to the parent; `max_task_steps` and `max_open_tasks` from the prior.
- [ ] T9 Replay scenario: the owner, in a Minecraft-like test organ, asks what Budi sent in a file on another body → answered (owner alone there) or moved to a private channel (others present); "send our work to Budi and ask if it fits" → `share.granted`, sent to Budi's direct place, a wait Expectation; Budi's reply resolves it and the owner hears back where the owner is now. The owner cannot share Fulan's private line to Budi. Second part: "finish my code, then send it to Budi via WhatsApp and to the group" → a task thread, a promise Expectation fired by its completion, delivery to Budi on WhatsApp, a share to the group's audience; with two candidate groups the soul asks which one before sending. While the code task runs, the owner's chat in the game place is still answered without waiting for the task's steps.

**Done when.** All tests and the replay scenario pass, all earlier tests and replay pass, no
private content reaches a person outside its audience in any test or replay, and every file is
within its size budget.

**Notes.**

---

## 011 — Core: replay scenarios and metrics (Track R)
Status: todo | Depends on: 001 (runs alongside every core task)
Read first: [core.md](../core.md), [core-math.md](../core-math.md) §13
May touch: `scenarios/`, `scripts/replay.py`, `tests/`

**Goal.** Replay proof for every core task and honest metrics.

- [ ] T0 Bring the replay suite to 25/25: the 8 scenarios failing since before 001 (reminders and `recall` with the scripted FakeLLM) are fixed by recording the model answers they need, not by changing the core.
- [ ] T1 Folder layout by brain function: `scenarios/<function>/` (memory, silence, turn-taking, promises, privacy, cross-body, learning, honesty); one scenario tests one behavior; model answers are recorded so a replay is deterministic.
- [ ] T2 [P] One or more scenarios per core task, written to fail before the task lands.
- [ ] T3 Report metrics, **missed-help rate first** (of the messages that should have been answered, how many passed the gate), then false-speak rate, tokens per simulated day, absolute-line violations (must be 0), honesty-principle compliance.

**Done when.** The suite passes in full; each new scenario fails before its task lands and
passes after.

**Notes.**

---

## 002 — Core: ACT-R activation (Track B)
Status: todo | Depends on: 020
Read first: [core.md](../core.md) §7, [core-math.md](../core-math.md) §5
May touch: `monoagens/memory.py`, `tests/test_memory.py`

**Goal.** Retrieval ranked by ACT-R activation, with use counts and a feeling-of-knowing helper.

- [ ] T1 Base-level activation B = ln(n/(1−d)) − d·ln(L) from the injected clock; d from `memory_persistence`.
- [ ] T2 Total activation A = B + w_ctx·cos + w_imp·importance; candidates top 40 by vector ∪ top 20 by text; update `use_count`.
- [ ] T3 Feeling-of-knowing helper (max activation among candidates, recent messages included) for task 003.

**Done when.** Frequently used memories outrank newer unused ones; activation is reproducible
under a fake clock.

**Notes.**

---

## 003 — Core: the five signals and habituation (Track C)
Status: todo | Depends on: 020; T3 on 002
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §2–3, §15
May touch: `monoagens/cognition/signals.py`, `monoagens/cognition/situation.py`, `tests/test_cognition.py`

**Goal.** Pure signal functions and per-source habituation.

- [ ] T1 [P] `own_name` (attention capture, not obligation).
- [ ] T2 [P] `info_gap` from the prior's reference sentences.
- [ ] T3 [P] `feeling_of_knowing` using the helper from 002.
- [ ] T4 [P] `relevance` to active promises and core memory.
- [ ] T5 [P] `social_cost`: the floor belongs to someone else, lowered by closeness.
- [ ] T6 Habituation per (place, who) with γ from `boredom_with_repeats`.

**Done when.** "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated
stickers fade; signals are pure (no I/O).

**Notes.**

---

## 005 — Core: turn-taking and patience (Track E)
Status: todo | Depends on: 020
Read first: [core.md](../core.md) §5–6, [core-math.md](../core-math.md) §4, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`

**Goal.** Patience as the self-selection rule of turn-taking.

- [ ] T1 `wait` Expectation created on a go for a turn not allocated to the soul, window from `patience_seconds`.
- [ ] T2 `not_who` cue: someone other than the sender takes the floor → silent.
- [ ] T3 Addressed → no wait.

**Done when.** Not addressed: someone else answers → silent, nobody → one reply; addressed:
answers without waiting.

**Notes.**

---

## 010 — Core: the desk and recall (Track K)
Status: todo | Depends on: 020
Read first: [core.md](../core.md) §4, §8, §9.4 (desk constants)
May touch: `monoagens/pfc.py`, the `self` organ module, `tests/test_pfc.py`

**Goal.** The prompt rebuilt per wake within budgets, the work of a task thread, recall and
back-fill.

- [ ] T1 Section token budgets from prior constants; trimming oldest-first per section.
- [ ] T2 A task thread's desk shows its work in `<doing>`: goal, one line per step, latest result verbatim, older results as pointers.
- [ ] T3 Middleware notes rendered with their event.
- [ ] T4 `recall(query | event_id, place?, from?, to?)`: time range, lineage, text search on events, scope reported; obeys absolute line 2 as built in 020.
- [ ] T5 Back-fill appraised as one age-discounted percept per place.

**Done when.** A 15-step task keeps a flat prompt size; "what did we do last month?" lists that
month's episodes with details from the original events, also after the vector faded and when
sleep failed; 40 unread messages are appraised once.

**Notes.**

---

## 004 — Core: threshold, gate and circuit breaker (Track D)
Status: todo | Depends on: 003
Read first: [core.md](../core.md) §6, [core-math.md](../core-math.md) §3, §15
May touch: `monoagens/cognition/appraise.py`, `monoagens/cognition/appraisal.py`, `monoagens/cognition/rng.py`, `tests/test_cognition.py`

**Goal.** Dynamic θ, the seeded go/no-go draw and the circuit breaker on commitments.

- [ ] T1 θ = base + a·talk_rate + b·budget_used − c·promise_debt + r·(1 − familiarity); familiarity from ACT-R activation of interactions with the place and people present.
- [ ] T2 P = σ((S_eff − θ)/τ) with the seeded draw; every number stored in the decision event.
- [ ] T3 Circuit breaker: `addressed`, an active Expectation fired or answered, or an active goal continued.
- [ ] T4 Knob mapping function (one place) from the prior's knobs to θ, τ, γ and weights.

**Done when.** Same events + seed → same decisions; talk rate ≤ 1/n in a busy group; an
addressed message always wakes; tuned for recall — the missed-help rate (011) is the first
number to bring down.

**Notes.**
