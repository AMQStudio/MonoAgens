# DECISIONS.md — Concept Decisions (design freeze)

Status: **concept maturation in progress; implementation is paused.**
These decisions are agreed between the owner and the architect. Where they conflict with
DESIGN.md or ROADMAP.md, **this file wins** until those documents are rewritten to match.
Workers must not start new work based on this file until DESIGN.md and ROADMAP.md are updated.

## Locked

**D1 — The core is only a brain.**
The core knows exactly three generic things: **senses** (anything that `put`s events),
**organs** (anything that receives actions via `act` and may send results back as events),
and **hooks** (points where middleware attaches). The core has no notion of "tool", "plugin",
"skill", "command", "gateway", or any platform.

**D2 — Only thought acts live in the core.**
Core acts: `think`, `stay_silent`, `expect`, `resolve`. The brain decides intent and content.
Every act on the world, **including speaking**, is a capability an organ declares
(name, JSON schema, description, and whether it is *communicative*). The organ decides how to
carry it out (Telegram splits into bubbles, a speaker uses TTS, a lamp blinks). Fine motor
control lives inside the organ; the PFC only gives high-level intentions ("go to the kitchen").
The PFC tool list is assembled from the organs that are currently present.
Signals that count "talking" (talk rate, forward model) use the *communicative* flag.

**D3 — Multi-step work is a chain of events.**
An organ's result comes back as an event; cognition treats "result of my own action" as a
reflex and wakes the PFC to continue. Every step is recorded, so tasks survive restarts and can
be interrupted by new messages. Limits: max steps per task, token budget, approvals via hooks.
Acting on the world also creates a forward-model Expectation ("I expect to arrive within 30 s");
a miss wakes the PFC.

**D4 — Two levels of forgetting.**
- *Soft* ("forget what I just said"): related memories get `valid_to`, the events are excluded
  from `<recent>` and retrieval, and a `forget.soft` event is written. Data remains, never
  surfaced.
- *Hard* ("delete my data"): the only exception to the immutable log. One dedicated function
  replaces event payloads with `{"forgotten": true}`, deletes their vectors, derived memories and
  related expectations, and writes a `forget.hard` event. The database trigger allows exactly
  this change and nothing else.
- Who may ask: the owner, or the person the data is about. Backups keep old data until they
  expire; this must be documented for users.

**D5 — Owner-only instructions switch on automatically in any group.**
Personal mode (no middleware) is only for a bot used purely by its owner. As soon as the bot is
in a group, the owner-only policy middleware is active: others may talk to it, only the owner
can instruct it, create promises on its behalf, or change settings.

**D6 — Memories record their source and trust.**
Sleep stores "Budi said X", not "X is true". Each memory has `source_who` and a trust level
(owner > known person > stranger). A stranger's claim never overrides an owner's fact.
Messages from non-owners appear in prompts as quoted, attributed data, never as instructions.
The strongest defense against leaks is the context hook: what is not in the prompt cannot leak.

**D7 — BYOK; one deployment is one soul.**
The user brings their own model key. Each deployment runs exactly one soul with its own
database. Keys come from the environment only, never from soul.yaml or the database. The
embedder stays a replaceable port.

**D8 — Open source under Apache 2.0.**
Public documents describe MonoAgens only; they mention no hosted service.

**D9 — Packages bundle organs, middleware, skills and dependencies.**
- A loader outside the core assembles packages; the core only sees the resulting senses,
  organs and hooks.
- Middleware scope is **shared** (applies to every organ that opts in: commands, skills, tool
  handling) or **organ-own** (only for things unique to that platform).
- **Commands are defined once** in a shared registry. Organs only translate their native syntax
  (`/help@bot`, `/help`, `:help`) into a neutral `command` event and may add presentation
  (menus, buttons) but never logic. `/help` is the same everywhere; packages register new
  commands into the same registry.
- **Owner policy middleware wraps every package middleware** and cannot be bypassed or removed
  by a package: `[owner policy — first] → [package middleware] → [owner policy — last]`.
- Name collisions (two packages claiming the same command or organ name) fail at boot.

**D10 — Identity across bodies.**
- Every native account (`telegram:123`, `whatsapp:628…`) maps to one person id; each mapping is
  an event and can be undone.
- Only the owner, or the person themself via a verification code sent from the second account,
  may link two accounts.
- The agent may suspect and ask the owner, but never links silently (linking lets memories flow
  between places).
- The alias map is an inbound middleware; the core never sees raw accounts.

**D11 — Skills are outside the core and learned skills need approval.**
- Installed skills: `skills/<name>/SKILL.md` (the common SKILL.md format), contributed by
  packages and injected through the context hook. Only names and descriptions of relevant skills
  enter the prompt; the body is loaded when the PFC asks for it.
- Learned skills: during sleep, a successful multi-step task may be distilled into a draft
  skill. A draft becomes active only after owner approval (an approval Expectation) or a
  configured number of successful uses.
- Skills are versioned; a new version never silently overwrites the old one. Skill use counts
  feed the same ACT-R activation as memories.

**D12 — Self-model: the PFC always knows who, when, where, what and how.**
Assembled by the kernel on every wake from data that already exists; the LLM only reads it.
- Stable prompt: *who I am* (soul.yaml), *how I work* (a short fixed self-description: I have
  memories that can be incomplete, I can be wrong, I act only through organs), *honesty rules*
  (below), and *my knowledge cutoff* (from model configuration).
- Semi-stable: `<self>` — what I can do, assembled from the registries of present organs,
  commands and skills, including a short "cannot do" list for commonly asked things; person
  cards. `/help` and "what can you do?" come from the same source.
- Volatile: `<now>` (local time, timezone, time since the last interaction), `<doing>` (my own
  last actions across places, running tasks), `<next>` (promises due soon).
- Honesty rules: (1) facts that change (office holders, prices, news, versions, schedules):
  use a search organ if one is present; otherwise answer as "as of my knowledge in <cutoff>,
  ... this may have changed". (2) Retrieved memories carry their source and age, and the PFC
  attributes them ("Budi said yesterday ...") instead of stating them as facts. (3) Low
  confidence from `think` means ask back or say "I don't know". Never claim a capability that is
  not in `<self>`.

## Accepted in principle, details pending

**P1 — Self-correction (track H).** A `revise` act: when corrected, the old memory gets
`valid_to` and a new one supersedes it. Important facts about people become passive
Expectations ("Budi lives in Bandung"); a contradicting event is a miss that triggers revision.
Sleep distills forward-model outcomes and corrections into `lesson` memories. The agent never
edits its own principles or persona; that stays with the owner.

## Still open
- Data growth: backups, database size, how far back retrieval looks.
- First users: the owner only, a specific community group, or other developers.
