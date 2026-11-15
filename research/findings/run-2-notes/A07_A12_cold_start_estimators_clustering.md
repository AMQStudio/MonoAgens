# MonoAgens Research Brief 2 — Part A (A-07, A-08, A-09, A-12)

> Scope: Answers **A-07 (Cold starts)**, **A-08 (The estimator in R-08)**, **A-09 (Recollection for automatic recall)**, and **A-12 (Domains)** in strict accordance with `ARCHITECTURE.md` invariants, `research/researcher-prompt-2.md`, and verified external literature as of 2026-11-16.

---

## Contradictions, Wrong Premises, and Gaps Found in `core-and-lab`

Inspection of `openspec/changes/core-and-lab/` against `research/findings/2026-11-15-run-1.md` and mathematical/cognitive theory reveals five concrete bugs or contradictions that the change following `core-and-lab` must fix:

1. **R-18 Content-miss z-score fails mathematically on small $n$ (`specs/expectations/spec.md` lines 138–150; `tasks.md` 14.3).** Neither spec nor task specifies what the z-score does before enough unrepaired replies exist in a place. By **Samuelson's inequality** (Samuelson 1968 [PR]), when the current observation $x_n$ is included in the Bessel-corrected sample $(\bar{x}_n, s_{n-1})$, $|x_n - \bar{x}_n|/s_{n-1} \le (n-1)/\sqrt{n} < 2$ for every $n \le 5$ — making `z_miss = 2` mathematically impossible for the first five replies. Conversely, excluding $x_n$ causes division by zero at $n \in \{1, 2\}$ and Cauchy-like false-positive spikes at $n = 3$ ($df = 1$, where Student-$t$ requires $T_3 \ge 13.97$ to match $\Phi(2) = 0.97725$).
2. **`typical latency` lives on capabilities, while `patience(P)` needs a place-level prior (`design.md` Decision 5 vs `specs/expectations/spec.md` lines 169–181).** Decision 5 supplies `typical latency` only on the *capability descriptor* (used on `act`), whereas `expectations/spec.md` states that `patience(P)` — triggered by an *incoming unallocated percept*, potentially before any capability has ever been invoked in place $P$ or in receive-only places — starts from *"the latency the body declared"*. Moreover, a single point scalar `typical_latency` cannot simultaneously equal both `patience(P)` ($q_p = 0.5$) and `expiry(P)` ($q_e = 0.9$) at $n = 0$ without collapsing the median and 90th percentile of a right-skewed distribution into one number.
3. **R-03 calibration assumes stationarity in $A^*$ and underpowers the 99.5th percentile bound (`design.md` Decision 15; `specs/lab/spec.md` lines 213–222; `tasks.md` 10.10).**
   - `fok_threshold` and `fok_scale` are calibrated once per embedder on the Lab dev set over $A^* = B + w_{\text{ctx}}\cos + w_{\text{imp}}\text{imp}$. Because Petrov's $B$ starts at $\ln((1/60)^{-0.5}) \approx +2.05$ for 1-minute-old events and drops to $\approx -3.29$ after 30 days ($d = 0.5$), a static `fok_threshold` calibrated on multi-day dev histories saturates `feeling_of_knowing` near $1.0$ on a brand-new soul's recent messages; conversely, `core-and-lab` provides no fallback when a new embedder is used without `monoagens-lab calibrate`.
   - R-03 sets the upper bound of `wake_gate_lo` at the **99.5th percentile** of unrelated-pair cosines. By distribution-free coverage (Murphy 1948 [PR]), guaranteeing 95% probability that the sample maximum even reaches the 99.5th percentile requires $m \ge \lceil \ln(0.05)/\ln(0.995) \rceil = 598$ *independent* disjoint pairs (requiring $\ge 1{,}196$ distinct uncorrelated events if pairs do not overlap); small dev sets cannot estimate the 99.5th percentile reliably nonparametrically.
4. **R-16's recollection rule contradicts cross-place automatic recall (`specs/memory-recall/spec.md` lines 82–100; `design.md` Decision 7 & 9 vs `tasks.md` 11.1).** `memory-recall/spec.md` requires *"actor or audience contains the people concerned, whose place matches and whose time falls within the grain claimed"* without distinguishing **explicit `memory.recall`** (where the query supplies target people/place/grain) from **automatic wake recall** (where nothing claims anything yet). If automatic recall tests against the *triggering event's* place/people/time, then a verbatim memory from Place A 30 days ago (`tasks.md` 11.1 `recall-across-places` and `reasoning/spec.md` "A conversation continues on another body") would be wrongly marked `familiar only` ($R = 0$).
5. **R-21 domain re-clustering breaks `(actor, domain)` Beta counts and `vector_released` replay (`design.md` Decision 10 & 11; `specs/beliefs/spec.md` lines 77–99; `tasks.md` 16.6–16.7).** `core-and-lab` recomputes $K$ centroids during sleep (`domains_updated`) and requires all credence counts to be rebuildable from the log, but specifies neither (a) cold-start $K_{\text{eff}}$ when $N_{\text{vec}} < K$, (b) stable domain ID preservation across sleep runs (without matching, cluster indices permute and silently corrupt per-domain actor reliability), nor (c) how `(α, β)` counts survive cluster splits, merges, retirements, and `vector_released` vector deletion.

---

# A-07 Cold starts

**Status:** partly answered

## Answer

Every rule that depends on place or soul history uses **empirical-Bayes shrinkage from a body/embedder prior at $n = 0$ toward place-level statistics as observations accumulate**, paired with **distribution-free sample-size thresholds** ($n_{\min}$) derived from Student-$t$/$\chi^2$ precision and order-statistic coverage. Nothing divides by zero, produces `NaN`, or suppresses/triggers signals artifactually on the first event, and every statistic degrades smoothly to a neutral or prior value across any body.

### 1. Content-miss z-score against unrepaired replies (R-18)

- **Why naive z-score fails:** For $n \le 1$, sample standard deviation $s$ is undefined or zero. If the current observation $x_n$ is included in $(\bar{x}_n, s_{n-1})$, Samuelson's inequality bounds $|x_n - \bar{x}_n|/s_{n-1} \le (n-1)/\sqrt{n} < 2$ for all $n \le 5$ (Samuelson 1968 [PR]). If $x_n$ is excluded, the sequential ratio $T_r = \sqrt{\frac{r-1}{r}} \frac{x_r - \bar{x}_{r-1}}{s_{r-1}}$ follows Student-$t_{r-2}$ (Quesenberry 1991 [PR]), which does not exist at $r \le 2$ and has heavy Cauchy tails at $r = 3$ ($t^{-1}(\Phi(2), df=1) \approx 13.97$). Classical Phase-I SPC requires $25\text{–}30$ observations before sample variance stabilizes (Hillier 1969 [PR]; Quesenberry 1991 [PR]): exact $\chi^2$ 95% confidence bounds for $\sigma$ narrow from $[0.599s, 2.874s]$ at $n=5$ ($df=4$) to $[0.688s, 1.826s]$ at $n=10$ ($df=9$), $[0.760s, 1.461s]$ at $n=20$ ($df=19$), and $[0.796s, 1.344s]$ at $n=30$ ($df=29$).
- **Rule & formula (all $n \ge 0$):** Track Welford's online mean $\bar{x}_n$ and sum of squares $M_{2,n}$ (Welford 1962 [PR]) over unrepaired replies per `(place, embedder_tag)` (resetting if the embedder changes). Shrink toward a hierarchical prior $(\mu_{\text{prior}}, \sigma_{\text{prior}}^2)$ — specifically the soul's pooled unrepaired-reply statistics across places sharing the same `embedder_tag` when $n_{\text{pool}} \ge n_{\min,\text{miss}}$, and otherwise an embedder/default prior $(\mu_{0,\text{miss}}, \sigma_{0,\text{miss}})$ — using conjugate Normal-Inverse-Gamma predictive standardization:
  $$\hat{\mu}_n = \frac{\nu_{0,\text{miss}}\,\mu_{\text{prior}} + n\,\bar{x}_n}{\nu_{0,\text{miss}} + n}, \qquad \hat{\sigma}_n^2 = \frac{\nu_{0,\text{miss}}\,\sigma_{\text{prior}}^2 + M_{2,n} + \frac{\nu_{0,\text{miss}}\,n}{\nu_{0,\text{miss}} + n}(\bar{x}_n - \mu_{\text{prior}})^2}{\nu_{0,\text{miss}} + n}$$
  $$z_{\text{miss,obs}}(x_{\text{new}}) = \frac{x_{\text{new}} - \hat{\mu}_n}{\sqrt{\hat{\sigma}_n^2\!\left(1 + \frac{1}{\nu_{0,\text{miss}} + n}\right) + \epsilon_{\sigma}^2}}$$
  where:
  - $x_i = 1 - \max(\cos(\text{reply}_i, \text{act}_i), \cos(\text{reply}_i, \text{expected}_i))$ is the content-miss score (R-18);
  - $\nu_{0,\text{miss}}$ is the prior pseudo-count (start value $10$);
  - $\epsilon_{\sigma} > 0$ is a numerical floor (start value $0.05$) preventing division by near-zero variance if all early replies are identical;
  - $n_{\min,\text{miss}}$ (start value $20$) is the threshold above which the place's own statistics dominate (and optionally switch to Quesenberry's exact $Q_r = \Phi^{-1}(G_{r-2}(T_r))$ once $r \ge n_{\min,\text{miss}}$, where $t^{-1}(\Phi(2), df=18) \approx 2.15$, provided approximate normality holds).
- **Body scope:** Applies only where events carry embeddings (chat/text/multimodal). For unembedded scalar sensors, content miss stays neutral (only time misses and explicit negative flags apply).

### 2. Per-place latency quantiles (R-08)

- **Minimum sample size:** By distribution-free order statistics (Thompson 1936 [PR]; Murphy 1948 [PR]; Meeker, Hahn & Escobar 2017 [PR]), for $P(X_{(n)} \ge \xi_q) = 1 - q^n \ge 1 - \alpha$:
  - Median ($q_p = 0.5$): $n \ge 6$ gives the minimal two-sided 95% CI $[X_{(1)}, X_{(6)}]$ ($96.875\%$ coverage); $n \ge 25$ achieves binomial rank SE $\le 0.10$.
  - 90th percentile ($q_e = 0.9$): $n \ge \lceil \ln(0.05)/\ln(0.90) \rceil = \mathbf{29}$ observations are required just so the sample maximum covers $\xi_{0.90}$ with 95% probability (at $n = 10$, coverage is only $1 - 0.9^{10} = 65.1\%$).
