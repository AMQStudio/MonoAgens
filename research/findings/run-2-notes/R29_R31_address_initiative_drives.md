# Research Brief 2 — Part B/C Deepening: R-29, R-30, and R-31

> **Scope and Date:** Written November 16, 2026 as specification-ready research input for MonoAgens Wave 2 (`R-29`, `R-30`, `R-31`), extending Round 1 (`research/findings/2026-11-15-run-1.md`), `core-and-lab/design.md` Decision 6, and Part A reconciliations (`A-03`, `A-04`, `A-05`, `A-06`, `A-13a` `R-02/R-04/R-07/R-08`, and `A-14`).
> **Core Invariants Preserved:** (1) Core sees only opaque IDs, timestamps, numeric cues, and capability descriptors—never message words, usernames, or body internals; (2) appraisal gate uses zero LLM calls; (3) no emotion labels or word lists; (4) zero new primitives (all mechanisms reuse `books`, `thought acts`, `doors`, `signals`, `drives`, and the `one gate formula`); (5) deterministic replay via injected clock and seeded RNG; (6) every mechanism has an explicit neutral value on one shared code path; (7) body-generic separation (`universal` vs `chat-only` vs `embodied-only`).

---

## Core-and-Lab and Round 1 Contradictions Resolved Here

1. **`core-and-lab` (`design.md` L173–175, Decision 6) multi-thread `answered` bug:** Setting `answered = 1` whenever *any* third party posts in the place afterward misfires in multi-threaded chats when Carol talks to Dave while Alice waits on Soul. Replacing binary `answered` with `answered(e) = max_{e' > e, k ∉ {s, soul}} deg(e', s)` (graded degree directed back to `s`) and retaining `A-13a` `R-02`'s `(λ_c, λ_a)` split fixes both false suppression and unmodulated third-party answers.
2. **`core-and-lab` (`protocol/spec.md`, Decision 5–6) missing spatial orientation slot:** Decision 5 carries only `addressees` and `named`. Embodied adapters must be allowed to pass an optional, pre-normalized dimensionless per-listener score `orient(e, l) ∈ [0, 1]` through the additive percept-feature contract (`0.0` default when absent).
3. **Round 1 `R-29` (`2026-11-15-run-1.md` L1138) `max(...)` aggregation loses cross-modal corroboration:** Face-to-face meeting studies (`E06-1022`) show conversational context (`73.11%`) + gaze (`66.45%`) combine to `80.03%`. Switching from `max` to noisy-OR preserves `deg = 1.0` on explicit/link cues while rewarding independent corroboration between chat continuity and embodied spatial cues, without letting pure chat continuity hijack the commitment breaker (`max_cont < θ_alloc`).
4. **Round 1 `R-30` (`2026-11-15-run-1.md` L1171–1181) open `"distinctive overlap"` & extra `δ_init` primitive:** Replaced with a closed-form content-blind `"distinctive overlap"` `DO(ctx_t, m) ∈ [0, 1]` over opaque structural cue IDs (IDF-adaptive ACT-R fan) plus background-centered embedding excess, and folded `δ_init` into an `h_init(p)` channel entry inside the existing `habituation` primitive (`A-03`).
5. **`core-and-lab` Decision 11 rigid sleep gate & `A-04` invariant:** Sleep pressure `D_sleep` must NOT raise gate threshold on external incoming events (`Δθ_sleep = 0`, `A-04`). Instead it modulates only `T_idle_eff = T_idle / clamp(D_sleep, 1.0, D_sleep^max)` once `unconsolidated >= N_min` and adds an opportunity-cost penalty solely to unprompted `R-30` idle initiatives.
6. **Part A `A-14` vs Round 1 `R-31` curiosity split:** Unified `A-14` (`4 FoK(1-FoK) + S_bar_exp`) with `R-31`'s exact 1-step Beta Expected Value of Sample Information `36 · ΔVar_1(b) · impact(b)` inside `D_curiosity`, feeding internal idle-probe `info_gap` and `ask/search` utility without contaminating incoming `dialogue_act_gap`.

---

# R-29: Who a line is for (§22)

**Status:** mostly settled (structure and update rules settled; threshold boundaries and modality weights require Lab/domain calibration)

**Answer.**

### 1. Normative Representation: Listener-Wise Independent Continuous Degree
Consistent with **Mori et al. (2026, SIGDIAL, `arXiv:2607.15648`)**, **Goffman (1979)**, and **Clark & Carlson (1982)**, address in MonoAgens is **continuous and listener-wise independent** rather than simplex-normalized across listeners (`∑_l deg(e, l)` need NOT equal `1`). A broadcast remark (`E=(1,1)` in Mori et al.) simultaneously assigns `deg ≈ 1` to multiple ratified listeners; a side-remark assigns `deg ≈ 1` to Bob and low or near-zero degree to side-participants (`Clark & Carlson, 1982`) or overhearers (`Goffman, 1979`). Core computes `deg(e, l) ∈ [0, 1]` for every active participant `l ∈ P_active(p) \ {s}` on an arriving event `e` from sender `s = actor(e)` in place `p = place(e)` using zero word reading.

### 2. Body-Generic Formula for `deg(e, l)`

#### Step A — Component Cues (with explicit channel scope)

1. **`explicit(e, l) ∈ {0, 1}`** (*`universal`*, with channel-type qualifier):
   $$\mathrm{explicit}(e, l) = \begin{cases} 1 & \text{if } l \in e.\mathrm{addressees} \;\lor\; \big(\mathrm{is\_dyadic\_chat}(p) \land |P_{\text{active}}(p)| = 2\big), \\ 0 & \text{otherwise.} \end{cases}$$
   *Qualifier:* `is_dyadic_chat(p)` is `true` only for **chat-only** strict 1:1 DM capabilities (`!embodied_spatial`). For **embodied-only** 2-person open-world rooms where `|P_active(p)| = 2` (human + robot), empty `e.addressees` does *not* default to `explicit = 1`, preventing self-talk or phone calls while facing away from tripping unconditional allocation; embodied bodies rely on `orient(e, l)` or explicit vocative tagging.

2. **`link(e, l) ∈ {0, 1}`** (*`universal`*):
   $$\mathrm{link}(e, l) = \mathbb{1}\!\Big[\big(\exists\, x \in \mathrm{Expectations}_{\text{open}}(p) : \mathrm{answers}(e, x) \land \mathrm{counterparty}(x) = l\big) \;\lor\; \big(e.\mathrm{reply\_to\_actor} = l\big)\Big]$$
   Captures structural reply-to references (e.g., thread parent ID in chat, `Ouchi & Tsuboi 2016`; `Gu et al. 2022` `HeterMPC` interlocutor–utterance graph) and open `response`/`wait`/`effect` expectation resolution (`core-and-lab/specs/expectations`).

3. **`cont(e, l) ∈ [0, 1]`** (*`universal`*, structural speaker–listener affinity with fan dilution per `Le et al. 2019`, `Jovanovic et al. 2006`, `Penzo et al. 2024`):
   Over the bounded place history window $\mathcal{H}_p(t)$ of the most recent $W_c$ events strictly before $e$ in place $p$ (default $W_c = 12$, deterministically indexed with injected clock $t$):
   $$C_p(s, l) = \sum_{j \in \mathcal{H}_p(t)} \gamma_c^{\Delta n_j} \cdot e^{-\Delta t_j / \tau_c} \cdot r_j(s, l)$$
   where $\Delta n_j \ge 1$ is event index distance from $e$ in $p$, $\Delta t_j = t(e) - t(j) \ge 0$ in seconds, and reciprocal tie credit $r_j(s, l)$ is:
   $$r_j(s, l) = \begin{cases}
   \mathrm{deg}(j, l) & \text{if } \mathrm{actor}(j) = s \land \mathrm{deg}(j, l) \ge \theta_{\text{trace}}, \\
   \mathrm{deg}(j, s) & \text{if } \mathrm{actor}(j) = l \land \mathrm{deg}(j, s) \ge \theta_{\text{trace}}, \\
   \frac{1}{\max(1,\, |P_{\text{active}}(p)| - 1)} & \text{if } \mathrm{actor}(j) = l \land \Delta n_j \le 2 \land \max_{v}\mathrm{deg}(j, v) < \theta_{\text{trace}}, \\
   0 & \text{otherwise.}
   \end{cases}$$
   (Note: `deg(j, ·)` uses already-stored past values, avoiding circularity; $\theta_{\text{trace}} = 0.45$.)
   Saturating fan-normalized continuity (capturing `Ouchi & Tsuboi 2016` and `Penzo et al. 2024` out-degree degradation as a speaker addresses multiple partners):
   $$\mathrm{cont}(e, l) = \frac{C_p(s, l)}{K_c + C_p(s, l) + \beta_{\text{fan}} \sum_{v \in P_{\text{active}}(p)\setminus\{s, l\}} C_p(s, v)}$$
   Notice that if $\mathcal{H}_p(t) = \emptyset$ or $C_p(s, l) = 0$, $\mathrm{cont}(e, l) = 0$.

