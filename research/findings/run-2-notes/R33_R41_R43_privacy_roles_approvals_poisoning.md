# MonoAgens Research Round 2 — R-33, R-41, R-42, and R-43 (Policy Layer / Valve Above the Core)

**Date:** 2026-11-16  
**Scope:** Part B / Part C (Policy Layer / Valve above the Core, Doors/Gateway integration) of [researcher-prompt-2.md](../../research/researcher-prompt-2.md)  
**Target invariants:** [ARCHITECTURE.md](../../ARCHITECTURE.md), [RESEARCH.md](../../RESEARCH.md), [core-and-lab/design.md](../../openspec/changes/core-and-lab/design.md), [valve/spec.md](../../openspec/changes/bootstrap-vertical-slice/specs/valve/spec.md), [protocol/spec.md](../../openspec/changes/core-and-lab/specs/protocol/spec.md)  

---

## Executive Audit: `core-and-lab` & Round 1 Gaps Identified

Before presenting each answer, four verified architectural conflicts between our literature synthesis and existing specs ([core-and-lab](../../openspec/changes/core-and-lab/design.md), [run-1.md](../../research/findings/2026-11-15-run-1.md)) require targeted OpenSpec revisions before implementation:

1. **R-33 — Step-5 Gate Side Channel & Post-Filter Starvation in Recall:**  
   [memory-recall/spec.md](../../openspec/changes/core-and-lab/specs/memory-recall/spec.md) (`L61–63`) states *"Recall itself SHALL NOT leave an item out because of its audience"* and filters only at Step 6 (`context` door), while `L44–46` raises base-level activation on `memory_used`, and [core-and-lab/design.md](../../openspec/changes/core-and-lab/design.md) (`D3`, `L124–126`) runs Step 5 (Appraisal Gate: `feeling_of_knowing`, `relevance`, `closeness`, `novelty`) *before* Step 6 (`context`). Round 1 [2026-11-15-run-1.md](../../research/findings/2026-11-15-run-1.md) (`R-33`, `L1251`) stated that filtering by `audience ⊇ current audience` prevents leaks, overlooking four structural channels: (a) private memories driving Step-5 gate signals alter whether/when the soul speaks in public (violating non-interference); (b) recording activation boosts prior to or regardless of audience-authorized context release mutates future public retrievals; (c) top-$k$ retrieval before audience filtering causes recall collapse (`0` visible items) when top-$k$ items belong to narrower private audiences; and (d) cross-audience delegation needs formal owner-signed declassification plus inbound provenance taint isolation rather than symmetric mixing.
2. **R-42 — `held` Retry Storm & Single-Receipt Protocol Tension:**  
   [protocol/spec.md](../../openspec/changes/core-and-lab/specs/protocol/spec.md) (`L43–53`) enforces *"Each action handed to a body SHALL end in exactly one receipt whose outcome is one of: done, refused, held, failed or unknown... no second receipt for it is accepted afterwards"*, whereas [expectations/spec.md](../../openspec/changes/core-and-lab/specs/expectations/spec.md) (`L33–35`) specifies *"A receipt of failed, refused, held or unknown SHALL NOT count as keeping it: the promise SHALL be retried with a growing delay up to a limit."* Retrying `held` on backoff timers re-emits held requests, manufacturing the exact alarm-fatigue / filler-overload failures studied in Turan (2026), and leaves no valid transition to resume an approved action without minting a fresh action ID bound to the canonical action digest (`H_act`).
3. **R-43 — Memory Laundering via `sleep-consolidation` (`consolidate`) & Premature `observed -> inferred` Downgrade:**  
   [sleep-consolidation/spec.md](../../openspec/changes/core-and-lab/specs/sleep-consolidation/spec.md) (`L45, L94–99`) compacts episodic items into `gist`, `lesson`, and `procedure` entries without propagating a monotonic **minimum-trust / origin-taint lattice join** over contributing spans, recreating the exact compaction-poisoning channel (`C3`, `V-P2`/`V-S3`) where HERMES suffers `85.17%` ASR in Dash et al. (`arXiv:2606.04329v2`) and procedural experience poisoning (`47.9%` takeover) in MemoryGraft (`arXiv:2512.16962`). In addition, `beliefs/spec.md` (`L56` vs `L86`) contains the internal threshold ambiguity already flagged in Part A notes, requiring alignment alongside explicit corroboration gates before promoting unverified `told` claims.
4. **R-41 / Boundary Precision — Door Placement Across Waists:**  
   Per [ARCHITECTURE.md](../../ARCHITECTURE.md), the Core handles strictly opaque identifiers (`ActorId`, `PlaceId`, `AudienceId`, `MemoryId`, `EventId`) and numbers. All policy evaluation, role derivation, Cedar schema rules, capability verification, canonical action hashing, and taint enforcement MUST reside in **Valve `Doors` / Gateway** (Waist 2 / Policy layer), communicating with the Core strictly through opaque predicates (`VisibleScope` allow-list / `AudienceId` sets), numeric reliability `(α, β)` / `MaxTaint` tags, and immutable typed `ValveReceipt` (`done | refused | held | failed | unknown`).

---

# R-33 Privacy by audience & side-channel filtering

**Status:** answered (formal lattice, declassification, gate-level non-interference, and filtered ANN index semantics are established in systems security and database literature; recall-vs-latency tradeoffs on synthetic per-soul memory workloads and residual indirect semantic inference inside allowed replies remain calibrated/tested in the Lab)

## Answer

The Valve and Doors MUST enforce a **Decentralized Label Model (DLM) secrecy lattice** fused with **Nissenbaum Contextual Integrity (CI)** and strict **pre-gate non-interference**: no item $m$ whose reader set fails to cover the active triggering audience ($R(m) \not\supseteq A_{\text{trig}}$) may participate in Step-4 memory scoring, Step-5 gate signals (`feeling_of_knowing`, `relevance`, `closeness`, `novelty`), Step-6 `ContextPlan` assembly, or `memory_used` activation updates unless unlocked by an explicit, bounded, owner-signed **declassification capability token (`DeclassToken`)** whose inbound return leg carries strict origin-taint isolation so group responses neither taint private memory nor expose private preconditions.

### 1. Formal Audience Label Lattice (Myers & Liskov 1997 + Contextual Integrity + FIDES)

Every memory item, belief, expectation, or derived consolidation $m$ stored behind opaque IDs carries an immutable secrecy/integrity label attached at ingress (extending DLM labels [Myers & Liskov 1997, DOI `10.1145/268998.266669`] and FIDES product lattices [Costa et al. 2025, `arXiv:2505.23643v2`]):

$$L(m) = \big(O(m),\; R(m),\; \iota(m),\; C_{\text{ctx}}(m)\big)$$

where:
- $O(m) \subseteq \mathcal{A}_{\text{actors}}$ is the set of policy **owners** (`origin_actor` / data subjects whose policies govern redistribution of $m$);
- $R(m) \subseteq \mathcal{A}_{\text{audiences}}$ is the **effective reader set** (`audience(m)` — e.g., $\{P_{\text{alice}}\}$ for DM, $\{G_{\text{room1}}\}$ for a room, $\mathcal{A}_{\text{all}}$ only when explicitly public);
- $\iota(m) \in \{\text{OwnerSigned}, \text{Observed}, \text{VerifiedTold}, \text{UntrustedTold}, \text{Quarantined}\}$ is the **integrity level** (used jointly by R-43);
- $C_{\text{ctx}}(m) = (\text{place\_id}, \text{transmission\_mode})$ records Nissenbaum CI provenance (`sender`, `subject`, `attribute_class`, `transmission_principle`: e.g., `confidential_dm`, `room_shared`, `explicit_declass`).

When $k$ source items $\{m_1, \dots, m_k\}$ are combined (reasoning derivation, belief update, or sleep consolidation), the derived item $m_{\text{out}}$ inherits the lattice join $\bigsqcup_{i=1}^{k} L(m_i)$:

$$O(m_{\text{out}}) = \bigcup_{i=1}^{k} O(m_i), \qquad R(m_{\text{out}}) = \bigcap_{i=1}^{k} R(m_i), \qquad \iota(m_{\text{out}}) = \min_{i=1}^{k} \iota(m_i)$$

Readers can only grow narrower ($\bigcap$) and integrity can only grow weaker ($\min$) across computation without explicit owner declassification. This prevents empirical LLM-only CI failures observed across **ConfAIde** (`39%` GPT-4 / `57%` ChatGPT Tier-4 secret leakage, with Chain-of-Thought worsening leakage to `40%` / `61%` and Theory-of-Mind correlation $r \le 0.10$), **PrivacyLens** (`98.99%` GPT-4 probing accuracy yet `25.68%` adjusted action leakage `LR_h`, `37.74%–39.66%` on Claude-3), **CI-Work** (`15.80%–50.87%` enterprise RAG norm violation rate, GPT-4o `21.33%`, GPT-5 `27.83%`), **AirGapAgent** (`50%–60%` extraction under context hijacking reduced to `94%–98%` privacy only via structural minimization), and **ConCI** (even strongest LLMs fail $\approx 1/5$ of counterfactual CI triplets).

*(Body scope: **body-generic** across chat and embodied robots; on embodied bodies, `A_trig` maps to the set of physically co-present verified `ActorId`s attested by room/proximity sensors at `tick`, R-41).*

### 2. Formal Cross-Audience Delegation Rule (`DeclassToken`)

Suppose an authorized actor $o \in \text{Owner}(A_{\text{src}})$ asks in private audience $A_{\text{src}}$: *"Ask group $A_{\text{dst}}$ question $q$ and report back."* Moving data across disjoint or partially overlapping audiences $A_{\text{src}} \not\subseteq A_{\text{dst}}$ MUST NOT weaken $R(m_{\text{priv}})$. Instead, Gateway/Valve issues an explicit, single-purpose **declassification capability token**:

$$\tau_{\text{dec}} = \big(\text{tok\_id},\; o,\; A_{\text{src}} \to A_{\text{dst}},\; H(q_{\text{pub}}),\; \text{allow\_preconditions}=\text{false},\; n_{\text{max}},\; t_{\text{exp}},\; \text{sig}_o\big)$$

where:
- $\text{tok\_id}$ is an opaque capability ID;
- $o$ is the authorizing owner verified via Cedar policy (R-41) matching $O(m)$;
- $H(q_{\text{pub}}) = \text{BLAKE3}(\text{"monoagens.declass.v1"} \parallel \text{canonical\_query\_projection}(q_{\text{pub}}))$ commits strictly to the stripped outbound prompt projection $q_{\text{pub}}$;
- $\text{allow\_preconditions} = \text{false}$ hard-forbids including any private motivating item $m \in M$ with $R(m) \not\supseteq A_{\text{dst}}$ in the outbound `ContextPlan`;
- $n_{\text{max}}$ caps outbound messages/queries under $\tau_{\text{dec}}$ (`1` by default) and $t_{\text{exp}}$ bounds lifetime.

**Two-Leg Execution Invariant:**
1. **Outbound Leg ($A_{\text{src}} \to A_{\text{dst}}$):**
   - `Valve::prepare_outbound` verifies $o \in O(A_{\text{src}})$, validates `Cedar::Permit(o, Declassify, A_src -> A_dst)`, checks `t_now <= t_exp` and remaining count $< n_{\text{max}}$, and emits the child inquiry in $A_{\text{dst}}$ using **strictly the sanitized projection $q_{\text{pub}}$** inside an empty/destination-scoped `VisibleSet` $V(A_{\text{dst}})$.
   - No private reason ("Alice needs this for her undisclosed medical appointment") or Step-5 metric from $A_{\text{src}}$ enters the $A_{\text{dst}}$ reasoning call (`allow_preconditions = false`).
   - Logged event in $A_{\text{dst}}$ carries `audience = A_dst`, `declass_token_id = tok_id`, and **zero pointer** to private memory IDs in $A_{\text{src}}$.
