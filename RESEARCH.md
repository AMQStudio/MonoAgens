# Research — Master Formula & Mechanism Index

Every formula, safety mechanism, kernel principle, and protocol contract behind MonoAgens' **Core**, **Valve**, and **Gateway**: what each symbol means, its verified literature source, and where full ablation derivations live. [`ARCHITECTURE.md`](ARCHITECTURE.md) states *what* the architecture guarantees (strict invariants owned by the repository owner); this index summarizes *how* and *why* each subsystem operates so both human reviewers and coding agents stay synchronized.

- **Full Research Compendia:** [`2026-11-15-run-1.md`](research/findings/2026-11-15-run-1.md) (`R-01`–`R-40` initial baseline + `R-28` citation audit) · [`2026-11-16-run-2.md`](research/findings/2026-11-16-run-2.md) (`A-01`–`A-16` spec audit & open-note resolutions + `R-29`–`R-50` full stack) · [`run-2-notes/`](research/findings/run-2-notes) (`15` detailed technical reports).
- **Active OpenSpec Changes:** [`core-and-lab`](openspec/changes/core-and-lab/design.md) (`22` Brain Map functions) · [`waist-valve-and-bodies`](openspec/changes/waist-valve-and-bodies/design.md) (`monoagens.body.v1`, Cedar Valve, Gateway, Conformance TCK).
- **Source Tags:** `[PR]` peer-reviewed · `[PP]` preprint · `[IND]` industry specification, crate, standard, or regulation. Parameters in `code font` are calibratable prior defaults; **start values** sit in bounded ranges tuned deterministically in `monoagens-lab`.

---

## 1. Design Stance, Kernel Principles & Section Crosswalk

We model **resource-rational cognitive functions** [Lieder & Griffiths 2020, `[PR]`] across the Soar/CoALA perception–appraisal–memory–action–consolidation cycle [Laird, Lebiere & Rosenbloom 2017, `[PR]`; Sumers et al. 2024, `[PR]`]. The main loop, task queues, and go/no-go gate already form a global workspace with ignition [Mashour et al. 2020, `[PR]`]; `reason` (the LLM) runs strictly *after* the sub-millisecond content-blind Core gate judges a moment worth waking [Liu et al. 2026 TGL `arXiv:2605.30152`, `[PP]`].

**Kernel Systems Principles (`§16` legacy):**
- **Minimality & Mechanism vs. Policy:** Only what cannot live outside stays inside Core [Liedtke 1995, `[PR]`]; policies (people, privacy, roles, quiet hours) belong strictly to Gateway & Valve [Levin et al. 1975; Wulf et al. 1974, `[PR]`].
- **Complete Mediation & Ports:** Every percept, context plan, and action passes deterministic doors [Saltzer & Schroeder 1975; Wright et al. 2002 LSM, `[PR]`], while devices & text indexes sit behind explicit ports [Cockburn 2005, `[IND]`].
- **Event Sourcing & Process Isolation:** Every mutation is an immutable versioned event enabling deterministic replay [Fowler 2005, `[IND]`], and external bodies run across process/RPC isolation (`Waist 2`) with an unbreaking ABI.

> **Architecture & Legacy Section Crosswalk (`§§1–26` → `§§1–11`):** Early documents (`architecture.md`, `host`, `dispatch door`, Core `"owner"`, Core text search) were reconciled to [`ARCHITECTURE.md`](ARCHITECTURE.md) (`Gateway`, `Valve`/`Doors`, `Waist 1`, `Waist 2`, per-actor Gateway weights, `Store` ID-returning text search). Where [`core-and-lab/design.md`](openspec/changes/core-and-lab/design.md) cites `§§1–26` numbers, map them as follows:
>
> | Legacy `§` | Current Section | Topic |
> |---|---|---|
> | `§1`, `§15`, `§16`, `§26` | `§1`, `§3`, `§10` | Design stance, generalization (`3/n`, chunking), kernel principles, ablations/NI |
> | `§2`–`§4`, `§22`–`§24` | `§2`, `§3`, `§4` | Signals, `deg(e, l)`, habituation, drives, go/no-go, patience, inner thought, opportunity costs |
> | `§5`, `§14`, `§25` | `§5` | ACT-R Petrov, source monitoring, Yonelinas `R/F`, cognitive maps, habits/conventions (`R-32`), long logs |
> | `§6`, `§17`, `§19`, `§20` | `§6` | Expectations, free-text wake gate, grounding, `effect` vs `response` predictions, judging silence |
> | `§7`, `§8`, `§18` | `§7` | Beta credence, bounded learning (`w_k`), ToM perception ledger & 4-pillar `self_card` |
> | `§12`, `§21` | `§8` | Snapshot-isolated sleep, `{open, provisional, sealed}`, `[0,1]` EM-LLM boundaries, Hungarian clusters, skills |
> | `§9`–`§11` | `§9` | Cache prefix ordering (`~88%` replay hit), Absolute Lines 1 & 2, Cedar Valve, `H_act`, taint, identity anchor |
> | `R-36`–`R-50` | `§10`–`§11` | Cross-body identity, attachments, `monoagens.body.v1`, quiet hours/digests, honest delivery, TCK & open lab calibration |

---

## 2. Core Signals (`s_k ∈ [0, 1]`)