4. **`orient(e, l) ∈ [0, 1]`** (*`embodied-only`*; identically `0.0` for **chat-only** bodies or omitted percept features):
   Embodied adapters compute the dimensionless scalar `orient(e, l)` from raw spatial sensors before passing it across Waist 1 (`protocol/spec.md` optional reportable cue):
   - **Gaze/head-pose alignment cone** (`Katzenmaier et al. 2004`, `Jovanovic et al. 2006`, `Sheikhi & Odobez 2015`, `Skantze 2021`):
     $$g_{\text{pose}}(s, l) = \max\!\left(0,\; \cos\!\left(\frac{\pi}{2}\cdot \min\!\left(1,\, \frac{|\Delta\phi_{s \to l}|}{\phi_{\max}}\right)\right)\right)$$
     where $|\Delta\phi_{s \to l}|$ is angular error (deg) between speaker head/gaze vector and listener bearing.
   - **Proxemic & Kendon (1990) F-formation O-space membership** (`Kendon 1990`, `Takayama & Pantofaru 2009`, `Bohus & Horvitz 2009`):
     $$p_{\text{prox}}(s, l) = \frac{1}{1 + \exp\!\big((d_{sl} - d_0)/\sigma_d\big)}$$
     where $d_{sl}$ is distance (m) and $F_{sl} = \mathbb{1}[l \in \mathcal{F}(s)] \in \{0, 1\}$ indicates shared O-space transactional membership.
   - **Embodied adapter normalized output sent to core:**
     $$\mathrm{orient}(e, l) = \mathrm{clamp}\!\Big(\alpha_g \cdot g_{\text{pose}}(s, l) \cdot \big(F_{sl} + (1 - F_{sl})\,\eta_{\text{out}}\big) + (1 - \alpha_g) \cdot F_{sl} \cdot p_{\text{prox}}(s, l),\; 0,\; 1\Big)$$
   Core itself executes *zero* geometry—it reads only the passed float `orient(e, l) ∈ [0, 1]` (default `0.0`).

#### Step B — Listener-Wise Noisy-OR Aggregation (*`universal`*)
$$\mathrm{deg}(e, l) = 1 - \big(1 - \mathrm{explicit}(e, l)\big)\big(1 - \mathrm{link}(e, l)\big)\big(1 - w_c \cdot \mathrm{cont}(e, l)\big)\big(1 - w_o \cdot \mathrm{orient}(e, l)\big)$$
Properties verified numerically (`K_c=0.35, β_fan=0.5, w_c=0.65, w_o=0.80, α_g=0.60, η_out=0.65`):
- Explicit address / 1:1 DM (`explicit=1`) or structural reply/expectation link (`link=1`) $\implies \mathrm{deg} = 1.000$.
- Broadcast `@everyone` $\implies \mathrm{deg}(e, l) = 1.000$ for every listener independently (`Mori et al. 2026` `E`).
- Active 2-turn chat dyad inside group (`C_p=1.4`, others `0`) $\implies \mathrm{deg} = 0.520$.
- Single incidental chat turn (`C_p=1.0`) $\implies 0.481$; busy round-table (`C_p=1.0`, others `2.0`) $\implies 0.277$ (fan-diluted).
- Embodied F-formation + direct gaze (`g_pose=1.0, p_prox=0.9, F=1`) $\implies \mathrm{deg} = 0.768$.
- Embodied ratified side-participant inside F-formation (`g_pose=0, F=1`) $\implies \mathrm{deg} = 0.288$.
- Distant passing glance outside F-formation (`g_pose=1.0, F=0`) $\implies \mathrm{deg} = 0.312$.

---

### 3. Exact Integration into Downstream Core Mechanisms (*`universal`*)

