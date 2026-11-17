# MonoAgens Run 3 Research Notes: Prospective Memory & ProactiveBench (`pm_bench`, `pis`, `capa`, `proactive_bench`)

**Scope:** External benchmark verification (`R-51`–`R-54`) for the four prospective-memory and proactive-delegation/triggering IDs in `crates/lab/src/benchmark.rs` (`standard_benchmark_registry()`): `pm_bench`, `pis`, `capa`, and `proactive_bench`.  
**Constraint check:** All facts below are verified from primary sources (`arxiv.org`, `github.com`, `huggingface.co`, `aclanthology.org`) via web search during Run 3. Repository commit SHAs, individual data file names/byte sizes inside repositories whose file trees are not exposed in search snippets, and verbatim judge prompt strings are explicitly marked **`unverified`** rather than extrapolated. No files were downloaded into `lab/datasets/`, `lab/scenarios/sealed/` was never read, and no code or spec files were edited.

---

## 1. Executive Summary & Owner Choices Needed

### Key Findings at a Glance

1. **`pis` (`arXiv:2609.01272`) is NOT an independent public dataset (`unavailable` / method-only).**  
   *Making Prospective Memory SLM-Shaped: Typed Intention Stores for Small-Model Agents* (Jinqing Zhao & Chengcan Wu, Sept 2026) introduces **PIS (Prospective Intention Store)**, a training-free agent scaffold evaluated **on `pm_bench`**. No independent `pis` dataset or public repository exists. Per `openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md`, it must be marked `unavailable` (or removed from the dataset registry in a future spec change) rather than silently aliased to `pm_bench`.
2. **`pm_bench` (`genglinliu/PMBench`, `arXiv:2607.12385`) has no root `LICENSE` file and a metric-name mismatch.**  
   - **License gap:** The GitHub repo `genglinliu/PMBench` lacks a standalone `LICENSE` file (observed in Run 2 and confirmed un-indexed/absent in Run 3). Under `benchmark-datasets/spec.md`, fetch must remain blocked until the owner contacts the authors or clears licensing.  
   - **Metric mismatch:** Registry lists `commission_error_rate`, whereas `PM-Bench`'s official metrics are **`Set-F1`**, **`False-Alarm (FA)`** / **`FP count`**, **`Precision`**, **Hit Rate (On-time/Late)**, **Cross-Day**, and **Update hit**.  
   - **Power:** 1 simulated 7-day week = **80 decision steps** (**83 task definitions**, **81 scored**). It cannot reach $S \ge 354\text{--}471$ independent items; it must be run in full as a structural diagnostic.
3. **`capa` (`FKIRSTE/emnlp2026-meeting-delegation`, `arXiv:2609.03923`) is dual-licensed (MIT + CC BY 4.0) and lacks per-turn timestamps in QMSum JSON.**  
   - **Provenance & license:** Code/prompts are **MIT**; underlying **137 AMI scenario meetings** (**1,126 idea-unit episodes**) are sourced from the AMI portion of QMSum under **CC BY 4.0** (AMI Meeting Corpus).  
   - **Official metrics match registry:** `credited_recovery` (loose recall) and `silence_rate` match the paper's headline metrics. Requires schema-constrained LLM judges (`GPT-4o` reported; Cohen's $\kappa = 0.71$ with humans).  
   - **Mapping caveat:** QMSum's `meeting_transcripts` JSON stores `{speaker, content}` in order **without wall-clock timestamps**, so conversion must synthesize deterministic turn-index virtual `timestamp_ms` (or fetch raw AMI NXT XML timings separately).