- **Rule & formula:** Fix the `core-and-lab` capability/place mismatch by letting a body/place declare a place-level `typical_latency` $L_{\text{typ}}(P)$ (falling back to the median `typical_latency` of capabilities bound to $P$, or a global default $L_{\text{default}}$ if none is declared). Define a log-normal prior CDF $F_0(x \mid L_{\text{typ}}(P), \sigma_{0,\ln})$ on positive latencies (justified by heavy-tailed CMC response times; Barabási 2005 [PR]; Malmgren et al. 2008 [PR]):
  $$F_0(x) = \Phi\!\left(\frac{\ln \max(x, L_{\min}) - \ln L_{\text{typ}}(P)}{\sigma_{0,\ln}}\right)$$
  Using the bounded log-bucketed DDSketch estimator (defined in A-08) with observed bucket weight $W_P = \sum_i w_i$ and cumulative weight $W_{\le x} = \sum_{\gamma^i \le x} w_i$, evaluate the posterior predictive CDF for **every $n \ge 0$**:
  $$\hat{F}_P(x) = \frac{n_0\,F_0(x) + W_{\le x}}{n_0 + W_P}, \qquad \hat{\xi}_q(P) = \inf\{x : \hat{F}_P(x) \ge q\}$$
  At $n = 0$, $\hat{F}_P = F_0$, so $\text{patience}(P) = L_{\text{typ}}(P)$ and $\text{expiry}(P) = L_{\text{typ}}(P)\exp(1.2816\,\sigma_{0,\ln})$ (e.g. $\approx 3.60\,L_{\text{typ}}(P)$ at $\sigma_{0,\ln} = 1.0$, avoiding the `core-and-lab` collapse where $\text{expiry}_0 = \text{patience}_0$). By $n = 29$, empirical observations carry $29/(10+29) = 74.4\%$ weight for $q_p$ (or $29/(20+29) = 59.2\%$ if $n_{0,e} = 20$ for $q_e$).
- **Body scope:** Generic across all bodies; bodies with fixed deterministic hardware round-trips (e.g., robotic actuators) simply declare small $\sigma_{0,\ln}$ (or rely on declared `effect` windows in R-09).

### 3. 95th-percentile inter-event gap without vectors (R-23)

- **Minimum sample size:** By Murphy's coverage bound ($1 - 0.95^n \ge 0.95$), estimating the 95th percentile nonparametrically requires at least:
  $$n_{\min,\text{gap}} = \left\lceil \frac{\ln(0.05)}{\ln(0.95)} \right\rceil = \mathbf{59}\text{ inter-event gaps in place } P$$
  (or $\ge 100$ gaps for a non-degenerate two-sided 95% CI). With fewer than $59$ gaps, raw empirical $q_{0.95}$ either under-covers (splitting coherent conversations prematurely) or collapses to the max gap seen so far.
