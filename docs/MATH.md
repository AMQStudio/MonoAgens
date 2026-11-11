# MATH.md — The Mathematics of MonoAgens and Its Sources

Every formula the core uses, what each symbol means, why it was chosen, and where the idea
comes from. DESIGN.md states *what* the core does; this file states *how* and *why*, so a
scientist can check it and a new session does not have to rediscover it.

Source tags: **[PR]** peer-reviewed · **[PP]** preprint · **[IND]** industry documentation or
standard. Many 2025–2026 agent papers are preprints; treat their numbers as the authors' claims.
Parameters named in `code font` are knobs, calibration values or constants of the prior
(docs/FORMATS.md §2).

---

## 1. Design stance: a quantized brain
We do not simulate neurons. We keep a few brain *functions* whose input–output behavior is
well studied and cheap to compute, in the spirit of resource-rational analysis: cognition as
the best use of limited computation [Lieder & Griffiths 2020, PR]. The cycle "predict → notice
surprise → attend → recall → choose an action at a cost → monitor oneself → consolidate in
sleep" recurs across cognitive architectures [Laird, Lebiere & Rosenbloom 2017, PR;
Sumers et al. 2024 (CoALA), PR]. The LLM plays the prefrontal cortex: it is only woken when
the cheap machinery judges a moment worth it.

## 2. Signals
Each signal is a pure function of the current situation with a value in [0, 1].

- **name_mentioned** ∈ {0, 1}: the soul's name appears in the text without a tag.
- **info_gap** = max_r cos(v, r) over reference sentences r for the language
  (`references.yaml`), where v is the event's embedding. Rationale: curiosity and helping start
  from a perceived information gap [Loewenstein 1994, PR]; embeddings instead of keywords keep it
  language- and style-independent.
- **feeling_of_knowing** = σ((A* − `fok_threshold`) / `fok_scale`), where A* is the highest memory
  activation (§5) among candidates, including recent messages in the place. Rationale:
  metamemory's feeling-of-knowing predicts whether recall will succeed [Nelson & Narens 1990, PR];
  LLMs show usable FOK/JOL signals when they are used for control [arXiv 2605.14186, PP].
- **relevance** = max cos(v, u) over vectors u of active promises and core memory.
- **social_cost** ∈ [0, 1]: combines having spoken recently here, how busy the place is, whether
  someone else already replied after the message, and closeness to the speaker (§7).

σ(x) = 1 / (1 + e^(−x)) throughout.

## 3. Salience, habituation, threshold and the go / no-go gate

**Salience.** S = clamp(Σₖ wₖ·sₖ, 0, 1) over the signals sₖ with weights wₖ (from knobs and
constants: `helpfulness` → info_gap and FOK weights; `name_mentioned_weight`, `relevance_weight`;
`restraint_after_speaking` → the social_cost weight, which is negative).
Rationale: the salience network selects the most relevant stimuli and switches between
inward and outward processing [Menon & Uddin 2010, PR; Menon 2011, PR].

**Habituation.** For each (place, who): h ← h · γ^(Δt in minutes) + 1 at every stimulus;
S_eff = S / (1 + h). γ comes from `boredom_with_repeats` (0 → 0.99/min, 1 → 0.8/min).
Rationale: repeated stimuli without consequence lose their pull — the most basic form of
learning in behavior.

**Threshold (drives, no emotion).**
θ = θ₀ + a · talk_rate + b · budget_used − c · promise_debt + newcomer(place)
- θ₀ from `talkativeness` (0 → 0.85, 1 → 0.35).
- talk_rate = the soul's communicative acts in this place over the last
  `talk_rate_window_minutes`, divided by max(1, active participants). The 1/n reference rate is
  how a fair participant behaves in group chat [Eckhaus et al. 2025, "Time to Talk", PR].
- budget_used = tokens spent today / daily budget (owner setting).
- promise_debt = share of the soul's promises due within the next hour.
- newcomer(place) = `shyness_in_new_places` · max(0, 1 − interactions_here /
  `newcomer_interactions`).
- a from `restraint_after_speaking`; b = `budget_weight`; c = `promise_debt_weight`.
Rationale: homeostatic drives shift the willingness to act without simulating feelings
[Wang et al. 2023, "Humanoid Agents", PR — we keep their basic-needs idea and drop emotion].