Throughout this index, $\sigma(x) = \frac{1}{1 + e^{-x}}$. Every signal computes deterministically inside Core without reading raw message bytes or calling an LLM:

1. **Direct Mention / Name (`own_name`):** `1` if Gateway's adapter flagged the soul's identifier while unallocated, else `0` (attention capture [Moray 1959; Röer & Cowan 2021, `[PR]`]).
2. **Information Gap (`info_gap` — Split Concepts, `A-14`):**
   - **External Request (`dialogue_act_gap`):** Computed at $O(d)$ from Gateway's frozen contrastive logistic head over prefixed embeddings: $\sigma(\mathbf{w}_{\text{gap}} \cdot \mathbf{e}_d(t_{\text{clf}}; x) + b_{\text{gap}})$ [Snell et al. 2017; Tunstall et al. 2022, `[PP]`], replacing positive-only cosine matching against `references.json`. Low speech ASR confidence (`asr_conf < 0.45`) adds a clarification cue (`R-37`).
   - **Internal Epistemic Gap (`own_info_gap`):** Raised purely from low Beta credence when a pending high-stakes decision depends on uncertain beliefs (`R-31`).
3. **Feeling of Knowing (`feeling_of_knowing`):** $\sigma((A^* - \text{fok\_threshold}) / \text{fok\_scale})$ over top retrieval activation $A^*$ [Nelson & Narens 1990, `[PR]`; `arXiv:2605.14186`, `[PP]`].
4. **Relevance (`relevance`):** $\max \cos(\mathbf{v}, \mathbf{u})$ across vectors $\mathbf{u}$ of active Expectations and visible memories (`VisibleScope`, `R-33`), using unified **`256d` `EmbeddingGemma 2`** (`Apache 2.0`, `arXiv:2610.37364`) tagged by `(embed_model_ver, dim_d, prompt_regime_id)` (`A-15`, `R-37`).
5. **Graded Addressivity (`deg(e, l) ∈ [0, 1]`, `R-29`):** Independent noisy-OR address probability combining explicit mentions, open reply/prediction links, dyadic turn momentum (diluted across group broadcast turns), and optional embodied `orient(e, l)` gaze/distance [Goffman 1979; Bohus & Horvitz 2009, `[PR]`; Mori et al. 2026 `arXiv:2607.15648`, `[PP]`]:
   $$\text{deg}(e, l) = 1 - \prod_{i \in \{\text{expl}, \text{pred}, \text{mom}, \text{orient}\}} (1 - c_i(e, l))$$
6. **Social Cost (`social_cost ∈ [0, 1]`, `R-02` + `R-29`):**
   $$\text{social\_cost}(e) = \max\!\Big(\text{clamp}\big(\max_{l \neq \text{soul}} \text{deg}(e, l) - \text{deg}(e, \text{soul}),\; 0,\; 1\big),\; \text{answered}(e)\Big) \cdot (1 - \lambda_c \cdot \text{closeness}(\text{sender}(e)))$$
   where $\text{closeness}(p) = \sigma(B_p)$ over exchanges with $p$ (neutral `0` on zero history), modulated by $\lambda_c \in [0, 1]$.

*(Multimodal & continuous-sensor bodies also pass Gateway Tier-1 novelty and a 10-element structural `f_modal` vector `{modality_slot, speech_active, vad_prob, speaker_cluster_id, pitch_z, energy_z, pause_ms, turn_end_prob, noise_floor, asr_conf}` — zero emotion classification labels, zero `[CHAT]` side effects (`R-36`, `R-37`).)*

---

## 3. Salience, Partitioned Habituation, Drives & Go/No-Go Gate

1. **Salience (`S`):** $S = \text{clamp}(\sum_k w_k \cdot s_k, 0, 1)$ [Menon & Uddin 2010; Menon 2011, `[PR]`]. Knob weights carry **exact reachable neutrals** (`0` when disabled, `A-01`): e.g., $w_{\text{sc}} = -\text{restraint\_scale} \cdot \text{restraint}$ (`0` at `restraint = 0`), never an unreachable offset `−(0.2 + 0.6ρ)`.
2. **Post-Evaluation Partitioned Habituation (`S_eff`, `A-03` + `R-36`):** Partitioned by `(place p, source_channel k_src)` so background chatter or noisy sensors never silence alarms, and evaluated **before** incrementing so stimulus 1 is never halved:
   $$h_{\text{pre}} = h_{(p, k_{\text{src}})} \cdot \gamma^{\Delta t_{\min}}, \qquad S_{\text{eff}} = \frac{S}{1 + \kappa \cdot h_{\text{pre}}}, \qquad h_{(p, k_{\text{src}})} \leftarrow h_{\text{pre}} + 1$$
   Neutral `κ = 0` yields $S_{\text{eff}} = S$ identically (`κ = 1.0` default; `boredom_with_repeats` maps $\gamma$: `0 → 0.99/min`, `1 → 0.8/min` [Thompson & Spencer 1966; Staddon & Higa 1996, `[PR]`]).
