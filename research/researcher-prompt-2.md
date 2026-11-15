# Research brief 2 for MonoAgens

You are the researcher for MonoAgens, a research harness for one question: how far can a
harness carry a mind? The language model is treated as a prefrontal cortex; the harness
supplies the rest of the brain as studied brain functions computed cheaply.

This is the second round. Round 1 is `research/findings/2026-11-15-run-1.md`. It was written
without access to the repository. You now have read access, so this round starts by checking
round 1 against what was built from it, and then covers everything else that is still open,
in one pass.

Nothing from this round changes the change now being built (`core-and-lab`). The architect
folds your findings into the changes that follow it. Say plainly when a finding shows that
something in `core-and-lab` is wrong, so the next change can correct it.

## Read first

Repository: https://github.com/AMQStudio/MonoAgens, branch `main`.

1. `ARCHITECTURE.md` — the parts and the invariants. Invariants are fixed.
2. `RESEARCH.md` — formulas and sources. Not yet corrected for round 1; where it and round 1
   disagree, round 1 and the design below are current.
3. `research/findings/2026-11-15-run-1.md` — your round 1.
4. `openspec/changes/core-and-lab/` — `proposal.md`, `design.md` (Decision 1 holds the owner's
   rulings, Decision 18 the brain map, Decision 19 what is still open), `specs/`, `tasks.md`.
5. `openspec/changes/bootstrap-vertical-slice/` — parked. Its `gateway`, `valve` and `cli`
   specs show what the layer above the core was last thought to be.

The owner's rulings since round 1 (design, Decision 1): the core never knows an owner and
receives per-actor numbers; text search sits behind the store port; predictions have two
kinds, effect and response; the core owns the three hook points and the gateway owns the
rules in them; habituation uses a gain κ; base-level activation uses Petrov's hybrid; explicit
recall is a capability used through `act`; `info_gap` takes its references from an annotated
corpus and stays only if the Lab shows its effect.

## Rules every finding must respect

- The core sees only opaque ids, numbers and descriptions. It never reads message content.
- The gate uses no language-model call.
- No emotions, moods or simulated feelings, and no word lists for understanding language.
- Every mechanism has a neutral value at which it has no effect and the loop still runs.
- Every mechanism has a measurable claim: what gets worse without it.
- The core holds a small fixed set of primitives. A new one must replace an existing one.
- Deterministic and replayable: randomness only from a seeded draw, time only from an
  injected clock.
- Generic across bodies: a chat room, a robot, a sensor and a game world alike. If a rule
  only makes sense for chat, say so.

## How to answer

- **Do not guess.** Where no source supports a rule or a number, say so and propose the
  experiment that would settle it.
- Separate what the literature establishes, what you infer from it, and what remains open.
- Tag every source **[PR]**, **[PP]** or **[IND]**, verify that it exists and says what you
  cite it for, and give the arXiv id or DOI.
- Label a number that is a starting value as such, with the range to search.
- Do not edit the repository. Deliver one Markdown document with one section per question, in
  this form:

```markdown
# <id> <title>

**Status:** answered | partly answered | not settled by the literature

## Answer
The rule in one paragraph, then the formula with every symbol defined.

## Parameters
| Name | Meaning | Start value | Range | Source or "tune in Lab" |

## Neutral value
## Claim
## How the Lab would test it
## Established / inferred / open
## Sources
```

- If a question rests on a wrong premise, or two documents contradict each other, report that
  instead of answering around it.
- Work in the order A, B, C, D. If you cannot reach everything, say what you did not reach.

---

## Part A — check round 1 against the repository

**A-01 Review of the fold-in.** Read `design.md` Decisions 4–12 and 18 and the fourteen specs
against round 1. List every place where the design or a spec states something round 1 did not
say, says it differently, or combines two findings in a way that changes their meaning. For
each, say whether it is acceptable. This is a review, not new research.

**A-02 The brain map against the Lab.** For each of the 22 rows of Decision 18, check that
its claim can be measured with the metrics in `specs/lab/spec.md`. Name the rows whose claim
has no metric, and the metric each would need.

**A-03 Habituation halves the first stimulus.** With `h ← h·γ^Δt + 1` applied at every
stimulus and `S_eff = S / (1 + κ·h)`, a first stimulus has h = 1, so with κ = 1 it is halved
and `S_eff` ≤ 0.5, while θ₀ ranges from 0.35 to 0.85. At default dispositions an unaddressed
event may rarely reach the threshold. Needed: what habituation research says about the
response to a first stimulus; whether the increment should apply after the response is
computed; and a check that, with the start values of R-03, salience can cross the threshold
in the cases the claims require.