- **Hierarchical pooling across places (scale-normalized for multi-body generality):** Raw gaps cannot be pooled directly across bodies operating at milliseconds (sensors) vs hours (email). Instead, normalize every observed inter-event gap $\Delta t_{P,j} = t_{P,j} - t_{P,j-1}$ by the place's scale anchor $L_{\text{typ}}(P)$:
  $$u_{P,j} = \frac{\Delta t_{P,j}}{\max(L_{\text{typ}}(P), L_{\min})}$$
  Pool normalized ratios $u$ globally across all places in the soul into one shared ratio DDSketch $\hat{F}_{\text{ratio}}(u)$ (capturing burstiness across the soul's traffic; Barabási 2005 [PR]), anchored by a baseline log-normal ratio prior $G_0(u) = \Phi((\ln u)/\sigma_{0,\text{gap}})$ with weight $n_{0,\text{global}}$ (start value $20$). Place $P$'s gap CDF is then:
  $$\hat{F}_{\text{gap},P}(\Delta t) = \frac{n_{0,\text{gap}}\,\hat{F}_{\text{ratio}}\!\big(\Delta t / L_{\text{typ}}(P)\big) + W_{\text{gap},P}(\le \Delta t)}{n_{0,\text{gap}} + W_{\text{gap},P}}$$
  and the boundary gap threshold is $\Delta t_{0.95}(P) = \inf\{\Delta t : \hat{F}_{\text{gap},P}(\Delta t) \ge 0.95\}$, bounded below by $T_{\text{idle}}$ or $2 \cdot \text{patience}(P)$ so short synchronous pauses do not split an ongoing turn.

### 4. Calibrated values of R-03 (`fok_threshold`, `fok_scale`, `wake_gate_lo`) on a brand-new soul or embedder

Recent 2026 agent-memory research confirms two cold-start facts directly relevant here:
- Pre-deployment/cold-start memory pipelines cannot rely on uncalibrated global confidence bars across heterogeneous assertion types or models (Owolabi, Gupta & Wang 2026, arXiv:2610.07100 [PP]; Cao et al. 2026, arXiv:2605.14186 [PP], who fit per-model decision boundaries rather than universal constants), while deterministic CPU-first memory scoring still relies on explicit logistic-midpoint calibration (Stabile & Zimuel 2026, arXiv:2606.03463 [PP]).
- Transformer embeddings exhibit model-specific anisotropy — nonzero baseline cosine similarities between unrelated texts (Ethayarajh 2019 [PR]) — so `wake_gate_lo` cannot transfer blindly across embedders.
- Adaptive resource/budget allocation across unknown horizons requires carrying calibrated baseline pressure without prior horizon knowledge (Zong et al. 2026, arXiv:2610.24955 [PP]; Choi et al. 2026, arXiv:2605.13880 [PP]).

**Cold-start rules and sample-size requirements:**

1. **Before any events exist ($n_{\text{events}} = 0$):**
   - `feeling_of_knowing = 0.0` (no recall candidates exist; neutral zero knowledge).
2. **`wake_gate_lo` (and bounds) when an embedder has no dev-set calibration:**
   - *Sample size needed:* Estimating the 95th percentile with 95% coverage requires $m \ge \lceil \ln(0.05)/\ln(0.95) \rceil = 59$ **independent disjoint** unrelated pairs ($\text{SE}(\hat{q}) \le 0.02 \implies m \ge 119$ disjoint pairs, i.e. $\ge 238$ uncorrelated events; all-pairs $\binom{N}{2}$ estimates have correlated U-statistic variance). Estimating R-03's **99.5th percentile** upper bound nonparametrically requires $m \ge 598$ independent disjoint pairs ($\ge 1{,}196$ events) — meaning `core-and-lab` should compute the 99.5th upper bound via empirical-Bayes normal tail $\hat{\mu}_{\cos} + 2.576\,\hat{\sigma}_{\cos}$ (or $m \ge 600$ unrelated pairs) when fewer pairs exist!
   - *Cold-start behavior ($m_{\text{pairs}} < m_{\min,\text{cos}}$):* Maintain an online buffer/DDSketch (or Welford mean/SD) of cosine similarities between non-adjacent pairs of incoming events (from distinct places/episodes, separated by $\ge 10$ turns) per `embedder_tag`, shrunk with pseudo-count $n_{0,\text{cos}} = 30$ toward a conservative isotropic/multilingual prior $(\mu_{0,\cos} = 0.15, \sigma_{0,\cos} = 0.12) \implies \text{prior } p_{95} \approx 0.35$:
     $$\text{wake\_gate\_lo}_{\text{cold}} = \text{clamp}\!\big(\hat{\mu}_{\cos} + 1.6449\,\hat{\sigma}_{\cos},\; 0.20,\; 0.85\big)$$
     Once $m_{\text{pairs}} \ge m_{\min,\text{cos}} = 120$ disjoint pairs (or when `monoagens-lab calibrate` ran on the dev set for that `embedder_tag`), switch smoothly to the empirical 95th percentile within calibrated `[80th, 99.5th]` bounds (and wake-band learning then refines it).
3. **`fok_threshold` and `fok_scale` on a brand-new soul / uncalibrated embedder:**
   - *Sample size needed:* For the median (`q=0.5`), $n \ge 25\text{–}100$ probe queries yield binomial rank SE $0.10\text{–}0.05$; stabilizing the IQR (`Q_{0.75} - Q_{0.25}`) within $\sim \pm 20\%$ requires $n_{\min,\text{fok}} = 50\text{–}100$ probes (start value $n_{\min,\text{fok}} = 50$).
   - *Fixing the $B$-drift bug during cold start:* In a new soul, $B$ is artificially elevated ($+2.05$ at $t=1\text{ min}$, vs $-3.29$ at 30 days) for recent events. Track an online quantile sketch (or reservoir) of the first $n_p$ observed $A^*$ probe scores in the soul, shrunk with pseudo-count $n_{0,\text{fok}} = 30$ toward an analytical cold-start prior centered on short-run $B_0$ plus prior unrelated cosine $\mu_{0,\cos}$:
     $$\text{fok\_threshold}_{\text{cold}} = \frac{n_{0,\text{fok}}\,A_{0}^* + \sum_{j=1}^{n_p} A_j^*}{n_{0,\text{fok}} + n_p} \quad \text{(or shrunk median)}, \qquad \text{fok\_scale}_{\text{cold}} = \max\!\left(\frac{\text{IQR}_{\text{shrunk}}(A^*)}{2.7},\; s_{\min}\right)$$
     where $A_0^* = w_{\text{ctx}}(\mu_{0,\cos} + 2\sigma_{0,\cos}) + w_{\text{imp}}\cdot 0.25 + B_{\text{med}}$ (start value $0.8$, assuming short-run context; tune in Lab) and $s_{\min} = 0.25$. Even better (recommended change after `core-and-lab`): calibrate `fok_threshold` separately for working-age vs long-horizon $B$ or compute FOK over `(cos, normalized_B)` rather than raw unbounded $A^*$.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---:|---|---|
| $\nu_{0,\text{miss}}$ | Prior weight (pseudo-count) for R-18 content miss | `10` | `3–30` | tune in Lab (Quesenberry 1991 shrinkage analogue) |
| $n_{\min,\text{miss}}$ | Unrepaired replies before place stats dominate / Q-chart | `20` | `10–30` | Hillier (1969); $\chi^2$ variance CI |
| $\mu_{0,\text{miss}}, \sigma_{0,\text{miss}}$ | Embedder-level fallback mean & SD of `miss` | `0.50`, `0.20` | `0.3–0.7`; `0.1–0.35` | tune in Lab per embedder |
| $\epsilon_{\sigma}$ | Variance floor preventing division by zero in $z_{\text{miss}}$ | `0.05` | `0.01–0.15` | tune in Lab |
| $L_{\min}, L_{\text{default}}$ | Latency lower clamp & global default if body omits `typical_latency` | `1 ms`, `5 s` | `1–100 ms`; `0.5–60 s` | tune in Lab |
| $\sigma_{0,\ln}$ | Prior log-SD for body latency distribution (R-08) | `1.0` | `0.5–2.0` | tune in Lab (log-normal CMC prior) |
| $n_{0,\text{global}}$ | Prior weight for global ratio DDSketch $\hat{F}_{\text{ratio}}$ | `20` | `10–50` | tune in Lab |
| $n_{0,\text{gap}}$ | Prior weight for per-place 95th-pct gap $\hat{F}_{\text{gap},P}$ (R-23) | `30` | `15–60` | tune in Lab |
| $n_{\min,\text{gap}}$ | Gaps per place for 95% coverage of $q_{0.95}$ | `59` | `45–100` | Murphy (1948); $\lceil\ln 0.05/\ln 0.95\rceil$ |
| $\sigma_{0,\text{gap}}$ | Log-SD of normalized inter-event gap prior | `1.5` | `0.8–2.5` | tune in Lab |
| $m_{\min,\text{cos}}$ | Disjoint unrelated pairs before empirical `wake_gate_lo` | `120` | `59–600` | Murphy (1948) / binomial SE |
| $n_{0,\text{cos}}$ | Prior weight for online `wake_gate_lo` fallback | `30` | `10–100` | tune in Lab |
| $\mu_{0,\cos}, \sigma_{0,\cos}$ | Fallback unrelated-pair cosine mean & SD | `0.15`, `0.12` | `0.0–0.4`; `0.05–0.25` | tune in Lab per embedder |
| $n_{\min,\text{fok}}$ | Probe count before empirical median/IQR of $A^*$ | `50` | `25–100` | Thompson (1936) / quantile precision |
| $n_{0,\text{fok}}, A_0^*, s_{\min}$ | Cold-start prior weight, center, and minimum FOK scale | `30`, `0.8`, `0.25` | `10–100`; `0.0–2.0`; `0.1–1.0` | tune in Lab |

## Neutral value

- **R-18:** Before $n \ge n_{\min,\text{miss}}$, if fallback is disabled (`ν_0 → ∞` with `μ_0 = 1`), no automatic z-score miss fires (only explicit negative/repair flags from the body fire).
- **R-08 / R-23:** At $n = 0$, durations equal the body/global prior (`F_0` / `G_0`). Setting `n_0 → ∞` locks estimates permanently at the declared prior (R-08's neutral value).
- **R-03:** At $n_{\text{events}} = 0$, `feeling_of_knowing = 0.0`; with `w_fok = 0` or `wake_gate_lo = 1.0`, the respective gates never fire on similarity/FOK.

## Claim

Without cold-start shrinkage and sample-size guards:
1. Content miss either never triggers (`n ≤ 5` under Samuelson's bound) or fires false alarms on early identical replies (`s ≈ 0`).
2. `patience(P)` and `expiry(P)` cannot initialize cleanly before a capability runs in `P` or collapse onto the same point value at $n = 0$.
3. Vector-free episode segmentation over-splits early conversations before ~59 gaps accumulate.
4. A brand-new soul or uncalibrated embedder either saturates `feeling_of_knowing` near 1.0 (excessive wakes on trivial events) or breaks `wake_gate_lo` (flooding `classify` calls or missing every free-text condition).

## How the Lab would test it

1. **Cold-start trajectory suite (first 1–100 events):** Run new souls initialized at $t = 0$ across (a) fast synchronous chat (seconds), (b) slow asynchronous messaging (hours), and (c) an uncalibrated new embedder (e.g. `hash` vs `openai-compatible` vs `EmbeddingGemma-300M`).
2. Compare naive unguarded formulas vs hierarchical shrinkage across events $1..100$: measure false content-miss alarm rate on normal replies ($n \le 20$), false episode split rate ($n \le 60$), false wake rate from `feeling_of_knowing` ($n \le 50$), and `classify` calls / missed condition rate before dev calibration.
3. Verify determinism: replay logs from event 1 produce identical cold-start transitions bit-for-bit.

## Established / inferred / open

- **Established:** Samuelson's bound $|z| \le (n-1)/\sqrt{n}$ (Samuelson 1968 [PR]); Quesenberry's self-starting $Q_r$ Student-$t$ transform (Quesenberry 1991 [PR], 1995 [PR]); Phase-I variance instability below $n \approx 20\text{–}30$ (Hillier 1969 [PR]); distribution-free sample sizes $n \ge \lceil \ln \alpha / \ln q \rceil$ for tail quantile coverage (Thompson 1936 [PR]; Murphy 1948 [PR]; Meeker et al. 2017 [PR]); embedding anisotropy across models (Ethayarajh 2019 [PR]); category/model-specific calibration beating fixed global thresholds in cold-start agent memory (Owolabi et al. 2026 [PP]; Cao et al. 2026 [PP]).
- **Inferred:** The conjugate Normal-Inverse-Gamma predictive formula for content miss, normalized ratio pooling across bodies for R-23's 95th-percentile gap, and the online shrunk fallbacks for R-03 (`fok_threshold`, `fok_scale`, `wake_gate_lo`).
- **Open:** Exact prior weights ($\nu_{0,\text{miss}}, n_{0,\text{gap}}, n_{0,\text{cos}}, n_{0,\text{fok}}$), whether `fok_threshold` should be reformulated over age-separated activation rather than raw $A^*$, and empirical values per embedder in the Lab.

## Sources

- Samuelson, P. A. (1968). How Deviant Can You Be? *Journal of the American Statistical Association*, 63(324), 1522–1525. DOI: `10.1080/01621459.1968.10480944` **[PR]**
- Quesenberry, C. P. (1991). SPC Q Charts for Start-Up Processes and Short or Long Runs. *Journal of Quality Technology*, 23(3), 213–224. DOI: `10.1080/00224065.1991.11979327` **[PR]**
- Quesenberry, C. P. (1995). On Properties of Q Charts for Variables. *Journal of Quality Technology*, 27(3). DOI: `10.1080/00224065.1995.11979592` **[PR]**
- Hillier, F. S. (1969). $\bar{X}$- and $R$-Chart Control Limits Based on a Small Number of Subgroups. *Journal of Quality Technology*, 1(1), 17–26. DOI: `10.1080/00224065.1969.11980343` **[PR]**
- Welford, B. P. (1962). Note on a Method for Calculating Corrected Sums of Squares and Products. *Technometrics*, 4(3), 419–420. DOI: `10.1080/00401706.1962.10490022` **[PR]**
- Thompson, W. R. (1936). On Confidence Ranges for the Median and Other Expectation Distributions for Populations of Unknown Distribution Form. *Annals of Mathematical Statistics*, 7(3), 122–128. DOI: `10.1214/aoms/1177732502` **[PR]**
- Murphy, R. B. (1948). Non-Parametric Tolerance Limits. *Annals of Mathematical Statistics*, 19(4), 581–589. DOI: `10.1214/aoms/1177730154` **[PR]**
- Meeker, W. Q., Hahn, G. J., & Escobar, L. A. (2017). *Statistical Intervals: A Guide for Practitioners and Researchers* (2nd ed.). Wiley. DOI: `10.1002/9781118594841` **[PR]**
- Ethayarajh, K. (2019). How Contextual are Contextualized Word Representations? Comparing the Geometry of BERT, ELMo, and GPT-2 Embeddings. *EMNLP-IJCNLP 2019*. DOI: `10.18653/v1/D19-1006` / `arXiv:1909.00512` **[PR]**
- Owolabi, O., Gupta, P., & Wang, F. (2026). When to Remember, When to Abstain: Category-Conditioned Retention for Reliable Agent Memory. `arXiv:2610.07100` **[PP]**
- Zong, Y., Piao, S., Jiang, J., & Tsung, F. (2026). MemPrice: Online Memory Allocation via Adaptive Pricing for Long-Horizon Agents. `arXiv:2610.24955` **[PP]**
- Choi, Y., Park, S., Kang, M., Baek, J., & Hwang, S. J. (2026). PREPING: Zero-Task Agent Memory Construction via Self-Practice. `arXiv:2605.13880` **[PP]**
- Stabile, M., & Zimuel, E. (2026). DMF: A Deterministic Memory Framework for Conversational AI Agents. `arXiv:2606.03463` **[PP]**
- Cao, Q., Wang, Y., Qin, P., Zhang, S., & Xie, P. (2026). LLMs Know When They Know, but Do Not Act on It: A Metacognitive Harness for Test-time Scaling. `arXiv:2605.14186` **[PP]**
- Barabási, A.-L. (2005). The origin of bursts and heavy tails in human dynamics. *Nature*, 435, 207–211. DOI: `10.1038/nature03459` **[PR]**
- Malmgren, R. D., Stouffer, D. B., Motter, A. E., & Amaral, L. A. N. (2008). A Poissonian explanation for heavy tails in e-mail communication. *PNAS*, 105(47), 18153–18158. DOI: `10.1073/pnas.0800332105` **[PR]**

---

# A-08 The estimator in R-08

**Status:** partly answered (estimator and log-latency established; stakes factor **not settled by the literature**)

## Answer

Use a **bounded log-spaced bucket histogram (DDSketch, Masson, Rim & Lee 2019 [PR]) with optional exponential decay and analytical log-normal prior blending** as the single latency quantile estimator for R-08 (`patience`, `expiry`, `silence_window`) and R-23 (`95th-percentile gap`). Latency is modeled on **log-latency** via geometric buckets because human response latencies are heavy-tailed across orders of magnitude, and DDSketch gives exact determinism, order-invariance, $O(1)$ bounded memory ($\sim 2\text{ KB}$ per place), $\le \alpha$ multiplicative error across all quantiles, and exact $n_0$ pseudo-observation blending without storing past samples. For the **stakes factor** on `expiry`, Clark & Brennan (1991) [PR] and Paek & Horvitz (2000) [PR] establish only qualitatively that the grounding criterion varies with fault cost vs. delay cost; **the functional form and numerical range of the stakes factor are not settled by the literature** and must be tuned in the Lab (with neutral value $1.0$).

### 1. Comparison of Streaming Quantile Estimators

| Estimator | Space / Place | Deterministic & Replayable? | Order-Invariant / Mergeable? | Heavy-Tail / Multi-Scale Accuracy | Handles Non-Stationarity? | Clean $n_0$ Prior Shrinkage? |
|---|---|---|---|---|---|---|
| **P²** (Jain & Chlamtac 1985 [PR]) | $O(1)$ (5 markers/quantile, ~80 B) | Deterministic on fixed sequence, **no arbitrary $q$** | **No** (strictly order-sensitive; non-mergeable) | **Poor** on skewed/heavy tails; breaks for $n < 5$ | **Poor** (marker step inertia lags regime shifts) | Awkward (must fake 5 initial markers) |
| **t-digest** (Dunning & Ertl 2019 [PP]; Dunning 2021 [PR]) | $O(\delta)$ (~100–200 centroids, 2–3 KB) | Sensitive to flush order & `asin`/`sin` libm | **No** (not strictly associative) | **No worst-case guarantee:** Cormode et al. (2021 [PR]) show arbitrarily large error on skewed/multi-scale data unless log-transformed | Needs explicit windowing/reset | Possible via weighted prior centroids, but loses log-space symmetry |
| **KLL sketch** (Karnin, Lang & Liberty 2016 [PR]) | $O(\frac{1}{\varepsilon}\sqrt{\log\frac{1}{\delta}})$ (or $O(\frac{1}{\varepsilon}\log(\varepsilon n))$ det.) | Randomized needs seeded PRNG; det. grows $\log n$ | Mergeable | **Uniform rank error** $\pm \varepsilon n$; large value error in tails ($q = 0.9, 0.95$) | Needs external sliding window | Awkward (discrete compactor levels) |
| **Tierney (1983) / Robbins-Monro SA** ([PR]) | $O(1)$ (~24 B per quantile) | Deterministic on fixed sequence | **No** (strictly order-sensitive) | **Slow tail convergence** at $q = 0.9, 0.95$ unless log-transformed; sensitive to density bandwidth $h_n$ | $O(1/n)$ freezes; constant $\eta$ adds steady-state noise | No exact $n_0$ Bayesian sample-weight semantics |
| **Bounded sliding window** (circular buffer of size $W$) | $O(W)$ (e.g. $W = 256 \implies 2\text{ KB}$) | **100% deterministic**, bit-exact | Order-invariant within last $W$ | **Exact** empirical order statistics inside window | **Native** (forgets $> W$ turns automatically) | **Exact** weighted empirical CDF vs $F_0$ |
| **DDSketch (log-bucket histogram)** (Masson et al. 2019 [PR]) | **Fixed $O(m)$** (e.g. $242$ buckets for $\alpha = 0.05$ over $1\text{ ms}\text{–}365\text{ d} \implies \sim 1.9\text{ KB}$) | **100% deterministic**, integer index $\lceil \log_\gamma x \rceil$, zero RNG | **Yes** (commutative & associative bucket sums) | **Provable $(1 \pm \alpha)$ relative value error** across arbitrary positive heavy tails (collapsing lower bins protects upper tails) | **Native via decay factor** $\rho_{\text{lat}} \in (0.99, 1]$ per observation | **Exact analytical blend** with continuous log-normal CDF $F_0$ |

*Note on Cormode et al. (2021 [PR], `arXiv:2102.09299`) and ReqSketch (Cormode et al. 2023 [PR], `arXiv:2004.01668`):* ReqSketch achieves formal relative-*rank* error guarantees where t-digest can fail, but uses randomness and grows in space as $O(\frac{1}{\varepsilon}\log^{1.5}(\varepsilon n))$. By contrast, on strictly positive durations (`latency > 0`), DDSketch achieves relative-*value* error $|\tilde{x}_q - x_q| \le \alpha x_q$ in fixed bounded memory (Masson et al. 2019 [PR]). (If desired as an ablation check in Lab, a bounded sliding window of $W = 256$ `f64` log-latencies gives exact order statistics over the most recent 256 replies in identical 2 KB memory.)

### 2. Why Latency Is Modeled on Log-Latency and How $n_0$ Shrinkage Works

- **Log-latency rationale:** Empirical human response latencies in email, chat, and interactive systems are strongly right-skewed and heavy-tailed (Barabási 2005 [PR]; Malmgren et al. 2008 [PR]; Avrahami & Hudson 2006 [PR]; Kalman et al. 2006 [PR]; Kalman & Rafaeli 2011 [PR]; Stivers et al. 2009 [PR]). While quantiles themselves are equivariant under $\ln(\cdot)$, **priors, centroid averaging, and step-size scaling** are not: arithmetic averages across milliseconds-to-days explode toward extreme outliers. Modeling the prior on log-latency $\ln L \sim \mathcal{N}(\ln L_{\text{typ}}(P), \sigma_{0,\ln}^2)$ aligns the prior median directly with $L_{\text{typ}}(P)$ and gives a realistic right-skewed tail ($F_0$ median $= L_{\text{typ}}$, 90th percentile $= L_{\text{typ}} \exp(1.2816\,\sigma_{0,\ln})$).
- **Exact DDSketch + $n_0$ shrinkage formulas:**
  1. Choose relative accuracy $\alpha_{\text{lat}}$ (start value $0.05$) so $\gamma = (1 + \alpha_{\text{lat}})/(1 - \alpha_{\text{lat}}) \approx 1.10526$, over clamped latency range $[L_{\min}, L_{\max}] = [1\text{ ms}, 365\text{ days}]$ (fixed array of $B = \lceil \log_\gamma(L_{\max}/L_{\min}) \rceil = 242$ buckets, indexed $k = \text{clamp}(\lceil \log_\gamma(\max(L, L_{\min})/L_{\min}) \rceil,\, 0,\, B-1)$; bucket upper bound $u_k = L_{\min}\gamma^k$, midpoint $\tilde{x}_k = \frac{2\,u_k}{\gamma + 1}$).
  2. On each observed latency $L > 0$ in place $P$, update bucket weights $w_0, \dots, w_{B-1}$ (starting at $0$):
     $$w_j \leftarrow \rho_{\text{lat}}\,w_j \quad (\forall j), \qquad w_k \leftarrow w_k + 1$$
     where $\rho_{\text{lat}} \in (0.99, 1.0]$ is a deterministic per-observation forgetting factor handling non-stationarity (setting $\rho_{\text{lat}} = 1.0$ yields exact unweighted history; $\rho_{\text{lat}} = 0.995$ gives effective sample horizon $1/(1-\rho_{\text{lat}}) = 200$). Total observed weight is $W_P = \sum_{j=0}^{B-1} w_j$.
  3. Define the shrunk posterior CDF at bucket boundary $u_k$:
     $$\hat{F}_P(u_k) = \frac{n_0\,F_0(u_k) + \sum_{j \le k} w_j}{n_0 + W_P}, \qquad F_0(x) = \Phi\!\left(\frac{\ln \max(x, L_{\min}) - \ln L_{\text{typ}}(P)}{\sigma_{0,\ln}}\right)$$
  4. For target quantile $q \in \{q_p, q_e, 0.95\}$, let $k^*$ be the smallest bucket index with $\hat{F}_P(u_{k^*}) \ge q$. Interpolate linearly within bucket $k^*$ (or return $\tilde{x}_{k^*} = 2 u_{k^*}/(\gamma+1)$ when $\alpha_{\text{lat}} \le 0.05$):
     $$\hat{\xi}_q(P) = \frac{2\,u_{k^*}}{\gamma + 1}$$
     (Note: at $W_P = 0$, $k^*$ finds the bucket containing $F_0^{-1}(q)$ directly within $\pm \alpha_{\text{lat}}$!)

### 3. Stakes Factor on `expiry(P)` (Clark & Brennan 1991; Paek & Horvitz 2000)

- **What grounding literature establishes:** Clark & Brennan (1991 [PR]) establish that partners establish mutual belief to a **grounding criterion** sufficient for current purposes, trading **fault cost** (cost of ungrounded error/misunderstanding) and **repair cost** against **delay cost** (cost of pausing/waiting) under the Principle of Least Collaborative Effort. Paek & Horvitz (2000 [PR]) formalize grounding as **expected utility under uncertainty**, weighting fault vs. wait costs.
- **What is NOT settled by the literature:** Neither source specifies a multiplicative "stakes factor" on expectation timeout `expiry(P)`, nor whether higher stakes should **lengthen** `expiry` (patiently awaiting explicit confirmation before giving up/acting on absence, matching high fault cost when delay cost is low) or **shorten** `expiry` (escalating/checking sooner when delay/deadline risk dominates).
- **Parameterized rule (for Lab testing; neutral = `1.0`):** Compute stakes ratio from the existing `think` fields (`cost`, `value` from R-15, defaulting to `cost = value = 1.0` when omitted, with floor $\epsilon_v = 0.01$ preserving `ARCHITECTURE.md` invariants without reading text):
  $$\text{expiry}(P) = \hat{\xi}_{q_e}(P) \cdot f_{\text{stakes}}, \qquad f_{\text{stakes}} = \text{clamp}\!\left(\left(\frac{\max(\text{cost}, \epsilon_v)}{\max(\text{value}, \epsilon_v)}\right)^{\eta_s},\; f_{\min},\; f_{\max}\right)$$
  and if linked to a promise/goal with due time $t_{\text{due}}$, cap at $\max(L_{\min}, t_{\text{due}} - t_{\text{now}})$. Setting $\eta_s = 0$ yields neutral $f_{\text{stakes}} = 1.0$.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---:|---|---|
| $\alpha_{\text{lat}}$ | DDSketch relative error tolerance | `0.05` | `0.01–0.10` | Masson et al. (2019); tune in Lab |
| $L_{\min}, L_{\max}$ | Bucket floor and ceiling | `1 ms`, `365 d` | `[1 ms, 1 s]`; `[30 d, 3650 d]` | Masson et al. (2019); body-generic |
| $\rho_{\text{lat}}$ | Per-observation recency decay ($1.0$ = no forgetting) | `0.995` | `0.98–1.0` | tune in Lab |
| $\sigma_{0,\ln}$ | Log-normal spread of body prior $F_0$ | `1.0` | `0.5–2.0` | tune in Lab |
| $n_0$ | Prior pseudo-observation weight | `10` (or `20` for $q_e$) | `3–50` | R-08; tune in Lab |
| $\epsilon_v$ | Floor preventing division by zero on `cost`/`value` | `0.01` | `1e-4–0.1` | tune in Lab |
| $\eta_s$ | Stakes-ratio exponent (not settled by literature) | `0.0` | `-0.5–+0.5` | **not settled**; tune in Lab |
| $f_{\min}, f_{\max}$ | Bounds on stakes multiplier $f_{\text{stakes}}$ | `0.5`, `2.0` | `0.25–1.0`; `1.0–4.0` | **not settled**; tune in Lab |

## Neutral value

- Estimator neutral: $n_0 \to \infty$ (or $W_P$ ignored) keeps durations fixed at the body prior (R-08 neutral value).
- Stakes factor neutral: $\eta_s = 0.0 \implies f_{\text{stakes}} = 1.0$ (stakes do not modify `expiry`).

## Claim

1. Compared to P² and raw-value t-digest, log-spaced DDSketch prevents multi-scale heavy-tail skew and order-dependent drift in `patience(P)` and `expiry(P)` at $< 2\text{ KB}$ per place while preserving exact bit-for-bit log replay.
2. Whether $\eta_s > 0$ (longer patience for high-fault-cost acts) or $\eta_s < 0$ (faster escalation on critical acts) improves missed-absence vs. premature-follow-up rate is an empirical claim to test in the Lab.

## How the Lab would test it

1. **Estimator accuracy & replay benchmark:** Generate synthetic heavy-tailed latency streams (log-normal $\sigma_{\ln} \in \{0.8, 1.5, 2.2\}$ and multimodal ms-to-hours streams, plus a regime shift from 5 s to 4 h at observation $t = 200$). Compare DDSketch ($\rho_{\text{lat}} = 1.0$ and $0.995$), bounded sliding window ($W = 256$), P², t-digest, and Tierney (1983) against exact offline quantiles on relative error at $q = 0.5, 0.9, 0.95$, adaptation lag after shift, memory bytes, and exact replay determinism.
2. **Stakes factor sweep:** Sweep $\eta_s \in \{-0.5, -0.25, 0.0, +0.25, +0.5\}$ across high-fault-cost / low-delay-cost scenarios (e.g., irreversible device actions or critical confirmations with slow human replies) vs. high-urgency scenarios, measuring false "noticed absence" rate, delay to recovery, and total wakes.

## Established / inferred / open

- **Established:** DDSketch achieves provable $(1 \pm \alpha)$ relative value error, full mergeability, and bounded memory via bucket collapsing (Masson et al. 2019 [PR]); t-digest can suffer arbitrarily large errors on multi-scale skewed streams (Cormode et al. 2021 [PR]); KLL targets additive rank error rather than tail value accuracy (Karnin et al. 2016 [PR]); P² fails on small $n$ and lags heavy-tailed shifts (Jain & Chlamtac 1985 [PR]); Tierney (1983) achieves asymptotic variance optimality in $O(1)$ space on stationary streams but relies on local density scaling (Tierney 1983 [PR]); human CMC latencies are heavy-tailed/right-skewed (Barabási 2005 [PR]; Malmgren et al. 2008 [PR]; Kalman et al. 2006 [PR]; Avrahami & Hudson 2006 [PR]); Clark & Brennan (1991 [PR]) and Paek & Horvitz (2000 [PR]) establish qualitative fault-vs-delay grounding trade-offs.
- **Inferred:** Combining DDSketch with analytical log-normal prior CDF $F_0$ and decay $\rho_{\text{lat}}$ as one shared quantile primitive for R-08 and R-23; parameterizing $f_{\text{stakes}}$ from existing `cost`/`value` fields.
- **Open:** **Not settled by the literature:** sign and magnitude of $\eta_s$ for `expiry(P)`, and whether `patience(P)` / `expiry(P)` convergence speed benefits more from exponential decay $\rho_{\text{lat}} < 1$ or sliding window $W = 256$.

## Sources

- Masson, C., Rim, J. E., & Lee, H. K. (2019). DDSketch: A fast and fully-mergeable quantile sketch with relative-error guarantees. *PVLDB*, 12(12), 2195–2205. DOI: `10.14778/3352063.3352135` / `arXiv:1908.10693` **[PR]**
- Cormode, G., Mishra, A., Ross, J., & Veselý, P. (2021). Theory meets Practice at the Median: A Worst Case Comparison of Relative Error Quantile Algorithms. *KDD 2021*. DOI: `10.1145/3447548.3467152` / `arXiv:2102.09299` **[PR]**
- Cormode, G., Karnin, Z., Liberty, E., Thaler, J., & Veselý, P. (2023). Relative Error Streaming Quantiles. *Journal of the ACM* (and *PODS 2021*). DOI: `10.1145/3617891` / `arXiv:2004.01668` **[PR]**
- Dunning, T., & Ertl, O. (2019). Computing Extremely Accurate Quantiles Using t-Digests. `arXiv:1902.04023` **[PP]**; Dunning, T. (2021). The t-digest: Efficient estimates of distributions. *Software Impacts*, 7, 100049. DOI: `10.1016/j.simpa.2020.100049` **[PR]**
- Karnin, Z., Lang, K., & Liberty, E. (2016). Optimal Quantile Approximation in Streams. *IEEE FOCS 2016*. DOI: `10.1109/focs.2016.17` / `arXiv:1603.05346` **[PR]**
- Jain, R., & Chlamtac, I. (1985). The $P^2$ algorithm for dynamic calculation of quantiles and histograms without storing observations. *Communications of the ACM*, 28(10), 1076–1085. DOI: `10.1145/4372.4378` **[PR]**
- Tierney, L. (1983). A Space-Efficient Recursive Procedure for Estimating a Quantile of an Unknown Distribution. *SIAM J. Sci. Stat. Comput.*, 4(4), 706–711. DOI: `10.1137/0904048` **[PR]**
- Robbins, H., & Monro, S. (1951). A Stochastic Approximation Method. *Annals of Mathematical Statistics*, 22(3), 400–407. DOI: `10.1214/aoms/1177729586` **[PR]**
- Avrahami, D., & Hudson, S. E. (2006). Responsiveness in instant messaging: predictive models supporting inter-personal communication. *CHI 2006*. DOI: `10.1145/1124772.1124881` **[PR]**
- Kalman, Y. M., Ravid, G., Raban, D. R., & Rafaeli, S. (2006). Pauses and Response Latencies: A Chronemic Analysis of Asynchronous CMC. *J. Computer-Mediated Communication*, 12(1), 1–23. DOI: `10.1111/j.1083-6101.2006.00312.x` **[PR]**
- Kalman, Y. M., & Rafaeli, S. (2011). Online Pauses and Silence: Chronemic Expectancy Violations in Written Computer-Mediated Communication. *Communication Research*, 38(1), 54–69. DOI: `10.1177/0093650210378229` **[PR]**
- Stivers, T., et al. (2009). Universals and cultural variation in turn-taking in conversation. *PNAS*, 106(26), 10587–10592. DOI: `10.1073/pnas.0903616106` **[PR]**
- Clark, H. H., & Brennan, S. E. (1991). Grounding in communication. In *Perspectives on Socially Shared Cognition*, 127–149. APA. DOI: `10.1037/10096-006` **[PR]**
- Paek, T., & Horvitz, E. (2000). Conversation as Action Under Uncertainty. *UAI 2000*. DOI: `10.48550/arXiv.1301.3883` / `arXiv:1301.3883` **[PR]**
- Barabási, A.-L. (2005). DOI: `10.1038/nature03459` **[PR]**; Malmgren et al. (2008). DOI: `10.1073/pnas.0800332105` **[PR]** (cited in A-07).

---

# A-09 Recollection for automatic recall

**Status:** partly answered

## Answer

At automatic wake recall (before reasoning makes any claim), **the retrieval cue is the current wake-situation context tuple** — trigger content vector, current place, present actors, current time, and active goal — governed by **encoding specificity** (Tulving & Thomson 1973 [PR]) and **Temporal Context Model (TCM) cue-plus-context reinstatement** (Howard & Kahana 2002 [PR]). Because nothing is claimed yet at wake, **recollection ($R$) must measure episodic source recovery (trace integrity)** rather than demanding equality with the current wake's place/time, while **partial cue matches** (e.g., right people + wrong place, right place + wrong time) are handled via **conditional context-match labeling paired with a graded alignment score**, preventing both false suppression of legitimate cross-place memories and **memory-conjunction errors** (Reinitz, Lammers & Cochran 1992 [PR]; Dodson, Holland & Shimamura 1998 [PR]; Yonelinas & Jacoby 1996 [PR]). Only **explicit `memory.recall`** (R-15 / R-16), which supplies claimed target people, place, and time grain, applies R-16's strict conjunction threshold before reasoning may assert specifics.

### 1. The Wake Retrieval Cue (Encoding Specificity + TCM)

At wake on trigger event $e_{\text{trig}}$ in place $P_{\text{now}}$ at time $t_{\text{now}}$ with present audience/sender set $A_{\text{present}}$ (plus optional active goal $g_{\text{active}}$), the cue is:
$$C_{\text{wake}} = \big(v_{\text{cue}},\; P_{\text{now}},\; A_{\text{present}},\; t_{\text{now}}\big)$$
where $v_{\text{cue}} = v(e_{\text{trig}})$ (or normalized blend $(1-\beta_g) v(e_{\text{trig}}) + \beta_g v(g_{\text{active}})$ when attributed to an active goal). Following Howard & Kahana (2002 [PR]), **TCM** operates through two complementary paths already mirrored in MonoAgens's retrieval formula:
- **Drifting temporal/situational context:** ACT-R base-level activation $B$ (recency + frequency) and recent place events bias retrieval toward recent local context;
- **Item-to-context reinstatement:** vector + text candidate matching (`top 40 vector ∪ top 20 text`) uses $v_{\text{cue}}$ to reinstate an older, possibly cross-place episode's own stored context $(P_m, A_m, t_m)$ (Tulving & Thomson 1973 [PR]).

### 2. Automatic Recollection vs. Claimed Recollection (Two-Stage Architecture)

Drawing on Yonelinas (1994 [PR], 2002 [PR]), Brainerd & Reyna (2002 [PR]), Johnson, Hashtroudi & Lindsay (1993 [PR]), and Yonelinas & Jacoby (1996 [PR]):
- **Stage 1 — Automatic Wake Recall (trace-integrity recollection, no claim yet):**
  For any candidate item $m$ entering `ContextPlan` step 5:
  $$R_{\text{auto}}(m) = \begin{cases} 1 & \text{if } m \text{ is a non-forgotten, non-superseded verbatim log event, or a gist/lesson/procedure} \\ & \text{whose } \texttt{source\_ids} \text{ resolve to } \ge 1 \text{ non-forgotten verbatim log event with intact } (\text{actor}, \text{audience}, P_m, t_m) \\ 0 & \text{otherwise (unanchored gist/inference with forgotten sources, or similarity-only hit)} \end{cases}$$
  $$F(m) = \max\!\big(\cos(v_{\text{cue}}, v_m),\; \tilde{A}(m)\big)$$
  where $\tilde{A}(m) = \sigma(A_m - \text{fok\_threshold})$ (normalized activation). When $R_{\text{auto}}(m) = 0$, the item is labeled `familiar_only` (verbatim source unavailable — reasoning must not assert unverified details).
- **Stage 2 — Explicit `memory.recall` Verification (R-16 claim test):**
  When reasoning explicitly invokes `memory.recall` with target query constraints $(A_{\text{claim}}, P_{\text{claim}}, [t_{\text{lo}}, t_{\text{hi}}])$:
  $$R_{\text{claim}}(m) = \mathbf{1}\!\Big[R_{\text{auto}}(m) = 1 \;\land\; \exists\, e \in \text{sources}(m) : (\text{actor}(e) \cup \text{audience}(e)) \supseteq A_{\text{claim}} \;\land\; P(e) = P_{\text{claim}} \;\land\; t(e) \in [t_{\text{lo}}, t_{\text{hi}}]\Big]$$
  (Note: if the query leaves a facet wildcarded/unspecified, only the specified constraints are enforced; `memory-recall/spec.md` lines 82–100 must be clarified to apply this strict constraint match to claimed queries rather than rejecting cross-place items at automatic wake.)

### 3. Handling Partial Matches (Graded Score + Conditional Context Labeling)

Source monitoring (Johnson et al. 1993 [PR]), noncriterial recollection (Yonelinas & Jacoby 1996 [PR]), partial-source memory (Dodson et al. 1998 [PR]), and conjunction-error findings (Reinitz et al. 1992 [PR]) rule out **both** extremes for partial matches (e.g., *right people + wrong place*, or *right place + 30 days ago*):
- Collapsing partial cue matches to binary $R = 0$ destroys legitimate cross-place recall (`tasks.md` 11.1 `recall-across-places`).
- Passing partial matches unlabeled as generic "recollected" causes conjunction errors where reasoning assumes a memory from another place/time occurred here and now.

**Rule (graded context alignment + mandatory conditional labeling in `ContextPlan`):**
Compute a graded structural alignment score between $m$'s bound provenance $(P_m, A_m, t_m)$ (for a gist with multiple sources, the set of places/participants and time span of recovered sources) and the wake cue $C_{\text{wake}}$:
$$M_{\text{ctx}}(m, C_{\text{wake}}) = w_p\,\mathbf{1}_{P_m = P_{\text{now}}} + w_a\,J(A_m, A_{\text{present}}) + w_\tau\,(1 + \Delta t_{\text{hours}})^{-d}$$
where $w_p + w_a + w_\tau = 1$, $J(A_m, A_{\text{present}}) = |A_m \cap A_{\text{present}}| / |A_m \cup A_{\text{present}}|$ (set to $0$ if both empty), and $\Delta t_{\text{hours}} = \max(0, t_{\text{now}} - t_m)$ in hours (reusing ACT-R decay $d$).
1. **Optional ranking bonus** inside `memory::recall` (neutral $w_{\text{align}} = 0$):
   $$A_{\text{total}}(m) = B_m + w_{\text{ctx}}\cos(v_{\text{cue}}, v_m) + w_{\text{imp}}\,\text{imp}_m + w_{\text{align}}\,M_{\text{ctx}}(m, C_{\text{wake}})$$
2. **Conditional labeling in `ContextPlan` (always active when $R_{\text{auto}} = 1$, purely structural from opaque IDs & timestamps):**
   Render each recalled item with its **exact bound provenance** and **cue-match tag**:
   - `exact` if $P_m = P_{\text{now}} \land J(A_m, A_{\text{present}}) \ge \theta_A \land \Delta t \le T_{\text{recent}}$;
   - `partial(matched=[...], differs=[...])` if at least one of `{place, people, recent_time}` matches and at least one differs (e.g., `matched=[people], differs=[place=P_other, age=30d]`);
   - `remote(differs=[place, people, age])` if only semantic/activation similarity matched from a disjoint context;
   - `familiar_only` if $R_{\text{auto}}(m) = 0$.

*Body generality:* Generic across all bodies (`place`, `actor/audience`, `time` are universal protocol facts; no text inspection occurs inside core).

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---:|---|---|
| $\beta_g$ | Weight of active goal vector in wake cue $v_{\text{cue}}$ | `0.3` | `0.0–0.5` | tune in Lab (TCM context blend) |
| $w_p, w_a, w_\tau$ | Weights on place, actor Jaccard, time recency in $M_{\text{ctx}}$ | `0.35, 0.35, 0.30` | each `0.0–1.0` ($\Sigma = 1$) | tune in Lab |
| $w_{\text{align}}$ | Optional retrieval ranking weight for $M_{\text{ctx}}$ | `0.2` | `0.0–1.0` | tune in Lab |
| $\theta_A$ | Actor Jaccard threshold for `exact` context tag | `0.5` | `0.3–1.0` | tune in Lab |
| $T_{\text{recent}}$ | Recency window for `exact` vs `remote` tag | `24 h` | `1 h–7 d` | tune in Lab |

## Neutral value

- Automatic recollection neutral: treat all recalled items identically without $R_{\text{auto}}$ / `familiar_only` / `partial` provenance tags (R-16 neutral: details asserted without R test), and $w_{\text{align}} = 0$, $\beta_g = 0$.

## Claim

1. Separating trace-integrity recollection ($R_{\text{auto}}$) from claim-constrained recollection ($R_{\text{claim}}$) allows 30-day cross-place verbatim recall (`tasks.md` 11.1) to remain `recollected` rather than falsely downgraded to `familiar_only`.
2. Conditional partial-match labels (`matched=[people], differs=[place, time]`) lower **source-conjunction confabulations** (mixing who/where/when across partial matches) compared to unlabeled retrieval, while keeping `familiar_only` DRM lure rejections intact.

## How the Lab would test it

1. **Cross-place 30-day recall (`tasks.md` 11.1 + `familiar-not-recollected`):** Verify that a verbatim fact from Place A 30 days ago recalled automatically in Place B carries `recollected` + `partial(matched=[actor], differs=[place, 30d])`, whereas a gist whose source events were removed/unverified carries `familiar_only`.
2. **Conjunction-lure / partial-match scenarios:** Construct two places or episodes with overlapping people (e.g., Alice discussed Topic X in Place A yesterday; Bob and Alice are in Place B today discussing Topic X with a different constraint, or two episodes differ only in place/time). Measure source-attribution accuracy, false claims merging Place A facts into Place B, and cross-place recall hits with conditional partial-match labeling on vs. off (and sweep $w_{\text{align}} \in \{0, 0.2, 0.5\}$).

## Established / inferred / open

- **Established:** Dual-process threshold recollection vs. continuous familiarity (Yonelinas 1994 [PR], 2002 [PR]); noncriterial / partial recollection where one contextual feature is recovered without another (Yonelinas & Jacoby 1996 [PR]; Dodson, Holland & Shimamura 1998 [PR]); heuristic vs. systematic source monitoring and source binding errors (Johnson et al. 1993 [PR]); memory-conjunction illusions from recombined features (Reinitz et al. 1992 [PR]); fuzzy-trace verbatim vs. gist dissociation (Brainerd & Reyna 2002 [PR]); encoding specificity of retrieval cues (Tulving & Thomson 1973 [PR]); TCM's dual current-context and reinstated-context retrieval mechanism (Howard & Kahana 2002 [PR]).
- **Inferred:** Distinguishing $R_{\text{auto}}$ (trace integrity) from $R_{\text{claim}}$ (explicit R-16 constraint satisfaction), the $M_{\text{ctx}}$ formula, and the conditional context-tag rendering in `ContextPlan`.
- **Open:** Optimal values for $\beta_g$, $w_{\text{align}}$, $(w_p, w_a, w_\tau)$, and whether LLMs respond more accurately to concise structured tags (`partial(...)`) vs. prose annotations (testable live in Lab).

## Sources

- Yonelinas, A. P. (1994). Receiver-operating characteristics in recognition memory: Evidence for a dual-process model. *J. Exp. Psychol.: Learning, Memory, and Cognition*, 20(6), 1341–1354. DOI: `10.1037/0278-7393.20.6.1341` **[PR]**
- Yonelinas, A. P. (2002). The Nature of Recollection and Familiarity: A Review of 30 Years of Research. *Journal of Memory and Language*, 46(3), 441–517. DOI: `10.1006/jmla.2002.2864` **[PR]**
- Yonelinas, A. P., & Jacoby, L. L. (1996). Noncriterial Recollection: Familiarity as Automatic, Irrelevant Recollection. *Consciousness and Cognition*, 5(1–2), 131–141. DOI: `10.1006/ccog.1996.0008` **[PR]**
- Johnson, M. K., Hashtroudi, S., & Lindsay, D. S. (1993). Source monitoring. *Psychological Bulletin*, 114(1), 3–28. DOI: `10.1037/0033-2909.114.1.3` **[PR]**
- Dodson, C. S., Holland, P. W., & Shimamura, A. P. (1998). On the recollection of specific- and partial-source information. *J. Exp. Psychol.: Learning, Memory, and Cognition*, 24(5), 1121–1136. DOI: `10.1037/0278-7393.24.5.1121` **[PR]**
- Reinitz, M. T., Lammers, W. J., & Cochran, B. P. (1992). Memory-conjunction errors: Miscombination of stored stimulus features can produce illusions of memory. *Memory & Cognition*, 20(1), 1–11. DOI: `10.3758/BF03208247` **[PR]**
- Tulving, E., & Thomson, D. M. (1973). Encoding specificity and retrieval processes in episodic memory. *Psychological Review*, 80(5), 352–373. DOI: `10.1037/h0020071` **[PR]**
- Howard, M. W., & Kahana, M. J. (2002). A Distributed Representation of Temporal Context. *Journal of Mathematical Psychology*, 46(3), 269–299. DOI: `10.1006/jmps.2001.1388` **[PR]**
- Brainerd, C. J., & Reyna, V. F. (2002). Fuzzy-Trace Theory and False Memory. *Current Directions in Psychological Science*, 11(5), 164–169. DOI: `10.1111/1467-8721.00192` **[PR]**

---

# A-12 Domains

**Status:** partly answered

## Answer

Compute domain centroids during sleep using **warm-started, history-regularized spherical k-means** (combining Arthur & Vassilvitskii 2007 [PR] k-means++ seeding, Dhillon & Modha 2001 [PR] spherical unit-norm updates, and Chakrabarti, Kumar & Tomkins 2006 [PR] evolutionary history regularization) over active event vectors sorted by `event_id` with adaptive cold-start count $K_{\text{eff}} = \min(K, \lfloor N_{\text{vec}}/m_{\min} \rfloor)$. Preserve opaque domain IDs across sleeps via **minimum-cost bipartite (Hungarian) assignment** (Kuhn 1955 [PR]; Munkres 1957 [PR]) on cosine distance $1 - \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}})$ gated by similarity thresholds $(\tau_{\text{match}}, \tau_{\text{rel}}, \tau_{\text{merge}})$ following cluster-transition tracking (Spiliopoulou et al. 2006 [PR]; Palla, Barabási & Vicsek 2007 [PR]). Update Beta $(\alpha, \beta)$ evidence counts via **Jøsang's cumulative fusion and drift discounting** (Ismail & Jøsang 2002 [PR]; Jøsang 2016 [PR]; Gama et al. 2014 [PR]), recording all resulting centroid transitions and credence re-basing inside `domains_updated` so replay remains exact even after `vector_released` deletes old vectors.