3. **Threshold & Opportunity-Cost Drives (`θ`, `R-03`, `R-30`, `R-31`, `A-04`, `A-13`):**
   $$\theta = \theta_0 + a \cdot \text{talk\_rate} + b \cdot \text{budget\_used} - c \cdot \text{promise\_debt} + r \cdot (1 - \text{familiarity}) + \Delta\theta_{\text{init}}$$
   - **Disposition Mappings & Neutrality:** `talkativeness` maps $\theta_0$ (`0 → 0.85`, `1 → 0.35`); `a` hits exact `0` when restraint is off; `Δθ_sleep = 0` permanently because async sleep reads an immutable `t_snapshot` and `"or asleep"` is deleted (`A-04`). Basic-needs homeostatic modulation without simulated emotion [Wang et al. 2023 Humanoid Agents / Borbély 1982, `[PR]`].
   - **Familiarity (`A-01`, `R-04`):** $\text{familiarity} = \sigma(B_{\text{ctx}})$ where $B_{\text{ctx}} = \frac{B_P + \sum_{p \in \text{Present}} B_p}{1 + |\text{Present}|}$ (neutral `0` with no history/attendees), shrinking unfamiliar places gracefully via report control without special rules.
   - **Undiluted `promise_debt` (`R-10`, `R-46`, `R-49`):** Sum of urgency weights $\sum \min(1, w_p)$ over promises due within `H_debt` (excluding passive `RecurrenceSpec` polls unless overdue/active).
   - **`talk_rate`:** Communicative acts over `talk_rate_window_minutes` divided by active speakers ($\max(1, n)$ vs fair-share `1/n` assumption [Eckhaus, Berger & Stanovsky 2025, `[PR]`]) with cold-start shrinkage (`A-07`).
   - **Opportunity-Cost Drives (`R-31`):** Sleep pressure (`unconsolidated >= N_min`), switching (`exploration_pressure` on stalled threads), curiosity (`own_info_gap` × stakes), and effort/control cost (`EVoC` scaled by stakes & queue load) map onto existing gate/reasoning budgets without emotion states [Kurzban et al. 2013; Shenhav et al. 2013; Agrawal et al. 2022; Gottlieb et al. 2013, `[PR]`].
   - **Idle Initiative Cooldown (`Δθ_init`, `R-30`):** Involuntary cues follow Berntsen (`2021`) / Christoff et al. (`2016`), but unprompted initiatives raise cooldowns and deliver strictly at `EM-LLM`/subtask boundaries rather than arbitrary quiet spells [Horvitz 1999, `[PR]`; Pu et al. 2025 `arXiv:2502.18658`, `[PR]`; Yang et al. 2026 ProACT `arXiv:2607.03730`, `[PP]`].
4. **Seeded Go/No-Go & Circuit Breaker:**
   - $P = \sigma((S_{\text{eff}} - \theta) / \tau)$ (`spontaneity` maps $\tau$: `0 → 0.02`, `1 → 0.2`) with explicit step limit $S_{\text{eff}} \ge \theta$ as $\tau \to 0$ (`A-01`). RNG seeded with `hash(seed, event.id)` outputs `go` if $u < P$, else `note` ($P \ge \text{note\_band}$) or `silent`.
   - **Circuit Breaker:** Unconditional bypass (`P = 1`, no RNG draw) on turns allocated to the soul, active Expectation/prediction firings or answers, or high-stakes active goal continuations [Corbetta & Shulman 2002; Miller & Cohen 2001, `[PR]`].
5. **Backlog Chunking & Statistical Stopping (`§15` legacy, `R-14`, `A-04`):** Triggers only when `now − t_event > patience_place` or events arrived while Core was genuinely `stopped/down` (never while sleep runs). Appraises up to working-memory chunk spans [Cowan 2001, `[PR]`] via event `max` with $(1 + L_{\text{hours}})^{-d}$ decay; zero-failure statistical gating uses the exact Beta/rule-of-three bound $p_{\text{upper}} \approx 3/n$ [Hanley & Lippman-Hand 1983, `[PR]`].

---

## 4. Patience, Inner Thought & Calibrated Reasoning Reports

1. **Shrinkage Patience (`wait`, `R-08`, `A-08`):** Unallocated `go` turns create a `wait` Expectation sized by a right-censored shrinkage quantile/survival estimator $\text{clamp}(t_p, [t_{\min}, t_{\max}])$ per `(place, medium)` (`0` when disabled), cancelling cleanly if a third party replies first [Sacks, Schegloff & Jefferson 1974, `[PR]`; Khan et al. 2026 CAPA `arXiv:2609.03923`, `[PP]`].
2. **Reasoning Reports (`[0, 20]` Scale & `R-15`, `A-10`):**
   - Reasoning evaluates the 8 `Inner Thoughts` dimensions (`relevance`, `info_gap`, `expected impact`, `urgency`, `coherence`, `originality`, `balance`, `dynamics` [Liu et al. 2025, `[PR]`]) while keeping motivation temporal buildup in Core (`talk_rate`), and emits balanced `1–5` motivation against `speaking_bar` plus an anchored **integer `0..20`** confidence (`c_raw = k / 20`, avoiding LLM clustering at `0.85–1.0`) calibrated per domain under strict `R | null` schemas [Singh et al. 2026 `arXiv:2604.25359`, `[PP]`].
   - **Metacognitive Report Control (`R-15`, `A-01`):** $P_{\text{rc}} = \text{clamp}(\text{cost\_error} / (\text{value} + \text{cost\_error}),\; \text{certainty\_bar},\; 0.99)$ (defaulting `0/0` → `certainty_bar`). If $c < P_{\text{rc}}$, Core triggers explicit `memory.recall` with synthesized claim-entity queries; if still below bar, chat-only coarsens grain or emits a `withheld` event logging both raw and calibrated confidence [Koriat & Goldsmith 1996, `[PR]`]; embodied bodies hold/defer rather than guessing.
   - **Citation Lock (`A-10`):** Every grounded claim MUST cite existing `event_id`/`mem_id` handles in Core's snapshot with verified `basis ∈ {observed, told, inferred, self_caused}`.