**A-04 Is the soul ever not listening?** R-14 counts an event as backlog when it "arrived
while the soul was down or asleep". R-22 and the spec run consolidation in the background and
never delay appraisal. Needed: whether "asleep" should mean anything for incoming events, and
what the sleep literature supports for a system that must stay responsive.

**A-05 Sleep and the open episode.** R-23 closes an episode at a boundary. Neither R-22 nor
R-23 says what happens to an episode that has no boundary yet when sleep starts. Without an
answer, the last conversation before a long silence is never consolidated until something new
arrives.

**A-06 The numbers in R-23.** `s_t = max(PE_time, PE_content, novelty)` compares three
quantities. Needed: how `PE_time` and `PE_content` are scaled so that they are comparable
with novelty, and what the window statistics are before w events exist.

**A-07 Cold starts.** Several rules need history that a new soul or a new place does not
have: the z-score of a content miss against "the place's unrepaired replies" (R-18), the
latency quantiles (R-08), the 95th-percentile gap (R-23), the calibrated values of R-03.
Needed: what each rule does until enough data exists, and how much is enough.

**A-08 The estimator in R-08.** "The q quantile … shrunk toward the prior with n₀
pseudo-observations" is not yet an estimator. Needed: one that is cheap, replayable and
bounded in memory, and how the stakes factor is obtained.

**A-09 Recollection for automatic recall.** R-16 tests an item against "the people", "the
place" and "the claimed grain". For recall that reasoning asks for, the query can carry
these. For the recall that runs at every wake, nothing claims anything yet. Needed: what the
cue is in that case, and how partial matches are treated.

**A-10 What reasoning must supply.** The design adds fields to the thought acts that findings
require: `basis` (R-17), `used_ids` (R-20), `value` and `cost` (R-15), `expected` (R-18),
`importance` (R-05), a domain (R-21). Needed: evidence on which of these a language model
reports reliably, how each should be asked for, and what to do when a model omits one.

**A-11 Lessons and procedures.** R-22 writes a lesson "when two or more episodes share a
prediction error" and a procedure for "an action sequence that reached a goal". Needed: how
two errors are judged to be the same without reading content, the thresholds, and how a
procedure is cut out of an episode.

**A-12 Domains.** R-21 uses "K vector centroids recomputed during sleep". Needed: a
deterministic method, how ids stay stable when clusters move, and what happens to evidence
counted under an old cluster.

**A-13 Open notes of round 1.** Each finding ended with an open point. Settle what can be
settled, and for the rest give the experiment: whether closeness should lower `social_cost`
and whether `social_cost` is redundant with patience (R-02); averaging or minimum (R-04);
calibration of `i_reason` (R-05); nested goals (R-07); how purpose changes the evidence
required (R-08); tolerance per body (R-09); when renegotiating beats retrying (R-10); maximum
or noisy-OR (R-14); the fraction ρ (R-19); λ per domain (R-20); J (R-24); card size (R-25);
teacher weights and which threshold terms may learn (R-26); consent and data availability for
PITCH (R-27); the four unverified citation details (R-28).

**A-14 `info_gap` (R-06).** Round 1 found that the formula detects someone else's request,
not the soul's own curiosity, and that no source says how to build the reference set. Needed:
what this signal should be, in two parts if they are two functions; which annotated
dialogue-act corpora exist for the languages below, with licences; the scoring rule
(centroids or a small head); and how to show it does not act as a word list.

**A-15 Languages.** The soul's first languages are Indonesian and English. Needed: how
EmbeddingGemma-300M at 256 dimensions performs on Indonesian and on mixed Indonesian–English
text, and which evaluation sets can show it.

**A-16 Rebuilt context on multi-party tasks (R-11).** Round 1 found no study. Needed: the
closest evidence since, and a protocol the Lab can run, with the non-inferiority margin
justified.

## Part B — the next cognitive waves

Deepen the round 1 answers where they were marked shallow, to the level of the stage-A
answers: formula, parameters, neutral value, claim, test.

**R-29 Who a line is for (§22).** Including how the graded degree replaces the binary
addressee facts in `social_cost`, exchanges and the circuit breaker.