### 1. Comparison of Deterministic Clustering Methods

| Method | Determinism & Replayability | Adaptation to Soul's Topics | Stability Across Sleep Runs | Suitability for MonoAgens |
|---|---|---|---|---|
| **Fixed codebook** (pre-computed or random unit axes) | 100% deterministic, zero sleep work | **Poor:** wastes clusters on unused topics, collides active topics into one bucket | Perfect (never moves) | Fails R-21 on specialized/multilingual souls |
| **Mini-batch / online sequential k-means** (Sculley 2010 [PR]) | Deterministic only on fixed sequence + batching; order-dependent | Adapts continuously, lower global objective quality | Moderate, drifts with recent arrivals | Unnecessary at sleep time ($N \le 10^5$, batch spherical k-means takes ms) |
| **Fresh seeded spherical k-means++** (Arthur & Vassilvitskii 2007 [PR]; Dhillon & Modha 2001 [PR]) | 100% deterministic if vectors sorted by `event_id` + seeded PRNG (`hash(seed, sleep_id)`) + fixed `max_iter` | Optimal $O(\log K)$ snapshot quality | **Low:** re-seeding from scratch jumps between local minima across sleeps | Ideal for **initial clustering** ($t = 1$) or seeding newly needed clusters |
| **Warm-started evolutionary spherical k-means** (Chakrabarti et al. 2006 [PR]; Chi et al. 2009 [PR]) | 100% deterministic (no RNG needed once warm-started; seeded k-means++ only for new slots) | Balances Snapshot Quality with History Cost | **High:** smoothly tracks gradual drift, anchors faded clusters | **Recommended for sleep updates** ($t \ge 2$) |

