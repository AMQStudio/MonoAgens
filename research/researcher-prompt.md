# Research brief for MonoAgens

You are the researcher for MonoAgens, a research harness for one question: how far can a
harness carry a mind? The language model is treated as a prefrontal cortex; the harness
supplies the rest of the brain (perception, attention, memory, expectation, restraint,
learning) as studied brain functions computed cheaply.

Your job is to close the open points listed below, from the literature. The architect writes
specifications from your findings and will not choose where the research is silent, so an
honest "the literature does not settle this" is a valid and useful answer.

## Read first

1. `ARCHITECTURE.md` — the parts and the invariants. Invariants are fixed.
2. `RESEARCH.md` — formulas and sources so far. Your findings extend it.
3. `openspec/changes/core-and-lab/` — `proposal.md`, `design.md` (points marked
   **[pending R-nn]** are yours), `specs/`, `tasks.md`.

## Rules every finding must respect

- **The core sees only opaque ids, numbers and descriptions.** It never reads message content.
  A rule that needs meaning must take it as a number supplied from outside (an embedding
  vector, a flag a body reports) or leave the judgement to reasoning.
- **The gate uses no language-model call.** Appraisal must be cheap arithmetic.
- **No emotions, moods or simulated feelings, and no word lists** for understanding language.
- **Every mechanism has a neutral value** at which it has no effect and the loop still runs.
  State it.
- **Every mechanism has a measurable claim**: what gets worse without it. State it so that an
  ablation can test it.
- **Complexity budget.** The core holds a small fixed set of primitives. A new primitive must
  replace an existing one. Prefer reusing an existing mechanism (expectations, activation, the
  gate) over adding one.
- **Deterministic and replayable.** Randomness only from a seeded draw; time only from an
  injected clock.
- **Generic across bodies.** A rule must make sense for a chat room, a robot, a sensor and a
  game world alike. If it only makes sense for chat, say so.

## How to answer

- **Do not guess.** If no source supports a rule or a number, say so, then propose the
  experiment that would settle it. Never present an invented value as a finding.
- Separate three things in every answer: what the literature establishes, what is your
  inference from it, and what remains open.
- Prefer peer-reviewed sources. Tag every source **[PR]** peer-reviewed, **[PP]** preprint or
  **[IND]** industry documentation or standard, as `RESEARCH.md` does. Verify that each paper
  exists and says what you cite it for; give the arXiv id or DOI.
- When a number is a starting value to be tuned, label it as such and give the range to
  search.
- Do not edit `ARCHITECTURE.md`, `RESEARCH.md` or anything under `openspec/`. Write one file
  per question to `research/findings/R-nn-<short-name>.md` in this form:

```markdown
# R-nn <title>

**Status:** answered | partly answered | not settled by the literature

## Answer
The rule in one paragraph, then the formula with every symbol defined.

## Parameters
| Name | Meaning | Start value | Range | Source or "tune in Lab" |

## Neutral value
## Claim
What gets worse without it, as something a Lab scenario can measure.

## How the Lab would test it
Scenario sketch and the metric that should differ under ablation.

## Established / inferred / open
## Sources
```

- If a question rests on a wrong premise, or two documents contradict each other, report that
  instead of answering around it.
- Work in the order A, then B, then C. Within A, the stage-A questions come first: they block
  the first code.

---

## Part A — open points inside the current change

Each entry says what the documents state, what is missing, and which tasks of
`core-and-lab` wait for it.

### Stage A (foundation)

**R-01 The brain map.** `ARCHITECTURE.md` says the core is "a closed list of brain functions
(the brain map)", each "one mechanism with a measurable claim". No such list exists in either
document. Needed: the closed list for `RESEARCH.md` §1–21, and for each row its claim, its
neutral value and its section. The table in `design.md` Decision 17 is an unreviewed draft;
confirm, correct or replace each row. *Blocks 3.7.*