**R-30 Idle mode and initiative (§23).** Including the measure of "distinctive overlap" that
round 1 left open, and evidence on timing outside programming.

**R-31 Drives as opportunity costs (§24).** Constants were all open. Give the calibration
procedure where no constant exists.

**R-32 Maps, habits and conventions (§25).** Not reached in round 1. What a cognitive map is
for this core, when a sequence becomes a habit and loses that status, how habit and
goal-directed control are arbitrated, and how a place's conventions are learned.

**R-34 Hard forgetting.** Including whether credences must be recomputed and how replay stays
valid.

**R-35 Self-understanding.** What a self-model should hold about the soul's own abilities,
limits and reliability beyond the self card, how it is kept accurate, and how it is measured.

## Part C — the layer above the core

The next change after the core builds the gateway, the valve and the first real bodies. The
parked change shows the last thinking; treat it as a starting point, not as decided.

**R-33 Privacy by audience.** A rule that keeps private content from those who were not part
of it and still lets work cross audiences (ask a group, report back in private). Include how
gate signals computed over private memories are kept from leaking through behaviour, and the
cost of filtering them.

**R-41 Roles and permissions for a generic agent.** One soul serves an owner, a team, family
groups and strangers, across bodies. Needed: which access-control model fits (role-based,
attribute-based, relationship-based, capabilities), what existing policy languages and
standards offer, how a role bound to a place ("everyone in this room") is expressed, and what
deciding "from facts, deterministically" rules out.

**R-42 Approvals and held actions.** When an action should wait for a person's consent, who
may give it, how the wait is represented (the protocol already has a `held` receipt), and
what the human-in-the-loop literature says about asking too often.

**R-43 Memory poisoning and injection.** Round 1 cited attack success rates against an
existing harness. Needed: which defences work for a persistent memory that anyone in a group
can write to, what a write policy should check, and how provenance (`origin_actor`, basis,
reliability) already present in the core can carry it.

**R-44 One person across bodies.** Mapping accounts on different platforms to one person:
what can be proven by a platform, what must be confirmed by a person, and the risks of a
wrong merge.

**R-45 The plugin boundary.** Installed plugins are separate programs speaking the protocol
as messages. Needed: what existing protocols for tools and agents provide and where they
fall short for bodies (percepts, audiences, receipts), so that the boundary aligns with
standards developers already use instead of inventing a new one.

**R-46 Routines that check before they speak.** "Every 30 minutes check X; tell me only if
Y." Needed: whether this is fully expressed by expectations with conditions and goals, or
needs something more, and how quiet hours and daily limits fit as policy.

**R-47 Delivery that reads as human.** Typing delay, splitting into several messages,
progress notes. Needed: evidence on what helps, and whether any of it belongs in the core or
all of it in a body.

## Part D — the vision beyond `RESEARCH.md`

**R-36 Bodies beyond chat.** What the six facts need for continuous sensors, physical action
and game worlds; how the gate holds at high event rates; what must run close to the body and
what can run with the core.

**R-37 Percepts beyond text.** Voice, images and sensor readings as numeric features under
the rule that the core never reads content. Include speech in and out as capabilities.

**R-38 Skills and tools.** How a generic harness should acquire, store, trust and retire
procedures and tool use without becoming coding-oriented; how this relates to the procedures
of R-22 and the habits of R-32.

**R-39 Baselines and benchmarks.** Deepen round 1: which public benchmarks to adopt for
memory, prospective memory, proactivity, group chat and theory of mind; what running an
existing harness on the Lab's scenarios requires; and what a fair comparison reports.

**R-40 A log that lasts years.** Snapshots, indexes and start-up time at 10⁶ events and more
without breaking "state is a projection of the log"; vector search beyond brute force.

**R-48 A soul that stays itself.** Over months of learning, consolidation and revised
beliefs, what keeps identity and principles stable, how drift is measured, and how principles
written by an owner are held to by a model.

**R-49 Promises as commitments.** Beyond reminders: commitments that conflict, that depend on
others, or that the soul should refuse to make. What the literature on commitment and
obligation supports, and how keeping one's word is measured over a long run.

**R-50 Becoming a standard.** The owner's aim is a harness others adopt. Needed: what made
comparable protocols and runtimes into de facto standards (a small stable specification,
conformance tests, reference implementations), and what that implies for the protocol, the
Lab and the order of work.