4. **`proactive_bench` (`thunlp/ProactiveAgent`, `arXiv:2410.12361`) has a public name collision and an underpowered official test split ($N = 233$).**  
   - **Name collision:** `ProactiveBench` matches **both** the intended **THUNLP ProactiveBench** (`thunlp/ProactiveAgent`, Apache-2.0, `arXiv:2410.12361`, ICLR 2025 — anchored by `benchmark.rs`'s `"Zero-LLM gate parity vs TGL"` caveat referencing `arXiv:2605.30152`) **and** an unrelated ECCV 2026 multimodal VQA benchmark (`tdemin16/proactivebench`, MIT).  
   - **Metric & split caveats:** Registry `false_detection` aligns with `FD` (False Positive / False-Detection scenarios), though the paper reports both raw `FD` outcome counts and reward-model `FD` agreement / rate; official test split has **233 test events** (some repo docs/issues mention **227** in an iteration/subset — verify exact file line count on fetch), which is below $S \ge 354$.

### Owner Choices Needed

| Dataset ID | Decision Point | Option A | Option B | Option C |
|---|---|---|---|---|
| **`pm_bench`** | Missing GitHub `LICENSE` file on `genglinliu/PMBench` | Contact Genglin Liu (UCLA) for explicit MIT/Apache/CC-BY grant before setting `license.status = "cleared"` | Keep `license.status = "pending"` (fetch blocked; reported as unverified/not run in standard suite) | — |
| **`pm_bench`** | Registry metric `commission_error_rate` vs official `False-Alarm (FA)` / `FP` | Rename registry metric to `false_alarm_rate` (matching official `PM-Bench` definition) | Keep `commission_error_rate` in registry as a Lab-renamed metric computed strictly over cancelled/lure false triggers | — |
| **`pis`** | `arXiv:2609.01272` is a method evaluated on `pm_bench`, with no independent dataset | Mark `availability = "unavailable"` in `lab/datasets.manifest.toml` (reason: method-only paper, no independent dataset) | Remove `pis` from `standard_benchmark_registry()` (in a code/spec change) and benchmark MonoAgens `expectations` vs PIS on `pm_bench` | *(Disallowed by spec without explicit change)* Alias `pis` to `pm_bench` (would double-count `pm_bench`) |
| **`capa`** | Evaluation split vs power target ($S \ge 354\text{--}471$) | Evaluate strictly on the **20 AMI test meetings** (~164 estimated episodes; `< 354`, underpowered) to preserve QMSum's train/val/test boundary | Evaluate across **all 137 AMI meetings (1,126 episodes)** as in the CAPA paper (with cluster-stratified subset $S = 400\text{--}471$ episodes if budgeting live runs, noting 97 meetings are QMSum train) | — |
| **`capa`** | Missing per-turn timestamps in QMSum `meeting_transcripts` JSON | Synthesize deterministic turn-index virtual `timestamp_ms` (`t_0 + i * Δt`) from utterance order | Fetch raw AMI Corpus NXT/XML timestamps externally to populate real utterance start/end ms | Mark `not_mappable` under strict real-timestamp requirement (not recommended; turn order is intact) |
| **`proactive_bench`** | Test split size (**233** events `< 354`) & `TGL` gate-parity vs RM scoring | Run **100% of official test split (233 events)** and report with exact underpowered CIs | Supplement with a seeded stratified sample ($S \ge 400$) from the unused **6,790 train events** (clearly labeled non-official split; higher contamination risk) | Score **both** deterministic trigger-only F1/FD (`TGL` parity, 0 judge calls) and full RM-judged content F1 (`YancyLee/ProactiveAgent` RM or LLM judge) |

---

## 2. Detailed Per-Dataset Analysis

### 2.1 `pm_bench` — PM-Bench: Evaluating Prospective Memory in LLM Agents

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **PM-Bench** (*"PM-Bench: Evaluating Prospective Memory in LLM Agents"*, Genglin Liu & Saadia Gabriel, COLM 2026).
- **Canonical source:**
  - Repository: `https://github.com/genglinliu/PMBench`
  - Paper: `https://arxiv.org/abs/2607.12385` (submitted July 2026)
- **Pinned revision & date:** Exact commit SHA and release tag are **`unverified`** (GitHub file tree/commits for `genglinliu/PMBench` are not exposed in public web search snippets). Must be pinned via `git ls-remote https://github.com/genglinliu/PMBench.git` at owner-authorized inspection/fetch.
- **Files needed & sizes:** Search snippets confirm standard layout (`README.md`, `requirements.txt`, `data/`, and `eval/` scoring scripts), encoding the 7-day Virtual Week simulation (80 steps, 83 task definitions, 74 lures, 11 updates, 11 state channels). Exact filenames inside `data/` and `eval/`, and byte sizes, are **`unverified`**.
- **Checksums:** None published at source (`checksums_recorded = "owner"` required via `monoagens-lab datasets fetch --record-checksums`).
- **License & terms:**
  - **Code/Data repo:** Standalone `LICENSE` file is **absent / 404** in `genglinliu/PMBench` (verified in Run 2 and corroborated by Run 3 search). No SPDX license is declared on the repository. Without a license file, standard copyright applies: local copies, commercial use, and redistribution of derived files are **not explicitly permitted** until clarified with the authors (Genglin Liu, UCLA).
  - **Paper:** `arXiv:2607.12385` (Run 2 recorded CC BY 4.0 on the paper PDF; unverified independently in Run 3 snippets).
  - **Access gate:** None observed (public GitHub repository URL), though web search crawlers did not index internal directory trees.

#### R-52 Official Evaluation Protocol
- **Official metrics & definitions (`arXiv:2607.12385`):**
  - **Set-F1 (primary metric):** Evaluated across decision steps $t \in \{1,\dots,80\}$, comparing the set of currently due tasks $D_t$ and agent-selected task handles $C_t$:
    $$\text{Set-F1} = \frac{2\,\text{TP}}{2\,\text{TP} + \text{FP} + \text{FN}}$$
  - **Additional official metrics:** **Precision**, **Hit Rate** (**On-time** and **Late** hits), **Cross-Day** hit rate, **Update hit** (on cancel/override/reschedule events), **False-Alarm (FA)** (false positives normalized by decision steps) / **FP count**, and **Queries** (calls to `query_state` / `check_time`).
- **Registry metric alignment (`crates/lab/src/benchmark.rs`):**
  - `set_f1` — **Official** (exact match to `Set-F1`).
  - `commission_error_rate` — **NOT official in `PM-Bench`.** "Commission error" is the classical cognitive-psychology term (e.g., Scullin, Bugg & McDaniel 2012) for executing a completed, cancelled, or lure intention. Closest official `PM-Bench` metrics are **False-Alarm Rate (`FA`)** / **FP count** (which penalize premature actions, lures, and action spamming) and **`1 - Precision`**. Under `benchmark-evaluation/spec.md`, either rename the registry metric to `false_alarm_rate` or keep `commission_error_rate` explicitly documented as a non-official lure/post-completion error metric.
- **Official split:** Single unified 7-day Virtual Week trajectory (80 decision steps; no train/test partition).
- **Official scoring script & revision:** Programmatic set-matching scorer inside `genglinliu/PMBench` (`eval/`; exact filename and commit SHA **`unverified`**). Can be reimplemented deterministically in Rust with parity fixtures.
- **LLM judge required?** **No.** Purely deterministic set matching on structured JSON outputs (`{"action": "choose", "choice": ..., "task_ids": [...], "channel": ...}`).
- **Contamination:** Published July 2026 (`arXiv:2607.12385`, COLM 2026). Post-dates pre-2026 training cutoffs, though late-2026 web crawls may index the paper/repo.

#### R-53 Mapping to MonoAgens `Percept` & `GoldBook`
- **Raw format & units:** Interactive 7-day Virtual Week text simulation spanning **80 decision steps** (10–13 steps/day). Each step presents a simulated day/time context, a short narrative, ongoing-activity choices, latent state channels (11 channels), dynamic intention updates (11 updates: 2 cancel, 3 override, 6 reschedule), and a menu of anonymous action handles (`task_ids`) mixing due tasks (83 task definitions, 81 scored) with **74 lures**. The agent responds in JSON using `choose` (with `choice` and `task_ids`), `query_state`, or `check_time`.
- **6-Fact `Percept` mapping:**
  - `actor`: `"env:virtual_week"` (environment narrator/simulator; `origin_actor = "env:virtual_week"`).
  - `place`: `"place:virtual_week_sim"` (single simulated environment place).
  - `audience`: `[soul_actor_id, "env:virtual_week"]` (satisfies non-empty `Audience` invariant).
  - `timestamp_ms`: Deterministic virtual clock derived from the simulation's `(day 1..7, step_time)` (e.g., epoch offset + day/hour/minute in ms).
  - `payload`: `OpaquePayload` carrying the step narrative, ongoing-activity menu, available anonymous `task_ids` handles, and any returned `channel` / time observation from prior `query_state` / `check_time` calls.
  - `capability` & `receipt`: Soul emits `Action` to a Virtual Week body capability (`choose`, `query_state`, `check_time`); the Lab simulator processes the JSON selection and emits a `Receipt` (and state-observation `Percept` if queried).
- **`GoldBook` separation:** Ground-truth due task sets $D_t$ at each step $t$, task types (event/time, regular/non-regular, cross-day), update schedules, and lure flags stay strictly inside `GoldBook`. The soul only ever sees anonymous `task_ids` and observable narratives.
- **Honesty / Missing-facts caveats:** **Fully mappable** with zero fabricated facts—simulated clock provides timestamps and the simulator provides `actor`. Two structural caveats: (1) single-user synthetic environment with no multi-party interruption noise (already noted in `benchmark.rs`), and (2) **interactive multi-step protocol** (the Lab driver must step the Virtual Week environment per action so `query_state`/`check_time` return latent channel values dynamically rather than flattening 80 steps statically).

#### R-54 Sampling, Power (`S >= 354–471`), Stratification & Cost
- **Item counts:** **1** 7-day trajectory = **80 decision steps**, **83 task definitions** (**81 scored**: 57 event-based, 26 time-based; 28 regular, 55 non-regular; 15 channel-triggered; 7 cross-day), **11 updates**, **74 lures** (per Run 2 / `arXiv:2607.12385` summary; exact per-file breakdown unverified without repo clone).
- **Power feasibility (`S >= 354–471`):** **Not feasible on `pm_bench` alone.** Even counting individual steps ($N = 80$) or scored tasks ($N = 81$) falls far below $S = 354\text{--}471$, and steps within a single 7-day timeline are serially dependent. `pm_bench` should be run in full as a deterministic paired comparison across arms, not treated as an independent $S \ge 354$ trial.
- **Stratification rule:** **Do not subsample**—run 100% of the 80 steps. Report disaggregated sub-metrics stratified by task type (event-based vs time-based, regular vs non-regular, channel-triggered, cross-day, updated, and lure steps).
- **Per-item LLM/judge cost shape:** ~80 decision turns per full benchmark run + intermediate `query_state`/`check_time` calls (~80–160 soul LLM calls total for the entire dataset); **0 LLM judge calls**.

---

### 2.2 `pis` — Typed Prospective Intention Store (`arXiv:2609.01272`)

#### R-51 Provenance
- **Public existence & canonical name:** **`unavailable` as a dataset.**  
  `arXiv:2609.01272` (*"Making Prospective Memory SLM-Shaped: Typed Intention Stores for Small-Model Agents"*, Jinqing Zhao & Chengcan Wu, Peking University, Sept 2026) introduces **PIS (Prospective Intention Store)**, which is a **training-free agentic scaffold / memory architecture** (externalizing deferred commitments into typed trigger-action-lifecycle records $(\phi, \alpha, \sigma)$ handled by deterministic code while delegating scoped semantic grounding to SLMs).
- **Canonical source:** Paper at `https://arxiv.org/abs/2609.01272`. No public official dataset or standalone GitHub/Hugging Face repository for `pis` was verified in Run 3 searches (community aggregators such as `Awesome-AI-Memory` index the paper; author profile `github.com/ChengcanWu` and `github.com/Always-On-Intelligence` exist, but no `pis` dataset release was found).
- **Benchmark evaluated in `arXiv:2609.01272`:** **PM-Bench** (`genglinliu/PMBench`) — e.g., `DeepSeek-Chat + PIS` reaches 82.9% Set-F1 and `Gemma-E2B + PIS` improves from 4.2% to 66.2% Set-F1.
- **Pinned revision / files / checksums / license:** Not applicable (`availability = "unavailable"`, reason: method-only paper evaluated on `pm_bench`; no independent public dataset).

#### R-52 Official Evaluation Protocol
- **Official metrics & alignment:** `arXiv:2609.01272` reports **`Set-F1`** on **`PM-Bench`** (matching `benchmark.rs`'s `&["set_f1"]`).
- **Status in Lab:** Cannot be scored independently of `pm_bench`. Per `benchmark-datasets/spec.md` ("The manifest names datasets that are not yet published"), `pis` should be marked `unavailable` in `lab/datasets.manifest.toml` and reported as not run with reason `method-only paper (arXiv:2609.01272) evaluated on pm_bench; no independent dataset`, rather than silently aliased to `pm_bench`.

#### R-53 & R-54 Mapping & Sampling
- **Not applicable** as a standalone dataset (inherits `pm_bench`'s exact percept mapping, 80-step / 81-task size, and cost shape if reproduced as a baseline method).

---

### 2.3 `capa` — Online Meeting Delegation / CAPA (`FKIRSTE/emnlp2026-meeting-delegation`, `arXiv:2609.03923`)

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Paper: *"Speak for Me: Giving LLMs the Situational Awareness to Participate in a Meeting"* (Muneeb Khan, Frederic Kirstein, Terry Ruas, Bela Gipp; EMNLP 2026, `arXiv:2609.03923`). Framework: **CAPA** (Collaborative Agent Predictive Architecture) for online meeting delegation on AMI meetings.
- **Canonical source:**
  - Repository: `https://github.com/FKIRSTE/emnlp2026-meeting-delegation`
  - Paper: `https://arxiv.org/abs/2609.03923`
  - Upstream data lineage: **AMI Meeting Corpus** (Carletta et al., 2005) via **QMSum** (Zhong et al., 2021), stored as parsed JSON transcripts under `data/ami/` in `FKIRSTE/emnlp2026-meeting-delegation`.
- **Pinned revision & date:** Exact commit SHA and release tag on `FKIRSTE/emnlp2026-meeting-delegation` are **`unverified`** from search snippets (EMNLP 2026 / `arXiv:2609.03923`, Sept 2026). Must be pinned upon fetch/inspection.
- **Files needed & sizes:** Search snippets confirm `data/README.md`, `data/ami/` (parsed JSON transcripts for 137 AMI scenario meetings: 97 train, 20 validation, 20 test), `shared_memory.py`, `environment_interpreter.py`, `two_tier_agent.py`, `capa/predictor.py`, `capa/judge.py`, `capa/delegate_judge.py`, `prompts/`, `requirements.txt`, and `LICENSE`. Exact filenames inside `data/ami/` and `prompts/` (plus episode annotation files) and byte sizes are **`unverified`**.
- **Checksums:** None published at source (`checksums_recorded = "owner"` via `--record-checksums`).
- **License & terms:**
  - **Code/scripts/prompts (`FKIRSTE/emnlp2026-meeting-delegation`):** **MIT License** (`SPDX: MIT`). Permits local copies, commercial use, modification, and redistribution with copyright/license notice.
  - **Meeting transcript data (AMI Meeting Corpus via QMSum):** **CC BY 4.0** (`SPDX: CC-BY-4.0`). Permits local copies, commercial use, adaptation, and redistribution with attribution.
  - **Ethical/deployment note:** Authors specify CAPA is offline research infrastructure; live deployment in meetings requires explicit recurring participant consent. No download gate observed on GitHub.

#### R-52 Official Evaluation Protocol
- **Official metrics & definitions (`arXiv:2609.03923`):**
  - **Silence rate (not-attempted rate):** Fraction of valid talking opportunities (idea-unit episodes of the absent participant $u^\star$) where the delegate stays `SILENT` (prompt-only baseline: 51.4%; CAPA: 2.5%).
  - **Credited recovery (Loose recall):** Fraction of idea-unit episodes where the delegate surfaces $u^\star$'s target proposition within a $k$-turn window following a cue without hallucination or redundancy (prompt-only: 26.1; CAPA: 52.2).
  - **Additional official metrics:** **Strict recall** (hit at exact anchor turn), **Decision F1**, **Hallucination rate** (e.g., 0.6%), **Redundancy**, **Alignment**, and **Environment forecast errors**.
- **Registry metric alignment (`crates/lab/src/benchmark.rs`):**
  - `credited_recovery` — **Official** (matches credited recovery / loose recall).
  - `silence_rate` — **Official** (matches silence / not-attempted rate).
- **Official split:** The 137 AMI scenario meetings follow QMSum's **97 train / 20 validation / 20 test** partition, containing **1,126 idea-unit episodes** across all 137 meetings (the paper reports main results across all 137 AMI meetings / 1,126 episodes, as well as a 20-meeting matched test/ablation subset). *Owner choice needed:* whether Lab reports on the **20 test meetings** or the **full 137-meeting (1,126-episode) pool**.
- **Official scoring script & revision:** `capa/judge.py` and `capa/delegate_judge.py` (+ `eval/` scripts) in `FKIRSTE/emnlp2026-meeting-delegation` (revision **`unverified`**). Because scoring depends on schema-constrained LLM judge prompts, Lab should invoke the official Python judge script via `lab/adapters` (or port its exact pinned prompt verbatim with a parity test).
- **LLM judge required?** **Yes.** Schema-constrained LLM judges (`Delegate Judge` for grounding/hallucination/redundancy/alignment/recovery; `Environment Judge` for state forecasts) using `GPT-4o` in the paper (human agreement Cohen's $\kappa = 0.71$). Exact prompt strings and judge parameters in `prompts/` / `capa/delegate_judge.py` are **`unverified`** verbatim from search snippets.
- **Contamination:** High for raw **AMI Meeting Corpus** (2005) and **QMSum** (2021) transcripts in pretraining corpora; low for CAPA's causal leave-one-participant-out episode cues, profiles ($p_u$), and briefings ($b_u$) (`arXiv:2609.03923`, Sept 2026).

#### R-53 Mapping to MonoAgens `Percept` & `GoldBook`
- **Raw format & units:** QMSum-derived AMI JSON transcripts (`meeting_transcripts`: chronologically ordered list of `{"speaker": "<role/id>", "content": "<utterance>"}`), paired with participant profile $p_u$, session briefing $b_u$, and episode definitions (absent participant $u^\star$, cue turn, anchor turn, target idea-unit proposition).
- **6-Fact `Percept` mapping:**
  - **Pre-meeting briefing percepts:** `actor = "system:briefing"` (or meeting organizer id), `place = "meeting:<ami_id>"`, `audience = [delegate_soul_id]`, `payload` carries participant profile $p_u$ and session briefing $b_u$.
  - **Streaming meeting turn percepts:** Each non-masked utterance in `meeting_transcripts` becomes a `Percept` with `actor = utterance.speaker` (e.g., `PM`, `ID`, `ME`, `UI`), `place = "meeting:<ami_id>"`, `audience = all_meeting_participants` (including `delegate_soul_id`), `payload = utterance.content`.
  - **`timestamp_ms` (synthetic pacing):** See caveat below — QMSum AMI JSON omits timestamps, so `timestamp_ms = t_0 + turn_idx * Δt_ms` (deterministic fixed turn spacing).
  - **`capability` & `receipt`:** Soul emits either silence or a `speak_in_meeting` `Action` proposing an utterance on behalf of $u^\star$; Lab logs the utterance and returns a synthetic `Receipt`.
- **`GoldBook` separation:** The absent participant $u^\star$'s masked target turns, target propositions/idea units, cue turns, anchor turns, and $k$-turn loose-recovery windows reside strictly in `GoldBook` and are never exposed to the soul.
- **Honesty / Missing-facts caveats:**
  - **Missing wall-clock timestamps:** Verified from QMSum JSON schema (`{"speaker": ..., "content": ...}` only, no timestamps). Synthesizing uniform turn-index `timestamp_ms` preserves causal order without inventing social facts, *provided* timing-sensitive features (inter-turn gap statistics) are understood to be uniform/synthetic unless raw AMI NXT XML `starttime`/`endtime` attributes are joined separately.
  - **Non-reactive replay:** Offline AMI transcripts cannot dynamically respond to the delegate's spoken turn, so masked target turns of $u^\star$ must remain withheld during the episode window to prevent gold leakage.

#### R-54 Sampling, Power (`S >= 354–471`), Stratification & Cost
- **Item counts:** **137 AMI meetings** (**97 train / 20 val / 20 test**) yielding **1,126 idea-unit episodes** total (~8.2 episodes/meeting on average). Exact episode count in the **20 test meetings** alone is **`unverified`** from search snippets (~164 episodes if uniform).
- **Power feasibility (`S >= 354–471`):**
  - **20 test meetings alone:** **No** (~164 estimated episodes $< 354$, clustered in 20 meetings).
  - **Full 137 meetings (1,126 episodes):** **Yes** in raw episode count ($1,126 \ge 471$), subject to **within-meeting clustering** (mean ~8.2 episodes/meeting) and the caveat that 97/137 meetings belong to QMSum's train split.
- **Stratification rule:** If subsampling $S = 400\text{--}471$ episodes from the 1,126-episode pool, sample **at the meeting cluster level** (or track meeting ID as cluster in paired variance) stratified by **QMSum split** (`train`/`val`/`test`), **absent role** (`PM`/`ID`/`ME`/`UI`), and **meeting episode-density quartile**, using a fixed deterministic seed.
- **Per-item LLM/judge cost shape:**
  - **Highest cost in this slice.** AMI meetings typically contain hundreds of turns (~400–800+ turns/meeting, ~8.2 episodes/meeting). Under causal turn-by-turn replay, if MonoAgens' zero-LLM gate wakes the soul on ~5–20% of turns, that is ~20–160 soul LLM calls per meeting (~2.5–20 calls per episode), plus **1–2+ schema-constrained `GPT-4o` judge calls (`Delegate Judge` / `Environment Judge`) per evaluated episode / spoken turn**.

---

### 2.4 `proactive_bench` — ProactiveBench (`thunlp/ProactiveAgent`, `arXiv:2410.12361`)

#### R-51 Provenance
- **Public existence & name collision warning:** Confirmed public, **with a critical name collision** that must be disambiguated per `researcher-prompt-3.md`:
  1. **Intended dataset — THUNLP ProactiveBench (`thunlp/ProactiveAgent`, `arXiv:2410.12361`, Lu et al., ICLR 2025):** Single-user activity-stream proactivity benchmark (Coding, Writing, Daily Life) used by `TGL Controller` (`arXiv:2605.30152`). Confirmed intended by `benchmark.rs`'s caveat `"Zero-LLM gate parity vs TGL"` and `R39_R48`.
  2. **Collision — ECCV 2026 MLLM ProactiveBench (`tdemin16/proactivebench` / HF `tdemin16/ProactiveBench`, MIT):** An unrelated multimodal VQA benchmark testing whether MLLMs ask clarifying questions on occluded/ambiguous images. Do **not** fetch `tdemin16/ProactiveBench`!
- **Canonical source (intended dataset):**
  - GitHub repository: `https://github.com/thunlp/ProactiveAgent`
  - Paper: `https://arxiv.org/abs/2410.12361` (Oct 2024; ICLR 2025)
  - Companion Hugging Face repo: `https://huggingface.co/YancyLee/ProactiveAgent` (**model checkpoints** for Proactive Agent & Reward Model — dataset files themselves live in `thunlp/ProactiveAgent/dataset/`).
- **Pinned revision & date:** Exact commit SHA / release tag on `thunlp/ProactiveAgent` is **`unverified`** from search snippets. Must be pinned on inspection/fetch.
- **Files needed & sizes:** Under `thunlp/ProactiveAgent/dataset/`:
  - `test_data/` — ProactiveBench test data (generated + manually collected traces with PII removed). Exact filenames inside `test_data/` and byte sizes are **`unverified`**.
  - `reward_data/` — `train_data.jsonl`, `test_data.jsonl`, `trainset_reward_llama.json`. Exact byte sizes are **`unverified`**.
  - `agent_data/` — agent training files (e.g., `agent_trainset.jsonl`).
- **Checksums:** None published at source (`checksums_recorded = "owner"` via `--record-checksums`).
- **License & terms:** **Apache License 2.0** (`SPDX: Apache-2.0` on `thunlp/ProactiveAgent`). Permits local copies, commercial use, modification, and redistribution of derived files with Apache-2.0 attribution/notice. No access gate.

#### R-52 Official Evaluation Protocol
- **Official metrics & definitions (`arXiv:2410.12361`):** Four event outcomes defined from ground-truth user need $N_t \in \{0, 1\}$ and agent proactive proposal $P_t$:
  - **Correct-Detection (CD):** $N_t = 1, P_t \neq \emptyset$ and accepted by user/RM (TP)
  - **Non-Response (NR):** $N_t = 0, P_t = \emptyset$ (TN)
  - **Missed-Need (MN):** $N_t = 1, P_t = \emptyset$ (FN)
  - **False-Detection (FD):** $N_t = 0, P_t \neq \emptyset$ (or rejected proposal, FP)
  - Official summary metrics: **Recall** $= \frac{\text{CD}}{\text{CD}+\text{MN}}$, **Precision** $= \frac{\text{CD}}{\text{CD}+\text{FD}}$, **Accuracy** $= \frac{\text{CD}+\text{NR}}{\text{CD}+\text{NR}+\text{MN}+\text{FD}}$, **F1** $= \frac{2 \cdot \text{Precision} \cdot \text{Recall}}{\text{Precision}+\text{Recall}}$, plus **False-Detection (`FD`)** agreement / rate on negative scenarios.
- **Registry metric alignment (`crates/lab/src/benchmark.rs`):**
  - `f1` — **Official** (matches ProactiveBench `F1`).
  - `false_detection` — **Ambiguity to pin on code inspection:** In `arXiv:2410.12361`, `False-Detection (FD)` denotes both the FP outcome category and (in RM validation / Run 2 table) **FD scenario agreement** (`0%` FD agreement for `GPT-4o` when it triggers on every non-need event, vs `100%` for the RM), whereas trigger papers (`TGL` `arXiv:2605.30152`) also evaluate **False-Detection Rate** ($\text{FPR} = \frac{\text{FD}}{\text{FD}+\text{NR}}$ or $\text{FDR} = 1 - \text{Precision}$). Lab must verify the exact formula in `thunlp/ProactiveAgent/eval/` (`reward_model_scoring.py`) and name the metric unambiguously (e.g., `false_detection_rate` vs `false_detection_count` vs `fd_agreement`).
- **Official split:** ProactiveBench test split (**233 test events** reported in `arXiv:2410.12361`; note some repo docs/issues reference **227** events in a subset/revision — exact line count inside `dataset/test_data/` must be verified on fetch). Reward-model validation split (`reward_data/test_data.jsonl`) has **120** labeled examples.
- **Official scoring script & revision:** `thunlp/ProactiveAgent/eval/` (including `reward_model_scoring.py`; revision **`unverified`**).
- **LLM judge required?**
  - **For full ProactiveBench content scoring:** **Yes** — official protocol uses the trained **Proactive Reward Model** (`YancyLee/ProactiveAgent` on Hugging Face, achieving **91.80% F1** and **100% FD agreement** against human annotator majorities) to judge whether an agent's proactive suggestion $P_t$ is helpful/accepted or rejected given context.
  - **For zero-LLM gate / TGL parity (`benchmark.rs` caveat):** Trigger timing (`wake` vs `silent` against binary ground-truth $N_t \in \{0,1\}$) can be scored **deterministically with 0 judge calls**; only evaluating the quality of a generated suggestion $P_t$ when triggered requires the Reward Model (or API judge).
- **Contamination:** Published Oct 2024 (`arXiv:2410.12361`, ICLR 2025) — **high contamination risk** in 2025–2026 foundation models.

#### R-53 Mapping to MonoAgens `Percept` & `GoldBook`
- **Raw format & units:** Activity Watcher + Environment Gym user activity streams across **Coding**, **Writing**, and **Daily Life** scenarios, containing `timestamp`, `event` logs (active window, VS Code/browser/keyboard/mouse actions), and aggregated `observation` state snapshots, paired with binary need $N_t$ and human accept/reject annotations.
- **6-Fact `Percept` mapping:**
  - `actor`: `"user:primary"` (or `"sensor:activity_watcher"` with `origin_actor = "user:primary"`).
  - `place`: `"place:<domain>:<scenario_id>"` (e.g., `place:coding:<id>`, `place:writing:<id>`, `place:daily_life:<id>`).
  - `audience`: `[soul_actor_id, "user:primary"]`.
  - `timestamp_ms`: Parsed directly from event stream `timestamp` (with deterministic monotonic fallback if any synthetic Gym event omits wall-clock ms).
  - `payload`: `OpaquePayload` holding the serialized `observation` / `event` trace text.
  - `capability` & `receipt`: Soul either remains silent (zero `Action`) or invokes a `propose_assistance` `Action` containing its proactive task suggestion; Lab logs the action and returns a synthetic `Receipt`.
- **`GoldBook` separation:** Ground-truth user need $N_t$, human accept/reject labels, and reference task descriptions stay strictly in `GoldBook`.
- **Honesty / Missing-facts caveats:** **Cleanly mappable** for single-user event streams (`timestamp`, `actor`, `place`, `payload` are naturally present in Activity Watcher traces). Caveat: single-user environment with no multi-party speaker turns; exact JSON field keys inside `dataset/test_data/` are **`unverified`** until fetch.

#### R-54 Sampling, Power (`S >= 354–471`), Stratification & Cost
- **Item counts:**
  - **Test split (`ProactiveBench` test):** **233 test events** (or **227** in an early repo subset — verify exact count on fetch) across scenarios; **`reward_data/test_data.jsonl`:** **120** labeled examples.
  - **Train split:** **6,790 training events** (Coding 2,275; Writing 2,354; Daily Life 2,161) across **136 scenarios** total; **`reward_data/train_data.jsonl`:** **1,640** labeled examples.
- **Power feasibility (`S >= 354–471`):**
  - **Official test split alone (`233` events):** **No** — $233 < 354$ (and events within the same scenario are clustered). Must be reported as an underpowered official-split run with exact binomial/cluster confidence intervals, **unless** the owner authorizes drawing a non-official extended external evaluation sample ($S \ge 400\text{--}471$ events) from the unused `6,790` training events (permitted under MonoAgens' rule that external sets never tune priors, though non-standard for literature comparison).
- **Stratification rule:** Run **100% of the official test split (233 events)** without subsampling. If an extended $S \ge 400\text{--}471$ subset is drawn from the full pool, stratify proportionally by domain (**Coding 33.5% / Writing 34.7% / Daily Life 31.8%**) and **need label ($N_t = 1$ vs $0$)**, sampling by **scenario cluster** with a fixed deterministic seed.
- **Per-item LLM/judge cost shape:**
  - **Trigger-only (`TGL` parity):** 1 zero-LLM gate evaluation per event (0 soul LLM calls when silent; 1 soul LLM call only when gate wakes), **0 judge calls** ($0$–$233$ soul LLM calls total on test split).
  - **Full ProactiveBench scoring:** Up to 1 soul LLM call per triggered event + **1 local Reward Model inference** (`YancyLee/ProactiveAgent`, ~7B/8B) or **1 API LLM-judge call** per triggered event ($P_t \neq \emptyset$). Very budget-friendly (~$0\text{--}466$ calls total across the whole test split).

---

## 3. Consolidated Summary Tables (`R-51`–`R-54`)

### Table 1: `R-51` Provenance, Files, Checksums & Licensing

| `dataset_id` | Public? & Canonical Name | Canonical Source & Pinned Revision | Needed Files & Sizes | Source Checksums? | License (SPDX) & Terms (Local / Commercial / Derived) | Access Gate? |
|---|---|---|---|---|---|---|
| `pm_bench` | **Yes** — **PM-Bench** (`arXiv:2607.12385`, COLM 2026) | `https://github.com/genglinliu/PMBench` · Revision: **`unverified`** (July 2026 paper) | `data/`, `eval/`, `README.md`, `requirements.txt` (exact file names & byte sizes **`unverified`**) | No (`owner` via `--record-checksums`) | **None in repo (`LICENSE` 404)** (Paper: CC BY 4.0 per Run 2). Default copyright on repo: local/commercial/derived **unverified / requires author permission** | None observed |
| `pis` | **No (`unavailable`)** — **PIS** (`arXiv:2609.01272`, Sept 2026) is a **method** evaluated on `pm_bench` | Paper only (`https://arxiv.org/abs/2609.01272`); no independent dataset repo exists | N/A (`unavailable`) | N/A | N/A (Method evaluated on `pm_bench`) | N/A |
| `capa` | **Yes** — **CAPA / Meeting Delegation (AMI via QMSum)** (`arXiv:2609.03923`, EMNLP 2026) | `https://github.com/FKIRSTE/emnlp2026-meeting-delegation` · Revision: **`unverified`** (Sept 2026) | `data/ami/` (137 AMI JSON transcripts), `capa/`, `prompts/` (exact file names & sizes **`unverified`**) | No (`owner` via `--record-checksums`) | **Dual:** Code = **`MIT`**; AMI data = **`CC-BY-4.0`**. Local copies, commercial use, derived redistribution allowed w/ attribution (offline research caveat) | None observed |
| `proactive_bench` | **Yes** *(name collision w/ `tdemin16/proactivebench`)* — **ProactiveBench** (`arXiv:2410.12361`, ICLR 2025) | `https://github.com/thunlp/ProactiveAgent` (RM weights at `huggingface.co/YancyLee/ProactiveAgent`) · Revision: **`unverified`** (Oct 2024) | `dataset/test_data/`, `dataset/reward_data/{train_data.jsonl, test_data.jsonl, trainset_reward_llama.json}` (sizes **`unverified`**) | No (`owner` via `--record-checksums`) | **`Apache-2.0`**. Local copies, commercial use, and derived redistribution permitted w/ Apache-2.0 notice | None observed |

---

### Table 2: `R-52` Official Evaluation Protocol & Registry Alignment

| `dataset_id` | Registry Metrics vs Official Metrics | Official Metric Definitions | Reporting Split | Official Scoring Script & Revision | LLM Judge Required? (Model & Prompt) | Known Contamination |
|---|---|---|---|---|---|---|
| `pm_bench` | `set_f1` = **Official**; `commission_error_rate` = **Not official** (closest official: `False-Alarm (FA)` / `FP count` / `1 - Precision`) | `Set-F1 = 2TP / (2TP + FP + FN)` over due set $D_t$ vs selected `task_ids` $C_t$ across 80 steps; `FA = FP / N_steps` | Full 7-day Virtual Week (80 steps, no train/test split) | `genglinliu/PMBench` (`eval/`, exact filename & rev **`unverified`**); deterministic set match | **No** (deterministic JSON `task_ids` set match) | Low–Moderate (published July 2026; Virtual Week paradigm is classic) |
| `pis` | `set_f1` = **Official** on `pm_bench` | Evaluated on `pm_bench` (`Set-F1`); no independent dataset protocol | N/A (`unavailable`) | Defers to `pm_bench` | Defers to `pm_bench` | N/A |
| `capa` | `credited_recovery` = **Official**; `silence_rate` = **Official** | `credited_recovery` (loose recall): target idea-unit surfaced within $k$-turn window w/o hallucination/redundancy; `silence_rate`: `SILENT` on valid $u^\star$ opportunities | 137 AMI meetings (97 train / 20 val / 20 test; paper reports 137 meetings / 1,126 episodes & 20-meeting subset) | `capa/judge.py`, `capa/delegate_judge.py` in `FKIRSTE/emnlp2026-meeting-delegation` (rev **`unverified`**) | **Yes** — schema-constrained LLM judge (`GPT-4o` in paper, $\kappa=0.71$ vs human); exact prompts **`unverified`** | **High** for raw AMI/QMSum transcripts; Low for Sept 2026 CAPA episode cues/briefings |
| `proactive_bench` | `f1` = **Official**; `false_detection` = **Ambiguous** (paper uses `FD` count & RM `FD` agreement; trigger parity uses `FPR` / `FDR`) | `CD` (TP), `NR` (TN), `MN` (FN), `FD` (FP); `Recall = CD/(CD+MN)`, `Precision = CD/(CD+FD)`, `F1 = 2PR/(P+R)`, `Acc = (CD+NR)/All` | `test_data` (**233** test events; verify **227** vs **233** on fetch); RM test = 120 | `thunlp/ProactiveAgent` `eval/` (`reward_model_scoring.py`, rev **`unverified`**) | **Yes for content** (`YancyLee/ProactiveAgent` RM, 91.80% F1, or LLM judge); **No** for binary trigger-only (`TGL`) parity | **High** (public GitHub release since Oct 2024 / ICLR 2025) |

---

### Table 3: `R-53` Mapping to MonoAgens 6-Fact `Percept` & `GoldBook` Separation

| `dataset_id` | Raw Units & Fields | Mapping to 6-Fact `Percept` (`actor`, `place`, `audience`, `timestamp_ms`, `payload`, `capability`/`receipt`) | `GoldBook` Held-Out Data | Mappability & Missing-Facts Caveats |
|---|---|---|---|---|
| `pm_bench` | 80 Virtual Week steps; step narrative, day/time, ongoing choices, anonymous `task_ids` menu, 11 channels, 11 updates | `actor="env:virtual_week"`, `place="place:virtual_week_sim"`, `audience=[soul, env]`, `timestamp_ms`=simulated `(day, time)`, `payload`=narrative+menus+`task_ids`, `Action`=`choose`/`query_state`/`check_time` + `Receipt` | Due task sets $D_t$ per step $t$, task taxonomy, update targets, lure flags | **Mappable** (no fabricated facts). Interactive multi-step driver needed for `query_state`/`check_time`; synthetic single-user week lacks multi-party noise |
| `pis` | N/A (method on `pm_bench`) | Defers to `pm_bench` | Defers to `pm_bench` | **`unavailable`** as dataset |
| `capa` | QMSum AMI JSON: `meeting_transcripts` (`{speaker, content}`), profile $p_u$, briefing $b_u$, episodes (cue/anchor/$u^\star$ proposition) | Briefing `Percept` (`actor="system:briefing"`) + per-turn `Percept` (`actor=speaker`, `place="meeting:<ami_id>"`, `audience=participants`, `timestamp_ms`=`t_0 + i*Δt`, `payload=content`); `Action`=`speak_in_meeting` + `Receipt` | Masked $u^\star$ turns, target propositions, cue/anchor bounds, $k$-turn recovery window | **Caveat: QMSum JSON lacks per-turn timestamps** (must synthesize deterministic turn-index `timestamp_ms` or join raw AMI NXT XML); offline transcripts are non-reactive |
| `proactive_bench` | Activity Watcher + Gym traces (`timestamp`, `event`, `observation`) across Coding, Writing, Daily Life | `actor="user:primary"` (or `"sensor:activity_watcher"`), `place="place:<domain>:<scenario>"`, `audience=[soul, user]`, `timestamp_ms`=event `timestamp`, `payload`=`observation`+`event`, `Action`=`propose_assistance` + `Receipt` | Binary need $N_t$, human accept/reject labels, reference task descriptions | **Mappable** (events carry timestamps & user actor; single-user stream without multi-party speakers; exact JSON keys **`unverified`** until fetch) |

---

### Table 4: `R-54` Split Sizes, Paired Power ($S \ge 354\text{--}471$), Stratification & Cost Shape

| `dataset_id` | Item Counts per Relevant Split | Reaches $S \ge 354\text{--}471$ Paired Power? | Stratification Rule | Rough Per-Item Call Cost Shape |
|---|---|---|---|---|
| `pm_bench` | **1** 7-day trajectory = **80 steps**, **83 task defs** (**81 scored**), **74 lures**, **11 updates** | **No** ($80 \ll 354$, serially dependent within 1 week; diagnostic run only) | **Run 100% (no subsampling).** Stratify reporting by task type (event/time, regular/non-regular, channel, cross-day, update, lure) | **~80–160 soul LLM calls total** for full benchmark (80 steps + state/time queries); **0 judge calls** |
| `pis` | **0** independent items (`unavailable`; uses `pm_bench`'s 80 steps) | **No** (defers to `pm_bench`) | N/A | Defers to `pm_bench` |
| `capa` | **137 AMI meetings** (**97 train / 20 val / 20 test**) = **1,126 episodes** total (~8.2/meeting; ~164 est. in 20 test) | **20 test:** **No** (~164 est. $< 354$). **Full 137 meetings:** **Yes** raw ($1,126 \ge 471$), but clustered within 137 meetings & 97 are QMSum train | If sampling $S=400\text{--}471$ from 1,126: cluster-sample by **meeting** stratified by **split**, **absent role** (`PM`/`ID`/`ME`/`UI`), and **episode-count quartile** | **High:** ~20–160 soul LLM calls/meeting (~2.5–20/episode if gate wakes on 5–20% of ~400–800 turns) + **1–2+ `GPT-4o` judge calls per episode/utterance** |
| `proactive_bench` | **Test:** **233 events** (verify 227 vs 233 on fetch); **RM test:** 120. **Train:** **6,790 events** (Coding 2,275; Writing 2,354; Daily 2,161; 136 scenarios total); **RM train:** 1,640 | **Official test (`233`):** **No** ($233 < 354$, clustered by scenario). **Extended train-pool sample:** **Yes** if owner authorizes $S \ge 400\text{--}471$ | Run **100% of test (233)**. If extending from pool: stratify by **domain** (Coding/Writing/Daily ~1/3 each) & **$N_t \in \{0,1\}$**, clustered by **scenario** | **Gate-only (`TGL` parity):** 0–1 soul call/event (only when gate wakes), **0 judge calls**. **Full:** 1 soul call + **1 local RM (or API judge) call per triggered event** |

---

## Primary Sources Cited

- **PM-Bench:** G. Liu & S. Gabriel (COLM 2026), *PM-Bench: Evaluating Prospective Memory in LLM Agents*, `https://arxiv.org/abs/2607.12385` · Repository: `https://github.com/genglinliu/PMBench`
- **PIS:** J. Zhao & C. Wu (Sept 2026), *Making Prospective Memory SLM-Shaped: Typed Intention Stores for Small-Model Agents*, `https://arxiv.org/abs/2609.01272`
- **CAPA:** M. Khan, F. Kirstein, T. Ruas, & B. Gipp (EMNLP 2026), *Speak for Me: Giving LLMs the Situational Awareness to Participate in a Meeting*, `https://arxiv.org/abs/2609.03923` · Repository: `https://github.com/FKIRSTE/emnlp2026-meeting-delegation` (with AMI/QMSum transcript lineage: Carletta et al. 2005; Zhong et al. 2021)
- **ProactiveBench (intended):** Y. Lu, S. Yang, C. Qian, et al. (ICLR 2025), *Proactive Agent: Shifting LLM Agents from Reactive Responses to Active Assistance*, `https://arxiv.org/abs/2410.12361` · Repository: `https://github.com/thunlp/ProactiveAgent` · Checkpoints: `https://huggingface.co/YancyLee/ProactiveAgent` · TGL parity reference: X. Liu, R. Zhang, et al. (2026), *Do Proactive Agents Really Need an LLM to Decide When to Wake and What to Anchor?*, `https://arxiv.org/abs/2605.30152`
- **ProactiveBench (name collision, NOT intended):** `https://github.com/tdemin16/proactivebench` / `https://huggingface.co/datasets/tdemin16/ProactiveBench` (ECCV 2026 MLLM benchmark, MIT)