### 2. Deterministic Clustering & Cold-Start Rule

1. **Cold start ($N_{\text{vec}} < m_{\min}$, start value $m_{\min} = 5$):** Use a single neutral domain `d_0` (no clustering until $m_{\min}$ embedded events exist). For $N_{\text{vec}} \ge m_{\min}$, set $K_{\text{eff}} = \min(K, \lfloor N_{\text{vec}} / m_{\min} \rfloor)$.
2. **Initial sleep clustering ($K_{\text{old}} = 0$):** Sort embedded, non-faded events by `event_id`. Run seeded **spherical k-means++** (Dhillon & Modha 2001 [PR]; Arthur & Vassilvitskii 2007 [PR]) using squared chordal distance $D^2(x, \mu) = 2(1 - \cos(x, \mu))$ with deterministic seed `hash(soul_seed, sleep_event_id)` for $K_{\text{eff}}$ clusters and fixed `max_iter = 25`. If any cluster becomes empty, deterministically reassign it to the point with lowest similarity to its assigned centroid.
3. **Subsequent sleeps ($K_{\text{old}} > 0$) — Evolutionary spherical k-means (Chakrabarti et al. 2006 [PR]):**
   - Initialize the first $\min(K_{\text{old}}, K_{\text{eff}})$ centroids from the previous centroids $\mu_j^{(t-1)}$; if $K_{\text{eff}} > K_{\text{old}}$ (the soul grew out of cold start), add $K_{\text{eff}} - K_{\text{old}}$ additional seeds via deterministic k-means++ over residual distances.
   - Iterate up to `max_iter = 25`: assign each active unit vector $x_i$ to $c(i) = \arg\max_j \cos(x_i, \nu_j)$ (ties broken by lowest prior index), then update with history-regularization pseudo-count $w_{\text{hist}} \ge 0$ (neutral $0$ = pure snapshot k-means):
     $$\nu_j \leftarrow \frac{w_{\text{hist}}\,\mu_j^{(t-1)} + \sum_{c(i)=j} x_i}{\left\|w_{\text{hist}}\,\mu_j^{(t-1)} + \sum_{c(i)=j} x_i\right\|_2}$$
     (Empty clusters drop from active candidates rather than keeping dead centroids, freeing slots for emergent dense regions via one deterministic k-means++ re-seed pass if $|C_j| = 0$ and unassigned points have $\max_r \cos(x_r, \nu_r) < \tau_{\text{birth}}$.)

