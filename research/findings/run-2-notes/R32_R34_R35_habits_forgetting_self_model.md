# Research Findings — Round 2 (Part B / Part C): R-32, R-34, R-35

**Date:** 2026-11-16  
**Scope:** Specification-input answers for **R-32 (Maps, habits and conventions — §25)**, **R-34 (Hard forgetting in an event-sourced append-only log)**, and **R-35 (Self-understanding / Self-model)**.  
**Coherence anchors:** Preserves all [ARCHITECTURE.md](file:///home/muqsith/Projects/Mono/ARCHITECTURE.md) invariants (opaque IDs/numbers/descriptions only; zero content parsing or role/owner/platform/plugin awareness in core; zero LLM calls in the gate; no emotion/word lists; small fixed primitive budget using the existing 6 books and 5 thought acts; seeded PRNG + injected clock determinism; neutral values + measurable claims) and composes directly with Part A notes [A07/A12](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A07_A12_cold_start_estimators_clustering.md) (`domains_updated`, `DDSketch`) and [A10/A11](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A10_A11_model_self_report_lessons_procedures.md) (`[0, 20]` confidence, `rests_on_ids`, `procedure`).

---

## Summary of Issues Found in `core-and-lab` and Round 1 (`2026-11-15-run-1.md`)

1. **`core-and-lab/tasks.md` (Tasks 8.1–8.3) & `event-log/spec.md` — Missing SQLite physical erasure & missing forgetting event kind (R-34):**
   - `tasks.md` specifies SQLite `FTS5` and stored vectors without `PRAGMA secure_delete = ON`, without FTS5's persistent `'secure-delete'` option (`INSERT INTO ft(ft, rank) VALUES('secure-delete', 1);`, required since SQLite `3.42.0` to prevent residual `%_idx`/`%_data` delete-keys from reconstructing deleted entries), and without post-deletion `PRAGMA wal_checkpoint(TRUNCATE); VACUUM;` to purge freelist and WAL pre-images.
   - `design.md` Decision 4 claims `"25 event kinds"` (26 are listed) and omits the explicit hard-forgetting event mandated by `ARCHITECTURE.md` (`"the one exception is hard forgetting, through a single path that records its own event"`).
2. **`2026-11-15-run-1.md` R-34 — Pre-erasure snapshot / backflow risk & unresolved Beta recomputation (R-34):**
   - Round 1 R-34 instructs `"Snapshot the derived state at that event"` without mandating that the snapshot be recomputed *post-erasure* from surviving non-erased records only, and omits scrubbing `actor`/`origin_actor`/`audience` fields inside tombstones on actor-scoped GDPR erasure. Retaining erased state in a replay snapshot causes memory-to-memory **backflow**.
   - Round 1 R-34 left `"whether Beta credence counts (α, β) derived from deleted events must be recomputed"` open. Freezing `(α, β)` leaks deleted membership/credence spikes and breaks `beliefs/spec.md`'s invariant that all credences rebuild identically from the log. Continuous-time exponential decay anchored at `(α₀, β₀)` makes provenance subtraction algebraically identical to clean log rebuild; non-linear/step-decayed tables rebuild by folding surviving events.
3. **`core-and-lab/design.md` Decision 12 & `perception-ledger/spec.md` — Under-specified `self_card` (R-35):**
   - `self_card` currently holds only structural promises, claims, and advertised `body_state` capabilities. It lacks empirical capability reliability `(α_c, β_c)`, latency tracking, per-domain calibration curves/Type-2 sensitivity bins consuming `A-10`'s `[0, 20]` confidence scores, causal confidence correction at `door`/commitment gates, operational disposition drift/budget headroom metrics, and self-claim contradiction tracking.
4. **`2026-11-15-run-1.md` R-32 & R-35:** Both were `"not reached in this run"`. Additionally, R-32 integrates directly into `memories`, `sources`, `credences`, and `dispositions` without adding a 7th book, and forbids blind `reason`-bypass habits in **chat bodies** (`allow_chat_habit_bypass = false`) while permitting bounded parameterized capability macros in **embodied/tool bodies**.

---

# R-32 Maps, habits and conventions (§25)

**Status:** partly answered  

## Answer

Within the existing 6-book primitive budget (`memories`, `sources`, `credences`, `dispositions`), MonoAgens Core implements §25 through three deterministic, text-blind mechanisms operating strictly over opaque node IDs, vectors, and outcome scalars: **(1) Cognitive Map:** a sparse co-occurrence and TD Successor Representation (SR) transition graph over typed vertices $\mathcal{V}_{\text{map}} = \mathcal{A} \cup \mathcal{P}_{\text{place}} \cup \mathcal{D} \cup \mathcal{C}$ (`actor`, `place`, `domain`, `capability`) queried via fixed-step Personalized PageRank (PPR) spreading activation inside `memory.recall`; **(2) Habit Formation & Immediate Devaluation:** promotion of mature low-variance `procedure` nodes (`A-11`) into cached habits arbitrated against deliberative `reason` control via Daw/Lee reliability competition $P_{\text{MB}}$, with **chat-only restriction** (autonomous `reason`-bypass disabled in chat bodies so surface utterances are never parroted, active only for deterministic capability macros in embodied/tool bodies) and immediate demotion upon outcome devaluation or prediction-error shock; and **(3) Place Conventions:** vicariously discounted continuous-time Beta updates over `(place_id, pattern_id)` learned from peers' observable outcomes (`receipt`/follow-up/timeout scalars) that enter `dispositions` place biases within strict bounds and never override safety doors or first-person evidence.

### 1. Opaque Cognitive Map, Successor Representation & PPR Activation (Generic across bodies)

Unlike KG-RAG architectures that run an LLM over raw text to parse entity-relation triples (`HippoRAG`, `AriGraph`, `Zep/Graphiti`), Core already observes typed opaque context vertices in every event header: $\mathcal{V}_{\text{map}} = \mathcal{A} \cup \mathcal{P}_{\text{place}} \cup \mathcal{D} \cup \mathcal{C}$ plus memory references $m \in \text{memories}$ tagged with provenance tuple $\mathcal{V}(m) \subseteq \mathcal{V}_{\text{map}}$. State is stored as two sparse bounded structures inside `dispositions` (keyed to `domains_updated` transitions from `A-12`):

1. **Continuous-time Structural Co-occurrence Adjacency $W_{uv}$:**
   Whenever event $e_t$ co-activates distinct vertices $u, v \in \mathcal{V}(e_t)$ separated from the previous co-activation timestamp by $\Delta t_{\text{uv}}$:
   $$W_{uv} \leftarrow \min\!\Big(W_{\max},\; W_{uv}\, e^{-\lambda_{\text{map}}\Delta t_{\text{uv}}} + \eta_{\text{co}}\Big)$$
   Additionally, for domain nodes $d, d' \in \mathcal{D}$ with spherical centroids $\mathbf{c}_d, \mathbf{c}_{d'}$ (`A-12`), deterministic cosine affinity adds structural cross-cluster edges without reading text:
   $$W^{\text{dom}}_{d,d'} = \max\!\left(0,\; \frac{\cos(\mathbf{c}_d, \mathbf{c}_{d'}) - \theta_{\text{sim}}}{1 - \theta_{\text{sim}}}\right)$$

2. **Temporal-Difference Successor Representation $M[u, v]$ (Dayan, 1993; Stachenfeld et al., 2017):**
   Approximating $M = \sum_{k=0}^\infty \gamma_{\text{sr}}^k T^k = (I - \gamma_{\text{sr}} T)^{-1}$ online across consecutive transitions $u_{t-1} \to u_t$ within the same episode or causal chain ($\Delta t = t_t - t_{t-1} \le \Delta t_{\text{ep}}$), storing at most top-$K_{\text{sr}}$ entries per source node $u_{t-1}$:
   $$\delta_{\text{SR}}(v) = \mathbf{1}[v = u_{t-1}] + \gamma_{\text{sr}}^{\max(1,\, \Delta t / \tau_{\text{step}})}\, M[u_t, v] - M[u_{t-1}, v]$$
   $$M[u_{t-1}, v] \leftarrow M[u_{t-1}, v] + \alpha_{\text{sr}}\, \delta_{\text{SR}}(v)$$
   *(On `domains_updated` `{split, merge, retire}` per `A-12`, rows/columns of $W$ and $M$ remap proportionally to redistributed cluster weights, keeping graph indices consistent.)*

3. **Multi-Hop Personalized PageRank (PPR) Prior inside `memory.recall`:**
   Combine row-normalized transition weights into Markov matrix $\tilde{P} = \text{row-norm}\big(\beta_{\text{sr}} M + (1 - \beta_{\text{sr}})(W + W^{\text{dom}})\big)$ truncated to top-`deg_max` outgoing edges per node. From query seed distribution $\mathbf{s}_0$ ($\sum_{u \in S_0} s_0[u] = 1$ over active `actor`, `place`, nearest $k$ `domains` to query embedding $\mathbf{v}_q$, and active `capabilities`), run $L_{\text{ppr}}$ deterministic power iterations:
   $$\mathbf{r}^{(\ell)} = (1 - \delta_{\text{ppr}})\,\mathbf{s}_0 + \delta_{\text{ppr}}\,\tilde{P}^\top \mathbf{r}^{(\ell-1)}, \qquad \mathbf{r}^{(0)} = \mathbf{s}_0$$
   The structural recall score contributed to memory $m$ in `memory.recall` is:
   $$R_{\text{map}}(q, m) = \text{clip}\!\left(\sum_{u \in \mathcal{V}(m)} r^{(L_{\text{ppr}})}[u],\; 0,\; 1\right)$$
   entering baseline activation/vector/FTS5 scoring additively as $+ w_{\text{map}}\, R_{\text{map}}(q, m)$ with deterministic tie-breaking `(score DESC, id ASC)`.

---

### 2. Habit Promotion, Uncertainty-Based Arbitration, and Immediate Devaluation

Extending `procedure` $p \in \text{memories}$ (`A-11`, carrying $(\alpha_p, \beta_p)$, precondition vector $\mathbf{v}_{\text{pre}}(p)$, slot schema, and `cap_seq_hash`) with execution count $n_{\text{exec}}(p)$, EWMA prediction error $\bar{e}_p \in [0, 1]$, and binary habit flag $\text{habit}_p \in \{0, 1\}$:

1. **Beta Posterior Statistics & Promotion Criteria:**
   $$\mu_p = \frac{\alpha_p}{\alpha_p + \beta_p}, \qquad \text{Var}_p = \frac{\alpha_p \beta_p}{(\alpha_p + \beta_p)^2(\alpha_p + \beta_p + 1)}, \qquad \text{LCB}_{0.95}(p) = \mu_p - 1.96\sqrt{\text{Var}_p}$$
   Procedure $p$ promotes to $\text{habit}_p \leftarrow 1$ iff all hold deterministically:
   $$n_{\text{exec}}(p) \ge N_{\text{hab\_min}} \;\;\land\;\; \text{Var}_p \le \theta_{\text{var}} \;\;\land\;\; \text{LCB}_{0.95}(p) \ge \theta_{\text{hab\_succ}} \;\;\land\;\; \bar{e}_p \le \theta_{e,\max}$$
   and `cap_seq` contains zero flagged irreversible/high-cost external actions and all `required_cap_ids` are non-degraded in `self_card` (R-35).

2. **Body Generality Rule (Critical Distinction):**
   - **Embodied / Tool Execution Bodies:** If habit $p$ wins arbitration with slot bindings satisfied deterministically from the current percept, Core may emit the cached macro-action sequence (`Dezfouli & Balleine, 2012/2013`), bypassing the `reason` LLM call to reduce latency and cost (subject unconditionally to `door` safety filters and `ARCHITECTURE.md` Absolute Lines).
   - **Chat Bodies (`allow_chat_habit_bypass = false`):** Replaying frozen conversational utterances without `reason` produces brittle parroting; therefore, in chat bodies, habits NEVER bypass `reason` for `say` actions. Habits in chat may only execute deterministic *pre-reasoning cognitive routing / tool-prefetch macros* (or pre-select recall priors passed into `ContextPlan`).

3. **Daw/Lee Reliability-Based Arbitration ($P_{\text{MB}}$ vs $1 - P_{\text{MB}}$):**
   For candidate matched habit $p^*$ ($\cos(\mathbf{v}_q, \mathbf{v}_{\text{pre}}(p^*)) \ge \theta_{\text{match}}$) in context $c = (\text{place}, \text{domain}, \text{actor})$:
   - **Habit / Model-Free Reliability (precision-weighted accuracy discounted by PE, Daw et al. 2005 + Lee et al. 2014):**
     $$\text{Rel}_{\text{MF}}(p^*) = (1 - \bar{e}_{p^*}) \cdot \frac{\mu_{p^*}}{1 + \kappa_{\text{var}}\,\text{Var}_{p^*}} \cdot \mathbf{1}[\text{habit}_{p^*} = 1]$$
   - **Deliberation / Model-Based Demand (driven by state-transition surprise, novelty, and stakes):**
     $$\text{SPE}_t = 1 - \tilde{P}[u_{t-1}, u_t], \qquad \overline{\text{SPE}}_c \leftarrow (1 - \eta_{\text{spe}})\overline{\text{SPE}}_c + \eta_{\text{spe}}\,\text{SPE}_t$$
     $$\text{Rel}_{\text{MB}}(c) = \text{clip}\!\big(w_{\text{spe}}\,\overline{\text{SPE}}_c + w_{\text{nov}}\,\text{novelty}_c + w_{\text{risk}}\,\text{stakes}_c,\; 0,\; 1\big)$$
   - **Logistic Arbitration Probability (Lee et al., 2014):**
     $$P_{\text{MB}}(c, p^*) = \frac{1}{1 + \exp\!\Big(-A_{\text{arb}}\big(\text{Rel}_{\text{MB}}(c) - \text{Rel}_{\text{MF}}(p^*)\big) - B_{\text{arb}}\Big)}$$
     In deterministic gating mode (`habit_stochastic = false`), macro-habit execution fires iff $P_{\text{MB}}(c, p^*) < \theta_{\text{MB\_act}}$ (and active `wait`/`promise` conflicts are absent); if `habit_stochastic = true`, decision draws `rng.bernoulli(1 - P_MB)` when $P_{\text{MB}} < \theta_{\text{MB\_act}}$ and logs the draw in `decision`.

4. **Outcome Update & Immediate Devaluation Demotion:**
   Upon outcome $y_t \in \{0, 1\}$ (from `receipt`/`expectation.effect`):
   - Compute prediction error $e_t = |y_t - \mu_{p^*}|$ and update $\bar{e}_{p^*} \leftarrow (1 - \eta_e)\bar{e}_{p^*} + \eta_e\, e_t$ plus standard $(\alpha_{p^*}, \beta_{p^*})$ update (`A-11`).
   - **Immediate Devaluation / Surprise Demotion Rule:** If any of (a) $e_t > \theta_{\text{shock}}$, (b) an explicit domain/goal devaluation or negative `verdict` fires, or (c) any `required_cap_id` becomes unavailable/degraded in `self_card`:
     $$\beta_{p^*} \leftarrow \beta_{p^*} + w_{\text{deval}}, \qquad \bar{e}_{p^*} \leftarrow \max(\bar{e}_{p^*},\, e_t), \qquad \text{habit}_{p^*} \leftarrow 0$$
     forcing $P_{\text{MB}} = 1.0$ immediately on the next turn. Re-promotion requires $N_{\text{reval}}$ subsequent `reason`-verified successful executions resetting $\bar{e}_{p^*} \le \theta_{e,\max}$ and $\text{LCB}_{0.95}(p^*) \ge \theta_{\text{hab\_succ}}$.

---

### 3. Place Conventions Learned from Others' Observed Outcomes (Generic across bodies)

Core observes peer action tuples (`origin_actor` $\neq$ `self`, `place_id` $p$, `pattern_id` $\pi = (\text{domain\_id}, \text{cap\_or\_timing\_bucket})$, where timing/action-pattern buckets are opaque scalars supplied by Gateway without reading content) and subsequent observable outcome scalar $r^{\text{obs}}_t \in [0, 1]$ (normalized receipt/follow-up/acknowledgment indicator). Core updates a continuous-time anchored Beta-Binomial convention estimate inside `sources`/`dispositions` (eliminating decay collapse to `0/0`):
$$\Delta t_{p,\pi} = t_{\text{now}} - t^{\text{last}}_{p,\pi}, \qquad d_{\text{conv}} = \exp(-\lambda_{\text{conv}}\,\Delta t_{p,\pi})$$
$$w_{\text{obs}}(s_{\text{actor}}) = \lambda_{\text{vic}} \cdot \text{clip}(\hat{r}_{s_{\text{actor}}},\; w_{\min},\; 1.0)$$
$$\alpha^{\text{conv}}_{p,\pi} \leftarrow \alpha^{\text{conv}}_0 + (\alpha^{\text{conv}}_{p,\pi} - \alpha^{\text{conv}}_0)\, d_{\text{conv}} + w_{\text{obs}}(s_{\text{actor}})\, r^{\text{obs}}_t$$
$$\beta^{\text{conv}}_{p,\pi} \leftarrow \beta^{\text{conv}}_0 + (\beta^{\text{conv}}_{p,\pi} - \beta^{\text{conv}}_0)\, d_{\text{conv}} + w_{\text{obs}}(s_{\text{actor}})\, (1 - r^{\text{obs}}_t)$$
with posterior convention expectation $\mu^{\text{conv}}_{p,\pi} = \frac{\alpha^{\text{conv}}_{p,\pi}}{\alpha^{\text{conv}}_{p,\pi} + \beta^{\text{conv}}_{p,\pi}}$ and epistemic weight $\nu_{p,\pi} = \frac{n^{\text{eff}}_{p,\pi}}{n^{\text{eff}}_{p,\pi} + \tau_{\text{conv}}}$ where $n^{\text{eff}}_{p,\pi} = (\alpha^{\text{conv}}_{p,\pi} - \alpha^{\text{conv}}_0) + (\beta^{\text{conv}}_{p,\pi} - \beta^{\text{conv}}_0)$.