---

## 5. Memory Activation (ACT-R), Dual-Process Recall, Cognitive Map & Storage

1. **Guarded Petrov Hybrid Base-Level Activation (`B`, `R-04`, `A-01`, `A-07`):**
   Replacing Round 1's non-monotonic single-power approximation, Core implements exact **Petrov (2006) hybrid (`k = 1` exact + power-law tail)**:
   $$B = \ln\!\left( t_1^{-d} + \frac{(n - k)\left(t_n^{1-d} - t_k^{1-d}\right)}{(1 - d)(t_n - t_k)} \right) \quad \text{when } n > k \land t_n > t_k; \qquad B = \ln\!\left(\sum_{j=1}^n t_j^{-d}\right) \text{ otherwise}$$
   with $t_j \ge 1/60\text{ h}$ (`memory_persistence` maps $d$: `0 → 0.7, 1 → 0.3`, default `0.5`) [Anderson & Schooler 1991; Petrov 2006; Fisher, Houpt & Gunzelmann 2018, `[PR]`]. Zero-history `n = 0` yields neutral `familiarity = 0`, shrinking toward body/place priors during cold start (`A-07`).
2. **Total Activation & Fade Threshold (`A-01`, `R-01`):**
   $$A = w_B \cdot B + \text{context\_weight} \cdot \cos(\mathbf{v}_{\text{ctx}}, \mathbf{v}_m) + \text{importance\_weight} \cdot \text{importance}$$
   where explicit weight `w_B` provides Row 10's exact reachable neutral (`w_B = 0` → similarity-only). Sleep vector release checks **base activation $B < \text{fade\_threshold} = -3.29$** (`n=1, t_1=720 h, d=0.5`), never undefined total $A$ (`A-01`). Precursor hybrid ranking follows Park et al. (`2023`, `[PR]`).
3. **Dual-Process Automatic Recall & Source Monitoring (`§14` legacy, `A-09`, `R-16`, `R-17`):**
   $$P(\text{"seen before"}) = R + (1 - R) \cdot P(F > c)$$
   Separates familiarity ($F$: activation + cosine similarity) from threshold recollection ($R$: verbatim entity/place/time match) without gating 30-day familiarity hits behind an LLM check (`A-09`, [Yonelinas 1994, 2002, `[PR]`]). Core asserts specific details only when $R$ holds ($F > c$ with $R = 0$ is *déjà vu* only), and tracks logged provenance `basis` as deterministic source monitoring [Johnson, Hashtroudi & Lindsay 1993; Mitchell & Johnson 2009, `[PR]`]. Full-text retrieval runs strictly behind Gateway's ID-returning `Store` port (`R-12`).
4. **Cognitive Map, Habits & Conventions (`§25` legacy, `R-32`):** Core maintains an edge-weighted co-occurrence + Successor Representation (`SR`) graph purely over opaque `{actor, place, concept}` IDs inside existing books [Behrens et al. 2018, `[PR]`]. Uncertainty arbitration between goal-directed planning and compiled habits (`Daw, Niv & Dayan 2005`, `[PR]`) is scoped to **embodied** fast-paths (`allow_chat_habit_bypass = false` in chat-only); place social conventions learn via conservative delta updates over observed reactions.
5. **Long-Horizon Storage (`R-40`):** Segmented SQLite (`hot.db` WAL + `PRAGMA secure_delete = ON` + read-only sealed temporal shards `ATTACH`ed cleanly), pre-filtered `usearch v2.26.6` HNSW indices, `blake3 v1.8.7` snapshots, and `postcard`/`minicbor` binary encoding (avoiding `bincode` `RUSTSEC-2025-0141`).

---

## 6. Expectations, Routines, Grounding & Agency (`R-07`–`R-10`, `R-36`, `R-46`, `R-49`)

