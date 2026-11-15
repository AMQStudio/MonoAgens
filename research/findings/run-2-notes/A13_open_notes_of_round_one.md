# A-13 Open notes of round 1 (R-02 to R-28)

> Consolidated from the two Round-1 open-notes streams: Part 1 (`R-02, R-04, R-05, R-07, R-08, R-09, R-10, R-14`) and Part 2 (`R-19, R-20, R-24, R-25, R-26, R-27, R-28`).

## Part 1 — Open notes R-02 to R-14

**Status:** partly answered

**Core-and-Lab issues identified:** Six defects or literature contradictions in `openspec/changes/core-and-lab/` were surfaced while settling these eight notes (detailed in each subsection and summarized below):
1. **R-02 (`specs/appraisal-gate/spec.md`, `design.md`):** A `go` on an unallocated backlog event whose `answered = 1` already proves a third party replied still opens a fresh `patience(P)` wait instead of closing immediately; additionally, `closeness(sender)` discounts `answered` equally with `sel_other`, whereas closeness licenses chiming in on an addressed friend (`sel_other`), not repeating an already answered thread.
2. **R-04 (`design.md` L182–184):** Base-level activation $B_p = \ln(\sum t_k^{-d})$ is undefined ($\ln 0 = -\infty$) when `n = 0` exchanges; substituting $-\infty$ collapses mean into `min` (single-stranger veto), whereas substituting `0` makes $\sigma(0) = 0.5$ (making 100 strangers yield `familiarity = 0.5`).
3. **R-08 (`design.md` L219–223):** `design.md` applies `stakes` only to `expiry(P)` while accepting the weakest evidence (`continued-attention` or addressee's next event) for all acts regardless of stakes—contradicting Clark & Brennan (1991) and Roque & Traum (2008).
4. **R-10 (`design.md` L229–231, `specs/expectations/spec.md` L33–35):** Uniformly retries `failed`, `refused`, `held`, and `unknown` with exponential backoff `t + b·2^(i-1)`, even though `refused` is deterministic/policy-blocked (wasting turns/spamming) and `held` is pending external approval.
5. **R-14 (`design.md` L195, `specs/appraisal-gate/spec.md` L118–120):** Computing `max_i` independently *per signal* across chunk members lets `social_cost = 1` (negative weight) from one `sel_other` message suppress an unanswered direct question (`social_cost = 0`) in the same backlog.
6. **R-07 (`design.md` L273):** `ContextPlan` passes *"commitments and the active goal"* (singular), hiding parent goals during subgoal execution.

---

### Answer

Across the eight open notes from Round 1, two design questions are settled by theory and empirical evidence (**R-14:** event-level `max` strictly dominates `noisy-OR` for correlated within-place backlogs; **R-05 (diagnosis):** uncalibrated LLM 1–10 importance ratings exhibit systematic endpoint compression, range sparsity, and strategic inflation), one structural concern is settled while its weight remains open (**R-02:** `social_cost` and `patience` act at distinct pipeline stages and are not redundant), and the remaining mechanisms (**R-04, R-07, R-08, R-09, R-10**) are partly settled in functional form while leaving body-specific thresholds and prior weights to deterministic Lab tuning. Importantly, every gate and expectation mechanism below remains **content-blind** (operating only on opaque IDs, vector similarities, timestamps, receipts, and body-reported counts), requires **zero LLM calls in the gate**, introduces **no new primitives** beyond the fixed set in `ARCHITECTURE.md`, and separates rules that are **generic across bodies** from those that are **chat/communicative-act specific** (**R-02, R-08**) or **embodied/sensorimotor specific** (**R-09 physical vector rule**).

Formally, the eight resolved rules compose cleanly onto the existing `core-and-lab` equations:

- **Pre-draw social cost & wait rule (R-02, chat / turn-allocated multi-party bodies):**
  $$\text{social\_cost}(e) = \max\Big(\text{sel\_other}(e)\cdot(1 - \lambda_c\,\text{closeness}(\text{sender})),\; \text{answered}(e)\cdot(1 - \lambda_a\,\text{closeness}(\text{sender}))\Big)$$
  where $\text{answered}(e)=1$ suppresses opening a post-`go` `patience(P)` wait unless an active commitment breaker overrides.
- **Bounded context activation (R-04, body-general):**
  $$B_{\text{ctx}}(P,\text{Present}) = \alpha_P\,\tilde{B}_P + (1 - \alpha_P)\,\frac{1}{|\text{Present}|}\sum_{p \in \text{Present}}\tilde{B}_p, \qquad \tilde{B}_x = \begin{cases} \ln\!\sum_k t_k^{-d}, & n_x > 0 \\ B_{\min}, & n_x = 0 \end{cases}$$
  with $\text{familiarity} = \sigma(B_{\text{ctx}})$ if any $n_x > 0$ else $0$, and $\text{closeness}(p) = \sigma(B_p)$ if $n_p > 0$ else $0$.
- **Calibrated reasoning importance (R-05, body-general):**
  $$\text{importance} = \max\!\big(i_{\text{commit}},\; i_{\text{surprise}},\; \min(i_{\text{reason\_cap}},\, Q_m(i_{\text{reason\_raw}}))\big)$$
  where $Q_m$ is a rolling streaming quantile map on anchored 5-level LLM ratings (falling back to identity below $n_{\text{cal}}$ samples).
- **Hierarchical goals via existing `expectations` primitive (R-07, body-general):**
  Add optional acyclic `parent_goal_id: Option<ExpectationId>` to `expectations` of `kind = goal`, with `depth(g) \le D_{\max}`; `ContextPlan` supplies `[g, \text{parent}(g), \dots]` up to `D_max`, attribution matches leaf-first, parent closure cascades cancellation to open children, and leaf `kept` wakes `reason` with the parent context.
- **Stakes-tiered grounding evidence (R-08, communicative acts; embodied acts use R-09 `effect`):**
  Define stakes $\rho_{\text{stakes}} \in [0,1]$ (`1.0` on commitments, else $\text{cost}/(\text{value}+\text{cost})$): accept Tier 1 (`done` OR `continued_attention` OR next event) when $\rho_{\text{stakes}} < \theta_{\text{med}}$; Tier 2 (`done` OR relevant non-negative reply via `classify`/vector match) when $\theta_{\text{med}} \le \rho_{\text{stakes}} < \theta_{\text{high}}$; Tier 3 (`done` on physical act OR verified content match $\cos \ge \text{wake\_gate\_hi}$ without repair) when $\rho_{\text{stakes}} \ge \theta_{\text{high}}$.
- **Adaptive comparator tolerance window (R-09; echo rule body-general, physical vector rule embodied-only):**
  Accept `self` if (`correlation_id` matches OR `actor == self`) OR (channel matches AND $t_{\text{act}} + \widehat{\text{lat}}_{\min} \le t_{\text{obs}} \le t_{\text{act}} + \widehat{\text{lat}}_{\max}$ AND $\|\Delta_{\text{obs}} - \Delta_{\text{pred}}\| \le k\,\hat{\sigma}_{\text{channel}}$), shrinking body priors toward confirmed `correlation_id` matches with $n_0$ pseudo-observations.
- **Receipt-typed retry vs inform/renegotiate (R-10, body-general):**
  `refused` $\to$ `0` retries, immediate wake to inform/renegotiate; `held` $\to$ wait without duplicate retry until external update or $\text{now} \ge \text{due} - \text{margin\_inform}$; `failed`/`unknown` $\to$ silent retry at $t + b\cdot 2^{i-1}$ iff $i \le n_{\max}(\rho_{\text{stakes}})$ AND $t + b\cdot 2^{i-1} + \widehat{\text{lat}}_{\max} \le \text{due} - \text{margin\_inform}$ AND ($\rho_{\text{stakes}} < \theta_{\text{high}}$ OR $i = 1$), else immediate wake to inform/renegotiate.
- **Event-level decayed `max` for backlog chunks (R-14, body-general):**
  Rank and appraise a place's backlog chunk by its peak decayed member event $i^* = \arg\max_i [u_{\text{gate}}(e_i)\cdot (1+L_i)^{-d}]$ (or pair the positive-signal maxima with $e_{i^*}$'s `social_cost`), preserving any commitment breaker (`commit = 1`) and capping handled chunks at $C_{\text{wake}} = 4$ per wake.

---

#### R-02 Does closeness/familiarity lower the social cost of taking an unallocated turn, and is `social_cost` redundant with `patience`?

1. **Verdict:** **Partly settled.** That `social_cost` and `patience` are **not redundant** is **settled** on architectural and timing grounds. That closeness lowers the social cost of overlapping / taking an unallocated turn is **partly settled qualitatively** (strongest between close friends vs strangers) and **not settled quantitatively** (no empirical study validates a linear slope $1 - \lambda_c\cdot\text{closeness}$; socio-pragmatic "bulge" effects create non-monotonicity across acquaintances vs intimates).
2. **Evidence & Rule:**
   - *Turn allocation and selection rights:* Sacks, Schegloff & Jefferson (1974, `10.1353/lan.1974.0010` `[PR]`) and Lerner (2003, `10.1017/S004740450332202X` `[PR]`) establish that when current speaker selects a next speaker (`sel_other = 1`), only the selected party has primary rights to the transition-relevance place; self-selection by a third party is locally dispreferred unless the selected party fails to take the floor. Unaddressed turns (`sel_other = 0`) remain open to anyone. Across languages, average inter-turn gaps cluster tightly (~0–500 ms across 10 languages; Stivers et al. 2009, `10.1073/pnas.0903616106` `[PR]`).
   - *Distance/closeness and interruption/overlap norms:* Brown & Levinson (1987, `10.1017/CBO9780511813085` `[PR]`) model the weightiness of a face-threatening act (which includes uninvited interruption or overriding speaker selection) additively as $W_x = D(S, H) + P(H, S) + R_x$, where smaller social distance $D(S, H)$ (higher closeness) monotonically decreases conversational cost $W_x$. Conversation-analytic and experimental studies confirm this direction for close friends vs strangers: Tannen (2005 [1984], `10.1093/oso/9780195221817.001.0001` `[PR]`) documents "high-involvement" styles where cooperative overlapping signals rapport rather than aggression; Coates (1996, ISBN `978-0631182528` `[PR]`) shows women friends routinely co-construct turns via simultaneous speech without face threat; Bilous & Krauss (1988, `10.1016/0271-5309(88)90016-X` `[PR]`) and Kennedy & Camden (1983, `10.1080/10570318309374104` `[PR]`) distinguish affiliative/supportive interruptions from power-assertive interruptions. Recent large-scale dyadic experiments by Templeton et al. (2022, `10.1073/pnas.2116915119` `[PR]`; 2023, `10.1098/rstb.2021.0471` `[PR]`) further show that friends tolerate longer inter-turn gaps (> 2 s) without feeling awkward or disconnected whereas strangers treat > 2 s gaps as disconnection, confirming that relational closeness systematically relaxes turn-timing penalties.
   - *Limitation / non-monotonicity caveat:* Wolfson's (1988, ISBN `978-0893914134` `[PR]`) *Bulge* theory demonstrates that speech-act negotiation and politeness investment are not always linear in social distance: behavioral constraints can peak among status-equal acquaintances (where relationships are negotiable) while both intimates and total strangers behave more directly. Thus the linear coefficient $(1 - \lambda_c\,\text{closeness}(\text{sender}))$ cannot be fixed from literature alone.
   - *Why `social_cost` is NOT redundant with `patience`:* `social_cost` acts **pre-draw** inside the gate formula at event arrival $t(e)$, penalizing events that are addressed to someone else (`sel_other = 1`) or already replied to (`answered = 1`). `patience` acts **post-`go`** (only if the gate already decided `go` on an unallocated turn) and imposes a reactive wait of `patience(P)` seconds (`q_p` quantile of first third-party reply). If `social_cost` were removed, every `sel_other = 1` event with positive urgency/relevance would fire `go`, open a timer, and still speak falsely whenever the rightful addressee replies slower than the `q_p` quantile (which occurs in $1 - q_p \approx 50\%$ of cases when $q_p = 0.5$!). Conversely, `patience` remains necessary when `sel_other = 0` (open-floor messages where `social_cost = 0` before any reply arrives) so another human can claim an unaddressed turn.
   - *Issues in `core-and-lab` and corrected rule:*
     1. In `openspec/changes/core-and-lab/design.md` (L171–174), `answered(e)` checks if an actor other than sender and soul acted after `e` in `P`. In live delivery (`now = t(e)`), `(t(e), now]` is always empty (`answered = 0`), so `answered` only operates during backlog/delayed appraisal. However, `openspec/changes/core-and-lab/specs/appraisal-gate/spec.md` (L100–103) states that *any* `go` on an unallocated turn *"SHALL open a wait"* for `patience(P)` seconds—meaning even if a backlog event already has `answered = 1`, passing `go` forces an irrational additional wait! **Fix:** When `answered(e) = 1` prior to wait creation (and `commit = 0`), treat the turn as already yielded (`silent`) rather than opening a redundant wait.
     2. `design.md` L172 multiplies `max(sel_other, answered)` by a single `(1 - λ_c · closeness(sender))`. Literature supports closeness lowering intrusion cost for `sel_other` (chiming in on a friend), **not** repeating an already answered question (`answered = 1`). Split the discount into:
        $$\text{social\_cost}(e) = \max\Big(\text{sel\_other}(e)\cdot(1 - \lambda_c\,\text{closeness}(\text{sender})),\; \text{answered}(e)\cdot(1 - \lambda_a\,\text{closeness}(\text{sender}))\Big)$$
        with start values $\lambda_c = 0.35$ (range $[0.0, 0.7]$) and $\lambda_a = 0.0$ (range $[0.0, 0.3]$).
   - *Body generality:* **Chat-only / turn-allocated multi-party places.** Physical embodied rooms with simultaneous non-verbal actions require an explicit physical-addressee convention (deferred to R-29) before `sel_other` applies cleanly.
3. **Lab Experiment:**
   - *Scenario:* Two-branch development ablation across multi-party chat scenarios crossing `sender_closeness` $\in \{0.0, 0.5, 0.9\}$ $\times$ `addressee_state` $\in \{\text{sel\_other, open\_floor, backlog\_answered}\}$ $\times$ addressee reply delay (above vs below `q_p`).
   - *Ablations:* (a) `λ_c ∈ {0.0, 0.35, 0.5, 0.7}` with `λ_a ∈ {0.0, λ_c}`; (b) drop `social_cost` (`w_sc = 0`, `patience`-only); (c) fix `answered = 1` post-`go` short-circuit vs `core-and-lab` spec.
   - *Metrics:* Unsolicited interruption rate (`go` when `sel_other = 1` and addressee eventually replies), missed helpful-interjection rate on stalled close-friend threads, redundant reply rate on `answered = 1` backlogs, and unnecessary wait-timer count (`patience` timers opened then aborted by reply).

---

#### R-04 Should ACT-R context activation `B_ctx` use averaging (`1 / (1 + |Present|)`) or `min`?

