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

## Phase 1 — Align the code with DESIGN (one track, first)
**Track A2** — bring the merged Track A in line with D1–D23:
- `say` / `react` move out of `pfc.py` into the Telegram and CLI organs as declared
  capabilities (with the `communicative` flag); the PFC tool list is assembled from organs.
- `act.out` / `act.result` replace `msg.out` / `react.out` (with a migration of old kinds in
  replay and tests).
- `event_vectors`: replace the no-delete trigger with the two dedicated delete paths
  (retention, hard forgetting) — paths may be stubs that only write their events for now.
- Hooks: four hooks named inbound / context / action / outbound; owner-policy slots first and
  last; shared vs organ-own scope; collision check at boot.
- `model.call` events for every model call (D23).
Done when: all tests and replay pass; grep shows no platform name and no world action in the core.

## Phase 2 — Cognition and memory (parallel tracks after A2)
| Track | Work | Done when |
|---|---|---|
| B | ACT-R activation, `use_count`, FOK helper | frequently used memories outrank newer unused ones |
| C | signals + habituation | "lupa nama tempatnya apa ya" scores high only when the answer is known; repeated stickers fade |
| D | dynamic θ, go/no-go, decision outcomes | same events + seed → same decisions; talk rate ≤ 1/n in a busy group |
| E | patience Expectations, `not_who` | someone else answers → silent; nobody → one reply |
| F | `think`, motivation/confidence gate, principles, person cards, self-model (D12) | proactive replies always have a recorded thought; "what can you do?" matches `<self>` |
| G | forward model, delta rule, dispositions with bounds (D16) | ignored proactive messages lower their causes; α = 0 disables learning |
| H | self-correction (P1): `revise`, guarded facts, lessons | a correction supersedes the old memory; a contradicted fact triggers revision |
| I | wake gate (D15, D21): relevance, staleness, allowed-now; free-text completion with cosine filter + classifier | a promise already fulfilled wakes no one; a question on the same topic never resolves a promise |
| J | memory source and trust (D6); retention by activation (D14); soft and hard forgetting (D4) | a stranger cannot override an owner fact; hard forget leaves only `{"forgotten": true}` |
| R | replay scenarios for B–J (any time) | each new scenario fails before its track lands and passes after |

## Phase 3 — Packages and the default owner policy
- Loader for packages (organs + middleware + skills + dependencies), shared/organ-own scope.
- Default owner policy, active out of the box (D22, D24): the two absolute lines (only the owner
  changes who the soul is; private content never enters another place), budget, quiet hours.
  Everything else is weighed by the PFC.
- Credence for skills and lessons (D24): provenance, track record, bounded updates.
- Shared packages: **commands** (one registry, identical `/help`), **alias map** (D10),
  **skills** (installed SKILL.md, learned skills, versions, rollback; D11, D19).
Done when: a DM fact is never visible or said in a group, even when someone tries to talk the
soul into it; a reasonable request from a group member is helped; a package cannot remove the
policy;
`/help` is identical on Telegram and CLI.

## Phase 4 — First users (D13)
- **The owner alone** for a few days with replay checks after each change.
- **One real group** the owner knows well, with the owner policy on.
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

## Phase 7 — Birth snapshot (D17, D18)
Export layers 1–2, generic and replay-backed; inheritance gate with owner curation.
Done when: a fresh soul born from a snapshot passes the replay suite with empty memory.

## Phase 8 — More bodies, more models
WhatsApp, Discord, voice (STT/TTS), the first non-chat organ (IoT device or robot sim);
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