**Go / no-go.** P = σ((S_eff − θ) / τ). Draw u ∈ [0, 1) from a generator seeded with
hash(seed, event.id); "go" if u < P. τ from `spontaneity` (0 → 0.02, 1 → 0.2).
P below `note_band` is recorded as silent, otherwise as note when there is no go.
Rationale: action selection as a soft, thresholded competition (a basal-ganglia-style
go / no-go) gives natural variation without if-else rules, and the seed makes every decision
exactly reproducible in replay. A small non-LLM trigger deciding *when* to act outperforms an
LLM deciding it on every event, at a fraction of the cost [arXiv 2605.30152, PP].

**Reflexes** bypass the draw: addressed events, a due promise that passed the wake gate, and
results of the soul's own actions.

## 4. Patience and inner thought

**Patience.** A non-reflex go creates a `wait` Expectation: if someone other than the sender
replies in this place within `patience_seconds`, the decision ends as silent; otherwise the PFC
wakes. Rationale: in multi-party chat the agent should not take the floor when someone else is
about to answer; silent abstention and interruption are the two failure modes to measure
[arXiv 2609.03923 (CAPA), PP; arXiv 2607.03730 (ProACT), PP].

**Inner thought.** On non-reflex wakes the PFC calls think(thought, motivation m ∈ 1..5,
confidence c ∈ [0, 1]). A communicative act passes only if m ≥ `speaking_bar` and
c ≥ `certainty_bar`. The criteria offered to the PFC are the eight heuristics of
[Liu et al. 2025, "Inner Thoughts", CHI, PR]: relevance, information gap, expected impact,
urgency, coherence, originality, balance, dynamics. Their own score is
Σᵢ p(sᵢ) · sᵢ · λ^(t − τ_p) with λ = 1.02 (motivation grows while silent and resets after
speaking); MonoAgens gets the same effect from talk_rate in θ and runs the LLM step only after
the cheap gate.

## 5. Memory activation (ACT-R)

**Base-level activation.** B = ln(Σⱼ tⱼ^(−d)) over the times tⱼ since each use
[Anderson & Schooler 1991, PR; Anderson 2007, ACT-R]. We use the cheap approximation
B ≈ ln(n / (1 − d)) − d · ln(L), with n = use_count + 1 and L = age in hours from the injected
clock [Petrov 2006, PR]. d from `memory_persistence` (0 → 0.7, 1 → 0.3; ACT-R's usual 0.5 in the
middle).

**Total activation.** A = B + `context_weight` · cos(context, memory) + `importance_weight` ·
importance. The cosine term is spreading activation from the current context.

**Retrieval.** Candidates = top 40 by vector ∪ top 20 by text; rank by A; return the top k;
increment use_count. Rationale: power-law forgetting keeps memories that are used often alive
for a long time and lets unused ones fade, unlike exponential decay, which flattens everything
after a month. The original recency × importance × relevance retrieval of Generative Agents
[Park et al. 2023, UIST, PR] is the precursor.

**Fade.** During sleep, per-message vectors whose A falls below a fixed threshold are released;
the gist remains in memories and the event remains in the log (D14, D32.5).

## 6. Expectations and the wake gate

**Prediction error.** An Expectation is a prediction about the world; a miss is a prediction
error that may wake the PFC. This is the engineering core of predictive processing and active
inference [Friston et al. 2016, PR], without the full free-energy machinery. Prospective memory
(remembering to act later) is the least solved part of agent memory, and storing intentions as
typed records whose lifecycle lives in code works better than leaving them to the model
[arXiv 2609.01272 (PIS), PP; arXiv 2611.05731 (Pro-Memory), PP; arXiv 2607.12385 (PM-Bench), PP].

**Free-text conditions.** m = cos(event vector, condition vector).
- m < `wake_gate_lo` → ignore.
- m ≥ `wake_gate_lo` → one classifier call; its verdict is recorded with m.
- `wake_gate_hi` = 1 (no "clearly yes" band) until at least 50 recorded verdicts with m above a
  proposed hi contain no "no".
Rationale: similarity measures "about the same thing", not "answers it" — a question on the
same topic is as similar as its answer (D21).

**Band learning.** `wake_gate_lo` moves toward (lowest m with a "yes" verdict) − margin, within
bounds, by the bounded-update rule of §8.

## 7. Trust, credence and reliability (beta reputation)

Credence of a learned procedure or lesson, and a person's reliability in a domain, are stored as
evidence counts (α, β), starting from a prior that depends on the source (owner high, stranger
low). Expected value E = α / (α + β). Each confirmed outcome adds 1 to α, each failed one 1 to β;
old evidence may be discounted by a forgetting factor [Jøsang & Ismail 2002, Beta Reputation
System, PR]. Rationale: humans keep "epistemic vigilance" towards both the source and the
content of what they are told [Sperber et al. 2010, PR]. Counts update conservatively by nature
(a single event moves E little once α + β is large) and are easy to recompute from the log.

