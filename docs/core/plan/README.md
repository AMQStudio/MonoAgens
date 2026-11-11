# Core — Plan

How plans work: this file fixes **what comes first and when a step is done**. Agents manage the
details themselves: they may add working files in this `plan/` folder (one per track or phase,
e.g. `A2.md`), tick steps and record findings. They never change DESIGN, DECISIONS, FORMATS or
MATH; a design question stops the work and goes to the architect. Rules for every plan:
[docs/README.md](../../README.md#rules-for-every-plan).

Global order of all layers: [docs/README.md](../../README.md#implementation-order).

## Done
- MVP: Telegram, the four books, gate, PFC, Expectations, memory and sleep; tests (a)–(d).
- Fix-up pass; replay tool with a real LLM (17 scenarios); CLI organ; `<recent>` across places.
- Track A foundation: cognition package, `event_vectors`, seeded generator, decision
  breakdowns, hook lists, token budget.
- Concept freeze: core design FINAL (D1–D41).

## Step 1 — Track A2: align the code with the core design (one track, first)
Sub-steps, one commit each:
1. Thought acts `think, act, expect, resolve, revise`; `say`/`react` become Telegram and CLI
   capabilities (descriptor in [FORMATS.md](../FORMATS.md)); silence = a wake without `act`;
   `msg.in` becomes `percept` with a description and optional data (D41); `act.out`/`act.result`
   replace `msg.out`/`react.out` in code, tests and replay.
2. Three hooks (inbound, context, action) with owner-policy slots first and last, shared vs
   organ-own scope, collision check at boot.
3. Provenance and privacy: `origin_actor` on events (inherited along `caused_by`),
   `identity_changing` check in the action hook; `place` + `private` on memories and the
   context-hook filter.
4. Lineage: `derived_from` on memories, expectations and vectors; replace the `event_vectors`
   no-delete trigger with the two dedicated paths (fade, hard forget) — stubs that write their
   events are fine.
5. Budget: `model.call` for every model call; the budget term in θ and `budget.exhausted`
   (no thrift mode, D40).
6. Expectation schema with the four kinds (`promise`, `wait`, `predict`, `guard`).
7. Injected clock everywhere (no `datetime.now()` in the core); replay uses event times.
8. Slow work off the loop (D33): PFC, classifier, embedding and sleep as background jobs whose
   results return as events; one PFC job per place.
9. Numbered SQL migrations with a version table and a backup before migrating; pairing code
   for the first owner.
10. Identity as birth content (D35): `soul.yaml` becomes the birth certificate; cognition numbers
    move to the default prior as knobs / calibration / constants (D36, see
    [birth plan](../../birth/plan/README.md)); identity stored in `state: identity` after birth;
    an `identity_changing` capability for owner changes.

Done when: all tests and replay pass; grep shows no platform name, no world action and no direct
clock or random call in the core.

## Step 2 — Cognition and memory (parallel tracks after A2)
| Track | Work | Done when |
|---|---|---|
| B | ACT-R activation, `use_count`, feeling-of-knowing helper | frequently used memories outrank newer unused ones |
| C | the five signals + habituation | "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated stickers fade |
| D | dynamic θ (incl. familiarity), seeded go/no-go, circuit breaker on commitments, decision outcomes (D40) | same events + seed → same decisions; talk rate ≤ 1/n in a busy group |
| E | `wait` Expectations as the self-selection rule, `addressed` from organs, `not_who` | not addressed: someone else answers → silent, nobody → one reply; addressed: answers without waiting |
| F | `think` with motivation/confidence, principles, person cards, self-model, D38 honesty principles | proactive replies always have a recorded thought; "what can you do?" matches `<self>`; a false claim about the past is not confirmed, and a missing record is not called proof |
| H | `revise`, `guard` beliefs checked in sleep, lessons and procedures with credence (α/β), `basis` on memories, source-over-gist, merge only on same people/place/time (D38) | a correction supersedes the old memory; a contradicted fact is revised at the next sleep |
| I | wake gate; free-text conditions with cosine filter + classifier; `lo` learning | a fulfilled promise wakes no one; a question on the same topic never resolves a promise |
| J | attribution and reliability (α/β) per domain; fade with a fixed threshold; soft and hard forgetting | a stranger cannot override an owner fact; hard forget leaves only `{"forgotten": true}` and metadata |
| K | the desk (D37): section token budgets, work thread in `<doing>` with pointers, the `self` organ with `recall` (query, time range, lineage, text search on events), back-fill appraised as one age-discounted percept per place | a 15-step task keeps a flat prompt size; "what did we do last month?" lists that month's episodes with details from the original events, also after the vector faded and when sleep failed; 40 unread messages are appraised once |
| R | replay scenarios for B–K, plus report metrics: false-speak rate, missed-help rate, tokens per simulated day, absolute-line violations (must be 0), honesty-principle compliance | each new scenario fails before its track lands and passes after |

**Deferred until weeks of real logs exist:** learned signal weights and knobs, adaptive fade
threshold, social standing, learned coverage. They tune existing functions; the design stays.

## Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).