### 3. Stable Opaque IDs via Minimum-Cost Bipartite Matching

Given old active clusters $\{(d_i, \mu_i^{\text{old}})\}_{i=1}^{K_{\text{old}}}$ and new centroids $\{\nu_j^{\text{new}}\}_{j=1}^{K_{\text{new}}}$:
1. Build cost matrix $M \in \mathbb{R}^{K_{\text{old}} \times K_{\text{new}}}$ with $M_{ij} = 1 - \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}})$.
2. Solve minimum-cost bipartite matching using the **Hungarian (Kuhn-Munkres) algorithm** (Kuhn 1955 [PR]; Munkres 1957 [PR]) in $O(\max(K_{\text{old}}, K_{\text{new}})^3)$ (deterministic tie-breaking by `d_i` then `j`; at $K = 16$, $16^3 = 4096$ operations).
3. Classify transitions with thresholds $\tau_{\text{match}}$ (continuation), $\tau_{\text{rel}}$ (split lineage), and $\tau_{\text{merge}}$ (merge) (following Spiliopoulou et al. 2006 [PR]; Palla et al. 2007 [PR]):
   - **Continuation:** If matched $(d_i, \nu_j)$ has $s_{ij} = \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}}) \ge \tau_{\text{match}}$, $\nu_j^{\text{new}}$ **keeps opaque ID $d_i$**.
   - **Split / Birth:** For any unmatched $\nu_j^{\text{new}}$ (or match with $s_{ij} < \tau_{\text{match}}$), allocate a **new deterministic opaque `DomainId`** (monotonically or via `hash(soul_seed, sleep_event_id, j)` — never reusing retired IDs). Let $i^* = \arg\max_i \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}})$. If $\cos(\mu_{i^*}^{\text{old}}, \nu_j^{\text{new}}) \ge \tau_{\text{rel}}$, label as **split** from parent $d_{i^*}$; otherwise label as **birth**.
   - **Merge / Retirement:** For any old $d_i$ not continued above, let $j^* = \arg\max_j \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}})$ where $\nu_{j^*}^{\text{new}}$ received ID $d_{\text{surv}}$. If $\cos(\mu_i^{\text{old}}, \nu_{j^*}^{\text{new}}) \ge \tau_{\text{merge}}$, label as **merge** ($d_i \to d_{\text{surv}}$); otherwise **retire** $d_i$.