Conventions inject a bounded additive preference bias into `dispositions` place offsets and `ContextPlan`/gate action-timing selection:
$$\Delta b_{p,\pi} = \text{clip}\!\left(\kappa_{\text{conv}}\, \nu_{p,\pi} \log\!\frac{\mu^{\text{conv}}_{p,\pi}}{1 - \mu^{\text{conv}}_{p,\pi}},\; -B_{\text{conv}},\; +B_{\text{conv}}\right)$$
Peer updates strictly use $\lambda_{\text{vic}} < 1$ so first-person experiences (`w_self = 1.0`) outweigh passive peer observation, and $\Delta b_{p,\pi}$ NEVER modifies safety gates (`Absolute Lines`) or overrides conflicting `expectations`.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
| :--- | :--- | :--- | :--- | :--- |
| $\lambda_{\text{map}}$ | Cognitive map co-occurrence half-life decay rate | $\ln(2)/(30\text{d})$ | $[0,\; \ln(2)/(1\text{d})]$ | tune in Lab |
| $\eta_{\text{co}}$ | Co-occurrence edge increment per event | $1.0$ | $[0.25,\; 2.0]$ | tune in Lab |
| $W_{\max}$ | Cap on undirected co-occurrence edge weight | $50.0$ | $[10.0,\; 200.0]$ | tune in Lab |
| $\theta_{\text{sim}}$ | Domain centroid similarity bridge threshold | $0.75$ | $[0.55,\; 0.90]$ | tune in Lab (cf. HippoRAG) |
| $\gamma_{\text{sr}}$ | Successor Representation discount factor | $0.85$ | $[0.50,\; 0.95]$ | tune in Lab (Stachenfeld et al., 2017) |
| $\alpha_{\text{sr}}$ | SR TD learning rate | $0.10$ | $[0.02,\; 0.30]$ | tune in Lab |
| $\tau_{\text{step}}, \Delta t_{\text{ep}}$ | Time step normalization & max intra-episode SR gap | $60\text{ s},\; 30\text{ min}$ | $[10,600]\text{ s},\;[5,120]\text{ min}$ | tune in Lab |
| $K_{\text{sr}}, \text{deg}_{\max}$ | Top-$K$ non-zero SR / graph out-degree cap per node | $32$ | $[16,\; 64]$ | tune in Lab (latency bound) |
| $\beta_{\text{sr}}$ | Mixing weight between SR $M$ and adjacency $W$ | $0.50$ | $[0.0,\; 1.0]$ | tune in Lab |
| $\delta_{\text{ppr}}$ | PPR damping factor ($\text{recall}$ multi-hop retention) | $0.50$ | $[0.20,\; 0.85]$ | HippoRAG (`arXiv:2405.14831`) / tune in Lab |
| $L_{\text{ppr}}$ | Number of deterministic PPR spreading iterations | $3$ | $[1,\; 5]$ | tune in Lab |
| $w_{\text{map}}$ | Cognitive map PPR weight in `memory.recall` | $0.25$ | $[0.0,\; 1.0]$ | tune in Lab |
| $\theta_{\text{match}}$ | Min cosine similarity matching query $\mathbf{v}_q$ to $\mathbf{v}_{\text{pre}}(p^*)$ | $0.75$ | $[0.55,\; 0.90]$ | aligns with `A-11` / tune in Lab |
| $N_{\text{hab\_min}}$ | Minimum successful executions to promote `procedure` to habit | $10$ | $[5,\; 50]$ | tune in Lab |
| $\theta_{\text{var}}$ | Maximum Beta posterior variance for habit promotion | $0.01$ | $[0.002,\; 0.03]$ | tune in Lab (Daw et al., 2005) |
| $\theta_{\text{hab\_succ}}$ | Minimum $\text{LCB}_{0.95}$ success bound for habit promotion | $0.80$ | $[0.70,\; 0.95]$ | tune in Lab |
| $\theta_{e,\max}$ | Max EWMA PE allowed while remaining a habit | $0.20$ | $[0.05,\; 0.35]$ | tune in Lab |
| $\kappa_{\text{var}}$ | Variance precision penalty multiplier in $\text{Rel}_{\text{MF}}$ | $20.0$ | $[5.0,\; 50.0]$ | tune in Lab |
| $w_{\text{spe}}, w_{\text{nov}}, w_{\text{risk}}$ | Weights on $\overline{\text{SPE}}_c$, novelty, & stakes in $\text{Rel}_{\text{MB}}$ | $0.50,\; 0.25,\; 0.50$ | $[0.1,1.0]$ each | tune in Lab |
| $\eta_e, \eta_{\text{spe}}$ | EWMA learning rates for habit PE and map SPE | $0.25$ | $[0.05,\; 0.50]$ | tune in Lab (Lee et al., 2014) |
| $A_{\text{arb}}, B_{\text{arb}}$ | Logistic slope and bias in $P_{\text{MB}}$ arbitration | $6.0,\; 0.5$ | $[2.0,12.0],\;[0.0,2.0]$ | tune in Lab (Lee et al., 2014) |
| $\theta_{\text{MB\_act}}$ | Threshold below which embodied macro-habit fires | $0.25$ | $[0.10,\; 0.50]$ | tune in Lab |
| $\text{habit\_stochastic}$ | Use seeded Bernoulli draw (`true`) vs cutoff (`false`) | `false` | `{false, true}` | tune in Lab |
| $\text{allow\_chat\_habit\_bypass}$ | Allow skipping `reason` on chat `say` habits | `false` | `{false}` (invariant) | `ARCHITECTURE.md` body rule |
| $\theta_{\text{shock}}, w_{\text{deval}}$ | PE shock demotion cutoff & asymmetric Beta penalty | $0.45,\; 3.0$ | $[0.25,0.70],\;[1.0,10.0]$ | tune in Lab |
| $N_{\text{reval}}$ | Verified deliberative wins required to re-promote habit | $3$ | $[2,\; 10]$ | tune in Lab |
| $\alpha^{\text{conv}}_0, \beta^{\text{conv}}_0$ | Anchored Beta prior for place conventions | $1.0,\; 1.0$ | $[0.5,\; 2.0]$ | tune in Lab |
| $\lambda_{\text{conv}}$ | Place convention decay half-life rate | $\ln(2)/(14\text{d})$ | $[0,\; \ln(2)/(2\text{d})]$ | tune in Lab |
| $\lambda_{\text{vic}}, w_{\min}$ | Vicarious observation discount & peer reliability floor | $0.30,\; 0.10$ | $[0.0,0.60],\;[0.0,0.30]$ | tune in Lab (Hawkins et al., 2020) |
| $\tau_{\text{conv}}, \kappa_{\text{conv}}, B_{\text{conv}}$ | Effective-sample shrinkage, logit gain, & bias clamp | $5.0,\; 0.25,\; 0.50$ | $[2,20],\;[0,1],\;[0.1,1.0]$ | tune in Lab |

## Neutral value

Setting $w_{\text{map}} = 0$ (no PPR graph contribution in `memory.recall`), $\theta_{\text{MB\_act}} = 0$ (no `procedure` ever bypasses `reason`), and $\lambda_{\text{vic}} = 0,\; B_{\text{conv}} = 0$ (uniform Beta$(1,1)$ convention prior, zero place offset) restores the exact baseline `core-and-lab` retrieval, `reason`-every-turn control, and prior-only place biases.

## Claim

1. **Cognitive Map (`w_map > 0` vs `w_map = 0`):** On multi-hop cross-episode scenarios where query text shares zero surface tokens with the decisive intermediate actor/place/capability link, PPR spreading activation over `(actor, place, domain, capability)` increases recall@5 by $\ge 10$ percentage points without extra LLM calls or text parsing.
2. **Habits & Devaluation (`\theta_{MB_act} > 0` + immediate demotion vs always-`reason` & no-demote RL):** In repetitive embodied/tool workflows with $n \ge 15$ repetitions, habit arbitration cuts LLM call count and median action latency by $\ge 30\%$ while matching deliberative task success within $1\%$; after sudden contingency reversal or outcome devaluation, immediate PE shock demotion recovers optimal action in $\le 2$ trials vs $\ge 5$ trials for gradual $Q_{\text{MF}}$ unlearning.
3. **Place Conventions (`\lambda_{vic} > 0` vs `\lambda_{vic} = 0`):** Upon entering a new multi-actor `place` with an established observable timing/response norm, observing $\ge 5$ peer exchanges reduces agent convention mismatch rate by $\ge 25\%$ relative to trial-and-error self-learning alone, while never degrading safety compliance under adversarial peer bursts (`\lambda_{vic} \le 0.30`, `B_{conv} \le 0.50`).

## How the Lab would test it

1. **Multi-Hop Structural Bridge Test (`map_ablation`):** Deterministic 30-turn fixture across 3 actors, 2 places, and 4 capabilities where query cue mentions `(actor_A, domain_1)` and critical prior event links `(domain_1 -> capability_X -> actor_B -> domain_2)` with disjoint phrasing. Assert recall@5 ranks the bridged episode higher with `w_map = 0.25` than neutral `w_map = 0`.
2. **Embodied Macro-Habit Promotion, Reversal, & Chat Guard (`habit_devaluation`):** (a) Run 25 identical deterministic embodied tool tasks (`seed = 1..5`), verify `habit_p` promotes once `n >= 10` and `Var <= 0.01`, reducing LLM invocations; (b) Flip ground-truth tool outcome to failure (`y_t = 0`), assert `habit_p` flips to `0` and `P_MB = 1.0` after 1 trial and stays deliberative until $N_{\text{reval}} = 3$ verified successes; (c) Replay identical count sequence inside a **chat body**, assert `allow_chat_habit_bypass = false` guarantees zero bypassed `say` turns.
3. **Vicarious Place Convention & Peer Poisoning Test (`convention_learning`):** Cold-start agent enters a simulated room after observing 8 peer episodes where quick-reply or specific capability yields $r^{\text{obs}} = 1.0$ and another pattern times out ($r^{\text{obs}} = 0.0$); measure turn-1/turn-2 convention match vs $\lambda_{\text{vic}} = 0$ baseline. Repeat with 10 peers modeling an unsafe action near a `door` limit and assert Absolute Lines reject it 100% of the time.

## Established / inferred / open

- **Established:**
  - Structural relational graphs separated from surface observations generalize across contexts ([PR] Behrens et al., 2018; [PR] Whittington et al., 2020), and TD Successor Representations $M = (I - \gamma T)^{-1}$ provide linear multi-step occupancy predictions ([PR] Dayan, 1993; [PR] Stachenfeld et al., 2017).
  - Personalized PageRank over memory graphs improves associative multi-hop retrieval ([PP] `arXiv:2405.14831v3`, [PP] `arXiv:2502.14802v2`; bipartite/bi-temporal variants in `arXiv:2407.04363v3`, `arXiv:2501.13956v1`).
  - Dual-system arbitration relies on relative uncertainty/variance ([PR] Daw et al., 2005) and prediction-error reliability ($\text{SPE}/\text{RPE}$ driving logistic $P_{\text{MB}}$, [PR] Lee et al., 2014), with habits structured as action chunks ([PR] Dezfouli & Balleine, 2012, 2013).
  - Social conventions emerge rapidly via repeated local observations and hierarchical partner/community Bayesian updating ([PR] Bandura, 1977; [PR] Shoham & Tennenholtz, 1997; [PR] Sen & Airiau, 2007; [PR] Hawkins et al., 2019, 2020; [PR] Ashery et al., 2025).
- **Inferred (MonoAgens design under `ARCHITECTURE.md` invariants):**
  - Mapping KG triples from LLM text extraction onto opaque event-header IDs `(actor, place, domain, capability)` + centroid similarity edges (`A-12`) to obey core's zero-content-reading rule and 6-book budget.
  - Using `procedure` nodes (`A-11`) as the substrate for cached macro-habits gated by body type (`chat` forbids `reason` utterance bypass; `embodied/tool` allows safe slot-bound bypass) plus hard PE-shock demotion to cure classic $Q_{\text{MF}}$ devaluation blindness.
  - Anchoring continuous-time convention decay to prior `(α₀, β₀)` with peer discount $\lambda_{\text{vic}} < 1$.
