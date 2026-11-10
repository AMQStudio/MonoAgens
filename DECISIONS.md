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
  configured number of successful uses. **Amended by D19: drafts now activate at once.**
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

**D13 — Order of users.**
First the owner alone, then one real group the owner knows well (which is also where the honest
human-likeness evaluation happens), then other developers once the contracts are stable.

**D14 — Retention by activation, not by age.**
No fixed day limits. During sleep every detail trace (per-message vector) is scored with the
same ACT-R activation as memories (use, recency, importance, links to promises and to active
memories). High activation keeps the detail; low activation drops the vector so only the gist in
memories remains. Raw events always stay in the log (subject to D4 hard forgetting). The
activation threshold adapts to a storage budget set by the owner, like the other drives.
Retention is adaptive but deterministic: same data and same rules give the same result; no
randomness. Backups are compressed daily with an owner-set retention period.

**D15 — Waking is not thinking.**
When an Expectation comes due, or an event might answer it, the kernel first runs a cheap gate
with no PFC call:
- *Still relevant?* If the completion condition is already met, the Expectation is resolved
  and nobody is woken.
- *Stale?* Past its expiry, it is dropped.
- *Allowed now?* Quiet hours, daily budget and nudge caps are checked.
Only what survives wakes the PFC. A woken PFC re-judges the situation; it never replays an old
promise blindly.
An Expectation's completion condition may be free text ("Muqsith answered about the
schedule"). It is judged by meaning: a match score from vector similarity, in three bands:
clearly no (ignore), clearly yes (resolve), gray (one call to a small classifier model, whose
verdict is recorded as an event). Optional hints (who, place) only narrow the search.

**D16 — Dispositions are learned, not configured.**
No tuning panel. The numbers that shape behavior (the band edges of D15, the speak/silent
threshold θ, decay rates, general talkativeness) start from an innate prior (D18, or soul.yaml
defaults) and move by small, bounded steps from experience. Every change is an event, so it can
be audited and rolled back. Main teacher: the classifier's verdicts on gray cases; they become
labelled examples that slowly narrow the gray band, so the gate gets cheaper over time.
Corrector: how people react (answered, ignored, told to stop). Learning is slow and bounded;
one bad day must not change the character. This extends the delta-rule learning already in
DESIGN.md.

**D17 — Three layers of learned state.**
1. *Dispositions* — the numbers of D16.
2. *Skills* — how to solve things, lessons from failures (what went wrong and why), and general
   social intuition ("when someone says 'whatever', they often want me to choose").
3. *Episodic memory* — conversations, facts about people, relationships.
Test for layers 1–2: "would this still make sense for another soul that does not know who
these people are?" If not, it belongs to layer 3. Tuning that concerns one relationship
("chatty with A, quiet with B") is layer 3. Lessons from P1 enter layer 2 only after being
generalised.

**D18 — Birth snapshot.**
Layers 1–2 can be exported as a snapshot; layer 3 never is. A snapshot is *generic* (names,
places and personal details removed), *reproducible* (it carries replay scenarios that prove
each item still works) and records the embedder and LLM models it was learned with (learned
numbers do not transfer across models). A new soul is born from a snapshot with empty memory
and keeps learning from its own users. Inheritance gate: an item is passed on only if it passes
replay on a fresh soul, no previously passing scenario regresses, and, at first, the owner
approves it as curator. Public documents say "snapshot" and nothing about hosting (D8).

**D19 — Autonomy lives in the core; brakes live in the owner policy. Amends D11.**
Learned skills become active immediately, are versioned, and can be rolled back. Requiring
approval before activation is no longer a core rule; an owner may still add it as an
owner-policy setting (D9). The same holds for every other brake: the core has none of its own.

**D20 — Product stance: out of the box.**
The target is non-technical people. A good first experience needs no configuration: the soul is
born with an innate prior (D18) and adapts. The core aims at human cognitive *functions*
(perception, attention, memory, judgement, learning, restraint), not emotions. Roles such as
assistant, customer service or sysadmin come from packages (D9), never from the core.

## Accepted in principle, details pending

**P1 — Self-correction (track H).** A `revise` act: when corrected, the old memory gets
`valid_to` and a new one supersedes it. Important facts about people become passive
Expectations ("Budi lives in Bandung"); a contradicting event is a miss that triggers revision.
Sleep distills forward-model outcomes and corrections into `lesson` memories. The agent never
edits its own principles or persona; that stays with the owner.

## Still open
- Track H (P1) details.
- Whether the agent may suggest installing a package when asked for something it cannot do.
- Optional deeper research on memory consolidation and forgetting formulas (D14).
- Whether the classifier call of D15 fits invariant 5 ("the LLM is only called through the
  gate") or the invariant needs wording that lets the gate use a small classifier.
- The update rule for band edges and dispositions (step size, bounds, minimum gray width).
- How scrubbing for D18 is done and checked. Automatic scrubbing is unreliable; owner curation
  is the backstop at first.
- Snapshot format and versioning.
- Rewrite DESIGN.md and ROADMAP.md from these decisions, then write prompts for the tracks.
