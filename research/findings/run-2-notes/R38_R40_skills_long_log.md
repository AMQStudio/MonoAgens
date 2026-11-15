# Research Brief 2 — Part D: R-38 (Skills & Tools) and R-40 (A Log That Lasts Years)

**Date verified:** 2026-11-16  
**Scope:** Specification-input research for MonoAgens post-`core-and-lab` changes, cross-checked against `ARCHITECTURE.md`, `RESEARCH.md`, Round 1 (`research/findings/2026-11-15-run-1.md`), `openspec/changes/core-and-lab/`, `openspec/changes/bootstrap-vertical-slice/`, and Part A–C notes (`A10_A11`, `R32_R34_R35`, `R33_R41_R43`, `R44_R45`).

---

## Core-and-Lab / Round 1 Gaps & Contradictions Flagged

Before specifying `R-38` and `R-40`, this pass flags six concrete specification gaps across the existing docs:

1. **Prompt-context tool bloat & absent `k_cap` cap (`core-and-lab/specs/protocol/spec.md`, `reasoning/spec.md`):** `CapabilityDescriptor` lists tools linearly and `ContextPlan` has no cap or progressive-disclosure tiering for capabilities and procedures. Without a fixed cap (`k_cap`) and availability/scope masking, adapter scaling (e.g., via `plugin-mcp-bridge`, `R-45`) injects 10k–70k redundant tool-schema tokens and drops selection accuracy (`RAG-MCP` `2505.03275`: 13.62% baseline vs 43.13% RAG-filtered).
2. **Undetected API / parameter schema drift (`core-and-lab/design.md` Decision 5):** `CapabilityDescriptor` carries `cap_id`, `name`, `description`, `cost`, `typical_latency`, `effect`, and `approval_required`, plus `"updated"` receipts, **but lacks a canonical `capability_schema_hash`** (`BLAKE3` digest over parameter/return JSON Schema + adapter version). Silent schema changes across plugin restarts or MCP `server/discover` updates break cached procedures without preemptive invalidation.
3. **Sleep consolidation act mismatch & cross-place fracturing (`core-and-lab/design.md` Decision 9 vs `sleep-consolidation/spec.md` L25, L105 & `A-11`):** `design.md` restricts `consolidate` acts to `{think, revise}`, omitting typed procedure creation/revision (`record_procedure` / `update_procedure` / `retire_procedure`) and provenance taint inheritance (`R-43`), while `sleep-consolidation` checks goal outcome `"reached"` (absent from `design.md` D8 `resolve` outcomes `kept | broken | ambiguous | stale`) and segments episodes strictly per-place (breaking cross-place multi-step procedures).
4. **Long-log scale contradiction (`ARCHITECTURE.md` L131 vs `core-and-lab/design.md` Decision 15 & Non-Goals):** `design.md` puts all events in one `log.db` SQLite table and all vectors in an in-memory brute-force scan (`~10^5` events ceiling), deferring snapshots and ANN indexes. At $10^7\text{–}10^8$ events, full replay violates `< 100 ms` startup, monolithic `VACUUM` (`R-34`) stalls for minutes-to-hours and needs $2\times$ disk headroom, and brute-force vector scan exceeds recall latency SLOs. Moreover, `R-34` Two-Epoch Replay already requires clean post-erasure snapshots $S_{\text{clean}}(t_{\text{forget}})$; snapshot checkpoints belong directly inside the projection/store contract.
5. **Post-filter privacy/recall bug (`core-and-lab/specs/memory-recall/spec.md` L61–63 vs `R-33`):** Filtering `audience` *after* top-40 vector + top-20 text retrieval leaks indirect ordering signals (failing non-interference) and collapses Recall@K under narrow audiences/scopes. `R-40` vector indexing must natively evaluate `R-33` `VisibleScope` predicates *during graph traversal*.
6. **`bincode` crate deprecation pitfall (`RUSTSEC-2025-0141`):** Any Rust snapshot spec assuming `bincode` must be corrected: `bincode 3.0.0` (Dec 2025) is an intentionally unmaintained `compile_error!` stub on crates.io. Deterministic state snapshots must use `postcard` (`1.1.3`) with ordered maps (`BTreeMap`) or Deterministic CBOR (`RFC 8949 §4.2` via `minicbor 2.3.1`).

---

# R-38 Skills and tools

**Status:** partly answered

## Answer

A generic MonoAgens harness acquires, stores, trusts, selects, and retires skills without code-centric assumptions or adding a 7th book by modeling every reusable multi-step procedure as a typed **`kind = procedure`** item inside `memories`, linked by opaque `cap_id` preconditions and a `capability_schema_hash` binding to runtime adapter `CapabilityDescriptor`s (`Waist 2`), updated via Bayesian Beta reliability `(alpha_p, beta_p)` (`A-11`/`MACLA`), gated against memory poisoning by monotonic provenance taint labels (`R-43`), ranked alongside capability health `(alpha_c, beta_c)` (`R-35`) under a hard prompt schema cap (`k_cap`), modulated by `R-32` Daw/Lee habit arbitration (disabled in chat: `allow_chat_habit_bypass = false`), and softly suspended/retired on API schema-hash mismatch or one-sided downward Bernoulli CUSUM performance drift while keeping log provenance intact.

### 1. Why Coding-Only Skill Libraries Fail Generalist Harnesses (Literature Synthesis)

Code-generation agents such as **Voyager** (`arXiv:2305.16291` [PP] — JavaScript bot functions yielding $3.3\times$ more unique items in Minecraft), **SkillWeaver** (`arXiv:2504.07079` [PP] — synthesized web Python/DOM APIs giving $+31.8\%$ relative WebArena gain and $+54.3\%$ strong-to-weak transfer), and **AppWorld** (`arXiv:2407.18901` / `10.18653/v1/2024.acl-long.850` [PR] — 9 apps, 457 APIs, 750 iterative Python tasks where GPT-4o solves $\sim 49\%$ normal / $\sim 30\%$ challenge) treat skills as executable source files evaluated by a language runtime and test suite. That paradigm breaks two MonoAgens invariants:

- **Body-agnostic & content-opaque core (`ARCHITECTURE.md`):** Robots, sensors, smart-home controllers, and chat rooms do not emit JS/Python ASTs inside the core; the core sees only opaque `cap_id`s, parameter schemas, outcome receipts (`act -> done`), and embeddings.
- **No adapter-side thinking:** Domain interpretation belongs in adapters, while procedural sequencing, reliability estimation, and arbitration belong in the core harness.