1. **Five Expectation Subtypes (`goal`, `wait`, `predict`, `monitor`, `pending_approval`):** Code-managed intentions guarantee wakeup reliability [Friston et al. 2016, `[PR]`; PIS `arXiv:2609.01272`; Pro-Memory `arXiv:2611.05731`; PM-Bench `arXiv:2607.12385`, `[PP]`]. Records store plural `places` and `people`, `domain?`, optional **`RecurrenceSpec`** (`cron`/`interval`, UTC-logged `due_at`, `miss_policy ∈ {coalesce_one, skip_to_next, catch_up_window}`, zero `promise_debt` distortion, `R-46`), and **`PromiseMeta`** (`promisee_ids`, `hard_deadline`, `depends_on?`, conflict detection, `R-49`).
2. **Lifecycle, Free-Text Wake Gate & Pre-Utterance `CommitCheck` (`A-01`, `R-49`):** `resolve` accepts `{kept, kept_late, renegotiated, released, failed, cancelled, answered, reached}`. Free-text condition matching uses cosine vs shrunk `q_0.10` `wake_gate_lo` (`A-08`) with `wake_gate_hi = 1` promoted only after `n >= 3/ε` clean classifier confirmations (`ε = 0.06 → n = 50` [Hanley & Lippman-Hand 1983, `[PR]`]). Before emitting promises, Core/Valve checks resource/time conflicts (`CommitCheck`) so impossible obligations renegotiate *before* commitment (`R-49`).
3. **Two `predict` Kinds: Grounding (`response`) vs. Agency (`effect`, `R-09`, `R-17`):**
   - **`effect` prediction (`self_caused` comparator, `§19` legacy):** Tracks `(channel, Δ_obs, Δ_pred)` tolerances; matching outcomes mark echoed deliveries/sensor changes as `self_caused` and **suppress** re-appraisal [Frith, Blakemore & Wolpert 2000, `[PR]`].
   - **`response` prediction (conditional relevance, `§17` legacy):** Expects partner acknowledgment/answer; arrival **wakes** reasoning cleanly through the circuit breaker, expiry records a *noticed absence* [Schegloff 1968; Schegloff & Sacks 1973; Clark & Brennan 1991, `[PR]`], and mismatch triggers repair [Enfield et al. 2013, `[PR]`].
4. **Preemption / Barge-In Receipts (`R-36`, `R-37` — Owner Option A):** Keeps `ARCHITECTURE.md` L198's exact 5-outcome enum (`{done, refused, held, failed, unknown}`) while adding optional fields `{termination_reason ∈ {normal, canceled_by_core, preempted_external, timed_out}, progress ∈ [0,1], delivered_units}`.

---

## 7. Credence, Bounded Learning & Perception Ledger (`R-19`–`R-26`, `R-35`, `A-01`, `A-13`)

1. **Beta Reputation (`E = α / (α + β)`):** Initialized from per-actor Gateway priors (`told` items included! fixing `beliefs/spec.md` L56 vs L86); confirmed/refuted evidence increments `α`/`β` discounted by `λ_domain` (`R-20`, [Jøsang & Ismail 2002; Sperber et al. 2010, `[PR]`]). **Critical Guard (`R-36`):** `canceled_by_core` and `preempted_external` receipts NEVER increment `β_c`.
2. **Sign-Corrected, IPS-Weighted & Teacher-Clipped Delta Rule (`R-19`, `R-26`, `A-01`, `A-13`):**
   $$w_k \leftarrow \text{clamp}\!\left(w_k + \alpha_{\text{learn}} \cdot w_{\text{ips}} \cdot \tilde{\eta}_{\text{teacher}} \cdot (r - P) \cdot s_k\right)$$
   - **Polarity Fix (`B-01`):** For wake probability $P = P(\text{go})$, binary target $r = 1$ on `{good_act, missed_silence}` and $r = 0$ on `{bad_act, answered_silence}` (replacing the `core-and-lab` sign inversion where missed silence lowered $P$), weighted by clipped IPS $w_{\text{ips}} = \min(M_{\text{ips}}, 1/\rho)$ on sampled silences [Rescorla & Wagner 1972; Schultz, Dayan & Montague 1997, `[PR]`].
   - **Teacher Reliability & Two-Timescale Bounds (`A-13`):** Fast local layer-3 offsets (`δ_actor`, `δ_place`) separate cleanly from slow global weights (`Borkar 1997` two-timescale separation `[PR]`), external `teacher_weight` $\tilde{\eta}$ tracks Beta concordance against ground-truth outcomes [Dawid & Skene 1979; Raykar et al. 2010, `[PR]`] with clipping, and all updates enforce 95% CI significance, daily `<= 5%` range cap, and freeze alerts.
3. **ToM Perception Ledger & Event-Derived `self_card` (`§18` legacy, `R-25`, `R-35`):** Tracks per-actor witnessed facts (`origin`/audiences, delivered claims, candidate false-belief deltas [Baker et al. 2017; Sclar et al. 2023 SymbolicToM; Jung et al. 2024, `[PR]`; Ullman 2023, `[PP]`]) alongside an audited 4-pillar read-only `self_card` (`(α_c, β_c)` latency/health, `[0,20]` ECE/`meta-d'` calibration, operational budget headroom, log-truth contradiction flags).

---

## 8. Sleep, Episodic Boundaries, Domains, Lessons & Skills (`A-05`, `A-06`, `A-11`, `A-12`, `R-38`)