1. **Verdict:** **Partly settled.** ACT-R mathematical theory **settles in favor of $1/n$ additive weighting (mean) over `min`** (hard bottleneck); however, exact weighting between place activation $B_P$ and present people $\text{Present}$ in large groups remains **not settled by the literature**, and `core-and-lab`'s zero-exchange edge case contains a concrete mathematical bug.
2. **Evidence & Rule:**
   - *ACT-R spreading activation theory:* In ACT-R (Anderson 1974, `10.1016/0010-0285(74)90021-8` `[PR]`; Anderson et al. 2004, `10.1037/0033-295X.111.4.1036` `[PR]`; Anderson 2007, `10.1093/acprof:oso/9780195324259.001.0001` `[PR]`), retrieval activation of chunk $i$ sums base-level activation and weighted spreading activation from all $n$ elements currently in the attentional/imaginal focus:
     $$A_i = B_i + \sum_{j \in \text{focus}} W_j\,S_{ji} + \epsilon, \qquad W_j = \frac{W}{n}, \qquad S_{ji} = S_{\max} - \ln(\text{fan}_j)$$
     Three structural facts follow directly from ACT-R: (a) source activation $W$ is a **fixed total budget divided equally** ($1/n$) across focus items (`1 + |Present|` sources: the place plus each present participant), which corresponds algebraically to **averaging**, *not* `min`; (b) $W_j$ is additive rather than a conjunction veto—one unfamiliar item reduces its $1/n$ share, rather than zeroing out activation from a deeply familiar place or close co-present partner (which `min` does); and (c) the empirical **fan effect** arises because adding more elements splits attention $W/n$ and increases cue fan.
   - *Mathematical defect in `core-and-lab` (`design.md` L182–184):* `design.md` defines `closeness(p) = σ(B_p), 0 with no exchange`, and immediately writes:
     $$B_{\text{ctx}} = \frac{B_P + \sum_{p \in \text{Present}} B_p}{1 + |\text{Present}|}, \qquad \text{familiarity} = \sigma(B_{\text{ctx}}),\; 0\text{ with no history}.$$
     Notice that when an entity has `n = 0` exchanges, the ACT-R sum $B_p = \ln(\sum_{k=1}^0 t_k^{-d}) = \ln(0) = -\infty$ is **undefined**:
     - If $B_p = -\infty$ enters the sum, any single stranger in a known group forces $B_{\text{ctx}} = -\infty \implies \text{familiarity} = 0$, silently turning the intended `mean` into a strict `min` veto!
     - If $B_p = 0$ is substituted as a naive zero fallback, then $\sigma(0) = 0.5$: a room full of strangers gives `familiarity ≈ 0.5`!
     - Furthermore, in large channels ($|\text{Present}| = 50$ lurkers), unweighted $1/(1+|\text{Present}|)$ lets passive strangers crush $B_P$ and the closeness of the active speaker, whereas real human attention focuses on *active* recent speakers (`fan` in imaginal buffer, Anderson et al. 2004).
   - *Corrected rule (Body-general):* Define a bounded activation floor $\tilde{B}_x$ when an entity has zero exchanges (`n_x = 0`), and separate place weight $\alpha_P$ from present-person aggregation (with optional restriction to recently active speakers when $|\text{Present}|$ is large, if available without content inspection):
     $$\tilde{B}_x = \begin{cases} \ln\!\left(\sum_{k=1}^{n_x} t_k^{-d}\right), & n_x > 0 \\ B_{\min}, & n_x = 0 \end{cases}$$
     $$B_{\text{ctx}}(P,\text{Present}) = \begin{cases} \tilde{B}_P, & |\text{Present}| = 0 \\ \alpha_P\,\tilde{B}_P + (1 - \alpha_P)\,\frac{1}{|\text{Present}|}\sum_{p \in \text{Present}}\tilde{B}_p, & |\text{Present}| > 0 \end{cases}$$
     $$\text{familiarity} = \begin{cases} 0, & n_P = 0 \text{ and } \forall p \in \text{Present},\, n_p = 0 \\ \sigma(B_{\text{ctx}}), & \text{otherwise} \end{cases}$$
     Start values: $B_{\min} = -4.0$ (range $[-6.0, -2.0]$, so $\sigma(-4.0) \approx 0.018$), $\alpha_P = 0.5$ when $|\text{Present}| > 0$ (range $[1/(1+|\text{Present}|),\, 0.7]$; setting $\alpha_P = 1/(1+|\text{Present}|)$ recovers `design.md`'s equal-source division while eliminating the $-\infty$ / $0.5$ zero-exchange bug).
3. **Lab Experiment:**
   - *Scenario:* Multi-room chat/physical scenarios holding `closeness(speaker)` fixed while varying room history (`known_private_dm` vs `familiar_group_with_1_stranger` vs `50_member_channel_with_2_friends` vs `all_strangers`).
   - *Ablations:* (a) `mean` with $\alpha_P = 1/(1+|\text{Present}|)$ vs fixed $\alpha_P = 0.5$; (b) `min` across `{P} ∪ Present`; (c) `B_min ∈ {-2.0, -4.0, -6.0}`.
   - *Metric:* Appropriateness of gate caution/speaking threshold in mixed company (false restraint when 1 stranger joins a familiar room vs false over-familiarity in public stranger-dominated rooms) and memory recall `precision@k` when context-familiarity boosts place/person match.

---

#### R-05 How well calibrated are LLM-assigned importance scores (`i_reason`, Park et al. 2023)?

1. **Verdict:** **Settled** that raw LLM-assigned ordinal / 1–10 importance scores (`i_reason`) are **systematically miscalibrated** (compressed toward the middle of the scale, sensitive to prompt/scale design, distributionally shifted across tasks/models, and vulnerable to inflation under competitive allocation). **Partly settled** on the exact post-hoc calibration transform.
2. **Evidence & Rule:**
   - *Park et al. (2023) baseline:* Park et al. (2023, `10.1145/3586183.3606763`, `arXiv:2304.03442` `[PR]`) introduced the 1–10 prompt-based *"poignancy"* / importance score (`1` = mundane like brushing teeth, `10` = breakup/college acceptance) summed with recency and relevance in memory retrieval. However, Park et al. **never measured calibration** of the 1–10 ratings against human judgments, and their Section 6.2 controlled ablation removed memory *types* (observation, reflection, planning) rather than ablating the importance weighting term.
   - *2024–2026 empirical critiques & calibration literature:*
     1. **Systematic central tendency bias / endpoint compression:** Zhang et al. (2026, `arXiv:2605.16386` `[PP]`) audited three frontier LLM families against supervised models on the 0–5 ordinal Shulman Clock Drawing Test rubric (two datasets). Although frontier models achieved competitive within-1 agreement (e.g., GPT-5 MAE `0.67`, within-1 `92%` vs fine-tuned ViT MAE `0.52`, within-1 `91%`), all three LLM families showed statistically significant endpoint compression: flatter calibration slopes ($\Delta\hat{\beta} = -0.049$, 95% CI $[-0.096, -0.002]$, $p = 0.020$) and elevated toward-center errors ($34.0\%$ vs $25.6\%$, $z = 3.16$, $p < 0.001$), over-predicting `0` as `1` and under-predicting `5` as `4`. Crucially, **neither few-shot exemplars spanning the entire 0–5 range nor prompt de-terminologizing eliminated central tendency bias**.
     2. **Scale-range, round-number, and prompt-sensitivity bias:** Stureborg, Alikaniotis & Suhara (2024, `arXiv:2405.01724` `[PP]`) analyzed LLM evaluators on `1–5`, `1–10`, and `1–100` scales, demonstrating strong anchoring, familiarity bias, arbitrary preference for round numbers (reducing effective resolution on `1–10` and `1–100`), and high score variance across prompt perturbations. Wang et al. (2024, `10.18653/v1/2024.acl-long.511` `[PR]`) likewise proved systematic judge miscalibration and positional bias that cannot be removed solely by prompting.
     3. **Distributional shift vs ordinal reality:** Subasic, Rabinovich & Chen (2026, `arXiv:2611.00447` `[PP]`) formalize LLM-as-a-judge ratings as distributionally shifted ordinal draws rather than calibrated cardinal utilities, showing that streaming quantile matching corrects marginal shift without supervised fine-tuning.
     4. **Strategic inflation under allocation competition:** Zhou et al. (2026, `arXiv:2610.33728` `[PP]`) demonstrate that when reported relevance/importance determines scarce downstream top-$K$ memory or attention allocation (which directly mirrors `importance` in MonoAgens!), LLMs exhibit strategic over-reporting/inflation unless scoring incentives or caps decouple allocation power from raw magnitude.
   - *Rule (Body-general):* Keep `design.md` (L202–205)'s deterministic structural anchors (`i_commit = 1` on expectation links and `i_surprise = clamp(z / z_max, 0, 1)`), and harden `i_reason` in two places:
     1. In `reason`, replace the unanchored `1–10` integer prompt with a **5-tier behaviorally anchored discrete rubric** $\{0.00, 0.25, 0.50, 0.75, 1.00\}$ (avoiding 1–10 round-number sparsity).
     2. In core (deterministic math, zero LLM calls), map raw `i_reason_raw` through a capped rolling quantile transform per active model $m$:
        $$i_{\text{reason\_cal}} = \min\!\big(i_{\text{reason\_cap}},\; Q_m(i_{\text{reason\_raw}})\big), \qquad \text{importance} = \max(i_{\text{commit}},\, i_{\text{surprise}},\, i_{\text{reason\_cal}})$$
        where $Q_m(x) = \hat{F}_m(x)$ is the empirical CDF rank over a ring buffer of the last $W_{\text{cal}}$ ratings from model $m$ (falling back to identity $x$ while sample count $< n_{\text{cal}}$). Start values: $i_{\text{reason\_cap}} = 0.80$ (range $[0.50, 1.00]$), $n_{\text{cal}} = 50$ (range $[20, 200]$), $W_{\text{cal}} = 500$. Capping below `1.0` ensures a miscalibrated or inflated LLM score can never tie with real commitments (`i_commit = 1`).
3. **Lab Experiment:**
   - *Scenario:* Multi-week longitudinal memory suite containing 20% routine chatter, 60% medium task updates, 10% novel non-commitment anomalies (`i_surprise` high), and 10% promises/goals (`i_commit = 1`), evaluated under real candidate `reason` models (and deliberate central-tendency / inflated synthetic `i_reason` perturbations).
   - *Ablations:* (a) raw `1–10` unanchored score `/ 10`; (b) 5-tier anchored rubric raw; (c) rubric + rolling $Q_m$ + `i_reason_cap ∈ {0.6, 0.8, 1.0}`; (d) `i_reason = 0` (`commit` + `surprise` only).
   - *Metrics:* Memory retention `recall@k` for true commitments and rare important facts after decay, ECE/slope calibration against oracle gold importance labels, and false-eviction rate of `i_commit` episodes by routine `0.8` chatter.

---

#### R-07 How should nested or hierarchical goals be handled, and can `expectations` carry `parent_goal_id` without a new primitive?

1. **Verdict:** **Partly settled.** That hierarchical goals can be modeled inside `expectations` via `parent_goal_id` **without adding any new architectural primitive** is **settled** (adding an optional typed parent pointer on existing `kind = goal` rows preserves the fixed primitive count). Whether human-like non-LIFO activation resumption outperforms strict top-down/leaf-only goal execution in long-horizon agent benchmarks requires Lab testing.
2. **Evidence & Rule:**
   - *Soar vs ACT-R goal management:* Classical **Soar** (Laird, Newell & Rosenbloom 1987, `10.1016/0004-3702(87)90050-6` `[PR]`; Laird 2012, `10.7551/mitpress/7688.001.0001` `[PR]`) creates hierarchical subgoals automatically whenever an *impasse* prevents operator selection in the current state, collapsing the substate once the result resolves the impasse. **ACT-R**, by contrast, historically tried a strict LIFO goal stack and **explicitly abandoned it** in ACT-R 5.0/6.0: Altmann & Trafton (2002, `10.1016/S0364-0213(01)00058-1` `[PR]`) and Anderson et al. (2004, `10.1037/0033-295X.111.4.1036` `[PR]`) showed that human goal management is not a rigid push-down stack—suspended goals decay, are retrieved opportunistically via environmental cues (non-LIFO resumption), and suffer predictable **post-completion errors** if completed subgoals are not actively inhibited/closed.
   - *Executive control & prospective memory sub-intentions:* Miller & Cohen (2001, `10.1146/annurev.neuro.24.1.167` `[PR]`) show prefrontal goal representations bias downstream pathway competition from superordinate goals to concrete actions. Marsh & Hicks (1998, `10.1037/0278-7393.24.2.336` `[PR]`) establish that prospective memory tracking competes with executive working-memory load, making flat goal lists brittle under concurrent tasks. Recent 2026 agentic prospective-memory systems confirm this: Zhao & Wu (2026, `arXiv:2609.01272` `[PP]`) show typed intention stores with deterministic schema-constrained state transitions outperform raw LLM prompting (after noting frontier scaffolds reach only `65.1%` Set-F1 on PM-Bench, Liu & Gabriel 2026 `arXiv:2607.12385` `[PP]`), and Bei et al. (2026, `arXiv:2611.05731` `[PP]`) distinguish intention retention from timely triggering/deferral.
   - *Rule & `core-and-lab` gap (Body-general):*
     1. **No new primitive:** Add optional `parent_goal_id: Option<ExpectationId>` on `expectations` rows with `kind = goal` (passed optionally via `expect(kind, about_ids, condition, due, expiry)` / typed field, rejecting cycles and depth $> D_{\max}$; start $D_{\max} = 3$, range $2\text{--}5$). No new book, thought act, door, signal, or drive is added.
     2. **Non-LIFO DAG/Tree semantics (ACT-R over strict stack):** Multiple sibling subgoals can exist concurrently and reactivate when their cues fire. **Attribution priority** matches leaf goals before ancestors (`depth` descending), so progress on a step does not prematurely resolve its parent.
     3. **Cascade & wake rules:** Closing a parent goal (`kept`, `cancelled`, `expired`) deterministically cascades `cancelled` to any open descendants (preventing Altmann & Trafton post-completion orphan actions). Completing a leaf subgoal (`kept`) injects a waking `expectation` event referencing `parent_goal_id` so `reason` inspects the parent and opens the next subgoal or closes the parent.
     4. **Contradiction / omission in `design.md` L273:** `design.md` L273 defines `ContextPlan` as loading *"commitments and the active goal"* (singular). With nested goals, loading only the active leaf hides *why* the subgoal exists and breaks superordinate alignment. **Fix:** Rebuild the active goal chain `[g_leaf, parent(g_leaf), ..., g_root]` (bounded by $D_{\max}$) into `ContextPlan`.
3. **Lab Experiment:**
   - *Scenario:* Multi-step interrupted goal workflows (1 parent goal with 2–3 sequential/parallel subgoals plus an unrelated high-urgency chat interruption mid-subgoal and parent cancellation mid-flight).
   - *Ablations:* (a) Flat goals (`parent_goal_id = None`); (b) Strict LIFO stack (only top of stack matchable); (c) Non-LIFO tree (`parent_goal_id` + leaf-first attribution + cascade cancel + ancestor `ContextPlan`, varying $D_{\max} \in \{2, 3, 5\}$).
   - *Metrics:* Goal completion rate, orphan subgoal/post-completion action rate after parent cancellation, non-LIFO resumption latency after interruption, and spurious parent-goal resolution rate.

---

#### R-08 How does communicative purpose / task stakes change the evidence of understanding required?

1. **Verdict:** **Partly settled.** Clark & Brennan (1991), Clark & Schaefer (1989), and Roque & Traum (2008) **settle** the principle that higher task stakes / consequential communicative acts demand **stronger, higher-tier grounding evidence** (demonstration or explicit content confirmation rather than mere continued attention). However, exact numeric boundaries on $\rho_{\text{stakes}}$ and cosine thresholds require Lab calibration per task distribution.
2. **Evidence & Rule:**
   - *Foundational psycholinguistics & grounding criterion:* Clark & Schaefer (1989, `10.1207/s15516709cog1302_7` `[PR]`) organize **evidence of understanding** into five ordered levels of strength (weakest to strongest): (1) *Continued attention* (no trouble indicated; next move proceeds without repair), (2) *Initiation of the relevant next contribution*, (3) *Acknowledgment* (backchannel/head-nod/`ok`), (4) *Demonstration* (executing or paraphrasing what was understood), and (5) *Display* (verbatim repetition). Clark & Brennan (1991, `10.1037/10096-006` `[PR]`, pp. 127–149) formulate the **Grounding Criterion**: *"The contributor and the partners mutually believe that the partners have understood what the contributor meant to a criterion sufficient for current purposes"*—explicitly stating that the required tier changes with stakes and medium costs.
   - *Computational operationalization:* Roque & Traum (2008, `https://aclanthology.org/W08-0107/` `[PR]`, pp. 54–63) operationalized a multi-degree grounding scale for dialogue systems (from `Agreed-Signal` via backchannel acknowledgment up to `Agreed-Content+` requiring explicit repeat-back/verification without rejection) explicitly indexed by **task stakes and risk of misunderstanding**. Similarly, Paek & Horvitz (2000, `arXiv:1301.3883` `[PR]`) in the Quartet architecture frame grounding as Value-of-Information (VoI) under uncertainty: confirmation acts are suppressed when stakes/misunderstanding cost is small relative to dialogue turn cost, and required when misunderstanding cost dominates.
   - *Contradiction in `core-and-lab` (`design.md` L218–223):* `design.md` L219–223 scales `expiry(P)` by a stakes factor, **yet fixes uniform minimal evidence** (*"`done` receipt, the addressee's next event in the place, or a continued-attention number the body reports"*) for all acts regardless of stakes! Accepting `continued-attention` or an arbitrary next event on a high-stakes request/instruction violates the Grounding Criterion of Clark & Brennan (1991) and Roque & Traum (2008).
   - *Rule (Chat / communicative-act bodies; embodied physical acts use R-09 `effect` predictions):*
     Let communicative stakes be $\rho_{\text{stakes}} \in [0, 1]$, defined deterministically without LLM calls at expectation open time:
     $$\rho_{\text{stakes}} = \begin{cases} 1.0, & \text{if linked to promise or goal} \\ \dfrac{\text{cost}}{\text{value} + \text{cost}} \in [0, 1], & \text{otherwise (from act/capability descriptor; default } 0.5\text{ if uncalibrated)} \end{cases}$$
     Map $\rho_{\text{stakes}}$ to Clark & Schaefer's tiers via thresholds $\theta_{\text{med}}$ (start `0.40`, range `[0.20, 0.60]`) and $\theta_{\text{high}}$ (start `0.75`, range `[0.60, 0.90]`):
     - **Tier 1 ($\rho_{\text{stakes}} < \theta_{\text{med}}$, low-stakes/phatic):** Accept `done` receipt **OR** body `continued_attention > 0` **OR** addressee's next event in `P`.
     - **Tier 2 ($\theta_{\text{med}} \le \rho_{\text{stakes}} < \theta_{\text{high}}$, medium-stakes query/coordination):** Reject `continued_attention`-only; require addressee's next event with non-negative `classify` match (`no repair/confusion`) **OR** `cos >= wake_gate_lo` (`reply` prediction met).
     - **Tier 3 ($\rho_{\text{stakes}} \ge \theta_{\text{high}}$, high-stakes commitment/instruction/parameter handoff):** Require either a verified execution receipt (`done` from target capability) **OR** addressee demonstration/explicit confirmation (`cos >= wake_gate_hi` or explicit affirmative `classify` without negative/repair cue); if only Tier 1/2 evidence arrives before `expiry(P)`, keep the expectation open / raise mismatch error so `reason` can verify cleanly.
3. **Lab Experiment:**
   - *Scenario:* Controlled dialogue scenarios spanning low-stakes greetings/status notes (`stakes < 0.4`), medium-stakes scheduling (`0.4–0.75`), and high-stakes parameter/action confirmations (`≥ 0.75`) where the conversational partner sometimes replies `"uh huh"` (`continued_attention`) or gives an off-topic/ambiguous next message before silently mis-executing.
   - *Ablations:* (a) `core-and-lab` uniform L1/L2 baseline (`θ_med = θ_high = 1.0`); (b) stakes-tiered rule with $(\theta_{\text{med}}, \theta_{\text{high}}) \in \{(0.3, 0.7), (0.4, 0.75), (0.5, 0.85)\}$.
   - *Metrics:* Premature-grounded silent error rate on high-stakes tasks vs unnecessary clarification-check rate on low-stakes phatic turns (precision-recall trade-off across stakes strata).

---

#### R-09 What quantitative tolerance and temporal windows does the comparator model / robotics sense-of-agency literature give?

1. **Verdict:** **Partly settled.** In human neurophysiology, the comparator / sense-of-agency binding and attenuation windows are well bounded around **$\sim 150\text{--}650\text{ ms}$** (strongest around **$200\text{--}300\text{ ms}$**). However, across MonoAgens's heterogeneous bodies (instant local physics vs 300–5000 ms chat webhooks vs external tool APIs), **universal fixed constants are impossible**—literature supports body-declared priors shrunk toward live channel observations (`core-and-lab`'s structural form), augmented with online `correlation_id` calibration.
2. **Evidence & Rule:**
   - *Human intentional binding & sensory attenuation:* Haggard, Clark & Kalogeras (2002, `10.1038/nn827` `[PR]`) discovered intentional binding (perceptual compression between voluntary action and sensory outcome) when testing `250 ms`, `450 ms`, and `650 ms` delays; as verified in Arikan et al. (2017, `10.1177/2041669517719297` `[PR]`), binding is **strongest at `250 ms`** and progressively weakens at `450 ms` and `650 ms` (reviewed in Moore & Obhi 2012, `10.1016/j.concog.2011.12.002` `[PR]`). Blakemore, Frith & Wolpert (1999, `10.1162/089892999563607` `[PR]`; Blakemore, Wolpert & Frith 2000, `10.1097/00001756-200008030-00002` `[PR]`) established that sensory attenuation of self-produced tactile stimuli breaks down significantly as artificial delay or trajectory perturbation increases beyond zero-lag motor prediction; Shimada (2022, `10.3390/s22135059` `[PR]`) synthesizes multisensory/visuotactile self-body experiments showing an **$\approx 200\text{--}300\text{ ms}$ window** (with active delay-detection threshold $\approx 230\text{ ms}$ and rapid nonlinear drop in self-attribution beyond $\sim 200\text{ ms}$). Franck et al. (2001, `10.1176/appi.ajp.158.3.454` `[PR]`) confirm that widened angular or temporal comparator tolerances produce clinical self-other misattribution (delusions of influence).
   - *Robotics self-other discrimination & normalized error bounds:* Gold & Scassellati (2009, `10.1016/j.robot.2008.07.006` `[PR]`) modeled robotic mirror self-recognition via Bayesian accumulation of temporal contingency between motor command and observed motion. Lanillos & Cheng (2018, `10.1109/iros.2018.8593684`, `arXiv:1805.03104` `[PR]`) and Lanillos, Pages & Cheng (2020, `arXiv:2004.05473` `[PP]`) formalized active-inference comparator self/other distinction using variance-normalized Gaussian prediction errors (Mahalanobis quadratic terms scaled by inverse sensory covariance $\Sigma^{-1}$) plus temporal action-feedback contingency $p(\text{cont})$. Chen et al. (2026, `arXiv:2606.13222` `[PP]`) similarly establish zero-label humanoid self-other distinction from proprioceptive-visual temporal/kinematic correspondence.
   - *Assessment of `core-and-lab` (`design.md` L224–227) & Rule:*
     `core-and-lab` correctly avoids hardcoding `200–650 ms` across bodies, specifying instead:
     1. **Echo / identity match (Body-general across chat, API, and embodied bodies):** Match with `basis = self` if `obs.correlation_id == act.correlation_id` OR `obs.actor == self`.
     2. **Comparator spatiotemporal channel match (primarily Embodied / sensorimotor bodies; optional on chat only if a channel produces numeric effect deltas):** Match if channel equals declared channel AND
        $$t_{\text{act}} + \widehat{\text{lat}}_{\min}(c) \le t_{\text{obs}} \le t_{\text{act}} + \widehat{\text{lat}}_{\max}(c) \quad\text{AND}\quad \|\Delta_{\text{obs}} - \Delta_{\text{pred}}\|_{\hat{\Sigma}_c^{-1}} \le k \quad (\text{1D scalar: } |\Delta_{\text{obs}} - \Delta_{\text{pred}}| \le k\,\hat{\sigma}_c)$$
     - *Gap identified in `core-and-lab`:* `design.md` treats body-declared `[lat_min, lat_max]` and `σ_channel` as static forever. When bodies mis-estimate latency jitter (e.g., network load or worn actuators), static bounds cause agency failures. **Addition:** Shrink declared channel priors $(\text{lat}_{\min,0}, \text{lat}_{\max,0}, \sigma_{c,0})$ toward confirmed `correlation_id` matches using the same empirical Bayes shrinkage parameter $n_0$ (`10` pseudo-observations, start $k = 2.5$, range $[1.5, 4.0]$; human sensorimotor defaults when a body simulates biological effectors: $[0.05, 0.30]\,\text{s}$ tight / $[0.15, 0.65]\,\text{s}$ wide).
3. **Lab Experiment:**
   - *Scenario:* Paired *chat-echo* scenario (bot + other users in Slack/IRC with injected webhook latencies $0.2\text{--}4\text{ s}$) and *embodied-agent* scenario (two agents acting in the same physical workspace with sensor noise $\sigma$ and overlapping action windows).
   - *Ablations:* $k \in \{1.5, 2.0, 2.5, 3.0, 4.0\}$ and static body priors vs shrunk online $(\widehat{\text{lat}}_{\min}, \widehat{\text{lat}}_{\max}, \hat{\sigma}_c)$ (`n_0 ∈ {5, 10, 20, ∞}`).
   - *Metrics:* Self-attribution false-positive rate (misattributing another agent's simultaneous act as `self`) vs false-negative rate (firing spurious `self` surprise / appraising one's own delayed echo).

---

#### R-10 When does renegotiating or informing beat silently retrying a failed commitment?

1. **Verdict:** **Partly settled.** Qualitative conditions favoring proactive informing/renegotiation over silent retrying are **settled** across organizational trust repair, service recovery, and HRI literature (broken integrity/concealment, deadline proximity/visible delay, deterministic refusal, and repeated strikes demand immediate transparency; brief invisible transient faults before coordination impact tolerate silent recovery). Exact retry budget `n_max` and lead-time `margin_inform` remain **open** parameters for Lab tuning.
2. **Evidence & Rule:**
   - *Trust violation, deception, and timing of repair:* Schweitzer, Hershey & Bradlow (2006, `10.1016/j.obhdp.2006.05.005` `[PR]`) show that while trust broken by untrustworthy performance *can* recover substantially through a series of trustworthy follow-up actions and promises, trust harmed by **deception or concealment** fails to recover—even after apology and subsequent trustworthy behavior. Kim et al. (2004, `10.1037/0021-9010.89.1.104` `[PR]`) demonstrate that apologies repair *competence* failures effectively, whereas concealment or denial hurts severely once guilt/failure is exposed.
   - *Service recovery & HRI speed/strike limits:* Classic service-recovery research establishes that speed of recovery and prompt proactive explanation dramatically improve post-failure evaluation while unacknowledged delays amplify dissatisfaction (Bitner, Booms & Tetreault 1990, `10.1177/002224299005400105` `[PR]`; Smith, Bolton & Wagner 1999, `10.1177/002224379903600305` `[PR]`; Wirtz & Mattila 2004, `10.1108/09564230410532484` `[PR]`). In human-machine and human-robot trust repair (de Visser, Pak & Shaw 2018, `10.1080/00140139.2018.1457725` `[PR]`; Baker et al. 2018, `10.1145/3181671` `[PR]`; Tolmeijer et al. 2020, `10.1145/3319502.3374793` `[PR]`), Robinette, Howard & Wagner (2015, `10.1007/978-3-319-25554-5_57` `[PR]`) and Nayyar & Wagner (2018, `10.1007/978-3-030-05204-1_26` `[PR]`) prove that **timing of repair is critical**: intervening/informing near the error or immediately at the interaction recovery point restores trust far better than late recovery after downstream harm occurs. Esterwood & Robert (2023a, `10.1038/s41598-023-37032-0` `[PR]`; 2023b, `10.1016/j.chb.2023.107658` `[PR]`) further establish a **"three strikes"** decay: trust repair succeeds on initial/isolated failures (`1–2` failures) but collapses after repeated failures ($\ge 3$ strikes).
   - *Defect / Contradiction in `core-and-lab` (`design.md` L229–231 & `specs/expectations/spec.md` L33–35):*
     `core-and-lab` currently groups **all four non-success receipts identically**: *"A receipt of failed, refused, held or unknown SHALL NOT count as keeping it: the promise SHALL be retried with a growing delay up to a limit"* (`t + b·2^(i-1)` for `i ≤ n_max`). This violates both protocol semantics (`specs/protocol/spec.md`) and trust-repair evidence:
     1. **`refused`:** Deterministic rejection/policy block—silent exponential retries of the same blocked action cannot succeed (`0` retries), delay notification past optimal repair timing (Robinette et al. 2015), and risk spamming.
     2. **`held`:** External approval/gate hold—re-issuing identical retries while `held` creates duplicate pending actions rather than waiting cleanly.
     3. **Missing stakes sensitivity on high-visibility commitments:** On high-stakes commitments (`ρ_stakes ≥ θ_high`), waiting through 2–3 exponential retries risks blowing `due` silently (perceived as concealment / late unannounced breach).
   - *Corrected Rule (Body-general):*
     Let $i \ge 1$ be the retry count, $\rho_{\text{stakes}}$ the promise stakes, and `margin_inform` a lead-time buffer before `due` (start `0.2 · (due - t_open)` clamped to `[30 s, 900 s]`, range `[0 s, 1800 s]`):
     - On **`refused`**: Do **not** auto-retry the identical act ($n_{\max} = 0$); immediately fire breaker to wake `reason` (`inform`, `renegotiate`, or choose alternative capability).
     - On **`held`**: Stay open, suppress duplicate retries while waiting for external release; fire breaker to `inform`/`renegotiate` only if `now >= due - margin_inform` (or body cancels hold).
     - On **`failed` or `unknown`**: Silently retry at $t_{\text{next}} = \text{now} + b\cdot 2^{i-1}$ **if and only if**:
       $$i \le n_{\max}(\rho_{\text{stakes}}) \quad\text{AND}\quad t_{\text{next}} + \widehat{\text{lat}}_{\max} \le \text{due} - \text{margin\_inform}$$
       where $n_{\max} = 2$ (range `1–3`, per Esterwood & Robert 2023b three-strikes boundary) for $\rho_{\text{stakes}} < \theta_{\text{high}}$, and $n_{\max} = 1$ (at most 1 fast retry) for $\rho_{\text{stakes}} \ge \theta_{\text{high}}$. Otherwise fire breaker immediately so `reason` informs or renegotiates *before* `due` expires.
3. **Lab Experiment:**
   - *Scenario:* Promise-fulfillment tasks crossing receipt type (`transient_failed`, `deterministic_refused`, `external_held`, `timeout_unknown`) $\times$ `due` slack (tight vs generous) $\times$ downstream counterparty dependency (another actor waits on the promise at `due`).
   - *Ablations:* (a) `core-and-lab` uniform backoff `t + b·2^(i-1)` on all 4 non-`done` receipts; (b) typed receipt routing (`refused -> 0`, `held -> wait until margin`, `failed/unknown -> gated backoff`) with `n_max ∈ {1, 2, 3}` and `margin_inform ∈ {0, 0.1·window, 0.2·window}`.
   - *Metrics:* Kept-before-due rate, *informed-before-due* rate (avoiding silent overdue surprises), duplicate/spam action count on `refused`/`held`, and simulated partner trust / promise-debt penalty (`lab/spec.md` honesty and keeping-commitments metrics).

