# Documentation map

MonoAgens is split into layers. Each layer has its own folder with the same shape:
`DESIGN.md` (how it works; not needed for project), `DECISIONS.md` (rulings), `FORMATS.md` (file
formats, where needed) and `plan/` (order of implementation; agents keep their working notes there).

| Layer | Folder | Status |
|---|---|---|
| Core — the soul, a generic brain | [core/](core/) (+ [MATH.md](core/MATH.md): formulas and sources) | **FINAL for v1, frozen** |
| Organs — bodies and tools | [organs/](organs/) | decided points + Draft |
| Middleware — static rules in the hooks | [middleware/](middleware/) | decided points + Draft |
| Packages — plugins | [packages/](packages/) | decided points + Draft |
| Birth inputs — birth certificate and prior | [birth/](birth/) | decided points + Draft |
| Project — license, users, release, evaluation | [project/](project/) | decided points + Draft |

Root files: [README.md](../README.md) (what it is, how to run) and [AGENTS.md](../AGENTS.md)
(rules for coding agents).

**The core is frozen.** Nothing in another layer, a plan, a package or a worker may change the
core design. A need that cannot be met outside the core waits for a v2 of the core, which only
the owner can open ([core rulings §0](core/DECISIONS.md)).

## Implementation order
| Step | Layer | What |
|---|---|---|
| 1 | core (+ organs, birth) | Track A2: align the code with the core design |
| 2 | core | cognition and memory tracks B–K + R, in parallel |
| 3 | middleware + packages | package loader, default owner policy, commands, skills, memory-view |
| 4 | project | first users, release gate, installer, README, LICENSE |
| 5 | project | honest human-likeness evaluation |
| 6 | organs | first tool organs: web search/fetch, file read |
| 7 | birth | born from a prior; default prior |
| 8 | organs | more bodies (WhatsApp, Discord, voice, IoT/robot sim), more models |
| 9 | packages | stable contracts for developers |

## Rules for every plan
1. A step is done only when its checks pass **and** every earlier test and the replay suite
   still pass.
2. **The core does not grow to make a feature fit.** New behavior is an organ, middleware or a
   package; the core design is frozen.
3. Contracts first, code second. Parallel tracks start only after their contract is merged.
4. Every behavior change ships with at least one replay scenario that shows it.
5. Public documents mention no hosted service (D8).
6. **Design by brain function, not by scenario** (D39): a new situation becomes a replay test;
   a missing behavior goes to middleware, never a rule for one case in the core.
