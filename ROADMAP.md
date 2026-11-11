# ROADMAP.md — From MVP to the Final Goal

The final goal: **one persistent self that lives in many bodies, behaves like a thoughtful
person (mostly quiet, helpful when it matters, honest about what it knows), learns from
experience, talks to many people safely, runs on any model the owner brings, grows through
packages, and works out of the box for non-technical people.**

LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the brain.

## Rules for every phase
1. Done only when its checks pass **and** every earlier test and the replay suite still pass.
2. **The core does not grow to make a feature fit.** New behavior is a signal, an Expectation,
   a state key, an organ, or middleware in a package. Anything else goes to the architect first.
3. Contracts first, code second. Parallel tracks start only after their contract is merged.
4. Every behavior change ships with at least one replay scenario that shows it.
5. Public documents mention no hosted service (D8).

## Done
- MVP: Telegram, the four books, gate, PFC, Expectations, memory and sleep; tests (a)–(d).
- Fix-up pass; replay tool with a real LLM (17 scenarios); CLI organ; `<recent>` across places.
- Track A foundation: cognition package, `event_vectors`, seeded generator, decision
  breakdowns, hook lists, token budget.
- Concept freeze: DECISIONS D1–D37, DESIGN.md finalized in eight sections.

## Phase 1 — Align the code with DESIGN (one track, first)
**Track A2** — bring the merged Track A in line with DESIGN (D32). Sub-steps, one commit each:
1. Thought acts `think, act, expect, resolve, revise`; `say`/`react` become Telegram and CLI
   capabilities (descriptor in docs/FORMATS.md); silence = a wake without `act`;
   `act.out`/`act.result` replace `msg.out`/`react.out` in code, tests and replay.
2. Three hooks (inbound, context, action) with owner-policy slots first and last, shared vs
   organ-own scope, collision check at boot.
3. Provenance and privacy: `origin_actor` on events (inherited along `caused_by`),
   `identity_changing` check in the action hook; `place` + `private` on memories and the
   context-hook filter.
4. Lineage: `derived_from` on memories, expectations and vectors; replace the `event_vectors`
   no-delete trigger with the two dedicated paths (fade, hard forget) — stubs that write their
   events are fine.
5. Budget: `model.call` for every model call; thrift mode and `budget.exhausted`.
6. Expectation schema with the four kinds (`promise`, `wait`, `predict`, `guard`).
7. Injected clock everywhere (no `datetime.now()` in the core); replay uses event times.
8. Slow work off the loop (D33): PFC, classifier, embedding and sleep as background jobs whose
   results return as events; one PFC job per place.
9. Numbered SQL migrations with a version table and a backup before migrating; pairing code
   for the first owner.
10. Identity as birth content (D35): soul.yaml becomes the birth certificate (cognition numbers
   move to the default prior as knobs / calibration / constants, D36); identity stored in `state: identity` after birth; an
   `identity_changing` capability for owner changes.
Done when: all tests and replay pass; grep shows no platform name, no world action and no
direct clock or random call in the core.

## Phase 2 — Cognition and memory (parallel tracks after A2)
| Track | Work | Done when |
|---|---|---|
| B | ACT-R activation, `use_count`, feeling-of-knowing helper | frequently used memories outrank newer unused ones |
| C | the five signals + habituation | "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated stickers fade |
| D | dynamic θ, seeded go/no-go, decision outcomes | same events + seed → same decisions; talk rate ≤ 1/n in a busy group |
| E | `wait` Expectations (patience), `not_who` | someone else answers → silent; nobody → one reply |
| F | `think` with motivation/confidence, principles, person cards, self-model | proactive replies always have a recorded thought; "what can you do?" matches `<self>` |
| H | `revise`, `guard` facts checked in sleep, lessons and procedures with credence (α/β) | a correction supersedes the old memory; a contradicted fact is revised at the next sleep |
| I | wake gate; free-text conditions with cosine filter + classifier; `lo` learning | a fulfilled promise wakes no one; a question on the same topic never resolves a promise |
| J | attribution and reliability (α/β) per domain; fade with a fixed threshold; soft and hard forgetting | a stranger cannot override an owner fact; hard forget leaves only `{"forgotten": true}` and metadata |
| K | the desk (D37): section token budgets, work thread in `<doing>` with pointers, the `self` organ with `recall` (query, time range, lineage, text search on events), back-fill handling and `catch_up` wakes | a 15-step task keeps a flat prompt size; "what did we do last month?" lists that month's episodes and details come from the original events, also after the vector faded and when sleep failed; 40 unread messages produce one catch-up wake |
| R | replay scenarios for B–J, plus report metrics: false-speak rate, missed-help rate, tokens per simulated day, absolute-line violations (must be 0), honesty-rule compliance | each new scenario fails before its track lands and passes after |