1. **Snapshot-Isolated Async Sleep (`A-04`, `A-05`):** Triggers when `unconsolidated >= N_min` (with `>=`, not `>`) and idle `>= T_idle`, reading immutable `t_snapshot` with `Δθ_sleep = 0`. Episodes transition **`{open, provisional, sealed}`**: idle pre-sleep tails (`L >= N_ep_min` or `Δt > τ_stale`) soft-close into `provisional` for next-day recall and transparently reopen (marking prior gist `superseded_by`) if resumed (`A-05`).
2. **Normalized Event Boundaries (`§21` legacy, `R-23`, `A-06`):** Combines time surprise, content prediction error, and embedding novelty normalized to `[0, 1]` (`EM-LLM` [Zacks et al. 2007; Fountas et al. 2025, `[PR]`]) with cold-start shrinkage (`A-07`, `n <= 5`). Merging episodes requires overlap across people, place, and time [Yassa & Stark 2011, `[PR]`].
3. **Hungarian-Aligned Domain Clustering (`A-12`):** Sleep warm-starts spherical k-means from previous centroids and aligns cluster IDs via Hungarian matching (`domains_updated` split/merge Beta fusion) so domain reliability history never shuffles across restarts.
4. **Contrastive Lessons & Schema-Hashed Procedures (`A-11`, `R-38`):** `consolidate` clusters repeated opaque failure signatures against paired successes to distill contrastive `lesson`s, traces `goal_id`/`caused_by` chains into parameterized `procedure`s tagged with **`capability_schema_hash`** & Bernoulli CUSUM drift checks, and **retires** degraded skills (`R-38`; [Voyager `arXiv:2305.16291`, `[PP]`; AWM `arXiv:2409.07429`, `[PP]`]). Verbatim-vs-gist verification runs in `reason` (`consolidate`), with logged verbatim events overriding gists [Brainerd & Reyna 2002, `[PR]`].

---

## 9. Privacy (`R-33`), Hard Forgetting (`R-34`), Security (`R-41`–`R-43`), Identity (`R-48`) & Prompting (`R-11`)

1. **Exact Privacy Non-Interference (`R-33`):** Every candidate memory passes **`VisibleScope` projection BEFORE Step-4 recall & Step-5 appraisal** (`audience(context) ⊆ audience(item)` absent an explicit logged `DeclassToken` [Myers & Liskov 1997, `[PR]`; Anderson 1972, `[IND]`]), guaranteeing $\Delta_{\text{NI}} = 0$ (no prompt or urgency leakage).
2. **Atomic `hard_forget` & `Two-Epoch Replay` (`R-34`):** Erases target records, scrubs SQLite (`PRAGMA secure_delete = ON`, FTS5 `'secure-delete'=1`), unlinks ref-counted `BLAKE3` attachments (`R-37`), deletes/re-synthesizes derived summaries, recomputes `(α, β)` solely from surviving evidence, and writes an epoch-bumping clean snapshot so replay never resurrects deleted data (`GDPR` Art. 17 `[IND]`).
3. **Cedar ABAC Valve (`R-41`) & Anti-Fatigue `PendingApproval` (`R-42`):** Valve embeds **`cedar-policy` (`v4.14.1`)** + **`cedar-policy-symcc` (`v0.8.1`)**, deriving ephemeral room roles strictly from `(log_seq, t_now)` snapshots (static SMT check rejects any non-owner `identity_changing` path [Cutler et al. 2024, `[PR]`]). Four risk tiers (`R_0..R_3`) halt `R_2` or tainted tools in non-retrying **`PendingApproval`** (`max_retry_on_held = 0`) pinned to canonical hash **`H_act`** (anti-TOCTOU / Loopjacking [`arXiv:2609.21081`, `[PP]`]) with hourly sliding prompt limits [Horvitz 1999, `[PR]`; Turan 2026 `arXiv:2606.08919`, `[PP]`].
4. **Three-Door Defense Against Poisoning & Prompt Injection (`R-43`):** Zero brittle regexes/perplexity blocklists (`47%–99%` bypass across `MPBench`/`MINJA`/`MemoryGraft`); instead:
   - **Write Door:** Skeptical corroboration (`k_corrob >= 2`) + monotonic `min(trust)` compaction join + `OwnerSigned`-only `procedure` creation;
   - **Context Door:** Immutable `ContextPlan` Spotlighting boundaries (`[SYSTEM]`, `[ANCHOR]`, `[TRUSTED]`, `[UNTRUSTED_QUOTED]` [Hines et al. 2024, `[PR]`]) ordered static-to-volatile (`~88%` prefix cache reuse in replay [OpenAI, `[IND]`]);
   - **Action Door:** Whenever turn taint `< Trusted`, Cedar blocks `>= R_2`, cross-audience, or identity acts absent owner `ApprovalGrant(H_act)` [Debenedetti et al. 2025/2026 CaMeL, `[PR]`; Wallace et al. 2025 `ICLR`, `[PP]`; Shi et al. 2025 Progent, `[PP]`; OWASP Agentic `2026`, `[IND]`].
5. **Identity Continuity Under Long Horizon (`R-48`):** Soul constitution locks under **`OwnerSigned`** (`Absolute Line 1`), pins an immutable `IdentityAnchor` + `PrincipleDescriptor`s in prompt slot `0`, and enforces deterministic **`[ANCHOR_CHECK]`** / pre-act **`PrincipleAudit`** against multi-turn drift [Guan et al. 2024; Cheng et al. 2025 ELEPHANT, `[PP]`].

---

## 10. Gateway, Identity, Waist-2 Bodies, Routines & Standards (`R-36`–`R-40`, `R-44`–`R-50`)

