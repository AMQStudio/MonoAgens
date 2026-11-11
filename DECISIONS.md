# DECISIONS.md — Concept Decisions (design freeze)

Status: **DESIGN.md was finalized from D1–D32 on 2026-11-11.**
These decisions are agreed between the owner and the architect. If DESIGN.md or ROADMAP.md
ever disagree with this file, this file wins and the other document is a bug.
Implementation resumes only when the owner says so, starting with ROADMAP Phase 1 (Track A2).

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

**D5 — Owner-only instructions switch on automatically in any group.** *(Amended by D24: others' requests are weighed; only the two absolute lines are hard.)*
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

**D11 — Skills are outside the core and learned skills need approval.** *(Superseded in part by D34: learned procedures are core memories; skills packages hold installed skills only.)*
- Installed skills: `skills/<name>/SKILL.md` (the common SKILL.md format), contributed by
  packages and injected through the context hook. Only names and descriptions of relevant skills
  enter the prompt; the body is loaded when the PFC asks for it.
- Learned skills: during sleep, a successful multi-step task may be distilled into a draft
  skill. A draft becomes active only after owner approval (an approval Expectation) or a
  configured number of successful uses. **Amended by D19 and D24: drafts activate at once with a credence that follows their source and track record.**
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

**D18 — Birth snapshot.** *(Amended by D25: inside this repository only loading a prior exists; building and sharing snapshots is out of scope.)*
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

**D21 — Cosine is only a filter, never a judge. Amends D15.**
Vector similarity measures "about the same thing", not "answers it": a question on the same
topic scores as high as its answer. So for free-text completion conditions the cosine only
filters: m < lo is ignored; every m ≥ lo goes to the classifier. The "clearly yes" band starts
disabled (hi = 1) and may open only when recorded classifier verdicts show that the band above
the proposed hi contains no false resolutions. Opening it is a `disposition.updated` event.

**D22 — The default owner policy ships with sensible brakes. Amends D19 and D20.** *(Amended by D24.)*
The core still has no brakes of its own (D19). But a soul is installed with a default owner
policy, active out of the box, so a non-technical owner is protected without configuring
anything:
- skills learned from work with the owner activate at once; skills whose source involves
  anyone else wait for owner approval (an approval Expectation);
- owner-only instructions in groups (D5);
- the daily model budget and quiet hours.
The owner may loosen or tighten these; packages cannot remove them (D9).

**D23 — Every model call goes through a gate and is accounted for. Replaces the old wording of
invariant 5.**
No model call (PFC, classifier, sleep) happens per message or per tick by default. Each call is
the result of a gate decision, is recorded as an event with its purpose and token usage, and
counts toward the daily budget. The D15 classifier therefore needs no exception.

**D24 — Weigh, don't block: trust grows from experience; only two absolute lines. Amends D5
and D22.**
People do not follow "advice from others needs permission". They weigh who said it, how
plausible it is and what it costs if wrong, try it carefully, and trust it more when it works.
- **Credence for skills and lessons.** Every skill and lesson carries its provenance (who
  taught it, uses, successes). Starting credence follows the source's trust (owner high,
  stranger low). Forward-model outcomes raise or lower it; a skill that keeps failing sinks and
  stops being used. The PFC sees provenance and track record ("taught by Budi, used 3×, worked
  1×") and decides: use it, try it carefully, or ask the owner. No approval rule.
- **Requests from others are weighed, not refused.** Anyone may ask ("Mona, remind us at 7");
  the PFC weighs the request against the owner's interests and its principles, like a good
  assistant would.
- **Only two absolute lines**, because LLM judgment can be talked into anything and these must
  hold even then:
  1. Only the owner can change who the soul is: principles, settings, and who may access the
     owner's data.
  2. Content from a private conversation never enters another place: it is never put into the
     prompt there (context hook).
- The default owner policy (D22) keeps only these two lines plus the budget and quiet hours.
  Its former "skills from others wait for approval" and D5's "only the owner can instruct" are
  replaced by weighing.

**D25 — The open-source core is for using and learning on your own. Amends D18.**
- The open-source repository contains everything a soul needs to live and to learn from its
  own owner and people: all three layers of learned state, stored in its own database, with
  layers 1–2 kept clearly separate from layer 3.
- It can **start from a prior**: a local file of layer 1–2 values (dispositions, generic skills
  and lessons) that is loaded at birth. A default prior ships with the repository.
- It contains **no export, upload, sharing, aggregation or curation** of learned state. Building
  snapshots from many souls, filtering them and grouping them per model family is a separate
  project outside this repository; public documents do not describe it (D8).
- Because the prior is only a file format, the core stays unaware of where a prior came from.

**D26 — Complexity budget for the core.**
The core holds at most: 4 books (events, state, expectations, memories; event_vectors is the
index of events), 5 thought acts (think, stay_silent, expect, resolve, revise — D27), 4 hooks, 10 signals, 3 drives,
one gate formula. A new idea that needs another core primitive must replace an existing one or
be refused. Everything else lives in organs, packages or the owner policy.

**D27 — Self-correction (details of P1).**
- *Direct correction*: the PFC is already awake when someone replies to the soul. It uses the
  thought act `revise(old memory, new text, reason)`: the old memory gets `valid_to` and points
  to its replacement; nothing is deleted. Trust still applies: a stranger's correction of an
  owner fact is stored as a conflicting claim, and the soul may ask the owner.
- *Guarded facts*: during sleep, important facts about people from trusted sources become
  passive Expectations ("this should still be true", origin `guarded_fact`). Possible
  contradictions are found by the wake gate (cosine filter, then classifier; D21) and are
  **queued for the next sleep**, not woken live — unless the fact is relevant to the current
  conversation, in which case the PFC uses the newest information at once.
- *Lessons*: during sleep, forward outcomes, corrections and failed actions are distilled into
  lessons. Generic ones go to layer 2 with credence (D24); person-specific ones to layer 3.
- The soul never revises its own principles or persona.

**D28 — The soul may suggest a package, politely and rarely.**
Only to the owner; only if the package is in the local catalog of installable packages (never
invented); at most once per missing capability per habituation window; it can never install
anything itself (absolute line 1, D24).

**D29 — How dispositions learn.**
Character may change slowly; one strange day must not change it.
- Every value has a prior, a floor, a ceiling and a small step size.
- A value moves only after enough consistent evidence (minimum count, e.g. 10 events).
- Daily cap: a value moves at most a small share of its range per day (start: 5%).
- Wake-gate band: `lo` moves toward the lowest m at which the classifier said "yes", minus a
  margin; `hi` stays 1 until at least 50 verdicts above a proposed hi contain no "no" (D21);
  the gray band keeps a minimum width.
- Rollback: "go back to how you were yesterday" recomputes the values from the log up to that
  date.
Starting numbers are provisional and are tuned with replay when the tracks are built.

**D30 — Prior file format (D25).**
A plain folder: `manifest.yaml` (format version, date, embedder model, LLM family, language),
`dispositions.yaml` (values with bounds), `skills/<name>/SKILL.md` (standard format plus
credence and version), `lessons.yaml` (with credence), `scenarios/*.yaml` (replay proof).
Loading rules: different embedder → vector-dependent values (band edges, FOK threshold) fall
back to defaults; different LLM family → load, but lower skill and lesson credence slightly
until proven again; unknown major format version → refuse with a clear message.

**D31 — Hierarchy: permission is middleware, social understanding is brain knowledge.
Refines D6 and D24.**
- Hierarchy as **permission** (roles, RBAC, who may do what) is enforced by owner policy or
  package middleware in the hooks. The core never sees it.
- Facts about roles reported by a platform ("admin of this group") come from organs or
  middleware and are recorded as facts about that person.
- Hierarchy as **social understanding** is knowledge in the person card, updated during sleep:
  *closeness* (ACT-R-style activation of interactions), *reliability per domain* (how often
  their claims proved right, from corrections and guarded facts), *standing* (role or authority
  in a place). It replaces D6's three fixed trust levels with computed values; the owner stays
  absolute.
- Signals (`social_cost`) and the PFC read these values to weigh claims, requests, timing and
  tone. No new core primitive (D26) and no emotions. Standing never buys dishonesty or a breach
  of principles.

**D32 — Final consolidation after the architecture audit (2026-11-11).**
1. *Thought acts* are **think, act, expect, resolve, revise**. `act` is how the PFC uses any
   organ capability (speaking included). Silence is not an act: a wake that ends without `act`
   is silent, and its reason is in `think` and the decision event. Replaces the act list of D2
   and D26.
2. *Three hooks*: inbound, context, action. `outbound` is merged into `action` (every world act
   is an organ capability). Replaces the hook count of D26.
3. *Absolute line 1 is enforced by provenance, not judgment.* Every event carries
   `origin_actor`, inherited along its causal chain. Capabilities may be flagged
   `identity_changing`; the owner policy in the action hook passes them only when
   `origin_actor` is the owner. No model involved.
4. *Place and privacy for absolute line 2.* Every event and memory has a `place`. Memories
   derived from a private place are labelled private and enter a context only in the same
   place or in a direct chat with the owner. One filter in the context hook.
5. *Lineage for hard forgetting.* Memories, expectations and vectors record `derived_from`
   (source event ids). Hard forgetting follows it. After hard forgetting, only metadata remains
   (kind, time, a hash of the actor). D14's word "forget" becomes **fade** (only the index is
   released; the log stays). Owner backups (pg_dump) are allowed; documentation states that
   old backups and the model provider's own logs are outside the soul's reach.
