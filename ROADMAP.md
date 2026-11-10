# ROADMAP.md — From MVP to the Final Goal

The final goal: **one persistent self that lives in many bodies, behaves like a thoughtful
person in groups (mostly quiet, helpful when it matters), talks to many people safely, runs on
any model, grows through plugins, and installs with one command.**

LLMs are a prefrontal cortex without the rest of the brain. MonoAgens is the rest of the brain.

## Rules for every phase
1. A phase is done only when its "done when" checks pass **and** every earlier test and the
   replay suite still pass.
2. **The kernel does not grow to make a feature fit.** New behavior arrives as a signal, an
   Expectation, a state key, a body, a tool, or middleware. If a phase seems to need kernel
   changes beyond the ones listed here, stop and bring it to the architect.
3. Contracts first, code second. Parallel work starts only after the contract it depends on is
   merged.
4. Every behavior change ships with at least one replay scenario that shows it.

## Done
- **MVP**: Telegram body, the four books, the gate, the PFC, Expectations, memory and sleep;
  tests (a)–(d).
- **Fix-up pass**: readable code, fired expectations always wake, sleep retry guard,
  clean `<recent>`.
- **Replay tool** with a real LLM and 17 scenarios.
- **CLI body** (in-process socket) and `<recent>` across places.

## Phase 1 — Cognition layer (parallel tracks)
See DESIGN "Cognition layer". Track A goes first and alone; B–G run in parallel after A is
merged, each in its own module.

| Track | Work | Done when |
|---|---|---|
| **A. Foundation** | `event_vectors` table and encode-once; `monoagens/cognition/` package with `Situation`, the `Signal` contract and registry, `appraise()` called by the kernel (reflexes only at first, same behavior as today); decision events carry S, θ, P, u, signals; token usage into `budget:<date>`; `origin` column on expectations; `use_count` on memories | All existing tests and replay pass unchanged; every decision event has the full breakdown |
| **B. Memory activation** | ACT-R activation replaces 0.995^hours; FOK helper | Unit tests on the activation curve; frequently used memories outrank newer unused ones |
| **C. Signals + habituation** | name_mentioned, info_gap, feeling_of_knowing, relevance, social_cost; habituation state | "lupa nama tempatnya apa ya" scores high when the answer is in memory, low when it is not; 10 repeated stickers fade |
| **D. Threshold + go/no-go** | dynamic θ (talk rate, budget, promise debt), seeded draw, decision outcomes | Same events + seed → identical decisions; its talk rate stays at or below 1/n in a busy-group scenario |
| **E. Patience** | patience Expectations, `not_who` in patterns | Someone else answers within the window → silent; nobody answers → one reply |
| **F. Inner thought** | `think` tool, motivation/confidence gate, constitution prompt, person cards in sleep and in `<people>` | Proactive replies always have a recorded thought; low motivation → silent with the thought logged |
| **G. Forward model + learning** | forward Expectations, `forward.outcome`, delta rule, `weights.updated` | Ignored proactive messages lower the weights that caused them; α = 0 disables learning |
| **R. Replay scenarios** (any time) | forgotten place name; question already answered by someone; repeated stickers; being corrected; quiet group for days; owner's promise during a busy chat | Each new scenario fails before its track lands and passes after |

## Phase 2 — Middleware hooks
- The four hooks (inbound, context, tool, outbound) with the Continue / Replace / Stop
  contract, `guard.stopped` events, fail-closed on exceptions.
- First middleware: **commands** (`/status`, `/pause`, `/forget`), and **owner-only
  instructions** (others may talk to it; only the owner can make it create promises for them
  or change settings).
- **Context privacy**: memories and recent lines from DMs never enter group contexts.
- Done when: personal mode (no middleware) behaves exactly as before; with the guard installed,
  a replay scenario proves a DM fact is never visible or said in a group.

## Phase 3 — Launch
- Installer: one script for Postgres (compose), schema, `.env` from `.env.example`, `soul.yaml`.
- README: what it is, the brain table, install, talk to it, tune with replay. No claims of
  consciousness or "% human".
- Done when: a clean machine goes from clone to a talking bot with one command, and a new user
  can install it without asking anyone.

## Phase 4 — Honest human-likeness evaluation
- Human-Parity Index per behavior: reply timing distribution, messages per hour vs 1/n,
  when-to-speak precision/recall against human annotation, silence on moments it should speak,
  interruptions on moments it should be quiet, promise keeping, recall after 1/7/30 days,
  blind "which one is the bot" test (target ≈ 50%).
- Always against a human baseline in the same group. Pre-register metrics and thresholds.
- Done when: a per-metric report exists; any public claim is phrased as "≥ X% of human
  baseline on these N behaviors".

## Phase 5 — Plugins and world tools
- One plugin mechanism for bodies, PFC tools and middleware; the kernel only sees registries.
- First tools: web search/fetch, file read. Introduces think → tool → think with a hard limit
  on rounds per wake. Tool calls go through hook 3.
- Done when: a plugin installed without touching `monoagens/` adds a working tool.

## Phase 6 — Full guard post
- Roles, approvals as Expectations, per-place privacy policies, all on the hooks from Phase 2.
- Done when: one soul serves the owner fully in a DM and safely in a public group.

## Phase 7 — More bodies
- WhatsApp, Discord, voice (STT/TTS), and the first non-chat body (IoT device or robot sim)
  whose sensors arrive as events and whose actuators are actions.
- Done when: one conversation continues across three bodies, and a non-chat body senses and
  acts through the same two ports.

## Phase 8 — Native providers and model split
- Native adapters next to the OpenAI-compatible one; optional `SLEEP_MODEL`; optional small
  model for inner thoughts.
- Done when: switching provider is a `.env` change and replay results stay within tolerance.

## Phase 9 — Product layer
- Operator CLI (start/stop/status/logs/doctor), presets ("apps": personal assistant,
  community bot, sysadmin = a soul + bodies + middleware), schema upgrades.

## Later ideas (not scheduled)
- A small learned classifier for the gate, trained on recorded decisions (only after > 10k
  labeled decisions).
- Skills: reusable procedures learned from repeated work.
- Multi-process deployment (inbox table with `FOR UPDATE SKIP LOCKED`).

## Open decisions for the owner
- Open source publicly, or keep private for AMQStudio first?
- Who are the first users: the owner only, a specific community group, or other developers
  building on MonoAgens?
