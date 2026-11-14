# Task 8 — First users and release

Status: todo | Depends on: Task 7

**Goal.** Real users, a release, an honest evaluation, and stable contracts for other
developers. The theory is in the spec (*Rulings*, *Evaluation*, *Plugins*, *Middleware*,
*Adapters*).

---

## Part 1 — First users and the release gate

*Theory:* spec *Rulings*, *Evaluation*, *Layers*.

- [ ] **1.1 The owner alone for a few days.**
  - *Must hold:* every change in this period is followed by a replay check.
- [ ] **1.2 One real group the owner knows well, with the valve on.**
- [ ] **1.3 The release gate.**
  - *Must hold:* end to end with a real model, a real database, long multi-day conversations and
    real people.
  - *Proof:* a replay report per model, the sealed set included; the recommended models are the
    ones that pass.
- [ ] **1.4 A README for technical self-hosters and the license.**
  - *Must hold:* it shows the brain map, the manual install on a server, how to talk to the soul
    and how to tune it with replay; it makes no claim of consciousness and does not present itself
    as made for non-technical users. The license is Apache 2.0.

## Part 2 — Honest human-likeness evaluation

*Theory:* spec *Evaluation*; research *Measuring human-likeness honestly*, *Proving a
mechanism: ablation and sealed sets*.

- [ ] **2.1 The metrics are registered before measuring.**
- [ ] **2.2 Measure against a human baseline in the same group**: reply timing, messages per hour
  against 1/n, when-to-speak precision and recall, wrong silences, promise keeping, recall after
  1, 7 and 30 days, and a blind "which one is the bot".
- [ ] **2.3 Publish the results** as "≥ X% of the human baseline on these N behaviors", with the
  valve's state for each result.

## Part 3 — Stable contracts for developers

*Theory:* spec *The contract*, *Changing the contract*, *Middleware*, *Adapters*, *Plugins*.

Only after the contracts have not changed for a while.

- [ ] **3.1 The contracts for adapters, plugins and the middleware API are versioned.**
- [ ] **3.2 Examples and a contribution guide** exist for each contract.

**Done when.** The release gate passes with recommended models, the evaluation results are
published, and the contracts are documented with examples.

**Notes.**