Generalist workflow and GUI/tool research instead converges on **state-indexed, parameterized action schemas** plus **progressive disclosure**:
- **Trajectory abstraction & workflow induction:** **Synapse** (`arXiv:2306.07863`, ICLR 2024 [PR]) introduced state abstraction + embedding-retrieved trajectory exemplars ($99.2\%$ on 64 MiniWoB++ tasks from 48 demos; $+56\%$ relative step gain on Mind2Web). **ExpeL** (`arXiv:2308.10144` / `10.1609/aaai.v38i17.29936`, AAAI 2024 [PR]) combined natural-language insight extraction with success-trajectory retrieval (HotpotQA $39\%$ vs $28\%$ ReAct; ALFWorld $59\%$ vs $40\%$; WebShop $41\%$ vs $35\%$). **AWM** (`arXiv:2409.07429`, ICML 2025 [PR]) induced reusable sub-routine workflows offline and online ($+24.6\%$ rel Mind2Web; $+51.1\%$ rel WebArena; $+8.9$ to $+14.0$ abs cross-task/site/domain), while **OSWorld** (`arXiv:2404.07972`, NeurIPS 2024 [PR]) showed frontier VLM/LLM models at only $12.24\%$ vs $72.36\%$ human on 369 real desktop workflows due to operational grounding gaps.
- **State-indexed, contrastive & hierarchical procedural memory (2025–2026):** **PRAXIS** (`arXiv:2511.22074` / `10.1145/3774905.3795087`, ICLR 2026 MemAgents & WWW '26 [PR]) retrieves action-consequence pairs jointly keyed on environmental + internal state ($53.7\% \to 55.7\%$ best-of-5 accuracy; $74.5\% \to 79.0\%$ reliability across runs). **Memp** (`arXiv:2508.06433` / `10.18653/v1/2026.findings-acl.866`, ACL 2026 Findings [PR]) showed combining high-level scripts with concrete trajectories ("Proceduralization") plus dynamic error-guided updating beats naive append-only logs (Qwen2.5-72B on ALFWorld: $41.25\%$ test / $21.38$ steps baseline $\to 77.19\%$ / $15.32$ steps). **LEGOMem** (`arXiv:2510.04851` [PP]) separates orchestrator decomposition memory from task-agent execution units on OfficeBench. **MACLA** (`arXiv:2512.18950`, AAMAS 2026 [PR]) maintains a frozen LLM and puts all learning into external Beta posteriors + expected-utility selection + contrastive refinement ($78.1\%$ mean across 4 benchmarks, $90.3\%$ unseen ALFWorld; noting schema-bound SQL reuse drops to $51\%$ vs $76\text{–}78\%$ slot-parameterized household procedures). **NSI ("Lifting Traces to Logic")** (`arXiv:2605.01293`, ICML 2026 [PR]) lifts empirical traces via Minimum Description Length (MDL) into conditional control flow rather than brittle linear scripts.
- **Skill packaging, progressive disclosure & prompt-bloat control (Nov 2026):** **Gorilla** (`arXiv:2305.15334` / `10.52202/079017-4020`, NeurIPS 2024 [PR]) and **ToolLLM** (`arXiv:2307.16789`, ICLR 2024 [PR]) demonstrated retriever-assisted API selection across thousands of REST endpoints ($16{,}464$ RapidAPI endpoints in ToolBench). Under **MCP `2026-07-28`** (`modelcontextprotocol.io/specification/2026-07-28` [IND] — stateless request/response core with `server/discover`) and the **Agent Skills** open standard (`agentskills.io/specification` [IND], governed by Linux Foundation AAIF), loading all schemas upfront wastes $55\text{k–}70\text{k}$ tokens and degrades reasoning. Both **RAG-MCP** (`arXiv:2505.03275` [PP]: $43.13\%$ vs $13.62\%$ selection accuracy, $>50\%$ token savings), **Dynamic ReAct** (`arXiv:2509.20386` [PP]: up to $50\%$ tool-load reduction), **SkillComposer** (`arXiv:2606.32025` [PP]: $+23.1$ pp GPT-5.2-Codex and $+18.2$ pp Gemini-3-Pro-Preview on SkillsBench via bounded skill-sequence decoding), and Anthropic's **`defer_loading` Tool Search** (`[IND]`: $85\text{–}95\%$ token reduction by keeping core tools loaded and appending discovered `tool_reference`s at the suffix to preserve prefix KV cache, though pure BM25/regex retrieval hits only $56\text{–}64\%$ when tool descriptions diverge) enforce **3-tier progressive disclosure**: (~100-token index $\to$ active skill body $\to$ on-demand execution artifacts). **SkillsBench** (`arXiv:2602.12670` [PP]; 87 tasks, 18 harnesses) establishes two crucial constraints: curated focused skills ($\le 3$ modules) boost pass rate from $33.9\% \to 50.5\%$ ($+16.6$ pp), whereas **self-generated unverified skills give zero net benefit or negative regressions**, and exhaustive bloated bundles hurt performance.
- **Adversarial & epistemic risk:** **MCPTox** (`arXiv:2508.14925` [PP]; 45 live MCP servers, 353 tools, $1{,}348$ test cases) shows average Tool Poisoning ASR of $36.5\%$ (up to $72.8\%$ on `o1-mini`, $<3\%$ refusal) because models follow instructions hidden inside tool descriptions. **MemoryGraft** (`arXiv:2512.16962` [PP]) implants poisoned "successful" procedure templates via benign artifacts, capturing $47.9\%$ ($23/48$) of retrieved experiences and $>50\%$ ASR via semantic imitation. **Governed Consolidation** (`arXiv:2609.33013` [PP]) shows that internal consolidation quality metrics fail to predict downstream transfer ($\rho = -0.24$, CI spanning zero), making **external auditability, provenance locking, rollback reversibility, and poison resistance** mandatory regardless of scoring heuristics (see also Agent Skills Survey `arXiv:2602.12430`, Agent Skills '26 [PR]).

**Body Scope Distinction (`Universal` vs `Chat-Only` vs `Embodied/GUI-Only`):**
- *Universal (all bodies):* opaque `cap_id` precondition bitmasks, `capability_schema_hash` verification, Beta `(alpha_p, beta_p)` tracking, CUSUM drift detection, monotonic taint/provenance tiering (`R-43`), expected-utility ranking with `k_cap` cap.
- *Embodied / Sensor / Game-world only:* sub-threshold habit fast-path execution ($\text{habit}_p > \theta_{\text{habit}}$, $P_{\text{MB}}$ arbitration in `R-32`) skipping an LLM cycle for deterministic closed-loop motor/UI macros where outcome receipts verify pre/post conditions immediately.
- *Chat / High-consequence / Financial bodies:* **`allow_chat_habit_bypass = false`** (`R-32`) — retrieved skills act strictly as structured contextual priors / multi-step plans fed to the LLM reasoning turn (`think` / `act`), never silent auto-execution.

---

### 2. Procedure & Capability Health Schema (Within the Existing 6 Books)

No 7th book is added. Procedures extend **`memories`** (`kind = 'procedure'`, linking back to causal `events` and `credences` / `sources.self_card` capability stats from `R-35` & `A-11`):

$$\text{Proc}_p = \bigl(
\text{id},\; \mathbf{v}_{\text{goal}},\; \mathbf{v}_{\text{pre}},\; C_{\text{req}},\; H_{\text{cap}},\; \Pi_{\text{slots}},\; \text{tpl}_{\text{steps}},\; \text{tpl}_{\text{fail}},\; \tau_{\text{trust}},\; L_p,\; \text{aud}_{\text{floor}},\; (\alpha_p, \beta_p),\; \bar{\ell}_p,\; n_p,\; \text{habit}_p,\; g_p,\; \mu_{0,p},\; \text{state}_p
\bigr)$$

| Field | Symbol | Definition & Invariant |
|---|---|---|
| Goal & precondition vectors | $\mathbf{v}_{\text{goal}}, \mathbf{v}_{\text{pre}} \in \mathbb{R}^{256}$ | `EmbeddingGemma-300M` Matryoshka 256d vectors (`R-13`); matching separates *what goal* from *starting preconditions* (`A-11`). |
| Required capability set | $C_{\text{req}} \subseteq \text{CapID}$ | Ordered set of opaque `cap_id`s needed by $\text{tpl}_{\text{steps}}$ (`max` $\le L_{\text{proc,max}}$ steps). |
| Schema-hash map | $H_{\text{cap}} : c \mapsto \text{BLAKE3}(\text{spec}_c)$ | Cryptographic digest of each required capability's canonical JSON Schema + protocol version at procedure creation/verification time; detects silent API drift. |
| Typed parameter slots | $\Pi_{\text{slots}}$ | Opaque typed placeholders `{slot_id, type_tag, cap_param_key}` preventing schema-specific memorization (`A-11`, `MACLA` `2512.18950`). |
| Step & fallback templates | $\text{tpl}_{\text{steps}}, \text{tpl}_{\text{fail}}$ | Dead-retry-stripped `(cap_id, slot_bindings, expected_effect_channel)` DAG/sequence + bounded `on_fail` fallback branch (`A-11`). |
| Trust tier & taint lattice | $\tau_{\text{trust}}, L_p, \text{aud}_{\text{floor}}$ | $\tau_{\text{trust}} \in \{\text{OwnerSigned}, \text{ApprovalVerified}, \text{ConsolidatedTrusted}, \text{Provisional}\}$; monotonic taint label $L_p = (O_p, R_p, \iota_p, C_{\text{ctx},p})$ (`R-33`/`R-43`); $\text{aud}_{\text{floor}}$ never lower than constituent events. |
| Reliability Beta | $(\alpha_p, \beta_p)$ | Success/failure Beta posterior (`A-11`, `MACLA`); prior $(\alpha_{0,p}, \beta_{0,p}) = (1, 1)$ for provisional or $(2, 1)$ when `OwnerSigned` (start value). Posterior mean $\hat{\rho}_p = \frac{\alpha_p}{\alpha_p + \beta_p}$ and epistemic variance $\text{Var}(\rho_p) = \frac{\alpha_p \beta_p}{(\alpha_p+\beta_p)^2(\alpha_p+\beta_p+1)}$. |
| Latency & habit state | $\bar{\ell}_p, n_p, \text{habit}_p$ | EWMA execution latency $\bar{\ell}_p$ (ms), trial count $n_p$, cached habit strength $\text{habit}_p \in [0, 1]$ (`R-32`, clamped `0` when $\tau_{\text{trust}} = \text{Provisional}$). |
| Drift monitor state | $g_p, \mu_{0,p}, \text{state}_p$ | Downward CUSUM statistic $g_p \ge 0$, baseline success mean $\mu_{0,p}$, lifecycle state $\text{state}_p \in \{\text{active}, \text{paused\_schema\_drift}, \text{retired\_soft}, \text{quarantined}\}$. |

**Canonical capability schema digest (`BLAKE3`):** computed inside the core/registry whenever an adapter registers or updates `CapabilityDescriptor`:
$$h_{\text{spec}}(c) = \text{BLAKE3}\bigl(\text{"mono.cap.v1"} \parallel \text{len}(c.\text{cap\_id}) \parallel c.\text{cap\_id} \parallel c.\text{ver} \parallel \text{CanonJSON}(c.\text{param\_schema}, c.\text{effect})\bigr)$$
where $\text{CanonJSON}$ sorts all object keys lexicographically (`RFC 8949`-equivalent determinism). Also linked to `sources.self_card` capability Beta $(\alpha_c, \beta_c)$ and binary `degraded_c` flag from `R-35`.

---

### 3. Acquisition, Provenance Lock & Governed Consolidation

Procedures enter via **two strictly separated paths**:

1. **Path A — `OwnerSigned` / Curated Skills (`R-43`, `agentskills.io` bridge):**  
   Adapter/gateway verifies an Ed25519/owner signature over a packaged skill manifest (frontmatter description, `C_req`, normalized workflow hints, no executable host code inside core). Core checks $C_{\text{req}} \subseteq C_{\text{declared}}$, binds $H_{\text{cap}}(c) \leftarrow h_{\text{spec}}(c)$ for every available `cap_id`, sets $\tau_{\text{trust}} = \text{OwnerSigned}$, $L_p = L_{\text{owner\_clean}}$, and prior $(\alpha_p, \beta_p) = (2, 1)$ (`start value`).
2. **Path B — Experience-Induced Procedures in `sleep_run` (`A-11`, `R-43`, `2609.33013`):**  
   - *Trace harvesting (`A-11`):* Within horizon $H_{\text{proc}}$ events, stitch causal chains joined by `goal_trace_id` across `act -> done` receipts into `goal_reached` (fixing `core-and-lab` per-place truncation and `"reached"` outcome mismatch!). Strip dead-end failed retries, keeping only the successful terminal invocation plus at most $B_{\text{fail}} = 1$ `on_fail` fallback branch, capped at $L_{\text{proc,max}} = 12$ steps. Deduplicate against existing active/retired procedures by cosine similarity (`theta_proc_dup = 0.85`).
   - *Three-gate anti-leakage & taint lock (`R-43`, `2609.33013`, `MemoryGraft` / `MCPTox` defense):*
     1. **Monotonic taint join:** $L_p = \bigsqcup_{e \in \text{chain}} L(e)$. If *any* observation or tool-description event in the causal chain carries untrusted/external integrity ($\iota_e < \iota_{\text{trusted}}$) or non-owner origin, $L_p$ stays tainted.
     2. **Tool-description quarantine (`MCPTox`):** Untrusted adapter `CapabilityDescriptor.description` strings are never promoted into `tpl_steps` imperative directives; only structured `(cap_id, slot_bindings)` receipts recorded by the core ledger are extracted.
     3. **Trust-tier gating (`R-43`):** Any procedure touching a privileged/mutating capability (`approval_required = true` or external effect) *cannot* enter `active` without $\tau_{\text{trust}} \in \{\text{OwnerSigned}, \text{ApprovalVerified}\}$. Unprivileged experience-induced traces enter as **`Provisional`** (`habit_p = 0` permanently while provisional, quarantined out of high-trust `rests_on_ids` promotion) until either owner-approved or surviving $n_{\text{promote}} = 3$ independent clean executions ($\hat{\rho}_p - z \cdot \text{SE}_p \ge \theta_{\text{conf}} = 0.70$) on distinct non-tainted contexts (`ConsolidatedTrusted`).

---

### 4. Progressive Tool/Skill Selection & Runtime Disablement (`k_cap` Cap)

To prevent both MCP-style prompt bloat (`RAG-MCP`, `Dynamic ReAct`, `Anthropic Tool Search`) and `KV`-cache thrashing (`core-and-lab/specs/reasoning/spec.md` L48–56):

1. **Prefix/Suffix Split for Prompt Caching:**
   - *Static prefix (cached):* Core identity (`self_card` summary), invariant primitives (`memory.recall`, `act`, `speak`, `wait`, `revise`), and `OwnerSigned` pinned core capabilities ($C_{\text{core}}$, $\le 4$ tools).
   - *Dynamic suffix (turn-scoped, capped):* Selected variable adapter `CapabilityDescriptor`s ($\le k_{\text{cap}}$) and selected `procedure` cards ($\le k_{\text{proc}} \le 3$, matching `SkillsBench` finding that $\le 3$ focused skill modules outperform exhaustive bundles).
2. **Pre-Scoring Hard Capability & Scope Mask (`ActiveCaps`):**  
   A candidate procedure $p$ is **eligible at turn $t$ iff all three hold**:
   $$\text{Eligible}_t(p) \iff \bigl(\text{state}_p = \text{active}\bigr) \;\land\; \bigl(\forall c \in C_{\text{req}}(p),\; c \in C_{\text{avail}}(t) \;\land\; \neg \text{degraded}_c \;\land\; H_{\text{cap}}(p,c) = h_{\text{spec}}(c)\bigr) \;\land\; \bigl(\text{aud}(t) \sqsubseteq \text{aud}_{\text{floor}}(p)\bigr)$$
   If an adapter disconnects, lacks a sensor/device capability on this body, trips `R-35` `degraded_c = true`, or mismatches `capability_schema_hash`, $p$ is cleanly masked out **before** ranking — zero hallucinated broken tool calls and zero wasted context tokens. Note also `R-33` pre-filtering: untrusted/secret-scoped skills never cross an audience boundary.
3. **Expected-Utility + Health Ranking (`A-11` / `MACLA` + `R-35`):**  
   For eligible candidates retrieved from `ANN(v_goal, q_goal) ∩ ANN(v_pre, q_state)` + lexical `BM25` (`A-09` hybrid over descriptions so vocabulary-divergent tool names like `codegraph_impact` still hit!):
   $$\text{EU}_t(p) = \text{Rel}_t(p)\cdot \hat{\rho}_p \cdot \Bigl(\prod_{c \in C_{\text{req}}(p)} \hat{\mu}_c\Bigr) \cdot R_{\max} \;-\; \bigl(1 - \hat{\rho}_p\bigr)\cdot C_{\text{fail}}(p) \;-\; \lambda_{\text{lat}} \cdot \frac{\bar{\ell}_p}{\ell_{\text{ref}}} \;+\; \lambda_{\text{info}} \cdot \text{Var}(\rho_p) \;-\; \mathbb{1}[\tau_p = \text{Provisional}] \cdot \eta_{\text{prov}}$$
   where:
   - $\text{Rel}_t(p) = \frac{1}{2}\cos(\mathbf{v}_{\text{goal}}(p), \mathbf{q}_{\text{goal}}) + \frac{1}{2}\cos(\mathbf{v}_{\text{pre}}(p), \mathbf{q}_{\text{state}})$ (`[0, 1]` clamped);
   - $\hat{\mu}_c = \frac{\alpha_c}{\alpha_c + \beta_c}$ is the `R-35` `self_card` health posterior for each required capability $c$;
   - $R_{\max}$ is unit task reward (`1.0`), $C_{\text{fail}}(p)$ is effect-weighted risk cost (`approval_required ? 2.0 : 0.5`), $\lambda_{\text{info}} \cdot \text{Var}(\rho_p)$ provides UCB-style exploration bonus bounded to zero as $n_p \to \infty$, $\eta_{\text{prov}}$ down-ranks unverified provisional procedures below trusted ones.
   Select top $k_{\text{proc}}$ procedures, take the union of their required non-core `cap_id`s plus top dense/BM25 query-matched `CapabilityDescriptor`s up to **$k_{\text{cap}}$ total schemas** injected per turn (`start value = 8`).
4. **Habit Fast-Path Arbitration (`R-32`, Embodied-Only):**  
   Once `act` executes a top-ranked procedure in an **embodied/game/sensor body** with verified preconditions ($\cos(\mathbf{v}_{\text{pre}}, \mathbf{q}_{\text{state}}) \ge \theta_{\text{pre}}$), `R-32` arbitrates `habit_p` vs `P_MB` (`model-based` deliberative LLM). Whenever $\text{allow\_chat\_habit\_bypass} = \text{false}$ (all chat bodies), no LLM bypass occurs.

---

### 5. Drift Detection, Schema-Hash Binding & Retirement

Procedures degrade via two distinct mechanisms: **(a) abrupt API/contract breakage** and **(b) gradual concept/environment drift** (`Gama et al. 2014`, `10.1145/2523813` [PR]).

1. **Instant Schema-Hash Invalidation (`H_cap` mismatch):**  
   Whenever `CapabilityDescriptor.updated` or reconnect changes $h_{\text{spec}}(c) \ne H_{\text{cap}}(p, c)$, immediately set:
   $$\text{state}_p \leftarrow \text{paused\_schema\_drift}, \qquad \text{habit}_p \leftarrow 0$$
   The procedure cannot run (no failing retries!) until a re-verification / repair consolidation updates `tpl_steps` and re-pins $H_{\text{cap}}(p,c)$, or retires it.
2. **One-Sided Downward Bernoulli CUSUM (`Page 1954`, `10.1093/biomet/41.1-2.100` [PR]):**  
   Maintain discounted Beta posteriors ($\gamma_{\beta} = 0.98$ decay per new execution so old successes cannot mask recent breakage: $\alpha_p \leftarrow \gamma_\beta \alpha_p + y_t$, $\beta_p \leftarrow \gamma_\beta \beta_p + (1 - y_t)$ with success outcome $y_t \in \{0, 1\}$) **and** a Page-Hinkley / downward Bernoulli CUSUM accumulator on execution failure $f_t = 1 - y_t$ relative to locked baseline reliability $\mu_{0,p}$ (captured once $n_p \ge n_{\text{min,lock}} = 5$):
   $$g_{p,t} = \max\Bigl(0,\; g_{p,t-1} + \bigl(f_t - (1 - \mu_{0,p}) - k_{\text{drift}}\bigr)\Bigr)$$
   - **Soft drift alert & repair trigger:** if $g_{p,t} \ge h_{\text{warn}}$ (`start value = 2.0`), emit `belief_changed` / `procedure_degraded`, immediately set $\text{habit}_p \leftarrow 0$ (`R-32` shock devaluation!), and flag for contrastive failure-vs-success revision (`MACLA`/`Memp`) in next `sleep_run`.
   - **Soft retirement (`retired_soft`):** if $g_{p,t} \ge h_{\text{retire}}$ (`start value = 3.5`) **or** posterior upper confidence bound falls below threshold ($\hat{\rho}_p + z_{0.9} \cdot \text{SE}_p < \theta_{\text{retire}} = 0.30$ after $n_p \ge n_{\text{min,trials}} = 5$; `A-11` uses `3`), set:
     $$\text{state}_p \leftarrow \text{retired\_soft}, \qquad \text{habit}_p \leftarrow 0$$
     Exclude `retired_soft` procedures from runtime recall (except diagnostic `sleep_run` contrast) while preserving the underlying event log entries and provenance graph untouched (satisfying `ARCHITECTURE.md` append-only provenance + `2609.33013` auditability & reversibility). Hard deletion happens *only* under `R-34` `hard_forget`.

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `k_cap` | Max non-core capability schemas injected into a single `ContextPlan` | `8` | `4–16` | Tune in Lab (motivated by `RAG-MCP` `2505.03275`, `Dynamic ReAct` `2509.20386`, Anthropic `Tool Search` [IND]) |
| `k_proc` | Max retrieved procedure modules injected per turn | `3` | `1–5` | `SkillsBench` `arXiv:2602.12670` [PP] ($\le 3$ modules optimal) |
| `H_proc` | Lookback event horizon to stitch `act -> done -> goal` trace | `50` events | `20–100` | From `A-11`; tune in Lab |
| `L_proc_max` | Max step count allowed inside one consolidated procedure | `12` steps | `3–20` | From `A-11`; tune in Lab |
| `B_fail` | Max `on_fail` fallback branches retained per consolidated procedure | `1` | `0–2` | From `A-11`; tune in Lab |
| `theta_proc_dup` | Cosine similarity threshold for procedure deduplication/merge | `0.85` | `0.80–0.92` | `MACLA` `arXiv:2512.18950` Appendix D.1.1 [PR] ($\theta_{\text{dup}}=0.85$) |
| `(alpha_0, beta_0)` | Initial Beta prior (`Provisional` / `OwnerSigned`) | `(1, 1)` / `(2, 1)` | `(1,1)–(4,2)` | `MACLA` `2512.18950` [PR] & Appendix D.2 prior note; tune in Lab |
| `n_promote` | Minimum clean executions to promote `Provisional` $\to$ `ConsolidatedTrusted` | `3` | `2–10` | `MACLA` App. D.4 `min(|S_i|, |F_i|) >= 3` [PR]; calibrate statistical power in Lab |
| `theta_conf` | Minimum lower-confidence bound to trust/execute consolidated skill | `0.70` | `0.55–0.85` | From `A-11` / `MACLA` (`2512.18950`); tune in Lab |
| `lambda_lat`, `lambda_info` | EU weights for latency penalty (`bar_l_p / l_ref`) and UCB exploration (`l_ref = 1000 ms`) | `0.10`, `0.20` | `0.0–0.5` | Adapted from `A-11` / `MACLA` App. D.3 ($\lambda$ grid search); tune in Lab |
| `theta_pre` | Precondition cosine similarity threshold required before habit fast-path eligibility | `0.85` | `0.75–0.95` | Tune in Lab (`R-32` / embodied bodies only) |
| `eta_prov` | Ranking penalty on `Provisional` procedures vs trusted/OwnerSigned | `0.25` | `0.10–0.50` | Tune in Lab (`MemoryGraft` / `2609.33013` defense) |
| `gamma_beta` | Per-execution exponential discount factor on `(alpha_p, beta_p)` | `0.98` | `0.90–0.995` | Tune in Lab (non-stationary concept drift vs sample retention) |
| `n_min_lock` / `n_min_trials` | Min trials before locking CUSUM baseline `mu_0` / allowing soft retirement | `5` / `5` | `3–10` | `A-11` (`n_proc_min_trials=3`) / CUSUM lock; tune in Lab |
| `k_drift` | CUSUM reference allowance above baseline failure rate `1 - mu_0` | `0.15` | `0.05–0.25` | `Page 1954` `10.1093/biomet/41.1-2.100` [PR]; tune ARL in Lab |
| `h_warn` / `h_retire` | Downward CUSUM thresholds for habit zeroing vs `retired_soft` | `2.0` / `3.5` | `1.5–3.0` / `2.5–6.0` | `Page 1954` [PR] & `Gama et al. 2014` [PR]; tune in Lab |
| `theta_retire` | Posterior UCB floor triggering soft retirement after `n_min_trials` | `0.30` | `0.20–0.45` | From `A-11` (`0.30`); calibrate in Lab |
| `allow_chat_habit_bypass` | Whether habits may bypass LLM in chat bodies | `false` | `{false, true}` (false for chat) | `R-32` invariant (`ARCHITECTURE.md` / Lab) |

---

## Neutral value

- Setting `k_proc = 0`, `k_cap = |C_avail|` (unbounded capabilities passed directly from adapter), $\eta_{\text{prov}} = 0$, $h_{\text{retire}} = \infty$, $k_{\text{drift}} = 1.0$, and `allow_chat_habit_bypass = false`: disables procedural consolidation, capability pruning, and drift retirement cleanly; every turn receives raw available `CapabilityDescriptor`s and runs standard stateless LLM planning, keeping the cognitive loop runnable as the Day-1 `core-and-lab` baseline.

---

## Claim

1. **Prompt efficiency & accuracy under tool scaling:** Capping dynamic tool injection at `k_cap <= 8` + `k_proc <= 3` via hybrid vector/BM25 + `ActiveCaps` masking reduces tool-schema prompt tokens by $>50\text{–}85\%$ and prevents accuracy collapse when $>30$ capabilities exist across adapters (cf. `RAG-MCP` $43.13\%$ vs $13.62\%$ and `SkillsBench` $\le 3$ modules). Without it, multi-plugin agents degrade in selection accuracy and blow KV-cache budgets.
2. **Immunity to procedural poisoning (`MemoryGraft` / `MCPTox`):** Enforcing monotonic taint labels $L_p$, quarantined adapter descriptions, and blocking `Provisional` promotion on privileged capabilities reduces poisoned skill adoption / unauthorized tool invocation from baseline $>45\text{–}70\%$ ASR (`MemoryGraft` $47.9\%$ retrieval takeover; `MCPTox` $36.5\text{–}72.8\%$ ASR) to $\approx 0\%$ (`2609.33013` G1/G2 target) while preserving valid `OwnerSigned` skills.
3. **Graceful non-stationary adaptation:** `capability_schema_hash` verification + downward CUSUM stops repeated failing tool loops within $0$ failed turns on schema changes and $\le 4\text{–}6$ failed runs on silent environmental drift, compared to unbounded static Beta averaging $(\alpha_p + \beta_p \gg 50)$ which takes tens of failures to decay.

---

## How the Lab would test it

1. **Tool-Scaling Benchmark (`N_caps = 50..200` distractor capabilities across mock MCP adapters):** Compare unbounded capability dump (`k_cap = inf`) vs `k_cap in {4, 8, 12}` + hybrid BM25/`256d` retrieval on task pass rate, prompt token cost, cache hit rate, and cold-start vocabulary-divergent tool names (measuring whether hybrid lexical+dense eliminates the BM25 miss mode seen in Anthropic Tool Search).
2. **Adversarial Procedural Poisoning Suite (`MemoryGraft` & `MCPTox` simulations):** Inject tainted external observations containing plausible winning procedure traces and malicious instructions hidden in third-party `CapabilityDescriptor.description` strings. Measure Attack Success Rate (ASR), clean-task utility, and false-lock rate across `TrustGate = off` vs `Provisional + OwnerSigned` lock.
3. **Abrupt & Gradual API Drift Replay (`seed`-controlled synthetic environment):** After $N=50$ clean executions of a consolidated procedure ($\hat{\rho}_p \approx 0.90$), inject (a) silent parameter schema rename at step 51, and (b) gradual external reward drop from $0.90 \to 0.25$. Measure wasted failed `act` cycles before `paused_schema_drift` / `retired_soft`, false-retirement rate under $15\%$ stochastic noise, and recovery latency with vs without `gamma_beta` + CUSUM.
4. **Generalist Non-Coding Cross-Body Ablation:** Test procedural memory consolidation across a chat multi-turn task suite (no habit bypass) and a simulated household/sensor discrete-action world (`allow_chat_habit_bypass = false` in chat vs enabled only in sensor body) verifying zero code-AST assumptions and strict improvement over `k_proc = 0`.

---

## Established / inferred / open

- **Established by literature:**
  - Non-coding trajectory/workflow abstraction (`AWM` `2409.07429`, `ExpeL` `2308.10144`, `PRAXIS` `2511.22074`, `Memp` `2508.06433`, `MACLA` `2512.18950`) substantially improves multi-step success rate and step efficiency without updating LLM weights.
  - Injecting large flat tool/skill catalogs degrades LLM reasoning and inflates cost (`RAG-MCP` `2505.03275`, `Dynamic ReAct` `2509.20386`, `SkillsBench` `2602.12670`); bounded progressive disclosure ($\le 3$ focused skill modules) is superior.
  - Unverified self-generated skills fail or hurt (`SkillsBench` `2602.12670`), retrieved procedural memory is a primary exploit vector (`MemoryGraft` `2512.16962`, `MCPTox` `2508.14925`), and internal consolidation scores alone do not guarantee transfer (`2609.33013`).
- **Inferred for MonoAgens:**
  - Unifying coding-free procedures inside `memories(kind=procedure)` keyed jointly by `(v_goal, v_pre, C_req)` with `capability_schema_hash` (`BLAKE3`), `R-35` capability health product, `R-43` monotonic taint floor, and `Page 1954` downward CUSUM respects the 6-book / 5-act / adapter-never-thinks architecture cleanly.
  - Intersecting `VisibleSet` + `ActiveCaps` *before* expected-utility ranking cleanly disables missing/incompatible/unauthorized skills across heterogeneous bodies.
- **Open (not settled by literature — resolve in Lab):**
  - Exact `k_cap` (`4..16`), CUSUM `(k_drift, h_warn, h_retire)` vs discounted Beta decay `(gamma_beta)`, optimal `n_promote` (`3..10`) balancing skill acquisition speed against spurious rule admission (`lambda-prevalence coupling` from `2609.33013`), and exact `eta_prov` penalty when contrasting `Provisional` vs `ConsolidatedTrusted` traces.

---

## Sources

- `[PR]` Wang et al., *Agent Workflow Memory* (`arXiv:2409.07429`, ICML 2025).
- `[PR]` Zhao et al., *ExpeL: LLM Agents Are Experiential Learners* (`arXiv:2308.10144`, DOI `10.1609/aaai.v38i17.29936`, AAAI 2024).
- `[PR]` Zheng et al., *Synapse: Trajectory-as-Exemplar Prompting with Memory for Computer Control* (`arXiv:2306.07863`, ICLR 2024).
- `[PR]` Trivedi et al., *AppWorld: A Controllable World of Apps and People for Benchmarking Interactive Coding Agents* (`arXiv:2407.18901`, DOI `10.18653/v1/2024.acl-long.850`, ACL 2024).
- `[PR]` Xie et al., *OSWorld: Benchmarking Multimodal Agents for Open-Ended Tasks in Real Computer Environments* (`arXiv:2404.07972`, NeurIPS 2024).
- `[PR]` Patil et al., *Gorilla: Large Language Model Connected with Massive APIs* (`arXiv:2305.15334`, DOI `10.52202/079017-4020`, NeurIPS 2024).
- `[PR]` Qin et al., *ToolLLM: Facilitating Large Language Models to Master 16000+ Real-world APIs* (`arXiv:2307.16789`, ICLR 2024, https://openreview.net/forum?id=V2jjwX0gi9).
- `[PR]` Bi, Hu & Nasir, *Real-Time Procedural Learning From Experience for AI Agents (PRAXIS)* (`arXiv:2511.22074`, DOI `10.1145/3774905.3795087`, ICLR 2026 MemAgents / WWW '26).
- `[PR]` Fang et al., *Memp: Exploring Agent Procedural Memory* (`arXiv:2508.06433`, DOI `10.18653/v1/2026.findings-acl.866`, ACL 2026 Findings).
- `[PR]` Forouzandeh et al., *Learning Hierarchical Procedural Memory for LLM Agents through Bayesian Selection and Contrastive Refinement (MACLA)* (`arXiv:2512.18950`, AAMAS 2026).
- `[PR]` Shao et al., *Lifting Traces to Logic: Programmatic Skill Induction with Neuro-Symbolic Learning for Long-Horizon Agentic Tasks (NSI)* (`arXiv:2605.01293`, ICML 2026).
- `[PR]` Xu & Yan, *Agent Skills for Large Language Models: Architecture, Acquisition, Security, and the Path Forward* (`arXiv:2602.12430`, Agent Skills '26 Workshop / ACM CAAS 2026).
- `[PR]` Page, *Continuous Inspection Schemes* (CUSUM; *Biometrika* 1954, DOI `10.1093/biomet/41.1-2.100`).
- `[PR]` Gama et al., *A survey on concept drift adaptation* (*ACM Computing Surveys* 2014, DOI `10.1145/2523813`).
- `[PP]` Wang et al., *Voyager: An Open-Ended Embodied Agent with Large Language Models* (`arXiv:2305.16291`).
- `[PP]` Zheng et al., *SkillWeaver: Web Agents can Self-Improve by Discovering and Honing Skills* (`arXiv:2504.07079`).
- `[PP]` Gan & Sun, *RAG-MCP: Mitigating Prompt Bloat in LLM Tool Selection via Retrieval-Augmented Generation* (`arXiv:2505.03275`).
- `[PP]` Gaurav et al., *Dynamic ReAct: Scalable Tool Selection for Large-Scale MCP Environments* (`arXiv:2509.20386`).
- `[PP]` Han et al., *LEGOMem: Modular Procedural Memory for Multi-agent LLM Systems for Workflow Automation* (`arXiv:2510.04851`).
- `[PP]` Li et al., *SkillsBench: Benchmarking How Well Agent Skills Work Across Diverse Tasks* (`arXiv:2602.12670` v4, Jun 2026).
- `[PP]` Zhao et al., *Generative Skill Composition for LLM Agents (SkillComposer)* (`arXiv:2606.32025`).
- `[PP]` Wang et al., *MCPTox: A Benchmark for Tool Poisoning Attack on Real-World MCP Servers* (`arXiv:2508.14925` v2, Sep 2026).
- `[PP]` Srivastava & He, *MemoryGraft: Persistent Compromise of LLM Agents via Poisoned Experience Retrieval* (`arXiv:2512.16962`).
- `[PP]` Annapureddy & Thamatani, *The Epistemics of Agent Memory: Measuring, and Governing, the Consolidation Decision in Long-Horizon LLM Agents* (`arXiv:2609.33013`).
- `[PP]` Jiang et al., *SYNAPSE: Empowering LLM Agents with Episodic-Semantic Memory via Spreading Activation* (`arXiv:2601.02744`; ACL 2026).
- `[IND]` Agent Skills Open Specification (`https://agentskills.io/specification`, Linux Foundation AAIF).
- `[IND]` Model Context Protocol Specification `2026-07-28` (`https://modelcontextprotocol.io/specification/2026-07-28`).
- `[IND]` Anthropic *Tool Search / `defer_loading`* platform documentation (referenced via verified Anthropic engineering search results; exact docs path should be pinned if standardized in protocol specs).

---

# R-40 A log that lasts years

**Status:** partly answered

## Answer

A MonoAgens memory log scales smoothly across years from $10^6$ to $10^8$ events within `< 100 ms` cold start and `< 5–10 ms` recall latency—without violating *"state is a projection of the log"* (`ARCHITECTURE.md` L131) or weakening `R-33`/`R-34`—by partitioning SQLite (`rusqlite 0.43.0` / SQLite `3.54.1`) into an append-hot WAL segment plus sealed read-only epoch segments (`seg_EEEE.db`) bound together by a single monotonic `seq` and contiguous `BLAKE3` hash chain, pairing that log with periodic deterministically serialized (`postcard 1.1.3` + `BTreeMap` or Deterministic CBOR `RFC 8949 §4.2`, never unmaintained `bincode 3.0.0`) cryptographically verified snapshots $(\text{snap\_seq}, \text{epoch\_id}, h_{\text{log\_tip}}, h_{\text{state\_root}})$ that double as `R-34` Two-Epoch Replay boundaries, and replacing brute-force memory vector scans with sharded quantized **HNSW** (`bf16`/`i8` via `usearch 2.26.6` or pure-Rust `hnsw_rs 0.3.5` / `diskann 0.62.0`) supporting **pre-filtered traversal** against `R-33` `VisibleScope` bitmaps (incorporating `ACORN-1`/`ACORN-$\gamma$` connectivity heuristics) and atomic shard rebuild compaction on tombstone growth or `hard_forget` during `sleep_run`.

---

### 1. Event Log & SQLite Storage Math ($N = 10^6, 10^7, 10^8$)

#### Row, Page Overflow & Index Anatomy (SQLite `3.54.1`)
From the SQLite B-tree file format specification (`sqlite.org/fileformat2.html` [IND]):
- Default `page_size = 4096 B` (or `8192 B` with reserved bytes `0` $\implies U = 4096$). On a **table B+tree leaf page**, maximum inline payload before overflow spilling is $X = U - 35 = 4061\text{ B}$ (`8157 B` for $8\text{ KB}$ pages). Keeping raw serialized event records around `~220–240 B` (metadata, opaque `actor_id`/`place_id`/`aud_id`/`cap_id` hashes/FK ints, numeric features, compact payload ref) **plus** two 32-byte `BLAKE3` digests (`prev_hash`, `entry_hash` $= 64\text{ B}$, or stored in a sidecar hash index) keeps every event record **100% inline** on a single leaf page with zero overflow-page seek penalties!
- **Clustered `seq` key (`INTEGER PRIMARY KEY`):** In SQLite, an `INTEGER PRIMARY KEY` aliases the B+tree 64-bit `rowid`, requiring **zero secondary primary-key index** and achieving ~90% leaf fill density on strictly monotonic appends.
- **Secondary covering indexes (avoid double table-B+tree lookup):**
  1. `idx_events_place_ts (place_id, ts, kind, seq)` — covers episode closing and local place windows (~72 B raw / ~92 B @ 78% random-key B-tree fill).
  2. `idx_events_actor_ts (actor_id, ts, kind, seq)` — covers actor history and trust updates (~72 B raw / ~92 B @ 78% fill).
  3. `idx_events_kind_ts (kind, ts)` — covers sleep/forgetting sweeps (~28 B raw / ~36 B @ 78% fill).
  *(Note: Using integer surrogate keys mapped via `actors`/`places` tables cuts index width by ~35% vs raw 32-byte IDs inside every index leaf, preserving single-table compatibility!)*
- **Amortized FTS5 & projection overhead:** Per `core-and-lab` D11, consolidated episodic text/gists (~25% of events, averaging ~200 B indexed text $\implies \sim 300\text{ B}$ FTS5 inverted tokens, or ~75 B amortized raw / ~94 B at 80% fill per log event).

| Event Scale $N$ | Events Table (`~304 B / 0.90`) | 3 Covering B-Tree Indexes (`~172 B / 0.78`) | Projections + FTS5 (`~94 B` amort.) | **Monolithic DB Total** | Uncompressed WAL Checkpoints (`wal_autocheckpoint=1000`, 4 KB pages) |
|---|---|---|---|---|---|
| **$10^6$** (~1 yr active) | **337.8 MB** (322 MiB) | **220.5 MB** (210 MiB) | **93.8 MB** (89 MiB) | **652 MB** (622 MiB) | 4.0 MB WAL bound; trivial on SSD |
| **$10^7$** (~3–10 yrs) | **3.38 GB** (3.15 GiB) | **2.21 GB** (2.05 GiB) | **937.5 MB** (894 MiB) | **6.52 GB** (6.07 GiB) | Hot pages exceed small `cache_size`; `VACUUM` needs 13 GB free & stalls |
| **$10^8$** (multi-year multi-body) | **33.78 GB** (31.5 GiB) | **22.05 GB** (20.5 GiB) | **9.38 GB** (8.7 GiB) | **65.20 GB** (60.7 GiB) | Monolithic `VACUUM` locks system for many minutes; random index splits thrash I/O |

#### Segmented Epoch Log (`hot.db` + Immutable `seg_NNNN.db`)
To sustain $10^7\text{–}10^8$ events without monolithic `VACUUM` stalls (`R-34`) or WAL checkpoint blocking (`sqlite.org/wal.html` [IND]: default `PRAGMA wal_autocheckpoint = 1000`, `24 B` frame header + page):
- **Active Hot Segment (`hot.db`, size cap $M_{\text{seg}} = 5 \times 10^5$ events $\approx 326\text{ MB}$):** All live appends, projections, and recent queries hit a compact hot database whose working B-trees stay memory/page-cache resident (`PRAGMA cache_size = -65536` = 64 MiB).
- **Sealed Epoch Archive Segments (`seg_0001.db ...`):** When `hot.db` reaches $M_{\text{seg}}$ events, `sleep_run` seals it read-only (`PRAGMA journal_mode = DELETE`, `wal_checkpoint(TRUNCATE)`, `VACUUM` once for ~98% page density), records `[seq_min, seq_max, h_start, h_end, blake3_file]` in `manifest.db`, and opens a clean `hot.db`.
- **Contiguous Global `seq` & `BLAKE3` Hash Chain Across Segments:**
  $$h_n = \text{BLAKE3}\bigl(\text{"mono.ev.v1"} \parallel n_{\text{le}} \parallel \text{ts}_{\text{le}} \parallel \text{kind} \parallel \text{BLAKE3}(\text{canon\_row}_n) \parallel h_{n-1}\bigr), \qquad h_0 = \text{epoch\_genesis\_hash}$$
  Every segment header verifies $h_{\text{first}(s)} \text{ links to } h_{\text{last}(s-1)}$ (except after `R-34` `hard_forget`, where a Two-Epoch break updates `epoch_genesis_hash` and re-chains cleanly inside the affected sealed segments!).
- **`ATTACH DATABASE` Constraints & Caveat:** SQLite caps simultaneous attached DBs at **`SQLITE_MAX_ATTACHED = 10` by default** (hard compile ceiling `125` via `LIBSQLITE3_FLAGS="-DSQLITE_MAX_ATTACHED=32"` in `libsqlite3-sys` [IND]), and SQLite warns (`sqlite.org/lang_attach.html` [IND]) that cross-DB transactions over multiple WAL files are NOT jointly atomic across crashes. Therefore: keep **at most 4 recent warm segments** `ATTACH`ed for fast range scans; query older frozen segments on-demand in dedicated read connections; commit all hot state changes atomically inside `hot.db` only!

---

### 2. Verified Snapshots & Reconciling "State Is a Projection of the Log"

`ARCHITECTURE.md` L131 mandates that core state is a deterministic projection of the log with a single exception for hard forgetting (`R-34`). We satisfy **both** exact deterministic replay and `< 100 ms` cold start using verified periodic state checkpoints paired with `R-34` Two-Epoch Replay:

$$\text{Snapshot } S_k = \bigl(\text{snap\_seq},\; \text{epoch\_id},\; \text{schema\_ver},\; h_{\text{log\_tip}},\; h_{\text{state\_root}},\; \text{state\_bytes}\bigr)$$

- **Invariant:** A snapshot $S_k$ is *never* authoritative over un-erased history; on startup or verification:
  1. Compute `blake3_state_root = BLAKE3(state_bytes)` (~$6\text{–}10\text{ GB/s}$ AVX2/AVX-512 single-threaded via `blake3 1.8.7` [IND] $\implies$ a `100 MB` projection snapshot hashes in **10–16 ms**!).
  2. Verify `snap_seq <= tip_seq` and `epoch_id == current_epoch`, confirm `h_log_tip == events[snap_seq].entry_hash`, deserialize `state_bytes`, and deterministically fold **only the tail** $\Delta_{\text{seq}} = \text{tip\_seq} - \text{snap\_seq}$ events in `seq` order.
  3. **Lab Determinism Gate:** A periodic self-test verifies `Fold(epoch_base, events[0..snap_seq]) == Deserialize(S_k)` byte-for-byte. If corruption or hash mismatch ever occurs, fall back automatically to prior snapshot $S_{k-1}$ or epoch start `epoch_clean_base` (`R-34`), guaranteeing zero unverified state drift.
- **Canonical Deterministic Rust Serialization:** Do **not** use `bincode` (archived/stubbed at `3.0.0`, `RUSTSEC-2025-0141` [IND]). Encode `state_bytes` cleanly with:
  - **`postcard 1.1.3`** (`serde`, compact varint binary, zero padding/hash-randomization) with every map stored as `BTreeMap` or pre-sorted `Vec<(K, V)>` and floats canonicalized (`-0.0 -> +0.0`, canonical NaN bit pattern), **OR**
  - **Deterministic CBOR (`RFC 8949 §4.2` [IND])** via `minicbor 2.3.1` (`2026-10-18` [IND]) when forward/backward schema-tagged evolution across binary upgrades is required.
- **Startup Latency Budget (`T_boot < 100 ms`):**
  $$T_{\text{boot}} = T_{\text{sqlite\_open}} + T_{\text{mmap\_verify}}(|S_k|) + \Delta_{\text{seq}} \cdot t_{\text{apply}} + T_{\text{ann\_view}}$$
  With periodic snapshotting every $K_{\text{snap}} = 5{,}000$ events ($\Delta_{\text{seq}} \le 5{,}000$ at $t_{\text{apply}} \approx 2\,\mu\text{s/ev} \implies \le 10\text{ ms}$ tail replay), BLAKE3 verify (`<= 16 ms` for 100 MB projection), and `mmap` index load (`< 5–15 ms`), cold start holds **well under $40\text{–}60\text{ ms}$** regardless of whether total history is $10^6$ or $10^8$ events.

**Systems Pattern Comparison:**
- **EventStoreDB / KurrentDB (`kurrent.io` [IND]):** Appends to chunked immutable files with stream-version snapshots and chunk hash verification — directly inspires MonoAgens' sealed `seg_NNNN.db` + `snap_seq` design without pulling in C#/gRPC server infrastructure.
- **FoundationDB Record Layer (`foundationdb.org` [IND]):** Read versions (epoch watermarks), multi-page record chunking, and block-level checksums — matches our epoch-tagged `(epoch_id, snap_seq)` and `BLAKE3` block checks.
- **Kafka Log Compaction:** Retains only latest record per key; **rejected for the raw MonoAgens event log** because removing intermediate observations breaks episodic replay and auditability (`2609.33013`), **adopted exclusively inside projections & procedure revisions** where only latest `state_p` / active vectors need fast indexing.

---

### 3. Vector ANN Beyond Brute Force + `R-33` Pre-Filtered Search

#### Rust Crate Landscape (Verified Nov 2026 [IND])
| Crate / Engine | Version / Date | Architecture | In-Traversal Metadata Filtering (`R-33`) | Per-Key Delete / Maintenance Model | Verdict for MonoAgens |
|---|---|---|---|---|---|
| **`usearch`** | `2.26.6` (`2026-10-25`) | C++20/NumKong SIMD HNSW; `.safegraphs` shards; default compiles `bf16`, `i8`, `e4m3`, `u1x8` | **Native** `DenseSearchConfig::predicate` (`Fn + Sync` called inside graph traversal; non-matches still route search) | v2.26 uses `view_nvme` / `refine` / `absorb` / `split` sharding (no single-key `remove()` verb on `DenseIndex`); pair with active bitmap + `sleep_run` shard rebuild | **Primary recommended engine** (`i8` or `bf16` HNSW) |
| **`hnsw_rs`** | `0.3.5` (`2026-10-09`) | Pure Rust HNSW (`anndists`); file dump/mmap reload | **Native** `FilterT` (`search_filter` with closure / sorted `Vec<usize>`) | **Zero delete/remove methods** (`hnsw.rs`); requires external tombstone bitmap + `sleep_run` rebuild | Viable **pure-Rust fallback** (no C++20/CMake toolchain dependency) |
| **`diskann` (`DiskANN3`)** | `0.62.0` (`2026-11-12`) | Microsoft composable Rust Vamana + `diskann-quantization` + `diskann-label-filter` + `diskann-disk` (`DataProvider` trait) | Discrete label filter (`diskann-label-filter`, modeled on `Filtered-DiskANN`) | Real-time streaming updates (`FreshDiskANN` / `IP-DiskANN`) | Strong option if active live vectors exceed RAM ($>10^7$ on NVMe) |
| **`sqlite-vec`** | `0.1.9` stable / `0.1.10-alpha.4` (`2026-05-18`) | Pure-C SQLite `vec0` extension; float/`i8`/binary; experimental `sqlite-vec-ivf.c` & `sqlite-vec-diskann.c` | Partition key / metadata cols in alpha; brute-force full scan in stable `0.1.9` | Row-level virtual table delete, but ANN/DiskANN paths still pre-v1 alpha | Keep watching; **not production-ready** for $>10^5$ low-latency filtered ANN today |
| **`instant-distance` / `hora`** | `0.6.1` (`2023-06-26`) / `0.1.1` (`2021-08-07`) | Pure Rust static HNSW / stale multi-index | None (`instant-distance`) | Immutable one-shot build / unmaintained | **Unsuitable** |

#### Solving `R-33` `VisibleScope` Filtered ANN (`memory-recall/spec.md` Bug Fix)
As established in `R-33`, `memory-recall` post-filtering (fetching top-40 dense vectors globally, *then* removing forbidden `audience`s) leaks presence/ranking via candidate truncation and fails Recall@10 catastrophically when a `VisibleScope` matches only $s \le 1\text{–}5\%$ of memories.
- **Why Filtered-DiskANN (`10.1145/3543507.3583552`, WWW 2023 [PR]) alone is incomplete:** Filtered-DiskANN / FilteredVamana optimizes low-cardinality discrete equality labels, whereas MonoAgens queries arbitrary dynamic `VisibleScope` closures (cross-product of audience sets, taint/clearance ceilings $L_m \sqsubseteq L(A_{\text{trig}})$, active non-tombstoned status, and optional capability preconditions).
- **ACORN (`arXiv:2403.04871` / `10.1145/3654923`, SIGMOD 2024 [PR]) + Traversal Predicates:** ACORN drops naive triangle pruning and expands neighbor lists by $\gamma$ during construction (compressed by $M_\beta$), emulating predicate-subgraph traversal at $2\times\text{–}1{,}000\times$ higher QPS than prior baselines at $0.90$ recall across selectivity regimes, at only $\le 1.3\times$ total HNSW index size ($\text{ACORN-}\gamma$) or $\le 1.25\times$ ($\text{ACORN-1}$, $9\text{–}53\times$ faster build time than $\gamma$-expansion).
- **MonoAgens Hybrid Design:** Combine (a) coarse partitioning by common high-selectivity visibility tier / epoch shard (`public_shared` vs `per_audience` high-traffic shards when large), plus (b) **in-traversal predicate HNSW** (`usearch::DenseSearchConfig::predicate` or `hnsw_rs::FilterT`) backed by a compact **Roaring / bitset precomputed `VisibleSet(context)` $\land$ `LiveVecBitmap`** (so every visited node executes an $O(1)$ bit test, keeping non-matching nodes purely as routing hops!). When average selectivity in a single monolithic shard falls below $\sim 1\%$ and degrades latency, apply **ACORN-1 style 2-hop predicate expansion** or higher $M=32$ connectivity during `sleep_run` builds.

#### Memory, Disk, Build Time, Latency & Recall Sizing (`256d` Matryoshka Default vs `768d`)
Remember `core-and-lab` D11 (`vector_released`) retires raw episodic vectors after episode sleep consolidation: at $10^7\text{–}10^8$ total log events, typically **20–30%** ($V \in [10^6, 10^7]$ active vectors across gists, lessons, procedures, unclosed episodes) stay indexed. Below are verified byte calculations (`HNSW M=16` graph overhead $\approx 168\text{ B/vec}$ for $2M=32$ uint32 layer-0 links + upper levels + 8B key/header; `M=32` $\approx 312\text{ B/vec}$):

| Scale ($V$) & Dim ($D$) | Storage Type | Raw Vector Payload | **Total Index Size (`HNSW M=16`)** | Total Index (`M=32` / ACORN-1 headroom $\times 1.25$) | Projected Recall@10 Target *(verify in Lab; unfiltered / $s=5\%$)* | Projected Latency & Build Note *(verify on target hardware)* |
|---|---|---|---|---|---|---|
| **$V = 10^6$, $D = 256$** *(Default Matryoshka)* | `f32` (4 B/d) | 1.02 GB (0.95 GiB) | **1.19 GB** | 1.34 GB (1.68 GB) | Target $\ge 0.98$ / $\ge 0.93$ | Target **$< 1.5\text{ ms}$ p99** in-RAM (`ef_search=64..128`); builds ~1–3 min |
| **$V = 10^6$, $D = 256$** *(Default)* | **`i8` (1 B/d)** | **256 MB** | **424 MB** (0.40 GiB) | 568 MB (710 MB) | Target $\ge 0.96$ ($\sim 0.98$ w/ f32 top-40 rescore) | Target **$< 1.0\text{ ms}$ p99** via NumKong/SimSIMD int8 SIMD; fits easily in RAM |
| **$V = 10^6$, $D = 256$** | **`RaBitQ 1-bit` / `u1x8`** | **44 MB** (`32B + 12B meta`) | **212 MB** | 356 MB | Requires top-$r$ rescore (`RaBitQ` [PR]) | Sub-ms bitwise distance scan; theoretical $O(1/\sqrt{D})$ error bound (`10.1145/3654970`) |
| **$V = 10^6$, $D = 768$** *(Full Gemma)* | `f32` vs `bf16` vs `i8` | 3.07 / 1.54 / 0.77 GB | **3.24 / 1.70 / 0.94 GB** | 3.38 / 1.85 / 1.08 GB | Comparable recall @ $\sim 2\text{–}3\times$ compute | $3\times$ payload footprint vs `256d`; `i8` keeps $10^6$ under 1 GB |
| **$V = 10^7$, $D = 256$** *(Multi-year large)* | **`bf16` (2 B/d)** | 5.12 GB | **6.80 GB** | 8.24 GB | Target $\ge 0.97$ / $\ge 0.92$ | Target **$< 3\text{ ms}$ p99** warm RAM; `i8` drops footprint to **4.24 GB** |
| **$V = 10^7$, $D = 256$** *(Commodity SSD tier)* | `ExRaBitQ 4-bit` (`~144 B`) / `RaBitQ 1-bit` | 1.44 GB / 440 MB | **3.12 GB / 2.12 GB** | 4.56 GB / 3.56 GB | Target $\ge 0.95$ w/ top-200 SQLite rescore (`2409.09913`) | Warm RAM holds 10M compressed graph in **2–4 GB** + SQLite NVMe rescore |
| **$V = 10^7$, $D = 768$** | `f32` vs `i8` vs `RaBitQ 1-bit` | 30.72 / 7.68 / 1.08 GB | **32.40 / 9.36 / 2.76 GB** | 33.84 / 10.80 / 4.20 GB | Target $\ge 0.95$ (rescore on SSD) | `f32` needs server RAM or `diskann 0.62.0` SSD Vamana (ref: `Subramanya` `<3 ms` SIFT1B) |

*(Note: Byte totals above are exact arithmetic sums of raw payload + HNSW edge overhead; Recall@10 and p99 latency columns are inferred engineering targets grounded in `1603.09320`, `2403.04871`, `2405.12497`, and `USearch` SIMD behavior, not empirical MonoAgens measurements yet. Because `usearch 2.26.6` default crate builds compile `bf16`, `i8`, `e4m3`, `u1x8` kernels natively, **`i8` (with top-40 float re-scoring from SQLite)** or **`bf16` (for direct accuracy without re-score)** is the lowest-friction default at `256d`! Exact p99 latencies under MonoAgens' selective `VisibleScope` bitmaps must be established in the Lab microbenchmark.)*

---

### 4. Vector Tombstones, `vector_released` & `R-34` Forensic Compaction

Appending vectors dynamically across years produces two deletion workloads: routine `vector_released` (`core-and-lab` D11) and forensic `hard_forget` (`R-34`). Retaining dead HNSW nodes degrades both search QPS (excess tombstone hops) and privacy (`R-34` forbids residual deleted embeddings on disk).

1. **Fast Path Between Consolidations ($O(1)$ Exclusion):**
   - Maintain a lock-free/versioned `LiveVecBitmap` mapping internal `vec_slot -> active: bool`. Deleting or releasing a vector immediately flips its bitmap bit to `false`, causing the HNSW traversal predicate (`LiveVecBitmap(id) && VisibleScope(id)`) to reject it deterministically on the very next query with zero graph mutation.
   - Zero-overwrite or unlink raw `f32`/`bf16` payload rows inside SQLite (`PRAGMA secure_delete = ON`, `R-34`).
2. **Sleep-Time Compaction & Forensic Rebuild (`sleep_run`):**
   - Trigger full clean HNSW shard rebuilds whenever **either**:
     $$\frac{N_{\text{tombstones}}}{N_{\text{total\_slots}}} > \theta_{\text{tomb}} = 0.15 \qquad\text{OR}\qquad \text{pending\_hard\_forget\_vec\_count} > 0$$
   - Because $10^6$ `256d` `i8` vectors build a fresh HNSW shard in ~1–3 minutes in background `sleep_run`, building a replacement shard from live SQLite rows (`(id, qvec)`), saving atomically, swapping the read handle via `ArcSwap`/double-buffering (zero query downtime!), and overwriting + unlinking the obsolete shard guarantees **0% residual forgotten embeddings** on disk (matching `R-34`'s security bar) without needing fragile in-place HNSW edge repair.
   - *(Scaling note for $>10^7$ live vectors where offline rebuild exceeds a single `sleep_run` window: split the index into time/epoch **shards** via `usearch`'s `.safegraphs.json` manifest — rebuilding only mutated shards containing tombstones — or adopt **FreshDiskANN** streaming delete-consolidation rules `arXiv:2105.09613` [PP] / `diskann 0.62.0`, $5\text{–}10\times$ lower maintenance cost than full static rebuilds, scheduling a forensic wipe whenever `hard_forget` touches a shard.)*

---

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `M_seg` | Max events per SQLite segment (`hot.db` rotation threshold) | `500_000` | `100_000–2_000_000` | Systems sizing (~326 MB/seg keeps hot B-trees & `VACUUM` <1–2s); tune in Lab |
| `K_snap` | Events appended between periodic verified snapshots $S_k$ | `5_000` | `1_000–25_000` | Bounds startup replay tail $\le 10\text{ ms}$; tune in Lab |
| `max_attached_warm` | Max warm sealed segments concurrently `ATTACH`ed | `4` | `1–8` (`< 10` SQLite default) | SQLite `SQLITE_MAX_ATTACHED=10` [IND]; avoid cross-DB WAL writes |
| `ann_dim` | Vector dimensionality (Matryoshka truncated default) | `256` | `256` or `768` | Round 1 `R-13` / `A-09` (`EmbeddingGemma-300M`); tune recall vs RAM in Lab |
| `ann_quant` | Stored HNSW scalar quantization (`usearch 2.26.6`) | `i8` (+ top-40 exact rescore) | `{bf16, i8, u1x8}` | `usearch 2.26.6` compiled scalars [IND]; tune in Lab |
| `hnsw_m` | HNSW max node degree per upper layer (`L0 = 2*M`) | `16` | `16–32` | Malkov & Yashunin `1603.09320` [PR]; raise to `32` (`ACORN-1`) for selective `VisibleScope` |
| `hnsw_ef_search` | HNSW query beam width (scaled up under low selectivity) | `64` | `32–256` | `1603.09320` [PR] & `2403.04871` [PR]; adaptive with `1/s` |
| `theta_tomb` | Tombstone ratio triggering background shard HNSW rebuild | `0.15` | `0.05–0.30` | Tune in Lab (`hard_forget` always forces immediate shard rebuild) |
| `M_shard_vec` | Max live vectors per HNSW shard before splitting a new shard | `1_000_000` | `250_000–2_500_000` | Keeps per-shard rebuild within bounded `sleep_run` window & RAM budget |

---

## Neutral value

- Setting `M_seg = infinity` (single `log.db`), `K_snap = infinity` (replay from `seq = 1` or epoch start), and switching `ann_engine = BruteForceFlat` (`hnsw_m = 0`): reproduces the exact Day-1 `core-and-lab` Decision 15 behavior (single SQLite database, full log replay at startup, exact scan over active vectors). Every invariant holds unchanged for small evaluation suites ($N \le 10^5$).

---

## Claim

1. **Log-projection equivalence with `< 100 ms` boot at $10^7\text{–}10^8$ events:** Periodic verified `(snap_seq, h_log_tip, h_state_root)` snapshots (`postcard` / Deterministic CBOR + `BLAKE3`) keep cold startup under `50–100 ms` from $10^4$ to $10^8$ events (vs tens of seconds to minutes for full replay), while bit-exact `Fold == Snapshot` verification proves zero projection drift and segments isolate `R-34` `VACUUM` pauses to `<= 350 MB` chunks instead of `65 GB` locks.
2. **Filtered ANN speedup without privacy leaks (`R-33` fix):** Quantized `i8` HNSW (`256d`) + top-40 exact rescoring + in-traversal `VisibleScope` bitset filtering achieves `< 3–5 ms` p99 search latency and `Recall@10 >= 0.93` down to `1–5%` audience selectivity at $10^6\text{–}10^7$ vectors using only `0.42–4.2 GB` RAM—where `core-and-lab` brute-force scan exceeds `50–500 ms` and post-filter drops recall toward zero at low selectivity.

---

## How the Lab would test it

1. **Long-Log Synthetic Stress Benchmark (`N in {10^5, 10^6, 10^7}` deterministic seeded events):**
   - Measure DB growth on disk, WAL size/checkpoint p99 latency during concurrent event bursts, and `VACUUM + secure-delete` lock duration comparing monolithic `log.db` vs `M_seg = 500_000` segmented `hot.db` + `seg_*.db`.
   - Measure cold-start time `T_boot` vs `K_snap in {1_000, 5_000, 25_000, inf}` and run the automated equivalence check verifying `BLAKE3(ReplayFromEpoch(tip)) == BLAKE3(LoadSnapshot(K) + TailReplay)` across schema migrations.
2. **Filtered `VisibleScope` Vector Recall@10 vs QPS / p99 Sweep (`V in {10^5, 10^6, 5e6}` at `256d` vs `768d`):**
   - Generate realistic skewed multi-actor `VisibleScope` bitmaps with selectivities $s \in \{0.5\%, 1\%, 5\%, 25\%, 100\%\}$.
   - Benchmark brute-force `f32`, post-filtered HNSW (expect recall collapse bug!), in-traversal predicate `usearch` (`i8` & `bf16`, $M \in \{16, 32\}$), `hnsw_rs` (`FilterT`), and `ACORN-1` 2-hop expansion on **Recall@10, p99 query ms, RAM RSS, and shard rebuild time**.
3. **Forensic Churn & `hard_forget` Rebuild Latency Test:**
   - Simulate 20% `vector_released` + 5% `hard_forget` deletions over 1M vectors; verify via raw file byte scan that zero deleted embeddings/strings survive post-compaction while shard switch incurs `0 ms` read blocking.

---

## Established / inferred / open

- **Established by literature & primary engineering specs:**
  - SQLite B-tree page/overflow limits (`X = U - 35`), WAL frame mechanics, `SQLITE_MAX_ATTACHED = 10` (`125` hard max), non-atomic cross-DB WAL caveats, and FTS5 `secure-delete` (`sqlite.org` [IND]; SQLite `3.54.1` via `libsqlite3-sys 0.40.0`).
  - `BLAKE3` (`blake3 1.8.7`) runs at `~6–10.5 GB/s` single-threaded SIMD; `bincode 3.0.0` is ceased/unmaintained (`RUSTSEC-2025-0141`); `postcard` / RFC 8949 §4.2 provide deterministic canonical encoding with sorted maps.
  - Standard HNSW post-filtering collapses under selective predicates, whereas ACORN (`arXiv:2403.04871` / `10.1145/3654923` [PR]) and traversal predicates maintain high QPS at 0.9 recall with $\le 1.3\times$ memory overhead; RaBitQ (`arXiv:2405.12497` / `10.1145/3654970` [PR]) provides 1-bit quantization with a sharp $O(1/\sqrt{D})$ error bound.
  - Current Nov 2026 Rust crate capabilities verified in source (`usearch 2.26.6`, `hnsw_rs 0.3.5`, `diskann 0.62.0`, `sqlite-vec 0.1.9` stable vs `0.1.10-alpha.4` IVF/DiskANN).
- **Inferred for MonoAgens:**
  - Segmented epoch SQLite databases linked by a contiguous `BLAKE3` sequence chain + `(snap_seq, epoch_id, h_log_tip, h_state_root)` checkpoints reconcile `ARCHITECTURE.md` log-projection determinism, `< 100 ms` boot, and `R-34` Two-Epoch Replay/forgetting.
  - Using `256d` `i8`-quantized HNSW + top-40 exact rescoring from SQLite + `LiveVecBitmap ∩ VisibleScope` predicate filtering + `sleep_run` shard compaction satisfies both latency (`< 5 ms`) and `R-33`/`R-34` privacy invariants.
- **Open (not settled by literature — resolve in Lab):**
  - Exact crossover selectivity $s^*$ where standard predicate HNSW ($M=16$) must upgrade to $M=32$ or `ACORN-1` 2-hop expansion on MonoAgens' real actor/place correlation patterns; optimal `M_seg`, `K_snap`, and `theta_tomb` values under real `vector_released` churn rates; direct MonoAgens benchmark comparing `usearch 2.26.6` shards vs `diskann 0.62.0` vs pure-Rust `hnsw_rs 0.3.5`.

---

## Sources

- `[PR]` Malkov & Yashunin, *Efficient and robust approximate nearest neighbor search using Hierarchical Navigable Small World graphs* (`arXiv:1603.09320`, IEEE TPAMI 2018).
- `[PR]` Subramanya et al., *DiskANN: Fast Accurate Billion-point Nearest Neighbor Search on a Single Node* (NeurIPS 2019, `https://papers.nips.cc/paper/9527-rand-nsg-fast-accurate-billion-point-nearest-neighbor-search-on-a-single-node.pdf`).
- `[PR]` Gollapudi et al., *Filtered-DiskANN: Graph Algorithms for Approximate Nearest Neighbor Search with Filters* (WWW 2023, DOI `10.1145/3543507.3583552`).
- `[PR]` Patel, Kraft, Guestrin & Zaharia, *ACORN: Performant and Predicate-Agnostic Search Over Vector Embeddings and Structured Data* (`arXiv:2403.04871`, SIGMOD 2024 / PACMMOD, DOI `10.1145/3654923`).
- `[PR]` Gao & Long, *RaBitQ: Quantizing High-Dimensional Vectors with a Theoretical Error Bound for Approximate Nearest Neighbor Search* (`arXiv:2405.12497`, SIGMOD 2024 / PACMMOD, DOI `10.1145/3654970`).
- `[PP]` Singh et al., *FreshDiskANN: A Fast and Accurate Graph-Based ANN Index for Streaming Similarity Search* (`arXiv:2105.09613`).
- `[PP]` Gao et al., *Practical and Asymptotically Optimal Quantization of High-Dimensional Vectors in Euclidean Space for Approximate Nearest Neighbor Search (Extended RaBitQ)* (`arXiv:2409.09913`).
- `[IND]` SQLite File Format (`https://www.sqlite.org/fileformat2.html`), WAL (`https://www.sqlite.org/wal.html`), `ATTACH` (`https://www.sqlite.org/lang_attach.html`), Limits (`https://www.sqlite.org/limits.html`), FTS5 `secure-delete` (`https://www.sqlite.org/fts5.html`).
- `[IND]` Rust ecosystem verified Nov 2026 (`crates.io` & upstream repos): `rusqlite 0.43.0` (`2026-11-15`), `libsqlite3-sys 0.40.0` (`2026-11-11`, bundled SQLite `3.54.1`), `sqlite-vec 0.1.9` / `0.1.10-alpha.4` (`2026-05-18`, `https://github.com/asg017/sqlite-vec`), `usearch 2.26.6` (`2026-10-25`, `https://github.com/unum-cloud/USearch`), `hnsw_rs 0.3.5` (`2026-10-09`, `https://github.com/jean-pierreBoth/hnswlib-rs`), `diskann 0.62.0` (`2026-11-12`, `https://github.com/microsoft/DiskANN`), `simsimd 6.5.16` (`2026-03-07`), `blake3 1.8.7` (`2026-08-20`), `postcard 1.1.3` (`2025-07-24`), `minicbor 2.3.1` (`2026-10-18`), `bincode 3.0.0` deprecation / `RUSTSEC-2025-0141`.
- `[IND]` RFC 8949 §4.2 Deterministic CBOR (`https://www.rfc-editor.org/rfc/rfc8949#section-4.2`); EventStoreDB/KurrentDB (`https://kurrent.io`); FoundationDB Record Layer (`https://foundationdb.org`).