**Closeness** to a person is the ACT-R base-level activation (§5) of the interactions with them.

## 8. Learning with bounds

**Delta rule** (for learned weights, when enabled):
wₖ ← clamp(wₖ + α_learn · (r − P) · sₖ), r = 1 if the outcome was good, 0 otherwise
[Rescorla & Wagner 1972, PR]. α_learn = `learning_rate`.

**Bounded updates** (for knobs, when enabled, D29/D32.7):
- move only after at least N consistent pieces of evidence (start: 10);
- each step ≤ a small share of the knob's range; total daily change ≤ 5% of the range;
- every knob has a floor and a ceiling; a knob that sits at a bound for several days freezes
  and is reported to the owner;
- teachers: the owner's reactions and classifier verdicts only (others tune layer-3,
  per-place and per-person values);
- each change is an event storing its own delta, so rollback and replay never depend on
  message contents.
These numbers are engineering defaults, not results from a study; they are tuned with replay.

## 9. Prompt layering for caching
The prompt is ordered from stable to volatile so the provider's prefix cache can reuse it: a
cache hit requires the whole rendered prefix to match [OpenAI prompt caching docs, IND].
Capability lists are sorted deterministically; `<now>` and the triggering event are last.
In replay with a real model, about 88% of prompt tokens were served from cache.

## 10. Safety mechanics
- **Absolute line 1** (identity changes only from the owner) is enforced by provenance:
  `origin_actor` is inherited along the causal chain and checked in the action hook for
  capabilities flagged `identity_changing`. This is a minimal form of separating control flow
  from untrusted data [Debenedetti et al., CaMeL, arXiv 2503.18813, PP; SaTML 2026, PR].
- **Absolute line 2** (private content never enters another place) is information-flow
  control in the context hook: what is not in the prompt cannot leak.
- Messages from non-owners are delimited as quoted data (spotlighting) [Hines et al. 2024,
  CAMLIS, PR]; this only lowers attack success and is never relied on for the absolute lines
  [Shi et al., arXiv 2505.14534, PP].
- Threat model follows the OWASP Top 10 for LLM Applications 2025 (prompt injection, data and
  model poisoning, excessive agency, unbounded consumption) and memory/context poisoning in the
  OWASP agentic list [IND].

## 11. Grounding in time
The knowledge cutoff comes from per-model configuration, set slightly earlier than claimed,
because effective cutoffs differ from reported ones [Cheng et al., "Dated Data",
arXiv 2403.12958, PP]; time-misaligned context hurts accuracy [EvolveBench, ACL 2025, PR].
The current date is always in `<now>`.

## 12. Sleep and consolidation
Consolidation runs off the critical path, when idle, as in sleep-time compute [Lin et al.,
arXiv 2504.13171, PP]. Raw events are never replaced by summaries: verbatim records beat lossy
extraction on long-term memory benchmarks [arXiv 2601.00821, PP]. Superseding instead of
overwriting (`valid_to`, `superseded_by`) is a minimal bi-temporal model [Zep, arXiv 2501.13956,
PP]; stale beliefs are a known failure of agent memory [STALE, arXiv 2605.06527, PP].

## 13. Measuring human-likeness honestly
Report per behavior against a human baseline in the same group: reply-timing distribution,
messages per hour vs 1/n, when-to-speak precision/recall against human annotation, false-speak
and missed-help rates, promise keeping, recall after 1/7/30 days, and a blind "which one is the
bot" test with a target near 50%. Turing-style tests measure persona skill, not cognition: a
persona-prompted GPT-4.5 was judged human 73% of the time [Jones & Bergen, arXiv 2503.23674, PP].

---

## References
- Anderson, J. R., & Schooler, L. J. (1991). Reflections of the environment in memory.
  *Psychological Science*. [PR]
- Anderson, J. R. (2007). *How Can the Human Mind Occur in the Physical Universe?* Oxford UP.
  (ACT-R) [PR, book]
- Petrov, A. A. (2006). Computationally efficient approximation of the base-level learning
  equation in ACT-R. *ICCM*. [PR]
- Park, J. S., et al. (2023). Generative Agents: Interactive Simulacra of Human Behavior.
  *UIST*. arXiv 2304.03442. [PR]
- Liu, X. B., et al. (2025). Proactive Conversational Agents with Inner Thoughts. *CHI*.
  arXiv 2501.00383. [PR]