1. **Cross-Body Identity (`R-44`):** Each `(body_id, platform_account_id)` hashes deterministically via length-prefixed `κ(b, a)` to isolated `per_<blake3>` IDs (`T_μ = +∞`, zero heuristic merging [Fellegi & Sunter 1969, `[PR]`]); cross-body linking executes only via owner config or append-only bilateral dual-nonce challenges (`actor_linked`).
2. **Attachment Store & Blind Multimodal/Speech Pipeline (`R-37`):** Gateway computes scope-keyed `blake3::keyed_hash` (`v1.8.7`) and embeds text/image/`audio_voice` uniformly into `256d` via **`EmbeddingGemma 2`**; Body runs `Silero VAD` / `WebRTC VAD`, causal `RAPT`/`YIN` normalization (no `openSMILE`), `Pyannote`, and `VAP`, sustaining **$< 50\text{ ms}$** TTS barge-in mute mapped to Option A `Receipt` (`Full-Duplex-Bench-v2`, `arXiv:2510.07838`).
3. **Waist-2 Protocol (`monoagens.body.v1`, `R-45`, `R-50`):** Stateless JSON-RPC 2.0 ($\le 12$ methods, OpenRPC `1.4.1` + JSON Schema `2020-12`, explicit `idempotency_key`, 3-class queue `256/512/256`). All `22` Brain Map functions (encompassing `~40` internal sub-rules & parameters across `R-01`–`R-50`) sit strictly behind Waist 1 while Gateway bridge plugins wrap **MCP `2026-07-28`**, **A2A `v1.0.1`**, and **ROS 2**.
4. **Valve Quiet Hours, Daily Caps & Honest Delivery (`R-46`, `R-47`):** Valve enforces recipient-local quiet hours (`22:00–07:00`), proactive cap `C_daily`, emergency overrides, and **`3×/day` batched digests** returning explicit `held` receipts (`R-46`). Gateway emits honest in-flight lifecycle cues (`1 s` `working_cue`, `10 s` `progress_note`) with zero fake delays (`d_art = 0 ms`, EU AI Act Art. 50 `[IND]`), while Chat Bodies track `pid_1..pid_M` chunks per `Receipt` (`R-47`).
5. **Benchmarks, NI Power & TCK Rollout (`§26` legacy, `A-02`, `A-16`, `R-39`, `R-50`):** Every one of the **22 Brain Map rows** carries explicit ablation metrics in `monoagens-lab` (`A-02` [Park et al. 2023, `[PR]`]), evaluated via `BaselineAdapter` on Pareto (accuracy vs cost/latency) fronts across 5 domains (`R-39` [Kapoor et al. 2025 `TMLR`, `[PR]`]), powered at **$S \ge 354\text{--}471$** scenarios for paired non-inferiority (`A-16`), and validated externally by a public black-box **`monoagens-conformance` TCK** across interleaved Phases 0–3 (`R-50`).

---

## 11. Open Empirical Calibration Items (Code, Lab & Hardware Required)