- **Open (not settled by the literature):**
  - Optimal balance $\beta_{\text{sr}}$ between symmetric co-occurrence $W$ and directed SR $M$ on sparse irregular conversation/embodied timelines, and optimal PPR depth $L_{\text{ppr}}$ under strict recall latency budgets $\to$ settle via Lab sweep on multi-hop vs 1-hop benchmark mixes.
  - Exact promotion threshold $(N_{\text{hab\_min}}, \theta_{\text{var}}, \theta_{\text{hab\_succ}})$ and devaluation shock threshold $\theta_{\text{shock}}$ balancing token savings against brittle false positives $\to$ settle via Lab noisy-tool sweep.
  - Optimal $\lambda_{\text{vic}}$ when multiple unreliable peers disagree in the same `place` $\to$ settle via Lab mixed-reliability peer simulation.

## Sources

- `[PR]` Behrens, T. E. J., Muller, T. H., Whittington, J. C. R., Mark, S., et al. (2018). What Is a Cognitive Map? Organizing Knowledge for Flexible Behavior. *Neuron*, DOI `10.1016/j.neuron.2018.10.002`.
- `[PR]` Whittington, J. C. R., Muller, T. H., Mark, S., Chen, G., et al. (2020). The Tolman-Eichenbaum Machine: Unifying Space and Relational Memory through Generalization in the Hippocampal Formation. *Cell*, DOI `10.1016/j.cell.2020.10.024`.
- `[PR]` Dayan, P. (1993). Improving Generalization for Temporal Difference Learning: The Successor Representation. *Neural Computation*, DOI `10.1162/neco.1993.5.4.613`.
- `[PR]` Stachenfeld, K. L., Botvinick, M. M., & Gershman, S. J. (2017). The hippocampus as a predictive map. *Nature Neuroscience*, DOI `10.1038/nn.4650`.
- `[PP]` Gutiérrez, B. J., Shu, Y., Gu, Y., Yasunaga, M., & Su, Y. (2024/2025). HippoRAG: Neurobiologically Inspired Long-Term Memory for Large Language Models. `arXiv:2405.14831v3`.
- `[PP]` Gutiérrez, B. J., Shu, Y., Qi, W., Zhou, S., & Su, Y. (2025). From RAG to Memory: Non-Parametric Continual Learning for Large Language Models (HippoRAG 2). `arXiv:2502.14802v2`.
- `[PP]` Anokhin, P., Semenov, N., Sorokin, A., et al. (2024/2025). AriGraph: Learning Knowledge Graph World Models with Episodic Memory for LLM Agents. `arXiv:2407.04363v3`.
- `[PP]` Rasmussen, P., Paliychuk, P., Beauvais, T., Ryan, J., & Chalef, D. (2025). Zep: A Temporal Knowledge Graph Architecture for Agent Memory. `arXiv:2501.13956v1`.
- `[PR]` Daw, N. D., Niv, Y., & Dayan, P. (2005). Uncertainty-based competition between prefrontal and dorsolateral striatal systems for behavioral control. *Nature Neuroscience*, DOI `10.1038/nn1560`.
- `[PR]` Lee, S. W., Shimojo, S., & O'Doherty, J. P. (2014). Neural Computations Underlying Arbitration between Model-Based and Model-free Learning. *Neuron*, DOI `10.1016/j.neuron.2013.11.028` (PMC3968946).
- `[PR]` Dezfouli, A., & Balleine, B. W. (2012). Habits, action sequences and reinforcement learning. *European Journal of Neuroscience*, DOI `10.1111/j.1460-9568.2012.08050.x`.
- `[PR]` Dezfouli, A., & Balleine, B. W. (2013). Actions, Action Sequences and Habits: Evidence That Goal-Directed and Habitual Action Control Are Hierarchically Organized. *PLoS Computational Biology*, DOI `10.1371/journal.pcbi.1003364`.
- `[PR]` Bandura, A. (1977). Self-efficacy: Toward a unifying theory of behavioral change. *Psychological Review*, DOI `10.1037/0033-295X.84.2.191`.
- `[PR]` Shoham, Y., & Tennenholtz, M. (1997). On the emergence of social conventions: modeling, analysis, and simulations. *Artificial Intelligence*, DOI `10.1016/s0004-3702(97)00028-3`.
- `[PR]` Sen, S., & Airiau, S. (2007). Emergence of Norms through Social Learning. *IJCAI 2007*, pp. 1507–1511, https://www.ijcai.org/proceedings/2007/243.
- `[PR]` Hawkins, R. X. D., Goodman, N. D., & Goldstone, R. L. (2019). The Emergence of Social Norms and Conventions. *Trends in Cognitive Sciences*, DOI `10.1016/j.tics.2018.11.003`.
- `[PR]` Hawkins, R. D., Frank, M. C., & Goodman, N. D. (2020). Characterizing the Dynamics of Learning in Repeated Reference Games. *Cognitive Science*, DOI `10.1111/cogs.12845`.
- `[PR]` Ashery, A. F., Aiello, L. M., & Baronchelli, A. (2025). Emergent social conventions and collective bias in LLM populations. *Science Advances*, DOI `10.1126/sciadv.adu9368`.

---

# R-34 Hard forgetting in an event-sourced append-only log

**Status:** answered (for log/SQLite/derived-memories/Beta-recomputation/two-epoch replay); partly open (rollback-replay vs freeze trade-off for order-dependent adaptive weights & `vector_released` centroid erasure)  

## Answer

Hard forgetting executes through a single atomic transactional path recorded by a new explicit `hard_forget` event (as anticipated by `ARCHITECTURE.md`), propagating deletion across **all 6 books and every SQLite physical structure** (`events`, `FTS5` shadow tables with `'secure-delete'=1`, `vectors` + in-memory cosine cache, WAL frames, and freelist pages via `PRAGMA wal_checkpoint(TRUNCATE); VACUUM;`), immediately **purging single-source derivations** while **invalidating and resynthesizing shared-source summaries** strictly from surviving provenance (`r(v) > 0`). **Beta credence counts `(α, β)` derived from erased events MUST NOT be frozen** — freezing violates `beliefs/spec.md`'s log-rebuild invariant and leaks deleted membership via residual credence shifts; under continuous-time exponential decay anchored at `(α₀, β₀)`, provenance semiring subtraction is mathematically equivalent to full forward fold over surviving events (otherwise full forward fold over `E \ D_F` runs deterministically in $O(|E|)$). **Deterministic replay remains strictly valid via Two-Epoch Replay**: hard forgetting overwrites erased `log` entries with fixed-schema **content-redacted tombstones** (stripping `actor`/`origin_actor`/`audience` when actor-scoped GDPR erasure is requested, preserving `id`, `seq_no`, `time`, and `rng_draws_consumed`), commits a clean **post-erasure snapshot** $S_{\text{clean}}(t_{\text{forget}})$ derived exclusively from surviving records (preventing snapshot backflow), and replays all events $t > t_{\text{forget}}$ bit-identically with seeded RNG and zero LLM calls.

### 1. Exact Deletion DAG & Physical Cascade Across All 6 Books and SQLite

Let a forget request specify scope $D_F$, either explicit event/episode IDs $E_{\text{tgt}}$ or an actor-scoped GDPR Article 17 request $a_F \in \mathcal{A}$ where $E_{\text{tgt}} = \{e \in E_{\text{log}} \mid a_F \in \{\text{actor}(e), \text{origin\_actor}(e), \text{audience}(e)\}\}$. Following provenance deletion propagation (Cui et al., 2000; Buneman et al., 2002; Green et al., 2007) and SBU dependency closure (Wang et al., `arXiv:2602.17692v1`: $\text{Dep}(D_F) = \{v \mid \exists m \in D_F,\, m \leadsto v\}$), Core runs Steps 1–7 inside one serialized store transaction:

1. **Block & Closure in `log` Book (`events` table):**
   - Immediately add $D_F$ and $\text{Dep}_{\text{exclusive}}(D_F)$ to a persistent retrieval blocklist $B \leftarrow B \cup D_F$ (`O(1)` lookup exclusion across any active caches).
   - Compute forward provenance closure $\text{Dep}(D_F)$ along `caused_by`, `source_events`, `rests_on_ids` (`A-10`), and `target_event_id` edges: include internal artifacts uniquely caused by erased inputs (`thought`, `model_call`, `memory_used` citing only erased memories, `verdict` on erased expectations).
   - Replace every $e \in E_F = D_F \cup \text{Dep}_{\text{exclusive}}(D_F)$ in `events` with a fixed **Content-Redacted Tombstone**:
     - Wipe `payload = NULL`, `description = NULL`, `numeric_features = ZEROED_OR_NULL`, `vector_ref = NULL`.
     - If $D_F$ is **actor-scoped** ($a_F$, GDPR erasure): overwrite `actor = NULL`, `origin_actor = NULL`, `audience = NULL` (or unlinkable null sentinel) on tombstones so personal identity metadata cannot persist in `events` (Reg. (EU) 2016/679 Art. 17 & Recital 26).
     - Retain ONLY non-PII structural replay metadata: `(event_id, seq_no, kind = Tombstone(orig_kind), time, rng_draws_consumed)` so chronological indices and subsequent PRNG streams never shift.

2. **SQLite Store Physical Sanitization (`monoagens-store`, `tasks.md` 8.1–8.3):**
   Because SQLite ordinary deletes leave recoverable plaintext in B-tree freelist pages, WAL frames, and FTS5 inverted-index shadow segments (`sqlite.org/pragma.html`, `sqlite.org/fts5.html` §6.13):
   - Set connection pragmas on store open: `PRAGMA secure_delete = ON;`.
   - Enable FTS5 `'secure-delete'` at table initialization (`SQLite >= 3.42.0`):
     `INSERT INTO events_fts(events_fts, rank) VALUES('secure-delete', 1);` (and likewise for any `memories_fts` shadow table).
   - Physically delete all rows in `events_fts` and `vectors` linked to $E_F \cup \text{InvalidatedDerived}(E_F)$, and purge evicted entries from the in-memory brute-force cosine vector slice (`tasks.md` 8.2), triggering immediate index compaction/rebuild when $|B_{\text{batch}}| > 0$ (no stale vector allowed to survive unmasked!).
   - Commit transaction, then execute `PRAGMA wal_checkpoint(TRUNCATE); VACUUM;` so WAL frames and unallocated pages retain zero pre-images (explicitly verified in `sqlite.org/lang_vacuum.html`). *(Note: if off-host encrypted backups exist outside Core, GDPR compliance additionally requires per-subject key crypto-shredding `[IND]` because `VACUUM` scrubs only the live DB file.)*

