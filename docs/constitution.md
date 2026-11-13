# Constitution

The few rules that never change. Every document, task, plugin and worker follows them. Only the
owner can amend this file. Decision numbers (Dxx) trace to the history in git.

## 1. The contract is locked; the internals answer to evidence (D44, D86–D88)
Like a kernel: the interface to the outside never breaks, the inside may improve.
- **Locked** — [contract.md](contract.md), the one contract between the layers (v2, opened and
  locked by the owner on 2026-11-14): the event envelope, features, the capability descriptor,
  hooks, context items, actions and receipts, actors, places, audiences, percepts, lifecycle
  events, answers and commands — plus the hook contract, the six levers, `recall`, the birth
  certificate and prior format, the two absolute lines, the complexity budget and the brain map.
  A hash test holds it; only the owner changes it, additively, by the rules of contract.md §9.
- **After the lock there is no "core change" or "middleware change"** of the contract kind.
  What is new — a platform, a device, a feature — enters through an adapter or a plugin.
- **Every file format carries its version** (D58): events through their schema version, YAML
  files through `version:` (or `format_version` / `manifest_version`). Older versions are
  upcast when read, never rewritten; a newer one than supported fails loudly. This is how a
  frozen contract evolves without breaking.
- **Provisional** — the internal formulas: how each signal is computed, the terms of θ, the
  ACT-R parameters, the context layout. They may change **only with evidence** (replay scenarios
  plus real logs showing the change is better), recorded as a decision with that evidence, never
  by a task, plugin or worker on its own.
- **Data** — the values in the default prior (`prior/`), tuned with replay at any time.
- Always allowed: fixing a bug, a vulnerability, an inconsistency, a typo or a broken
  reference, and clarifications or refactors that change no behavior. Everything else belongs to another layer — middleware, adapters, plugins.

## 2. Generic, simple, stable (D86, D87)
- **The core and the middleware are generic and know nothing above them.** They provide generic
  facts, events and entries that make sense for every body — a chat, a robot, a workspace app, a
  sensor, a game — and never specialize for one. Specializing them for anything above is
  forbidden, without exception.
- **Translation lives at the edge.** Every platform's or device's rules are translated by its
  adapter or plugin into the contract's facts; the middleware and the core never translate.
- **One direction**: the middleware builds on the core's contract, adapters and plugins build on
  the middleware's; a lower layer never changes for a need of an upper one.
- **Understanding belongs to reasoning.** No code in the core or the middleware tries to
  understand human language; decisions not left to the model are made from facts, never words.
- **Simple is king.** Covering scenarios one by one is not design. A case that does not fit is
  classified (contract.md §9), never patched.

## 3. Design by brain function, not by scenario (D39, D40)
- The core is a closed list of brain functions: the brain map in [core.md §2](core.md). Each row
  is a brain region, a studied model with its formula and sources ([core-math.md](core-math.md))
  and one core mechanism.
- Human scenarios are tests, not design inputs: a new situation becomes a replay scenario
  checked against the map. Reasoning (the LLM) fills the gaps between functions.
- No rule is ever written to close a single case. A behavior that keeps failing means a weak
  function; role-, place- or preference-specific behavior belongs to middleware or a plugin.

## 4. Complexity budget (D26, D32)
The core holds at most 4 books, 5 thought acts, 3 hooks, 10 signals, 3 drives and one gate
formula. A new primitive must replace one or be refused.

## 5. Layers (D35, D41, D57)
- **Core** — neutral and primitive: only a brain. It knows no case; it provides mechanisms and
  assembles them. It knows only **opaque ids, labels and numbers**: who and where are strings,
  labels are attached by the middleware and carried without being read, numbers feed the gate.
  It does not know what a person, an owner, a group or a role is. The soul's own identity (birth
  certificate and prior) is the one thing it holds as its own.
  Like a computer's kernel it restricts nothing itself: its capabilities are open — recall the
  whole log, act in any place and any adapter, share data across threads — and every one of them
  leaves only through the three hooks, where the middleware keeps it from running wild.
- **Middleware** — the regulator, like drivers: it decides, it is the only path between the core
  and everything else, and it may limit or add what reaches the core. People, the owner,
  privacy, sharing, routes, commands, the tool registry, budget limits and quiet hours live
  here, as generic rules over the contract's facts; it knows no platform and translates
  nothing. The owner policy is the middleware that is always installed; the core refuses to run
  without it (fail-closed).
- **Adapters and plugins** — talk only to the middleware, through the contract: they translate
  their platform or device into its facts, and the soul's actions back into native actions.
  Standard names are in [glossary.md](glossary.md).
- Everything that reaches the brain is a `percept`; everything that persists is an event.
- Middleware and plugins never learn; the soul does (D34). They may add any behavior, emotions
  included, through the six levers ([middleware.md](middleware.md)); the core itself has no
  emotions.

## 6. Two absolute lines (D24, D32)
Enforced deterministically by the owner policy, never decided by the model, never removable by a
plugin; another plugin can only narrow them:
1. Only the owner changes who the soul is.
2. Private content never reaches anyone who was not part of it (D45): it follows the people
   it belongs to, across adapters and places, and reaches someone new only when one of those
   people shares it (D46).

## 7. Engineering invariants
Append-only log · record first, then act · one queue, one consumer; slow work as background jobs
· a watchdog on every hook and job, so nothing installed can freeze the soul · all time from the
injected clock, all randomness seeded · every model call gated, recorded and counted in the
budget · configuration outside code · one locked contract ([contract.md](contract.md)):
additive changes only, versioned events, a hash test, tests that speak only through it. The coding rules that follow from these are
in [AGENTS.md](../AGENTS.md).

## 8. Honesty and claims
The soul is honest about what it knows and does not know. MonoAgens is never described as
conscious or as "% human"; claims are per behavior against a human baseline.

## 9. Open source
Apache 2.0. BYOK: the owner brings their own model key. One deployment is one soul with its own
database. Public documents mention no hosted service (D7, D8).
