# Task 7 — Core: depth of mind

Status: todo | Depends on: Task 6. Parts 1–2 start now; Parts 3–7 after some weeks of daily use.

**Goal.** The soul runs whole and on its own, with a valve the owner controls; every mechanism can
be switched to a neutral value and proven by switching it off. Then the wave 1 mind: it knows what
others have and have not seen, waits for evidence that it was understood, knows its own acts from
the world's, learns from its silences, thinks before it speaks, keeps its beliefs up to date and
lets go of what it should forget. The theory is in the spec (*Layers*, *Mental model*, *Cognition*,
*Memory, sleep and forgetting*) and in research.

---

## Part 1 — Settle what Task 6 left (now)

- [ ] **1.1 Every act opens a prediction; a conversation continues through it.**
  - *Function:* a forward model after acting. Continuation of a conversation comes from the open
    prediction, never from a time window.
  - *Must hold:* after every act the soul holds a prediction of a response from whom it acted
    toward, in that place, until it expires. A line from that source answers it and wakes the
    soul through the circuit breaker. A line from someone else, or from the same source after the
    prediction expired, goes through the gate. No time window decides continuation anywhere.
  - *Proof:* the same person replies after half a minute and after ten minutes, both within the
    prediction's life → both are continuations; another person's line in between → through the
    gate; the same person after expiry → through the gate. Every passing scenario still passes
    with the time window gone.
  - *Neutral:* no prediction is opened.

- [ ] **1.2 Identity has one tunable source.**
  - *Function:* who the soul is lives in one place that the owner can tune like any configuration.
  - *Must hold:* a change to that source is applied at start and on reload. A change made in
    conversation or in the panel is written to the same source, so no route overwrites another.
    Every change is recorded as an identity change, and the soul can say what it was called
    before. With the valve on, only the owner can change it.
  - *Proof:* rename the soul in the source and reload → the next reply uses the new name and,
    asked, the soul says its former name; rename it in conversation → the source shows the new
    name after a restart; a non-owner asks for a new name → refused.

## Part 2 — The valve and neutral values (now)

- [ ] **2.1 The core runs without middleware.**
  - *Function:* the core is the whole soul, fully autonomous.
  - *Must hold:* with no middleware at all and adapters plugged straight in, the soul perceives,
    remembers, thinks, speaks, stays silent, keeps promises and sleeps.
  - *Proof:* the five test bodies run the launch scenarios with an empty middleware; behavior that
    does not depend on the valve passes as it does with the valve.

- [ ] **2.2 The valve is on by default, switched only by the owner, and visible.**
  - *Function:* the middleware as a safety valve.
  - *Must hold:* a fresh install has the valve on. Only the owner can switch it off, wholly or in
    part; no plugin can. Whether it is on is recorded in the log and shown to the owner. With the
    valve off, the core still records who started each chain and who could see each item.
  - *Proof:* a plugin's attempt to remove the valve fails; the owner switches it off → the log and
    the owner's view show it; private content said while it was off is still withheld from the
    wrong audience once it is back on.

- [ ] **2.3 Every built mechanism has a neutral value.**
  - *Function:* switching a mechanism is moving it to or from its neutral value.
  - *Must hold:* each built row of the brain map can be set to its neutral value from the
    middleware's knob lever, and the loop still runs correctly at every combination; no switch
    selects another code path. Learning keeps the learned value apart from plugin offsets: the
    effective knob is the learned value plus active offsets, and learning never reads or moves an
    offset.
  - *Proof:* each row at its neutral value, one at a time and all at once → the soul still runs
    the launch scenarios without errors.
  - *Neutral:* —

- [ ] **2.4 Every built mechanism's claim is measured.**
  - *Function:* ablation, so each mechanism earns its place.
  - *Must hold:* for each built row, the scenarios run once as usual and once with the row at its
    neutral value; the difference is reported against the row's *Without it* claim, with model
    calls per event.
  - *Proof:* an ablation report per row. A row whose removal changes nothing measurable is
    reported as a question for the design, not removed by the task.

## Part 3 — Inner thought, theory of mind and the self

- [ ] **3.1 Think before speaking unasked.**
  - *Function:* inner thought (Inner Thoughts criteria).
  - *Must hold:* when the soul self-selects, it records a thought with its motivation and
    confidence before any message; the message passes only at or above the speaking and certainty
    bars.
  - *Proof:* every self-selected message in replay has a recorded thought before it; weak or
    unsure thoughts end in silence.
  - *Neutral:* bars at their minimum.

- [ ] **3.2 Every thought carries the soul's principles, honesty and knowledge cutoff.**
  - *Must hold:* in every wake the soul knows its principles, the honesty principles and its
    knowledge cutoff for the model in use.
  - *Proof:* asked about something after its cutoff with no search ability, the soul answers as of
    its cutoff and says so.

- [ ] **3.3 The soul knows exactly what it can do.**
  - *Function:* closed-world self-knowledge.
  - *Must hold:* what the soul says it can do matches what is installed and running now, the
    capabilities of every running tool included; what is not there, it says it cannot do. The cards
    of the people present are in view.
  - *Proof:* "what can you do?" lists only what is running; a plugin switched off disappears from
    the answer.