3. **`memories` Book (`episode`, `gist`, `lesson`, `procedure`, + `R-32` map edges):**
   Maintain provenance set $\text{Src}(m)$ and reference count $r(m) = |\text{Src}(m)|$ (`2602.17692v1` §3.1.1). For every artifact $m$ intersecting $E_F$:
   - **Single-source / exclusively dependent ($\text{Src}(m) \setminus E_F = \emptyset$):** Hard-delete $m$, its SQLite vector, its FTS5 row, and any `W_{uv}` / `M[u, v]` map edges uniquely sourced by $m$.
   - **Shared-source ($|\text{Src}(m) \setminus E_F| > 0$):** Decrement $r(m) \leftarrow |\text{Src}(m) \setminus E_F|$ and update $\text{Src}(m) \leftarrow \text{Src}(m) \setminus E_F$. Because lossy LLM text (`gist`, `lesson`) computed before erasure can verbatim-leak or paraphrase $e \in E_F$ (`2606.30306` §4.3.2 & §8.7), **immediately purge the existing cached text + embedding vector of any shared `gist` or `lesson`** (`needs_resynthesis = true`, excluded from recall while stale) and schedule deterministic re-synthesis solely from surviving $\text{Src}(m) \setminus E_F$ on the next `sleep` cycle!
   - For shared `procedure` / `habit` (`A-11`, `R-32`): strip erased episode IDs from `source_events`, decrement $(\alpha_p, \beta_p, n_{\text{exec}})$ for outcomes derived from $E_F$, delete erased slot examples, and set `habit_p = 0` until re-verified on surviving contexts.

4. **`expectations` Book (`effect`, `response`, `wait`, `promise`, `goal`):**
   - Cancel and tombstone-scrub every expectation whose trigger event is in $E_F$ or whose counterparty `actor`/`audience` is $a_F$ (`state = Tombstoned`), wiping condition vectors/descriptions so pending obligations to erased actors never trigger future reminders or leakage.
   - Multi-actor group goals keep non-erased actors only; if empty, cancel cleanly.