All architectural contracts and equations above are settled for Phases 0–3; remaining unknowns are strictly **empirical engineering/UX numbers** deferred to Lab sweeps & target hardware profiling (non-blocking for Phase 0–1 implementation; see [`core-and-lab/design.md`](openspec/changes/core-and-lab/design.md#L407-L419)):

| Open Calibration Item | Run / Scope | Resolution Path |
|---|---|---|
| `EmbeddingGemma 2` ARM/CPU `int4`/`int8` latency & `MultiLexNorm`/`IndoCollex` `256d` vs `512d` (`A-15`, `R-37`) | Gateway & Lab | Profile quantized runtime on target devices; sweep Indonesian/code-switched dev/val sets |
| Indonesian `VAP` (`τ_barge`, `θ_turn_end`) & ergonomic limits (`B_hr` `R-42`, `C_daily` `R-46`, `L_soft`/`d_gap` `R-47`) | Gateway, Valve & UX | Bounded defaults (`6/h`, `8/d`, `0 ms` delay) refined via Lab & controlled user trials |
| Paired `S >= 354–471` Non-Inferiority vs full-transcript `Arm T` & 5-domain Pareto scorecard (`A-16`, `R-39`) | `monoagens-lab` | Run deterministic scripted suites first, then owner-approved live baseline comparison |
| Post-`hard_forget` weight policy (`Two-Epoch Replay` freeze vs full rollback-replay, `R-34`) | Store & Bounded Learning | Evaluate drift sensitivity in Lab replay suites; snapshot-freeze default stays reversible |
| Brain Map row pruning (`A-02`, `R-01`) | `monoagens-lab` | Any of the 22 rows showing zero effect beyond seed variance across 5+ seeds is flagged for owner removal |

---

## Key Sources Index (`[PR]` / `[PP]` / `[IND]`)

Full verified citations (DOIs, arXiv IDs, URLs, crate versions, and license checks through November 2026) are cataloged in:
- [`2026-11-15-run-1.md`](research/findings/2026-11-15-run-1.md) (Round-1 baseline citations + primary-source verification in `R-28`)
- [`2026-11-16-run-2.md`](research/findings/2026-11-16-run-2.md#verified-bibliography-across-all-38-run-2-questions) (Consolidated Run-2 bibliography across `A-01`–`A-16` and `R-29`–`R-50`)
- [`run-2-notes/`](research/findings/run-2-notes) (Detailed proofs, benchmark tables, and primary-source audits per topic)

Core foundational anchors cited above:
- **Cognitive & Memory:** Lieder & Griffiths (2020, `[PR]`); Laird, Lebiere & Rosenbloom (2017, `[PR]`); Sumers et al. (2024 CoALA, `[PR]`); Mashour et al. (2020, `[PR]`); Anderson & Schooler (1991, `[PR]`); Anderson (2007, `[PR]`); Petrov (2006, `[PR]`); Fisher, Houpt & Gunzelmann (2018, `[PR]`); Thompson & Spencer (1966, `[PR]`); Staddon & Higa (1996, `[PR]`); Rescorla & Wagner (1972, `[PR]`); Schultz, Dayan & Montague (1997, `[PR]`); Borkar (1997, `[PR]`); Dawid & Skene (1979, `[PR]`); Raykar et al. (2010, `[PR]`); Jøsang & Ismail (2002, `[PR]`); Sperber et al. (2010, `[PR]`); Nelson & Narens (1990, `[PR]`); Koriat & Goldsmith (1996, `[PR]`); Yonelinas (1994, 2002, `[PR]`); Johnson, Hashtroudi & Lindsay (1993, `[PR]`); Mitchell & Johnson (2009, `[PR]`); Brainerd & Reyna (2002, `[PR]`); Yassa & Stark (2011, `[PR]`); Cowan (2001, `[PR]`); Hanley & Lippman-Hand (1983, `[PR]`); Behrens et al. (2018, `[PR]`); Daw, Niv & Dayan (2005, `[PR]`); Kurzban et al. (2013, `[PR]`); Shenhav et al. (2013, `[PR]`); Agrawal et al. (2022, `[PR]`); Gottlieb et al. (2013, `[PR]`); Borbély (1982, `[PR]`); Menon & Uddin (2010, `[PR]`); Menon (2011, `[PR]`).
- **Interaction, ToM, Proactivity & Evaluation:** Sacks, Schegloff & Jefferson (1974, `[PR]`); Schegloff (1968, `[PR]`); Schegloff & Sacks (1973, `[PR]`); Clark & Brennan (1991, `[PR]`); Enfield et al. (2013, `[PR]`); Goffman (1979, `[PR]`); Bohus & Horvitz (2009, `[PR]`); Frith, Blakemore & Wolpert (2000, `[PR]`); Zacks et al. (2007, `[PR]`); Fountas et al. (2025 EM-LLM, *ICLR*, `[PR]`); Baker et al. (2017, `[PR]`); Sclar et al. (2023 SymbolicToM, *ACL*, `[PR]`); Jung et al. (2024, *EMNLP*, `[PR]`); Ullman (2023, `[PP]`); Horvitz (1999, `[PR]`); Liu et al. (2025 Inner Thoughts, *CHI*, `[PR]`); Pu et al. (2025, *CHI*, `arXiv:2502.18658`, `[PR]`); Eckhaus et al. (2025, *Findings EMNLP*, `[PR]`); Park et al. (2023, *UIST*, `[PR]`); Kapoor et al. (2025, *TMLR*, `[PR]`); Mori et al. (2026, `arXiv:2607.15648`, `[PP]`); Liu et al. (2026 TGL, `arXiv:2605.30152`, `[PP]`); Singh et al. (2026 SOB, `arXiv:2604.25359`, `[PP]`); Khan et al. (2026 CAPA, `arXiv:2609.03923`, `[PP]`); Yang et al. (2026 ProACT, `arXiv:2607.03730`, `[PP]`); `PIS` (`2609.01272`), `Pro-Memory` (`2611.05731`), `PM-Bench` (`2607.12385`), `Voyager` (`2305.16291`), `AWM` (`2409.07429`) (`[PP]`).
- **Security, Policy, Kernel & Standards:** Liedtke (1995, `[PR]`); Levin et al. (1975, `[PR]`); Saltzer & Schroeder (1975, `[PR]`); Wright et al. (2002 LSM, `[PR]`); Anderson (1972, `[IND]`); Myers & Liskov (1997, `[PR]`); Fellegi & Sunter (1969, `[PR]`); Debenedetti et al. (2025/2026 CaMeL, *SaTML 2026*, `[PR]`); Hines et al. (2024 Spotlighting, `[PR]`); Wallace et al. (2025, *ICLR*, `[PP]`); Shi et al. (2025 Progent, `arXiv:2504.11703v3`, `[PP]`); Cutler et al. (2024 Cedar, *OOPSLA*, `[PR]`); Turan (2026, `arXiv:2606.08919`, `[PP]`); Arun Kumar (2026, `arXiv:2609.21081`, `[PP]`); Guan et al. (2024, `[PP]`); Cheng et al. (2025 ELEPHANT, `[PP]`); Schechter Vera et al. (`EmbeddingGemma 2`, `arXiv:2610.37364`, Apache 2.0, `[PP]`); Ekstedt & Skantze (`VAP`, `arXiv:2205.09812`, `[PP]`); Lin et al. (`Full-Duplex-Bench-v2`, `arXiv:2510.07838`, `[PP]`); MCP `2026-07-28`, A2A `v1.0.1`, OpenRPC `1.4.1`, JSON Schema `2020-12`, Cedar `v4.14.1` / `symcc v0.8.1`, `usearch v2.26.6`, `blake3 v1.8.7`, `postcard v1.1.3`, `minicbor v2.3.1`, EU AI Act Art. 50, GDPR Art. 17 (`[IND]`).