1. **`social_cost` (`core-and-lab` + `A-13a` `R-02` split):**
   Replace binary flags with continuous degrees while keeping `A-13a`'s `(λ_c, λ_a)` separation:
   $$\mathrm{sel\_other}(e) = \mathrm{clamp}\!\left(\max_{l \in P_{\text{active}}(p) \setminus \{s, \text{soul}\}} \mathrm{deg}(e, l) - \mathrm{deg}(e, \text{soul}),\; 0,\; 1\right)$$
   $$\mathrm{answered}(e) = \max_{\substack{e' \in \mathrm{after}(e, p) \\ \mathrm{actor}(e') \notin \{s, \text{soul}\}}} \mathrm{deg}(e', s)$$
   $$s_{\text{sc}}(e) = \max\!\Big(\mathrm{sel\_other}(e)\cdot \big(1 - \lambda_c \cdot \mathrm{closeness}(s)\big),\; \mathrm{answered}(e)\cdot \big(1 - \lambda_a \cdot \mathrm{closeness}(s)\big)\Big)$$
   *Fixes `core-and-lab` bug:* Unrelated third-party chatter in place $p$ gives `deg(e', s) ≈ 0`, so `answered(e)` only fires when the third party actually addresses `s` back (`e'` during patience wait short-circuits wait when `answered(e) >= θ_alloc`). Broadcasts give `sel_other = 1 - 1 = 0`. Side-participants (`deg(Bob)=0.77, deg(Soul)=0.29`) get graded deference `0.48`.

2. **`closeness` exchanges (`θ_ex` on discrete Petrov updates):**
   Because Petrov's hybrid `B_p` (`design.md` L181–182) requires integer count $n$ and exact timestamps $t_j$, record a bilateral exchange timestamp between `soul` and $s$ iff:
   $$\max\big(\mathrm{deg}(e_{\text{in}}, \text{soul}),\; \mathrm{deg}(e_{\text{out}}, s)\big) \ge \theta_{\text{ex}} \quad\land\quad \sum_{l \in P_{\text{active}}(p) \setminus \{s\}} \mathrm{deg}(e, l) \le M_{\text{ex}}$$
   With start values $\theta_{\text{ex}} = 0.50$ (`0.30–0.80`) and $M_{\text{ex}} = 3.0$ (`2.0–6.0`, or $\infty$ to disable), active implicit 2-turn chat dyads (`deg = 0.520`) sustain closeness growth, whereas glancing side-looks (`0.29–0.31`) or mass `@channel` broadcasts (`∑ deg >> 3`) feed only familiarity `B_ctx` (`presence`) rather than dyadic `closeness`.

3. **Commitment breaker & unallocated salience (`θ_alloc`):**
   - **Allocated (`deg(e, soul) >= θ_alloc`, start `0.68`, range `0.50–0.85`):** Commitment breaker fires (`go` without threshold suppression, skipping post-eval `speaking_bar`/`certainty_bar` checks). Setting `w_c = 0.65 < θ_alloc = 0.68` guarantees chat continuity alone *cannot* unilaterally hijack the commitment breaker (matching `Le et al. 2019` / `Ouchi & Tsuboi 2016` error rates of 23–45% on implicit turns), whereas explicit addresses (`1.0`), expectation links (`1.0`), or strong embodied F-formation + direct gaze (`0.768`) do break through cleanly.
   - **Unallocated (`deg(e, soul) < θ_alloc`):** Breaker stays off. Instead `deg(e, soul)` enters the existing `commitment` signal (`s_commitment = max(promise_commitment, deg(e, soul))`, zero new signal primitive!) while reducing `social_cost` above; full appraisal gate and post-eval `motivation >= speaking_bar` ∧ `confidence >= certainty_bar` checks remain enforced.

**Parameters.**

| Parameter | Controls | Start value | Search range / Calibration | Unit | Scope |
|---|---|---:|---|---|---|
| `θ_alloc` | Commitment breaker threshold on `deg(e, soul)` | `0.68` | `0.50 – 0.85` (above `w_c`) | ratio `[0,1]` | universal |
| `θ_ex` | Min degree to log discrete closeness exchange | `0.50` | `0.30 – 0.80` | ratio `[0,1]` | universal |
| `M_ex` | Max sum-of-degrees audience size for dyadic closeness | `3.0` | `2.0 – 6.0` (or `∞`) | effective listeners | universal |
| `w_c` | Noisy-OR weight on chat/structural continuity | `0.65` | `0.40 – 0.80` (`< θ_alloc` if cautious) | weight `[0,1]` | universal |
| `w_o` | Noisy-OR weight on embodied orientation cue | `0.80` | `0.55 – 0.95` | weight `[0,1]` | embodied-only |
| `γ_c` | Per-turn decay of speaker–listener affinity | `0.65` | `0.40 – 0.85` | per turn | universal |
| `τ_c` | Wall-clock time constant of continuity window | `120` | `30 – 600` | seconds | universal |
| `W_c` | Bounded place history ring buffer length | `12` | `6 – 24` | events | universal |
| `K_c`, `β_fan` | Continuity saturation & multi-partner fan penalty | `0.35`, `0.50` | `0.2–1.0`, `0.2–1.0` | affinity units | universal |
| `θ_trace` | Min past degree counted as directed affinity | `0.45` | `0.30 – 0.65` | ratio `[0,1]` | universal |
| `λ_c`, `λ_a` | Closeness relief on `sel_other` vs `answered` (`A-13a`) | `0.35`, `0.00` | `0.0–0.6`, `0.0–0.15` | ratio | universal |
| `φ_max` | Head/gaze attention half-cone angle (adapter) | `35` | `20 – 45` | degrees | embodied-only |
| `d_0`, `σ_d` | Proxemic dropoff midpoint & width (adapter) | `2.2`, `0.5` | `1.5–3.5`, `0.3–1.0` | meters | embodied-only |
| `α_g`, `η_out` | Gaze vs O-space balance & out-of-F-formation attenuation | `0.60`, `0.65` | `0.4–0.8`, `0.2–0.8` | ratio `[0,1]` | embodied-only |

**Neutral value.** Set `w_c = 0.0`, `w_o = 0.0`, `M_ex = ∞`, `θ_ex = 0.50`, `θ_alloc = 0.50`, and restrict `link` & graded `answered` back to binary `core-and-lab` checks: `deg(e, l) = explicit(e, l) ∈ {0, 1}`, collapsing identically to Decision 6 binary addressing.

**Claim.** Replacing binary addressee detection with noisy-OR `deg(e, l)` and `θ_alloc = 0.68 > w_c` will reduce false interjections in multi-party rooms where two humans converse implicitly (false-go rate drop ≥ 40% vs naive "any non-other turn" or low threshold) while cutting missed implicit responses in 2-party implicit follow-ups and embodied F-formations (recall ≥ 0.85 when gaze+O-space aligns, `0` closeness inflation on `@everyone` broadcasts).

**How the Lab would test it.**
1. *Multi-threaded chat probe (deterministic synthetic):* 4 actors in one room (`Alice ↔ Soul` 2-turn implicit thread, simultaneous unrelated `Carol → Dave` messages during patience wait). Assert: (a) `Carol → Dave` yields `answered(Alice) < 0.25` (does not abort Soul's reply to Alice); (b) `Alice` implicit turn 2 yields `0.50 <= deg < 0.68` (logs closeness exchange `θ_ex`, does not trip breaker); (c) replay invariance under identical seed/clock.
2. *Embodied F-formation & side-participant probe:* Triad (`F={Alice, Bob, Soul}` + passerby `Dan`), varying `Δφ` and `F_sl`. Assert: direct gaze in O-space (`0.768 >= θ_alloc`), side-participant (`0.288`), distant glance (`0.312`), `@everyone` broadcast (`∑ deg > 3 -> no closeness update`).
3. *Neutral equivalence check:* Verify `(w_c=0, w_o=0)` reproduces baseline binary Decision 6 behavior across single-thread fixtures.

**Established / inferred / open.**
- *Established:* Address is continuous, listener-wise non-simplex, and predicts next speaker (`elpd_diff = -14.4`), gaze (`-39.2`), and backchannels (`-3.5`) better than discrete labels (`Mori et al., 2026`); addressee recognition is predominantly structural (`Penzo et al., 2024`: `STRUCT` matches/beats text+structure and beats text-only) while group fan degrades accuracy (`Ouchi & Tsuboi, 2016`; `Le et al., 2019`; `Penzo et al., 2024`); explicit addressees occur in only ~20% of face-to-face triadic turns (`Inoue et al., 2025`; `Mori et al., 2026b` TEIDAN); combining conversation context (`73.11%`) and gaze (`66.45%`) reaches `80.03%` (`Jovanovic et al., 2006`), complemented by F-formations (`Kendon, 1990`) and engagement intention/action separation (`Bohus & Horvitz, 2009`).
- *Inferred:* Noisy-OR parameter combination (`w_c, w_o`), fan-diluted `cont(e, l)`, separating `θ_ex < w_c < θ_alloc`, and `answered(e) = max deg(e', s)`.
- *Open / not settled by literature:* Exact `w_c, w_o, θ_alloc` values vary across room latency and culture (e.g., compact collaborative Japanese backchannels vs longer English holds in `TEIDAN`, `arXiv:2609.00802`), requiring per-deployment/Lab empirical Bayes calibration via observed `answered` / user continuation feedback.

**Sources.**
- Mori, T., Inoue, K., Lala, D., & Kawahara, T. (2026). *On the Structure of Address in Multi-Party Dialogue: From Discrete Labels to Continuous Levels*. Proc. SIGDIAL 2026, pp. 668–679. `[PR]` DOI: `10.18653/v1/2026.sigdial-1.47`, `arXiv:2607.15648v1`.
- Mori, T., Inoue, K., Elmers, M., Lala, D., & Kawahara, T. (2026). *TEIDAN: A Multilingual Multiparty Dialogue Corpus*. ICMI Companion '26. `[PR]` DOI: `10.1145/3776591.3836650`, `arXiv:2609.00802v1`.
- Inoue, K., Lala, D., Elmers, M., Ochi, K., & Kawahara, T. (2025). *An LLM Benchmark for Addressee Recognition in Multi-modal Multi-party Dialogue*. IWSDS 2025, pp. 330–334. `[PR]` `arXiv:2501.16643`.
- Goffman, E. (1979). *Footing*. *Semiotica*, 25(1–2), 1–30. `[PR]` DOI: `10.1515/semi.1979.25.1-2.1`.
- Clark, H. H., & Carlson, T. B. (1982). *Hearers and Speech Acts*. *Language*, 58(2), 332–373. `[PR]` DOI: `10.1353/lan.1982.0042` (`10.2307/414102`).
- Ouchi, H., & Tsuboi, Y. (2016). *Addressee and Response Selection for Multi-Party Conversation*. EMNLP 2016, pp. 2133–2143. `[PR]` DOI: `10.18653/v1/D16-1231`.
- Le, R., Hu, W., Shang, M., You, Z., Bing, L., Zhao, D., & Yan, R. (2019). *Who Is Speaking to Whom? Learning to Identify Utterance Addressee in Multi-Party Conversations*. EMNLP-IJCNLP 2019, pp. 1909–1919. `[PR]` DOI: `10.18653/v1/D19-1199`.
- Gu, J.-C., Tan, C.-H., Tao, C., Ling, Z.-H., Hu, H., Geng, X., & Jiang, D. (2022). *HeterMPC: A Heterogeneous Graph Neural Network for Response Generation in Multi-Party Conversations*. ACL 2022, pp. 5086–5097. `[PR]` DOI: `10.18653/v1/2022.acl-long.349`.
- Penzo, N., Sajedinia, M., Lepri, B., Tonelli, S., & Guerini, M. (2024). *Do LLMs suffer from Multi-Party Hangover? A Diagnostic Approach to Addressee Recognition and Response Selection in Conversations*. EMNLP 2024, pp. 11210–11233. `[PR]` DOI: `10.18653/v1/2024.emnlp-main.628`.
- Bohus, D., & Horvitz, E. (2009). *Models for Multiparty Engagement in Open-World Dialog*. SIGDIAL 2009, pp. 225–234. `[PR]` DOI: `10.3115/1708376.1708409`, `https://aclanthology.org/W09-3933/`.
- Kendon, A. (1990). *Conducting Interaction: Patterns of Behavior in Focused Encounters*. Cambridge University Press. `[PR]` (Reviewed: `10.2307/2075490`).
- Jovanovic, N., op den Akker, R., & Nijholt, A. (2006). *Addressee Identification in Face-to-Face Meetings*. EACL 2006, pp. 169–176. `[PR]` `https://aclanthology.org/E06-1022/`.
- Katzenmaier, M., Stiefelhagen, R., & Schultz, T. (2004). *Identifying the addressee in human-human-robot interactions based on head pose and speech*. ICMI '04, pp. 144–151. `[PR]` DOI: `10.1145/1027933.1027959`.
- Sheikhi, S., & Odobez, J.-M. (2015). *Combining dynamic head pose–gaze mapping with the robot conversational state for attention recognition in human–robot interactions*. *Pattern Recognition Letters*, 66, 81–90. `[PR]` DOI: `10.1016/j.patrec.2014.10.002`.
- Takayama, L., & Pantofaru, C. (2009). *Influences on proxemic behaviors in human-robot interaction*. IEEE/RSJ IROS 2009, pp. 5495–5502. `[PR]` DOI: `10.1109/iros.2009.5354145`.
- Skantze, G. (2021). *Turn-taking in Conversational Systems and Human-Robot Interaction: A Review*. *Computer Speech & Language*, 67, 101178. `[PR]` DOI: `10.1016/j.csl.2020.101178`.

---

# R-30: Idle mode and initiative (§23)

**Status:** partly open (computational form specified below; utility weights and per-channel cooldown slopes require Lab/field calibration)

**Answer.**

Idle mode runs in the existing cognitive loop between external turns (`unallocated`, queued behind incoming percepts, never blocking incoming events and yielding to sleep consolidation when triggered per `R-31`/`A-04`). It consists of four deterministic, content-blind stages over existing primitives.

### 1. Computable "Distinctive Overlap" (`DO(ctx_t, m) ∈ [0, 1]`, *`universal`*, zero word reading)
Building on **Berntsen (2020/2021)**, **Berntsen, Staugaard & Sørensen (2013)** (unique cues trigger spontaneous involuntary retrieval whereas repeated/overloaded cues fail), **Anderson (1974)** / **Anderson & Reder (1999)** / **Anderson et al. (2004)** ACT-R associative strength $S_{jm} = S - \ln(\mathrm{fan}_j)$ with source weights $W_j$, **Nairne (2002)** (match diagnosticity relative to competing memory traces), and **McDaniel & Einstein (2000)** (focal cue-specific involuntary prospective retrieval):

Every memory episode or pending intention (`goal` / `promise` / `wait` / unsealed episode) $m$ holds an opaque structural cue set $\mathcal{C}(m) \subseteq \{\text{place\_id}, \text{actor\_ids}, \text{domain\_id}, \text{goal\_id}\}$ and an optional unit embedding vector $\mathbf{v}_m \in \mathbb{R}^d$ (`||v_m||_2 = 1`) emitted upstream as opaque floats, plus a moving background similarity baseline $\bar{s}_m \in [0, 1]$ updated slowly on recall checks to detect semantic "hubs".

1. **Structural Cue Discriminability (ACT-R fan / corpus-adaptive IDF, *`universal`*):**
   Let $N_{\text{mem}} \ge 1$ be total active/archival memories indexed in Books 4–5 and $\mathrm{fan}_j \ge 1$ be the inverted-index count of memories containing opaque cue ID $j$. Each cue's normalized discriminability weight is:
   $$\mathrm{disc}(j) = \frac{\max\big(0,\; \ln(N_{\text{mem}} + 1) - \ln(\mathrm{fan}_j + 1)\big)}{\ln(N_{\text{mem}} + 1)} \in [0, 1]$$
   Given current context structural cues $\mathcal{C}_t$ (current `place`, active `actor` set, active `domain_id`, active `goal_id`) with ACT-R attentional dilution $w_{\text{attn}} = \min(1,\, |\mathcal{C}_t|^{-p_{\text{fan}}})$ across active cues ($|\mathcal{C}_t| \ge 1$):
   $$\mathrm{ov}_{\text{struct}}(\mathcal{C}_t, m) = 1 - \prod_{j \in \mathcal{C}_t \cap \mathcal{C}(m)} \big(1 - \mathrm{disc}(j) \cdot w_{\text{attn}}\big)$$
   *(Guard: if $\mathcal{C}_t \cap \mathcal{C}(m) = \emptyset$, $\mathrm{ov}_{\text{struct}} = 0$.)* Unique cues ($\mathrm{fan}_j = 1$ at $N_{\text{mem}} = 200$, $|\mathcal{C}_t|=4$, $p_{\text{fan}}=0.5$) yield `0.435` alone, whereas common overloaded room IDs ($\mathrm{fan}_j = 120$) yield only `0.048`.

2. **Background-Centered Semantic Vector Excess (*`universal`*, zero text reading):**
   Over the top-$K_{\text{pool}}$ (`<= 64`) ACT-R base-level retrieved candidate pool for context vector $\mathbf{v}_t$, let $\bar{c}_t = \mathrm{mean}_{i \in \text{pool}} \cos(\mathbf{v}_t, \mathbf{v}_i)$ and $\sigma_{c,t} = \mathrm{std}_{i \in \text{pool}} \cos(\mathbf{v}_t, \mathbf{v}_i)$. Excess distinctiveness relative to *both* present background anisotropy ($\bar{c}_t$) and item hubness ($\bar{s}_m$) is:
   $$\mathrm{ov}_{\text{sem}}(\mathbf{v}_t, m) = \mathrm{clamp}\!\left(\frac{\cos(\mathbf{v}_t, \mathbf{v}_m) - \max(\bar{c}_t,\; \bar{s}_m)}{z_{\max} \cdot (\sigma_{c,t} + \epsilon)},\; 0,\; 1\right)$$
   *(Guard: if embeddings are omitted/zero, $\mathrm{ov}_{\text{sem}} = 0$ and $\alpha_{\text{do}} = 1.0$.)*

3. **Combined Distinctive Overlap (*`universal`*):**
   $$\mathrm{DO}(\mathrm{ctx}_t, m) = \mathrm{clamp}\!\Big(\alpha_{\text{do}} \cdot \mathrm{ov}_{\text{struct}}(\mathcal{C}_t, m) + (1 - \alpha_{\text{do}}) \cdot \mathrm{ov}_{\text{sem}}(\mathbf{v}_t, m),\; 0,\; 1\Big)$$
   Only top-$K_{\text{cand}}$ candidates passing $\mathrm{DO}(\mathrm{ctx}_t, m) \ge \theta_{\text{do}}$ (start `0.35`, aligning closely with `Inner Thoughts` `arXiv:2501.00383` cosine `0.3` filter) enter utility evaluation, avoiding wasteful background scans.

---

### 2. Horvitz (1999 CHI) Expected Utility `EU` (zero LLM call in gate, *`universal`*)
Following **Horvitz (1999, `10.1145/302979.303030`)**, a candidate idle thought/intention $m$ targeted at place $p$ beats inaction ($\neg A_m$) iff:
$$\mathrm{EU}(m, t, p) = p_{\text{matters}}(m, t, p) \cdot B_m(t) - \big(1 - p_{\text{matters}}(m, t, p)\big) \cdot C_{\text{int}}(t, p) - c_{\text{attn}}(p) > 0$$
*(Algebraically matching `p_matters * benefit - cost_interrupt` when total interruption penalty combines false-positive disruption `(1-p)·C_int` and attention overhead `c_attn`.)* Using purely numeric/structural core state:
- **$p_{\text{matters}}(m, t, p) ∈ [0, 1]$:**
  $$p_{\text{matters}}(m, t, p) = \mathrm{clamp}\!\Big(\mathrm{DO}(\mathrm{ctx}_t, m) \cdot \hat{p}_{\text{acc}}(p) \cdot e^{-\Delta t_{\text{staleness}}(m)/\tau_{\text{stale}}},\; 0,\; 1\Big)$$
  where $\hat{p}_{\text{acc}}(p) = \frac{\alpha_{\text{acc}, p}}{\alpha_{\text{acc}, p} + \beta_{\text{acc}, p}}$ is the Beta acceptance posterior from existing outcome Ledger receipts in place $p$ (prior $\mathrm{Beta}(2, 2) \implies 0.50$, adapting downward automatically during platform quiet hours or high-ignore periods per `A-04`).
- **Expected Benefit $B_m(t) ∈ [0, 1]$ (with `A-03` Rumination Habituation):**
  $$B_{\text{raw}}(m, t) = \mathrm{clamp}\!\Big(w_{\text{st}} \cdot \rho_{\text{stakes}}(m) + w_{\text{cur}} \cdot D_{\text{curiosity}}(m, t) + w_{\text{urg}} \cdot \mathrm{urg}(m),\; 0,\; 1\Big)$$
  $$B_m(t) = \frac{B_{\text{raw}}(m, t)}{1 + \kappa_h \cdot h_m^{\text{prior}}}, \qquad h_m^{\text{prior}} = h_m \cdot \gamma_{\text{rum}}^{\Delta t_m}$$
  Reuses **R-01 / `A-03` post-evaluation habituation** directly on candidate ID `thought:m` (**Christoff et al., 2016**): every idle cycle where candidate $m$ is evaluated and *not* resolved/backed by new external evidence updates $h_m \leftarrow h_m^{\text{prior}} + 1$, collapsing `B_m` across consecutive idle cycles (`1 -> 0.5 -> 0.33 -> ...`) so the core cannot ruminate on a sticky thought! New disconfirming/advancing percepts referencing $m$ reset $h_m \leftarrow 0$.
- **Interruption Cost $C_{\text{int}}(t, p)$ & Breakpoint Modulation:**
  $$C_{\text{int}}(t, p) = c_{\text{base}} \cdot \big(1 - \beta_{\text{bp}} \cdot b_{\text{bp}}(t, p)\big) + \beta_{\text{load}} \cdot \mathrm{load}(t, p) + \lambda_{\text{sleep,init}} \cdot \max\!\big(0,\, D_{\text{sleep}}(t) - 1\big)$$
  where $\mathrm{load}(t, p) \in [0, 1]$ is normalized recent turn burst rate in $p$, $\lambda_{\text{sleep,init}}$ yields unprompted chatter when sleep backlog $D_{\text{sleep}} > 1$ (`R-31`), and $c_{\text{attn}}(p) = 0.05 \cdot c_{\text{base}}$.

---

### 3. Breakpoint Delivery Across Chat and Physical Bodies (*`universal` + channel tags*)
Empirical interruption science establishes that **coarse subtask/activity boundaries, NOT raw inactivity timeouts, minimize disruption**:
- Desktop & knowledge work: interrupting at **coarse breakpoints** cuts annoyance **56% vs worst moments** and **43% vs random** (**Adamczyk & Bailey, 2004**; **Bailey & Iqbal, 2008**; **Iqbal & Horvitz, 2007**; **Mark et al., 2008**).
- Mobile phones (non-programming): physical postural/ambulatory transitions (`91.2%` accuracy, **Ho & Intille, 2005**) and interaction/app-switching breakpoints (**Attelia II**, **Okoshi et al., 2015**, **-46% cognitive load** in lab / **-33%** in the wild; **InterruptMe**, **Pejovic & Musolesi, 2014**) strongly outperform random pings, while **Mehrotra et al. (2016)** show low utility drives dismissal (`40.58%` not important, `43.47%` not urgent, `62.52%` overall click rate).
- Domestic smart speakers (non-programming): **Cha et al. (2020)** (`40` households, `3,572` reports across `19` activities) find opportune moments align with activity transitions and natural domestic breaks rather than dual-task focus.
- Collaborative coding & multi-party dialogue: **Pu et al. (2025, Codellaborator, `arXiv:2502.18658`)** prove **30 s raw idleness fails** because idle coders are under high planning/thinking load, whereas completion/execution boundaries achieve **66.7%–73.1%** engagement vs **12.1%** overall disruption; **Liu et al. (2025, Inner Thoughts, `arXiv:2501.00383`)** and **Yang et al. (2026, ProACT, `arXiv:2607.03730`)** show turn-boundary filtering raises silence appropriateness from `65.2%` to `89.6%` (`+1.12` to `+1.33` in governance/academic/design vs `+0.38` in coding).

Accordingly, core computes **`b_bp(t, p) ∈ {0.0, 0.5, 1.0}`** deterministically without reading words:
$$b_{\text{bp}}(t, p) = \begin{cases}
1.0 & \text{if } (\text{R-23 surprise/episode seal boundary in } p \text{ within } \Delta t \le \tau_{\text{win}}) \;\lor\; (\text{expectation in } p \text{ just resolved } \texttt{done}) \;\lor\; (b_{\text{phys}}(t, p) = 1), \\
0.5 & \text{if } \mathrm{open\_waits}(p) = 0 \;\land\; \Delta t_{\text{quiet}}(p) \ge \kappa_{\text{bp}} \cdot \mathrm{med\_lat}(p), \\
0.0 & \text{otherwise (active mid-burst / mid-task pause where } \Delta t_{\text{quiet}}(p) < \kappa_{\text{bp}}\cdot\mathrm{med\_lat}(p)\text{).}
\end{cases}$$
- **Chat-only:** $b_{\text{phys}}(t, p) = 0$ always; relies on R-23 episode boundaries, expectation completion, and latency-scaled turn pauses ($\kappa_{\text{bp}} \cdot \mathrm{med\_lat}(p)$ from `expectations/spec.md`, preventing `Pu et al.` naive idle-timeout misfires!).
- **Embodied-only:** Optional body-reported $b_{\text{phys}}(t, p) \in \{0, 1\}$ for posture/mobility transitions (`Ho & Intille 2005`, `Cha et al. 2020`) or F-formation opening/dissolution (`Bohus & Horvitz 2009`), defaulting to `0`.

---

### 4. Initiative Budget `δ_init` with Existing Gate & Quiet Hours (`A-04` invariant)
**Zero new primitives:** Store the initiative load counter `h_init(p)` as a reserved synthetic channel ID (`init:<place_id>`) inside the existing **`habituation` table** (`A-03`).
- On every evaluation cycle at time $t$:
  $$h_{\text{init}}^{\text{prior}}(p) = h_{\text{init}}(p) \cdot \gamma_{\text{init}}^{\Delta t}, \qquad \delta_{\text{init}}(p) = \mathrm{step}_{\text{init}} \cdot h_{\text{init}}^{\text{prior}}(p)$$
- When candidate $m$ passes `EU > 0`, it enters the **existing appraisal gate** (`S_eff >= θ_pro`) with **zero new gate formula**:
  $$\theta_{\text{pro}}(p) = \theta(p) + \delta_{\text{init}}(p)$$
  and communicative output must still pass `motivation >= speaking_bar` and `confidence >= certainty_bar` (`design.md` L282–283).
- Updates on `h_init(p)`:
  - Emitting proactive output: `h_init(p) <- h_init_prior(p) + Δ_emit` (`Δ_emit = 1.0`, raising threshold immediately by `+step_init`).
  - Outcome `ignored` / `refused` / `held` (no reciprocal exchange within wait window, or `held` receipt from Valve quiet hours): `h_init(p) <- h_init_prior(p) + Δ_ign` (`Δ_ign = 1.5`, echoing `Pu et al. 2025`'s adaptive penalty backoff, plus updates $\hat{p}_{\text{acc}}(p)$ Beta failures).
- **Quiet hours separation (`A-04`):** Core does *not* know time-of-day DND (`Δθ_sleep = 0`). Valve/Body enforces quiet hours by returning `held`/`refused`, which organically raises `h_init` and lowers $\hat{p}_{\text{acc}}(p)$ without clock-rule duplication in core.

**Parameters.**

| Parameter | Controls | Start value | Search range | Unit | Scope |
|---|---|---:|---|---|---|
| `α_do` | Structural vs vector balance in `DO` | `0.55` | `0.20 – 0.80` (`1.0` if no vec) | weight `[0,1]` | universal |
| `p_fan` | ACT-R cue dilution exponent `|C_t|^{-p_fan}` | `0.50` | `0.25 – 1.00` | exponent | universal |
| `z_max` | Saturation z-score for embedding excess | `2.5` | `1.5 – 4.0` | std devs | universal |
| `θ_do` | Min distinctive overlap to evaluate candidate | `0.35` | `0.15 – 0.60` | ratio `[0,1]` | universal |
| `c_base` | Base interruption cost mid-burst | `0.45` | `0.25 – 0.70` | utility | universal |
| `β_bp` | Breakpoint cost discount (`b_bp = 1`) | `0.70` | `0.40 – 0.85` | ratio `[0,1]` | universal |
| `β_load` | Place burst-load cost coefficient | `0.25` | `0.10 – 0.50` | utility | universal |
| `κ_bp` | Turn-pause multiplier over `med_lat(p)` | `1.8` | `1.2 – 3.5` | × median lat | universal |
| `τ_win` | Post-event-boundary receptivity window | `30` | `10 – 120` | seconds | universal |
| `step_init` | Threshold raise per `h_init` unit (`δ_init`) | `0.10` | `0.02 – 0.30` | threshold | universal |
| `γ_init` | Initiative budget decay base | `0.50` | `0.20 – 0.80` | per hour | universal |
| `Δ_emit`, `Δ_ign` | `h_init` step on emit vs ignore/held | `1.0`, `1.5` | `0.5–1.5`, `0.5–3.0` | count equiv | universal |
| `γ_rum`, `κ_h` | Rumination habituation decay & suppression (`A-03`) | `0.70`/h, `1.0` | `0.3–0.9`/h, `0.5–2.0` | per hour, weight | universal |
| `w_st`, `w_cur`, `w_urg` | Benefit weights (stakes, curiosity, urgency) | `0.45`, `0.35`, `0.20` | simplex/Lab tuned | weight `[0,1]` | universal |
| `τ_stale` | Candidate relevance decay constant | `48` | `12 – 168` | hours | universal |

**Neutral value.** Set `θ_do = 1.1` (no candidate qualifies; idle emission never fires) OR set `step_init = 0.0`, `κ_h = 0.0`, `c_base = 0.0`, `β_bp = 0.0` (eliminates initiative budget, rumination braking, and breakpoint deferral while reverting to baseline static `EU`). Using `allow_idle_initiative = false` (`θ_do = 1.1`) restores pure passive/allocated `core-and-lab` operation on one shared path.

**Claim.** Combining `DO`-filtered Horvitz `EU` with latency-scaled breakpoint timing `b_bp`, `h_m` rumination habituation, and `h_init` budget cooldown will (a) trigger ≥ 80% of proactive outputs at natural/coarse boundaries rather than mid-bursts (`c_int = 0.135` vs `0.450`), (b) halve ignored pings under repeated user non-response via `Δ_ign` + $\hat{p}_{\text{acc}}$, and (c) suppress repeat idle surfacing of an unresolved sticky thought within ≤ 3 polls (`B_m` drops ≥ 66%).

**How the Lab would test it.**
1. *Unique vs overloaded cue discrimination test:* Store `N=200` episodes sharing `place_main` (`fan=120`) plus 1 episode tagged with unique `goal_rare` (`fan=1`). Cue with `(place_main, goal_rare)` and measure `DO`: unique-linked episode must rank #1 with `DO_unique >= 2 × DO_overloaded`.
2. *Breakpoint vs mid-burst timing & ignore backoff probe:* Replay `p=0.55, B=0.35` candidate during active rapid turns (`b_bp=0 -> EU = -0.010`, deferred), then at post-expectation breakpoint (`b_bp=1 -> EU = +0.132`, fires). Deliver mock `ignored` feedback (`h_init` jumps by `+1.5`, blocking subsequent low-stakes pings until `γ_init` decay).
3. *Rumination lock-out & Valve quiet-hours test:* Freeze context cues for 5 idle polls with unresolved candidate `m`; verify `B_m` strictly decreases (`1.0x -> 0.5x -> 0.33x -> 0.25x`) and stops firing without hardcoding time-of-day inside core (`Δθ_sleep = 0`), while `held` receipts during Valve quiet window drive $\hat{p}_{\text{acc}}(p)$ down cleanly.

**Established / inferred / open.**
- *Established:* Involuntary retrieval favors unique/distinctive cues over overloaded cues (`Berntsen et al., 2013`; `Berntsen, 2020/2021`; `Nairne, 2002`; ACT-R fan `S - ln(fan_j)`, `Anderson, 1974`; `Anderson & Reder, 1999`; `Anderson et al., 2004`; focal retrieval `McDaniel & Einstein, 2000`). Expected utility mixed-initiative timing (`Horvitz, 1999`) and coarse-breakpoint delivery outperform random or raw inactivity triggers across programming (`Pu et al., 2025`: 30s idle failed vs 66.7–73.1% boundary engagement), desktop knowledge work (`Adamczyk & Bailey, 2004`: -56% annoyance vs worst; `Iqbal & Horvitz, 2007`; `Bailey & Iqbal, 2008`; `Mark et al., 2008`), multi-party dialogue (`Liu et al., 2025`; `Yang et al., 2026`: silence appropriateness `65.2% -> 89.6%`), mobile devices (`Ho & Intille, 2005`; `Pejovic & Musolesi, 2014`; `Okoshi et al., 2015`: -46% lab / -33% wild cognitive load; `Mehrotra et al., 2016`), and domestic smart speakers (`Cha et al., 2020`). Spontaneous thought sticks absent shifting/inhibitory decay (`Christoff et al., 2016`).
- *Inferred:* Specific IDF-scaled noisy-OR `DO` normalization, latency-relative `κ_bp · med_lat(p)` chat breakpoint proxy, folding `δ_init` into existing `habituation` table `h_init`, and applying `A-03` habituation to rumination suppression.
- *Open / not settled by literature:* Cross-domain calibration of `c_base`, `β_bp`, and `Δ_ign` between synchronous voice/embodied spaces vs asynchronous team chat; no universal constant exists across user tolerance profiles (`Active` vs `Selective` users in `Liu et al., 2025`), requiring online Empirical Bayes learning of $\hat{p}_{\text{acc}}(p)$ and Lab sweeps.

**Sources.**
- Berntsen, D. (2020/2021). *Involuntary autobiographical memories and their relation to other forms of spontaneous thoughts*. *Phil. Trans. R. Soc. B*, 376(1817), 20190693. `[PR]` DOI: `10.1098/rstb.2019.0693`.
- Berntsen, D., Staugaard, S. R., & Sørensen, L. M. T. (2013). *Why am I remembering this now? Predicting the occurrence of involuntary (spontaneous) episodic memories*. *J. Exp. Psychol.: General*, 142(2), 426–444. `[PR]` DOI: `10.1037/a0029128`.
- Nairne, J. S. (2002). *The myth of the encoding-retrieval match*. *Memory*, 10(5–6), 389–395. `[PR]` DOI: `10.1080/09658210244000216`.
- Anderson, J. R. (1974). *Retrieval of propositional information from long-term memory*. *Cognitive Psychology*, 6(4), 451–474. `[PR]` DOI: `10.1016/0010-0285(74)90021-8`.
- Anderson, J. R., & Reder, L. M. (1999). *The fan effect: New results and new theories*. *J. Exp. Psychol.: General*, 128(2), 186–197. `[PR]` DOI: `10.1037/0096-3445.128.2.186`.
- Anderson, J. R., Bothell, D., Byrne, M. D., Douglass, S., Lebiere, C., & Qin, Y. (2004). *An Integrated Theory of the Mind*. *Psychological Review*, 111(4), 1036–1060. `[PR]` DOI: `10.1037/0033-295X.111.4.1036`.
- McDaniel, M. A., & Einstein, G. O. (2000). *Strategic and automatic processes in prospective memory retrieval: a multiprocess framework*. *Applied Cognitive Psychology*, 14(7), S127–S144. `[PR]` DOI: `10.1002/acp.775`.
- Christoff, K., Irving, Z. C., Fox, K. C. R., Spreng, R. N., & Andrews-Hanna, J. R. (2016). *Mind-wandering as spontaneous thought: a dynamic framework*. *Nature Reviews Neuroscience*, 17(11), 718–731. `[PR]` DOI: `10.1038/nrn.2016.113`.
- Horvitz, E. (1999). *Principles of mixed-initiative user interfaces*. CHI '99, pp. 159–166. `[PR]` DOI: `10.1145/302979.303030`.
- Adamczyk, P. D., & Bailey, B. P. (2004). *If not now, when? The effects of interruption at different moments within task execution*. CHI '04, pp. 271–278. `[PR]` DOI: `10.1145/985692.985727`.
- Iqbal, S. T., & Horvitz, E. (2007). *Disruption and recovery of computing tasks: field study, analysis, and directions*. CHI '07, pp. 677–686. `[PR]` DOI: `10.1145/1240624.1240730`.
- Bailey, B. P., & Iqbal, S. T. (2008). *Understanding changes in mental workload during execution of goal-directed tasks and its application for interruption management*. *ACM TOCHI*, 14(4), 1–28. `[PR]` DOI: `10.1145/1314683.1314689`.
- Mark, G., Gudith, D., & Klocke, U. (2008). *The cost of interrupted work: more speed and stress*. CHI '08, pp. 107–110. `[PR]` DOI: `10.1145/1357054.1357072`.
- Ho, J. C., & Intille, S. S. (2005). *Using context-aware computing to reduce the perceived burden of interruptions from mobile devices*. CHI '05, pp. 909–918. `[PR]` DOI: `10.1145/1054972.1055100`.
- Pejovic, V., & Musolesi, M. (2014). *InterruptMe: designing intelligent prompting mechanisms for pervasive applications*. UbiComp '14, pp. 897–908. `[PR]` DOI: `10.1145/2632048.2632062`.
- Okoshi, T., Ramos, J., Nozaki, H., Nakazawa, J., Dey, A. K., & Tokuda, H. (2015). *Reducing users' perceived mental effort due to interruptive notifications in multi-device mobile environments*. UbiComp '15, pp. 475–486. `[PR]` DOI: `10.1145/2750858.2807517`.
- Mehrotra, A., Pejovic, V., Vermeulen, J., Hendley, R., & Musolesi, M. (2016). *My Phone and Me: Understanding People's Receptivity to Mobile Notifications*. CHI '16, pp. 1021–1032. `[PR]` DOI: `10.1145/2858036.2858566`.
- Cha, N., Kim, A., Park, C. Y., Kang, S., Park, M., Lee, J.-G., Lee, S., & Lee, U. (2020). *"Hello There! Is Now a Good Time to Talk?": Opportune Moments for Proactive Interactions with Smart Speakers*. *PACM IMWUT*, 4(3), 1–28. `[PR]` DOI: `10.1145/3411810`.
- Liu, X. B., Fang, S., Shi, W., Wu, C.-S., Igarashi, T., & Chen, X. 'A'. (2025). *Proactive Conversational Agents with Inner Thoughts*. CHI '25. `[PR]` DOI: `10.1145/3706598.3713760`, `arXiv:2501.00383v2`.
- Pu, K., Lazaro, D., Arawjo, I., Xia, H., & Xiao, T. (2025). *Assistance or Disruption? Exploring and Evaluating the Design and Trade-offs of Proactive AI Programming Support*. CHI '25. `[PR]` DOI: `10.1145/3706598.3713357`, `arXiv:2502.18658v4`.
- Yang, S., Xu, D., Pei, J., & Wang, D. (2026). *ProACT: Towards Breakdown-Aware Proactive Agent in Multi-User Collaboration*. `[PP]` `arXiv:2607.03730v1`.

---

# R-31: Drives as opportunity costs (§24)

**Status:** partly open (normative functional forms and mappings onto existing core terms are fixed below; literature supplies no universal scalar weights for synthetic agents, so explicit Lab calibration procedures are specified)

**Answer.**

Grounded in **Agrawal, Mattar, Cohen & Daw (2022)** (fatigue as opportunity cost of foregoing *internal offline computation/replay* $\mathrm{EVB}=\mathrm{Need}\times\mathrm{Gain}$, boredom as opportunity cost of foregoing *external exploration/switching*), **Kurzban et al. (2013)** (effort tracks next-best alternative computation cost), **Shenhav, Botvinick & Cohen (2013)** (Expected Value of Control, $\mathrm{EVC}(\text{signal}, \text{state}) = \sum_i P(\text{Outcome}_i \mid \text{signal}, \text{state}) V(\text{Outcome}_i) - \mathrm{Cost}(\text{signal})$), **Gottlieb et al. (2013)** (curiosity as Expected Value of Information modulated by learnability/progress), and **Borbély (1982) / Daan, Beersma & Borbély (1984)** (two-process homeostatic drive), all four drives compute purely from existing core counters/state and feed **only existing loop/gate terms (zero new primitives)**.

> **Verification note on Agrawal et al. (2022):** While the peer-reviewed record (`10.1037/rev0000309`), core thesis partitioning fatigue (offline replay `EVB = Need × Gain` vs time cost) from boredom (external exploration / Value of Information), and simulation paradigms are verified, exact publisher equations inside the paywalled/Cloudflare-protected full text were not directly inspectable; therefore the closed-form core counters below are **explicitly inferred MonoAgens normative instantiations**, not verbatim equations copied from Agrawal et al. (2022). Likewise, biological Process S time constants (`τ_r = 18.2 h`, `τ_d = 4.2 h`, `H_m = 0.67`, `L_m = 0.17` in human EEG, `Daan et al. 1984`) are replaced by event/backlog-normalized dynamics suited to variable-rate software/embodied agents.

---

### 1. Drive 1 — Sleep Pressure $D_{\text{sleep}}(t)$ (*`universal`*)
**State inputs:** `U(t)` (count of unconsolidated episodes/events in Books 4–5), `N_min` (existing sleep consolidation minimum backlog constant, `design.md` L309), and `Δt_wake` (elapsed awake wall time since last sleep cycle via injected clock).

$$D_{\text{sleep}}(t) = \mathrm{clamp}\!\left(\alpha_u \cdot \frac{U(t)}{\max(1,\, N_{\min})} + (1 - \alpha_u)\cdot \left(1 - e^{-\Delta t_{\text{wake}} / \tau_{\text{wake}}}\right),\; 0,\; D_{\text{sleep}}^{\max}\right)$$
- When $\alpha_u = 0.85, \tau_{\text{wake}} = 24\text{ h}, D_{\text{sleep}}^{\max} = 2.0$: pressure scales smoothly across both fast bursty Lab replays (`U/N_min`) and slow real-time chats (`Δt_wake`).
- **Existing term fed (honoring `A-04` invariant `Δθ_sleep = 0`):**
  1. Feeds **existing sleep consolidation trigger** (`sleep-consolidation/spec.md`, Step 8): triggers offline consolidation when $U(t) \ge N_{\min} \;\land\; \Delta t_{\text{idle\_all}} \ge T_{\text{idle}} / \max(1.0,\, D_{\text{sleep}}(t))$. Heavy unconsolidated backlogs (`D_sleep = 2.0`) cut required idle pause from `T_idle` to `T_idle / 2` because the EVB of replay outweighs waiting.
  2. Feeds **`R-30` idle initiative cost `C_int`** via `λ_sleep,init · max(0, D_sleep - 1)` so unprompted idle initiatives defer to pending consolidation. External incoming percept gates keep `Δθ_sleep = 0`.

---

### 2. Drive 2 — Switching / Exploration Drive $D_{\text{switch}}(g, t) ∈ [0, 1]$ (*`universal`*)
**State inputs:** `n_stag(g)` (reasoning chain steps taken on goal/thread $g$ since the last resolved sub-expectation or non-trivial belief update `|Δμ| > ε_b`), `n_fail(g)` (repeated identical `failed` / `refused` / `unknown` receipt or content-miss count on $g$), existing budget constant `B_steps`, and `ρ_stakes(g) ∈ [0, 1]` (`A-13a` `R-08`).

$$D_{\text{switch}}(g, t) = \big(1 - \lambda_{\text{stakes}}\cdot \rho_{\text{stakes}}(g)\big) \cdot \left(1 - \exp\!\left(-\frac{n_{\text{stag}}(g) + w_{\text{fail}} \cdot n_{\text{fail}}(g)}{\tau_{\text{sw}} \cdot \max(1,\, B_{\text{steps}})}\right)\right)$$
- **Edge-case guarantee:** If $n_{\text{stag}} = 0 \land n_{\text{fail}} = 0$, $D_{\text{switch}}(g, t) = 0$ identically regardless of `ρ_stakes`.
- **Opportunity-cost semantics:** Repeated dead-ends amplify stagnation by `w_fail`, pushing exploration/switching; high-stakes promises (`ρ_stakes = 1.0`, `λ_stakes = 0.5`) resist premature abandonment while still saturating cleanly if genuinely blocked.
- **Existing term fed:** Feeds **existing reasoning step-termination / thought-act selection** (`reasoning/spec.md`, `design.md` D9):
  - When $D_{\text{switch}}(g, t) \ge \theta_{\text{sw}}$ (start `0.70`, range `0.45–0.90`), reasoning pops/suspends leaf sub-goal $g$ toward its parent (`parent_goal_id`, `A-13a` `R-07`) or emits an `ask` / `search` / alternative-door thought act instead of burning remaining `B_steps` on a dead loop. Effectively reduces remaining single-strategy step allowance via existing budget guard `B_eff = ceil(B_steps · (1 - w_sw · D_switch))`.

---

### 3. Drive 3 — Curiosity Drive $D_{\text{curiosity}}(g, t) ∈ [0, 1]$ (*`universal`*, reconciling `A-14` & `R-31`)
Integrates **`A-14`'s internal own-gap (`FoK × relevance + surprise`)** with **`R-31`'s Bayesian Value of Sample Information (VoI)** over pending Beta beliefs (`beliefs/spec.md`, `design.md` D10):

For each pending belief $b \sim \mathrm{Beta}(\alpha_b, \beta_b)$ with pseudo-count $n_b = \alpha_b + \beta_b > 0$ linked to an active goal/decision:
- **Exact 1-step Expected Variance Reduction (EVSI) identity:**
  $$\Delta\mathrm{Var}_1(b) = \mathrm{Var}(b) - \mathbb{E}_{x_1}\!\big[\mathrm{Var}(b \mid x_1)\big] = \frac{\alpha_b \beta_b}{n_b^2 (n_b + 1)^2}$$
  *(Normalized by maximum at $\mathrm{Beta}(1,1)$ where $\Delta\mathrm{Var}_1 = 1/36$: $36\,\Delta\mathrm{Var}_1(b) \in (0, 1]$ vanishes cleanly both when $p_b \to 0/1$ and when $n_b$ is already large, capturing **Gottlieb et al., 2013** epistemic uncertainty + learnability.)*
- **Pending decision relevance:**
  $$\mathrm{impact}(b) = \max_{g \in \mathcal{G}_{\text{active}}} \Big(\mathbb{1}[b \in \mathrm{deps}(g)] \cdot \rho_{\text{stakes}}(g) \cdot \big(1 - |2\, p_b - 1|\big)\Big), \qquad p_b = \frac{\alpha_b}{\alpha_b + \beta_b}$$
- **Unified Curiosity Drive:**
  $$D_{\text{curiosity}}(g, t) = \mathrm{clamp}\!\Big(\lambda_{\text{fok}} \cdot \mathrm{rel}(g) \cdot 4\,\mathrm{FoK}_t(1 - \mathrm{FoK}_t) + \lambda_{\text{surp}} \cdot \bar{S}_{\text{exp}} + \lambda_{\text{voi}} \cdot \max_{b \in \mathcal{B}_{\text{pend}}} \big[36\,\Delta\mathrm{Var}_1(b)\cdot \mathrm{impact}(b)\big],\; 0,\; 1\Big)$$
  where `rel(g)` is active goal relevance and $\bar{S}_{\text{exp}}$ is recent prediction-error surprise EMA (`A-14`). Note: peak `4·FoK(1-FoK)` captures proximal learning competence (`Gottlieb et al. 2013`).
- **Existing term fed:**
  1. Feeds **existing reasoning thought-act preference** (`ask` / `search` / `recall`) and **R-30 internal candidate benefit `B_raw(m, t)`** (`w_cur · D_curiosity`).
  2. Populates the **existing `info_gap` gate signal slot strictly for self-generated idle probes** (incoming external events exclusively keep `A-14` `dialogue_act_gap` to prevent cross-talk).

---

### 4. Drive 4 — Effort Cost & EVC Deliberation Control $D_{\text{effort}}(t) ∈ [0, 1]$ (*`universal`*)
Implements **Shenhav, Botvinick & Cohen (2013)** EVC + **Kurzban et al. (2013)** time/capacity opportunity cost across core's existing discrete deliberation intensities (chain depth `k ∈ {0, ..., B_steps}` and recall count `n_recall ∈ {0, ..., N_rec,max}`):

- **Ambient Opportunity Cost $\rho_{\text{opp}}(t)$:**
  $$\rho_{\text{opp}}(t) = \mathrm{clamp}\!\Big(\frac{|\mathrm{Queue}_{\text{pending}}(t)|}{Q_{\text{ref}}} + \eta_{\text{sl}}\cdot\max(0, D_{\text{sleep}}(t) - 1) + \eta_{\text{tr}}\cdot\mathrm{talk\_rate}(t),\; 0,\; 2\Big)$$
  Tracks foregone responsiveness in the 1-queue single-consumer turn loop (`ARCHITECTURE.md`).
- **Super-linear Control Cost & Marginal EVC of step $k \to k+1$:**
  $$u(k, n_{\text{rec}}) = \frac{k}{B_{\text{steps}}} + w_{\text{rec}}\cdot \frac{n_{\text{rec}}}{N_{\text{rec,max}}}, \qquad \mathrm{Cost}_{\text{ctrl}}(u, t) = c_0 \cdot \big(1 + \gamma_{\text{opp}}\cdot\rho_{\text{opp}}(t)\big) \cdot u^{\,\alpha_{\text{pow}}}$$
  $$\Delta\mathrm{Benefit}(k, t) = \rho_{\text{stakes}}(g) \cdot (1 - \mathrm{conf}_t) \cdot e^{-k / \tau_k}$$
  $$\mathrm{EVC}_{\text{marg}}(k, t) = \Delta\mathrm{Benefit}(k, t) - \Delta\mathrm{Cost}_{\text{ctrl}}(k, t)$$
  $$D_{\text{effort}}(t) = \mathrm{clamp}\!\left(\frac{\mathrm{Cost}_{\text{ctrl}}(u, t)}{\max(\epsilon,\, \rho_{\text{stakes}}(g) + c_0)},\; 0,\; 1\right)$$
  Verified across regimes (`B_steps=8, c0=0.25, α_pow=1.6, τ_k=3.0, γ_opp=0.75`):
  - Low-stakes trivial (`stakes=0.1, conf=0.7, ρ_opp=0.3`) stops at **`k* = 1`**;
  - Low-stakes uncertain (`stakes=0.1, conf=0.2`) stops at **`k* = 3`**;
  - Medium-stakes uncertain (`stakes=0.5, conf=0.5`) stops at **`k* = 5`**;
  - High-stakes low-confidence quiet (`stakes=1.0, conf=0.2, ρ_opp=0.05`) reaches **`k* = 8`**, trimming to **`k* = 7`** when busy (`ρ_opp=1.8`); high-stakes already confident (`0.9`) trims to **`k* = 4`**.
- **Existing term fed:** Feeds **existing reasoning loop `stop` rule & `report_control` (`n_recall` budget)** (`reasoning/spec.md`, `design.md` L284–287): reasoning stops / emits act as soon as $\mathrm{EVC}_{\text{marg}} \le 0$ (or `conf_t >= P_rc`), unifying step depth and recall depth under one cost-benefit law without adding a primitive.

---

### 5. Calibration Procedures (where literature gives no universal constants)
Because neural/biological timescales (`18.2 h`, EEG SWA, RT/reward units in lab tasks) do not transfer directly to multi-timescale software/robot agents:
1. **`D_sleep` (`α_u, N_min, D_sleep^max`):** Replay synthetic 7-day mixed bursty/sparse logs; sweep grid minimizing `episode_consolidation_lag` subject to `< 2%` mid-conversation false sleep interruptions.
2. **`D_switch` (`τ_sw, w_fail, θ_sw, λ_stakes`):** Run Lab dead-end tool trap vs long productive multi-step vs high-stakes promise suites; select Pareto point with `>= 90%` escape within `ceil(0.6·B_steps)` on dead-ends and `<= 5%` premature drop on solvable promises.
3. **`D_curiosity` (`λ_fok, λ_surp, λ_voi`):** Run uncertain decision probes (`Beta(2,2)` high-stakes vs `Beta(20,2)` settled vs unlearnable noise); tune weights maximizing downstream decision accuracy per `ask` turn.
4. **EVC `D_effort` (`c0, γ_opp, α_pow, τ_k`):** Sweep on joint trivial-chitchat + high-stakes verification + queue-burst stress suite; optimize `decision_accuracy − λ_lat · mean_queue_wait_s`.

**Parameters.**

| Parameter | Drive / Controls | Start value | Calibration / Search range | Unit | Scope |
|---|---|---:|---|---|---|
| `α_u` | Sleep backlog vs wake-time weight | `0.85` | `0.50 – 1.00` (Lab) | ratio `[0,1]` | universal |
| `τ_wake` | Wake-time pressure time constant | `24` | `8 – 72` | hours | universal |
| `D_sleep^max` | Max speed-up factor on `T_idle` | `2.0` | `1.2 – 3.0` | multiplier | universal |
| `λ_sleep,init` | Sleep backlog penalty on `R-30` initiative | `0.20` | `0.00 – 0.50` | utility | universal |
| `τ_sw` | Switching horizon relative to `B_steps` | `0.50` | `0.25 – 1.00` | ratio | universal |
| `w_fail` | Extra stagnation weight per repeated failure | `2.0` | `1.0 – 4.0` | steps / fail | universal |
| `λ_stakes` | Switching resistance under high stakes | `0.50` | `0.00 – 0.80` | ratio `[0,1]` | universal |
| `θ_sw`, `w_sw` | Switch threshold & budget compression | `0.70`, `0.50` | `0.45–0.90`, `0.2–0.7` | ratio `[0,1]` | universal |
| `λ_fok`, `λ_surp`, `λ_voi` | Curiosity (`FoK×rel`, surprise, Beta VoI) | `0.40`, `0.30`, `0.40` | `0.0–0.8` each (Lab) | weight | universal |
| `c0` | EVC base control cost scaling (`u^α_pow`) | `0.25` | `0.10 – 0.60` (Lab) | utility | universal |
| `γ_opp` | Sensitivity to queue/talk opportunity cost | `0.75` | `0.20 – 1.50` | ratio | universal |
| `α_pow` | Super-linear EVC cost exponent | `1.6` | `1.3 – 2.5` | exponent | universal |
| `τ_k`, `w_rec` | Step marginal return decay & recall cost weight | `3.0`, `0.50` | `1.5–6.0`, `0.2–1.0` | steps, ratio | universal |
| `η_sl`, `η_tr` | Opportunity-cost weights on sleep/talk rate | `0.30`, `0.50` | `0.0–0.8`, `0.1–1.0` | weight | universal |

**Neutral value.**
- Sleep: `α_u = 1.0, D_sleep^max = 1.0, λ_sleep,init = 0.0` (exact fixed `T_idle` + `U >= N_min`, zero `R-30` penalty, `Δθ_sleep = 0`).
- Switching: `θ_sw = 1.1` (unreachable) and `w_sw = 0.0` (uses fixed `B_steps`).
- Curiosity: `λ_fok = λ_surp = λ_voi = 0.0` (`D_curiosity = 0.0`).
- Effort / EVC: `c0 = 0.0` (marginal cost `0 ->` uses fixed `B_steps` / `P_rc` stop limits).

**Claim.** Mapping all four opportunity-cost drives onto existing loop/gate/stop terms (with zero new primitives) will (a) clear `>= 90%` of dead-end tool loops within `<= 60%` of `B_steps` while preserving `>= 95%` completion on high-stakes promises, (b) cut thinking latency on low-stakes trivial queries (`k*=1–2` vs `8`) while keeping high-stakes accuracy non-inferior, and (c) trigger sleep consolidation earlier under backlog spike without ever falsely suppressing an incoming direct user message (`Δθ_sleep = 0`).

**How the Lab would test it.**
1. *Dead-end vs promise trap (`D_switch`):* Run paired deterministic episodes: (i) unsolvable tool returning repeated `failed` (`n_fail` increments), (ii) high-stakes deep chain returning sub-expectation wins (`n_stag` resets). Verify dead-end pops/switches early (`D_switch >= 0.70`) while solvable promise completes without drop.
2. *EVC stakes × queue-load probe (`D_effort`):* Feed `(stakes=0.1, conf=0.7)`, `(stakes=1.0, conf=0.2, quiet)`, and `(stakes=1.0, conf=0.2, busy queue)` into the cognitive loop; verify exact stopping depths `k* ∈ {1, 8, <=7}` respectively.
3. *Beta VoI curiosity probe (`D_curiosity`):* Compare pending decisions dependent on `Beta(1.5, 1.5)` vs `Beta(20, 2)` vs zero-FoK unlinked items; verify `36·ΔVar_1(b)·impact(b)` ranks uncertain pending high-stakes gap strictly highest (`>= 5×` settled belief).
4. *Sleep backlog + `A-04` gate invariance (`D_sleep`):* At `U = 2 · N_min`, inject direct incoming user event after `T_idle / 2`: assert incoming event is NEVER dropped by sleep (`Δθ_sleep = 0`), while during pure idle (`U >= N_min`) consolidation starts at `T_idle / D_sleep` and suppresses low-utility `R-30` chatter.

**Established / inferred / open.**
- *Established:* Fatigue and boredom track temporal opportunity costs of offline replay (`EVB = Need × Gain`) and external exploration respectively rather than metabolic resource depletion (`Agrawal et al., 2022`; `Mattar & Daw, 2018`; `Kurzban et al., 2013`); cognitive control intensity selection maximizes `EVC = E[Payoff | signal] − Cost(signal)` with super-linear intensity cost (`Shenhav, Botvinick & Cohen, 2013`); epistemic exploration prioritizes intermediate competence/learnability and Expected Value of Information (`Gottlieb et al., 2013`); sleep regulation tracks accumulating homeostatic need across wake intervals (`Borbély, 1982`; `Daan, Beersma & Borbély, 1984`).
- *Inferred:* Core-native closed-form formulas (`D_sleep`, `D_switch`, `D_curiosity` combining `A-14` + Beta 1-step EVSI `αβ / [n²(n+1)²]`, and EVC `ΔBenefit − ΔCost` over `(k, n_recall)`) and mapping onto zero new primitives.
- *Open / not settled by literature:* Exact numerical utility exchange rates `(c0, γ_opp, λ_voi, τ_sw, α_u)` across synthetic tasks, which literature leaves unconstrained and must be calibrated per deployment regime in Lab sweeps.

**Sources.**
- Agrawal, M., Mattar, M. G., Cohen, J. D., & Daw, N. D. (2022). *The temporal dynamics of opportunity costs: A normative account of cognitive fatigue and boredom*. *Psychological Review*, 129(3), 564–585. `[PR]` DOI: `10.1037/rev0000309`, PMID: `34383523`.
- Mattar, M. G., & Daw, N. D. (2018). *Prioritized memory access explains planning and hippocampal replay*. *Nature Neuroscience*, 21(11), 1609–1617. `[PR]` DOI: `10.1038/s41593-018-0232-z`.
- Kurzban, R., Duckworth, A., Kable, J. W., & Myers, J. (2013). *An opportunity cost model of subjective effort and task performance*. *Behavioral and Brain Sciences*, 36(6), 661–679. `[PR]` DOI: `10.1017/S0140525X12003196`.
- Shenhav, A., Botvinick, M. M., & Cohen, J. D. (2013). *The Expected Value of Control: An Integrative Theory of Anterior Cingulate Cortex Function*. *Neuron*, 79(2), 217–240. `[PR]` DOI: `10.1016/j.neuron.2013.07.007`.
- Gottlieb, J., Oudeyer, P.-Y., Lopes, M., & Baranes, A. (2013). *Information-seeking, curiosity, and attention: computational and neural mechanisms*. *Trends in Cognitive Sciences*, 17(11), 585–593. `[PR]` DOI: `10.1016/j.tics.2013.09.001`.
- Borbély, A. A. (1982). *A two process model of sleep regulation*. *Human Neurobiology*, 1(3), 195–204. `[PR]` PMID: `7185792`, `https://pubmed.ncbi.nlm.nih.gov/7185792/`.
- Daan, S., Beersma, D. G. M., & Borbély, A. A. (1984). *Timing of human sleep: recovery process gated by a circadian pacemaker*. *Am. J. Physiol.-Regul. Integr. Comp. Physiol.*, 246(2), R161–R183. `[PR]` DOI: `10.1152/ajpregu.1984.246.2.R161`.
