# Research — Master Formula & Mechanism Index

Every formula, safety mechanism, and protocol contract behind MonoAgens' **Core**, **Valve**, and **Gateway**: what each symbol means, its verified literature source, and where full ablation derivations live. [`ARCHITECTURE.md`](file:///home/muqsith/Projects/Mono/ARCHITECTURE.md) states *what* the architecture guarantees (strict invariants owned by the repository owner); this index summarizes *how* and *why* each subsystem operates so both human reviewers and coding agents stay synchronized.

- **Full Research Compendia:** [`2026-11-15-run-1.md`](file:///home/muqsith/Projects/Mono/research/findings/2026-11-15-run-1.md) (`R-01`–`R-40` initial baseline) · [`2026-11-16-run-2.md`](file:///home/muqsith/Projects/Mono/research/findings/2026-11-16-run-2.md) (`A-01`–`A-16` spec audit & open-note resolutions + `R-29`–`R-50` full stack) · [`run-2-notes/`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes) (`15` detailed technical reports).
- **Source Tags:** `[PR]` peer-reviewed · `[PP]` preprint · `[IND]` industry specification, crate, standard, or regulation. Parameters in `code font` are calibratable prior defaults; **start values** sit in bounded ranges tuned deterministically in `monoagens-lab`.

---

## 1. Design Stance & Structural Corrections Over Initial Draft

We model **resource-rational cognitive functions** [Lieder & Griffiths 2020, `[PR]`] matching the canonical Soar/CoALA perception–appraisal–memory–action–consolidation loop [Laird, Lebiere & Rosenbloom 2017, `[PR]`; Sumers et al. 2024, `[PR]`]. The LLM (`reason`) runs strictly after a sub-millisecond, content-blind Core gate judges an event worth waking for [TGL `arXiv:2605.30152`, `[PP]`].

> **Architecture Alignment Notes (Round 1 & Round 2):** Earlier drafts referred to `architecture.md`, `host`, `dispatch door`, and phantom headings; canonical terms follow [`ARCHITECTURE.md`](file:///home/muqsith/Projects/Mono/ARCHITECTURE.md) (`Gateway`, `Valve` / `Doors`, `Waist 1`, `Waist 2`, per-actor Gateway weights instead of Core `"owner"`, opaque Gateway `Store` text-search ports returning IDs, pre-Step-5 audience visibility, and explicit separation of Waist-1's `6` domain payload facts from Waist-2's JSON-RPC lifecycle methods).

---

## 2. Core Signals (`s_k ∈ [0, 1]`)

Every signal is deterministic inside Core without reading raw message bytes or calling an LLM:

1. **Direct Mention / Name (`own_name`):** `1` if Gateway's adapter flagged the soul's identifier while unallocated, else `0` (attention capture [Moray 1959; Röer & Cowan 2021, `[PR]`]).
2. **Information Gap (`info_gap` — Split Concepts, `A-14`):**
   - **External Request (`dialogue_act_gap`):** Computed at `O(d)` from Gateway's frozen contrastive logistic head over prefixed text/modal embeddings: `σ(w_gap · e_d(t_clf; x) + b_gap)` [Snell et al. 2017; Tunstall et al. 2022, `[PP]`], replacing brittle positive-only cosine matching against `references.json`. Low speech ASR confidence (`asr_conf < 0.45`) adds an additive clarification cue (`R-37`).
   - **Internal Epistemic Gap (`own_info_gap`):** Raised purely from low Beta belief credence on a pending high-stakes decision (`R-31`).
3. **Feeling of Knowing (`feeling_of_knowing`):** `σ((A* − fok_threshold) / fok_scale)` over the top retrieval activation `A*` (`[0, 1]` confidence heuristic [Nelson & Narens 1990, `[PR]`]).
4. **Relevance (`relevance`):** `max cos(v, u)` across vectors `u` of active Expectations and visible memories (`VisibleScope`, `R-33`), using the **unified `256d` `EmbeddingGemma 2` vector space** (`Apache 2.0`, `arXiv:2610.37364`) tagged by `(embed_model_ver, dim_d, prompt_regime_id)` (`A-15`, `R-37`).
5. **Graded Addressivity (`deg(e, l) ∈ [0, 1]`, `R-29`):** Instead of single-label speaker selection, Core assigns each active participant `l` an independent noisy-OR address probability combining explicit mentions, open reply/prediction links, dyadic turn momentum (diluted across group broadcast turns), and optional embodied `orient(e, l)` gaze/distance (`R-29`; [Goffman 1979; Bohus & Horvitz 2009, `[PR]`; Mori et al. 2026 `arXiv:2607.15648`, `[PP]`]):
   $$\text{deg}(e, l) = 1 - \prod_{i \in \{\text{expl}, \text{pred}, \text{mom}, \text{orient}\}} (1 - c_i(e, l))$$
6. **Social Cost (`social_cost ∈ [0, 1]`, `R-02` + `R-29`):**
   $$\text{social\_cost}(e) = \max\!\Big(\text{clamp}\big(\max_{l \neq \text{soul}} \text{deg}(e, l) - \text{deg}(e, \text{soul}),\; 0,\; 1\big),\; \text{answered}(e)\Big) \cdot (1 - \lambda_c \cdot \text{closeness}(\text{sender}(e)))$$
   where `closeness(p) = σ(B_p)` over exchanges with `p` (neutral `0` on zero history), lowered by `λ_c ∈ [0, 1]`.

*(Note: Multimodal & continuous-sensor bodies also pass Gateway Tier-1 novelty and a 10-element structural `audio_voice`/prosody vector `{speech_active, vad_prob, speaker_cluster_id, pitch_z, energy_z, pause_ms, turn_end_prob, noise_floor, asr_conf, modality_slot}` — zero emotion classification labels, zero `CHAT` impact (`R-36`, `R-37`).)*

---

## 3. Salience, Partitioned Habituation, Drives & Go/No-Go Gate

1. **Salience (`S`):** `S = clamp(Σ_k w_k · s_k, 0, 1)` [Menon & Uddin 2010; Menon 2011, `[PR]`]. Weights derive from disposition knobs with **exact reachable neutrals** (`0` when disabled, `A-01`): e.g., `w_sc = −restraint_scale · restraint` (`0` when `restraint = 0` or `social_cost_off`), not an unreachable offset `−(0.2 + 0.6ρ)`.
2. **Post-Evaluation Partitioned Habituation (`S_eff`, `A-03` + `R-36`):** Partitioned by `(place p, source_channel k_src)` so high-rate background sensors cannot silence alarms, and evaluated **before** incrementing so stimulus 1 is never halved:
   $$h_{\text{pre}} = h_{(p, k_{\text{src}})} \cdot \gamma^{\Delta t_{\min}}, \qquad S_{\text{eff}} = \frac{S}{1 + \kappa \cdot h_{\text{pre}}}, \qquad h_{(p, k_{\text{src}})} \leftarrow h_{\text{pre}} + 1$$
   Neutral value `κ = 0` gives `S_eff = S` exactly (`κ = 1.0` default; monotonic decay [Thompson & Spencer 1966; Staddon & Higa 1996, `[PR]`]).
3. **Threshold & Opportunity-Cost Drives (`θ`, `R-03`, `R-30`, `R-31`, `A-04`, `A-13`):**
   $$\theta = \theta_0 + a \cdot \text{talk\_rate} + b \cdot \text{budget\_used} - c \cdot \text{promise\_debt} + r \cdot (1 - \text{familiarity}) + \Delta\theta_{\text{init}}$$
   - **Exact Neutrality:** `a` is `0` when restraint is disabled; `Δθ_sleep = 0` permanently in canonical Core because consolidation runs asynchronously on an immutable snapshot (`t_snapshot`) and `"or asleep"` is deleted (`A-04`).
   - **Undiluted `promise_debt` (`R-10`, `R-46`, `R-49`):** Sum of urgency weights `Σ min(1, w_p)` over promises due within `H_debt` (excluding recurring `RecurrenceSpec` polls unless overdue/active), preventing low-priority future promises from diluting imminent obligations.
   - **`talk_rate`:** Group-normalized vs `1/n` fair-share [Eckhaus et al. 2025, `[PR]`] with cold-start shrinkage smoothing (`A-07`).
   - **Opportunity-Cost Drives (`R-31`):** Sleep pressure (`unconsolidated_backlog >= N_min`), switching (`exploration_pressure` on stalled goal), curiosity (`own_info_gap` weighted by stakes), and effort/thinking budget (`EVoC` scaled by stakes & queue load) map onto existing knobs/thresholds without extra emotion variables [Kurzban et al. 2013; Shenhav et al. 2013; Agrawal et al. 2022, `[PR]`].
   - **Idle Initiative Cooldown (`Δθ_init`, `R-30`):** Unprompted or ignored initiatives raise temporary habituation cooldowns and only fire at natural `EM-LLM`/dialogue breakpoints [Horvitz 1999, `[PR]`; Pu et al. 2025 `arXiv:2502.18658`, `[PR]`; ProACT `arXiv:2607.03730`, `[PP]`].
4. **Seeded Go/No-Go & Circuit Breaker:**
   - `P = σ((S_eff − θ) / τ)` with explicit step limit `S_eff >= θ` as `τ → 0` (`A-01`); deterministic RNG keyed on `(seed, event.id)` returns `go` if `u < P`, else `note` (`P >= note_band`) or `silent` (`P < note_band`).
   - **Circuit Breaker:** Unconditional bypass (`P = 1` without RNG draw) when turn is allocated to soul, active Expectation/prediction fires or needs response, or high-stakes active goal continues [Corbetta & Shulman 2002; Miller & Cohen 2001, `[PR]`].
5. **Backlog Chunking (`R-14`, `A-04`, `R-29`):** Triggers only when `now − t_event > patience_place` or events arrived while Core was genuinely `stopped/down` (never while background sleep runs). Combines signals across chunked backlog via event-level `max` with `(1 + L_hours)^(−d)` decay.

---

## 4. Patience, Inner Thought & Calibrated Reasoning Reports

1. **Shrinkage Patience (`wait`, `R-08`, `A-08`):** Unallocated `go` turns create a `wait` Expectation sized by a right-censored shrinkage quantile/survival estimator `clamp(t_p, [t_min, t_max])` per `(place, medium)` (`0` when disabled), cancelling cleanly if a third party replies first [Sacks, Schegloff & Jefferson 1974, `[PR]`; CAPA `arXiv:2609.03923`, `[PP]`].
2. **Reasoning Reports (`[0, 20]` Scale & `R-15`, `A-10`):**
   - Because raw LLM `[0, 1]` confidences cluster heavily near `0.85–1.0`, reasoning emits a verbally anchored **integer `0..20`** plus ordinal stakes and balanced `1..5` motivation (`Inner Thoughts`, [Liu et al. 2025, `[PR]`]); Core converts `c_raw = k / 20`, calibrates per-domain via temperature/histogram bins, and requires strict nullable JSON schemas (`R | null`) [Singh et al. 2026 `arXiv:2604.25359`, `[PP]`].
   - **Metacognitive Report Control (`R-15`, `A-01`):** `P_rc = clamp(cost_error / (value + cost_error), certainty_bar, 0.99)` (guarded `0/0` -> `certainty_bar`). If `c < P_rc`, reasoning triggers one `memory.recall` query first; if still below bar, chat-only coarsens grain or outputs an explicit `withheld` event (both raw + calibrated scores logged) [Koriat & Goldsmith 1996, `[PR]`]. Embodied bodies defer/hold instead of guessing.
   - **Citation Lock (`A-10`):** Every grounded claim MUST reference existing retrieved `event_id`/`mem_id` handles inside Core's snapshot; Core verifies ID existence & `basis ∈ {observed, told, inferred, self_caused}` deterministically (`0` fabricated citations).

---

## 5. Memory Activation (ACT-R), Dual-Process Recall & Long-Horizon Storage

1. **Guarded Petrov Hybrid Base-Level Activation (`B`, `R-04`, `A-01`, `A-07`):**
   Replacing Round 1's misattributed single-power approximation (`B ≈ ln(n/(1-d)) - d ln L`, non-monotonic in `d`), Core uses the real **Petrov (2006) hybrid (`k = 1` exact most-recent + power-law tail)**:
   $$B = \ln\!\left( t_1^{-d} + \frac{(n - k)\left(t_n^{1-d} - t_k^{1-d}\right)}{(1 - d)(t_n - t_k)} \right) \quad \text{when } n > k \land t_n > t_k; \qquad B = \ln\!\left(\sum_{j=1}^n t_j^{-d}\right) \text{ otherwise}$$
   with `t_j >= 1/60 h` (1-minute floor), eliminating `0/0` when `t_n = t_k` [Anderson & Schooler 1991; Petrov 2006, `[PR]`; Fisher et al. 2018, `[PR]`]. **Zero-history `n = 0` yields `B_0` -> neutral `familiarity = 0`**, shrinking via pseudo-counts toward body/place priors during cold start (`A-07`).
2. **Total Activation & Fade Threshold (`A-01`, `R-01`):**
   $$A = w_B \cdot B + \text{context\_weight} \cdot \cos(\mathbf{v}_{\text{ctx}}, \mathbf{v}_m) + \text{importance\_weight} \cdot \text{importance}$$
   where explicit switch/weight `w_B` allows Row 10's exact neutral (`w_B = 0` -> similarity-only). Sleep vector fade (`vector_released`) checks **base activation `B < fade_threshold = -3.29`** (Petrov exact value for `n=1, t_1=720 h, d=0.5`), never undefined total `A` (`A-01`).
3. **Dual-Process Automatic Recall (`A-09`, `R-16`):** Separation of familiarity (`A + threshold`) and recollection (`verbatim entity/place/time match`) happens *across* two channels without blocking 30-day familiarity hits behind mandatory LLM verification (`Yonelinas 1994, 2002, [PR]`). Text search executes behind Gateway's `Store` capability port (`R-12`).
4. **Cognitive Map (`R-32`):** Core maintains an edge-weighted co-occurrence + Successor Representation (`SR`, `M = (1 - γ_sr) Σ ...`) transition graph purely over opaque `{actor, place, concept}` IDs inside the 6 books [Behrens et al. 2018, `[PR]`].
5. **Long-Horizon Engine (`R-40`):** Segmented SQLite (`hot.db` WAL + read-only sealed temporal shards `ATTACH`ed cleanly), pre-filtered HNSW via `usearch v2.26.6` (with metadata bitsets synced under `R-33`/`R-34`), `blake3 v1.8.7` snapshots, and `postcard`/`minicbor` binary serialization (avoiding deprecated `bincode` `RUSTSEC-2025-0141`).

---

## 6. Expectations, Promises, Recurrence & Grounding (`R-07`–`R-10`, `R-46`, `R-49`)

1. **Five Expectation Subtypes (`goal`, `wait`, `predict`, `monitor`, `pending_approval`):** Storing intentions in deterministic code guarantees reliable wakeup [PIS `arXiv:2609.01272`, `[PP]`; PM-Bench `arXiv:2607.12385`, `[PP]`]. Records track plural `places` and `people`, `domain?`, optional **`RecurrenceSpec`** (`cron`/`interval`, `missed_policy ∈ {coalesce_one, skip_to_next}`, zero `promise_debt` inflation for silent monitoring polls, `R-46`), and **`PromiseMeta`** (`promisee_ids`, `hard_deadline`, `depends_on?`, conflict detector, `R-49`).
2. **Lifecycle Outcomes & Pre-Utterance `CommitCheck` (`A-01`, `R-49`):** `resolve` supports `{kept, kept_late, renegotiated, released, failed, cancelled, answered, reached}`. Before emitting an outbound utterance containing a promise, Core/Valve validates resource/time conflicts (`CommitCheck`) so conflicting obligations are refused/renegotiated *before* promising (`R-49`).
3. **Grounding & Sense of Agency (`R-09`, `R-17`, `R-36`):** Every outbound act opens a `predict` carrying `(channel, Δ_obs, Δ_pred)` (tolerance scaled by medium & modality); matching outcomes mark `self_caused` rather than external surprise [Frith, Blakemore & Wolpert 2000; Clark & Brennan 1991, `[PR]`].
4. **Preemption / Barge-In Receipts (`R-36`, `R-37` — Owner Option A):** Preserves `ARCHITECTURE.md` L198's exact 5-outcome enum (`{done, refused, held, failed, unknown}`) while adding optional fields `{termination_reason ∈ {normal, canceled_by_core, preempted_external, timed_out}, progress ∈ [0,1], delivered_units}`.

---

## 7. Credence, Bounded Learning & Metacognitive Self-Model (`R-20`, `R-26`, `R-35`, `A-01`)

1. **Beta Reputation (`E = α / (α + β)`):** Initialized from Gateway per-actor source priors (`told` items included! fixing `beliefs/spec.md` L56 vs L86); updated by confirmed/refuted evidence and discounted over domain-specific forgetting factor `λ_domain` (`R-20`, [Jøsang & Ismail 2002, `[PR]`; Sperber et al. 2010, `[PR]`]). **Critical Guard (`R-36`):** `canceled_by_core` and `preempted_external` receipts NEVER increment `β_c`.
2. **Sign-Corrected & IPS-Weighted Delta Rule (`R-19`, `R-26`, `A-01`, `A-13`):**
   - For wake probability `P = P(go)`, target `r` MUST equal **`1`** for `{good_act, missed_silence}` and **`0`** for `{bad_act, answered_silence}` (fixing `core-and-lab` sign inversion where missed silence had `r = 0`), weighted by clipped Inverse Propensity Scoring `w_ips = min(M_ips, 1 / ρ)` on sampled judged silences (`R-19`) and Gateway per-actor `teacher_weight` (`R-26`).
   - All knob updates respect statistical significance (95% CI excluding 0), per-day `<= 5%` range cap, and freeze-alert bounds [Rescorla & Wagner 1972, `[PR]`].
3. **Event-Derived `self_card` (`R-35`):** Continuously updated in Core with 4 read-only pillars injected cleanly into `reason`:
   1. Per-capability empirical `(α_c, β_c)` + latency (`action_feedback` / receipts);
   2. Historical calibration (`[0, 20]` Brier/ECE + `meta-d'` gap);
   3. Operational headroom (token budget, queue pressure, switch count);
   4. Pending/recent claim-contradiction flags against log truth.

---

## 8. Sleep, Episodic Boundaries, Domains, Lessons & Skills (`A-05`, `A-06`, `A-11`, `A-12`, `R-38`)

1. **Snapshot-Isolated Async Sleep (`A-04`, `A-05`):** Triggered when `unconsolidated >= N_min` (with `>=`, not `>`) and global idle `>= T_idle`, reading immutable `t_snapshot` with `Δθ_sleep = 0`. Episodes move across **`{open, provisional, sealed}`**: idle pre-sleep tails (`L >= N_ep_min` or `Δt > τ_stale`) soft-close into `provisional` for next-day recall and transparently reopen (marking prior gist `superseded_by`) if the session resumes seamlessly (`A-05`).
2. **Normalized Event Boundaries (`R-23`, `A-06`):** Combines time surprise, content prediction error, and embedding novelty normalized consistently to `[0, 1]` (`EM-LLM`, [Zacks et al. 2007; Fountas et al. 2025, `[PR]`]) with cold-start shrinkage (`A-07`, `n <= 5` Samuelson bound guard). Episode merge requires high overlap in people, place, and time [Yassa & Stark 2011, `[PR]`].
3. **Hungarian-Aligned Domain Clustering (`A-12`):** Sleep warm-starts spherical k-means from previous centroids and aligns cluster IDs via Hungarian matching (`domains_updated` split/merge Beta fusion events) so domain reliability history never shuffles across restarts.
4. **Contrastive Lessons & Parameterized Procedures (`A-11`, `R-38`):** `consolidate` clusters repeated opaque failure signatures alongside paired successes to distill contrastive `lesson`s, traces cross-place `goal_id`/`caused_by` chains into slot-parameterized `procedure`s tagged with **`capability_schema_hash`** & CUSUM failure drift, and **retires** (never deletes silently) degraded skills (`R-32`, `R-38`; [Voyager `arXiv:2305.16291`, `[PP]`; AWM `arXiv:2409.07429`, `[PP]`]). Verbatim-vs-gist conflict resolution runs in `reason` (`consolidate`), with verbatim logged events beating gists [Brainerd & Reyna 2002, `[PR]`].

---

## 9. Privacy (`R-33`), Hard Forgetting (`R-34`), Security (`R-41`–`R-43`), Identity (`R-48`) & Prompting (`R-11`)

1. **Exact Privacy Non-Interference (`R-33`):** Every candidate memory/event passes Valve/Core **`VisibleScope` projection BEFORE Step-4 recall & Step-5 appraisal** (enforcing `audience(context) ⊆ audience(item)` unless authorized by an explicit logged, scoped `DeclassToken` [Myers & Liskov 1997, `[PR]`; Anderson 1972, `[IND]`]). This achieves $\Delta_{\text{NI}} = 0$ — secrets neither leak into text nor distort wake urgency or activation counts.
2. **Atomic `hard_forget` & `Two-Epoch Replay` (`R-34`):** Removes target events, scrubs SQLite (`PRAGMA secure_delete = ON`, FTS5 `'secure-delete'=1`), unlinks ref-counted `BLAKE3` attachments (`R-37`), deletes/re-synthesizes derived summaries, recomputes `(α, β)` solely from surviving evidence, and writes an epoch-bumping clean snapshot so deterministic replay never resurrects forgotten data (`GDPR` Art. 17 `[IND]`).
3. **Valve Roles (`R-41`) & Anti-Fatigue Approvals (`R-42`):** Gateway compiles ephemeral **`cedar-policy` (`v4.14.1`)** entity memberships per request from logged `actor_facts` & `body_state` (zero Core knowledge of `"admin"` or `"moderator"`), verifiable via SMT (`cedar-policy-symcc`). External actions fall into 4 risk tiers; uncertain/high-risk mutations enter non-retrying **`PendingApproval`** pinned to canonical hash **`H_act`** (anti-TOCTOU / Loopjacking) with hourly prompt fatigue budgets.
4. **Three-Door Defense Against Poisoning & Prompt Injection (`R-43`):** Instead of brittle perplexity or word blocklists (`47%–99%` adaptive bypass), MonoAgens enforces:
   - **Write Door:** Skeptical corroboration (`k_corrob`) and min-trust taint propagation (`min(trust)` on compaction so stranger text never becomes trusted self-knowledge `[CaMeL Debenedetti et al. 2025/2026, [PR]]`);
   - **Context Door:** Immutable `ContextPlan` Spotlighting boundaries (`[SYSTEM]`, `[ANCHOR]`, `[TRUSTED]`, `[UNTRUSTED_QUOTED]` [Hines et al. 2024, `[PR]`]) ordered static-to-volatile (`88%` prompt cache reuse);
   - **Action Door:** Cedar blocks privileged/state-mutating/identity capabilities whenever untrusted taint touched the causal chain unless owner-approved [Wallace et al. 2025 `ICLR`, `[PP]`; OWASP Agentic `2026`, `[IND]`].
5. **Identity Continuity Under Long Horizon (`R-48`):** Soul constitution locks under **`OwnerSigned`** provenance (`ARCHITECTURE.md` Absolute Line 1), carries an immutable `IdentityAnchor` in stable prompt slot `0`, and runs deterministic **`[ANCHOR_CHECK]`** / pre-act **`PrincipleAudit`** so multi-turn sycophancy or register drift cannot hijack the persona even across context compactions (`Cheng et al. 2025 ELEPHANT`; `Guan et al. 2024 Deliberative Alignment`).

---

## 10. Gateway, Identity, Waist-2 Bodies, Routines, Benchmarks & Standards (`R-36`–`R-40`, `R-44`–`R-50`)

1. **Cross-Body Identity (`R-44`):** Each `(body_id, platform_account_id)` hashes deterministically to isolated opaque `per_` IDs; merging identities across bodies requires owner config or a bilateral challenge-response committed to the event log (preventing impersonation escalation).
2. **Attachment Store & Blind Multimodal/Speech Pipeline (`R-37`):** Gateway computes scope-keyed `blake3::keyed_hash` (`v1.8.7`) and embeds text/image/`audio_voice` uniformly into `256d` via **`EmbeddingGemma 2`**; Body runs `Silero VAD` / `WebRTC VAD`, causal `RAPT`/`YIN` `F0`+RMS normalization (avoiding research-only `openSMILE`), `Pyannote`, and `VAP`, sustaining **$< 50\text{ ms}$** TTS barge-in mute mapped to Option A `Receipt`.
3. **Waist-2 Protocol (`monoagens.body.v1`, `R-45`, `R-50`):** Stateless JSON-RPC 2.0 ($\le 12$ methods, OpenRPC `1.4.1` + JSON Schema `2020-12`, explicit `idempotency_key`, 3-class prioritized queue `256/512/256`), leaving all 40 cognitive rules behind Waist 1 while dedicated Gateway bridge plugins wrap **MCP `2026-07-28`**, **A2A `v1.0.1`**, and **ROS 2**.
4. **Human-Like Honest Delivery (`R-47`):** Gateway emits honest in-flight lifecycle cues (`1 s` typing pulse, `10 s` progress note), never fake post-completion delays or impersonation (EU AI Act Art. 50 `[IND]`); Chat Body handles platform chunking (`message_ids` -> single Receipt).
5. **Benchmarks, NI Power & TCK Rollout (`A-02`, `A-16`, `R-39`, `R-50`):** Every one of the **22 Brain Map rows** carries explicit ablation metrics in `monoagens-lab` (`A-02`), evaluated via `BaselineAdapter` on Pareto (accuracy vs cost/latency) fronts across 5 domains (`R-39`), powered at **$S \approx 354\text{--}471$** scenarios for paired non-inferiority (`A-16`), and validated externally by a public black-box **`monoagens-conformance` TCK** across interleaved Phases 0–3 (`R-50`).

---

## Key Sources Index (`[PR]` / `[PP]` / `[IND]`)

Full verified citations (with DOIs, arXiv IDs, URLs, crate versions, and license checks through November 2026) are cataloged in:
- [`2026-11-15-run-1.md`](file:///home/muqsith/Projects/Mono/research/findings/2026-11-15-run-1.md) (Round-1 baseline citations + primary-source verification in `R-28`)
- [`2026-11-16-run-2.md`](file:///home/muqsith/Projects/Mono/research/findings/2026-11-16-run-2.md#verified-bibliography-across-all-38-run-2-questions) (Consolidated Run-2 bibliography across `A-01`–`A-16` and `R-29`–`R-50`)
- [`run-2-notes/`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes) (Detailed proofs, benchmark tables, and primary-source audits per topic)

Core foundational anchors referenced above: Anderson & Schooler (1991, `[PR]`); Anderson (2007, `[PR]`); Petrov (2006, *ICCM*, `[PR]`); Fisher, Houpt & Gunzelmann (2018, `[PR]`); Thompson & Spencer (1966, `[PR]`); Staddon & Higa (1996, `[PR]`); Rescorla & Wagner (1972, `[PR]`); Jøsang & Ismail (2002, `[PR]`); Sperber et al. (2010, `[PR]`); Nelson & Narens (1990, `[PR]`); Koriat & Goldsmith (1996, `[PR]`); Yonelinas (1994, 2002, `[PR]`); Brainerd & Reyna (2002, `[PR]`); Yassa & Stark (2011, `[PR]`); Frith, Blakemore & Wolpert (2000, `[PR]`); Zacks et al. (2007, `[PR]`); Fountas et al. (2025, `EM-LLM`, *ICLR*, `[PR]`); Sacks, Schegloff & Jefferson (1974, `[PR]`); Clark & Brennan (1991, `[PR]`); Goffman (1979, `[PR]`); Bohus & Horvitz (2009, `[PR]`); Mori et al. (2026, `arXiv:2607.15648`, `[PP]`); Liu et al. (2025, *CHI*, `[PR]`); Pu et al. (2025, *CHI*, `arXiv:2502.18658`, `[PR]`); Eckhaus et al. (2025, *Findings EMNLP*, `[PR]`); Debenedetti et al. (2025/2026, *CaMeL*, *SaTML 2026*, `[PR]`); Myers & Liskov (1997, *SOSP*, `[PR]`); Hines et al. (2024, *CAMLIS*, `[PR]`); Wallace et al. (2025, *ICLR*, `[PP]`); Kapoor et al. (2025, *TMLR*, `[PR]`); Schechter Vera et al. (2025, `arXiv:2509.20354`, `[PP]`; 2026, `EmbeddingGemma 2`, `arXiv:2610.37364`, `[PP]`); Ekstedt & Skantze (2022, `VAP`, `arXiv:2205.09812`, `[PP]`); Lin et al. (2025/2026, `Full-Duplex-Bench-v2`, `arXiv:2510.07838`, `[PP]`); MCP `2026-07-28`, A2A `v1.0.1`, OpenRPC `1.4.1`, JSON Schema `2020-12`, Cedar `v4.14.1`, SQLite `3.54.1`, `usearch v2.26.6`, `blake3 v1.8.7`, `postcard v1.1.3`, `minicbor v2.3.1` (`[IND]`).