- Eckhaus, N., Berger, U., & Stanovsky, G. (2025). Time to Talk: LLM Agents for Asynchronous
  Group Communication in Mafia Games. *Findings of EMNLP*. arXiv 2506.05309. [PR]
- Sumers, T., Yao, S., Narasimhan, K., & Griffiths, T. (2024). Cognitive Architectures for
  Language Agents. *TMLR*. arXiv 2309.02427. [PR]
- Laird, J., Lebiere, C., & Rosenbloom, P. (2017). A Standard Model of the Mind. *AI Magazine*.
  [PR]
- Lieder, F., & Griffiths, T. (2020). Resource-rational analysis. *Behavioral and Brain
  Sciences*. [PR]
- Friston, K., et al. (2016). Active inference and learning. *Neuroscience & Biobehavioral
  Reviews*. [PR]
- Menon, V., & Uddin, L. Q. (2010). Saliency, switching, attention and control: a network model
  of insula function. *Brain Structure and Function*. [PR]
- Menon, V. (2011). Large-scale brain networks and psychopathology: a unifying triple network
  model. *Trends in Cognitive Sciences*. [PR]
- Loewenstein, G. (1994). The psychology of curiosity. *Psychological Bulletin*. [PR]
- Nelson, T. O., & Narens, L. (1990). Metamemory: a theoretical framework and new findings.
  *Psychology of Learning and Motivation*. [PR]
- Rescorla, R. A., & Wagner, A. R. (1972). A theory of Pavlovian conditioning. In *Classical
  Conditioning II*. [PR]
- Jøsang, A., & Ismail, R. (2002). The Beta Reputation System. *Bled eConference*. [PR]
- Sperber, D., et al. (2010). Epistemic Vigilance. *Mind & Language*. [PR]
- Wang, Z., et al. (2023). Humanoid Agents. *EMNLP System Demonstrations*. arXiv 2310.05418. [PR]
- Debenedetti, E., et al. (2025/2026). Defeating Prompt Injections by Design (CaMeL).
  arXiv 2503.18813; *SaTML 2026*. [PP → PR]
- Hines, K., et al. (2024). Defending Against Indirect Prompt Injection Attacks With
  Spotlighting. *CAMLIS*. arXiv 2403.14720. [PR]
- Shi, C., et al. (2025). Lessons from Defending Gemini Against Indirect Prompt Injections.
  arXiv 2505.14534. [PP]
- Lin, K., et al. (2025). Sleep-time Compute: Beyond Inference Scaling at Test-time.
  arXiv 2504.13171. [PP]
- Rasmussen, P., et al. (2025). Zep: A Temporal Knowledge Graph Architecture for Agent Memory.
  arXiv 2501.13956. [PP]
- Chhikara, P., et al. (2025). Mem0. arXiv 2504.19413. [PP]
- Cheng, J., et al. (2024). Dated Data: Tracing Knowledge Cutoffs in Large Language Models.
  arXiv 2403.12958. [PP]
- EvolveBench (2025). *ACL*. [PR]
- Jones, C., & Bergen, B. (2025). Large Language Models Pass the Turing Test. arXiv 2503.23674.
  [PP]
- Hu, Y., et al. (2025). Memory in the Age of AI Agents (survey). arXiv 2512.13564. [PP]
- Always-On Agents: A Survey of Persistent Memory, State, and Governance in LLM Agents (2026).
  arXiv 2606.30306. [PP]
- Do Proactive Agents Need an LLM to Decide When to Act? (TGL, 2026). arXiv 2605.30152. [PP]
- Speak for Me (CAPA, 2026). arXiv 2609.03923. [PP]
- ProACT (2026). arXiv 2607.03730. [PP]
- DiscussLLM (2025). arXiv 2508.18167. [PP]
- LLMs Know When They Know, but Do Not Act on It: A Metacognitive Harness (2026).
  arXiv 2605.14186. [PP]
- Prospective Intention Store (PIS, 2026). arXiv 2609.01272. [PP]
- Pro-Memory (2026). arXiv 2611.05731. [PP]
- PM-Bench (2026). arXiv 2607.12385. [PP]
- Fidelity Before Structure (2026). arXiv 2601.00821. [PP]
- STALE (2026). arXiv 2605.06527. [PP]
- OWASP Top 10 for LLM Applications 2025; OWASP Top 10 for Agentic Applications. [IND]
- OpenAI, Prompt caching guide. [IND]

Author names and venues for 2026 preprints are given as found during research; verify against
arXiv before citing them formally.