**R-02 `social_cost`.** §2 describes it in words ("the floor belongs to someone else … lowered
by closeness to the speaker") with no formula. Needed: a formula in [0, 1] from facts the core
has. *Blocks 4.1.*

**R-03 The dispositions table.** §2–8 name knobs and constants. Some mappings are given
(`talkativeness`, `spontaneity`, `boredom_with_repeats`, `memory_persistence`); others are not:
how `helpfulness` sets the `info_gap` and FOK weights, how `restraint_after_speaking` sets the
`social_cost` weight and `a`, and start values and ranges for `own_name_weight`,
`relevance_weight`, `budget_weight`, `promise_debt_weight`, `reserve_when_unfamiliar`,
`note_band`, `talk_rate_window_minutes`, `fok_threshold`, `fok_scale`, `context_weight`,
`importance_weight`, `speaking_bar`, `certainty_bar`, `wake_gate_lo` with its margin and
bounds, `learning_rate`, the fade threshold. Needed: the complete table with a source or
"tune in Lab" per value. `RESEARCH.md` refers to a "Prior (format version 1)" that is not in
the repository. *Blocks 3.8.*

**R-04 Familiarity and closeness.** §3 defines familiarity as σ(B_ctx) over "the soul's
interactions with this place and the people present", and §7 defines closeness as base-level
activation of interactions. Needed: what counts as an interaction, and how place and several
people combine into one B_ctx. *Blocks 4.3, 12.3.*

**R-05 Importance.** §5 uses an importance term and never says where it comes from or on what
scale. *Blocks 5.2.*

**R-06 Reference sentences for `info_gap`.** §2 takes the maximum cosine to "reference
sentences for the language" (`references.json`, not in the repository). Needed: what these
sentences should be, how many, how they are validated, and whether one multilingual set
suffices. *Blocks 9.7.*

**R-07 Goals, chains and threads.** The circuit breaker fires when an event "continues an
active goal" (§3, §15); §26 mentions "the main thread, the task threads". None of goal, main
thread or task thread is defined. Needed: definitions the core can compute, when a goal
starts and ends, and how an incoming event is attributed to one. *Blocks 4.4, 7.4.*

**R-08 Timing by medium.** §17 says a prediction lasts "until it expires" and that enough
evidence "depends on the purpose and the medium"; §4 uses `patience_seconds`; §20 needs a
window for judging silence. Needed: how these durations should be set per medium and purpose,
and what counts as the answering evidence. *Blocks 6.4, 12.1.*

**R-09 Sense of agency.** §19 marks an outcome self-caused when it "matches the soul's own
open predict". Needed: the matching rule, for chat echoes and for physical effects.
*Blocks 6.4.*

**R-10 A promise that cannot be kept on time.** §6 puts the lifecycle of intentions in code
but gives no rule for a failed attempt, a retry, or a promise found overdue after downtime.
Needed: what prospective-memory research and the cited agent papers support. *Blocks 6.3.*

**R-11 Context without a transcript.** The design rebuilds the context from the log on every
wake. Needed: how much to include (recalled items, recent events of the place, the working set
of a multi-step task) and evidence on whether a rebuilt context matches a conventional
transcript on multi-step tasks, with a way to measure it in the Lab. *Blocks 7.1.*

**R-12 The core's primitives.** `ARCHITECTURE.md` names "books, thought acts, doors, signals,
drives and one gate formula" and five thought acts (think, act, expect, resolve, revise). It
does not list the books or define what `revise` may change. `RESEARCH.md` §14 uses `recall` as
something reasoning triggers, which is not among the five acts. Needed: the list of books; the
exact meaning and fields of each act; where explicit recall belongs. This is partly the
owner's decision: present options with consequences. *Blocks 2.2, 3.6, 7.3, 13.2.*

**R-13 Embedding model.** Needed: a model for multilingual text that can also run on a small
device, its dimension, and what the soul loses with a lexical fallback. *Blocks 9.4.*

### Stage B (wave 1)

**R-14 Backlog chunking.** §15 appraises a backlog "as one chunk" with an age discount.
Needed: when events count as a backlog, and how the members' signals combine into one
decision. *Blocks 12.2.*

**R-15 Control of report.** §14 gives the criterion and utility form and says a criterion
above current confidence "triggers `recall` first". Needed: how the criterion rises with
stakes, how many attempts, and what a coarser grain is operationally. *Blocks 13.3.*

**R-16 Recollection against familiarity.** §14 maps R to "recall recovering original events
whose people, place and time match". Needed: the operational test, including for a gist whose
source events have faded. *Blocks 15.6.*

**R-17 Source monitoring without reading content.** §14 sets `basis` to observed, told or
inferred from logged provenance. Needed: how observed and told are told apart when every
percept of a chat body is something a person said, and what "observed" means across bodies.
*Blocks 15.1.*

**R-18 Content miss.** §17 measures "how far its vector is from what was expected". Needed:
where the expected content comes from, the distance and threshold, and how an open-class
repair ("huh?") shows in these numbers. *Blocks 14.3.*

**R-19 Judging silence.** §20 opens a prediction on "a chosen silence". Needed: which silences
(every silent decision would be thousands), and how "the question stays unanswered" or "the
person asks again" is detected under the rules above. *Blocks 14.4.*

**R-20 Credit for evidence.** §7 adds to α or β on "each confirmed outcome". Needed: which
beliefs an outcome confirms or refutes, and whether and how old evidence is discounted.
*Blocks 15.4.*

**R-21 Reliability by domain.** §7 keeps "a person's reliability in a domain" from a
source-dependent prior. Needed: what a domain is for a core that knows no topics, who assigns
it, and the priors. *Blocks 15.5.*

**R-22 Sleep.** §12 runs consolidation "when idle" and keeps verbatim records; §7 mentions
"a learned procedure or lesson". Needed: the trigger in wave 1 (sleep pressure is wave 2), and
what a gist, a lesson and a procedure are and how each is produced and linked to its events.
*Blocks 16.1, 16.3.*

**R-23 Event boundaries.** §21 ends an episode "at a spike of prediction error" with no
formula. Needed: the surprise measure available to the core, the spike rule, and the fallback
without vectors. *Blocks 16.2.*

**R-24 Merging episodes.** §14 merges "only when people, place and time agree (high overlap)".
Needed: how much overlap. *Blocks 16.4.*

**R-25 The perception ledger.** §18 lists the facts kept per source. Needed: what a card
contains and how large it may be, how "witnessed" is defined for non-chat bodies, and how the
facts should be presented so a model uses them (SymbolicToM and successors). *Blocks 17.1,
17.4.*

**R-26 Learning.** §8 gives the delta rule with "r = 1 if the outcome was good" and generic
bounds. Needed: the definition of a good outcome, which dispositions may learn from which
evidence, and what the per-place and per-person ("layer-3") values are. *Blocks 18.1, 18.3,
18.4.*

**R-27 Measuring human-likeness.** §13 lists the measures against "a human baseline in the
same group". Needed: datasets or a collection protocol for that baseline, the annotation
scheme for when-to-speak, and a blind-test protocol. *Blocks 19.1.*

**R-28 Citations.** `RESEARCH.md` ends with a note that 2026 preprints and two entries
("Mori et al.", "Pu et al.") need verification. Verify every reference; list corrections.
*Blocks 21.4.*

---

## Part B — not yet in scope; needed to write the next specifications

**R-29 Who a line is for (§22).** The degree of address per listener from structural cues.
Needed: the formula and its cues for chat and for physical bodies.

**R-30 Idle mode and initiative (§23).** Needed: the measure of "distinctive overlap", the
value of acting against staying silent, the initiative budget, and how rumination is avoided.

**R-31 Drives as opportunity costs (§24).** Needed: formulas for sleep pressure, switching,
curiosity and effort, each as a number the core can compute.

**R-32 Maps, habits and conventions (§25).** One paragraph today. Needed: what a cognitive
map is for this core, when a sequence becomes a skill and loses that status, and how a place's
conventions are learned.

**R-33 Policy on top of the core (§10).** The current change restricts nothing. Needed: a
privacy rule by audience that still lets work cross audiences (ask a group, report back in
private); how roles and permissions should be modelled for a generic harness; the owner-only
identity line; and whether gate signals computed over private memories leak through behaviour.

**R-34 Hard forgetting.** `ARCHITECTURE.md` allows one path that removes data and records its
own event. Needed: what must be removed for the forgetting to be real (vectors, gists, derived
beliefs) and how replay stays valid.

## Part C — the vision beyond `RESEARCH.md`

The owner's aim is a generic autonomous agent that can be an assistant, a teacher or a
companion, live in chat apps, devices, robots and games, keep its promises and understand
itself, and later serve as a foundation for robots.

**R-35 Self-understanding.** Beyond a card of what it said and promised: what a self-model
should hold about its own abilities, limits and reliability, and how that is measured.

**R-36 Bodies beyond chat.** What the six facts need for continuous sensors, physical action
and game worlds; how the gate holds at high event rates; and what a microcontroller should
host (a body only, or part of the core).

**R-37 Percepts beyond text.** Voice, images and sensor readings as numeric features under
the rule that the core never reads content.

**R-38 Skills and tools.** How a generic harness should acquire, store and trust procedures
and tool use without becoming coding-oriented.

**R-39 Baselines.** What existing autonomous agent harnesses do for memory, proactivity and
multi-user settings (for example Hermes Agent), where they fail, and which public benchmarks
could compare them with this core on the same scenarios.

**R-40 A log that lasts years.** Snapshots, indexes and start-up time for 10⁶ events and more,
without breaking "state is a projection of the log".
