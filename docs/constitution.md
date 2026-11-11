# Constitution

The few rules that never change. Every document, task, plugin and worker follows them. Only the
owner can amend this file. Decision numbers (Dxx) trace to the history in git.

## 1. The core is a frozen brain
The core design ([core.md](core.md)) is **FINAL for v1 and frozen**. Allowed edits to it: fixing
an inconsistency, a typo or a broken reference, and clarifications that change no behavior.
Everything else belongs to another layer — middleware, bodies, plugins, birth inputs. A need that
truly cannot be met there waits for a v2 of the core, which only the owner can open, with a new
major version. No task, plugin or worker changes the core design.

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
- **Core** — the soul, a generic brain. **Middleware** — the foundation in the middle: policy,
  commands & tools, install, skills, identity, RBAC. **Bodies** — organs for platforms and
  devices. **Plugins** — features built on the middleware. **Birth inputs** — birth certificate
  and prior.
- Bodies and plugins reach the core only through the middleware: the three hooks and the organ
  contract. Everything that reaches the brain is a `percept`; everything that persists is an
  event.
- Middleware and plugins never learn; the soul does (D34). They may add any behavior, emotions
  included, through the six levers ([middleware.md](middleware.md)); the core itself has no
  emotions.

## 5. Two absolute lines (D24, D32)
Enforced deterministically by the owner policy, never decided by the model, never removable by a
plugin:
1. Only the owner changes who the soul is.
2. Private content never enters another place.

## 6. Engineering invariants
Append-only log · record first, then act · one queue, one consumer; slow work as background jobs
· all time from the injected clock, all randomness seeded · every model call gated, recorded and
counted in the budget · configuration outside code. The coding rules that follow from these are
in [AGENTS.md](../AGENTS.md).

## 7. Honesty and claims
The soul is honest about what it knows and does not know. MonoAgens is never described as
conscious or as "% human"; claims are per behavior against a human baseline.

## 8. Open source
Apache 2.0. BYOK: the owner brings their own model key. One deployment is one soul with its own
database. Public documents mention no hosted service (D7, D8).