- [ ] **3.4 A perception ledger per source (theory of mind).**
  - *Function:* the facts reasoning needs to infer what someone wants and believes.
  - *Must hold:* for each relevant source, reasoning sees what it witnessed, what the soul told it
    and whether that was understood, its open requests, and what changed after it last witnessed
    it. Inferences about minds are kept as inferred, with credence, never as facts. "Witnessed"
    works for any body (same place, sensor range, game range).
  - *Proof:* a person leaves, something changes, the person returns and asks → the soul takes their
    missed change into account; the soul does not repeat the update to someone who saw it. Small
    alterations of the scenario (names, order, an extra witness) do not lower the result.
  - *Neutral:* cards without the ledger.

- [ ] **3.5 The soul keeps a card for itself.**
  - *Function:* self-model, with the same structure as the cards of others.
  - *Must hold:* reasoning sees what the soul said, promised and claimed, to whom, and what its body
    can do now.
  - *Proof:* across sessions the soul does not retell the same story to the same person, does not
    contradict its own claim without noticing, and does not offer what its body cannot do.
  - *Neutral:* no self card.

## Part 4 — Grounding, agency and silence

- [ ] **4.1 A reply that misses what was expected is noticed.**
  - *Function:* grounding by content.
  - *Must hold:* each prediction holds what content it expects; a reply that arrives but misses it
    is a prediction error that wakes reasoning, which judges the kind of miss (confused, surprised,
    correcting, a new topic) and repairs when needed. The amount of evidence required rises with
    the stakes.
  - *Proof:* the soul states something and the reply shows surprise or confusion → the soul
    clarifies, never "there is no unanswered question"; a plain acknowledgement closes the
    prediction quietly.
  - *Neutral:* predictions check arrival and timing only.

- [ ] **4.2 The soul knows its own acts from the world's.**
  - *Function:* sense of agency.
  - *Must hold:* an outcome that matches the soul's own prediction is marked self-caused and is not
    appraised as a new stimulus; a similar change it did not predict is external.
  - *Proof:* the echo of its own message or action never wakes it as news; the same change made by
    someone else does.
  - *Neutral:* every outcome counts as external.

- [ ] **4.3 Silence is judged too.**
  - *Function:* learning from staying silent.
  - *Must hold:* a chosen silence opens its own prediction; a miss (the question stays unanswered,
    the person asks again) is recorded as a prediction error and feeds learning like any other.
  - *Proof:* in replay, a wrong silence shows as an error in the log and in the report.
  - *Neutral:* silence opens no prediction.

## Part 5 — Revision, beliefs, lessons and procedures

- [ ] **5.1 Corrections supersede; nothing is deleted.**
  - *Proof:* a correction makes the old memory superseded, with its time; both remain in history.
- [ ] **5.2 Every memory knows how it was known** (observed, told, inferred, self-caused); sleep
  summaries are inferred.
  - *Proof:* asked how it knows something, the soul's answer matches the memory's basis.
- [ ] **5.3 Sources win over summaries.**
  - *Must hold:* when the original events and a sleep summary disagree, the events win and the
    summary is revised.
  - *Proof:* a summary that drifted from its events is corrected at the next sleep.
- [ ] **5.4 Episodes merge only when people, place and time agree.**
  - *Proof:* two similar episodes with different people stay apart.
- [ ] **5.5 Important beliefs are checked.**
  - *Must hold:* a contradiction of a guarded belief is a prediction error weighted by the
    belief's credence and the new source's reliability; relevant now → reasoning sees it live;
    otherwise it is settled in sleep.
  - *Proof:* a contradicted belief is revised at the next sleep.
- [ ] **5.6 Lessons and procedures carry credence and travel across adapters.**
  - *Must hold:* each starts from its source's trust and moves with outcomes; generic ones are used
    in every adapter.
  - *Proof:* a lesson learned in one adapter is used in another.

## Part 6 — Trust, fading and forgetting

The fade and hard-forget paths exist since Task 1; this part adds the policy around them.

- [ ] **6.1 Attribution and reliability per domain.**
  - *Must hold:* memories say who said what; reliability is kept per domain; a non-owner never
    supersedes an owner's fact, it becomes a conflicting claim.
  - *Proof:* a stranger cannot override an owner fact.
- [ ] **6.2 Fading with a fixed threshold during sleep.**
- [ ] **6.3 Soft forgetting**: what the person asked to forget leaves retrieval and recent view.
- [ ] **6.4 Hard forgetting removes the shares too.**
  - *Must hold:* hard forgetting leaves only a forgotten mark and metadata, also after a rebuild,
    and removes any share granted for the forgotten content.
  - *Proof:* after hard forgetting and a rebuild, nothing of the content can be recalled or shared.

## Part 7 — Sealed sets and the real model

- [ ] **7.1 A sealed set is written before Part 3 starts** and is not read while Parts 3–6 are
  built: small alterations of passing scenarios, kinds of surprise never seen, the same structure
  in another test body.
- [ ] **7.2 The full test suite and the fake replay pass.**
- [ ] **7.3 Real-model replay** with the model in use: missed-help rate, false-speak rate, wrong
  silences, tokens per simulated day and absolute-line violations (0), against Task 2's numbers;
  the sealed set's result reported apart.
- [ ] **7.4 Ablation report** for the rows built in this task (as in 2.4).

**Done when.** All parts are ticked, the sealed set, both replays and the ablation report are
reported, and the code is readable.

**Notes.**