---

#### R-14 For combining signals of a backlog chunk, what supports `max` vs `noisy-OR`?

1. **Verdict:** **Settled** in favor of **`max` over `noisy-OR`** across events within a single place's backlog chunk. However, **`core-and-lab`'s exact per-signal `max` implementation has an algebraic bug** when positive and negative gate signals mix across events in the same chunk.
2. **Evidence & Rule:**
   - *Why `noisy-OR` fails mathematically:* Pearl (1988, ISBN `978-0-934613-73-6` `[PR]`) and Heckerman & Breese (1996, `10.1109/3468.541341` `[PR]`) prove that the Noisy-OR gate:
     $$P_{\text{noisy-OR}}(e_1,\dots,e_m) = 1 - \prod_{i=1}^m (1 - s(e_i))$$
     requires **causal / statistical independence** among exceptions across events $e_i$. Likewise, Pelli (1985, `10.1364/josaa.2.001508` `[PR]`) demonstrates in psychophysical signal detection that probability summation ($1-\prod(1-p_i)$) over correlated or irrelevant distractors causes severe threshold drop and false-alarm inflation, whereas optimal decision under correlated uncertainty follows `max` (peak signal) rather than sum. Events piling up inside **one place** while the soul was asleep or saturated are *strongly positively correlated* (one thread, one speaker burst, one shared context). Using `noisy-OR`, 30 mundane chat lines each with routine salience `s = 0.10` yield $1 - (1 - 0.10)^{30} = 0.958$, firing the gate purely from message count—directly violating MonoAgens's R-14 volume-invariance requirement (*"keeps volume alone from firing the gate"*).
   - *Chunk capacity:* Cowan (2001, `10.1017/s0140525x01003922` `[PR]`) establishes working-memory capacity around **$4 \pm 1$ chunks**, directly supporting `core-and-lab`'s cap of **at most 4 chunks per wake** ordered by peak salience (generic across all bodies).
   - *Bug found in `core-and-lab` (`design.md` L195 & `specs/appraisal-gate/spec.md` L118–120):*
     `design.md` L195 writes `s_k = max_i s_k(e_i)·(1 + L_i)^(-d)` **independently for each signal $k$**, and `spec.md` L118–120 specifies *"Each signal of the chunk SHALL be the highest of its members' values after each is discounted by its age"*. In the gate equation (`design.md` Decision 7), four signals have positive weights (`rel`, `surp`, `urg`, `gap`) while `social_cost` has a **negative** weight (`-w_sc`). Suppose a backlog in place `P` contains:
     - $e_1$: an unanswered urgent question directly addressed to the soul (`rel=0.8, urg=0.7, social_cost=0.0`);
     - $e_2$: a casual greeting between two others (`rel=0.1, urg=0.0, sel_other=1 \implies \text{social\_cost}=1.0$).
     Independent per-signal `max_i` takes `rel=0.8, urg=0.7` from $e_1$ **and** `social_cost=1.0` from $e_2$, letting $e_2$'s social penalty wrongly suppress $e_1$!
   - *Corrected Rule (Body-general):* Preserve the pairing between positive signals and `social_cost` at the event level by scoring each backlog member first and taking the chunk score from its highest-utility event $i^*$:
     $$i^* = \arg\max_{i \in \text{chunk}} \Big[\big(u_{\text{gate}}(e_i)\big) \cdot (1 + L_i)^{-d}\Big], \qquad \text{chunk\_signals} = \big\{s_k(e_{i^*}) \cdot (1 + L_{i^*})^{-d}\big\}_{k=1..5}$$
     (Alternative equivalent when reporting vector summaries: apply `max_i` over positive signals $k \in \{1..4\}$ and set `social_cost_chunk` to `min_i [social_cost(e_i)·(1+L_i)^(-d)]`—zero if *any* event in the backlog had `social_cost = 0`—or take all signals from peak event $i^*$). In all cases, if **any** event in the chunk links to an active commitment (`commit(e_i) = 1`), the commitment breaker fires regardless of `max`.
3. **Lab Experiment:**
   - *Scenario:* Two targeted backlog stress suites: (1) *Flood test*: 5, 15, 30, and 60 correlated low-salience messages (`s ≈ 0.08–0.12`) arriving during sleep in one place; (2) *Mixed-signal test*: 1 urgent unaddressed direct question mixed with 5 `sel_other = 1` third-party messages in the same place backlog.
   - *Ablations:* (a) `noisy-OR` across members; (b) `core-and-lab` independent per-signal `max_k`; (c) proposed event-level `max` / peak-$i^*$ pairing.
   - *Metrics:* False wake rate vs burst length $N \in \{5, 15, 30, 60\}$ (volume invariance), missed-urgent-question rate under mixed `sel_other` bursts, and chunk-ordering F1 across top-4 chunks per wake.

---

### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
| :--- | :--- | :--- | :--- | :--- |
| $\lambda_c$ | Closeness discount on `sel_other` social cost | `0.35` | `[0.0, 0.7]` | Tune in Lab (directional support: Brown & Levinson 1987; Templeton et al. 2023; caveat: Wolfson 1988) |
| $\lambda_a$ | Closeness discount on `answered` social cost | `0.0` | `[0.0, 0.3]` | Tune in Lab (separates redundancy from chiming in) |
| $B_{\min}$ | Prior ACT-R activation for `0`-exchange entity | `-4.0` | `[-6.0, -2.0]` | Tune in Lab (fixes `ln(0)` bug in `design.md`; $\sigma(-4.0)\approx 0.018$) |
| $\alpha_P$ | Weight of place $B_P$ vs mean $\tilde{B}_p$ in $B_{\text{ctx}}$ | `0.5` (or `1/(1+\|Present\|)`) | `[1/(1+\|Present\|), 0.7]` | Tune in Lab (Anderson et al. 2004 source-sharing vs group dilution) |
| $i_{\text{reason\_cap}}$ | Upper cap on calibrated `i_reason` | `0.80` | `[0.50, 1.00]` | Tune in Lab (prevents inflation tying `i_commit = 1`; Zhou et al. 2026) |
| $n_{\text{cal}}$ | Min ratings before quantile map $Q_m$ activates | `50` | `[20, 200]` | Tune in Lab (Subasic et al. 2026 streaming quantile calibration) |
| $D_{\max}$ | Maximum nested `goal` tree depth | `3` | `[2, 5]` | Tune in Lab (bounds prompt ancestor context & recursion) |
| $\theta_{\text{med}}, \theta_{\text{high}}$ | Stakes thresholds for Tier 2 & Tier 3 grounding | `0.40`, `0.75` | `[0.20, 0.60]`, `[0.60, 0.90]` | Tune in Lab (Clark & Brennan 1991; Roque & Traum 2008) |
| $k_{\text{agency}}$ | Comparator normalized error tolerance multiplier | `2.5` | `[1.5, 4.0]` | Tune in Lab (Lanillos et al. 2020; human bounds in Haggard 2002 / Shimada 2022) |
| $n_{0,\text{agency}}$ | Shrinkage pseudo-observations for `[lat_min, lat_max, σ]` | `10` | `[5, 25]` | Tune in Lab (empirical Bayes shrinkage per channel) |
| $n_{\max}$ | Max silent retries (`low/med` stakes vs `high` stakes) | `2` / `1` | `1–3` / `0–2` | Esterwood & Robert 2023b ("3 strikes"); refine in Lab |
| $b_{\text{retry}}$ | Base backoff interval for `failed`/`unknown` retry | `30 s` (body-scaled) | `[10 s, 300 s]` | Tune in Lab (subject to `t_next + lat_max ≤ due - margin_inform`) |
| $\text{margin\_inform}$ | Lead time before `due` forcing inform/renegotiate | `clamp(0.2·window, 30 s, 900 s)` | `[0 s, 1800 s]` | Tune in Lab (Robinette et al. 2015; Schweitzer et al. 2006) |
| $d_{\text{backlog}}$ | Backlog member age decay exponent | `0.5` | `[0.25, 1.0]` | Tune in Lab (matches memory power-law decay `d`, Anderson et al. 2004) |
| $C_{\text{wake}}$ | Max backlog chunks handled per wake | `4` | `[2, 6]` | Cowan 2001 (`4 ± 1` STM chunks); verify in Lab |

---

### Neutral value

Each rule has an explicit identity/ablation point that reduces cleanly to neutral baseline behavior:
- **R-02:** $\lambda_c = 0, \lambda_a = 0$ removes all closeness discounting (`social_cost = max(sel_other, answered)`).
- **R-04:** Setting $\alpha_P = 1/(1 + |\text{Present}|)$ with $B_{\min} = 0$ recovers the naive numeric average; setting $w_{\text{fam}} = 0$ disables familiarity.
- **R-05:** Setting $i_{\text{reason\_cap}} = 1.0$ and $Q_m = \text{id}$ recovers raw uncalibrated prompt scores; setting `i_reason = 0` tests pure `max(i_commit, i_surprise)`.
- **R-07:** Setting `parent_goal_id = None` (or $D_{\max} = 1$) reduces to flat single-level goals as in current `design.md`.
- **R-08:** Setting $\theta_{\text{med}} = \theta_{\text{high}} = 1.0$ accepts Tier 1 (`continued_attention` / next event / `done`) for all communicative acts (current `core-and-lab` behavior).
- **R-09:** Setting $n_{0,\text{agency}} \to \infty$ disables online adaptation (keeping static body priors); setting $k_{\text{agency}} = 0$ accepts `self` only via `correlation_id` or `actor == self`.
- **R-10:** Treating all non-`done` receipts uniformly with $n_{\max} = \text{const}$ and $\text{margin\_inform} = 0$ reduces to `core-and-lab`'s current silent-retry rule.
- **R-14:** On single-event bursts ($|\text{chunk}| = 1$), event-level `max` equals per-signal `max` and equals `noisy-OR` for $s \in [0, 1]$ (trivially invariant).

---

### Claim

1. Separating pre-draw `social_cost` (with split $\lambda_c, \lambda_a$) from post-`go` `patience`, and short-circuiting `patience` when `answered = 1`, reduces unsolicited `sel_other` interruptions and redundant backlog waits without hurting open-floor helpfulness.
2. Giving zero-exchange participants an explicit floor $B_{\min} < 0$ eliminates `ln(0)` ambiguity in $B_{\text{ctx}}$, preserving group familiarity signal without letting one stranger veto a trusted room (`min`) or 100 strangers yield `familiarity = 0.5`.
3. Replacing unanchored 1–10 `i_reason` with a 5-tier anchored rubric + capped rolling quantile normalization preserves `i_commit = 1` retention under LLM central tendency and score inflation.
4. Adding acyclic `parent_goal_id` + ancestor `ContextPlan` + cascade cancellation enables multi-step interrupted goal completion and eliminates post-completion orphan actions with zero new primitives.
5. Tiering grounding evidence by $\rho_{\text{stakes}}$ cuts silent misunderstanding errors on high-stakes commitments without adding clarification overhead on low-stakes phatic chat.
6. Retaining body-declared comparator windows plus `n_0` online shrinkage generalizes self-other attribution across millisecond embodied sensors and multi-second chat APIs.
7. Differentiating `refused` (0 retries, instant inform) and `held` (wait until lead margin) from transient `failed`/`unknown` reduces late unannounced breaches and eliminates futile retries on hard refusals.
8. Event-level decayed `max` across backlog chunks preserves gate false-wake invariance as backlog size scales $5 \to 60$, and closes `core-and-lab`'s cross-event `social_cost` leakage bug.

---

### How the Lab would test it

1. **Deterministic ablation protocol (`specs/lab/spec.md`):** Run all proposed rules against the existing development and validation suites (**never reading or overfitting `lab/scenarios/sealed/`**) under paired single-factor ablations against each mechanism's **Neutral value** above.
2. **Core metrics mapped per mechanism:**
   - *Gate & attention discipline (R-02, R-04, R-14):* False-wake rate under 5–60 event bursts; `sel_other` interruption rate; backlog `answered = 1` double-wait count; chunk-priority accuracy across 4-chunk wake caps.
   - *Calibration & agency (R-05, R-08, R-09):* Importance `recall@k` & ECE vs oracle labels; high-stakes premature-grounding error vs low-stakes over-asking rate; self-other `basis = self` confusion matrix across multi-latency chat and simulated embodied environments.
   - *Commitments & nested goals (R-07, R-10):* Promise kept-or-informed-before-due rate; duplicate retry count on `refused`/`held`; hierarchical goal completion, orphan post-completion error rate, and post-interruption resumption latency.

---

### Established / inferred / open

#### Established by the literature
- **R-02:** Speaker selection governs turn ownership (Sacks et al. 1974; Lerner 2003); lower social distance reduces face-threat weight (Brown & Levinson 1987) and relaxes turn-gap/overlap norms between friends vs strangers (Tannen 2005; Coates 1996; Bilous & Krauss 1988; Templeton et al. 2022, 2023), while speech-act effort is non-monotonic in acquaintance ranges (Wolfson 1988).
- **R-04:** ACT-R distributes fixed source activation budget $W$ equally across $n$ cue/focus slots ($W_j = W/n$, additive log-activation; Anderson 1974; Anderson et al. 2004, 2007), which matches weighted averaging over a hard `min` veto.
- **R-05:** Park et al. (2023) did not validate or ablate raw 1–10 importance calibration; LLMs exhibit persistent central tendency / endpoint compression unaffected by few-shot full-range exemplars (Zhang et al. 2026), scale/round-number/prompt instability (Stureborg et al. 2024; Wang et al. 2024), distributional ordinal shift (Subasic et al. 2026), and strategic inflation under allocation pressure (Zhou et al. 2026).
- **R-07:** Soar uses impasse subgoaling (Laird et al. 1987; Laird 2012), while ACT-R showed human goals obey activation-based non-LIFO resumption requiring post-completion suppression (Altmann & Trafton 2002; Anderson et al. 2004); executive WM tracks superordinate context (Miller & Cohen 2001; Marsh & Hicks 1998); typed deterministic PM state tracking beats free-text LLM tracking (Zhao & Wu 2026; Bei et al. 2026).
- **R-08:** Grounding requires ordered evidence tiers (Clark & Schaefer 1989) scaled to task purposes/stakes (Clark & Brennan 1991; Roque & Traum 2008; Paek & Horvitz 2000).
- **R-09:** Human intentional binding peaks around `250 ms` and decays over `450–650 ms` (Haggard et al. 2002; Moore & Obhi 2012; Arikan et al. 2017); multisensory self-body integration spans `~200–300 ms` (Shimada 2022; Blakemore et al. 1999, 2000; Franck et al. 2001); robotics self-other distinction uses covariance-normalized prediction error + temporal contingency (Gold & Scassellati 2009; Lanillos & Cheng 2018; Lanillos et al. 2020; Chen et al. 2026).
- **R-10:** Unannounced concealment/deception destroys trust recovery permanently (Schweitzer et al. 2006); prompt transparent recovery mitigates competence failures (Kim et al. 2004; Smith et al. 1999; Wirtz & Mattila 2004; Bitner et al. 1990; Robinette et al. 2015; Nayyar & Wagner 2018; de Visser et al. 2018; Baker et al. 2018; Tolmeijer et al. 2020); trust collapses past ~3 consecutive failures (Esterwood & Robert 2023a, 2023b).
- **R-14:** `noisy-OR` presupposes independent causal exceptions (Pearl 1988; Heckerman & Breese 1996) and probability summation over correlated distractors inflates false alarms (Pelli 1985); working-memory chunk capacity centers near 4 (Cowan 2001).

#### Inferred (engineering synthesis consistent with `ARCHITECTURE.md` & literature)
- Separating `λ_c` (`sel_other`) from `λ_a` (`answered`) and short-circuiting post-`go` `patience` when `answered = 1` (**R-02**).
- Introducing bounded zero-exchange prior `B_min` and parameterized `α_P` to fix `core-and-lab`'s `ln(0)` bug (**R-04**).
- Combining a 5-tier anchored rubric, rolling quantile map `Q_m`, and `i_reason_cap < 1.0` so uncalibrated ratings cannot crowd out `i_commit = 1` (**R-05**).
- Realizing hierarchical goals purely via `parent_goal_id` + ancestor `ContextPlan` + cascade cancel (**R-07**), tiering `core-and-lab`'s grounding checks via `θ_med`/`θ_high` (**R-08**), adding `n_0` empirical-Bayes shrinkage to channel latency/tolerance (**R-09**), splitting `refused`/`held` vs `failed`/`unknown` receipts (**R-10**), and switching backlog `max` from independent per-signal to peak-event pairing (**R-14**).

#### Open (not settled by the literature; must be tuned or decided in the Lab)
- Exact numeric slopes/weights: $\lambda_c, \lambda_a$ (**R-02**), $B_{\min}$ and $\alpha_P$ for large rooms (**R-04**), optimal $i_{\text{reason\_cap}}$ and whether `i_reason` improves recall over `commit + surprise` alone (**R-05**), depth cap $D_{\max}$ (**R-07**), stakes thresholds $\theta_{\text{med}}, \theta_{\text{high}}$ (**R-08**), per-body $k_{\text{agency}}$ and `n_0` (**R-09**), exact retry cap `n_max` by stakes and `margin_inform` (**R-10**), and whether `max` per positive signal + `min` social_cost beats strict single-event `i*` projection (**R-14**).

---

### Sources

#### R-02 (Turn-taking, politeness, overlap/closeness)
- `[PR]` Sacks, H., Schegloff, E. A., & Jefferson, G. (1974). A simplest systematics for the organization of turn-taking for conversation. *Language*, 50(4), 696–735. DOI: `10.1353/lan.1974.0010`
- `[PR]` Lerner, G. H. (2003). Selecting next speaker: The context-sensitive operation of a context-free organization. *Language in Society*, 32(2), 177–201. DOI: `10.1017/S004740450332202X`
- `[PR]` Brown, P., & Levinson, S. C. (1987). *Politeness: Some Universals in Language Usage*. Cambridge University Press. DOI: `10.1017/CBO9780511813085`
- `[PR]` Stivers, T., Enfield, N. J., Brown, P., Englert, C., Hayashi, M., Heinemann, T., Hoymann, G., Rossano, F., de Ruiter, J. P., Yoon, K.-E., & Levinson, S. C. (2009). Universals and cultural variation in turn-taking in conversation. *Proceedings of the National Academy of Sciences*, 106(26), 10587–10592. DOI: `10.1073/pnas.0903616106`
- `[PR]` Tannen, D. (2005 [1984]). *Conversational Style: Analyzing Talk Among Friends*. Oxford University Press. DOI: `10.1093/oso/9780195221817.001.0001`
- `[PR]` Coates, J. (1996). *Women Talk: Conversation Between Women Friends*. Blackwell Publishers. ISBN `978-0631182528` (review DOI: `10.2307/417904`)
- `[PR]` Bilous, F. R., & Krauss, R. M. (1988). Dominance and accommodation in the conversational behaviours of same- and mixed-gender dyads. *Language & Communication*, 8(3–4), 183–194. DOI: `10.1016/0271-5309(88)90016-X`
- `[PR]` Kennedy, C. W., & Camden, C. T. (1983). A new look at interruptions. *Western Journal of Speech Communication*, 47(1), 45–58. DOI: `10.1080/10570318309374104`
- `[PR]` Templeton, E. M., Chang, L. J., Reynolds, E. A., Cone LeBeaumont, M. D., & Wheatley, T. (2022). Fast response times signal social connection in conversation. *Proceedings of the National Academy of Sciences*, 119(4), e2116915119. DOI: `10.1073/pnas.2116915119`
- `[PR]` Templeton, E. M., Chang, L. J., Reynolds, E. A., Cone LeBeaumont, M. D., & Wheatley, T. (2023). Long gaps between turns are awkward for strangers but not for friends. *Philosophical Transactions of the Royal Society B*, 378(1875), 20210471. DOI: `10.1098/rstb.2021.0471`
- `[PR]` Wolfson, N. (1988). The bulge: A theory of speech behavior and social distance. *Penn Working Papers in Educational Linguistics*, 2(1), 55–83; also in J. Fine (Ed.), *Second Language Discourse: A Textbook of Current Research*, Ablex, pp. 21–38. ISBN `978-0893914134`

#### R-04 (ACT-R activation & fan effect)
- `[PR]` Anderson, J. R. (1974). Retrieval of propositional information from long-term memory. *Cognitive Psychology*, 6(4), 451–474. DOI: `10.1016/0010-0285(74)90021-8`
- `[PR]` Anderson, J. R., Bothell, D., Byrne, M. D., Douglass, S., Lebiere, C., & Qin, Y. (2004). An integrated theory of the mind. *Psychological Review*, 111(4), 1036–1060. DOI: `10.1037/0033-295X.111.4.1036`
- `[PR]` Anderson, J. R. (2007). *How Can the Human Mind Occur in the Physical Universe?* Oxford University Press. DOI: `10.1093/acprof:oso/9780195324259.001.0001`

#### R-05 (LLM importance & judge calibration)
- `[PR]` Park, J. S., O'Brien, J. C., Cai, C. J., Morris, M. R., Liang, P., & Bernstein, M. S. (2023). Generative Agents: Interactive Simulacra of Human Behavior. *Proceedings of UIST 2023*, pp. 1–22. DOI: `10.1145/3586183.3606763` (`arXiv:2304.03442`)
- `[PR]` Wang, P., Li, L., Chen, L., Cai, Z., Zhu, D., Lin, B., Cao, Y., Kong, L., Liu, Q., Liu, T., & Sui, Z. (2024). Large Language Models are not Fair Evaluators. *Proceedings of ACL 2024 (Volume 1: Long Papers)*, pp. 9440–9450. DOI: `10.18653/v1/2024.acl-long.511`
- `[PP]` Stureborg, R., Alikaniotis, D., & Suhara, Y. (2024). Large Language Models are Inconsistent and Biased Evaluators. `arXiv:2405.01724`
- `[PP]` Zhang, J., Elluri, S., Cherukuvada, B., Joffe, Y., et al. (2026). Auditing Multimodal LLM Raters: Central Tendency Bias in Clinical Ordinal Scoring. `arXiv:2605.16386`
- `[PP]` Subasic, I., Rabinovich, A., & Chen, Z. (2026). Quantiles, Not Points: Streaming Quantile Matching for Ordinal-Aware LLM-as-a-Judge Calibration. `arXiv:2611.00447`
- `[PP]` Zhou, J., Zhang, Z., Xu, X., & Wu, J. (2026). Strategic Calibration in LLM Multi-Agent Systems: Audited Pivot Scoring for Truthful Elicitation. `arXiv:2610.33728`

#### R-07 (Nested goals & prospective memory)
- `[PR]` Laird, J. E., Newell, A., & Rosenbloom, P. S. (1987). SOAR: An architecture for general intelligence. *Artificial Intelligence*, 33(1), 1–64. DOI: `10.1016/0004-3702(87)90050-6`
- `[PR]` Laird, J. E. (2012). *The Soar Cognitive Architecture*. MIT Press. DOI: `10.7551/mitpress/7688.001.0001`
- `[PR]` Altmann, E. M., & Trafton, J. G. (2002). Memory for goals: an activation-based model. *Cognitive Science*, 26(1), 39–83. DOI: `10.1016/S0364-0213(01)00058-1`
- `[PR]` Miller, E. K., & Cohen, J. D. (2001). An integrative theory of prefrontal cortex function. *Annual Review of Neuroscience*, 24(1), 167–202. DOI: `10.1146/annurev.neuro.24.1.167`
- `[PR]` Marsh, R. L., & Hicks, J. L. (1998). Event-based prospective memory and executive control of working memory. *Journal of Experimental Psychology: Learning, Memory, and Cognition*, 24(2), 336–349. DOI: `10.1037/0278-7393.24.2.336`
- `[PP]` Liu, G., & Gabriel, S. (2026). PM-Bench: Evaluating Prospective Memory in LLM Agents. `arXiv:2607.12385`
- `[PP]` Zhao, J., & Wu, C. (2026). Making Prospective Memory SLM-Shaped: Typed Intention Stores for Small-Model Agents. `arXiv:2609.01272`
- `[PP]` Bei, Y., Liu, Z., Wei, T., Zhang, W., et al. (2026). Prospective Memory for Proactive LLM Agents. `arXiv:2611.05731`

#### R-08 (Grounding criterion & evidence tiers)
- `[PR]` Clark, H. H., & Schaefer, E. F. (1989). Contributing to discourse. *Cognitive Science*, 13(2), 259–294. DOI: `10.1207/s15516709cog1302_7`
- `[PR]` Clark, H. H., & Brennan, S. E. (1991). Grounding in communication. In *Perspectives on Socially Shared Cognition*, APA, pp. 127–149. DOI: `10.1037/10096-006`
- `[PR]` Roque, A., & Traum, D. (2008). Degrees of grounding based on evidence of understanding. *Proceedings of the 9th SIGdial Workshop on Discourse and Dialogue*, ACL, pp. 54–63. URL: `https://aclanthology.org/W08-0107/`
- `[PR]` Paek, T., & Horvitz, E. J. (2000). Conversation as action under uncertainty. *Proceedings of UAI 2000* (`arXiv:1301.3883`)

#### R-09 (Comparator model, intentional binding, robotics self-other distinction)
- `[PR]` Haggard, P., Clark, S., & Kalogeras, J. (2002). Voluntary action and conscious awareness. *Nature Neuroscience*, 5(4), 382–385. DOI: `10.1038/nn827`
- `[PR]` Arikan, B. E., van Kemenade, B. M., Straube, B., Harris, L. R., & Kircher, T. (2017). Voluntary and involuntary movements widen the window of subjective simultaneity. *i-Perception*, 8(4), 2041669517719297. DOI: `10.1177/2041669517719297`
- `[PR]` Blakemore, S.-J., Frith, C. D., & Wolpert, D. M. (1999). Spatio-temporal prediction modulates the perception of self-produced stimuli. *Journal of Cognitive Neuroscience*, 11(5), 551–559. DOI: `10.1162/089892999563607`
- `[PR]` Blakemore, S.-J., Wolpert, D., & Frith, C. (2000). Why can't you tickle yourself? *NeuroReport*, 11(11), R11–R16. DOI: `10.1097/00001756-200008030-00002`
- `[PR]` Franck, N., Farrer, C., Georgieff, N., Marie-Cardine, M., Daléry, J., d'Amato, T., & Jeannerod, M. (2001). Defective recognition of one's own actions in patients with schizophrenia. *American Journal of Psychiatry*, 158(3), 454–459. DOI: `10.1176/appi.ajp.158.3.454`
- `[PR]` Moore, J. W., & Obhi, S. S. (2012). Intentional binding and the sense of agency: A review. *Consciousness and Cognition*, 21(1), 546–561. DOI: `10.1016/j.concog.2011.12.002`
- `[PR]` Shimada, S. (2022). Multisensory and sensorimotor integration in the embodied self: Relationship between self-body recognition and the mirror neuron system. *Sensors*, 22(13), 5059. DOI: `10.3390/s22135059`
- `[PR]` Gold, K., & Scassellati, B. (2009). Using probabilistic reasoning over time to self-recognize. *Robotics and Autonomous Systems*, 57(4), 384–392. DOI: `10.1016/j.robot.2008.07.006`
- `[PR]` Lanillos, P., & Cheng, G. (2018). Adaptive robot body learning and estimation through predictive coding. *2018 IEEE/RSJ IROS*, pp. 4083–4090. DOI: `10.1109/iros.2018.8593684` (`arXiv:1805.03104`)
- `[PP]` Lanillos, P., Pages, J., & Cheng, G. (2020). Robot self/other distinction: active inference meets neural networks learning in a mirror. `arXiv:2004.05473`
- `[PP]` Chen, Y., Gao, T., Ge, Y., Ban, S., et al. (2026). Proprioceptive-visual correspondence enables self-other distinction in humanoid robots. `arXiv:2606.13222`

#### R-10 (Promise breaking, service recovery, trust repair)
- `[PR]` Schweitzer, M. E., Hershey, J. C., & Bradlow, E. T. (2006). Promises and lies: Restoring violated trust. *Organizational Behavior and Human Decision Processes*, 101(1), 1–19. DOI: `10.1016/j.obhdp.2006.05.005`
- `[PR]` Kim, P. H., Ferrin, D. L., Cooper, C. D., & Dirks, K. T. (2004). Removing the shadow of suspicion: The effects of apology versus denial for repairing competence- versus integrity-based trust violations. *Journal of Applied Psychology*, 89(1), 104–118. DOI: `10.1037/0021-9010.89.1.104`
- `[PR]` Bitner, M. J., Booms, B. H., & Tetreault, M. S. (1990). The service encounter: Diagnosing favorable and unfavorable incidents. *Journal of Marketing*, 54(1), 71–84. DOI: `10.1177/002224299005400105`
- `[PR]` Smith, A. K., Bolton, R. N., & Wagner, J. (1999). A model of customer satisfaction with service encounters involving failure and recovery. *Journal of Marketing Research*, 36(3), 356–372. DOI: `10.1177/002224379903600305`
- `[PR]` Wirtz, J., & Mattila, A. S. (2004). Consumer responses to compensation, speed of recovery and apology after a service failure. *International Journal of Service Industry Management*, 15(2), 150–166. DOI: `10.1108/09564230410532484`
- `[PR]` de Visser, E. J., Pak, R., & Shaw, T. H. (2018). From 'automation' to 'autonomy': the importance of trust repair in human–machine interaction. *Ergonomics*, 61(10), 1409–1427. DOI: `10.1080/00140139.2018.1457725`
- `[PR]` Baker, A. L., Phillips, E. K., Ullman, D., & Keebler, J. R. (2018). Toward an understanding of trust repair in human-robot interaction. *ACM Transactions on Interactive Intelligent Systems*, 8(4), 1–30. DOI: `10.1145/3181671`
- `[PR]` Tolmeijer, S., Weiss, A., Hanheide, M., Lindner, F., Powers, T. M., Dixon, C., & Tielman, M. L. (2020). Taxonomy of trust-relevant failures and mitigation strategies. *Proceedings of ACM/IEEE HRI 2020*, pp. 3–12. DOI: `10.1145/3319502.3374793`
- `[PR]` Robinette, P., Howard, A. M., & Wagner, A. R. (2015). Timing is key for robot trust repair. *ICSR 2015*, LNCS, pp. 574–583. DOI: `10.1007/978-3-319-25554-5_57`
- `[PR]` Nayyar, M., & Wagner, A. R. (2018). When should a robot apologize? Understanding how timing affects human-robot trust repair. *ICSR 2018*, LNCS, pp. 265–274. DOI: `10.1007/978-3-030-05204-1_26`
- `[PR]` Esterwood, C., & Robert, L. P. (2023a). The theory of mind and human–robot trust repair. *Scientific Reports*, 13(1), 9877. DOI: `10.1038/s41598-023-37032-0`
- `[PR]` Esterwood, C., & Robert, L. P. (2023b). Three Strikes and you are out!: The impacts of multiple human–robot trust violations and repairs on robot trustworthiness. *Computers in Human Behavior*, 142, 107658. DOI: `10.1016/j.chb.2023.107658`

#### R-14 (Noisy-OR independence, probability summation, chunk capacity)
- `[PR]` Pearl, J. (1988). *Probabilistic Reasoning in Intelligent Systems: Networks of Plausible Inference*. Morgan Kaufmann. ISBN `978-0934613736` / `978-1558604797`
- `[PR]` Heckerman, D., & Breese, J. S. (1996). Causal independence for probability assessment and inference using Bayesian networks. *IEEE Transactions on Systems, Man, and Cybernetics - Part A: Systems and Humans*, 26(6), 826–831. DOI: `10.1109/3468.541341`
- `[PR]` Pelli, D. G. (1985). Uncertainty explains many aspects of visual contrast detection and discrimination. *Journal of the Optical Society of America A*, 2(9), 1508. DOI: `10.1364/josaa.2.001508`
- `[PR]` Cowan, N. (2001). The magical number 4 in short-term memory: A reconsideration of mental storage capacity. *Behavioral and Brain Sciences*, 24(1), 87–114. DOI: `10.1017/s0140525x01003922`

---

## Part 2 — Open notes R-19 to R-28

> **Scope:** Second half of Research Brief 2 **A-13**, resolving the seven remaining open notes from [2026-11-15-run-1.md](file:///home/muqsith/Projects/Mono/research/findings/2026-11-15-run-1.md) (`R-19`, `R-20`, `R-24`, `R-25`, `R-26`, `R-27`, `R-28`) against [core-and-lab design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md), the `core-and-lab` specs, [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md), and primary literature verified directly in this session.
>
> **Verification date:** November 16, 2026. Repository files were not edited. Every external paper, DOI, arXiv ID, repository license, equation, and quote below was verified against primary sources during this session; anything inferred beyond what those sources directly prove is labeled **Inferred** or **Open**.

---

### Executive Summary

| ID | Topic | Prompt Verdict | Brief-2 Status | Body Scope | Impact on `core-and-lab` / `RESEARCH.md` |
|---|---|---|---|---|---|
| **R-19** | Sampling fraction $\rho$ of `silent` decisions + IPS correction | `partly settled` | `partly answered` | Rule generic across bodies (`abstain/observe`); default `silent` omission wording in spec is chat-specific | **2 critical bugs found** in `bounded-learning/spec.md` & `design.md`: sign inversion on missed silences + missing $1/\rho$ IPS weight and variance clipping |
| **R-20** | Domain-specific Jøsang forgetting factor $\lambda$ | `partly settled` | `partly answered` | Generic across bodies | Compatible with `design.md` §10 (`beliefs/spec.md`); supports domain-tiered or hazard-modulated $\lambda_d$ keyed by opaque domain ID |
| **R-24** | Quantitative overlap threshold $J$ for episode merge | `not settled by the literature` | `not settled by the literature` | Generic across bodies | Neuroscience/cognitive science proves boundary & interference effects but supplies no universal Jaccard threshold $J$; must tune in Lab |
| **R-25** | ToM card capacity (`ledger_items`) & belief-depth limits | `partly settled` | `partly answered` | Multi-actor epistemic structure generic; cited benchmarks are chat/narrative-centered | Supports keeping `max_order = 2`, bounding `ledger_items` tightly (start `8–10` per field, search `4–15`), and pruning stale entries |
| **R-26** | Safe online learning scope & per-actor teacher weights | `partly settled` | `partly answered` | Generic across bodies | **2 issues found**: `design.md` leaves threshold learning & scale collinearity unconstrained; `bounded-learning/spec.md` diverges from `RESEARCH.md` §8 on global teachers without reliability shrinkage/caps |
| **R-27** | PITCH dataset availability, license, consent & chat baselines | `settled` (for PITCH) | `answered` | Chat-specific benchmark & human-data collection | PITCH (`arXiv:2610.25896`) is publicly released on GitHub under **MIT** license with verified IRB & post-hoc withdrawal consent |
| **R-28** | Primary-source verification of 4 Round-1 citations | `settled` | `answered` | Mixed (see details per paper) | All 4 citations verified at peer-reviewed venues (`[PR]`); adds discrete-turn clarification to `RESEARCH.md` L96 for Liu et al. `λ = 1.02` |

---

### Findings Affecting `core-and-lab` and `RESEARCH.md`

1. **[CRITICAL BUG — Sign Inversion] Missed silence is assigned `r = 0` in a `go`-polarity delta rule (`bounded-learning` & `design.md`).**
   - [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L11-L16) defines teaching signals as `Good` (`r = 1`) vs `Bad` (`r = 0`), explicitly classifying **`"a judged silence that was missed"`** as `Bad` (`r = 0`), while omitting answered silences.
   - [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L30-L33) and [design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L341) then update every signal weight via `w ← clamp(w + learning_rate·(r − P)·s)`, where `P` is the gate's **`go` probability**.
   - **Why this is wrong:** On a silent omission miss, `a = 0` (`no-go`) and `r = 0`, so `(r − P)·s = −P·s < 0` for positive signals (`s ≥ 0`). This **decreases** signal weights after a false silence, driving $P(\text{go})$ **down** on future similar events instead of **up**. Meanwhile, if an unjudged or answered silence is excluded from `Good`, only missed silences ever fire on `a = 0` and push weights in the wrong direction.
   - **Required fix:** Either (a) express the update in action-reward REINFORCE polarity $(a - P)\,(R(a) - b)\,w_{\text{ips}}\,s$ (so $a=0, R(0)=0, b>0 \implies (0-P)(0-b) = +P b > 0$, pushing $P$ **up** when silence fails and $a=0, R(0)=1, b<1 \implies -(1-b)P < 0$, pushing $P$ **down** when silence succeeds), or (b) map `a = 0` omissions to `go`-target label $y_{\text{speak}} = 1$ (since speaking was warranted) and `a = 0` answered silences to $y_{\text{speak}} = 0$, weighted by the sampling/IPS factor below. *(See `R-19`.)*

2. **[CRITICAL BUG — Propensity Bias] Unweighted combination of 100% `note` and `ρ`-sampled `silent` omissions (`design.md` Decision 8 & `bounded-learning`).**
   - [design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L249-L253) and [expectations/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/expectations/spec.md#L151-L167) open silence predictions on **100%** of `note` decisions and a seeded fraction **$\rho$** (start `0.02`) of `silent` decisions, but [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L30-L33) applies the same unweighted `learning_rate` step to all resolved predictions.
   - **Why this is wrong:** Conditional on abstaining (`a = 0`), borderline omissions (`note`, $P \ge \text{note\_band}$) are observed with propensity $1$, whereas low-probability omissions (`silent`, $P < \text{note\_band}$) are observed with propensity $\rho = 0.02$. Unweighted learning undercounts `silent` feedback by $50\times$ relative to `note` and biases the gradient away from $\nabla \mathbb{E}[R(a)] = P(1-P)(R(1)-R(0))$ toward $P(1-P)(R(1) - \rho R(0))$ on the `silent` region. Conversely, naive $1/\rho$ weighting inflates per-sample second moments by $1/\rho = 50\times$, requiring clipped IPS, SNIPS normalization, or a control-variate baseline (`R-19`).

3. **[STABILITY & IDENTIFIABILITY GAP] Unconstrained online updates across `"salience weights and threshold terms"` (`design.md` Decision 12 & `bounded-learning`).**
   - [design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L339-L343) states: `"salience weights and threshold terms from outcomes; timing from latencies (Decision 8); the wake band from classifier verdicts."`
   - Allowing free online optimization over all threshold coefficients `(θ₀, a, b, c, r)` alongside `w_k` creates two failure modes: (i) learning `b` (on `budget_used`) or `c` (on `promise_debt`) can erode resource caps and commitment compliance via reward hacking; (ii) simultaneously scaling `(w_k, θ₀)` without fixing temperature `τ` and scale introduces positive homogeneity / collinearity (`(k w, k θ, k τ)` invariance) and closed-loop performative oscillation. In addition, [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L82-L85) adapts `wake_gate_lo` toward `"the lowest similarity that received a positive classification, less a margin"` — a single minimum order statistic that can be pulled down permanently by one outlier false positive unless replaced by a lower empirical quantile (`q_0.05` / `q_0.10`) or running quantile with shrinkage. *(See `R-26`.)*

4. **[CONTRADICTION WITH `RESEARCH.md` & MISSING ROBUSTNESS] Unshrunk global `teacher_weight` in `bounded-learning/spec.md`.**
   - [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L163-L164) restricts global (`layer-1/2`) updates to `"the owner's reactions and classifier verdicts only"`, relegating other actors to layer-3 per-place/per-actor offsets.
   - [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L68-L72) follows Decision 1's invariant ("core never knows an owner") by accepting an external `teacher weight` per actor and scaling global updates linearly by it, with **no** upper bound / clipping, **no** online noise estimation, and **no** shrinkage when a teacher gives noisy, delayed, or conflicting feedback. If a host gateway assigns nonzero teacher weight to non-owner peers, Dawid–Skene / RE-PO results show naive linear weighting destabilizes global alignment unless weights are shrunk by inferred teacher reliability and strictly clipped. *(See `R-26`.)*

5. **[CITATION & FORMULA NUANCES] Verification of the four unverified Round-1 papers + SNIPS arXiv ID (`RESEARCH.md` & `run-1.md`).**
   - All four `R-28` citations are verified peer-reviewed `[PR]` publications. However, [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L96) cites Liu et al. (2025) `λ^(t − τ_p)` with `λ = 1.02` without stating that in the original CHI 2025 paper $t - \tau_p$ counts **discrete conversation turns**, not wall-clock minutes — applying `1.02` per minute or millisecond causes exponential runaway (`1.02^60 ≈ 3.28` in one hour).
   - Separately, in Round 1 (`R-19`), `arXiv:1505.07549` does **not** belong to Swaminathan & Joachims' SNIPS paper; the verified SNIPS primary source is the NeurIPS 2015 paper (`https://papers.nips.cc/paper_files/paper/2015/hash/39027dfad5138c9ca0c474d71db915c3-Abstract.html`) alongside Counterfactual Risk Minimization (`arXiv:1502.02362`).

---

#### R-19 What fraction $\rho$ of `silent` (non-event) decisions should be sampled to learn from omissions?

**Verdict:** `partly settled` *(Brief-2 status: `partly answered` — estimator mathematics and bias/variance bounds are settled; the exact scalar $\rho$ depends on non-stationarity rate, evaluation cost, and batch size and must be tuned in Lab).*

##### Evidence & Rule

**Answer.** A binary speak/stay-silent gate observed only on spoken actions (`go`) and borderline silences (`note`) suffers from deterministic missing-not-at-random censorship on `silent` ($a=0,\, P < \text{note\_band}$) decisions: without a non-zero counterfactual exploration/evaluation propensity $\rho > 0$ on `silent`, no off-policy estimator satisfies positivity for low-salience regions. To combine `go` ($p_{\text{eval}}=1$), `note` ($p_{\text{eval}}=1$), and `silent` ($p_{\text{eval}}=\rho$) outcomes without bias or gradient inversion, MonoAgens should sample `silent` omission checks deterministically at rate $\rho \in [0.005, 0.10]$ (start value `0.02`, increased up to `0.05–0.10` in low-volume or rapidly drifting places when classifier budget permits), count both **answered** and **missed** counterfactual silences with **action-reward polarity** $(a - P)$, and multiply `silent` counterfactual gradients by a **clipped inverse propensity weight** $\tilde{w} = \min(M,\, 1/\rho)$ (with clipping cap $M \in [5, 25]$, start value `10`) plus a mean-zero control-variate baseline $b_0 \in [0, 1]$ (or Self-Normalized IPS / Doubly Robust batch weighting).

**Formulas & Symbols.** Let an event $i$ produce gate speak probability $P_i = P(a_i=1 \mid x_i) = \sigma\!\big((S_{\text{eff},i} - \theta_i)/\tau\big)$ and seeded action $a_i = \mathbb{I}[u_i < P_i] \in \{0, 1\}$, where $u_i \sim \mathrm{hash}(\text{seed}, \text{event}_i.\text{id})$. Let evaluation indicator $e_i \in \{0, 1\}$ decide whether a counterfactual prediction is opened when $a_i = 0$:

$$
p_{\text{eval}}(a_i, P_i) =
\begin{cases}
1, & a_i = 1 \text{ (`go` prediction)} \\
1, & a_i = 0,\ P_i \ge \text{note\_band} \text{ (`note` silence prediction)} \\
\rho, & a_i = 0,\ P_i < \text{note\_band} \text{ (`silent` silence prediction; } e_i = \mathbb{I}[v_i < \rho]\text{)}
\end{cases}
$$

with independent deterministic draw $v_i = \mathrm{hash}(\text{seed}_{\text{eval}}, \text{event}_i.\text{id}) \in [0,1)$. Note that under this two-stage design, the **action** executed in the world is $a_i$ (already drawn with probabilities $P_i$ and $1 - P_i$), while $e_i$ only gates whether we expend memory/classifier cost to evaluate the `silent` decision (`100%` of `go` and `note`, `ρ` of `silent`).

1. **Proof that omitting `1/ρ` and keeping current `r=0` polarity biases learning (Mathematical Verification):**
   Suppose the true expected utility of action $a \in \{0, 1\}$ is $R(a) = \mathbb{E}[r_i \mid a_i = a, x_i] \in [0, 1]$, where for silence ($a=0$), $r_i = 1$ if silence succeeded (e.g., another participant replied or no follow-up failure occurred within expiry) and $r_i = 0$ if silence missed (omission error); for speaking ($a=1$), $r_i = 1$ if answered cleanly without miss/repair and $r_i = 0$ otherwise. Under the true objective $J(w) = \mathbb{E}_{x}\mathbb{E}_{a \sim P_w}[R(a)]$, the exact policy gradient on logit $z_i = (S_{\text{eff},i} - \theta_i)/\tau$ is:
   $$
   \frac{\partial J}{\partial z_i}
   = \frac{\partial}{\partial z_i}\big[P_i R(1) + (1-P_i) R(0)\big]
   = P_i(1-P_i)\big(R(1) - R(0)\big).
   $$
   Now inspect the score-function identity over observed outcomes when `silent` evaluations are gated by $e_i \sim \mathrm{Bernoulli}(\rho)$:
   - On an **on-policy REINFORCE gradient** using unweighted resolved events (dropping $1/\rho$), the expected update term on event $i$ (holding $\tau$ fixed and letting $s_k$ be feature $k$) is:
     $$
     \mathbb{E}\big[e_i (a_i - P_i) r_i \mid x_i,\, P_i < \text{note\_band}\big]
     = P_i(1-P_i) R(1) + (1-P_i)(-P_i)\,\rho\, R(0)
     = P_i(1-P_i)\big(R(1) - \rho R(0)\big),
     $$
     which is **biased** whenever $\rho < 1$: the counterfactual value of silence $R(0)$ is shrunk by $\rho$ ($0.02\times$), making the soul systematically over-prefer speaking relative to staying silent on `silent`-band events (and under-penalize/under-reward `silent` vs `note` by factor $1/\rho = 50\times$).
   - **Worse, `core-and-lab` currently introduces a polarity sign inversion:** In [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L12-L16) and [L31-L33](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L31-L33), `Bad` (`r = 0`) includes both `go` failures and `"a judged silence that was missed"`, and the weight update is `Δw_k ∝ (r − P)·s_k` (no `(a − P)` or speak-target flipping, and answered silences are not listed in `Good`):
     - On a `go` success ($a=1, r=1$): $(1-P)s_k > 0$ (correct: raises $P$).
     - On a `go` failure ($a=1, r=0$): $(0-P)s_k < 0$ (correct: lowers $P$).
     - On a `silent`/`note` miss ($a=0$, omission occurred so $P$ was **too low**): assigning $r=0$ in $(r-P)s_k$ produces $(0-P)s_k < 0$, which **decreases** $w_k$ and lowers $P$ even further!
   - **Clean unbiased fix (IPS-corrected policy-gradient or speak-target form):**
     Use the inverse-evaluated weight $q_i = \frac{e_i}{p_{\text{eval}}(a_i, P_i)}$ (with clipping cap $M$ for variance control, $\tilde{q}_i = e_i \cdot \min\!\big(M,\, p_{\text{eval}}(a_i, P_i)^{-1}\big)$). Since $\mathbb{E}[e_i / p_{\text{eval}}(a_i, P_i) \mid a_i, x_i] = 1$ and using a baseline $b_i \in [0, 1]$ (which preserves unbiasedness under exact propensity because $\mathbb{E}[(a_i - P_i) b_i] = 0$, note that weighted separately on $e_i/p_{\text{eval}}$ we also have $\mathbb{E}[(e_i/p_{\text{eval}})(a_i - P_i) b_i \mid x_i] = \mathbb{E}[(a_i - P_i)b_i \mid x_i] = 0$):
     $$
     g_i^{\text{IPS}} = \tilde{q}_i \, (a_i - P_i)\,(r_i - b_i),
     \qquad
     \Delta w_k \leftarrow \mathrm{clamp}\!\left(\alpha_{\text{learn}} \cdot \tilde{\eta}_a \cdot \frac{\sum_{i \in \mathcal{B}} \tilde{q}_i (a_i - P_i)(r_i - b_i)\, s_{k,i}}{Z_{\mathcal{B}}},\ \text{bounds}\right)
     $$
     where:
     - $r_i \in \{0, 1\}$ is **1 iff the decision's prediction succeeded** (`go` answered cleanly; or `note`/`silent` stayed silent without omission/repeat) and **0 iff it failed** (`go` unanswered/miss/repair/negative flag; or `note`/`silent` missed);
     - $\tilde{q}_i = e_i \cdot \min(M, 1/\rho)$ on `silent` ($a_i=0, P_i<\text{note\_band}$) and $\tilde{q}_i = 1$ on `go` and `note`;
     - $b_i \in [0, 1]$ is a causal running mean reward (or predicted outcome $\hat{R}(x_i, a_i)$ from a Doubly Robust baseline, start fixed `b_i = 0.5`);
     - $Z_{\mathcal{B}} = |\mathcal{B}|$ for clipped IPS, or $Z_{\mathcal{B}} = \sum_{i \in \mathcal{B}} \tilde{q}_i$ for **SNIPS** (Self-Normalized IPS) when aggregating batches/days.
     *(Algebraic check on $a_i=0$ with $b_i=0.5$:* if silence **missed** ($r_i=0$), $(a_i-P_i)(r_i-b_i) = (-P_i)(-0.5) = +0.5\, P_i > 0$, raising $w_k$; if silence **succeeded** ($r_i=1$), $(a_i-P_i)(r_i-b_i) = (-P_i)(+0.5) = -0.5\, P_i < 0$, lowering $w_k$.)*

2. **Variance Penalty vs Sampling Rate $\rho$ (Why raw $1/\rho$ explodes for tiny $\rho$):**
   For a Bernoulli sampling indicator $e_i \sim \mathrm{Bernoulli}(\rho)$ on `silent` events, the importance ratio $Q = e_i / \rho$ satisfies:
   $$
   \mathbb{E}[Q] = 1, \qquad \mathrm{Var}(Q) = \frac{1-\rho}{\rho}, \qquad \mathrm{Var}\!\left(g_i^{\text{IPS}}\right) \ge \frac{1}{\rho}\,\mathbb{E}\big[(a_i - P_i)^2 (r_i - b_i)^2\big] - \big(\mathbb{E}[(a_i-P_i)(r_i-b_i)]\big)^2.
   $$
   - At $\rho = 0.02$, raw importance weights multiply `silent` update values by **$50\times$** and inflate sampling variance of `silent` events by $(1-0.02)/0.02 = \mathbf{49\times}$, risking hitting the `5%/day` step clamp from single outlier misses.
   - At $\rho = 0.05$, $(1-\rho)/\rho = 19$; at $\rho = 0.10$, $(1-\rho)/\rho = 9$ (still $9\times$ variance, though $5\times$ cheaper to evaluate than $\rho=0.50$).
   - **Literature settlement on stabilization (without knowing content):**
     - **Truncated / Clipped IPS** (`Ionides 2008` `[PR]`, *JCGS* `10.1198/106186008X320456`; `Bottou et al. 2013` `[PR]`, *JMLR* 14:3207–3260; `Swaminathan & Joachims 2015` CRM `[PR]`, `arXiv:1502.02362`): capping importance weights at $M$ ($w_i' = \min(w_i, M)$) bounds variance by at most $M^2$ while driving bias $O(1)$ to $0$ as $M \to 1/\rho$ (or asymptotically as $n \to \infty$ when $\tau_n$ grows). In CRM, adding an empirical-variance regularizer $\sqrt{\widehat{\mathrm{Var}} / n}$ prevents policies from chasing rare high-weight samples. Note: at fixed $\rho = 0.02$ and constant cap $M = 10$, silent omissions still receive $10\times$ larger weight than `note` (capped rather than $50\times$, trading a bounded bias $(1 - \rho M) = 0.8$ downward shrinkage on pure `silent` weight for $25\times$ lower variance than unclipped $50\times$). Raising $\rho$ to `0.05` with $M = 20$ eliminates clipping bias (`0.05 × 20 = 1`) at half the maximum weight (`20` vs `50`) whenever evaluation budget permits.
     - **Self-Normalized IPS (SNIPS)** (`Joachims & Swaminathan 2015` `[PR]`, NeurIPS 28, `https://papers.nips.cc/paper_files/paper/2015/hash/39027dfad5138c9ca0c474d71db915c3-Abstract.html`): dividing by $\sum_i \tilde{q}_i$ eliminates propensity overfitting (where a policy pushes probability onto under-sampled regimes solely to exploit positive/negative constant shifts) and ensures equivariance when reward baseline shifts occur.
     - **Doubly Robust (DR) Control Variates** (`Dudík, Langford & Li 2011` `[PR]`, *ICML 2011*, `arXiv:1103.4601`): replacing $r_i - b_i$ on evaluated samples by $(r_i - \hat{R}(x_i, a_i))$ plus $\hat{R}(x_i, a_i)$ reduces variance strictly whenever $\hat{R}$ predicts outcome mean better than a constant. Even without an ML predictor, substituting the place's empirical omission rate $b_0$ (`running average of resolved outcomes in place`) is a causal DR zero-order control variate that is zero-bias and lowers variance inside the core.
     - **Production Bandits Reference Rate** (`Agarwal et al. 2016/2017` `[PP]`, MSN Decision Service `arXiv:1606.03966`): In high-nonstationarity personalized news at MSN, default `EpsilonGreedy` used $\epsilon = 33\%$ across tens of millions of users, deployed new models every 5 minutes, and measured **1.2×–8.7×** metric distortions (including `3.0×` at a 10% logging override error and `1.3×–1.7×` delay bias) when propensities were mislogged or outcomes delayed. This directly reinforces two MonoAgens mechanisms: (i) logging exact `p_eval` (`P`, `note_band`, `ρ`) deterministically inside every decision event so logged propensities can never drift, and (ii) never opening/updating a decision until the patience/silence expiry closes (preventing delay bias).

**Established / Inferred / Open.**
- *Established:* (a) Without logging and dividing by evaluation propensity (`1/ρ`), sampled `silent` outcomes misweight counterfactual losses vs unsampled `note` outcomes (`Ionides 2008` `[PR]`, `Bottou et al. 2013` `[PR]`, `Agarwal et al. 2016` `[PP]`); (b) raw IPS suffers $O(1/\rho)$ variance inflation that clipped IPS (`min(M, 1/ρ)`, `Ionides 2008`, `Swaminathan & Joachims 2015a` `[PR]`), SNIPS (`Joachims & Swaminathan 2015b` `[PR]`), and Doubly Robust baselines (`Dudík et al. 2011` `[PR]`) control; (c) MonoAgens' current `r = 0` polarity for `"judged silence that was missed"` in `bounded-learning/spec.md` invertedly pushes gate weights down instead of up.
- *Inferred:* Replacing MonoAgens' current delta rule with clipped-IPS/SNIPS REINFORCE $(a - P)(r - b)\min(M, p_{\text{eval}}^{-1})$ fixes both sign inversion and propensity bias without adding LLM calls or content access; using place-level running omission baseline $b$ lowers variance cleanly within opaque-ID bounds.
- *Open:* Optimal $(\rho, M)$ pair trading classifier/evaluation cost (remembering re-raised silent misses trigger a `classify` LLM call in `design.md` Decision 8) against omission recall across sparse vs dense places and physical/game bodies.

##### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `rho` ($\rho$) | Seeded fraction of `silent` decisions checked for omission | `0.02` (`0.05` in quiet places) | `0.005–0.15` | Tune in Lab (balancing variance $1/\rho$ vs `classify` cost) |
| `ips_clip` ($M$) | Upper clip bound on $1/p_{\text{eval}}$ (`Ionides 2008`, `Swaminathan & Joachims 2015` `[PR]`) | `10.0` (raise to `20.0` if $\rho \ge 0.05$) | `5.0–25.0` | Tune in Lab |
| `reward_baseline` ($b_0$) | Control-variate baseline subtracted from outcome $r_i \in \{0,1\}$ (`Dudík et al. 2011` `[PR]`) | `0.5` (or place EMA) | `0.2–0.8` | Tune in Lab |

##### Neutral value
$\rho = 0$ (no `silent` decisions are sampled; only `go` and `note` predictions feed learning).

##### Claim
Sampling `silent` decisions at $\rho > 0$ with clipped IPS weighting $\min(M, 1/\rho)$ and corrected $(a - P)$ polarity recovers from false-silence equilibria (where unaddressed low-salience events never wake reasoning) faster and with lower calibration error than $\rho = 0$ (`note`-only sampling) or unweighted $\rho > 0$, without destabilizing `w_k` into the `5%/day` freeze guard.

##### Lab Experiment
- **Body generality:** Implementable across chat and non-chat bodies (in device/game bodies, `"silence missed"` corresponds to a follow-on exogenous distress/retry percept arriving within the body's expectation window without an intervening response by another actor). Note `design.md` L249's `"sender posts again"` trigger is chat-worded and should be paired with a generic `"same-source repeat or unresolved state flag"` abstraction for non-chat bodies.
- **Scenario:** Multi-week virtual-time scenario (e.g., 30 days, ≥5 seeds per condition per [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L90-L95)) initialized in a **conservative silent bias** (high $\theta_0$ or low `w_info`), where true-need `silent`-band queries ($P < \text{note\_band}$) arrive alongside ignorable background chatter.
- **Ablations (5 arms × ≥5 seeds):**
  1. Neutral $\rho = 0$ (`go` + `note` only);
  2. Current `core-and-lab` rule (`r = 0` sign bug, unweighted);
  3. Corrected polarity $(a - P)$, **unweighted** (`M = 1` effective, no $1/\rho$);
  4. Corrected polarity + **unclipped** $1/\rho$ at $\rho \in \{0.02, 0.05\}$;
  5. Corrected polarity + **clipped IPS / SNIPS** across grid $\rho \in \{0.01, 0.02, 0.05, 0.10\} \times M \in \{5, 10, 20\}$.
- **Metrics (`lab/spec.md` L131–145):** `when-to-speak precision and recall`, `missed replies`, `false speaks`, days-to-recover recall on initially silent queries, `w_k` variance across seeds & frequency of `freeze and report` triggers, and `classify` model-call cost per event.

##### Sources
- `[PP]` **Agarwal, A., Bird, S., Cozowicz, M., Hoang, L., Langford, J., Lee, S., Li, J., Melamed, D., Oshri, G., Ribas, O., Sen, S., & Slivkins, A. (2016).** "Making Contextual Decisions with Low Technical Debt." `arXiv:1606.03966v2`. URL: [`https://arxiv.org/abs/1606.03966`](https://arxiv.org/abs/1606.03966).
- `[PR]` **Ionides, E. L. (2008).** "Truncated Importance Sampling." *Journal of Computational and Graphical Statistics*, 17(2):295–311. DOI: [`10.1198/106186008X320456`](https://doi.org/10.1198/106186008X320456).
- `[PR]` **Bottou, L., Peters, J., Quiñonero-Candela, J., Charles, D. X., Chickering, D. M., Portugaly, E., Ray, D., Simard, P., & Snelson, E. (2013).** "Counterfactual Reasoning and Learning Systems: The Example of Computational Advertising." *Journal of Machine Learning Research*, 14(101):3207–3260. URL: [`https://www.jmlr.org/papers/v14/bottou13a.html`](https://www.jmlr.org/papers/v14/bottou13a.html).
- `[PR]` **Swaminathan, A., & Joachims, T. (2015a).** "Counterfactual Risk Minimization: Learning from Logged Bandit Feedback." *ICML 2015*, PMLR 37 (`arXiv:1502.02362`, URL: [`https://proceedings.mlr.press/v37/swaminathan15.html`](https://proceedings.mlr.press/v37/swaminathan15.html); extended in *JMLR* 16: [`https://www.jmlr.org/papers/v16/swaminathan15a.html`](https://www.jmlr.org/papers/v16/swaminathan15a.html)).
- `[PR]` **Joachims, T., & Swaminathan, A. (2015b).** "The Self-Normalized Estimator for Counterfactual Learning." *Advances in Neural Information Processing Systems (NeurIPS)* 28. URL: [`https://papers.nips.cc/paper_files/paper/2015/hash/39027dfad5138c9ca0c474d71db915c3-Abstract.html`](https://papers.nips.cc/paper_files/paper/2015/hash/39027dfad5138c9ca0c474d71db915c3-Abstract.html). *(Note: not `arXiv:1505.07549`, which is an unrelated paper.)*
- `[PR]` **Dudík, M., Langford, J., & Li, L. (2011).** "Doubly Robust Policy Evaluation and Learning." *ICML 2011* (`arXiv:1103.4601`). URL: [`https://arxiv.org/abs/1103.4601`](https://arxiv.org/abs/1103.4601).

---

#### R-20 Should Jøsang's forgetting/longevity factor $\lambda$ for reputation evidence differ per domain?

**Verdict:** `partly settled` *(Brief-2 status: `partly answered` — the conceptual principle that $\lambda$ should reflect domain/entity rate of change is explicitly established in trust literature; universal domain-specific constants do not exist and must be parameterized by opaque domain volatility or empirical change rate).*

##### Evidence & Rule

**Answer.** Yes: reputation and trust modeling literature explicitly establishes that forgetting/longevity factor $\lambda \in [0, 1]$ governs the tradeoff between convergence stability in static settings and adaptation speed under behavioral shift, and **should vary with the expected volatility ("rapidity of change") of the domain or task**. Because MonoAgens' core only sees opaque `domain_id` values (`K` sleep-computed vector centroids or gateway/reasoning-supplied IDs) and never parses domain semantics, the core must **not** hardcode semantic domain labels (such as `"coding"` vs `"scheduling"`). Instead, per-[design.md §10](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L300-L306), each `(actor, domain_id)` reliability cell starts with uniform longevity $\lambda_d = \lambda_{\text{base}}$ (start `0.90` per period) and optionally accepts either a **gateway-declared prior $\lambda_{d,0}$** (for externally supplied opaque `domain_id`s, e.g., ephemeral telemetry vs stable capability) or an **empirical hazard-modulated forgetting rate $\lambda_d$** driven solely by recent within-domain outcome flip frequency (high regime-shift rate $\implies$ smaller $\lambda_d$, faster forgetting; stable history $\implies$ higher $\lambda_d \to 0.99$).

**Formulas & Symbols.** Keeping [design.md §10](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L300-L305) intact: for `(actor a, domain d)` Beta state $(\alpha_{a,d}, \beta_{a,d})$ relaxing toward prior $(\alpha_{a,d,0}, \beta_{a,d,0})$ over $\Delta$ elapsed periods:

$$
\alpha_{a,d} \leftarrow \alpha_{a,d,0} + (\alpha_{a,d} - \alpha_{a,d,0})\,\lambda_d^{\Delta},
\qquad
\beta_{a,d} \leftarrow \beta_{a,d,0} + (\beta_{a,d} - \beta_{a,d,0})\,\lambda_d^{\Delta}.
$$

Effective evidence window in periods is $N_{\text{eff}}(d) = \frac{1}{1 - \lambda_d}$ (e.g., $\lambda_d = 0.50 \Rightarrow 2$ periods; $\lambda_d = 0.90 \Rightarrow 10$ periods; $\lambda_d = 0.98 \Rightarrow 50$ periods).

*(Optional adaptive extension keyed strictly by opaque `domain_id` $d$, no content inspection):* track an EWMA regime-shift indicator $v_d \in [0, 1]$ when outcome surprises $|r_{t} - \mathbb{E}[p_{a,d}]|$ exceed threshold across resolved predictions in domain $d$, mapping cleanly inside bounds:
$$
\lambda_d = \mathrm{clamp}\big(\lambda_{\max} - (\lambda_{\max} - \lambda_{\min})\, v_d,\ \lambda_{\min},\ \lambda_{\max}\big),
$$
with neutral behavior when $v_d \equiv 0$ or adaptive toggle disabled ($\lambda_d = \lambda_{\text{base}}$).

**Verified Literature Findings:**
1. **Foundational Beta Reputation rule:** In Jøsang & Ismail (2002) `[PR]` (*15th Bled Electronic Commerce Conference*, `https://mn.uio.no/ifi/english/people/aca/josang/publications/ji2002-bled.pdf`, `https://aisel.aisnet.org/bled2002/41/`, Section 3.5 / Eqs. 12–14), the forgetting factor $\lambda \in [0, 1]$ is explicitly introduced so it ***"can be adjusted according to the expected rapidity of change in the observed entity"***; $\lambda = 0$ retains only the latest observation, while $\lambda = 1$ never forgets. Section 3.5 notes qualitatively that lower $\lambda$ adapts faster to change at the cost of noisier, less extreme converged estimates, illustrating representative values $\lambda \in \{0.0, 0.5, 0.7, 0.9, 1.0\}$. Jøsang, Ismail & Boyd (2007) `[PR]` (*Decision Support Systems*, `10.1016/j.dss.2005.05.019`) and Jøsang (2016) `[PR]` (*Subjective Logic*, `10.1007/978-3-319-42337-1`) reaffirm domain/context-dependent base rates and aging.
2. **Hierarchical / Multi-context Trust:**
   - **TRAVOS** (Teacy et al. 2006 `[PR]`, *AAMAS* `10.1007/s10458-006-5952-x`) notes that Beta probability density estimates must remain context-bounded when agent capabilities vary across interaction types.
   - **HABIT** (Teacy et al. 2012 `[PR]`, *Artificial Intelligence* `10.1016/j.artint.2012.09.001`) models multi-context agent competence hierarchically across related tasks rather than pooling all observations with one global decay rate.
   - **RATEWeb** (Malik & Bouguettaya 2009 `[PR]`, *VLDB J.* `10.1007/s00778-009-0138-1`) differentiates reputation stability across contexts and weighs recent vs historical feedback by temporal sensitivity.
   - **Rehak et al. (2006)** `[PR]` (*IEEE/WIC/ACM IAT*, `10.1109/iat.2006.107`) represent multi-agent trust per context rather than globally when operational situations change at different speeds.
3. **Adaptive Forgetting Against Change & Manipulation:**
   - **Sun et al. (2006a, 2006b)** `[PR]` (*IEEE JSAC* `10.1109/jsac.2005.861389`; *IEEE CISS* `10.1109/ciss.2006.286695`) analyze oscillation and trust-modulation attacks ("on-off" behavior) showing that static long-memory ($\lambda \approx 1$) is sluggish to detect sudden degradation, whereas short memory forgets stable track records—motivating anomaly-reactive / multi-timescale forgetting.
   - **Dagdanov, Andrejević, Liu & Lin (2025)** `[PR]` (*IEEE RA-L* `10.1109/lra.2025.3585653`, `arXiv:2411.01866`) apply Beta reputation at fine-grained operational timescales in human-robot collaboration, coupling history dependence with explicit aging and asymmetric MLE-fitted evidence weights ($\omega^{\text{s}} = 3.7897 < \omega^{\text{f}} = 4.5390$, demonstrating faster trust drops on adverse operational failures than recovery on successes).

**Established / Inferred / Open.**
- *Established:* (a) Trust and Beta reputation literature establishes that $\lambda$ depends on entity/domain volatility (`Jøsang & Ismail 2002` `[PR]`), (b) trust generalizes across contexts hierarchically or context-specifically (`HABIT 2012` `[PR]`, `Rehak et al. 2006` `[PR]`), and (c) operational human-robot / dynamic regimes benefit from fine-grained aging and asymmetric failure weighting (`Dagdanov et al. 2025` `[PR]`).
- *Inferred:* Because MonoAgens core only handles opaque `domain_id`s, supporting an optional host/prior-supplied $\lambda_{d,0}$ plus a domain-volatility estimator $v_d$ gives domain-differentiated forgetting without violating `ARCHITECTURE.md` invariants. For MonoAgens' two primary targets: *fast operational competence/availability* vs *stable epistemic reliability*, uniform $\lambda = 0.90$ is a single-timescale compromise.
- *Open:* Whether K-means vector-centroid domains during sleep capture volatility differences cleanly enough to outperform uniform $\lambda$, and whether separate asymmetric decay/update weights ($\omega^{\text{f}} > \omega^{\text{s}}$ per Dagdanov et al.) beat symmetric Beta updates in MonoAgens.

##### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `lambda_base` ($\lambda_{\text{base}}$) | Default per-period Beta evidence longevity | `0.90` | `0.50–0.99` | Round 1 R-20 (`Jøsang & Ismail 2002` `[PR]`) |
| `lambda_min`, `lambda_max` | Bounds for host-supplied or volatility-adapted $\lambda_d$ | `0.70`, `0.98` | `0.50–0.99` | Tune in Lab |
| `lambda_period` | Elapsed-time unit $\Delta$ for longevity decay | `1 day` | `1 hour – 30 days` | Round 1 R-20; tune per body cadence in Lab |

##### Neutral value
Uniform $\lambda_d \equiv \lambda_{\text{base}}$ (no per-domain differentiation), or $\lambda = 1.0$ (no forgetting at all; mechanism neutral).

##### Claim
Allowing volatile operational domains (high outcome flip rate / supplied fast-domain ID) to forget faster ($\lambda_d < \lambda_{\text{base}}$) while keeping stable epistemic domains slow-forgetting ($\lambda_d > \lambda_{\text{base}}$) simultaneously reduces time-to-detect sudden degradation in volatile domains and reduces false distrust spikes in stable competence domains.

##### Lab Experiment
- **Body generality:** Fully generic across chat, device/sensor, and game bodies.
- **Scenario:** Dual-domain multi-week scenario (30 days, ≥5 seeds) featuring two opaque domain IDs for the same source/device: Domain A (volatile operational reliability—alternating good and bad weeks mimicking outage/shift regimes) and Domain B (stable epistemic competence with 90% accuracy + rare single errors).
- **Ablations:** (1) $\lambda = 1.0$ (no forgetting); (2) uniform $\lambda \in \{0.70, 0.90, 0.98\}$; (3) per-domain $\lambda_A \neq \lambda_B$ (supplied tier vs hazard-adapted $\lambda_d$).
- **Metrics:** Detection latency (events/hours until Beta mean crosses threshold after Domain A flips from reliable to faulty), regret on Domain A, and calibration Brier error / over-reaction rate on Domain B after isolated misses (reported with seed confidence intervals per `lab/spec.md` L92–94).

##### Sources
- `[PR]` **Jøsang, A., & Ismail, R. (2002).** "The Beta Reputation System." *15th Bled Electronic Commerce Conference*. PDF: [`ji2002-bled.pdf`](https://mn.uio.no/ifi/english/people/aca/josang/publications/ji2002-bled.pdf) / [`AISeL`](https://aisel.aisnet.org/bled2002/41/). *(Note: PDF lists Jøsang first; AISeL metadata lists Ismail first.)*
- `[PR]` **Jøsang, A., Ismail, R., & Boyd, C. (2007).** "A survey of trust and reputation systems for online service provision." *Decision Support Systems*, 43(2):618–644. DOI: [`10.1016/j.dss.2005.05.019`](https://doi.org/10.1016/j.dss.2005.05.019).
- `[PR]` **Jøsang, A. (2016).** *Subjective Logic*. Springer. DOI: [`10.1007/978-3-319-42337-1`](https://doi.org/10.1007/978-3-319-42337-1).
- `[PR]` **Teacy, W. T. L., Patel, J., Jennings, N. R., & Luck, M. (2006).** "TRAVOS: Trust and Reputation in the Context of Inaccurate Information Sources." *Autonomous Agents and Multi-Agent Systems*, 12(2):183–198. DOI: [`10.1007/s10458-006-5952-x`](https://doi.org/10.1007/s10458-006-5952-x).
- `[PR]` **Teacy, W. T. L., Luck, M., Rogers, A., & Jennings, N. R. (2012).** "An efficient and versatile approach to trust and reputation using hierarchical Bayesian modelling." *Artificial Intelligence*, 193:149–185. DOI: [`10.1016/j.artint.2012.09.001`](https://doi.org/10.1016/j.artint.2012.09.001).
- `[PR]` **Malik, Z., & Bouguettaya, A. (2009).** "RATEWeb: Reputation Assessment for Trust Establishment among Web services." *The VLDB Journal*, 18(4):885–911. DOI: [`10.1007/s00778-009-0138-1`](https://doi.org/10.1007/s00778-009-0138-1).
- `[PR]` **Sun, Y. L., Yu, W., Han, Z., & Liu, K. J. R. (2006a).** "Information theoretic framework of trust modeling and evaluation for ad hoc networks." *IEEE JSAC*, 24(2):305–317. DOI: [`10.1109/jsac.2005.861389`](https://doi.org/10.1109/jsac.2005.861389).
- `[PR]` **Sun, Y., Han, Z., Yu, W., & Liu, K. R. (2006b).** "Attacks on Trust Evaluation in Distributed Networks." *IEEE CISS 2006*, pp. 1461–1466. DOI: [`10.1109/ciss.2006.286695`](https://doi.org/10.1109/ciss.2006.286695).
- `[PR]` **Rehak, M., Gregor, M., Pechoucek, M., & Bradshaw, J. (2006).** "Representing Context for Multiagent Trust Modeling." *IEEE/WIC/ACM IAT 2006*, pp. 737–746. DOI: [`10.1109/iat.2006.107`](https://doi.org/10.1109/iat.2006.107).
- `[PR]` **Dagdanov, R., Andrejević, M., Liu, D., & Lin, C.-T. (2025).** "Improving Trust Estimation in Human-Robot Collaboration Using Beta Reputation at Fine-Grained Timescales." *IEEE Robotics and Automation Letters*, 10(8):8562–8569 (`arXiv:2411.01866`). DOI: [`10.1109/lra.2025.3585653`](https://doi.org/10.1109/lra.2025.3585653).

---

#### R-24 Quantitative evidence on how much overlap of people, place, and time makes two episodes "the same" (`J`)?

**Verdict:** `not settled by the literature` *(Brief-2 status: `not settled by the literature` — hippocampal pattern separation and Event Horizon cognition explain non-linear separation vs completion dynamics and show that spatial/actor/temporal shifts break episodes, but provide no quantitative Jaccard cutoff or overlap percentage $J$).*

##### Evidence & Rule

**Answer.** Neither neuroscience (hippocampal dentate gyrus / CA3 pattern separation) nor cognitive event segmentation establishes a numerical overlap cutoff $J$ (nor an exact percentage of shared people, space, and time) defining when two episodes merge. On the contrary, `Stark et al. (2019)` `[PR]` explicitly show that behavioral similarity scales in the Mnemonic Similarity Task (Lure Bins 1–5) are calibrated *empirically* from population false-alarm rates per stimulus set rather than from an invariant similarity threshold. Consequently, [design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L317-L320) and [sleep-consolidation/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/sleep-consolidation/spec.md) already make the right structural choice (`place_a == place_b`, $\mathrm{Jaccard}(P_a, P_b) \ge J$, overlapping/adjacent time within gap threshold, vector similarity alone never merges); $J$ should remain flagged as a **start value** (`0.80`, range `0.50–1.00`) to be tuned against Lab fan-effect / retrieval precision vs consolidation bloat.

**Formulas & Symbols** (unchanged structural rule from `design.md` Decision 11):
Episodes $E_1 = (\text{place}_1, P_1, [t_{1,\text{start}}, t_{1,\text{end}}])$ and $E_2 = (\text{place}_2, P_2, [t_{2,\text{start}}, t_{2,\text{end}}])$ are eligible to merge during sleep iff:

$$
\text{place}_1 = \text{place}_2
\quad\wedge\quad
\frac{|P_1 \cap P_2|}{|P_1 \cup P_2|} \ge J
\quad\wedge\quad
\max(t_{1,\text{start}}, t_{2,\text{start}}) - \min(t_{1,\text{end}}, t_{2,\text{end}}) \le \Delta_{\text{gap}}(\text{place}_1),
$$

with $J \in [0.50, 1.00]$ (start `0.80`) and where $\Delta_{\text{gap}}(\text{place})$ uses the place's 95th-percentile inter-event gap (`A-07` cold-start fallback when sparse). Note: for singleton/dyadic or fixed-cast sensor/device places where $P_1 = P_2$ is constant across days, the time-gap criterion dominates; for shifting group chats (e.g., 3–6 speakers where one leaves/joins, giving Jaccard `0.67`–`0.80`), $J = 0.80$ requires ≥4 of 5 actors in common while $J = 0.60$ tolerates 1 churned speaker in a 3–4 actor group.

**Verified Literature Findings:**
1. **Hippocampal Pattern Separation vs Completion:** **Yassa & Stark (2011)** `[PR]` (*Trends in Neurosciences*, `10.1016/j.tins.2011.06.006`) review Marr-style DG orthogonalization and CA3 auto-associative attractor dynamics, showing a qualitative sigmoidal transfer curve from pattern completion to separation (DG diverges sharply for small input differences; CA3 exhibits a completion plateau followed by steep separation; CA1 responds linearly). Critically, the paper plots schematic/qualitative input-similarity vs output-similarity curves without deriving any universal numerical threshold.
2. **Task-Relative Calibration in MST:** **Stark, Kirwan & Stark (2019)** `[PR]` (*Trends in Cognitive Sciences*, `10.1016/j.tics.2019.08.003`) explicitly describe how MST lure similarity bins (Bins 1–5) are parameterized *empirically* from human "Old" false-alarm rates per stimulus family rather than by a cross-domain mathematical constant—proving no single threshold transfers automatically from objects/scenes to discrete social-event sets.
3. **Cognitive Event Boundaries & Fan-Effect Interference:**
   - **Zacks, Speer, Swallow, Braver & Reynolds (2007)** `[PR]` (*Psychological Bulletin*, `10.1037/0033-2909.133.2.273`) and **Zacks, Speer & Reynolds (2009)** `[PR]` (*JEP:G*, `10.1037/a0015305`) establish Event Segmentation Theory: boundaries arise when prediction errors spike upon shifts in time, spatial location, characters, objects, causes, or goals.
   - **Horner, Bisby, Wang, Bogus & Burgess (2016)** `[PR]` (*Cognition*, `10.1016/j.cognition.2016.05.013`) show experimentally that crossing a spatial boundary (walking through a doorway) significantly disrupts cross-boundary temporal-order retrieval compared with staying within the same room ($\eta_p^2 = 0.25$)—supporting MonoAgens' hard `place_1 == place_2` barrier.
   - **Ezzyat & Davachi (2011)** `[PR]` (*Psychological Science*, `10.1177/0956797610393742`) show that even adjacent sentences separated by a brief event-shift cue show weakened cross-boundary binding compared to continuous episodes.
   - **Robin & Moscovitch (2014)** `[PR]` (*JEP:LMC*, `10.1037/a0034886`) document how spatial contextual familiarity shapes episodic memory binding vs schematic retrieval.
   - **Radvansky & Zacks (2017)** `[PR]` (*Curr. Opin. Behav. Sci.*, `10.1016/j.cobeha.2017.08.006`) synthesize the Event Horizon Model: boundaries segregate distinct event models, whereas maintaining multiple distinct, non-integrated event models that share common elements degrades retrieval via **fan-effect competitive interference**—directly framing the exact tradeoff that $J$ must balance in Lab (under-merging $\Rightarrow$ fan-effect clutter across fragment models; over-merging $\Rightarrow$ context contamination across non-shared participants).

**Established / Inferred / Open.**
- *Established:* Spatial, actor, and temporal discontinuities fragment episodes (`Zacks et al. 2007/2009`, `Horner et al. 2016`, `Ezzyat & Davachi 2011` `[PR]`), over-splitting creates fan-effect retrieval interference (`Radvansky & Zacks 2017` `[PR]`), and similarity thresholds are empirically calibrated per stimulus/task (`Stark et al. 2019` `[PR]`).
- *Inferred:* No single literature number exists for participant Jaccard $J$. Because missing one person in a 4-person conversation drops Jaccard to $3/4 = 0.75$ (below `0.80`), $J = 0.80$ effectively requires identical casts for groups of ≤4 while allowing 1 speaker join/leave at size ≥5 (`4/5 = 0.80`); `0.60–0.80` is therefore the crucial transition band to test in Lab.
- *Open:* Empirical optimal $J$ across small groups (2–5 actors) vs larger channels (>5 actors) and whether size-normalized overlap (Sørensen–Dice or asymmetric containment $\ge J$ for brief drop-ins) beats unweighted Jaccard.

##### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `merge_jaccard_min` ($J$) | Minimum participant-set Jaccard overlap for episode merge (`design.md` §11) | `0.80` | `0.50–1.00` | Tune in Lab (no literature constant) |
| `delta_gap` ($\Delta_{\text{gap}}$) | Max temporal separation between intervals in the same place | Place 95th-percentile gap | Place-dependent (`A-07` fallback) | `design.md` §11; tune in Lab |

##### Neutral value
$J = 1.01$ (or merge disabled: never merge separate closed episodes during sleep; mechanism neutral).

##### Claim
Merging close episodes sharing place, time proximity, and sufficient participant overlap ($J \in [0.50, 1.00]$) reduces episodic fan-effect fragment clutter during `memory.recall` (higher recall hits / less wasted prompt budget) without causing cross-participant confusion (no false-belief or privacy leakage to non-participants).

##### Lab Experiment
- **Body generality:** Generic across multi-actor bodies (chat rooms with joining/leaving users; game worlds or shared rooms with moving entities). In single-source bodies ($|P| = 1$), Jaccard is trivially `1.0`, so only place + time-gap decide merges.
- **Scenario:** Multi-episode scripted stream across 7–30 virtual days in a 4-to-5-actor room with natural brief drop-ins/drop-outs (testing adjacent episodes with $J \in \{0.50, 0.67, 0.75, 0.80, 1.0\}$) plus planted privacy/audience-restricted facts and delayed recall probes.
- **Ablation:** Sweep $J \in \{\text{off/neutral } (>1.0),\, 1.0,\, 0.80,\, 0.67,\, 0.50\}$ across ≥5 seeds.
- **Metrics (`lab/spec.md` L141–145):** `recall hits at gaps of one, seven and thirty days`, episode count / context token overhead, `number of items from a narrower audience placed in a wider context` (audience leakage / cross-cast contamination, must stay 0), and `correct answers on scripted false-belief questions`.

##### Sources
- `[PR]` **Yassa, M. A., & Stark, C. E. L. (2011).** "Pattern separation in the hippocampus." *Trends in Neurosciences*, 34(10):515–525. DOI: [`10.1016/j.tins.2011.06.006`](https://doi.org/10.1016/j.tins.2011.06.006).
- `[PR]` **Stark, S. M., Kirwan, C. B., & Stark, C. E. L. (2019).** "Mnemonic Similarity Task: A Tool for Assessing Hippocampal Integrity." *Trends in Cognitive Sciences*, 23(11):938–951. DOI: [`10.1016/j.tics.2019.08.003`](https://doi.org/10.1016/j.tics.2019.08.003).
- `[PR]` **Radvansky, G. A., & Zacks, J. M. (2017).** "Event boundaries in memory and cognition." *Current Opinion in Behavioral Sciences*, 17:133–140. DOI: [`10.1016/j.cobeha.2017.08.006`](https://doi.org/10.1016/j.cobeha.2017.08.006).
- `[PR]` **Horner, A. J., Bisby, J. A., Wang, A., Bogus, K., & Burgess, N. (2016).** "The role of spatial boundaries in shaping long-term event representations." *Cognition*, 154:151–164. DOI: [`10.1016/j.cognition.2016.05.013`](https://doi.org/10.1016/j.cognition.2016.05.013).
- `[PR]` **Zacks, J. M., Speer, N. K., Swallow, K. M., Braver, T. S., & Reynolds, J. R. (2007).** "Event perception: A mind-brain perspective." *Psychological Bulletin*, 133(2):273–293. DOI: [`10.1037/0033-2909.133.2.273`](https://doi.org/10.1037/0033-2909.133.2.273).
- `[PR]` **Zacks, J. M., Speer, N. K., & Reynolds, J. R. (2009).** "Segmentation in reading and film comprehension." *Journal of Experimental Psychology: General*, 138(2):307–327. DOI: [`10.1037/a0015305`](https://doi.org/10.1037/a0015305).
- `[PR]` **Ezzyat, Y., & Davachi, L. (2011).** "What Constitutes an Episode in Episodic Memory?" *Psychological Science*, 22(2):243–252. DOI: [`10.1177/0956797610393742`](https://doi.org/10.1177/0956797610393742).
- `[PR]` **Robin, J., & Moscovitch, M. (2014).** "The effects of spatial contextual familiarity on remembered scenes, episodic memories, and imagined future events." *JEP:LMC*, 40(2):459–475. DOI: [`10.1037/a0034886`](https://doi.org/10.1037/a0034886).

---

#### R-25 How many items per person (`ledger_items`) should a theory-of-mind card hold before a model stops using it well?

**Verdict:** `partly settled` *(Brief-2 status: `partly answered` — 2023–2026 ToM and entity-tracking studies consistently prove two strong qualitative rules: (1) filtering cards to target-relevant mental states & removing irrelevant/non-target chatter yields large gains, and (2) LLM accuracy degrades sharply across >4 sequential state changes/updates and long middle spans; however, no study sweeps exact per-person `ledger_items ∈ [5, 30]` cap sizes in multi-card prompts, so the exact integer cap must be tuned in Lab).*

##### Evidence & Rule

**Answer.** Keep second-order nesting strictly bounded at `max_belief_order = 2` (and in core per [design.md §11](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L331-L333), only on the self-card for the soul's own communicated claims), cap per-field entries tightly at `ledger_items` start **`8`–`10`** (search range **`4–15`**, tightened below Round 1's upper bound of `30` because state-tracking failures emerge around 4–7 scenario shifts / ~5+ updates), and prioritize **active, ungrounded/unreconciled discrepancies** over raw chronological bloat so superseded beliefs do not crowd active cards. When cards exceed `ledger_items`, evict resolved/grounded facts first (preserving unresolved requests and active false-belief candidates) and retain superseded state only in long-term episodic storage rather than inline ToM cards.

**Formulas & Symbols.** For person $p$ present in place $L$, card $\mathcal{C}_p$ maintains four bounded lists matching [perception-ledger/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/perception-ledger/spec.md):
- $W_p$ (`witnessed`), $T_p$ (`told` + grounding status), $Q_p$ (`open_requests`), $F_p$ (`candidate_false_beliefs`), each bounded by:

$$
|X_p| \le \text{ledger\_items}, \qquad
\text{order} \le \text{max\_belief\_order} = 2 \text{ (self-claims only)}, \qquad
\text{evict key} = \big(\text{is\_unresolved},\ \text{salience/recency}\big).
$$

Total injected ledger lines scale as $O(N_{\text{present}} \cdot 4 \cdot \text{ledger\_items})$: with 4 people present and `ledger_items = 8` vs `30`, total card items differ between `≤ 128` items and `≤ 480` items—directly interacting with inhibitory-control dilution demonstrated in `PercepToM` and temporal middle-span decay in `DynToM`.

**Verified Literature Findings:**
1. **Perspective Isolation vs Over-Filtering (`SymbolicToM`, `PercepToM`, `FANToM`):**
   - **SymbolicToM** (Sclar et al. 2023 `[PR]`, *ACL 2023*, `10.18653/v1/2023.acl-long.780`, `arXiv:2306.00924`): builds explicit per-character belief graphs $B_{p_1,\dots,p_k}$ ($k \le m = 2$ for main benchmarks, and shows $k=2$ graph pruning even boosts third-order questions in modified $D_2$ stories). Extracting *only* the relevant character's belief graph outperforms unfiltered prompts across zero-shot models, though extreme single-entity filtering caused a room-vs-container answer bias in `Flan-T5-XXL` (showing cards should not strip essential spatial/grounding anchors).
   - **PercepToM** (Jung et al. 2024 `[PR]`, *EMNLP 2024*, `10.18653/v1/2024.emnlp-main.1105`, `arXiv:2407.06004`): finds LLMs infer local perceptions well (`0.781` on `Percept-ToMi`, `0.926` on `Percept-FANToM`), yet Table 2 (`PercepToM+Oracle` vs `perception-to-belief inference`) demonstrates that leaving **irrelevant context** (other actors' perceptions and unperceived events) in the prompt significantly hurts belief inference **even when ground-truth perceptions are already given**—explicitly linking this to human *inhibitory control* limits.
   - **FANToM** (Kim et al. 2023 `[PR]`, *EMNLP 2023*, `10.18653/v1/2023.emnlp-main.890`, `arXiv:2310.15421`): up to 5 interacting speakers; demonstrates >70% human–model gaps on full multi-party conversations and worse accuracy in full vs shortened contexts.
2. **Dynamic Update Capacity & Middle-Sequence Collapse (`DynToM`, `DToM-Track`, `Entity Tracking`, `OpenToM`, Long-Context, `Cowan`):**
   - **DynToM** (Xiao et al. 2025 `[PR]`, *ACL 2025*, `10.18653/v1/2025.acl-long.1171`, `arXiv:2505.17663`): evaluates multi-scenario temporal state evolution (Table 4): GPT-4o transformation accuracy drops across middle spans from **`64.0%`** (span `1–2`) to **`50.0%`** (`2–3`) in 6 scenarios, and falls from **`56.0%`** (`1–2`) down to **`26.0%`** (`3–4` and `5–6`) in 7 scenarios; truncating sequences back to **4 scenarios** (Table 5) boosts middle-span `3–4` accuracy by **`+21.0 pp`** (`26.0% → 47.0%`).
   - **DToM-Track** (Nguyen, Phan & Gonzalez 2026 `[PP]`, `arXiv:2603.14646`): across tested LLMs (3B–70B), models detect belief updates (`67.5%`) and recall post-update states (`63.9%`) far better than **pre-update states (`27.7%`)**—exhibiting strong recency overwrite bias when prior and updated belief items coexist in context. *(Implication for MonoAgens: superseded claims must be evicted/archived out of active ToM cards, never mixed alongside current entries.)*
   - **OpenToM** (Xu et al. 2024 `[PR]`, *ACL 2024*, `10.18653/v1/2024.acl-long.466`, `arXiv:2402.06044`): expanding narrative length ~`2.5×` (`OpenToM-L`, Appendix D.5 / Table A6) degrades fine-grained state tracking (`Loc_fine`), motivating compact character-centric symbolic summaries.
   - **Entity Tracking** (Kim & Schuster 2023 `[PR]`, *ACL 2023*, `10.18653/v1/2023.acl-long.213`, `arXiv:2305.02363`), **Lost in the Middle** (Liu et al. 2024 `[PR]`, *TACL* 12:157–173, `10.1162/tacl_a_00638`), **RULER** (Hsieh et al. 2024 `[PP]`, `arXiv:2404.06654`), **NoLiMa** (Modarressi et al. 2025 `[PP]`, `arXiv:2502.05167`, accepted at ICML 2025: 11 of 13 `128K+` LLMs drop below 50% of short `<1K` baseline at 32K under non-literal retrieval; GPT-4o `99.3% → 69.7%`), and human working-memory bounds (**Cowan 2001** `[PR]`, *BBS*, `10.1017/S0140525X01003922`: `~4` chunks) consistently penalize large unpruned state lists even well below nominal context limits.

**Established / Inferred / Open.**
- *Established:* Keeping irrelevant non-target actor items or >4–7 unpruned temporal updates inside the reasoning prompt degrades ToM false-belief and transformation accuracy (`PercepToM` `[PR]`, `DynToM` `[PR]`, `DToM-Track` `[PP]`, `OpenToM` `[PR]`).
- *Inferred:* `ledger_items` should sit near **4–10** items per field (upper end `≤15`, narrower than Round 1's `5–30`) paired with strict eviction of superseded/resolved entries so cards stay compact and free of stale state (`27.7%` pre-update overwrite confusion in `DToM-Track`).
- *Open:* Exact per-model inflection point of `ledger_items ∈ {4, 8, 12, 20, 30}` as a function of concurrent actors ($N_{\text{present}} \in \{2..6\}$) on structured JSON/text cards vs prose stories.

##### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `ledger_items` | Max entries per field per person card (`witnessed`, `told`, `open_requests`, `candidate_false_beliefs`) | `8` (or `10` per Round 1) | `4–15` (search up to `30` in ablation) | Informed by `DynToM` (`4` scenarios) & `PercepToM` `[PR]`; tune in Lab |
| `max_belief_order` | Max nesting order on self-card | `2` | `1–2` | Round 1 R-25 (`SymbolicToM` `[PR]`) |

##### Neutral value
`ledger_items = 0` (no perception ledger cards placed in context; reasoning relies only on raw recent events and explicit recall).

##### Claim
Bounding `ledger_items` to a compact active window (`~8–10` per field, evicting resolved/superseded items) improves `correct answers on scripted false-belief questions` and lowers `superseded beliefs presented as current` relative to both `ledger_items = 0` (omits asymmetric witness tracking) and unpruned large cards (`≥ 25`, which trigger middle-card distraction and inhibitory-control drops).

##### Lab Experiment
- **Body generality:** Structural epistemic tracking (`who observed what`, `who was told what`) applies across chat groups, multi-robot rooms, and multi-agent games; prose-phrasing robustness tests apply mainly to chat.
- **Scenario:** Multi-party false-belief + belief-update stream across 3–5 actors with 2–4 audience splits (some actors leave and miss updates; several beliefs change across 6+ turns; unresolved requests accumulate alongside benign filler facts).
- **Ablation:** Sweep `ledger_items ∈ {0, 4, 8, 12, 20, 30}` and eviction strategy (`strict FIFO` vs `unresolved-first`) across both scripted and **live** LLM runs (`≥5 seeds`).
- **Metrics (`lab/spec.md` L141–145):** `correct answers on scripted false-belief questions`, `superseded beliefs presented as current`, `assertions of detail made without recollection`, `number of items from a narrower audience placed in a wider context`, and prompt `tokens per event` (`reason` purpose).

##### Sources
- `[PR]` **Sclar, M., Kumar, S., West, P., Suhr, A., Choi, Y., & Tsvetkov, Y. (2023).** "Minding Language Models' (Lack of) Theory of Mind: A Plug-and-Play Multi-Character Belief Tracker." *ACL 2023*, pp. 13960–13980 (`arXiv:2306.00924`). DOI: [`10.18653/v1/2023.acl-long.780`](https://doi.org/10.18653/v1/2023.acl-long.780).
- `[PR]` **Jung, C., Kim, D., Jin, J., Kim, J., Seonwoo, Y., Choi, Y., Oh, A., & Kim, H. (2024).** "Perceptions to Beliefs: Exploring Precursory Inferences for Theory of Mind in Large Language Models." *EMNLP 2024*, pp. 19794–19809 (`arXiv:2407.06004`). DOI: [`10.18653/v1/2024.emnlp-main.1105`](https://doi.org/10.18653/v1/2024.emnlp-main.1105).
- `[PR]` **Kim, H., Sclar, M., Zhou, X., Bras, R., Kim, G., Choi, Y., & Sap, M. (2023).** "FANToM: A Benchmark for Stress-testing Machine Theory of Mind in Interactions." *EMNLP 2023*, pp. 14397–14413 (`arXiv:2310.15421`). DOI: [`10.18653/v1/2023.emnlp-main.890`](https://doi.org/10.18653/v1/2023.emnlp-main.890).
- `[PR]` **Xu, H., Zhao, R., Zhu, L., Du, J., & He, Y. (2024).** "OpenToM: A Comprehensive Benchmark for Evaluating Theory-of-Mind Reasoning Capabilities of Large Language Models." *ACL 2024*, pp. 8593–8623 (`arXiv:2402.06044`). DOI: [`10.18653/v1/2024.acl-long.466`](https://doi.org/10.18653/v1/2024.acl-long.466).
- `[PR]` **Xiao, Y., Wang, J., Xu, Q., Song, C., Xu, C., Cheng, Y., Li, W., & Liu, P. (2025).** "Towards Dynamic Theory of Mind: Evaluating LLM Adaptation to Temporal Evolution of Human States." *ACL 2025*, pp. 24036–24057 (`arXiv:2505.17663`). DOI: [`10.18653/v1/2025.acl-long.1171`](https://doi.org/10.18653/v1/2025.acl-long.1171).
- `[PP]` **Nguyen, T. N., Phan, D. N., & Gonzalez, C. (2026).** "Dynamic Theory of Mind as a Temporal Memory Problem: Evidence from Large Language Models." `arXiv:2603.14646`. URL: [`https://arxiv.org/abs/2603.14646`](https://arxiv.org/abs/2603.14646).
- `[PR]` **Kim, N., & Schuster, S. (2023).** "Entity Tracking in Language Models." *ACL 2023*, pp. 3835–3855 (`arXiv:2305.02363`). DOI: [`10.18653/v1/2023.acl-long.213`](https://doi.org/10.18653/v1/2023.acl-long.213).
- `[PR]` **Liu, N. F., Lin, K., Hewitt, J., Paranjape, A., Bevilacqua, M., Petroni, F., & Liang, P. (2024).** "Lost in the Middle: How Language Models Use Long Contexts." *TACL*, 12:157–173. DOI: [`10.1162/tacl_a_00638`](https://doi.org/10.1162/tacl_a_00638).
- `[PP]` **Hsieh, C.-P., Sun, S., Kriman, S., Acharya, S., Rekesh, D., Jia, F., Zhang, Y., & Ginsburg, B. (2024).** "RULER: What's the Real Context Size of Your Long-Context Language Models?" `arXiv:2404.06654` *(arXiv comment notes COLM 2024)*. URL: [`https://arxiv.org/abs/2404.06654`](https://arxiv.org/abs/2404.06654).
- `[PP]` **Modarressi, A., Deilamsalehy, H., Dernoncourt, F., Bui, T., Rossi, R. A., Yoon, S., & Schütze, H. (2025).** "NoLiMa: Long-Context Evaluation Beyond Literal Matching." `arXiv:2502.05167` *(arXiv comment notes accepted at ICML 2025)*. URL: [`https://arxiv.org/abs/2502.05167`](https://arxiv.org/abs/2502.05167).
- `[PR]` **Cowan, N. (2001).** "The magical number 4 in short-term memory: A reconsideration of mental storage capacity." *Behavioral and Brain Sciences*, 24(1):87–114. DOI: [`10.1017/S0140525X01003922`](https://doi.org/10.1017/S0140525X01003922).

---

#### R-26 Which gate/threshold terms may safely learn online without destabilizing the policy/gate, and how should per-actor teacher weights be set?

**Verdict:** `partly settled` *(Brief-2 status: `partly answered` — performative prediction, two-timescale stochastic approximation, knapsack dual pricing, and noisy-label RLHF/EM theory cleanly separate which parameters can vs cannot co-adapt online, and show that naive linear teacher weights without reliability estimation/shrinkage are unsafe; exact MonoAgens step sizes, Beta teacher-reliability priors, and dual-timescale ratios require Lab calibration).*

##### Evidence & Rule

**Answer.** Do **not** let all `"salience weights and threshold terms"` co-adapt freely at the same rate (`design.md` Decision 12). Under [RESEARCH.md §3–§4](file:///home/muqsith/Projects/Mono/RESEARCH.md#L46-L74) ($P = \sigma((S_{\text{eff}}-\theta)/\tau)$ with $\theta = \theta_0 + a\cdot\text{talk\_rate} + b\cdot\text{budget\_used} - c\cdot\text{promise\_debt} + r\cdot(1-\text{familiarity})$), partition gate parameters into three stability classes obeying **two-timescale separation** (`Borkar 1997` `[PR]`) and **performative contraction** (`Perdomo et al. 2020` `[PR]`):
1. **Never learn online from outcomes:** constraint/stabilizer parameters (`b` on `budget_used`, `c` on `promise_debt`, `τ` temperature, habituation decay `γ` & gain `κ`, circuit-breaker backoff bounds, and the `0` negative anchor of `w_sc` if needed for safety). Fixing `τ` breaks the `(k w, k θ₀, k τ)` scale non-identifiability, and fixing `b, c` prevents reward-driven erosion of hard operational limits (`Badanidiyuru et al. 2013/2018` `[PR]`).
2. **Fast timescale ($\alpha_{\text{learn}}$ + per-event/batch):** bounded **salience signal weights** $w_k$ (via clipped-IPS rule from `R-19`) and **scoped layer-3 offsets** $\delta_{\text{place}}, \delta_{\text{actor}}$ (zero-mean clamped shifts per opaque place/actor ID), plus `timing` latencies (`Decision 8`) and quantile-regularized `wake_gate_lo` (`q_0.10` rather than single minimum order statistic).
3. **Slow timescale (≤ `0.2×` weight learning rate, sleep/day cadence, 95%-CI gated):** global intercept **`θ₀`** and unfamiliarity reserve **`r`** (optionally talk-rate damping **`a`**, clamped $\ge a_{\min} > 0$ so flooding guard cannot invert sign).
4. **Per-actor teacher weights (resolving `bounded-learning/spec.md` vs `RESEARCH.md` §8):** Consistent with `ARCHITECTURE.md` ("core never knows owner"), the gateway supplies an initial per-actor teacher prior weight $\eta_{a,0} \in [0, 1]$ per opaque `actor_id` $a$ (e.g., `1.0` for primary steward/system classifier verdicts, `0.0` default for arbitrary strangers/peers unless the host intentionally elevates a trusted mentor, matching `RESEARCH.md` §8's safety posture). If non-zero teacher weights are given to noisy or fallible sources, modulate the effective teacher weight online via **RE-PO / Dawid–Skene reliability tracking**: maintain a clipped Beta reliability state $(\alpha_a^{\text{teach}}, \beta_a^{\text{teach}})$ updated by agreement with objective/ground-truth verdicts (`classifier verdicts`, `receipt outcomes`, `high-reliability consensus`) and clip $\tilde{\eta}_a = \mathrm{clamp}(\eta_{a,0} \cdot \hat{\rho}_a,\, 0,\, \eta_{\max})$. Stranger/peer idiosyncrasies still feed **layer-3 scoped offsets** (`δ_actor`, `δ_place`) in full without corrupting global weights.

**Formulas & Symbols.**
- **Partitioned Gate Adaptation:**
  - *Frozen safety/scale constants:* $\tau, b, c, \gamma, \kappa$ fixed (modified only offline / via explicit prior revision).
  - *Fast-timescale weight updates* ($k \in \{\text{own\_name}, \text{info\_gap}, \text{fok}, \text{relevance}, \text{social\_cost}\}$):
    $$
    w_k \leftarrow \mathrm{clamp}\!\Big(w_k + \alpha_w \cdot \tilde{\eta}_a \cdot g_i^{\text{IPS}} \cdot s_{k,i},\ w_{k,\min},\ w_{k,\max}\Big), \qquad w_{\text{sc}} \le w_{\text{sc},\max} < 0.
    $$
  - *Slow-timescale threshold updates* (only if $95\%$ CI excludes 0 over $\ge n_{\min}$ resolved outcomes, and rate-limited at $\alpha_\theta \le 0.2\,\alpha_w$ and `≤ 5%/day` total move):
    $$
    \theta_0 \leftarrow \mathrm{clamp}\!\big(\theta_0 - \alpha_\theta \cdot \tilde{\eta}_a \cdot \bar{g}_{\mathcal{B}}^{\text{IPS}},\ \theta_{0,\min},\ \theta_{0,\max}\big), \quad
    r \leftarrow \mathrm{clamp}\!\big(r - \alpha_\theta \cdot \tilde{\eta}_a \cdot \bar{g}_{\mathcal{B}}^{\text{IPS}} \cdot (1 - \mathrm{fam}),\ r_{\min},\ r_{\max}\big),
    $$
    while `a` stays either fixed or strictly bounded above $a_{\min} > 0$.
- **Per-Actor Reliability-Modulated Teacher Weight (Opaque IDs Only):**
  Each opaque actor $a$ enters with gateway-supplied $\eta_{a,0} \in [0, 1]$ (classifier verdicts enter with fixed $\eta_{\text{cls}} = 1$). When outcomes from actor $a$ coexist with objective/ground-truth outcomes (failed receipts, repeated self-corrections, or consensus/classifier confirmation $c_i \in \{0, 1\}$ matching actor $a$'s implied judgment):
  $$
  \hat{\rho}_a = \frac{\alpha_a^{\text{teach}}}{\alpha_a^{\text{teach}} + \beta_a^{\text{teach}}},
  \qquad
  \tilde{\eta}_a = \begin{cases}
  0, & \eta_{a,0} = 0,\\
  \mathrm{clamp}\!\big(\eta_{a,0} \cdot \mathbb{I}[\hat{\rho}_a \ge \rho_{\text{floor}}]\cdot \hat{\rho}_a,\ 0,\ 1\big), & \eta_{a,0} > 0,
  \end{cases}
  $$
  with slow EMA/Beta decay (`RE-PO` used $\eta_0 = 0.9, \alpha_{\text{EMA}} = 0.1$ `[PP]`). Setting `α_EMA = 0` (no online reliability adjustment) reduces identically to the host's static $\eta_{a,0}$.
- **`wake_gate_lo` Robustness Fix:** Replace single-minimum tracking in [bounded-learning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md#L82-L85) with the $q_{0.10}$ empirical quantile of positively classified similarities (shrunk toward prior with $n_0$ pseudo-counts), less margin, clamped to bounds.

**Verified Literature Findings:**
1. **Why unconstrained closed-loop gate learning destabilizes (Performative Prediction, Bandits with Knapsacks, Two-Timescale SA):**
   - **Perdomo, Zrnic, Mendler-Dünner & Hardt (2020)** `[PR]` (*ICML 2020*, `PMLR 119:7599–7609`, `arXiv:2002.06673`): Prove that under performative feedback (where policy decisions alter future interaction distributions, as a talkative or silent agent alters room chatter), repeated retraining only converges to a stable fixed point when distribution-shift sensitivity satisfies the contraction condition $\varepsilon < \gamma / \beta$ (smoothness/strong convexity ratio); large step sizes or simultaneous shift of multiple coupled parameters violate $\varepsilon < \gamma/\beta$ and induce limit-cycle oscillations. MonoAgens' `5%/day` cap and `95%-CI` gate act as discrete contraction controls, provided `τ` and constraint weights remain fixed.
   - **Borkar (1997)** `[PR]` (*Systems & Control Letters*, `10.1016/s0167-6911(97)90015-3`): Proves stochastic approximation stability for coupled recursions when fast-timescale parameters converge cleanly relative to slower quasi-static parameters (step-size separation $b(n)/a(n) \to 0$). Applied to a logistic gate: if feature weights $w_k$ (fast) and base threshold/reserve $\theta_0, r$ (slow) update at separated rates with fixed $\tau$, $\theta_0$ tracks a quasi-stationary equilibrium instead of chasing short-run feedback loops; updating all threshold terms jointly at the same rate violates timescale separation.
   - **Badanidiyuru, Kleinberg & Slivkins (2013/2018)** `[PR]` (*FOCS 2013* `10.1109/focs.2013.30` / *JACM* 65(3):1–55 `10.1145/3164539`): Bandits with Knapsacks prove that optimal resource-constrained decision policies require **monotone dual/shadow pricing** against consumed budget (`b·budget_used`, `c·promise_debt`). Allowing naïve outcome rewards (especially short-term positive replies before budget depletion or broken promises surface) to push `b` or `c` to `0` destroys knapsack feasibility.
2. **Heterogeneous / Noisy / Strategic Teachers (Crowdsourcing & RLHF):**
   - **Dawid & Skene (1979)** `[PR]` (*Applied Statistics*, `10.2307/2346806`), **Raykar et al. (2010)** `[PR]` (*JMLR* 11:1297–1322, `https://www.jmlr.org/papers/v11/raykar10a.html`), and **Zheng et al. (2017)** `[PR]` (*PVLDB* 10(5):541–552, `10.14778/3055540.3055547`): Show that naïve equal or uncalibrated weighting of multiple teachers fails whenever annotators have heterogeneous error rates or systematic bias; EM confusion-matrix estimation recovers latent truth only when annotator reliability is tracked and poor/adversarial sources are downweighted or inverted-filtered.
   - **RE-PO** (Cao et al. 2025/2026 `[PP]` / project site reports acceptance at *ICLR 2026*, `arXiv:2509.24159`, `https://repo-alignment.github.io/`): Models annotator reliability $\eta_k \in (0, 1)$ in RLHF via EM E-step posterior weighting and M-step policy updates. In sensitivity ablations (§5.4), optimal denoising occurred around initial prior $\eta_0 = 0.9$ and reliability EMA rate $\alpha = 0.1$ (whereas $\alpha = 1.0$ caused volatile oscillation and $\alpha = 0.001$ adapted too slowly; overly optimistic $\eta_0 = 0.99$ trusted noisy teachers too strongly early on). Theorem 4.1 shows exact identification requires calibrated baseline assumptions—meaning reliability estimation is not self-identifying from arbitrary ungrounded labels alone, justifying external `η_{a,0}` priors plus classifier/receipt grounding.
   - **Park, Liu, Kong, Zhang & Ozdaglar (2024)** `[PP]` (*ICML 2024 TF2M Workshop*, `arXiv:2405.00254`): Show formally that aggregating heterogeneous human preferences into one global reward model fails under diverse subgroups (justifying MonoAgens' split between **global weights/dispositions** and **scoped per-place / per-actor offsets**) and is vulnerable to **strategic manipulation** (teachers exaggerating preferences unless bounded/mechanism-designed, which motivates MonoAgens' `≤ 5%/day` clamp, per-actor weight cap $\le 1$, and restricting non-trusted peers to local scoped offsets).

**Established / Inferred / Open.**
- *Established:* (a) Two-timescale separation stabilizes coupled fast/slow adaptive loops (`Borkar 1997` `[PR]`); (b) closed-loop distribution feedback diverges when shift sensitivity exceeds smoothness bounds (`Perdomo et al. 2020` `[PR]`); (c) budget/resource shadow weights require monotone feasibility (`Badanidiyuru et al. 2018` `[PR]`); (d) uncalibrated heterogeneous or strategic teachers bias global models unless weighted by estimated reliability and disaggregated via personalization/local offsets (`Dawid & Skene 1979`, `Raykar et al. 2010` `[PR]`, `RE-PO` `arXiv:2509.24159`, `Park et al. 2024` `[PP]`).
- *Inferred:* `core-and-lab` should freeze `(τ, b, c, γ, κ)` online, update `θ₀, r` strictly slower than `w_k`, shrink noisy/uncertain teachers via `η_{a,0} · ρ_hat_a`, keep strangers' `η_{a,0} = 0` so only scoped offsets move, and switch `wake_gate_lo` from raw minimum to $q_{0.10}$.
- *Open:* Exact fast/slow ratio ($\alpha_\theta / \alpha_w$) under 30-day multi-place operation, whether `a` (talk-rate feedback gain) benefits from slow bounded tuning above a positive floor, and threshold $\rho_{\text{floor}}$ below which a fallible teacher's global weight drops to zero.

##### Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `alpha_w` ($\alpha_w$) | Fast learning rate for salience signal weights $w_k$ and scoped offsets | `0.02` | `0.005–0.05` (neutral `0`) | Tune in Lab (`bounded-learning`) |
| `alpha_theta_ratio` ($\alpha_\theta / \alpha_w$) | Slow-timescale multiplier for `θ₀`, `r` (`Borkar 1997` `[PR]`) | `0.2` | `0.05–0.3` (`0` freezes threshold) | Tune in Lab |
| `eta_0` ($\eta_{a,0}$) | Host-supplied teacher prior weight per opaque actor ID (`1.0` steward/classifier; `0.0` strangers) | `1.0` / `0.0` | `0.0–1.0` | Host policy (`RESEARCH.md` §8; `RE-PO` `η₀=0.9` prior for noisy teachers) |
| `rho_ema` ($\alpha_{\text{teach}}$) | Reliability update rate for fallible teachers (`RE-PO` §5.4) | `0.1` | `0.0–0.2` (`0` = static `eta_0`) | `RE-PO` (`arXiv:2509.24159`); tune in Lab |
| `wake_q` | Target lower quantile of positive `classify` similarities for `wake_gate_lo` | `0.10` | `0.05–0.20` | Tune in Lab (replaces brittle `min`) |

##### Neutral value
- Online learning neutral: `alpha_w = 0`, `alpha_theta_ratio = 0` (static prior gate).
- Teacher-reliability modulation neutral: `alpha_teach = 0` (uses host's static `teacher_weight` unchanged).

##### Claim
Restricting online updates to bounded `w_k`, scoped place/actor offsets, and slow-timescale `(θ₀, r)` while holding `(τ, b, c, γ, κ)` fixed prevents budget/promise collapse and closed-loop threshold oscillations; simultaneously, RE-PO-style reliability shrinkage protects global parameters when non-steward or noisy teachers have non-zero teacher weight, without blocking adaptation in scoped per-place offsets.

##### Lab Experiment
- **Body generality:** Fully generic across chat, device/sensor, and game bodies.
- **Scenario A (Gate term stability):** 30-day multi-place stream with bursty feedback and sparse long-term promise debt / tight daily budget (`≥5 seeds`).
  - *Ablations:* (i) freeze all learning (`neutral`), (ii) free adaptation of all `(w_k, θ₀, a, b, c, τ)` at equal rate (`core-and-lab` unconstrained interpretation), (iii) constrained fast `w_k` + slow `(θ₀, r)` with fixed `(b, c, τ)`.
  - *Metrics:* policy oscillation variance of $P$ and $\theta$, `promises kept on time`, budget overshoot rate, `when-to-speak precision/recall`, `freeze and report` events.
- **Scenario B (Noisy / Adversarial Teacher Resistance):** 3 actors: Actor 1 (`η_0 = 1.0`, clean), Actor 2 (`η_0 = 0.5`, 30–40% noisy/contradictory reactions vs ground-truth receipt/classifier checks), Actor 3 (`η_0 = 0.0`, hostile/high-volume spammer in Place 2 only).
  - *Ablations:* static linear `teacher_weight` (`bounded-learning` current) vs `RESEARCH.md` binary (`{0, 1}`) vs reliability-modulated `η_{a,0}·ρ_hat_a` + scoped Place 2 offset.
  - *Metrics:* global drift error on Place 1 (uncontaminated place), Place 2 adaptation accuracy (`precision/recall`), and days-to-suppress noisy Actor 2 without corrupting global weights.

##### Sources
- `[PR]` **Perdomo, J. C., Zrnic, T., Mendler-Dünner, C., & Hardt, M. (2020).** "Performative Prediction." *ICML 2020*, PMLR 119:7599–7609 (`arXiv:2002.06673`). URL: [`https://proceedings.mlr.press/v119/perdomo20a.html`](https://proceedings.mlr.press/v119/perdomo20a.html).
- `[PR]` **Borkar, V. S. (1997).** "Stochastic approximation with two time scales." *Systems & Control Letters*, 29(5):291–294. DOI: [`10.1016/s0167-6911(97)90015-3`](https://doi.org/10.1016/s0167-6911(97)90015-3).
- `[PR]` **Badanidiyuru, A., Kleinberg, R., & Slivkins, A. (2018; conf. 2013).** "Bandits with Knapsacks." *Journal of the ACM*, 65(3):1–55 (orig. *FOCS 2013*, `arXiv:1305.2545`). DOI: [`10.1145/3164539`](https://doi.org/10.1145/3164539) / [`10.1109/focs.2013.30`](https://doi.org/10.1109/focs.2013.30).
- `[PR]` **Dawid, A. P., & Skene, A. M. (1979).** "Maximum Likelihood Estimation of Observer Error-Rates Using the EM Algorithm." *Applied Statistics*, 28(1):20–28. DOI: [`10.2307/2346806`](https://doi.org/10.2307/2346806).
- `[PR]` **Raykar, V. C., Yu, S., Zhao, L. H., Valadez, G. H., Florin, C., Bogoni, L., & Moy, L. (2010).** "Learning From Crowds." *Journal of Machine Learning Research*, 11(43):1297–1322. URL: [`https://www.jmlr.org/papers/v11/raykar10a.html`](https://www.jmlr.org/papers/v11/raykar10a.html).
- `[PR]` **Zheng, Y., Li, G., Li, Y., Shan, C., & Cheng, R. (2017).** "Truth inference in crowdsourcing." *Proceedings of the VLDB Endowment*, 10(5):541–552. DOI: [`10.14778/3055540.3055547`](https://doi.org/10.14778/3055540.3055547).
- `[PP]` **Cao, X., Xu, Z., Guang, M., Long, K., Bakker, M. A., Wang, Y., & Yu, C. (2025).** "RE-PO: Robust Enhanced Policy Optimization as a General Framework for LLM Alignment." `arXiv:2509.24159` *(authors' project site [`repo-alignment.github.io`](https://repo-alignment.github.io/) states accepted at ICLR 2026)*.
- `[PP]` **Park, C., Liu, M., Kong, D., Zhang, K., & Ozdaglar, A. (2024).** "RLHF from Heterogeneous Feedback via Personalization and Preference Aggregation." `arXiv:2405.00254` (ICML 2024 TF2M Workshop). URL: [`https://arxiv.org/abs/2405.00254`](https://arxiv.org/abs/2405.00254).

---

#### R-27 Verify the PITCH dataset (`arXiv:2610.25896`) directly: public release, license, participant consent, and comparable multi-party chat protocols

**Verdict:** `settled` *(Brief-2 status: `answered` — verified directly from `arXiv:2610.25896v2` HTML/abstract, GitHub repositories, MIT `LICENSE` files, and peer-reviewed comparative corpora).*

##### Evidence & Rule

**Answer.** PITCH (*Proactive Intervention Timing in Chat*; Wang, Yu, Zhou, Chang, Siangliulue, Bragg, Jiang & Zhang, `arXiv:2610.25896v2`, `[PP]`) **is publicly released** on GitHub and its project site under the **MIT License**, with verified **IRB approval, informed consent for public release, $35 compensation, name removal, and right of post-hoc withdrawal**. For any future data collection or Lab corpus expansion (especially Indonesian + English group chats), MonoAgens should adopt an **MIT / Apache-2.0 / CC BY 4.0** release structure paired with a pre-conversation + post-conversation **reaffirmed informed consent, alias de-identification, AI-participant disclosure (if an AI joins live), PII scrubbing, and post-hoc withdrawal protocol** modeled on PITCH, CANDOR, and LLMafia.

**Verified Primary Details for PITCH (`arXiv:2610.25896v2`):**
- **Authors:** Ruotong Wang, Brian S. Yu, Xinyi Zhou, Joseph Chee Chang, Pao Siangliulue, Jonathan Bragg, Liwei Jiang, Amy X. Zhang.
- **Public Releases & Exact Files Verified:**
  - Project website: [`https://social.cs.washington.edu/PITCH-website/`](https://social.cs.washington.edu/PITCH-website/)
  - Benchmark data repository: [`https://github.com/Social-Futures-Lab/pitch`](https://github.com/Social-Futures-Lab/pitch) (branch `master`), containing `data/pitch-benchmark-data.json` (`6,012,106` bytes), `data/human-baseline-data.csv` (`38,618` bytes), and `CITATION.cff` (`2026-10-18`).
  - Human baseline repository: [`https://github.com/Social-Futures-Lab/pitch-human-baseline`](https://github.com/Social-Futures-Lab/pitch-human-baseline) (branch `main`).
- **License Verified:** Both `Social-Futures-Lab/pitch` (`master/LICENSE`) and `Social-Futures-Lab/pitch-human-baseline` (`main/LICENSE`) carry the **MIT License** (`Copyright (c) 2026 PITCH Benchmark Contributors` / `Copyright (c) 2026 Brian Yu`), permitting commercial, research, and evaluation reuse with attribution; the arXiv paper itself carries `CC BY 4.0`.
- **Corpus Size (`§2`, Tables 1–2):** 35 unscripted typed conversations in groups of 2–4, **2,194 turns**, **384** participant-authored factual candidate messages, **1,091** timing judgments (ideal turn / acceptable window / should not appear) from **89 participants** (`96` group memberships; Table 2 lists `96 (83)` unique annotators under human timing labels), median 3 and minimum 2 preferences per candidate message.
- **IRB & Participant Consent Protocol (Verbatim quote, Ethics statement §6 / L401–407):**
  > *"The study was reviewed and approved by our institution’s IRB. Participants gave informed consent to the public release of the conversation they held and the annotations they produced, were compensated $35 for the session, and could withdraw at any point, including after the fact. Participants’ names are removed from the released dataset."*
- **Limitations explicitly noted in §6:** English-language text among small, broadly egalitarian US-heavy research/social networks—so timing norms learned directly from PITCH reflect one cultural prior (critical when MonoAgens runs multilingual Indonesian/English chats).

**Comparable Multi-Party / Dialogue Consent & Release Protocols:**

| Corpus | Tag / Venue / ID | Consent & Ethics Protocol | De-identification & Gating | Release License |
|---|---|---|---|---|
| **PITCH** (Wang et al. 2026) | `[PP]` `arXiv:2610.25896v2` | Institutional IRB; informed consent specifically covering *public release* of chat + annotations; $35 for 90-min session; right to withdraw *even after the fact* | Participant names removed from released data; publicly downloadable without gate | **MIT License** (data/code repos); paper `CC BY 4.0` |
| **CANDOR** (Reece et al. 2023) | `[PR]` *Science Advances* 9(13), `10.1126/sciadv.adf3197` | Ethical & Independent Review Services IRB `#19160-01`; US Prolific 18+; **two-stage reaffirmed consent** warning explicitly of personal identification risk from A/V; `$0.85` intake + `$14.15` completion | 1,656 conversations; researcher-registration / access-controlled release due to raw video/audio identity risk | Controlled researcher access (`sciadv.adf3197`) |
| **LLMafia** (Eckhaus et al. 2025) | `[PR]` *Findings of EMNLP 2025*, `10.18653/v1/2025.findings-emnlp.608`, `arXiv:2506.05309` | University Ethics Committee approval; signed consent form (Appendix A.1); **explicitly discloses one player is an AI** without revealing which character | 33 games, 3,593 turns, 93 players; **fresh random character alias per game** preventing cross-game identity tracking | Paper `arXiv` / ACL Anthology (`CC BY 4.0` ACL default) |
| **AMI Meeting Corpus** (Carletta et al.) | `[IND]` [`groups.inf.ed.ac.uk/ami/corpus/license.shtml`](https://groups.inf.ed.ac.uk/ami/corpus/license.shtml) | Recorded workplace/scenario meetings with participant consent & anonymized speaker labels | Speaker pseudonyms (`FEE005` etc.); raw audio/transcripts | **Creative Commons Attribution 4.0 (`CC BY 4.0`)** |
| **PersuasionForGood** (Wang et al. 2019) | `[PR]` *ACL 2019*, `10.18653/v1/P19-1566` | Institutional protocol; MTurk active task consent; anonymized IDs; (§8 ethics discussion) | Crowdworker IDs stripped (`gitlab.com/ucdavisnlp/persuasionforgood`) | **Apache-2.0** (`LICENSE` verified) |
| **Ubuntu Dialogue Corpus** (Lowe et al. 2015) | `[PR]` *SIGDIAL 2015*, `arXiv:1506.08909` | Scraped public technical IRC logs (`#ubuntu`, public channel norms; no individual consent form—not recommended for new private chat collection) | Raw IRC handles retained in many dumps | Open community scripts/links (`github.com/rkadlec/ubuntu-ranking-dataset-creator`) |
| **ProACT** (Yang et al. 2026) | `[PP]` `arXiv:2607.03730` | Derived from public GitHub issue threads + QMSum + synthetic multi-party data *(paper notes provenance/handle considerations)* | Derived benchmark | `CC BY 4.0` (`arXiv` license) |
| **When2Speak** (Nama et al. 2026) | `[PP]` `arXiv:2605.05626` | Synthetic multi-party dialogues grounded on Yahoo conversations (avoids live human consent requirements for synthetic turns, but reflects source corpus norms) | Synthetic speakers (`huggingface.co/datasets/duke-trust-lab/When2Speak`) | `CC BY-SA 4.0` (`arXiv` license) |

**Established / Inferred / Open.**
- *Established:* PITCH is publicly downloadable under MIT License with IRB approval, $35 compensation, name removal, and post-hoc withdrawal (`arXiv:2610.25896`). Comparable human multi-party chat collections converge on IRB approval, explicit public-release consent, alias anonymization per session (`LLMafia`), AI-presence disclosure (`LLMafia`), and two-step consent whenever identifying media exist (`CANDOR`).
- *Inferred:* PITCH can be legally ingested **into Lab test-body scenarios** right away under MIT License (keeping its binary/graded human timing labels as scenario annotations under [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L130-L135), without modifying core code); however, PITCH does not cover Indonesian/bilingual group norms or physical/sensor bodies.
- *Open:* Empirical timing differences between PITCH's US-English egalitarian groups and Indonesian/multilingual group norms (`A-15`), requiring a small IRB-consented bilingual validation set or Lab cross-cultural comparison if multilingual timing thresholds diverge.

##### Parameters

*(No new core parameter; dataset adoption is Lab-level.)*

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `pitch_tolerance_band` | Mapping from PITCH `ideal` vs `acceptable` turn windows to Lab positive reply checks | `ideal ∪ acceptable` | `{ideal only, ideal ∪ acceptable}` | `arXiv:2610.25896` §3 |

##### Neutral value
Run Lab only on internally scripted development/validation scenarios without PITCH external chat replay (`PITCH = off`).

##### Claim
Evaluating MonoAgens' gate and waiting/turn-taking timing against human-consensus multi-party timing labels in PITCH gives an external non-self-authored ground truth (`when-to-speak precision and recall`, reply-delay distribution) that catches premature interjection biases invisible in synthetic self-scripted chats.

##### Lab Experiment
- **Body generality:** **Chat-only** (PITCH is strictly multi-party typed synchronous group conversation; physical/sensor/game bodies require separate body-native timing traces).
- **Scenario:** Convert PITCH `data/pitch-benchmark-data.json` conversations (`35` dialogues, `384` candidate intervention points, mapped onto opaque speaker IDs `p_1..p_k` and injected clock timestamps) into deterministic scripted Lab test-body replay scenarios (`specs/lab/spec.md` §"The Lab reports behaviour against annotation"). Compare `note` + `go` threshold timing (`θ₀`, `patience`, `note_band`, `restraint_after_speaking`) against PITCH's `1,091` human turn-timing preferences.
- **Ablation:** MonoAgens full gate vs static threshold (`neutral` habituation, `neutral` patience, `neutral` learning) across seeds (`≥5` for stochastic draws).
- **Metrics (`lab/spec.md` L131–135):** `when-to-speak precision and recall` against `ideal` and `acceptable` windows, `false speaks`, `missed replies`, `soul's share of messages vs 1/N`, and early-intervention rate before `acceptable` window opens.

##### Sources
- `[PP]` **Wang, R., Yu, B. S., Zhou, X., Chang, J. C., Siangliulue, P., Bragg, J., Jiang, L., & Zhang, A. X. (2026).** "When Should AI Chime In? Benchmarking Timely Contribution of Proactive Agents in Multi-Party Chat." `arXiv:2610.25896v2`. URL: [`https://arxiv.org/abs/2610.25896`](https://arxiv.org/abs/2610.25896).
- `[IND]` **PITCH GitHub Repositories & Website (2026):** [`Social-Futures-Lab/pitch`](https://github.com/Social-Futures-Lab/pitch) (`MIT License`), [`Social-Futures-Lab/pitch-human-baseline`](https://github.com/Social-Futures-Lab/pitch-human-baseline) (`MIT License`), [`https://social.cs.washington.edu/PITCH-website/`](https://social.cs.washington.edu/PITCH-website/).
- `[PR]` **Reece, A., Cooney, G., Bull, P., Chung, C., Dawson, B., Fitzpatrick, C., Glazer, T., Knox, D., Liebscher, A., Marin, S., et al. (2023).** "The CANDOR corpus: Insights from a large multimodal dataset of naturalistic conversation." *Science Advances*, 9(13). DOI: [`10.1126/sciadv.adf3197`](https://doi.org/10.1126/sciadv.adf3197).
- `[PR]` **Eckhaus, N., Berger, U., & Stanovsky, G. (2025).** "Time to Talk: LLM Agents for Asynchronous Group Communication in Mafia Games." *Findings of EMNLP 2025* (`arXiv:2506.05309`). DOI: [`10.18653/v1/2025.findings-emnlp.608`](https://doi.org/10.18653/v1/2025.findings-emnlp.608).
- `[IND]` **AMI Meeting Corpus License:** Creative Commons Attribution 4.0 International (`CC BY 4.0`). URL: [`https://groups.inf.ed.ac.uk/ami/corpus/license.shtml`](https://groups.inf.ed.ac.uk/ami/corpus/license.shtml).
- `[PR]` **Wang, X., Shi, W., Kim, R., Oh, Y., Yang, S., Zhang, J., & Yu, Z. (2019).** "Persuasion for Good: Towards a Personalized Persuasive Dialogue System for Social Good." *ACL 2019*, pp. 5635–5649 (`Apache-2.0` at [`gitlab.com/ucdavisnlp/persuasionforgood`](https://gitlab.com/ucdavisnlp/persuasionforgood)). DOI: [`10.18653/v1/P19-1566`](https://doi.org/10.18653/v1/P19-1566).
- `[PR]` **Lowe, R., Pow, N., Serban, I., & Pineau, J. (2015).** "The Ubuntu Dialogue Corpus: A Large Dataset for Research in Unstructured Multi-Turn Dialogue Systems." *SIGDIAL 2015* (`arXiv:1506.08909`). URL: [`https://arxiv.org/abs/1506.08909`](https://arxiv.org/abs/1506.08909).
- `[PP]` **Yang, S., Xu, D., Pei, J., & Wang, D. (2026).** "ProACT: Towards Breakdown-Aware Proactive Agent in Multi-User Collaboration." `arXiv:2607.03730` (`CC BY 4.0`). URL: [`https://arxiv.org/abs/2607.03730`](https://arxiv.org/abs/2607.03730).
- `[PP]` **Nama, V., Mendi, S., Ye, Z., & Bent, B. (2026).** "When2Speak: A Dataset for Temporal Participation and Turn-Taking in Multi-Party Conversations for Large Language Models." `arXiv:2605.05626` (`CC BY-SA 4.0`). URL: [`https://arxiv.org/abs/2605.05626`](https://arxiv.org/abs/2605.05626).

---

#### R-28 Primary-source verification of the four unverified Round-1 citations

**Verdict:** `settled` *(Brief-2 status: `answered` — all four citations, venues, DOIs, author lists, mathematical formulas, and exact quoted claims verified directly from Crossref, arXiv HTML/full texts, ACL Anthology, ACM DL, and IEEE).*

##### Evidence & Rule

**Answer.** All four citations flagged as unverified in Round 1 (`R-28`) are real, accurately attributed peer-reviewed (`[PR]`) papers matching their Round-1 claims. Every DOI, venue, exact author list, and primary quotation is established below. Two precision notes apply to [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md): (1) Liu et al.'s $\lambda = 1.02$ increase rate applies per **discrete conversation turn/timestep**, whereas MonoAgens computes elapsed duration / talk rates continuously via injected clock; (2) CaMeL (`SaTML 2026`) and Humanoid Agents (`EMNLP 2023 Demos`) apply generically to tool/agent security and cognitive simulation, while Inner Thoughts (`CHI 2025`) and Mori et al. (`SIGDIAL 2026`) specifically study multi-party human–AI conversation.

1. **Liu et al. (2025) — "Proactive Conversational Agents with Inner Thoughts" (`arXiv:2501.00383`, CHI 2025 `[PR]`):**
   - **Verified Metadata:** Xingyu Bruce Liu, Shitao Fang, Weiyan Shi, Chien-Sheng Wu, Takeo Igarashi, & Xiang 'Anthony' Chen (2025). *Proceedings of the 2025 CHI Conference on Human Factors in Computing Systems (CHI '25)*, pp. 1–19. DOI: [`10.1145/3706598.3713760`](https://doi.org/10.1145/3706598.3713760); `arXiv:2501.00383v2`.
   - **Verification of "eight heuristics" (§4.3.2 & §5.4, exact quotes):**
     - §4.3.2: *"We summarize eight heuristics to determine whether a participant wants to express or withhold a thought (Figure 2), and collectively name them the intrinsic motivations for participants to engage in conversations."*
     - §5.4 lists all eight verbatim: **`Relevance`**, **`Information Gap`**, **`Expected Impact`**, **`Urgency`**, **`Coherence`**, **`Originality`**, **`Balance`**, and **`Dynamics`** (derived from 394 retrospective think-aloud quotes across 24 participants, with mention counts 77, 33, 23, 14, 30, 16, 33, 30 respectively).
   - **Verification of $\lambda = 1.02$ and formula (§5.4, exact equation & quote):**
     $$
     \text{score} = \sum_{i=1}^{5} p(s_i)\cdot s_i \cdot d_p, \qquad d_p = \lambda^{(t - \tau_p)}
     $$
     with verbatim definition: *"where $\lambda$ is the increase rate of motivation score (`1.02`), $t$ is the current timestep, and $\tau_p$ is the last time when party $p$ spoke."* *(Distinguish carefully from §5.2 memory retrieval decay $d_x = \lambda^{(t-\tau_x)}$ where retrieval decay uses $\lambda = 0.95$.)*
   - **MonoAgens implication:** Verified `[PR]`. Update [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L93-L98) to clarify that $t - \tau_p$ in Liu et al. measures discrete conversational turns rather than continuous wall-clock seconds/minutes.

2. **CaMeL — "Defeating Prompt Injections by Design" (`arXiv:2503.18813`, IEEE SaTML 2026 `[PR]`):**
   - **Verified Metadata:** Edoardo Debenedetti, Ilia Shumailov, Tianqi Fan, Jamie Hayes, Nicholas Carlini, Daniel Fabian, Christoph Kern, Chongyang Shi, Andreas Terzis, & Florian Tramèr (2026). *2026 IEEE Conference on Secure and Trustworthy Machine Learning (SaTML)*, pp. 587–618. DOI: [`10.1109/satml68715.2026.00040`](https://doi.org/10.1109/satml68715.2026.00040); `arXiv:2503.18813v2`.
   - **MonoAgens implication:** Verified `[PR]` at IEEE SaTML 2026 (confirming [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L180) and upgrading its `[PP]` tag cleanly to `[PR]`). Supports `origin_actor` / capability-tagged provenance propagation along causal chains across all bodies.

3. **"Humanoid Agents" (`arXiv:2310.05418`, EMNLP 2023 System Demonstrations `[PR]`):**
   - **Verified Metadata:** Zhilin Wang, Yu Ying Chiu, & Yu Cheung Chiu (2023). *"Humanoid Agents: Platform for Simulating Human-like Generative Agents."* *Proceedings of the 2023 Conference on Empirical Methods in Natural Language Processing: System Demonstrations*, pp. 167–176. DOI: [`10.18653/v1/2023.emnlp-demo.15`](https://doi.org/10.18653/v1/2023.emnlp-demo.15); `arXiv:2310.05418v1` (arXiv comment also confirms: `Accepted at EMNLP System Demonstrations 2023`).
   - **MonoAgens implication:** Verified `[PR]` at EMNLP 2023 System Demonstrations.

4. **Mori et al. (2026) — "On the Structure of Address in Multi-Party Dialogue: From Discrete Labels to Continuous Levels" (`arXiv:2607.15648`, SIGDIAL 2026 `[PR]`):**
   - **Verified Metadata:** Taiga Mori, Koji Inoue, Divesh Lala, & Tatsuya Kawahara (2026). *Proceedings of the 27th Annual Meeting of the Special Interest Group on Discourse and Dialogue (SIGDIAL 2026)*, pp. 668–679. DOI: [`10.18653/v1/2026.sigdial-1.47`](https://doi.org/10.18653/v1/2026.sigdial-1.47); `arXiv:2607.15648v1`.
   - **Verification of "moderate agreement" wording (§3.4, exact quote):**
     > *"As a measure of inter-annotator agreement, we calculated Fleiss’ kappa. For the 3,078 turns annotated by three annotators, the agreement was $\kappa = 0.52$, indicating moderate agreement. This suggests that the addressee annotation was reasonably consistent overall, while still leaving room for ambiguity..."*
   - **MonoAgens implication:** Verified `[PR]` at SIGDIAL 2026, confirming Round 1's justification (`R-29` motivation) for continuous addressee degree rather than purely crisp binary addressee labels in multi-party chat.

**Established / Inferred / Open.**
- *Established:* All four target bibliographic items are published peer-reviewed papers (`[PR]`) with verified DOIs, authors, proceedings, equations, and quotes.
- *Inferred:* Liu et al.'s discrete-turn `1.02^(t - τ_p)` must not be ported verbatim to wall-clock timestamps without unit normalization; Mori et al.'s Fleiss' $\kappa = 0.52$ holds in multi-party dialogue and motivates continuous addressee scoring (`R-29`) specifically in chat/speech settings.
- *Open:* None (bibliographic verification of the four Round-1 items is complete).

##### Parameters
*(Verification question; no new parameters introduced. Keep `RESEARCH.md` discrete-turn note on Liu et al. `1.02`.)*

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| — | No new runtime parameter introduced by `R-28` | — | — | Verification only |

##### Neutral value
Not applicable (`R-28` is bibliographic verification of existing references in `RESEARCH.md` and Round 1).

##### Claim
Every citation used as design justification in `RESEARCH.md` and `run-1.md` (`Inner Thoughts`, `CaMeL`, `Humanoid Agents`, `Mori et al.`) corresponds accurately to a peer-reviewed primary source (`[PR]`), and misapplying discrete-turn `λ = 1.02` as continuous-time rate is avoided.

##### Lab Experiment
- **Body generality:** No runtime mechanism added; verification audit complete. If comparing Liu et al.'s `1.02` turn growth vs MonoAgens' talk-rate damping (`a·talk_rate`), test turn-indexed vs elapsed-time normalization in multi-party chat scenarios (`PITCH` / Lab group chat, `≥5 seeds`, measuring `false speaks` and `soul's share of messages vs 1/N`).

##### Sources
- `[PR]` **Liu, X. B., Fang, S., Shi, W., Wu, C.-S., Igarashi, T., & Chen, X. 'A'. (2025).** "Proactive Conversational Agents with Inner Thoughts." *Proceedings of the 2025 CHI Conference on Human Factors in Computing Systems (CHI '25)*, pp. 1–19 (`arXiv:2501.00383v2`). DOI: [`10.1145/3706598.3713760`](https://doi.org/10.1145/3706598.3713760).
- `[PR]` **Debenedetti, E., Shumailov, I., Fan, T., Hayes, J., Carlini, N., Fabian, D., Kern, C., Shi, C., Terzis, A., & Tramèr, F. (2026).** "Defeating Prompt Injections by Design." *2026 IEEE Conference on Secure and Trustworthy Machine Learning (SaTML)*, pp. 587–618 (`arXiv:2503.18813v2`). DOI: [`10.1109/satml68715.2026.00040`](https://doi.org/10.1109/satml68715.2026.00040).
- `[PR]` **Wang, Z., Chiu, Y. Y., & Chiu, Y. C. (2023).** "Humanoid Agents: Platform for Simulating Human-like Generative Agents." *Proceedings of the 2023 Conference on Empirical Methods in Natural Language Processing: System Demonstrations*, pp. 167–176 (`arXiv:2310.05418v1`). DOI: [`10.18653/v1/2023.emnlp-demo.15`](https://doi.org/10.18653/v1/2023.emnlp-demo.15).
- `[PR]` **Mori, T., Inoue, K., Lala, D., & Kawahara, T. (2026).** "On the Structure of Address in Multi-Party Dialogue: From Discrete Labels to Continuous Levels." *Proceedings of the 27th Annual Meeting of the Special Interest Group on Discourse and Dialogue (SIGDIAL 2026)*, pp. 668–679 (`arXiv:2607.15648v1`). DOI: [`10.18653/v1/2026.sigdial-1.47`](https://doi.org/10.18653/v1/2026.sigdial-1.47).