**Deferred until weeks of real logs exist (D32.9):** learned signal weights, learned
dispositions (D16, D29), adaptive fade threshold (D14), social standing (D31), identity linking
(D10), package suggestions (D28).

## Phase 3 — Packages and the default owner policy
- Loader for packages (organs + middleware + skills + dependencies), shared/organ-own scope.
- Default owner policy, active out of the box (D22, D24): the two absolute lines (only the owner
  changes who the soul is; private content never enters another place), budget, quiet hours.
  Everything else is weighed by the PFC.
- **memory-view** package (D37): list what the soul remembers about the asker or this place,
  corrections through `revise`, owner-only Markdown export.
- Shared packages: **commands** (one registry, identical `/help`) and **skills** (installed
  SKILL.md only; static; D34). The alias map waits for a second chat app.
Done when: a DM fact is never visible or said in a group, even when someone tries to talk the
soul into it; a reasonable request from a group member is helped; a package cannot remove the
policy;
`/help` is identical on Telegram and CLI.

## Phase 4 — First users and the release gate (D13, D33)
- **The owner alone** for a few days with replay checks after each change.
- **One real group** the owner knows well, with the owner policy on.
- Release gate: end-to-end tests with a real model, a real database, long multi-day
  conversations and real people; a per-model replay report and a list of recommended models.
- Installer (one command from clone to a talking soul) and README (brain table, install, talk,
  tune with replay; no claims of consciousness or "% human").

## Phase 5 — Honest human-likeness evaluation
Per-behavior parity against a human baseline in the same group: reply timing, messages per hour
vs 1/n, when-to-speak precision/recall against human annotation, silence where it should speak,
interruptions where it should be quiet, promise keeping, recall after 1/7/30 days, blind
"which one is the bot" (target ≈ 50%). Pre-registered metrics. Public claims phrased as
"≥ X% of human baseline on these N behaviors".

## Phase 6 — World organs and multi-step work
First world organs: web search/fetch, file read. Long-running actions report through
`act.result`; limits on steps per task. Freshness rules (D12) use the search organ.
Done when: "who is the president now?" triggers a search when a search organ exists, and an
honest "as of my knowledge" answer when it does not.

## Phase 7 — Born from a prior (D17, D25)
Loading a prior (docs/FORMATS.md) at birth; a default prior in the repository.
Done when: a fresh soul born from the default prior passes the replay suite with empty memory.

## Phase 8 — More bodies, more models
WhatsApp (with `history` and back-fill on connect, D37), Discord, voice (STT/TTS), the first non-chat organ (IoT device or robot sim);
native provider adapters; optional separate models for sleep and the classifier.
Done when: one conversation continues across three bodies and a non-chat organ senses and acts
through the same ports; switching provider is a `.env` change.

## Phase 9 — Developers (D13)
Stable, versioned contracts for organs, packages and middleware; examples; contribution guide.
Starts only after the contracts have not changed for a while.

## Later ideas (not scheduled)
- A small learned classifier for the gate from recorded decisions (> 10k labeled).
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).

## Open questions
See "Still open" in DECISIONS.md.