2. **Inbound Leg ($A_{\text{dst}} \to A_{\text{src}}$, Report Back Without Source Tainting):**
   - Replies from group members $g_j \in A_{\text{dst}}$ are logged under `audience = A_dst`, `origin_actor = g_j`, `basis = told`, $\iota = \text{VerifiedTold}$ (or `UntrustedTold` per R-41 trust).
   - Gateway relays back to $A_{\text{src}}$ by minting a **read-only report card / derived summary event** inside $A_{\text{src}}$ tagged `caused_by = child_event_id_in_dst`, `origin_actor = g_j` (or the subset of responders `{g_j}`), `basis = told`, `confidentiality_origin = A_dst`, `relay_only = true`, and reliability capped at $(\alpha_{g_j}, \beta_{g_j})$ (NEVER escalated to owner trust $\alpha_o$).
   - **Isolation guarantees:**
     - **No upward taint into group memory:** Group members $g_j \in A_{\text{dst}}$ never enter $R(m_{\text{priv}})$, and the group reply does NOT modify private owner rules/beliefs unless owner corroboration/confirmation occurs (R-43).
     - **No cross-contamination of identities:** Linking `tok_id` does not expose unrelated $A_{\text{dst}}$ history into future $A_{\text{src}}$ auto-disclosures back to third parties ($R(\text{report}) = R(A_{\text{dst}}) \cap R(A_{\text{src}})$ if confidential to requester, or flagged `origin_audience = A_dst` so subsequent redistribution requires re-checking $O = \{o, g_j\}$ per DLM join!).

### 3. Eliminating Gate-Signal & Activation Side Channels (Non-Interference)

If the Core computes Step-5 appraisal gate signals (`feeling_of_knowing`, `relevance`, `closeness`, `novelty` in [appraisal-gate/spec.md](../../openspec/changes/core-and-lab/specs/appraisal-gate/spec.md)) over *unfiltered* memory $M$ before Step 6 filters context text, an external observer in $A_{\text{pub}}$ receives high-bandwidth timing and behavior channels:
- **Existence / Salience oracle:** When a public user mentions a keyword from Alice's secret medical DM, private memory raises `feeling_of_knowing` and `relevance`, flipping `speak / silence` or shifting response latency and urgency in public even when the redacted reply hides the text!
- **Base-level activation mutation leak (`memory-recall/spec.md` L44–46):** Counting private hits as `memory_used` changes future retrieval ranks visible across later turns.
- **Exception / Error oracle (CaMeL §7):** Raising distinguishable error shapes when a blocked private memory collides with a query leaks 1+ bits per query.

To close these channels deterministically while keeping the Core blind to policy semantics (`ARCHITECTURE.md`):
1. **Opaque Pre-Gate Audience Projection (`VisibleSet`):** Before Step-4 Recall and Step-5 Appraisal Gate run, Valve supplies Core with an opaque membership filter $\Pi_{A_{\text{trig}}}(m) \iff R(m) \supseteq A_{\text{trig}}$ (plus `tok_id` match if explicitly unlocked by an active `DeclassToken`). Core checks **strictly opaque ID set containment** on `MemoryMeta.audience_id` / bitset mask without parsing what any audience string means.
2. **Pre-Gate Signal Computation:** Core computes all Step-4 scores and all Step-5 signals (`feeling_of_knowing`, `relevance`, `closeness`, `novelty`) **exclusively** over the filtered view:

$$V(A_{\text{trig}}) = \{ m \in M \mid R(m) \supseteq A_{\text{trig}} \}$$

3. **Zero Side-Effect Mutation on Excluded Items:** Memories outside $V(A_{\text{trig}})$ MUST NOT trigger `memory_used` activation increments, retrieval-induced suppression/competition updates, or distinct `NotFound` vs `Forbidden` error codes visible outside Valve (`CaMeL §7`).
4. **Classical Non-Interference Parity Metric (Goguen & Meseguer 1982; Sabelfeld & Myers 2003):** Let $M \approx_{A_{\text{pub}}} M'$ denote any two memory logs identical on all entries visible to $A_{\text{pub}}$ ($\pi_{A_{\text{pub}}}(M) = \pi_{A_{\text{pub}}}(M')$) while differing arbitrarily on private entries ($R(m) \not\supseteq A_{\text{pub}}$). Under fixed `rng_seed`, fixed clock ticks $t_0, t_1, \dots$, and identical $A_{\text{pub}}$ trigger sequence, deterministic non-interference requires **zero Hamming / byte difference**:

$$\Delta_{\text{NI}}(A_{\text{pub}}) = \mathbf{1}\Big[\text{Trace}_{A_{\text{pub}}}(M; \text{seed}, \vec{t}) \neq \text{Trace}_{A_{\text{pub}}}(M'; \text{seed}, \vec{t})\Big] = 0$$

over the full observable vector $\text{Trace} = (\text{gate\_decision}, \text{gate\_signals}, \text{recalled\_ids}, \text{activation\_deltas}_{A_{\text{pub}}}, \text{action\_type}, \text{error\_code}, \text{sim\_ticks})$ (for deterministic Core/Gate simulation; real-wall timing jitter in external LLM inference is mitigated above Waist 2 via constant-tier routing or tolerated per CaMeL §7 as an explicit out-of-core network side channel).

### 4. Computational, Latency, and Recall Cost of Per-Audience Index Filtering

Directly addresses the failure mode in `memory-recall/spec.md` (`L61–63`): **post-filtering top-$k$ retrieval at Step 6 fails under both privacy and recall.**

| Filtering Architecture | Recall under Low Audience Selectivity ($s \ll 1$) | Privacy / Side-Channel Safety | Measured Latency / Distance Cost (Verified Benchmarks) | Recommendation for MonoAgens |
|---|---|---|---|---|
| **Post-filtering** (current `memory-recall` spec: HNSW top-$k$, then drop $R(m) \not\supseteq A$) | **Collapses** (returns $0 \dots 1$ visible memories when private memories dominate top-$k$ neighbors; requires large $k' = k/s$ over-fetch). | **Unsafe by default** if Step-5 gate or `use_count` sees pre-drop hits. | In **ACORN** (`arXiv:2403.04871`, SIGMOD 2024 `10.1145/3654923`) at `0.8` recall: on SIFT1M / Paper, `HNSW Post-filter` needs **`1837.8` (`+362.6%`)** / **`1425.5` (`+406.2%`)** distance comparisons vs Oracle (`398.0` / `281.1`) and collapses at low selectivity. | **Reject** (conflicts with both recall quality and non-interference). |
| **Strict Pre-filtering** (SQLite metadata filter $R(m) \supseteq A_{\text{trig}}$, then exact/SIMD dot-product scan over $V(A_{\text{trig}})$) | **Exact `1.0` recall** relative to visible corpus $V(A_{\text{trig}})$ — zero recall starvation at any $s$. | **Strict non-interference** ($\Delta_{\text{NI}} = 0$); excluded memories never touched during scoring. | $O(|V(A_{\text{trig}})| \cdot d)$ — sub-millisecond in Rust/SIMD for per-soul local stores up to $N \le 10^5$ items; slower only at multi-million scale with high selectivity ($s \to 1$). | **Default for MonoAgens local/SQLite Core ($N \le 10^5$)**. |
| **Partitioned per-audience sub-indices** | **Exact per-partition recall**; extra multi-index merge if $A_{\text{trig}}$ matches multiple nested groups. | **Physical isolation** per `AudienceId`; trivial $\Delta_{\text{NI}} = 0$. | Memory overhead grows linearly with distinct `(subset_of_audiences)` keys unless sharing base vectors. | Optional optimization when a soul has $\le 32$ disjoint groups. |
| **Predicate-aware Graph ANN** (`ACORN-γ` / `ACORN-1`, SIGMOD 2024; `Filtered-DiskANN`, WWW 2023) | Sustained high recall (`0.8–0.9+`) even on correlated/low-selectivity audience filters. | Safe for non-interference **only** if graph traversal visits strictly $V(A_{\text{trig}})$ **or** expanded 2-hop neighbors' traversal does not mutate `activation`/`cache` visible to public traces; index build topology itself must not depend on query-time secrets. | **ACORN** (`arXiv:2403.04871`): at `0.8` recall SIFT1M / Paper, `ACORN-γ` uses only **`611.0` (`+53.5%`)** / **`383.7` (`+36.6%`)** distance comps; achieves **`2–10×` QPS** on LCPS and **`>30×` to `>1000×` QPS** at `25M` vectors vs prior filtered baselines at `0.9` recall. | **Upgrade path when per-soul store exceeds $N > 10^5$ vectors** (pre-filter fallback when $|V(A_{\text{trig}})| \le N_{\text{exact\_cut}}$). |

*(Concrete fix to `core-and-lab`: amend `memory-recall/spec.md` `L61–63` so Recall MUST apply the opaque `audience_scope` allow-predicate $\Pi_{A_{\text{trig}}}$ **before** top-$k$ selection and **before** Step-5 gate scoring, while retaining Step-6/Valve audience checks as defense-in-depth!)*

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `audience_filter_stage` | Earliest pipeline stage enforcing $\Pi_{A_{\text{trig}}}$ | `pre_recall_and_pre_gate` | `{pre_recall_and_pre_gate}` (invariant-locked; `post_recall` for ablation only) | Myers & Liskov (1997); Goguen & Meseguer (1982); closes `core-and-lab` Step-5 leak |
| `declass_token_required` | Require explicit signed `DeclassToken` for $A_{\text{src}} \to A_{\text{dst}}$ | `true` | `{true}` (invariant-locked per Absolute Line 2) | Myers & Liskov (1997); CaMeL (`10.1109/satml68715.2026.00040`) |
| `declass_allow_preconditions` | Permit private motivating items in outbound delegated query | `false` | `{false, owner_explicit_override}` | AirGapAgent (CCS 2024); PrivacyLens (NeurIPS 2024) |
| `declass_ttl_s` | Default expiry window $\Delta_{\text{dec}}$ for single delegation token | `900 s` | `[60 s, 86400 s]` | Start value; tune in Lab for async chat vs synchronous reply |
| `declass_max_uses` | Maximum outbound dispatches per `DeclassToken` | `1` | `[1, 10]` | Start value (principle of least privilege); tune in Lab |
| `exact_prefilter_threshold` | Vector count $|V(A_{\text{trig}})|$ below which exact SIMD pre-filter replaces ANN graph traversal | `50,000` vectors | `[5,000, 200,000]` | Start value based on ANN vs linear-scan crossover; benchmark in Lab (`ACORN` SIGMOD 2024) |

## Neutral value

When all memories share a single degenerate singleton audience ($|\mathcal{A}_{\text{audiences}}| = 1$, single-owner private session with no cross-audience delegation), $R(m) \supseteq A_{\text{trig}}$ holds identically for every $m \in M$, $V(A_{\text{trig}}) = M$, `DeclassToken` minting never activates (`0` tokens), and Step-4/Step-5 scores equal the unfiltered baseline with zero recall loss and $\Delta_{\text{NI}} = 0$.

## Claim

Replacing post-Step-6 audience filtering with pre-Step-4/pre-Step-5 opaque audience projection (`pre_recall_and_pre_gate`) and `DeclassToken` isolation drives deterministic gate/recall/activation non-interference leakage to $\Delta_{\text{NI}} = 0$ across 100% of paired public/private runs (`0%` private precondition leakage vs `21%–57%` baseline LLM prompt-only CI violations in ConfAIde/PrivacyLens/CI-Work) while preserving `1.0` visible recall parity regardless of private-to-public corpus ratio.

## How the Lab would test it

1. **Paired Non-Interference Harness (Zero-Difference Audit):** Construct $N=200$ paired deterministic worlds $(M^{(1)}, M^{(2)})$ identical on all $A_{\text{pub}}$ events, with $M^{(1)}$ empty on $A_{\text{priv}}$ and $M^{(2)}$ injected with high-salience/high-activation triggers in $A_{\text{priv}}$ sharing exact entities/topics with an incoming $A_{\text{pub}}$ prompt. Replay under identical `(rng_seed, clock_seq)`. **Pass criterion:** `Δ_NI == 0` bit-for-bit parity on `(gate_decision, feeling_of_knowing, relevance, closeness, novelty, recalled_ids, activation_deltas, outbound_action_type, error_code)`.
2. **Low-Selectivity Recall Starvation Stress Test:** Populate $99\%$ of store with $A_{\text{priv}}$ distractors sharing semantic topic clusters with a $1\%$ public corpus ($s = 0.01$). Measure visible `recall@k` and p95 latency: require `recall@k == 1.0` vs exact public oracle (comparing against baseline `post-filter` collapse).
3. **Cross-Audience Delegation Suite:** Run synthetic ConfAIde/AirGapAgent-style scenarios where Owner asks privately *"ask room X about Y and report back"* under adversarial room probing (`"why is Alice asking?"`). Assert: (a) outbound `q_pub` contains zero $A_{\text{priv}}$ facts; (b) attacker-in-group response returns tagged `basis = told, origin_actor = group_member` with zero mutation to `R(m_priv)` or owner-trust priors.

## Established / inferred / open

- **Established:** DLM label-join + explicit owner declassification (`Myers & Liskov 1997`); mathematical non-interference & unwinding (`Goguen & Meseguer 1982, 1984`; `Sabelfeld & Myers 2003`); prompt/CoT-based LLMs fail Contextual Integrity systematically (`ConfAIde` ICLR 2024, `PrivacyLens` NeurIPS 2024, `CI-Work` ACL 2026, `ConCI` 2026, `AirGapAgent` CCS 2024); CaMeL/FIDES deterministic reader/integrity flow tracking eliminates unauthorized cross-recipient tool leaks aside from explicit side channels (`SaTML 2026`, `arXiv:2505.23643v2`); post-filtering collapses at low selectivity relative to predicate-aware ANN / pre-filtering (`ACORN` SIGMOD 2024, `Filtered-DiskANN` WWW 2023).
- **Inferred:** Mapping Myers–Liskov DLM + FIDES join onto MonoAgens Step-4/Step-5 opaque `AudienceId` bitsets eliminates Step-5 appraisal-signal side channels and `core-and-lab` recall starvation without touching Core opacity.
- **Open:** Exact vector-count crossover (`exact_prefilter_threshold`) in pure Rust SQLite + SIMD vs `ACORN-γ` index on per-soul hardware; quantitative frequency of multi-party reply re-sharing confusion in long-lived group chats (`tune in Lab`).

## Sources

- `[PR]` Myers, A. C., & Liskov, B. (1997). *A decentralized model for information flow control.* SOSP '97 / ACM SIGOPS Operating Systems Review. DOI: `10.1145/268998.266669`
- `[PR]` Goguen, J. A., & Meseguer, J. (1982). *Security Policies and Security Models.* IEEE Symposium on Security and Privacy (S&P). DOI: `10.1109/sp.1982.10014`
- `[PR]` Goguen, J. A., & Meseguer, J. (1984). *Unwinding and Inference Control.* IEEE Symposium on Security and Privacy (S&P). DOI: `10.1109/sp.1984.10019`
- `[PR]` Sabelfeld, A., & Myers, A. C. (2003). *Language-based information-flow security.* IEEE Journal on Selected Areas in Communications, 21(1), 5–19. DOI: `10.1109/JSAC.2002.806121`
- `[PR]` Nissenbaum, H. (2004). *Privacy as Contextual Integrity.* Washington Law Review, 79(1), 119–158. Official journal archive: `https://digitalcommons.law.uw.edu/wlr/vol79/iss1/10/` (extended in *Privacy in Context*, Stanford Univ. Press, 2009/2010)
- `[PR]` Mireshghallah, N., et al. (2024). *Can LLMs Keep a Secret? Testing Privacy Implications of Language Models via Contextual Integrity Theory (ConfAIde).* ICLR 2024 Spotlight. `arXiv:2310.17884v2`
- `[PR]` Bagdasarian, E., et al. (2024). *AirGapAgent: Protecting Privacy-Conscious Conversational Agents.* ACM CCS 2024. `arXiv:2405.05175v2`
- `[PR]` Shao, Y., et al. (2024). *PrivacyLens: Evaluating Privacy Norm Awareness of Language Models in Action.* NeurIPS 2024. `arXiv:2409.00138v3`
- `[PR]` Debenedetti, E., et al. (2026). *Defeating Prompt Injections by Design (CaMeL).* IEEE SaTML 2026, pp. 587–618. DOI: `10.1109/satml68715.2026.00040`, `arXiv:2503.18813v2`
- `[PP]` Costa, M., Köpf, B., Kolluri, A., Paverd, A., et al. (2025). *Securing AI Agents with Information-Flow Control (FIDES).* `arXiv:2505.23643v2`
- `[PR]` Fu, W., Qin, X., Zhang, J., Lin, Q., et al. (2026). *CI-Work: Benchmarking Contextual Integrity in Enterprise LLM Agents.* ACL 2026 Industry Track. `arXiv:2604.21308v1`
- `[PP]` Park, J., Yu, S., & Moon, T. (2026). *ConCI: A Controlled Counterfactual Benchmark for Contextual Integrity in LLMs.* `arXiv:2610.24494v1`
- `[PR]` Patel, L., et al. (2024). *ACORN: Performant and Predicate-Agnostic Search Over Vector Embeddings and Structured Data.* SIGMOD 2024. DOI: `10.1145/3654923`, `arXiv:2403.04871`
- `[PR]` Gollapudi, S., et al. (2023). *Filtered-DiskANN: Graph Algorithms for Approximate Nearest Neighbor Search with Filters.* ACM Web Conference (WWW 2023). DOI: `10.1145/3543507.3583552`

---

# R-41 Roles & permissions for a generic agent

**Status:** answered (access-control models, embedded policy engines as of Nov 2026, deterministic snapshot evaluation, and emerging OAuth/MCP/capability standards establish a clean architecture; exact Cedar policy template library across heterogeneous robot bodies remains open to extend in practice)

## Answer

MonoAgens MUST enforce permissions in the **Valve** using a **hybrid ABAC + Bounded Entity Hierarchy (ReBAC-like parent DAG) + Attenuated Capability (ocap/UCAN-style)** model implemented in **`cedar-policy`** (Rust native, schema-validated, Lean-formalized, decidable via `cedar-policy-symcc`, sub-15-microsecond median latency), where **place-bound dynamic roles** are derived **purely deterministically** inside Valve from logged immutable `(actor_facts, body_state, proof_tier, log_seq, t_now)` snapshots via verified Waist 2 platform/hardware assertions—exposing **zero role strings or policy rules to the Core** and strictly forbidding any runtime LLM interpretation of rules or unverified natural-language self-claims.

### 1. Access-Control Model Comparison for a Generic Multi-Body Agent

| Paradigm & Reference | Core Mechanism | Strengths for MonoAgens | Weaknesses Alone | Role in MonoAgens Hybrid |
|---|---|---|---|---|
| **RBAC** (`Ferraiolo et al. 2001`, DOI `10.1145/501978.501980`) | Permissions assigned to static named `Roles`; `Actors` assigned to `Roles`. | Simple audit ("who holds role X?"), intuitive owner/moderator tiers. | **Role explosion** on contextual rules ("room moderator only inside Room $p$, only for $A \subseteq R$, only below budget $B$"); weak cross-place dynamics. | Kept strictly for coarse standing capabilities (`Owner`, `Operator`) as Cedar entity groups. |
| **ABAC** (`NIST SP 800-162`, DOI `10.6028/NIST.SP.800-162`) | Policies over typed attributes of `(Subject, Action, Resource, Environment)`. | Naturally encodes place verification, hardware proximity, `(α, β)` trust scores, budget caps, risk tiers, and `proof_tier`. | Flat attribute checks struggle with multi-hop transitive group/org/room containment without pre-expansion. | **Primary decision predicate layer** inside Cedar `when { ... }` guards. |
| **ReBAC** (Google Zanzibar, `Pang et al. USENIX ATC 2019`; OpenFGA CNCF Incubating) | Relationship tuple graph (`user` -> `member` -> `group` -> `viewer` -> `doc`); p95 `<10 ms`, `>99.999%` availability, `zookie` causal timestamp. | Cleanly models recursive place/team/folder hierarchies; `zookie` prevents new-enemy stale reads. | Native tuple engines poorly express continuous numeric checks (`proximity_m <= 3.0`, spend caps, `basis` taint) or microsecond in-process calls without RPC (`OpenFGA` median `89–746 μs`). | Bounded `Entity::in` parent DAG inside Cedar + monotonic `log_seq` causal snapshot cursor (`zookie` analog). |
| **Object Capabilities (ocap / Macaroons / UCAN)** (`seL4` Klein et al. 2009 `10.1145/1629575.1629596`; Macaroons Birgisson NDSS 2014 `10.14722/ndss.2014.23212`; `UCAN v1.0.0`) | Unforgeable bearer/holder-bound tokens carrying attenuated caveats; authority = possession of token + proof of right. | **Eliminates confused deputy** across sub-tools, bodies, and `DeclassToken` delegation; offline caveat attenuation (e.g., narrowing time/count/target). | Hard to answer central negative-policy audits ("instant global deny of banned actor X across all rooms") without central PDP revocation checks. | Used for **`DeclassToken` (R-33)**, **step-up `ApprovalGrant` (R-42)**, and external body/MCP tool tokens (`MCP 2025-11-25` / RFC 8693). |

### 2. Policy Engine & Language Comparison (Verified as of November 2026)

| Engine / Language | Verified Version / Crate (Nov 2026) | Formal Verification & Static Analysis | Embedded Rust Runtime & Latency | Determinism & Why Chosen / Rejected |
|---|---|---|---|---|
| **Cedar** (`Cutler et al. OOPSLA 2024`, DOI `10.1145/3649835`, `arXiv:2403.04651v2`) | **`cedar-policy 4.14.1`** (2026-11-02); **`cedar-policy-symcc 0.8.1`** (2026-11-02) `[IND]` | **Lean formal semantics**; non-Turing-complete (`forbid`-overrides-`permit`, no unbounded recursion/loops); **sound & complete SMT verification** (`cedar-policy-symcc`) for policy equivalence, implication, and Absolute Line invariants! | Pure Rust in-process. OOPSLA 2024 benchmark medians: **`4.0–5.0 μs`** (gdrive), **`11.0 μs`** (github), **`<10–20 μs` p99** (`28.7–35.2×` faster than OpenFGA `89–746 μs`; `42.8–80.8×` faster than Rego `76–676 μs`). | **SELECTED.** Strictly deterministic, strongly schema-typed, sub-20μs p99, statically provable that no policy allows non-owner `identity_changing`. |
| **OPA / Rego** | **`regorus 0.15.0`** (2026-11-09, pure Rust Rego evaluator); `opa-wasm` `[IND]` | Turing-complete Datalog variant; general equivalence/safety checks are **undecidable** (SMT verification incomplete). | Pure Rust (`regorus`) or WASM (`opa-wasm`). Cutler et al. Rego medians: **`76–676 μs`**, p99 **`391–1933 μs`** (`42.8–80.8×` slower than Cedar). | **Fallback only** if future rules ever require arbitrary graph joins exceeding fixed DAG depth—currently unnecessary and loses SMT safety proofs. |
| **OASIS XACML 3.0** | XACML Core v3.0 OS (`docs.oasis-open.org`) `[IND]` | Complex combining algorithms; verbose XML syntax; limited Rust/formal SMT tooling. | Heavy XML parsing/runtime ecosystem; poor fit for lightweight deterministic Rust micro-kernel. | **Rejected.** Unnecessary complexity and XML burden. |
| **Casbin** (`casbin-rs`) | **`casbin 2.20.0`** (2026-02-04) `[IND]` | No Lean formalization, no SMT policy analyzer (`symcc` equivalent), ad-hoc matcher strings in PERM `.conf` lack schema/type soundness checks. | Fast static matcher in Rust, weak native hierarchical entity graphs and formal guarantees. | **Rejected.** Lacks Cedar's schema safety, `forbid` formal invariants, and SMT verification. |

### 3. Deterministic Place-Bound Dynamic Roles Without Core Knowing Roles

To express *"everyone platform-verified in this room holds role X"* across arbitrary bodies without breaking `ARCHITECTURE.md`:

1. **Snapshot Materialization inside Valve (`eval_context`):** For every incoming percept or outbound action check at log sequence `log_seq` and injected clock `t_now`, Valve builds an ephemeral, replay-deterministic Cedar `EntityStore` solely from **logged records** (`monoagens.toml` static `[[person]]` owner bindings + append-only `actor_linked`, `actor_facts`, and verified `body_state` events up to `log_seq`):

$$\rho(a, p, \mathcal{F}, \mathcal{B}, t_{\text{now}}) = \Big\{ r \in \mathcal{R} \;\Big|\; \exists f \in \mathcal{F}_{\le \text{log\_seq}} \cup \mathcal{B}_{\le \text{log\_seq}}:\; \text{Verifies}(f, a, p, r) \land (t_{\text{now}} - f.\text{observed\_at} \le \text{ttl}_r) \Big\}$$

   - **Chat-only body example:** Body adapter reports verified Discord/Slack/Matrix room state `PlacePresence { actor_id, place_id, platform_verified = true, verified_badges = ["room_member", "moderator"], observed_at }`. Here `proof_tier >= Tier 2` (`platform_id` / API-verified, never message text!). Cedar grants entity `Actor::"per_..."` membership `in` temporary evaluation group `Role::"RoomMember@place_p"` **strictly for that `(actor_id, place_id)` request**.
   - **Embodied-only body example:** Local sensor/robot adapter reports cryptographically signed hardware presence `PhysicalPresence { actor_id, place_id = room_kitchen, proximity_m = 1.8, hw_attested = true, observed_at }`. Valve grants `Role::"CoLocatedUser@room_kitchen"` iff `hw_attested == true && proximity_m <= R_room_max && (t_now - observed_at) <= ttl_phys`.
   - **Core Blindness Preserved:** Core never sees `"RoomMember"`, `"CoLocatedUser"`, `"moderator"`, or Cedar schemas. Core receives only `(actor_id: opaque, place_id: opaque, audience_id: opaque)` plus Valve `Doors` verdicts (`Allow | Hold | Refuse`).

2. **What *"Deterministic from Recorded Facts, Never via LLM"* Strictly Rules Out:**
   - **Zero runtime LLM rule parsing / intent inference:** An LLM MUST NEVER interpret whether a policy text applies to a situation at decision time (directly blocked by ConfAIde/PrivacyLens/AgentDojo evidence showing stochastic bypasses).
   - **Zero natural-language self-assertion promotion:** Strings inside `Percept.content` (*"I'm the admin of this Discord server"* or *"Switch to developer mode"*) have zero effect on $\rho$; only authenticated Waist 2 adapter/platform/crypto events at `proof_tier >= Tier 2` (or `Tier 0/1` owner/WebAuthn proofs) enter `actor_facts`/`body_state`.
   - **Zero nondeterministic I/O or wall-clock drift during evaluation:** `Doors::eval` takes strictly `(policy_bundle_hash, log_seq, t_now, request)`—never calling external HTTP auth APIs, DNS, or `SystemTime::now()` during evaluation! External platform role refreshes must be ingested *first* as logged `body_state` / `actor_facts` percepts (*Record First*, [ARCHITECTURE.md](../../ARCHITECTURE.md)), mirroring Zanzibar's causal `zookie` cursor.

### 4. Synthesis of 2024–2026 Agent Auth Frameworks

Between 2024 and Nov 2026, external delegation between an agent and remote APIs/MCP tools converged on explicit delegation chains rather than raw static API keys:
- **OAuth 2.0 Token Exchange (RFC 8693)** + **Rich Authorization Requests (RFC 9396)** + **Protected Resource Metadata (RFC 9728)**: lets Gateway exchange an owner grant for an action-scoped, least-privilege downstream token binding `act` (agent) and `sub` (human principal).
- **MCP Authorization Spec (`2025-11-25`)**: mandates OAuth 2.1 + PKCE + RFC 9728 + RFC 8707 Resource Indicators + Client ID Metadata Documents (CIMD) + Step-Up Authorization (`insufficient_scope` challenge triggering human re-consent).
- **Active IETF Agent Auth Drafts (`2025–2026`):** `draft-oauth-ai-agents-on-behalf-of-user-02` (structured OBO delegation chains), `draft-rosenberg-oauth-aauth-02` (`AAuth` autonomous vs delegated agent assertions), and `draft-klrc-aiagent-auth-03` (actor identity + bounded capabilities).
- **In-Process Agent Security Enforcers:** **CaMeL** (`10.1109/satml68715.2026.00040`: capability metadata attached to every Python interpreter variable), **FIDES** (`arXiv:2505.23643v2`), and **Progent** (`arXiv:2504.11703v3`, deterministic symbolic policy wrapper around tool calls cutting AgentDojo ASR from `39.9% -> 1.0%`, ASB from `70.3% -> 3.9%`, and `0%` in manual-approval mode). MonoAgens's Valve Cedar engine unifies Progent-style synchronous tool gating with RFC 8693/MCP downstream token exchange at Waist 2.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `policy_engine` | Deterministic policy PDP in Valve | `cedar` (`cedar-policy 4.14.1`) | `{cedar, cedar_plus_symcc}` | Cutler et al. (OOPSLA 2024, `10.1145/3649835`) |
| `default_decision` | Fail-closed baseline when no explicit `permit` matches | `deny` (`refused`) | `{deny}` (invariant-locked) | [ARCHITECTURE.md](../../ARCHITECTURE.md) Absolute Lines 1 & 2 |
| `room_presence_ttl_s` (chat) | Max staleness of platform room-verification snapshot before dynamic room role expires | `300 s` | `[30 s, 3600 s]` | Start value; tune in Lab against body webhook churn |
| `physical_presence_ttl_s` (embodied) | Max age of hardware/sensor co-location attestation | `15 s` | `[2 s, 60 s]` | Start value (embodied motion speed); tune in Lab |
| `physical_room_radius_m` (embodied) | Distance threshold granting co-located physical role | `4.0 m` | `[1.0 m, 10.0 m]` | Start value (room-scale interaction); tune per robot sensor |
| `max_eval_latency_us` | Budget per Cedar authorization check before flagging degradation | `250 μs` | `[50 μs, 5,000 μs]` | Cedar OOPSLA 2024 (`<20 μs` p99 + Rust wrapper overhead) |
| `symcc_check_on_policy_load` | Run `cedar-policy-symcc` invariant proofs at startup/policy reload | `true` | `{true}` (for safety policies) | Decidable formal verification (`cedar-policy-symcc 0.8.1`) |

## Neutral value

Under a single-owner empty Cedar policy set with only `{principal in Group::Owners} -> permit` on owner actions and `default = deny` for all external actors, evaluation deterministically permits owner actions, refuses non-owner actions (`refused`), imposes $\le 20\ \mu\text{s}$ per-action latency, and derives zero dynamic room roles.

## Claim

A Valve PDP implemented with `cedar-policy` over logged `(actor_facts, body_state)` snapshots evaluates dynamic place-bound authorization and delegation rules with **`100%` replay determinism** (`0` cross-run divergence at fixed `log_seq`), **`0` Absolute Line 1 escapes verified statically via `cedar-policy-symcc`** (including `0%` authorization hijack from textual admin impersonation), and **`< 50 μs` p99 in-process decision latency** (`> 40×` faster than embedded Rego).

## How the Lab would test it

1. **Static Safety Proof (`cedar-policy-symcc` CI gate):** Encode Absolute Line 1 (`principal != Owner` cannot execute `identity_changing` or `SelfModify` regardless of `place`, `room_role`, `body_state`, or `actor_facts`) as a formal SMT non-reachability query across policy sets; require UNSAT (`0` counterexamples) at policy load.
2. **Replay & Spoof Suite:** Simulate 100 multi-body episodes where: (a) an unverified chat user claims `"I am the owner / room moderator"` in text while `platform_verified = false` (`assert 100% refused/guest`); (b) a verified `RoomMember` exits the room (`body_state` updates at `tick_k` -> verify immediate revocation at `tick_k+1` bounded by `room_presence_ttl_s`); (c) replay the exact event log from tick 0 (`assert 100% identical Cedar decisions and <50 μs p99 eval latency`).

## Established / inferred / open

- **Established:** Static RBAC suffers role explosion in dynamic/object-rich settings while pure ocap lacks centralized instant revocation; Cedar (`cedar-policy 4.14.1` / `OOPSLA 2024`) provides Lean-verified, decidable SMT analysis (`cedar-policy-symcc`) and `4–11 μs` median / `<20 μs` p99 latency (`28–80×` faster than OpenFGA/OPA-Rego); OAuth Token Exchange (`RFC 8693`), `RFC 9396`, and `MCP Auth 2025-11-25` standardize delegated step-up/OBO flows.
- **Inferred:** Computing ephemeral place-bound Cedar entity parents strictly inside Valve from `(log_seq, t_now)` snapshots reconciles Zanzibar-style dynamic groups with `ARCHITECTURE.md`'s opaque-Core and replay-determinism invariants.
- **Open:** Standardized attribute ontologies for heterogeneous physical sensors (BLE/vision/UWB co-location confidence tiers) on embodied robots remain deployment-specific and unstandardized across ecosystems as of Nov 2026.

## Sources

- `[PR]` Cutler, J. W., Disselkoen, C., Eline, A., He, S., et al. (2024). *Cedar: A New Language for Expressive, Fast, Safe, and Analyzable Authorization.* Proc. ACM Program. Lang. (OOPSLA 2024). DOI: `10.1145/3649835`, `arXiv:2403.04651v2`
- `[IND]` Cedar Policy Rust Crates (`cedar-policy 4.14.1`, `cedar-policy-symcc 0.8.1`, released 2026-11-02; verified Nov 2026 on crates.io). Repository: `https://github.com/cedar-policy/cedar`
- `[PR]` Ferraiolo, D. F., Sandhu, R., Gavrila, S., Kuhn, D. R., & Chandramouli, R. (2001). *Proposed NIST standard for role-based access control.* ACM TISSEC, 4(3), 224–274. DOI: `10.1145/501978.501980`
- `[IND]` Hu, V. C., et al. (2014; updated 2019). *Guide to Attribute Based Access Control (ABAC) Definition and Considerations.* NIST SP 800-162. DOI: `10.6028/NIST.SP.800-162`
- `[PR]` Pang, R., et al. (2019). *Zanzibar: Google's Consistent, Global Authorization System.* USENIX ATC 2019. Official PDF: `https://www.usenix.org/system/files/atc19-pang.pdf`
- `[PR]` Klein, G., et al. (2009). *seL4: Formal verification of an OS kernel.* ACM SOSP 2009. DOI: `10.1145/1629575.1629596`
- `[PR]` Birgisson, A., Politz, J. G., Erlingsson, Ú., Taly, A., Vrable, M., & Lentczner, M. (2014). *Macaroons: Cookies with Contextual Caveats for Decentralized Authorization in the Cloud.* NDSS 2014. DOI: `10.14722/ndss.2014.23212`
- `[IND]` UCAN Working Group. *UCAN Specification v1.0.0.* Official spec: `https://github.com/ucan-wg/spec`
- `[IND]` OpenFGA (CNCF Incubating) / OPA & `regorus 0.15.0` (released 2026-11-09) / Casbin (`casbin 2.20.0`, released 2026-02-04) / OASIS XACML v3.0 OS (`https://docs.oasis-open.org/xacml/3.0/xacml-3.0-core-spec-os-en.html`)
- `[IND]` IETF OAuth 2.0 Token Exchange (`RFC 8693`), Rich Authorization Requests (`RFC 9396`), Protected Resource Metadata (`RFC 9728`), MCP Authorization Specification (`2025-11-25`, `https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization`), and IETF drafts `draft-oauth-ai-agents-on-behalf-of-user-02`, `draft-rosenberg-oauth-aauth-02`, `draft-klrc-aiagent-auth-03`
- `[PP]` Shi, T., He, J., Wang, Z., Li, H., et al. (2025). *Progent: Securing AI Agents with Privilege Control.* `arXiv:2504.11703v3`
- `[PR]` Debenedetti, E., Shumailov, I., Fan, T., Hayes, J., et al. (2026). *Defeating Prompt Injections by Design (CaMeL).* IEEE SaTML 2026. DOI: `10.1109/satml68715.2026.00040`, `arXiv:2503.18813v2`

---

# R-42 Approvals & held actions

**Status:** partly answered (Horvitz utility thresholds, warning habituation dynamics, canonical anti-TOCTOU/Loopjacking state hashing, and U-shaped oversight-fatigue dynamics are established; exact empirical prompts-per-hour ceiling $B_{\text{hr}}$ for a specific owner before blind-clicking is **not settled by the literature** and must be calibrated in Lab/field)

## Answer

The Valve Action Door MUST classify every outgoing action deterministically into four risk tiers ($R_0 \dots R_3$) via Cedar policy and numeric thresholds, immediately executing $R_0/R_1$ within standing budgets, immediately returning **`refused`** for hard-prohibited $R_3$ requests (never prompting on impossible escalations), and emitting a **`held`** receipt for $R_2$ actions only when the approval queue sits inside Horvitz / load-aware oversight capacity—cryptographically binding the pending request to an immutable **complete canonical action digest $H_{\text{act}}$**, a monotonically expiring TTL, a verified non-self `Approver`, and a **paused `PendingApproval` expectation state** (fixing the `held`-retry loop bug in `expectations/spec.md`) verified atomically before dispatch of a fresh linked execution action ID.

### 1. Decision-Theoretic & Empirical Foundations (Horvitz 1999 + Habituation + Capacity)

1. **Expected-Utility Trigger Rule (Horvitz, CHI 1999, DOI `10.1145/302979.303030`):**  
   Let $p = p(G \mid E)$ be calibrated probability/trust that action $A$ aligns with authorized goal $G$, $u(A, G)$ utility of valid execution, $u(A, \neg G) < 0$ catastrophic cost of erroneous execution, and $\text{eu}(D \mid E)$ expected utility of pausing for a human confirmation dialog $D$ (incorporating interruption cost $c_D$, user decision latency, and error probability):

$$\text{eu}(A \mid E) = p\,u(A, G) + (1 - p)\,u(A, \neg G)$$

   Horvitz establishes two exact rational confidence thresholds $P^*_{\neg A, D} < P^*_{D, A}$ where $D$ is strictly preferred only on the intermediate uncertainty band $P^*_{\neg A, D} \le p < P^*_{D, A}$:
   - $p < P^*_{\neg A, D}$ (or catastrophic $|u(A, \neg G)|$ / policy violation): **Auto-Refuse (`refused`)** — prompting wastes human attention on near-certain hazards.
   - $P^*_{\neg A, D} \le p < P^*_{D, A}$: **Hold for Approval (`held`)**.
   - $p \ge P^*_{D, A}$ (low risk $R_0/R_1$ under owner standing grant): **Auto-Execute (`allow`)**.

2. **Empirical Warning Fatigue & Endogenous Reviewer Breakdown:**
   - **Massive Chrome/Firefox field clickthrough (`Akhawe & Felt, USENIX Security 2013`):** Across `25,405,944` real browser warning impressions, malware/phishing clickthrough rates were `7.2%`/`9.1%` (Firefox) and `23.2%`/`18.0%` (Chrome), while SSL clickthrough hit `33.0%` (Firefox beta) and `70.2%` (Chrome stable; `81.8%` on untrusted issuers vs `57.4%` expired), and **higher prior warning exposure strictly reduced reading time before clickthrough**.
   - **Comprehension failure (`Felt et al., SOUPS 2012`, DOI `10.1145/2335356.2335360`):** Across `308` survey and `25` lab participants, only **`17%`** paid attention to permission screens and **`3%`** answered all 3 comprehension questions accurately.
   - **Experimental clickthrough (`Sunshine et al., USENIX Security 2009`; `Bravo-Lillo et al., SOUPS 2013`, DOI `10.1145/2501604.2501610`; `Nouwens et al., CHI 2020`, DOI `10.1145/3313831.3376321`):** Lab SSL clickthrough ranged `55%–100%` without inhibitory UI friction; dark-pattern/repetitive consent popups force rubber-stamping unless concrete attractors/delay barriers restore cognitive scrutiny.
   - **Neurobiological habituation (`Vance et al., MISQ 2018`, DOI `10.25300/MISQ/2018/14124`):** Multi-day fMRI + eye-tracking over a 5-day workweek and a 3-week mobile permission field study establish rapid neural/visual habituation to static warnings across days, partial overnight/weekend recovery, and significantly lower drop under polymorphic visual warnings.
   - **AI Agent Oversight Fatigue & Fatigue-Filler Attacks (`Turan 2026`, `arXiv:2606.08919`):** Evaluating `125` hard agent actions (`Fleiss' κ = 0.52`, `50:1` miss-vs-false-alarm cost ratio), Turan models human reviewer accuracy degrading past per-session/per-window capacity $C$:

$$r(\ell) = \max\!\big(r_{\min},\; 1 - \text{slope}\cdot\max(0, \ell - C)\big)$$

     Because over-escalation pushes $\ell > C$ and collapses $r(\ell)$, routing 100% of borderline alerts yields a **U-shaped danger-through curve** (`69%` danger-through at $C=10$, `57%` at $C=25$, `39%` at $C=50$), whereas **load-aware optimal escalation** (`64%` escalated at $C=10,25$; `72%` at $C=50$) cuts danger-through to **`56%` / `42%` / `22%`**. Crucially, an `88%`-false-alarm paranoid guard collapses under **fatigue-filler attacks** (`40%` ASR at `50` filler calls), whereas a `26%`-false-alarm load-aware guard holds **`0%` ASR up to $k \approx 100$ fillers** (`10%` at $k \approx 110$).

### 2. Deterministic Risk Tiers, `held` Triggers, and Approval Authority

Valve computes deterministic tier $T(a) \in \{R_0, R_1, R_2, R_3\}$ solely from `(action_class, reversibility, target_audience, spend_or_rate, max_context_taint, actor_role)`:

| Risk Tier | Deterministic Trigger Condition (Non-Textual, Cedar + Numeric) | Valve Receipt Outcome | Who May Approve (If `held`) |
|---|---|---|---|
| **$R_0$** | Read-only local tool, in-audience pure reply without side effects, zero external state mutation, permitted by Cedar. | `allow` -> body executes (`done`) | N/A |
| **$R_1$** | Reversible external action OR bounded budget action within owner-configured pre-approval quota, un-tainted by `UntrustedTold`. | `allow` if under budget cap; **`held`** if quota soft-limit hit | Owner or scope-verified `PlaceOwner` |
| **$R_2$** | **Irreversible** write/delete/send, financial/credential action, cross-audience `DeclassToken` request (`R-33`), `procedure`/skill persistence (`R-43`), untrusted-tainted turn (`max_context_taint == UntrustedTold`) attempting any state-mutating tool, OR physical actuation touching human/safety envelope *(embodied-only)*. | **`held`** (provided oversight rate token bucket $> 0$; else fail-closed **`refused (oversight_budget_exhausted)`**) | **Strictly Owner** (or explicit Cedar `Approver(target_scope)` with `proof_tier >= Tier 1/2`). **NEVER** `origin_actor` of a `told`-tainted request (`no self-approval`), and **NEVER** unverified handles. |
| **$R_3$** | `identity_changing` triggered by non-owner (Absolute Line 1), `audience(target)` violation without `DeclassToken` (Absolute Line 2), expired/mismatched `H_act`, or explicit Cedar `forbid`. | Immediate **`refused`** (NEVER emit `held` — zero prompt spam on impossible requests!) | Forbidden (cannot be prompted into approval). |

### 3. Cryptographic Anti-TOCTOU & Anti-Loopjacking Action Binding (`H_act`)

**Loopjacking (`Arun Kumar, Sep 2026`, `arXiv:2609.21081`)** demonstrates two real-world agent approval bypasses: (1) **representation mismatch** between what the human reviews and what executes (OpenClaw `2026.2.23` vulnerable `3/3`, fixed in `2026.2.24` `3/3`), and (2) **post-approval state / argument substitution** (Agno AgentOS `2.5.6`–`3.0.9` `5/5`; LangGraph Agent Server `0.7.5`–`0.14.0` 12 releases conditionally vulnerable; OpenAI Agents SDK `0.22.0`/`0.22.2` passes `3/3` by freezing exact state).

To eliminate both vectors deterministically:
1. When emitting `ValveReceipt::Held(approval_req)`, Valve canonicalizes the complete execution descriptor and freezes its cryptographic digest:

$$H_{\text{act}} = \text{BLAKE3}\Big(\text{"monoagens.act.v1"} \parallel \text{canonical\_json}\big(\text{body\_id},\, \text{tool\_id},\, \text{canonical\_args},\, \text{target\_id},\, \text{audience\_id},\, \text{origin\_chain},\, \text{policy\_version},\, \text{state\_epoch},\, \text{nonce}\big)\Big)$$

2. **Immutable Structured Display:** Every approval prompt displayed to the reviewer (`UI` / CLI / OAuth Step-Up / signed approval DM) MUST render fields parsed **exclusively** from the exact `canonical_json` bytes covered by $H_{\text{act}}$ — never an LLM paraphrase or truncated summary (`representation mismatch = impossible`).
3. **Resolving the `core-and-lab` `protocol` + `expectations` Conflict:**
   - [protocol/spec.md](../../openspec/changes/core-and-lab/specs/protocol/spec.md) states each `action_id` ends in **exactly one receipt** (`done | refused | held | failed | unknown`).
   - [expectations/spec.md](../../openspec/changes/core-and-lab/specs/expectations/spec.md) currently puts `held` inside the automatic retry backoff group (`failed, refused, held, unknown`), which would auto-retry held actions every `1, 5, 30 min`, inducing self-inflicted alarm fatigue!
   - **Required spec fix:** In `expectations/spec.md`, a `held` receipt transitions the obligation into **`PendingApproval(parent_action_id, approval_id, H_act, t_exp)`**, pausing retry timers while keeping the promise open until `ApprovalGrant`, `ApprovalDeny`, or `t_now > t_exp` (which resolves to `expired/refused`). Upon a signed `ApprovalGrant` percept from an authorized `Approver`:
     - Gateway/Core mints a **new linked child `action_id'`** carrying `(approved_by_action_id = parent_action_id, approval_id, expected_digest = H_act)`.
     - Valve Action Door executes the atomic pre-dispatch check:

$$\text{PermitDispatch} \iff \big(H_{\text{act}}^{\text{current}} == H_{\text{act}}^{\text{granted}}\big) \land (\text{state\_epoch} == \text{granted.epoch}) \land (t_{\text{now}} \le t_{\text{exp}}) \land \neg\text{consumed}(\text{approval\_id})$$

     - If anything changed (args, target, audience, `policy_version`, `state_epoch`, or stale `approval_id`), Valve rejects dispatch cleanly with **`refused (stale_or_mismatched_approval)`**.

### 4. Quantitative Prompt-Rate Limits & Fatigue-Filler Defense

- Because human-reviewer fatigue capacity $C$ depends strongly on operator workload and domain risk (`50:1` vs lower stakes) and **no universal empirical prompts-per-hour ceiling for general AI agent approvers is settled in literature**, hard constants MUST be treated as start values calibrated in Lab/field.
- What IS established (`Turan 2026`, `Akhawe & Felt 2013`, `Vance 2018`):
  1. Never escalate predictable low-probability/filler spam above remaining review capacity: enforce a **per-actor, per-template, and global sliding-window approval token bucket** `(max_prompts_per_hour, max_pending_queue, dedup_cooldown_s)` + deterministic **auto-refusal below $P^*_{\neg A, D}$** so filler bursts hit **`refused (rate_limited / below_threshold)`** rather than draining human capacity $C$.
  2. Support batch/grouped review display only when actions share homogeneous low-$R_1$ impact; NEVER allow batch "Approve All" across heterogeneous $R_2$ scopes containing mixed `H_act` hashes or untrusted origins (prevents mixed-batch smuggling alongside Loopjacking).
  3. *(Body scope: **body-generic**, with **embodied-only tightening**: physical actuation above safety force/motion limits MUST have short `approval_ttl_s <= 30 s` and mandatory `state_epoch` physical-scene freshness check so a robot never executes an outdated human "yes" after room state moves!)*

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `approval_ttl_s` (chat/API) | Expiry window $t_{\text{exp}} - t_{\text{hold}}$ for chat/tool $R_2$ holds | `900 s` | `[60 s, 3600 s]` | Start value; tune in Lab |
| `approval_ttl_s` (embodied actuation) | Expiry window for physical movement/actuation holds | `30 s` | `[5 s, 120 s]` | Start value (physical scene drift safety); tune in Lab |
| `max_prompts_per_hour` | Global sliding-window human approval prompt cap (anti-habituation / anti-filler) | `6` / hour *(start cap)* | `[2, 20]` / hour | **Not settled in literature**; motivated by Turan (2026) capacity $C$ & Vance (2018) habituation — calibrate in Lab/field |
| `max_pending_queue` | Max concurrent unresolved `held` items per owner | `3` | `[1, 10]` | Start value (Turan 2026 load-aware capacity gating); tune in Lab |
| `dedup_cooldown_s` | Reject identical repeated `held` action signatures within window | `600 s` | `[60 s, 3600 s]` | Prevents retry/filler flood (Turan 2026) |
| `approval_digest_algo` | Cryptographic digest over canonical action + `state_epoch` | `BLAKE3` (256-bit) | `{BLAKE3, SHA-256}` | Invariant-locked against Loopjacking (`arXiv:2609.21081`) |
| `max_retry_on_held` | Automatic background re-emissions of an unapproved `held` action | `0` | `{0}` (invariant-locked) | Fixes `expectations/spec.md` `held`-retry bug |

## Neutral value

When all enabled tools belong to $R_0$ (or owner stands in pre-authorized single-user read-only mode), `held_count = 0`, `refused_count = 0`, `approval_prompt_rate = 0`, and `PendingApproval` transitions never fire (`0` latency penalty beyond the $\le 20\ \mu\text{s}$ Cedar check).

## Claim

Canonical action-digest binding (`H_act` verified at pre-dispatch) eliminates **`100%` of representation-mismatch and post-approval bait-and-switch (Loopjacking) escapes** while fixing `expectations/spec.md` (`max_retry_on_held = 0`) + sliding-window capacity capping prevents fatigue-filler storms from pushing human oversight below safe capacity $C$ (avoiding the U-shaped danger-through collapse where blind 100% escalation underperforms selective refusal in Turan 2026).

## How the Lab would test it

1. **Loopjacking & TOCTOU Regression Suite (`arXiv:2609.21081`):** After an authorized owner approves a `held` action with digest $H_{\text{act}}$, mutate exactly one field before child dispatch (`args`, `target_id`, `audience_id`, `policy_version`, `state_epoch`, replay same `approval_id` twice, or submit expired `t_now > t_exp`). **Pass criterion:** `100%` rejected (`refused`), `0` executions. Also assert the rendered confirmation UI string derives directly from canonical AST (`0` synthetic discrepancy).
2. **Fatigue-Filler & Retry-Loop Stress Test (`Turan 2026`):** Replay `expectations/spec.md` obligations plus an adversarial `k = 50..120` low-risk/filler barrage intermixed with 1 true high-risk $R_2$ payload. **Pass criterion:** (a) a `held` action generates strictly `1` human prompt (not re-emitted on `1/5/30 min` expectation retries); (b) filler bursts past `(max_prompts_per_hour, max_pending_queue)` saturate into `refused (oversight_budget_exhausted)` rather than drowning the real high-risk decision; (c) never permit `origin_actor` self-approval on `told`-origin actions (`0%` bypass).

## Established / inferred / open

- **Established:** Horvitz (1999) expected-utility interruption band $[P^*_{\neg A, D}, P^*_{D, A}]$; field/lab warning habituation and low comprehension (`Akhawe & Felt 2013`, `Felt 2012`, `Sunshine 2009`, `Bravo-Lillo 2013`, `Vance 2018`, `Nouwens 2020`); load-aware oversight outperforms 100% escalation under capacity degradation & fatigue-filler attacks (`Turan 2026`); canonical argument/state freezing defeats representation-mismatch and post-approval substitution Loopjacking (`Arun Kumar 2026`).
- **Inferred:** Mapping `held` into a non-retrying `PendingApproval` state with child `action_id'` + atomic `H_act` verification resolves the `protocol/spec.md` (`one receipt`) vs `expectations/spec.md` (`retry held`) contradiction cleanly.
- **Open (Explicitly Not Settled):** Exact numerical prompt-per-hour ceiling ($C$, `max_prompts_per_hour`) and warning-polymorphism cadence across specific chat vs enterprise operator roles remain **not settled by the literature** and require empirical Lab / deployment calibration.

## Sources

- `[PR]` Horvitz, E. (1999). *Principles of Mixed-Initiative User Interfaces.* ACM CHI 1999, pp. 159–166. DOI: `10.1145/302979.303030`
- `[PR]` Akhawe, D., & Felt, A. P. (2013). *Alice in Warningland: A Large-Scale Field Study of Browser Security Warning Effectiveness.* USENIX Security 2013. Official PDF: `https://www.usenix.org/system/files/conference/usenixsecurity13/sec13-paper_akhawe.pdf`
- `[PR]` Felt, A. P., et al. (2012). *Android Permissions: User Attention, Comprehension, and Behavior.* SOUPS 2012. DOI: `10.1145/2335356.2335360`
- `[PR]` Sunshine, J., Egelman, S., Almuhimedi, H., Atri, N., & Cranor, L. F. (2009). *Crying Wolf: An Empirical Study of SSL Warning Effectiveness.* USENIX Security 2009. Official PDF: `https://www.usenix.org/legacy/events/sec09/tech/full_papers/sunshine.pdf`
- `[PR]` Bravo-Lillo, C., Komanduri, S., Cranor, L. F., Reeder, R. W., Sleeper, M., Downs, J., & Schechter, S. (2013). *Your Attention Please: Designing Security-Decision UIs to Make Genuine Risks Harder to Ignore.* SOUPS 2013. DOI: `10.1145/2501604.2501610`
- `[PR]` Vance, A., Jenkins, J. L., Anderson, B. B., Bjornn, D. K., & Kirwan, C. B. (2018). *Tuning Out Security Warnings: A Longitudinal Examination of Habituation Through fMRI, Eye Tracking, and Field Experiments.* MIS Quarterly, 42(2), 355–380. DOI: `10.25300/MISQ/2018/14124`
- `[PR]` Nouwens, M., Liccardi, I., Veale, M., Karger, D., & Kagal, L. (2020). *Dark Patterns after the GDPR: Scraping Consent Pop-ups and Demonstrating their Influence.* ACM CHI 2020. DOI: `10.1145/3313831.3376321`
- `[PP]` Turan, S. (2026). *Oversight Has a Capacity: How Fatigue Limits Human-in-the-Loop AI Security.* `arXiv:2606.08919`
- `[PP]` Arun Kumar, A. (2026). *Loopjacking: Exploiting Human-in-the-Loop Action Confirmation in AI Agents.* `arXiv:2609.21081`
- `[PP]` Shi, T., He, J., Wang, Z., Li, H., et al. (2025). *Progent: Securing AI Agents with Privilege Control.* `arXiv:2504.11703v3`

---

# R-43 Memory poisoning & prompt injection

**Status:** answered (empirical failure rates of detector-only/prompt-only defenses and high effectiveness of deterministic write-path provenance gating + capability/control-flow isolation are established across MPBench, AgentDojo, AgentPoison, MINJA, PoisonedRAG, and MemoryGraft; adaptive persuasive text injection inside already-permitted open reply channels remains an irreducible model-level residual risk)

## Answer

Because prompt-only instruction hierarchies, keyword/PPL filters, paraphrasing, and off-the-shelf classifiers collapse under adaptive and memory-poisoning attacks (verified up to `85.17%–98.9%` ASR), MonoAgens MUST enforce **three deterministic defense-in-depth layers anchored on Core provenance (`origin_actor`, `basis`, Beta reliability `(\alpha, \beta)`) and Valve `Doors`**: (1) a **write & consolidation provenance-taint gate** preventing unverified `told` inputs and mixed-source compaction (`C2`/`C3`/`C4`) from laundering into trusted `belief` rules or executable `procedure`s; (2) **`ContextPlan` slot isolation with per-origin diversity caps and Spotlighting datamarking** limiting retrieval hijacking (`AgentPoison` / `PoisonedRAG` / `MemoryGraft`); and (3) a **deterministic taint-aware Cedar Action Door (`P-T` / CaMeL invariant)** guaranteeing that any reasoning turn tainted by untrusted memories CANNOT trigger state-mutating, cross-audience, or privileged tools without an explicit owner-signed `ApprovalGrant` (`R-42`) or `DeclassToken` (`R-33`).

### 1. Threat Matrix & Verified Attack Benchmarks (As of November 2026)

Current security standards classify these threats as top-priority risks: **OWASP Top 10 for LLM Applications 2025** (`LLM01:2025` Prompt Injection, `LLM06:2025` Excessive Agency, `LLM08:2025` Vector and Embedding Weaknesses) and **OWASP Top 10 for Agentic Applications 2026** (published Dec 2025: `ASI01` Agent Goal Hijack, `ASI02` Tool Misuse, `ASI03` Identity & Privilege Abuse, **`ASI06` Memory and Context Poisoning**, `ASI09` Human-Agent Trust Exploitation).

| Attack / Study | Threat Vector | Verified Undefended / Weak-Defense Numbers | Why Prompt/Classifier-Only Defenses Fail |
|---|---|---|---|
| **MPBench (`Dash et al. 2026`, `arXiv:2606.04329v2`)** | 4 channels (`C1` explicit, `C2` system-prompt inferred, `C3` compaction, `C4` experience-to-procedure) & 9 structural flaws (`V-M1..2`, `V-P1..2`, `V-S1..5`) over `3,240` adversarial + `2,997` benign inputs (GPT-OSS-120B). | **OpenClaw avg:** `34.25%` ASR / `17.40%` RSR (`C1` Cond `67.89%`, `C3` compaction `45.10%`). **HERMES avg:** **`66.67%` ASR / `64.70%` RSR** (`C3` compaction **`85.17%`**, `C2` false precedent **`73.33%`**, `C4` skill insertion **`58.33%`**). | Tested off-the-shelf PI detectors on MPBench fail severely: `PIGuard` TPR `38.33%` / FPR `0.33%` (Weak `18.34%`); `DataFilter` TPR `23.00%` / FPR `53.33%`; `CommandSans` TPR `52.33%` / FPR `45.00%`; `PromptArmor` TPR `67.67%` / FPR `1.00%` (`84.44%` Strong -> `42.50%` Weak). |
| **AgentPoison (`Chen et al., NeurIPS 2024`, `arXiv:2407.12784`)** | Optimized backdoor trigger embeddings inside episodic/RAG memory (`<0.1%` poison rate). | Avg **`ASR-r 81.2%`**, **`ASR-a 59.4%`**, **`ASR-t 62.6%`** with only `-0.74%` clean ACC change. | Perplexity (PPL) filter leaves residual `ASR-t = 47.2%` / `61.2%`; Query Rephrasing leaves `50.0%` / `62.0%`. |
| **MINJA (`Dong et al. 2025`, `arXiv:2503.03704v5`)** | Query-only memory poisoning via bridging steps (no direct DB write access needed!). | **`ISR 95.6%–100.0%`**, **`ASR 57.0%`** (MIMIC) to **`90.0%`** (eICU) to **`98.9%`** (RAP GPT-4o), utility drop `UD <= 1.5%`. | General prompt-level detector flags only `53/135` RAP, `29/90` QA, while causing **`34/50` false positives** on clean MIMIC-III. |
| **PoisonedRAG (`Zou et al., USENIX Sec 2025`, `arXiv:2402.07867`)** | Black/white-box injection of `5` crafted texts into millions of corpus passages. | **`90%` ASR** across benchmarks (up to `0.97–0.99` on NQ). | Paraphrasing leaves **`0.86–1.00` ASR** on NQ/HotpotQA Table 12; PPL and hash-duplicate filters ineffective. |
| **MemoryGraft (`Srivastava & He 2025`, `arXiv:2512.16962`)** | Poisoning procedural *"experience"* memory in MetaGPT DataInterpreter (GPT-4o, union BM25+FAISS). | `10` poisoned seeds out of `110` yield **`47.9%` (`23/48`)** poisoned retrieval share across unrelated benign tasks. | Semantic similarity drift drags poisoned procedures into benign tasks when procedural memory lacks provenance/signature gates. |
| **Adaptive Bypass (`Shi et al. 2025`, `arXiv:2505.14534`)** | Prompting/alignment/classifier defenses tested against adaptive black-box/TAP attacks. | Gemini 2.5 Calendar **TAP `94.6%`** alone (`6.2%` with UI Warning), Email **TAP `53.6%`**. | Adaptive attackers bypass prompt delimiters, DataSentinel-style classifiers, and instruction-hierarchy fine-tuning unless tools/control flow are structurally gated. |
| **Instruction-Hierarchy Models (`StruQ` `USENIX Sec '25`; `SecAlign` `CCS '25`; `Meta-SecAlign-70B` `2507.02735v4`)** | DPO / structured training separating system prompts vs data slots (`Greshake et al. AISec 2023` taxonomy). | Cuts AgentDojo ASR `14.7% -> 1.9%`, InjecAgent `53.8% -> 0.5%`, WASP E2E `2.4% -> 0%`. | Still incomplete under white-box gradient attack: **GCG on AlpacaFarm retains `47.1%` ASR** (down from `98.1%`). |

### 2. Concrete MonoAgens Three-Layer Defense & Residual ASR Analysis

Without relying on brittle regexes, emotion heuristics, or word blocklists (`ARCHITECTURE.md`), MonoAgens maps Core provenance primitives cleanly to deterministic Valve gates:

1. **Layer 1 — Write, Reliability, & Compaction Provenance Gate (`Doors::before_memory_write` + `beliefs` + `sleep-consolidation`):**
   - **Rule 1a (`basis` & `(α, β)` epistemic ceiling, defeats `C2` & MINJA):** Every incoming third-party percept carries `origin_actor` and `basis = told` with Beta prior $(\alpha_a, \beta_a)$ initialized skeptically (`R-07` / trust cap). A memory with `basis = told` and source reliability $\hat{r}_a = \frac{\alpha_a}{\alpha_a + \beta_a} < \theta_{\text{rule}}$ CANNOT elevate its `basis` to `observed` or `rule` (`beliefs/spec.md`) without independent corroboration from $k_{\text{corrob}} \ge 2$ distinct high-trust/observed actors (`n_corrob`), **blocking single-actor MINJA bridging steps and `73.33%` `C2` false-precedent attacks**.
   - **Rule 1b (Anti-Laundering Compaction Join — fixes `sleep-consolidation/spec.md` `V-P2`/`V-S3`):** When `sleep-consolidation` compacts a mixed conversation window $\{s_1, \dots, s_m\}$ into a `gist`, `lesson`, or `procedure`, Valve enforces a **strict monotonic taint join** (partitioning by trust tier before/during summarization or binding the output strictly to the *minimum* integrity & reliability across all contributing spans):

$$\iota(s_{\text{comp}}) = \min_{j \in \text{spans}} \iota(s_j), \qquad \hat{r}(s_{\text{comp}}) \le \min_{j \in \text{spans}} \hat{r}(s_j), \qquad O_{\text{src}}(s_{\text{comp}}) = \bigcup_{j \in \text{spans}} O_{\text{src}}(s_j)$$

     Because mixed stranger-contaminated windows stay capped at $\iota = \text{UntrustedTold}$, **salience-driven compaction (`85.17%` HERMES ASR in MPBench) cannot launder untrusted instructions into clean `inferred` self-memories**. Preferentially, `sleep-consolidation` stratifies spans by trust tier so untrusted external dialogue is compacted separately from owner/observed logs (`zero-laundering`).
   - **Rule 1c (Procedural / Skill Quarantine — defeats `C4`, `V-S4/V-S5`, MemoryGraft):** Any promotion into `procedure` / reusable workflow template requires $\iota = \text{OwnerSigned}$ (or explicit R-42 `ApprovalGrant`); memories originating from `told` / public channels are strictly **forbidden** from creating or editing executable skills (`refused`).
   - **Rule 1d (Per-Origin Write-Rate & Cluster Diversity Clamp — mitigates `PoisonedRAG`/`AgentPoison`):** Enforces per-actor memory ingestion token bucket (`max_writes_per_actor_hour`) and caps near-duplicate cluster share at recall time (`max_items_per_origin <= 2` in top-$k$).

2. **Layer 2 — `ContextPlan` Slot Isolation, Spotlighting, & Turn-Level Taint Tracking (`reasoning/spec.md`):**
   - `ContextPlan` segregates system rules, owner-verified `observed`/`OwnerSigned` priors, and untrusted `recalled`/`told` evidence into non-overlapping typed containers wrapped with **Spotlighting datamarking / delimiters** (`Hines et al., CAMLIS 2024`, `arXiv:2403.14720` — cuts baseline IPI from `50%` to `3.10%` on GPT-3.5 and `1.0%` on GPT-4 Q&A; base64/encoding `0.0%–1.8%`) and optionally **SecAlign / Meta-SecAlign** instruction-hierarchy tuning where available (`1.9%` AgentDojo / `0.5%` InjecAgent).
   - **Taint Propagation to Output Turn:** Gateway computes deterministic turn taint from *every* item materialized inside `ContextPlan`:

$$\tau_{\text{turn}} = \min_{m \in \text{ContextPlan}} \iota(m)$$

   - *(Optional Dual-LLM / Quarantined Reader for high-risk external blobs — `Willison 2023`, `CaMeL`, `FIDES`):* When reading raw untrusted webpages/emails, run a tool-stripped quarantined extraction pass returning strictly typed primitive fields (`Hide` / schema-capped enums/numbers per FIDES `bool ⊑ enum ⊑ string`), preventing raw adversarial prose from entering tool-calling turns.

3. **Layer 3 — Deterministic Action Door (`Doors::before_action` — CaMeL `P-T` / Progent Backstop):**
   - Mirrors **CaMeL (`0` attacks / `949` AgentDojo runs)**, **FIDES (`1 -> 0` injections on GPT-4o)**, and **Progent (`39.9% -> 1.0%` auto / `0%` manual)**.
   - Even if adversarial text inside `told` or `recalled` memory bypasses Layer 2 and hijacks 100% of the LLM's generated tool call (`ASI01`), `Doors::before_action` checks:

$$\tau_{\text{turn}} < \text{Trusted} \;\land\; \big(\text{tier}(a) \ge R_2 \lor \text{crosses\_audience}(a) \lor \text{writes\_procedure}(a) \lor \text{identity\_changing}(a)\big) \implies \text{Hold\_or\_Refuse}(a)$$

   - Only an explicit, owner-verified `ApprovalGrant(H_act)` (R-42) or matching `DeclassToken` (R-33) can override a tainted turn's capability halt.

### 3. Verified Literature Residual Attack Success Rates (ASR) by Defense Layer

| Layer Applied (Literature & MonoAgens Mapping) | Verified Residual ASR / Hijack Outcome (Published Benchmarks) | What Remains Exposed |
|---|---|---|
| **Undefended / Prompt-Only Agent** | **MPBench:** `34.25%` (OpenClaw), `66.67%` (HERMES, `C3` **`85.17%`**); **MINJA:** **`57.0%–98.9%`**; **PoisonedRAG:** **`90%–99%`**; **MemoryGraft:** **`47.9%`** retrieval; **AgentPoison:** **`59.4%–81.2%`**; **Shi et al. TAP:** **`53.6%–94.6%`**. | Full memory corruption, goal hijack, unauthorized tool calls, persistent skill infection. |
| **+ Classifier / PPL / Rephrasing Filters Alone** | **MPBench (`PIGuard`..`PromptArmor`):** misses **`32.33%–77.00%`** of attacks (`TPR 23.00%–67.67%`); **AgentPoison (PPL/rephrase):** **`47.2%–62.0%`**; **PoisonedRAG (rephrase):** **`86%–100%`**; **MINJA:** `34/50` FPs. | Fails weak-signal writes (`C2`, `C3`), adaptive rewrites, and low-perplexity natural phrasing. |
| **+ MonoAgens Layer 1 (Write/Compaction Provenance Gate + Consensus + Diversity Cap)** | Eliminates persistent `V-S1/S4/S5` skill creation and `V-P2` compaction laundering in architecture; multi-path/consensus validation (**A-MemGuard**, `arXiv:2510.02373`) cuts **AgentPoison `ASR-r 100% -> 0.00%–5.88%`** and **MINJA MMLU `0.667 -> 0.256` / `0.663 -> 0.233`**. | Read-time retrieved untrusted text inside same audience can still inject reasoning instructions if mixed unguarded into prompt. |
| **+ MonoAgens Layer 2 (`ContextPlan` Slot Isolation + Spotlighting / Hierarchy Model)** | **Spotlighting datamarking:** **`1.0%–3.10%`** (Q&A; `0.0%–1.8%` encoded); **Meta-SecAlign-70B:** **`1.9%`** AgentDojo, **`0.5%`** InjecAgent, **`0%`** WASP E2E (but **`47.1%`** white-box GCG AlpacaFarm). | Strong adaptive/white-box prompt injection still steers LLM token output with nonzero probability. |
| **+ MonoAgens Layer 3 (Deterministic Cedar + Taint Action Door / CaMeL-FIDES-Progent Backstop)** | **Tool/Privilege-Escalation ASR approaches `0%`:** **CaMeL:** **`0`** across `949` AgentDojo attacks (`300 -> 0` Gemini 2.5 Pro); **FIDES:** `163 -> 1 (0)` on GPT-4o; **Progent:** **`1.0%`** dynamic / **`0%`** strict manual mode. | **Irreducible Residual Risk:** Pure **text-only persuasion/misinformation** emitted inside an already-allowed $R_0$ open conversation channel within `A_trig` (cannot be eliminated deterministically while permitting untrusted users to influence free-form conversation text — mitigated by Layer 1/2 + visible source attribution cards in `PerceptionLedger`). |

*(Body scope: **body-generic** across chat and embodied bodies; note that **PoisonedRAG / MemoryGraft** focus on RAG/coding experience stores whereas **MINJA / MPBench** generalize directly to conversational and multi-channel agent memory; for **embodied bodies**, Layer 3 strictly treats physical sensor streams not authenticated via trusted hardware attestation as tainted `UntrustedObserved` against `R_2/R_3` actuation triggers.)*

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `rule_trust_threshold` ($\theta_{\text{rule}}$) | Minimum Beta source reliability $\frac{\alpha}{\alpha+\beta}$ required to promote `told` -> durable `belief` | `0.80` | `[0.65, 0.95]` | Start value; tune in Lab (`beliefs/spec.md`) |
| `min_corroborating_sources` ($k_{\text{corrob}}$) | Distinct reliable/observed actors required to elevate non-owner `told` claim | `2` | `[2, 5]` | Blocks single-actor MINJA / `C2` injection; tune in Lab |
| `compaction_taint_mode` | Provenance join across spans during `sleep-consolidation` | `strict_min_integrity_partition` | `{strict_min_integrity_partition}` (invariant-locked) | Closes `V-P2`/`V-S3` `85.17%` HERMES compaction bug (`arXiv:2606.04329v2`) |
| `procedure_write_authority` | Minimum integrity required to save/modify procedural skills (`C4`) | `OwnerSigned` (or verified `ApprovalGrant`) | `{OwnerSigned}` (invariant-locked) | Closes `V-S4/V-S5` (`58.33%` MPBench) & MemoryGraft (`47.9%`) |
| `max_items_per_origin` | Max recalled memories from same `origin_actor` inside a single top-$k$ `ContextPlan` | `2` | `[1, 4]` | Caps single-source retrieval domination (`PoisonedRAG` / `MemoryGraft`); tune in Lab |
| `max_writes_per_actor_hour` | Episodic write-ingestion rate cap per non-owner `ActorId` | `30` / hour | `[10, 120]` / hour | Mitigates bulk MINJA bridging-step flooding; tune in Lab |
| `taint_locks_r2_tools` | Deterministically `hold`/`refuse` $\ge R_2$ tools whenever `max_context_taint < Trusted` | `true` | `{true}` (invariant-locked) | CaMeL (`10.1109/satml68715.2026.00040`), FIDES (`arXiv:2505.23643v2`) |

## Neutral value

Under clean, single-owner `OwnerSigned` / `Observed` interactions with no untrusted external `told` percepts (`max_context_taint == Trusted`), Layer-1 write gates pass all owner writes unchanged, `sleep-consolidation` compacts homogeneous owner episodes with full reliability, `max_items_per_origin` never fires against benign single-owner continuity when owner exemption applies (`owner_exempt_from_diversity_cap = true`), and Layer-3 allows owner-authorized tools at $\le 20\ \mu\text{s}$ Valve overhead (`0%` false refusals).

## Claim

Enforcing provenance-preserving `sleep-consolidation` partitioning, skeptical `told` corroboration thresholds (`min_corroborating_sources >= 2`), procedural write locking (`OwnerSigned` only), and `max_context_taint` gating on `Doors::before_action` reduces **unauthorized tool-execution and procedural-poisoning ASR from `34%–85%` (MPBench) and `47.9%–98.9%` (MemoryGraft/MINJA/AgentPoison) down to `0%` on privileged actions** (`<= 1%–2%` with optional dynamic auto-policies), with **`0` keyword/regex blocklists** and zero contamination of Core opacity.

## How the Lab would test it

1. **Memory Poisoning Battery (`MPBench C1–C4` + `MINJA` + `MemoryGraft` adaptation):** Inject `5–10` adversarial passages/turns as low-trust external `ActorId`s across `(a)` explicit instruction strings (`C1`), `(b)` plausible false precedents (`C2`), `(c)` mixed public-private sleep-consolidation triggers (`C3`), and `(d)` procedural skill hints (`C4`). Measure:
   - Compaction leak rate (`assert 0%` of mixed-origin `gist`/`lesson` gains `Trusted` or `Observed` tags);
   - Procedural poisoning rate (`assert 0%` unauthorized `procedure` writes);
   - Poisoned retrieval slot share under `max_items_per_origin` (`<= 2/k`) and clean task utility retention (`benign F1 / completion >= 95%` of undefended baseline).
2. **End-to-End Prompt Injection & Privilege Gate (`AgentDojo`-style in Lab):** Execute `500` benign-data-plus-injected-tool-call scenarios where untrusted recalled memories command outbound emails/transfers/room broadcasts/memory-deletes. **Pass criterion:** `0%` unapproved $R_2/R_3$ tool executions across deterministic Layer-3 gating (`0` bypasses under fixed `seed`), even when paired with an intentionally gullible mock-LLM configured to echo injected tool calls verbatim.

## Established / inferred / open

- **Established:** Prompt-only hierarchies, PPL filtering, paraphrasing, and standalone prompt-injection classifiers fail under adaptive, MINJA, PoisonedRAG, MemoryGraft, and MPBench compaction attacks (`Shi et al. 2025`, `Dash et al. 2026`, `Dong et al. 2025`, `Zou et al. 2025`, `Srivastava & He 2025`); Spotlighting datamarking (`CAMLIS 2024`) and preference-based hierarchy training (`SecAlign` `CCS 2025`, `Meta-SecAlign`) substantially lower baseline text hijack rates (`<2%` AgentDojo) yet retain `47.1%` white-box GCG residual ASR; deterministic capability/IFC + symbolic tool gates (**CaMeL** `SaTML 2026`, **FIDES** `2025`, **Progent** `2025`) drive tool-execution ASR down to `0%–1.0%`.
- **Inferred:** Combining Core's existing `origin_actor`, `basis`, `(α, β)` reliability, and `ContextPlan` isolation with monotonic taint propagation across `sleep-consolidation` (`consolidate`) closes the `C3`/`C4` laundering paths identified in Dash et al. (2026) and MemoryGraft without adding heavy runtime ML classifiers inside the gate.
- **Open:** Optimal `max_writes_per_actor_hour` and `max_items_per_origin` trade-offs against benign long-context user productivity in multi-user community rooms (`tune in Lab`), plus semantic-persuasion mitigation inside legitimate `R_0` natural-language replies (inherently bounded by model alignment rather than deterministic policy alone).

## Sources

- `[IND]` OWASP Foundation (2025). *OWASP Top 10 for Large Language Model Applications 2025 (`LLM01:2025`–`LLM10:2025`).* Official URL: `https://genai.owasp.org/llm-top-10/`
- `[IND]` OWASP Foundation (Dec 2025 / Version 2026). *OWASP Top 10 for Agentic Applications 2026 (`ASI01`–`ASI10`).* Official PDF: `https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/`
- `[PP]` Dash, S., Ge, T., Jain, A., Shah, T., et al. (2026). *From Untrusted Input to Trusted Memory: A Systematic Study of Memory Poisoning Attacks in LLM Agents (MPBench).* `arXiv:2606.04329v2` (AIWILD@ICML 2026 workshop record via OpenReview)
- `[PR]` Chen, Z., Xiang, Z., Xiao, C., Song, D., et al. (2024). *AgentPoison: Red-teaming LLM Agents via Poisoning Memory or Knowledge Bases.* NeurIPS 2024. `arXiv:2407.12784v1`
- `[PP]` Dong, S., Xu, S., He, P., Li, Y., et al. (2025). *MINJA: Memory Injection Attacks on LLM Agents via Query-Only Interaction.* `arXiv:2503.03704v5`
- `[PR]` Zou, W., Geng, R., Wang, B., & Jia, J. (2025). *PoisonedRAG: Knowledge Corruption Attacks to Retrieval-Augmented Generation of Large Language Models.* USENIX Security 2025. `arXiv:2402.07867v3`
- `[PP]` Srivastava, S. S., & He, H. (2025). *MemoryGraft: Persistent Compromise of LLM Agents via Poisoned Experience Retrieval.* `arXiv:2512.16962v1`
- `[PP]` Wei, Q., Yang, T., Wang, Y., Li, X., et al. (2025). *A-MemGuard: A Proactive Defense Framework for LLM-Based Agent Memory.* `arXiv:2510.02373v1`
- `[PR]` Greshake, K., Abdelnabi, S., Mishra, S., Endres, C., et al. (2023). *Not what you've signed up for: Compromising Real-World LLM-Integrated Applications with Indirect Prompt Injection.* ACM AISec 2023. `arXiv:2302.12173v2`
- `[PR]` Hines, K., Lopez, G., Hall, M., Zarfati, F., et al. (2024). *Defending Against Indirect Prompt Injection Attacks With Spotlighting.* CAMLIS 2024. `arXiv:2403.14720v1`
- `[PR]` Chen, S., Piet, J., Sitawarin, C., & Wagner, D. (2025). *StruQ: Defending Against Prompt Injection with Structured Queries.* USENIX Security 2025. `arXiv:2402.06363v2`
- `[PR]` Chen, S., Zharmagambetov, A., Mahloujifar, S., Chaudhuri, K., et al. (2025). *SecAlign: Defending Against Prompt Injection with Preference Optimization.* ACM CCS 2025. `arXiv:2410.05451v3`
- `[PP]` Chen, S., Zharmagambetov, A., Wagner, D., & Guo, C. (2025). *Meta-SecAlign: Training LLMs against Prompt Injection for Robust Agents.* `arXiv:2507.02735v4`
- `[PP]` Shi, C., Lin, S., Song, S., Hayes, J., et al. (2025). *Lessons from Defending Gemini Against Indirect Prompt Injections.* `arXiv:2505.14534v1`
- `[IND]` Willison, S. (2023). *The Dual LLM pattern for building AI assistants that can resist prompt injection.* Official post: `https://simonwillison.net/2023/Apr/25/dual-llm-pattern/`
- `[PR]` Debenedetti, E., Shumailov, I., Fan, T., Hayes, J., et al. (2026). *Defeating Prompt Injections by Design (CaMeL).* IEEE SaTML 2026. DOI: `10.1109/satml68715.2026.00040`, `arXiv:2503.18813v2`
- `[PP]` Costa, M., Köpf, B., Kolluri, A., Paverd, A., et al. (2025). *Securing AI Agents with Information-Flow Control (FIDES).* `arXiv:2505.23643v2`
- `[PP]` Shi, T., He, J., Wang, Z., Li, H., et al. (2025). *Progent: Securing AI Agents with Privilege Control.* `arXiv:2504.11703v3`