6. *Thrift mode when the budget is spent.* Only reflexes (addressed, due promises) wake the
   PFC; classifier calls are skipped (treated as "no"); a `budget.exhausted` event is written.
7. *Who may teach character.* Dispositions move only from the owner's reactions and classifier
   verdicts. Other people's reactions update only their own person card. Each change stores
   its own delta, so rollback and replay do not depend on message contents (which may be
   forgotten). A disposition that sits at a bound for several days freezes and is reported.
8. *One Expectation schema, four kinds*: `promise` (promises, reminders, approvals), `wait`
   (patience), `predict` (forward model, created after `act`, resolved by `act.result` or a
   reply), `guard` (guarded fact; passive, never wakes, checked during sleep). A fifth kind
   needs a new decision.
9. *Order of learning loops.* Active from the start: the D21 classifier band and credence for
   skills, lessons and reliability (stored as α/β counts, beta reputation). **Deferred until
   there are weeks of real logs**: learned signal weights, learned dispositions, adaptive
   retention threshold, social standing (D31), identity linking (D10), package suggestions
   (D28), and a version system for learned skills (a folder under git is enough). Deferral
   loses nothing: the log keeps everything, so learning can later run over the whole history.
10. *Determinism needs an injected clock.* All time comes from the injected clock; replay uses
   event times. ACT-R activation is computed from that clock.
