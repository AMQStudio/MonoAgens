# Constitution

The few rules that never change. Every document, task, plugin and worker follows them. Only the
owner can amend this file. Decision numbers (Dxx) trace to the history in git.

## 1. The core's contract is frozen; its internals answer to evidence (D44)
Like a kernel: the interface to the outside never breaks, the inside may improve.
- **Frozen for v1** — the stability contract ([core.md §3](core.md)): percept, capability
  descriptor, hook contract, the six levers, events, `recall`, birth certificate and prior
  format — plus the two absolute lines, the complexity budget and the list of brain functions
  (the brain map). Breaking any of these is a core v2, which only the owner can open.
- **Provisional** — the internal formulas: how each signal is computed, the terms of θ, the
  ACT-R parameters, the desk layout. They may change **only with evidence** (replay scenarios
  plus real logs showing the change is better), recorded as a decision with that evidence, never
  by a task, plugin or worker on its own.
- **Data** — the values in the default prior (`prior/`), tuned with replay at any time.
- Always allowed: fixing a bug, a vulnerability, an inconsistency, a typo or a broken
  reference, and clarifications or refactors that change no behavior. Everything else belongs to another layer — middleware, bodies, plugins.

## 2. Design by brain function, not by scenario (D39, D40)
- The core is a closed list of brain functions: the brain map in [core.md §2](core.md). Each row
  is a brain region, a studied model with its formula and sources ([core-math.md](core-math.md))
  and one core mechanism.
- Human scenarios are tests, not design inputs: a new situation becomes a replay scenario
  checked against the map. The PFC (the LLM) fills the gaps between functions.
- No rule is ever written to close a single case. A behavior that keeps failing means a weak
  function; role-, place- or preference-specific behavior belongs to middleware or a plugin.

## 3. Complexity budget (D26, D32)
The core holds at most 4 books, 5 thought acts, 3 hooks, 10 signals, 3 drives and one gate
formula. A new primitive must replace one or be refused.

## 4. Layers (D35, D41)
- **Core** — the soul, a generic brain, including what it is born with (birth certificate and
  prior). **Middleware** — the foundation in the middle: policy, commands & tools, install,
  skills, identity, RBAC. **Bodies** — organs for platforms and devices. **Plugins** — features
  built on the middleware.
- **The middleware decides, the core records and carries out.** The core holds mechanisms
  and the labels in the log (who was present, where a memory came from, who shared what); every
  rule that reads those labels — privacy, who is which person, who may share, budget limits,
  quiet hours — is middleware. The owner policy is the middleware that is always installed; the
  core refuses to run without it (fail-closed).
- Bodies and plugins reach the core only through the middleware: the three hooks and the organ
  contract. The standard names (adapter, tool, context, reasoning) are in
  [glossary.md](glossary.md). Everything that reaches the brain is a `percept`; everything that persists is an
  event.
- Middleware and plugins never learn; the soul does (D34). They may add any behavior, emotions
  included, through the six levers ([middleware.md](middleware.md)); the core itself has no
  emotions.

## 5. Two absolute lines (D24, D32)
Enforced deterministically by the owner policy, never decided by the model, never removable by a
plugin; another plugin can only narrow them:
1. Only the owner changes who the soul is.
2. Private content never reaches anyone who was not part of it (D45): it follows the people
   it belongs to, across bodies and places, and reaches someone new only when one of those
   people shares it (D46).

## 6. Engineering invariants
Append-only log · record first, then act · one queue, one consumer; slow work as background jobs
· a watchdog on every hook and job, so nothing installed can freeze the soul · all time from the
injected clock, all randomness seeded · every model call gated, recorded and counted in the
budget · configuration outside code · a stable contract for everything outside the core:
additive changes only within a major version, versioned events ([core.md §3](core.md)). The coding rules that follow from these are
in [AGENTS.md](../AGENTS.md).

## 7. Honesty and claims
The soul is honest about what it knows and does not know. MonoAgens is never described as
conscious or as "% human"; claims are per behavior against a human baseline.

## 8. Open source
Apache 2.0. BYOK: the owner brings their own model key. One deployment is one soul with its own
database. Public documents mention no hosted service (D7, D8).