### 4. Beta $(\alpha, \beta)$ Evidence Counts Under Drift, Split, Merge, and Retirement

Let $\Delta\alpha_{a,d} = \max(0,\, \alpha_{a,d} - \alpha_{0,a})$ and $\Delta\beta_{a,d} = \max(0,\, \beta_{a,d} - \beta_{0,a})$ denote actor $a$'s accumulated evidence above its supplied prior $(\alpha_{0,a}, \beta_{0,a})$ in domain $d$ (Ismail & Jøsang 2002 [PR]; Jøsang 2016 [PR]). Because individual event vectors may be released later by `vector_released`, **apply and record credence transitions atomically in `domains_updated`**:

1. **Continuation (drift $s_{ij} = \cos(\mu_i^{\text{old}}, \nu_j^{\text{new}}) \ge \tau_{\text{match}}$):**
   Scale accumulated non-prior evidence by drift retention $s_{ij}^{\gamma_{\text{drift}}}$ (Gama et al. 2014 [PR] concept-drift forgetting; $\gamma_{\text{drift}} = 0$ keeps counts unchanged, $\gamma_{\text{drift}} > 0$ discounts toward prior as the centroid moves):
   $$\Delta\alpha'_{a,d_i} = s_{ij}^{\gamma_{\text{drift}}}\,\Delta\alpha_{a,d_i}, \qquad \Delta\beta'_{a,d_i} = s_{ij}^{\gamma_{\text{drift}}}\,\Delta\beta_{a,d_i}$$