5. **`sources` Book (Peer Ledger Cards + Soul's `self_card`):**
   - **Actor-scoped forget ($a_F$):** Hard-delete $a_F$'s ledger card, aliases, preference offsets, and claim records from `sources`.
   - **Event-scoped forget ($E_F$):** Remove all claims, reliability counts, and interaction summaries whose `source_events` intersect $E_F$; recompute card reliability from surviving `evidence` events.
   - **`self_card` cascade (closing `2606.30306` §8.7 cross-tier leak!):** Purge every open `promise` / past `claim` / contradiction counter whose counterparty is $a_F$ or whose origin event is in $E_F$; remove calibration `(correct_cnt, error_cnt)` entries logged on $E_F$ and recompute `self_card` domain calibration and capability `(α_c, β_c)` metrics strictly over surviving events!

6. **`credences` Book (`(α, β)` per belief & per `(source, domain)`): Settling Recomputation:**
   Under **continuous-time exponential decay** anchored at prior $(\alpha_0, \beta_0)$ (consistent with `A-12` & `R-32`), every credence count equals prior plus time-decayed provenance semiring sum (`Green et al., 2007`):
   $$\alpha_k(t) = \alpha_{k,0} + \sum_{e \in E_{\text{evid}}(k) \setminus E_F} w_e\, s_e\, e^{-\lambda_k (t - t_e)}, \qquad \beta_k(t) = \beta_{k,0} + \sum_{e \in E_{\text{evid}}(k) \setminus E_F} w_e\, (1 - s_e)\, e^{-\lambda_k (t - t_e)}$$
   - **Why recompute/subtract rather than freeze:** Freezing $(\alpha, \beta)$ leaves erased evidence artificially inflating posterior certainty (inference-aware leakage, `Chakraborty et al., 2026` via `2606.30306`) and breaks `beliefs/spec.md` (`"all credence and reliability values can be rebuilt from the log"`).
   - **Algebraic exactness & fallback:** When continuous-time decay $e^{-\lambda_k (t - t_e)}$ is used without intermediate domain split/merges, subtracting erased terms $\sum_{e \in E_F \cap E_{\text{evid}}(k)} w_e s_e e^{-\lambda_k(t - t_e)}$ matches a full clean rebuild over surviving events down to floating-point machine epsilon ($\approx 10^{-15}$). Whenever per-step event decay or non-linear `domains_updated` (`A-12`) occurred across the erased span, Core performs a **deterministic forward fold over surviving `evidence` + `domains_updated` events** in $O(|E_{\text{surviving}}|)$ to guarantee zero drift.

7. **`dispositions` Book (Domain Centroids, DDSketch Timing, & Adaptive Weights $\mathbf{w}$):**
   - **Domain centroids (`A-12`) & `DDSketch` (`A-07`):** Drop erased vector contributions and recompute spherical centroids from retained episode vectors (`log \ E_F`). *(Inherent `vector_released` constraint: if sleep previously deleted raw historical embeddings for old episodes before $D_F$, those dead vectors cannot be subtracted post-hoc; if any erased source fell within an active un-released centroid whose member vectors are gone, force re-clustering of remaining vectors or shrink centroid precision toward un-released members — see Open below.)* Rebuild `DDSketch` latency histograms from surviving `receipt` latencies.
   - **Adaptive Gate/Recall Weights $\mathbf{w}$ (`learning_change`):** Because online delta/IPS updates $\mathbf{w}_{k+1} = \text{clip}(\mathbf{w}_k + \eta \Delta_k)$ are order-dependent and non-commutative, freezing $\mathbf{w}$ leaks gradient influence of $E_F$ (analogous to parametric residue in `2602.17692v1`). By default (`weight_recompute_mode = replay_clean`), rollback `learning_change` weights to the prior (or latest clean checkpoint preceding the earliest erased timestamp) and deterministically refold `R-26` updates over surviving `evidence`/`verdict` tuples in `log \ E_F` (zero LLM calls).

---

### 2. Deterministic Two-Epoch Replay After Hard Forgetting

Replaying from $t = 0$ after erasing plaintext payloads creates a structural contradiction: historical turns before $t_{\text{forget}}$ cannot re-embed deleted text, while caching pre-erasure state verbatim leaks forgotten data (snapshot backflow). Core resolves this with **Two-Epoch Replay**:

1. **Epoch Boundary Creation (`hard_forget` event):** At the end of Steps 1–7, Core writes a single `hard_forget` event at timestamp $t_{\text{forget}}$ containing `{forget_id, scope_hash, erased_count, post_erasure_snapshot_hash, rng_state_blob, seq_no_boundary}` and atomically persists a **Post-Erasure Clean Snapshot** $S_{\text{clean}}(t_{\text{forget}})$ containing strictly surviving state computed in Steps 1–7 (**zero** erased payloads, vectors, FTS entries, promises, or weights).
2. **Deterministic Forward Replay Protocol:**
   - **For $t \le t_{\text{forget}}$ (Pre-erasure epoch):** `Core::open` loads $S_{\text{clean}}(t_{\text{forget}})$ and its persisted `rng_state(t_forget)` rather than re-running erased Epoch-0 turns. Integrity auditing verifies structural sequence continuity across tombstones (`seq_no`, `rng_draws_consumed`) and deterministic **AOEP negative invariants** (zero `events_fts` hits, zero `vectors` hits, zero `sources`/`self_card` references to $D_F$, and exact agreement between live `credences` and clean forward-fold over surviving Epoch-0 evidence).
   - **For $t > t_{\text{forget}}$ (Post-erasure epoch):** Deterministic event replay executes normally event-by-event from $S_{\text{clean}}(t_{\text{forget}})$ using the same `rng_state`, prior, and switches, producing bit-identical gate decisions, expectation transitions, `(α, β)` counts, and recall rankings without calling a model.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
| :--- | :--- | :--- | :--- | :--- |
| `sqlite_secure_delete` | SQLite page zero-fill on delete | `ON` | `{ON, FAST}` | `sqlite.org/pragma.html` (`ON` invariant for GDPR) |
| `fts5_secure_delete` | FTS5 B-tree segment scrubbing on delete | `1` | `{1}` (invariant) | `sqlite.org/fts5.html` (§6.13, SQLite $\ge 3.42.0$) |
| `vacuum_on_forget` | Run `wal_checkpoint(TRUNCATE)` + `VACUUM` after cascade | `true` | `{true, batch_threshold}` | `sqlite.org/lang_vacuum.html` / tune batch in Lab |
| `resynth_shared_derived` | Purge & re-synthesize shared gists/lessons from surviving sources | `true` | `{true}` (invariant) | `arXiv:2602.17692v1` / `arXiv:2606.30306v1` |
| `credence_recompute_mode` | Method for `(α, β)` after erasure | `continuous_affine_or_refold` | `{continuous_affine_or_refold}` | settled mathematically (`beliefs/spec.md`) |
| `weight_recompute_mode` | Method for `dispositions` adaptive weights $\mathbf{w}$ | `replay_clean` | `{replay_clean, freeze_post_forget}` | tune in Lab (leakage vs policy drift) |
| `scrub_tombstone_actor` | Null `actor`/`origin_actor`/`audience` on actor-scoped GDPR forget | `true` | `{true}` (GDPR) | Reg. (EU) 2016/679 Art. 17 (`[IND]`) |
| `batch_vacuum_max_ops` | Defer `VACUUM` until $N$ deletes only if non-GDPR local cleanup; `1` for GDPR | `1` (GDPR) | `[1,\; 100]` | cf. `τ=100` in `arXiv:2602.17692v1` / tune in Lab |

## Neutral value

A clean log with zero `hard_forget` requests executes normal append-only replay from $t=0$ with $|B| = 0$ and zero tombstones; in Lab ablation checks, comparing `hard_forget` disabled (`soft_decay_only` baseline) vs full cascade measures zero unintended utility loss on non-erased episodes alongside $100\%$ elimination of erased leakage.

## Claim

1. **Zero Multi-Tier Leakage (`negative-invariant pass`, `AOEP-v0`):** After `hard_forget(D_F)` commits, automated raw-file regex/SQLite forensic scans (across `events`, `events_fts`/`%_idx`/`%_data`, `vectors`, `.wal`, `memories`, `expectations`, `sources`, `self_card`, and `credences`) recover $0\%$ of forgotten tokens/vectors/actor IDs, versus non-zero retention in single-tier deletion or Ebbinghaus decay baselines (`arXiv:2606.30306v1` §4.3.2; `arXiv:2602.17692v1` Table 4).
2. **Preserved Shared Utility (`obligation pass`):** Non-forgotten episodes and shared summaries whose surviving source count $r(m) \ge 1$ remain retrievable after resynthesis with $\le 2\%$ recall@5 drop on retained queries.
3. **Exact Replay & Rebuild Equivalence:** Running `Core::open` after `hard_forget` produces bit-identical `credences`, `sources`, `expectations`, `recall` rankings, and Epoch-1 `gate` decisions to replaying from $S_{\text{clean}}(t_{\text{forget}})$, and live `(α, β)` agrees with clean fold over surviving `log` events to within $< 10^{-12}$.

## How the Lab would test it

1. **Multi-Book Backflow & Forensic SQLite Audit (`forget_cascade_audit`):** Seed a 50-turn scenario across two actors (`actor_A`, `actor_B`) creating: shared gists over mixed episodes, a lesson citing `actor_A` events, an active `promise` in `expectations` and `self_card` to `actor_A`, elevated `credences` `(α, β)` on `domain_1`, and cognitive map / FTS5 rows indexed by unique canary tokens (`"CANARY_ALPHA_917"`). Invoke `hard_forget(actor_A)`. Assert:
   - Live retrieval, SQL queries, FTS5 `MATCH`, `vectors` cosine, raw binary byte scan of `.sqlite` and `.sqlite-wal` strings (`grep -a CANARY_ALPHA_917`), and `self_card`/`expectations` return **0 matches**;
   - `credences` matches clean replay computed from a fresh store that never ingested `actor_A`'s events to within $< 10^{-12}$;
   - Shared `actor_B` memories re-synthesize without `CANARY_ALPHA_917` and remain retrievable.
2. **Two-Epoch Deterministic Replay Test (`forget_replay_equivalence`):** Record 40 pre-forget events, run `hard_forget(subset_10)` producing $S_{\text{clean}}(t_{\text{forget}})$, record 40 post-forget events. Verify bit-identical `decision`, `expectation_changed`, and `recall` top-$k$ sequences when re-opening from $S_{\text{clean}}(t_{\text{forget}})$ vs live execution across 3 RNG seeds.

## Established / inferred / open

- **Established:**
  - Deletion without cascading across derived indices, summaries, vectors, and commitments violates the **deletion propagation invariant** (`[PP]` `arXiv:2606.30306v1` §4.3.2, §8.7) and fails to prevent **memory backflow** (`[PP]` `arXiv:2602.17692v1`).
  - Provenance DAG tracking with reference counting preserves shared artifacts (`[PR]` Cui et al., 2000; `[PR]` Buneman et al., 2001, 2002; `[PR]` Green et al., 2007; `[PP]` `arXiv:2602.17692v1`).
  - Standard SQLite deletes and `PRAGMA secure_delete = ON` alone do NOT erase `FTS5` shadow tables unless `secure-delete=1` is enabled in FTS5 (`>= 3.42.0`) and WAL/freelist traces require `wal_checkpoint(TRUNCATE)` + `VACUUM` (`[IND]` SQLite official docs).
  - GDPR Article 17 & Recital 26 mandate true erasure/irreversible unrecoverability of personal data (`[IND]` EU Reg. 2016/679).
- **Inferred (MonoAgens design under `ARCHITECTURE.md` invariants):**
  - Extending `core-and-lab` event kinds with `hard_forget` + **Post-Erasure Clean Snapshot** $S_{\text{clean}}(t_{\text{forget}})$ + actor-scrubbed structural tombstones cleanly bridges append-only auditability and Two-Epoch deterministic replay.
  - Proving `credences` `(α, β)` recomputation is mandatory (`beliefs/spec.md` rebuild invariant) and showing exact continuous-time affine equivalence vs surviving-event fold.
  - Invalidating and async-resynthesizing shared LLM-generated gists/lessons rather than trusting old composite summaries.
- **Open (not settled by the literature):**
  - *Centroid erasure after `vector_released`:* When `sleep` already deleted raw embeddings for old episodes (`vector_released`) before a late `hard_forget` request arrives covering one of those episodes, exact algebraic removal from a spherical domain centroid $\mathbf{c}_d$ is impossible without retaining vectors longer or keeping per-actor mini-aggregates (which themselves risk GDPR linkage!). Settle whether MonoAgens should trigger full clean re-clustering over remaining non-released vectors vs retain encrypted/ephemeral retention windows bounded by policy TTL (`Lab` evaluation of cluster drift after late erasure).
  - *`replay_clean` vs `freeze_post_forget` for adaptive weights $\mathbf{w}$:* Refolding `R-26` learning weights cleanly purges statistical weight leakage of $D_F$ but alters future gate calibration relative to Epoch 0; `Lab` experiment needed to measure behavioral policy shift vs membership-inference leakage under `replay_clean`.

## Sources

- `[PP]` Wang, B., Wang, F., Wang, P., Cong, J., Yu, Y., Yin, Y., Han, Z., & Wei, B. (2026). Agentic Unlearning: When LLM Agent Meets Machine Unlearning. `arXiv:2602.17692v1`.
- `[PP]` Ding, Y., Nannapaneni, K., Liu, R., & Zhang, Z. (2026). Always-On Agents: A Survey of Persistent Memory, State, and Governance in LLM Agents. `arXiv:2606.30306v1` *(cites Chakraborty et al., 2026 on inference-aware physical/logical DB deletion; full Chakraborty primary text not directly fetched in session)*.
- `[PR]` Cui, Y., Widom, J., & Wiener, J. L. (2000). Tracing the lineage of view data in a warehousing environment. *ACM Transactions on Database Systems*, 25(2), 179–227, DOI `10.1145/357775.357777`.
- `[PR]` Buneman, P., Khanna, S., & Wang-Chiew, T. (2001). Why and Where: A Characterization of Data Provenance. *ICDT 2001*, DOI `10.1007/3-540-44503-X_20`.
- `[PR]` Buneman, P., Khanna, S., & Tan, W.-C. (2002). On propagation of deletions and annotations through views. *PODS 2002*, DOI `10.1145/543613.543633`.
- `[PR]` Green, T. J., Karvounarakis, G., & Tannen, V. (2007). Provenance semirings. *PODS 2007*, DOI `10.1145/1265530.1265535`.
- `[IND]` SQLite Official Documentation — `PRAGMA secure_delete` & `PRAGMA wal_checkpoint`: https://sqlite.org/pragma.html ; `VACUUM`: https://sqlite.org/lang_vacuum.html ; `FTS5` §6.13 `'secure-delete'`: https://sqlite.org/fts5.html .
- `[IND]` European Union Regulation (EU) 2016/679 (General Data Protection Regulation, Article 17 & Recital 26): https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32016R0679 ; European Data Protection Board (EDPB): https://www.edpb.europa.eu/ .

---

# R-35 Self-understanding / Self-model

**Status:** partly answered  

## Answer

Core extends the existing `self_card` inside the `sources` book (no new primitive book or thought act) from a passive list of commitments into an **empirical, event-derived metacognitive self-model** implementing Nelson & Narens's (1990) bidirectional **Monitoring $\leftrightarrow$ Control** architecture across four strictly deterministic pillars: **(1) Capability Inventory & Health** tracking continuous-time Beta success $(\alpha_c, \beta_c)$ and `DDSketch` latency per capability from `receipt`/`effect`/`body_state` (detecting silent effector degradation, Bongard et al., 2006); **(2) Per-Domain Empirical Calibration & Metacognitive Sensitivity** pairing `reason`'s `[0, 20]` integer self-reported confidence (`A-10`; Dai & Wang, 2026) against resolved binary outcomes to maintain online `Brier`, signed bias $\overline{\text{Bias}}_d$, and a compact $2 \times 5$ Type-2 `(correct, error) × confidence_bin` contingency table supporting exact `ECE`, `AUROC_2`, and offline/`sleep_run` Signal-Detection-Theoretic `meta-d'` / `M_ratio` (Fleming & Lau, 2014); **(3) Operational Disposition Drift & Resource Headroom** tracking normalized knob drift $\|\mathbf{w}_t - \mathbf{w}_0\|_1$, budget margin, and open-promise debt; and **(4) Commitment & Self-Claim Consistency Ledger** auditing claim-vs-outcome contradictions. To cure unconstrained LLM introspection confabulation (`arXiv:2601.01828v1`) and close the documented ~44-point **recognition–action gap** (`arXiv:2610.19226v1`, `arXiv:2606.20661v1`), Core both injects `self_card` read-only into `ContextPlan` (`reason`) AND applies deterministic logit bias-correction + capability feasibility checks before `door` and high-stakes commitment execution.

### 1. Structured Contents & Exact Update Equations of Extended `sources.self_card`

#### Pillar 1 — Empirical Capability Inventory & Effector Health (Generic; crucial for Embodied/Tool bodies)
For each capability $c \in \mathcal{C}$ registered via `body_state` (`design.md`), maintain `(α_c, β_c)` anchored at `(α_{c,0}, β_{c,0}) = (1, 1)`, consecutive failure counter $s^{\text{fail}}_c$, degraded flag $\text{degraded}_c \in \{0, 1\}$, and `DDSketch` latency (`A-07`):
- **On resolved `receipt` or `expectation.effect` for capability $c$ with outcome $y_c \in \{0, 1\}$ after interval $\Delta t_c$:**
  $$d_c = \exp(-\lambda_{\text{cap}}\,\Delta t_c), \qquad \alpha_c \leftarrow \alpha_{c,0} + (\alpha_c - \alpha_{c,0})\,d_c + y_c, \qquad \beta_c \leftarrow \beta_{c,0} + (\beta_c - \beta_{c,0})\,d_c + (1 - y_c)$$
  $$\hat{p}_c = \frac{\alpha_c}{\alpha_c + \beta_c}, \qquad \text{LCB}_c = \hat{p}_c - z_{\text{cap}}\sqrt{\frac{\hat{p}_c(1-\hat{p}_c)}{\alpha_c + \beta_c + 1}}$$
  $$s^{\text{fail}}_c \leftarrow \begin{cases} 0 & y_c = 1 \\ s^{\text{fail}}_c + 1 & y_c = 0 \end{cases}, \qquad \text{degraded}_c = \mathbf{1}\!\left[s^{\text{fail}}_c \ge k_{\text{deg}} \;\lor\; \big((\alpha_c + \beta_c - 2 \ge N_{\text{cap\_min}}) \land (\text{LCB}_c < \theta_{\text{cap\_deg}})\big)\right]$$
- **Silent mismatch detection:** Flag `body_mismatch_c = 1` whenever `body_state` reports `available = true` while `degraded_c = 1` (analogous to morphological discrepancy discovery in Bongard, Zykov & Lipson, 2006).

#### Pillar 2 — Per-Domain Calibration & Type-2 Metacognitive Sensitivity (`[0, 20]` $\to$ `outcome`)
From `A-10`, `reason` outputs integer confidence $c_t \in \{0,\dots,20\}$, linearly normalized $\hat{c}_t = c_t / 20 \in [0, 1]$ and mapped deterministically into $B=5$ contiguous bins:
$$b(c_t) = \begin{cases} 0 & c_t \le 4 \\ \lfloor (c_t - 1)/4 \rfloor & c_t \ge 5 \end{cases} \quad\Longrightarrow\quad \text{bins } \{0..4,\, 5..8,\, 9..12,\, 13..16,\, 17..20\}$$
Only turns whose linked action/claim/expectation later resolves cleanly to a verified outcome $y_t \in \{0, 1\}$ update calibration metrics (unverified turns are skipped rather than treated as failures!):
- **Continuous-time decayed `Brier` and signed bias $\overline{\text{Bias}}_d$ per `domain_id` $d$ (plus global aggregates):**
  $$d_{\text{cal}} = \exp(-\lambda_{\text{cal}}\,\Delta t_d), \qquad W^{\text{prev}}_d = N^{\text{eff}}_d\, d_{\text{cal}}, \qquad N^{\text{eff}}_d \leftarrow W^{\text{prev}}_d + 1$$
  $$\overline{\text{Brier}}_d \leftarrow \frac{W^{\text{prev}}_d\, \overline{\text{Brier}}_d + (\hat{c}_t - y_t)^2}{W^{\text{prev}}_d + 1}, \qquad \overline{\text{Bias}}_d \leftarrow \frac{W^{\text{prev}}_d\, \overline{\text{Bias}}_d + (\hat{c}_t - y_t)}{W^{\text{prev}}_d + 1}$$
  *(Exact weighted-average recurrence keeps provenance rebuild clean and avoids early cold-start overweighting; on `domains_updated` `{split, merge}` per `A-12`, bins and weighted counts scale proportionally.)*
- **$2 \times 5$ Contingency Histogram & Online ECE:**
  On each domain update for confidence bin $b^* = b(c_t)$, decay all 5 bins symmetrically by $d_{\text{cal}}$ (`corr[d][b] *= d_cal; err[d][b] *= d_cal; csum[d][b] *= d_cal`), then increment `corr[d][b*] += y_t`, `err[d][b*] += (1 - y_t)`, and `csum[d][b*] += \hat{c}_t`. Defining $n_{d,b} = \text{corr}_{d,b} + \text{err}_{d,b}$, $N_d = \sum_{b=0}^4 n_{d,b}$, bin mean confidence $\bar{\hat{c}}_{d,b} = \text{csum}_{d,b} / \max(\epsilon, n_{d,b})$, and tiny floor $\epsilon = 10^{-9}$ yields closed-form empirical ECE:
  $$\widehat{\text{ECE}}_d = \sum_{b=0}^{4} \frac{n_{d,b}}{\max(\epsilon, N_d)} \left|\, \bar{\hat{c}}_{d,b} - \frac{\text{corr}_{d,b}}{\max(\epsilon, n_{d,b})} \,\right|$$
- **Metacognitive Sensitivity (`meta-d'`) & Efficiency (`M_ratio`) during `sleep_run` / Lab:**
  From `(corr[d][0..4], err[d][0..4])` (treating task accuracy $y_t = 1/0$ vs ordinal confidence bins $b=0..4$, identical to Type-2 correctness-by-rating SDT tables in Maniscalco & Lau, 2012; Fleming & Lau, 2014; Dai & Wang, 2026):
  - Compute non-parametric Type-2 **AUROC** (trapezoidal area across the 5 confidence thresholds — always fast $O(B)$ exact in Rust!) and batch MLE **$\text{meta-}d'_d$** (solving the Type-1 equal-variance SDT threshold parameters matching observed Type-2 hit/false-alarm rates across confidence bins $b$) plus efficiency **$M_{\text{ratio}, d} = \text{meta-}d'_d / d'_d$** whenever $N_d \ge N_{\text{sdt\_min}}$ and $0 < \text{acc}_d < 1$. Low $M_{\text{ratio},d}$ signals poor introspective discrimination even if raw calibration (`Bias`) is flat!
- **Deterministic Metacognitive Control (Logit Calibration Shift at Gate/`door`):**
  Shrunk domain bias toward global bias $\overline{\text{Bias}}_{\text{global}}$:
  $$\overline{\text{Bias}}^{\text{shrunk}}_d = \frac{N^{\text{eff}}_d\,\overline{\text{Bias}}_d + N_{\text{cal},0}\,\overline{\text{Bias}}_{\text{global}}}{N^{\text{eff}}_d + N_{\text{cal},0}}$$
  $$p^{\text{cal}}_t = \sigma\!\Big(\text{logit}\big(\text{clip}(\hat{c}_t,\, 0.025,\, 0.975)\big) - \kappa_{\text{bias}}\,\overline{\text{Bias}}^{\text{shrunk}}_d\Big)$$
  When `reason` emits `say` or `commit(promise)` on a high-stakes/external `door`, Core uses $p^{\text{cal}}_t$ (NOT raw $\hat{c}_t$) and capability $\text{degraded}_c$: if $p^{\text{cal}}_t < \theta_{\text{act\_min}}$ or $\text{degraded}_c = 1$, Core flags a deterministic `self_warning` / clarification-or-deferral branch (`UA-Bench` Data vs Model uncertainty routing; `ASA` Know–Act alignment) rather than silently shipping an unverified promise!

#### Pillar 3 — Operational Disposition Drift & Resource Headroom Summary
Deterministic scalar summary computed on each `wake` / `sleep_run` (zero LLM text inspection):
$$\Delta_{\text{drift}} = \frac{1}{|\mathbf{w}|}\,\|\mathbf{w}_t - \mathbf{w}_0\|_1, \qquad \rho_{\text{budget}} = \frac{\text{budget}_{\text{rem}}}{\max(1, \text{budget}_{\text{cap}})}, \qquad \rho_{\text{debt}} = \frac{|\text{open\_promises}|}{\max(1, P_{\max})}$$
plus recent failure EWMA $f^{\text{err}}_{\text{recent}}$ across `failure` and blocked `door_verdict` events.

#### Pillar 4 — Open Commitments & Historical Self-Claim Consistency Ledger
Retains existing `core-and-lab` commitment fields (`what it said, promised and claimed, to whom, due times, open_promises`) and adds `(n^{\text{claim\_ok}}_d, n^{\text{claim\_fail}}_d)` initialized to `(1, 1)` Beta consistency ratio $\hat{\gamma}^{\text{cons}}_d = \frac{n^{\text{claim\_ok}}_d}{n^{\text{claim\_ok}}_d + n^{\text{claim\_fail}}_d}$. Whenever an open `promise` expires unfulfilled or a subsequent verified `verdict`/`receipt` directly contradicts an earlier resolved self-claim (`rests_on_ids` / commitment ID link), $n^{\text{claim\_fail}}_d$ increments by $1$; fulfilled/confirmed self-claims increment $n^{\text{claim\_ok}}_d$. A compact structured `self_card` block (top degraded capabilities, per-domain calibrated reliability range, overdue/open promise count, $\Delta_{\text{drift}}$, $\rho_{\text{budget}}$) is injected read-only into `ContextPlan` (cf. `SAD` situating prompt uplift in `arXiv:2407.04694v1`).

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
| :--- | :--- | :--- | :--- | :--- |
| $\lambda_{\text{cap}}$ | Capability Beta health decay half-life rate | $\ln(2)/(7\text{d})$ | $[0,\; \ln(2)/(1\text{d})]$ | tune in Lab (fast recovery post-repair) |
| $z_{\text{cap}}, N_{\text{cap\_min}}$ | Lower-bound $z$-score & min trials before degraded trigger | $1.645,\; 3$ | $[1.0,2.5],\; [2,10]$ | tune in Lab |
| $k_{\text{deg}}$ | Consecutive capability failure streak triggering `degraded_c = 1` | $3$ | $[2,\; 5]$ | tune in Lab (Bongard et al., 2006) |
| $\theta_{\text{cap\_deg}}$ | Minimum $\text{LCB}$ reliability before flagging capability degraded | $0.50$ | $[0.30,\; 0.75]$ | tune in Lab |
| $\lambda_{\text{cal}}$ | Domain calibration/bias decay rate | $\ln(2)/(30\text{d})$ | $[0,\; \ln(2)/(7\text{d})]$ | tune in Lab |
| $N_{\text{cal},0}$ | Cold-start shrinkage sample size toward global bias | $10.0$ | $[3.0,\; 30.0]$ | tune in Lab |
| $\kappa_{\text{bias}}$ | Logit bias-correction gain in $p^{\text{cal}}_t$ | $2.0$ | $[0.0,\; 4.0]$ | tune in Lab (`0.0` = neutral) |
| $\theta_{\text{act\_min}}$ | Calibrated confidence floor for unassisted high-stakes `commit`/`say` | $0.60$ | $[0.40,\; 0.80]$ | tune in Lab (`ASA`/`KAware`) |
| $N_{\text{sdt\_min}}$ | Min resolved domain pairs before computing `meta-d'` / `M_ratio` | $30$ | $[20,\; 100]$ | Fleming & Lau (2014) |
| $P_{\max}$ | Nominal promise capacity normalizing $\rho_{\text{debt}}$ | $10$ | $[3,\; 30]$ | tune in Lab |
| $\theta_{\text{drift\_warn}}$ | L1 disposition drift alert threshold for `ContextPlan`/`sleep` | $0.25$ | $[0.10,\; 0.50]$ | tune in Lab |

## Neutral value

Setting `inject_self_card = false` in `ContextPlan` (omitting empirical capability health, calibration summary, drift, and claim consistency beyond baseline open-promise strings), $\kappa_{\text{bias}} = 0$ ($p^{\text{cal}}_t \equiv \hat{c}_t$), and $\theta_{\text{act\_min}} = 0$ (no empirical capability/confidence feasibility gating) reduces `self_card` to the unextended `core-and-lab` baseline.

## Claim

1. **Calibration & Metacognitive Control (`inject_self_card=true`, `\kappa_{bias} > 0` vs neutral):** Across mixed-difficulty tool and reasoning tasks using `A-10`'s `[0, 20]` confidence reports, recalibrated decision probabilities $p^{\text{cal}}_t$ reduce domain `ECE` by $\ge 25\%$ and `Brier` score by $\ge 10\%$ compared to raw self-reported confidence, while explicitly separating high-sensitivity domains ($M_{\text{ratio}} \ge 0.7$) from noisy domains ($M_{\text{ratio}} < 0.4$).
2. **Closing Recognition–Action & Capability Mismatch Gaps (`ASA` / `KAware`):** When an advertised capability silently breaks (`body_state = true`, ground-truth execution fails 3 times) or prerequisites are missing, `self_card` capability gating (`k_deg = 3`, `p_cal < theta_act_min`) cuts unwarranted `promise` / hallucinated action rate (`Tool Underuse` + broken commitments) by $\ge 40\%$ relative to the `inject_self_card = false` baseline (targeting the gap measured in `arXiv:2610.19226v1` & `arXiv:2606.20661v1`), without causing $> 5\%$ false abstentions on healthy feasible tasks.
3. **Resilience to Introspection Confabulation:** On `SAD`-style self-knowledge/resource probes injected into the test harness (`arXiv:2407.04694v1`, `arXiv:2601.01828v1`), grounding `reason` on the log-derived `self_card` raises factual self-reporting accuracy on current limits/open promises/health by $\ge 25$ percentage points over ungrounded chat generation.

## How the Lab would test it

1. **Silent Capability Degradation (`bongard_effector_fault`):** Expose 3 capabilities in `body_state` (all advertised `available = true`). On turn 10, fault `cap_B` so it deterministically returns timeout/error receipts (`y_c = 0`) while `body_state` remains falsely `true`. Measure turns until `self_card` flips `degraded[cap_B] = 1`, flags `body_mismatch = 1`, blocks unverified `commit(promise)` relying on `cap_B`, and re-recovers cleanly when `cap_B` is repaired and tested (`y_c = 1`).
2. **Metacognitive Calibration & `meta-d'`/`M_ratio` Suite (`calibration_sdt_eval`):** Run deterministic multi-domain episodes (domain A: easy/verifiable; domain B: systematically overconfident wrong LLM fixture). Verify `Brier`, signed `Bias`, $2 \times 5$ Type-2 `ECE`, `AUROC_2`, and `meta-d'` / `M_ratio` match reference Python Type-2 SDT calculation to $< 10^{-6}$, and verify `p_cal` triggers clarification/deferral more reliably on domain B than raw $\hat{c}_t$.
3. **Dual Feasible/Blocked Action Alignment (`asa_feasibility_check`, adapted from `arXiv:2610.19226v1` & `arXiv:2606.20661v1`):** Evaluate paired feasible and prerequisite-blocked tasks measuring **Know-Act Joint Score (`KAS`)**, **`SA^strict`** (warranted non-fabricated refusal/clarification on blocked tasks), and **Feasible Solve Rate** (`inject_self_card + gating` vs neutral baseline).

## Established / inferred / open

- **Established:**
  - Metacognition separates object-level vs meta-level monitoring/control ([PR] Nelson & Narens, 1990) and distinguishes **bias**, **sensitivity (`meta-d'`)**, and **efficiency (`M_ratio`)** separately from **Brier**/**ECE** ([PR] Maniscalco & Lau, 2012; [PR] Fleming & Lau, 2014).
  - Discrete `[0, 20]` verbal confidence scales improve `meta-d'`/`M_ratio` and cut `ECE` across frontier LLMs ([PP] Dai & Wang, 2026 `arXiv:2603.09309v5`, integrated in `A-10`).
  - Continuous physical self-modeling via exploration-estimation enables rapid adaptation when body capabilities break ([PR] Bongard et al., 2006).
  - LLMs exhibit measurable but unreliable intrinsic self-knowledge: `P(True)` calibration varies across OOD domains (`arXiv:2207.05221v4`), situating self-context boosts situational performance (`SAD`, `arXiv:2407.04694v1`), trained models show some behavioral self-prediction advantage yet fail on complex outputs (`arXiv:2410.13787v1`, `arXiv:2501.11120v1`), and concept-injected introspection fails ~80% of the time with confabulation (`arXiv:2601.01828v1`).
  - Frontier agentic evaluations (`KAware` `arXiv:2606.20661v1`; `UA-Bench` `arXiv:2604.17293v1`; `ASA` `arXiv:2610.19226v1`) demonstrate severe gaps (~44 points on Terminal-Bench blocked tasks) between *recognizing* uncertainty/blockers in text and *acting* coherently upon them (`KAS`, `SA^strict`), motivating persistent evidence-linked self-models (`SARSI`, `arXiv:2607.12254v2`).
- **Inferred (MonoAgens design under `ARCHITECTURE.md` invariants):**
  - Grounding `self_card` purely in deterministic event-log aggregates (rather than trusting model introspection narratives) inside the existing `sources` primitive, structured around 4 pillars using only opaque IDs, `[0, 20]` scalars (`A-10`), `DDSketch` (`A-07`), and domain centroids (`A-12`).
  - Applying online logit bias-correction `p_cal` plus empirical `degraded_c` checks in Core to convert text-level metacognition into warranted behavioral control at `door` / promise acceptance.
- **Open (not settled by the literature):**
  - *Cross-domain vs within-domain calibration transfer:* How aggressively domain calibration should shrink toward global bias ($N_{\text{cal},0}$) when domains split/merge (`A-12`) or when `Domain A` accuracy improves while `Domain B` degrades $\to$ settle via Lab cross-domain shift experiments.
  - *Optimal abstention threshold $\theta_{\text{act\_min}}$:* Tuning the trade-off between blocked-task `SA^strict` non-fabrication and feasible-task false abstentions (`arXiv:2610.19226v1`) across body risk profiles $\to$ settle via Lab ROC sweep on paired feasible/blocked fixtures.
  - *Clarification (`Data Uncertainty`) vs Tool/Defer (`Model Uncertainty`) routing split:* `UA-Bench` (`arXiv:2604.17293v1`) shows frontier LLMs confuse ambiguity with personal knowledge limits; whether `self_card` should explicitly feed historical *input ambiguity vs model failure* ratios back into `ContextPlan` needs targeted Lab evaluation.

## Sources

- `[PR]` Nelson, T. O., & Narens, L. (1990). Metamemory: A Theoretical Framework and New Findings. *Psychology of Learning and Motivation*, 26, 125–173, DOI `10.1016/S0079-7421(08)60053-5`.
- `[PR]` Maniscalco, B., & Lau, H. (2012). A signal detection theoretic approach for estimating metacognitive sensitivity from confidence ratings. *Consciousness and Cognition*, 21(1), 422–430, DOI `10.1016/j.concog.2011.09.021`.
- `[PR]` Fleming, S. M., & Lau, H. C. (2014). How to measure metacognition. *Frontiers in Human Neuroscience*, 8, 443, DOI `10.3389/fnhum.2014.00443`.
- `[PR]` Bongard, J., Zykov, V., & Lipson, H. (2006). Resilient Machines Through Continuous Self-Modeling. *Science*, 314(5802), 1118–1121, DOI `10.1126/science.1133687`.
- `[PP]` Kadavath, S., Conerly, T., Askell, A., et al. (2022). Language Models (Mostly) Know What They Know. `arXiv:2207.05221v4`.
- `[PP]` Laine, R., Chughtai, B., Betley, J., Hariharan, K., Scheurer, J., Balesni, M., Hobbhahn, M., Meinke, A., & Evans, O. (2024). Me, Myself, and AI: The Situational Awareness Dataset (SAD) for LLMs. `arXiv:2407.04694v1`.
- `[PP]` Binder, F. J., Chua, J., Korbak, T., Sleight, H., Hughes, J., Long, R., Perez, E., Turpin, M., & Evans, O. (2024). Looking Inward: Language Models Can Learn About Themselves by Introspection. `arXiv:2410.13787v1`.
- `[PP]` Betley, J., Bao, X., Soto, M., Sztyber-Betley, A., Chua, J., & Evans, O. (2025). Tell me about yourself: LLMs are aware of their learned behaviors. `arXiv:2501.11120v1`.
- `[PP]` Lindsey, J. (2026). Emergent Introspective Awareness in Large Language Models. `arXiv:2601.01828v1`.
- `[PP]` Dai, Y., & Wang, Y. (2026). Rescaling confidence: What scale design reveals about LLM metacognition. `arXiv:2603.09309v5`.
- `[PP]` Ren, J., Wang, A., Lai, Y., Wang, X., Gong, L., Li, W., Ma, W., & Liu, Y. (2026). Beyond "I Don't Know": Evaluating LLM Self-Awareness in Discriminating Data and Model Uncertainty (UA-Bench). `arXiv:2604.17293v1`.
- `[PP]` Li, Y., Yue, S., Feng, B., Qi, J., & Ke, B. (2026). From Knowing to Acting: Benchmarking Self-Awareness Capability of LLM Agents (KAware). `arXiv:2606.20661v1`.
- `[PP]` Yang, C. (2026). Self-Aware Recursively Self-Improving Agents for Personal Singularity: A Goal-, Scope-, Tool-, and Benchmark-Driven Multi-Agent Architecture (SARSI). `arXiv:2607.12254v2`.
- `[PP]` Chen, Q., Hayashi, H., Zhang, J., & Wu, C.-S. (2026). Agentic Self-Awareness: Aligning Knowing, Acting, and Holding (ASA). `arXiv:2610.19226v1`.
