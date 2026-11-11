# Documentation map

MonoAgens is split into layers. Each layer has its own folder with the same shape:
`DESIGN.md` (how it works; not needed for project), `DECISIONS.md` (rulings), `FORMATS.md` (file
formats, where needed) and `plan/` (order of implementation; agents keep their working notes there).

| Layer | Folder | Status |
|---|---|---|
| Core — the soul, a generic brain | [core/](core/) (+ [MATH.md](core/MATH.md): formulas and sources) | **FINAL for v1, frozen** |
| Middleware — the foundation in the middle (policy, rbac, commands & tools, install, identity, skills) | [middleware/](middleware/) (one folder per module) | decided points + Draft |
| Bodies — platforms and devices | [bodies/](bodies/) | decided points + Draft |
| Plugins — features on the middleware (tools, memory-view, modes, roles) | [plugins/](plugins/) | decided points + Draft |
| Birth inputs — birth certificate and prior | [birth/](birth/) | decided points + Draft |
| Project — license, users, release, evaluation | [project/](project/) | decided points + Draft |

Root files: [README.md](../README.md) (what it is, how to run) and [AGENTS.md](../AGENTS.md)
(rules for coding agents).

Words: the core documents say *organ* for anything that senses or acts (bodies and tool organs)
and *package* for a plugin.

**The core is frozen.** Nothing in another layer, a plan, a plugin or a worker may change the
core design. A need that cannot be met outside the core waits for a v2 of the core, which only
the owner can open ([core rulings §0](core/DECISIONS.md)).

## Implementation order
| Step | Layer | What |
|---|---|---|
| 1 | core (+ bodies, birth) | Track A2: align the code with the core design |
| 2 | core | cognition and memory tracks B–K + R, in parallel |
| 3 | middleware, then plugins | foundation (policy → commands & tools → install → skills); then the modes example and memory-view plugins |
| 4 | project | first users, release gate, installer, README, LICENSE |
| 5 | project | honest human-likeness evaluation |
| 6 | plugins | first tool plugins: web search/fetch, file read |
| 7 | birth | born from a prior; default prior |
| 8 | bodies | more bodies (WhatsApp, Discord, voice, IoT/robot sim), more models |
| 9 | plugins | stable contracts for developers |

## Rules for every plan
1. A step is done only when its checks pass **and** every earlier test and the replay suite
   still pass.
2. **The core does not grow to make a feature fit.** New behavior is a body, a plugin, or a
   middleware module; the core design is frozen.
3. Contracts first, code second. Parallel tracks start only after their contract is merged.
4. Every behavior change ships with at least one replay scenario that shows it.
5. Public documents mention no hosted service (D8).
6. **Design by brain function, not by scenario** (D39): a new situation becomes a replay test;
   a missing behavior goes to middleware, never a rule for one case in the core.