2. **Split ($d_{\text{old}} \to \{d_{\text{old}}, d_{\text{new}}\}$):**
   - For all **active (non-faded) `evidence` events** still holding vectors, recompute exact nearest-centroid assignment and assign each active event's discounted evidence directly to its new centroid ($d_{\text{old}}$ vs $d_{\text{new}}$).
   - For any **residual faded evidence** $(\Delta\alpha^{\text{res}}, \Delta\beta^{\text{res}})$ whose vectors were released, either keep it conservatively on $d_{\text{old}}$ ($d_{\text{new}}$ starts at prior $(\alpha_{0,a}, \beta_{0,a})$ when $\rho_{\text{split}} = 0$, preventing false cross-topic generalization; Jøsang 2016 [PR]), OR split proportionally by point-mass fraction $f_{\text{split}} = \frac{|C_{\text{new}} \cap \text{prev}(d_{\text{old}})|}{|\text{prev}(d_{\text{old}})|}$ discounted by parent similarity $s_p^{\gamma_{\text{drift}}} \cdot \rho_{\text{split}}$ **without double-counting** (conservation: transferred mass is subtracted from $d_{\text{old}}$'s residual!).
3. **Merge ($d_{\text{dead}} \to d_{\text{surv}}$ with $s_m = \cos(\mu_{\text{dead}}^{\text{old}}, \nu_{\text{surv}}^{\text{new}}) \ge \tau_{\text{merge}}$):**
   Because $d_{\text{dead}}$ and $d_{\text{surv}}$ previously partitioned disjoint sets of events, their accumulated evidence counts are statistically disjoint. Apply **Jøsang's cumulative Beta fusion** (Jøsang 2016 [PR]) discounted by merge similarity:
   $$\Delta\alpha'_{a,d_{\text{surv}}} = \Delta\alpha_{a,d_{\text{surv}}} + s_m^{\gamma_{\text{drift}}}\,\Delta\alpha_{a,d_{\text{dead}}}, \qquad \Delta\beta'_{a,d_{\text{surv}}} = \Delta\beta_{a,d_{\text{surv}}} + s_m^{\gamma_{\text{drift}}}\,\Delta\beta_{a,d_{\text{dead}}}$$
4. **Retirement (unmatched $d_{\text{dead}}$ with max cosine $< \tau_{\text{merge}}$):**
   Freeze and archive $(a, d_{\text{dead}})$ (no new incoming events route to it; relax toward prior via $\lambda^\Delta$ if aged). Never transfer its evidence to an unrelated domain.
5. **Log replay invariant (`beliefs/spec.md` lines 92–99):** `domains_updated` records `{centroids: [(id, vec)], transitions: [...], credence_rebased: [...]}` so `Core::open` reproduces identical `(actor, domain)` counts from the log even after `vector_released` deletes historical event vectors.

*Body generality:* Generic across any body that produces vector embeddings (`reason`); places/bodies without vectors route to external/reasoning-supplied domain IDs or the default neutral domain `d_0`.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---:|---|---|
| $K$ | Maximum domain centroids | `16` | `4–64` | R-21; tune in Lab |
| $m_{\min}$ | Minimum vectors per active cluster (cold-start $K_{\text{eff}}$) | `5` | `3–10` | tune in Lab |
| `max_iter` | Spherical k-means iterations per sleep | `25` | `10–50` | tune in Lab |
| $w_{\text{hist}}$ | Evolutionary history pseudo-count weight | `5.0` | `0.0–20.0` | Chakrabarti et al. (2006); tune in Lab |
| $\tau_{\text{match}}$ | Min cosine for Hungarian ID continuation | `0.70` | `0.50–0.85` | tune in Lab |
| $\tau_{\text{rel}}$ | Min cosine to mark unmatched new centroid as a split | `0.50` | `0.30–0.70` | tune in Lab |
| $\tau_{\text{merge}}$ | Min cosine to merge unmatched old centroid into survivor | `0.75` | `0.60–0.90` | tune in Lab |
| $\tau_{\text{birth}}$ | Max cosine below which dense residuals seed an empty slot | `0.55` | `0.35–0.75` | tune in Lab |
| $\gamma_{\text{drift}}$ | Exponent for cosine drift discounting of $(\Delta\alpha, \Delta\beta)$ | `1.0` | `0.0–2.0` | tune in Lab |
| $\rho_{\text{split}}$ | Fraction of residual faded evidence inherited on split ($0$ = clean prior) | `0.0` | `0.0–0.5` | tune in Lab |

## Neutral value

- One global domain `d_0` for all events (R-21 neutral value: $K = 1$), or $w_{\text{hist}} = 0$, $\gamma_{\text{drift}} = 0$, $\rho_{\text{split}} = 0$.

## Claim

1. Without Hungarian ID alignment + evolutionary regularization, sleep re-clustering permutes opaque cluster indices and corrupts `(actor, domain)` reliability after sleep.
2. Without cold-start $K_{\text{eff}}$ capping and proper split/merge Beta fusion rules, early clusters degenerate when $N_{\text{vec}} < K$ and reliability either leaks across split topics or vanishes across merges and `vector_released` fading.

## How the Lab would test it

1. **Multi-topic longitudinal simulation with sleep & fading:** Script an actor who is $90\%$ accurate on Topic A (e.g., schedules) and $20\%$ accurate on Topic B (e.g., technical advice), run multiple sleep cycles across weeks (including Topic C arriving later to trigger splits/drift, and `vector_released` firing on old events).
2. Measure:
   - Domain ID stability (Adjusted Rand / persistence of domain IDs on unchanged topics across 10 sleeps);
   - Calibration error (Brier score / ECE of actor reliability predictions on Topics A, B, and newly split C) comparing: (a) unaligned fresh k-means (baseline), (b) Hungarian-aligned k-means, and (c) Hungarian + evolutionary regularization + split/merge Beta handling;
   - Exact log replay check after `vector_released`.

## Established / inferred / open

- **Established:** Seeded k-means++ $O(\log K)$ approximation (Arthur & Vassilvitskii 2007 [PR]); spherical k-means for L2-normalized text/semantic embeddings (Dhillon & Modha 2001 [PR]); mini-batch order sensitivity vs batch k-means (Sculley 2010 [PR]); evolutionary clustering balancing snapshot quality and history cost (Chakrabarti et al. 2006 [PR]; Chi et al. 2009 [PR]); optimal polynomial-time Hungarian bipartite matching (Kuhn 1955 [PR]; Munkres 1957 [PR]); cluster lifecycle transitions — continuation, split, merge, birth, death (Spiliopoulou et al. 2006 [PR]; Palla et al. 2007 [PR]); Beta Reputation cumulative fusion and discounting (Ismail & Jøsang 2002 [PR]; Jøsang 2016 [PR]); concept-drift forgetting (Gama et al. 2014 [PR]).
- **Inferred:** Combining evolutionary spherical k-means + Hungarian matching + cosine-gated Beta evidence transfer/re-basing inside `domains_updated` to preserve log replayability under `vector_released`.
- **Open:** Optimal $K$, $w_{\text{hist}}$, similarity thresholds $(\tau_{\text{match}}, \tau_{\text{rel}}, \tau_{\text{merge}})$, and whether $\rho_{\text{split}} = 0$ (conservative clean prior on split) outperforms fractional residual transfer in Lab topic-split scenarios.

## Sources

- Arthur, D., & Vassilvitskii, S. (2007). k-means++: The Advantages of Careful Seeding. *SODA '07*, 1027–1035. DOI: `10.5555/1283383.1283494` **[PR]**
- Dhillon, I. S., & Modha, D. S. (2001). Concept Decompositions for Large Sparse Text Data Using Clustering. *Machine Learning*, 42, 143–175. DOI: `10.1023/a:1007612920971` **[PR]**
- Sculley, D. (2010). Web-scale k-means clustering. *WWW '10*, 1177–1178. DOI: `10.1145/1772690.1772862` **[PR]**
- Chakrabarti, D., Kumar, R., & Tomkins, A. (2006). Evolutionary clustering. *KDD '06*, 554–560. DOI: `10.1145/1150402.1150467` **[PR]**
- Chi, Y., Song, X., Zhou, D., Hino, K., & Tseng, B. L. (2009). On evolutionary spectral clustering. *ACM TKDD*, 3(4), Article 17. DOI: `10.1145/1631162.1631165` **[PR]**
- Kuhn, H. W. (1955). The Hungarian method for the assignment problem. *Naval Research Logistics Quarterly*, 2(1–2), 83–97. DOI: `10.1002/nav.3800020109` **[PR]**
- Munkres, J. (1957). Algorithms for the Assignment and Transportation Problems. *J. SIAM*, 5(1), 32–38. DOI: `10.1137/0105003` **[PR]**
- Spiliopoulou, M., Ntoutsi, I., Theodoridis, Y., & Schult, R. (2006). MONIC: Modeling and monitoring cluster transitions. *KDD '06*, 706–711. DOI: `10.1145/1150402.1150491` **[PR]**
- Palla, G., Barabási, A.-L., & Vicsek, T. (2007). Quantifying social group evolution. *Nature*, 446, 664–667. DOI: `10.1038/nature05670` **[PR]**
- Gama, J., Žliobaitė, I., Bifet, A., Pechenizkiy, M., & Bouchachia, A. (2014). A survey on concept drift adaptation. *ACM Computing Surveys*, 46(4), Article 44. DOI: `10.1145/2523813` **[PR]**
- Ismail, R., & Jøsang, A. (2002). The Beta Reputation System. *15th Bled Electronic Commerce Conference*, Paper 41. URL: `https://aisel.aisnet.org/bled2002/41/` **[PR]**
- Jøsang, A. (2016). *Subjective Logic: A Formalism for Reasoning Under Uncertainty*. Springer. DOI: `10.1007/978-3-319-42337-1` **[PR]**