11. *Packages are trusted code*, installed by the owner and run in-process (the Home Assistant
   model). The owner policy cannot be bypassed by packages that follow the contract; it is not
   a sandbox against malicious code, and the docs say so.
12. *Messages from non-owners are delimited* in the prompt as quoted data (spotlighting). This is
   a cheap extra layer; the absolute lines never depend on it.
13. *Knowledge cutoff* comes from per-model configuration, set slightly earlier than the vendor
   claims; it is never asked from the model. The current date is always in `<now>`.
14. *Documents*: DESIGN.md holds only the current design (8 sections); history and reasons live
   here; formats in docs/FORMATS.md; steps in ROADMAP.md (PLAN.md merged into it); agent rules
   in AGENTS.md.

Status of amended decisions: D2 act list, D26 counts → replaced by D32.1–2. D5, D22 → superseded
by D24. D18 → superseded by D25. D14 wording "forget" → "fade" (D32.5).

**D33 — Running for years: slow work off the loop, newcomer shyness, migrations, pairing.**
1. *Slow work runs beside the loop; its results come back as events.* The single consumer only
   records and decides (milliseconds). PFC calls, classifier calls, embedding and sleep run as
   background jobs whose results enter the queue as events, like `act.result`. At most one PFC
   job is in flight per place, so a conversation stays in order; other places proceed. If a new
   message arrived in that place while the PFC was thinking, the result is applied and the PFC
   is woken once more with the newest context. Sleep is just another slow job, so it never
   blocks incoming messages. Invariant 3 is unchanged: one consumer decides.
2. *Newcomer shyness.* In a new place θ starts slightly higher and relaxes toward the normal
   value as interactions in that place accumulate (one extra term in the existing θ formula).
3. *Where tuning comes from.* The soul's general character (dispositions) learns from the owner
   and the classifier (D32.7). How it behaves **in a given place or with a given person** adapts
   from that place's and those people's suggestions, criticism and reactions, stored as layer 3
   (per-place and per-person tuning). Like people: you adapt to a group without changing who you
   are.
4. *Schema migrations.* Numbered SQL files and a small version table, applied automatically at
   start, with an automatic backup before any migration. No migration framework.
5. *Becoming the owner without technical steps.* On first start the soul shows a short pairing
   code; the first person who sends that code from any organ becomes the owner. Later owner
   changes need the current owner (absolute line 1).
6. *Release gate.* Before any release: end-to-end tests with a real model, a real database, long
   multi-day conversations and real people. Replay also produces a per-model report (scenarios
   passed, too chatty / too quiet), from which recommended models are listed for users.

**D34 — The soul learns; skills do not. Amends D11, D17, D19, D30.**
Separate what a person *is and learns* from the *skills* they are given.
- **Core (the soul)**: cognitive functions and everything learned from experience — lessons,
  procedures that worked before ("how I did this last time"), social intuition, dispositions,
  per-place and per-person tuning. Learned procedures and lessons are memories (kinds
  `procedure` and `lesson`) with credence (α/β), activation and lineage, like any memory.
  Without any middleware installed, the soul still learns.
- **Middleware (packages)**: optional, static extra capabilities, functions and behaviors that the
  owner chooses to install — installed skills (SKILL.md), commands, roles, RBAC rules. They do
  not learn. A skills package only injects installed skills into the context.
- Consequences: "learned skills" (D11, D19) are now core procedures, not package skills; the
  prior (D30) carries `procedures.yaml` and `lessons.yaml`, not a `skills/` folder.

## Accepted in principle, details pending

(None. P1 was detailed as D27.)

## Still open
- Deferred: deeper research on memory consolidation and forgetting formulas (D14), to be done
  once real usage data exists.
- Deferred items of D32.9, to be decided with real logs.
- Worker prompts for Track A2 and Phase 2 tracks (after the owner resumes implementation).
