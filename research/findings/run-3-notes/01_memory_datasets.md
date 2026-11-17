# MonoAgens Run 3 Research Note — Domain 1: Memory Datasets (`R-51`–`R-54`)

- **Scope:** Five registered external memory datasets in [`standard_benchmark_registry()`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L74-L116): `locomo`, `longmemeval`, `memory_agent_bench`, `group_mem_bench`, and `stale`.
- **Verification Standard:** All URLs, file names, item counts, evaluation scripts, licenses, and schemas below were checked via web search against primary sources (dataset repositories, Hugging Face dataset cards, and arXiv papers). Where an exact commit SHA, exact byte size, SHA-256 checksum, or verbatim prompt/key could not be directly observed from search results, it is explicitly tagged `[unverified from search; resolve via repo/HF API before fetch]`.
- **No Owner Decisions Made:** License clearance, split selection, judge model selection, metric naming/additions, and mapping choices are presented as options for the owner.

---

## 1. Executive Summary & Owner Choices Needed

All five Memory datasets exist publicly under their registered identities. None of the five are blocked as `unavailable`, though `memory_agent_bench` requires an owner ruling on synthetic chunk timestamps before final `mappable` vs. `not_mappable` status, and **four** of the five carry important alignment gaps between `crates/lab/src/benchmark.rs` and upstream official protocols.

### Critical Findings & Owner Choices

1. **License Clearance Required Before Fetch (`benchmark-datasets` spec):**
   - `locomo`: **`CC-BY-NC-4.0`** (non-commercial restriction; forbids commercial use).
   - `longmemeval`: **`MIT`**.
   - `memory_agent_bench`: **`MIT`** (aggregator repo/HF card; wraps third-party corpora such as LongMemEval [MIT], ∞Bench, RULER, EventQA, DetectiveQA, Movie-Rec whose upstream terms should be checked if redistributing).
   - `group_mem_bench`: **`CC-BY-4.0`** (HF conversation dataset) + **`MIT`** (GitHub code/evaluation repo; verify `questions/` directory terms in root `LICENSE`).
   - `stale`: **`CC-BY-4.0`** (STALE dataset) + **`MIT`** (LongMemEval distractor sessions).
   - *Owner choice:* Explicitly set `license.status = "cleared"` or `"refused"` in `lab/datasets.manifest.toml` (especially for `locomo` `CC-BY-NC-4.0`).

2. **Metric & Protocol Alignment Gaps (`R-52` vs. [`benchmark.rs`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L76-L116)):**
   - **`locomo` (`qa_f1`, `cat5_adversarial_acc`):** Upstream `snap-research/locomo` `task_eval/evaluation.py` computes deterministic Porter-stemmed token F1 on Categories 1–4 (`qa_f1`) and a deterministic substring refusal check on Category 5 (`cat5_adversarial_acc`), with **no LLM judge**. By contrast, the Mem0 literature (`arXiv:2504.19413`) uses a `gpt-4o-mini` judge (`J`) and **drops Category 5** (violating MonoAgens' registry caveat). *Owner choice:* Keep official `evaluation.py` deterministic scoring (0 judge cost, exact parity) vs. add a separate renamed LLM-judge metric (e.g. `llm_judge_acc` / `cat5_adversarial_judge_acc`) alongside it.
   - **`longmemeval` (`acc`, `abstention_f1`):** Official `print_qa_metrics.py` reports `overall_acc`, `task_averaged_acc`, and per-category accuracy including **`abstention_acc`** on the 30 `_abs` questions—it does **not** define or compute `abstention_f1`. *Owner choice:* Rename `abstention_f1` in `benchmark.rs` to official `abstention_acc` (or keep `abstention_f1` as a MonoAgens-defined F1 over answerable vs. unanswerable decisions). Also choose primary reporting split: `longmemeval_s_cleaned.json` (standard ~115K tok, ~277 MB), `longmemeval_m_cleaned.json` (stress ~1.5M tok, ~2.74 GB), and/or `longmemeval_oracle.json` (~2K tok, ~15.4 MB).
   - **`memory_agent_bench` (`ar_acc`, `selective_forgetting_acc`):** Upstream HF split for Selective Forgetting is named **`Conflict_Resolution`** (`FactConsolidation-SH` & `FactConsolidation-MH`). `AR` combines 4 sub-tasks, one of which (`LongMemEval-S*`, 300 QAs) uses a `gpt-4o` judge in the official repo while the others use `SubEM`/`EM`. *Owner choice:* Score full `AR` (requires `gpt-4o` judge on `LME(S*)` + `SubEM`/`EM` on others) vs. deterministic `SubEM`/`EM` sub-tasks only (renamed metric), and decide whether to also include `Test_Time_Learning` and `Long_Range_Understanding` as supplementary.
   - **`group_mem_bench` (`acc`, `knowledge_update`, `term_ambiguity`):** Matches official 6-category GPT-5 judge accuracy protocol cleanly. *Owner choice:* Accept `gpt-5` as official judge (or record an alternate judge under a renamed metric).
   - **`stale` (`state_resolution`, `premise_resistance`):** Official benchmark probes **3** equal dimensions across 400 scenarios (1,200 queries): `State Resolution` (`SR`), `Premise Resistance` (`PR`), and **`Implicit Policy Adaptation` (`IPA`)**. `benchmark.rs` omits `IPA`. *Owner choice:* Add `implicit_policy_adaptation` to `benchmark.rs` or score it as a supplementary metric.

3. **Timestamp & Speaker Honesty (`R-53`):**
   - `group_mem_bench` natively provides ISO 8601 per-message timestamps, speaker IDs, `reply_to` parent links, and multi-channel places—highest-fidelity 6-fact `Percept` fit.
   - `locomo`, `longmemeval`, and `stale` have **session-level** timestamps (`date_time` / `haystack_dates`) without per-turn milliseconds; deterministic monotonic intra-session offsets (e.g., `+1000 ms/turn`) are required and do not fabricate calendar ordering.
   - `memory_agent_bench` chunks (`RULER`, `EventQA`, `FactConsolidation`, `InfBench`, `DetectiveQA`) **lack wall-clock timestamps and multi-party speakers** and use an artificial `"Please memorize it..."` prompt wrapper. *Owner choice:* Accept deterministic monotonic stream indices (`T_0 + i * Δt`) from `ActorId("stream_source")` as an honest stream-order representation, or mark non-conversational sub-tasks `not_mappable`.
   - `longmemeval` and `stale` histories contain **pre-written `assistant` turns**. *Owner choice:* Ingest pre-written `assistant` turns as observed `Percept`s from `ActorId("prior_assistant")` (zero fabricated Core receipts) vs. replay through soul as prior acts.

4. **Statistical Power & Clustering (`R-54`, `A-16` `S >= 354–471`):**
   - **Independent-scenario power (`DE = 1.0` at `R=1`):** `longmemeval` (`500` independent items >= `471`) and `stale` (`400` independent scenarios >= `354` for `R=2`, slightly short of `471` at `R=1`) support paired NI testing cleanly.
   - **Clustered datasets (cannot reach `354` independent conversations/contexts):** `locomo` has `1,986` QAs clustered inside only **10 conversations**; `memory_agent_bench` has `2,071` QAs clustered inside **146 rows** (only **22** rows in `AR` and **8** rows in `CR`); `group_mem_bench` has `745` QAs clustered inside **4 domains / 33 channels**. All three MUST report **cluster-bootstrap CIs** (clustered by `conversation_id`, `row_id`, or `channel_id`), and `locomo` / `memory_agent_bench` cannot standalone-certify `S >= 354` scenario-level NI.

---

## 2. Detailed Per-Dataset Findings

---

### 2.1 `locomo`

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **LoCoMo** (*Evaluating Very Long-Term Conversational Memory of LLM Agents*, Maharana et al., ACL 2024 / `arXiv:2402.17753`).
- **Canonical source & revision:**
  - Repository: `https://github.com/snap-research/locomo`
  - Paper: `https://arxiv.org/abs/2402.17753`
  - Pinned revision & date: `[unverified from search; resolve via repo/HF API before fetch]` (repository uses default branch `main`; paper posted February 2024).
  - *Important scope note:* Although `arXiv:2402.17753` evaluates 50 conversations (`7,512` QAs), **only 10 conversations (`locomo10.json`) are publicly released** in `snap-research/locomo`.
- **Files needed & sizes:**
  - `data/locomo10.json` — observed size `~2.7–2.8 MB` (`[exact byte count unverified from search; resolve via repo/HF API before fetch]`).
  - Reference scorer (optional if calling Python parity fixture): `task_eval/evaluation.py` in `snap-research/locomo`.
- **Checksums:** Source publishes **no** SHA-256 checksums (`checksums_recorded = "owner"` via `monoagens-lab datasets fetch --record-checksums`).
- **License & access gate:**
  - SPDX ID: **`CC-BY-NC-4.0`** (Creative Commons Attribution-NonCommercial 4.0 International, per `LICENSE` in `snap-research/locomo` covering code and dataset).
  - Terms: Permits local copies, sharing, and adaptation with attribution; **forbids commercial use** (requires owner legal clearance).
  - Access gate: None (public GitHub download).

#### R-52 Official Evaluation Protocol
- **Registry metrics (`benchmark.rs`):** `qa_f1`, `cat5_adversarial_acc`.
- **Official definitions & alignment:**
  - `task_eval/evaluation.py` (`eval_question_answering`) in `snap-research/locomo` scores QA items by `category` integer in `locomo10.json`:
    - **Categories 1–4 (`qa_f1`):** Normalized token-level F1 (`f1_score`) with Porter stemming; Category 1 (multi-hop) splits ground-truth/predicted sub-answers on commas for partial F1, whereas Categories 2–4 compute standard normalized token F1 and exact match.
    - **Category 5 (`cat5_adversarial_acc`):** Binary refusal accuracy (`all_ems.append(1)` if `'no information available'` or `'not mentioned'` appears in `output.lower()`, else `0`). Matches MonoAgens' `cat5_adversarial_acc` concept, though `evaluation.py` aggregates it as a per-category accuracy rather than that exact key name.
    - *Integer category mapping caveat:* Community audits of `snap-research/locomo` note a numbering mismatch between the paper prose and the integer `category` tags in `locomo10.json` / `evaluation.py` (`1` = Multi-hop [282], `2` = Temporal [321], `3` = Open-domain [96], `4` = Single-hop [841], `5` = Adversarial [446]). Scorers MUST key off `locomo10.json` / `evaluation.py` integer conventions directly.
  - **Literature divergence (Mem0 `J` vs. Official `evaluation.py`):** Mem0 (`arXiv:2504.19413`) and several downstream harnesses replace F1 with a `gpt-4o-mini` LLM-as-a-judge binary accuracy `J` and **exclude Category 5** entirely. That conflicts with `benchmark.rs`'s explicit caveat (`"Must retain Category-5 adversarial questions"`). If an LLM judge is additionally run for comparability, it must be reported under a separate metric name (e.g., `qa_judge_acc`, `cat5_adversarial_judge_acc`).
- **Reporting split:** All 10 conversations in `data/locomo10.json` (1,986 QA pairs).
- **Official scoring script & revision:** `snap-research/locomo/task_eval/evaluation.py` (`[commit SHA unverified from search; resolve via repo/HF API before fetch]`). Deterministic Rust reimplementation with a Python parity fixture is straightforward.
- **LLM judge required?** **No** for official `task_eval/evaluation.py` (`qa_f1` + `cat5_adversarial_acc` are 100% deterministic). *Optional choice:* Mem0-style `gpt-4o-mini` judge (non-official). Note that the strict substring rule (`'no information available' in output.lower() or 'not mentioned' in output.lower()`) requires the evaluation prompt/post-processor to instruct the model on exact refusal phrasing—otherwise valid refusals like `"I don't know"` score `0` under `evaluation.py`!
- **Known contamination:** High exposure risk in frontier models trained after Feb 2024, as `locomo10.json` is open on GitHub and widely cited across memory papers.

#### R-53 Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`
- **Raw JSON structure (`locomo10.json`):** Array of 10 conversation objects:
  - `conversation`: `{ speaker_a: str, speaker_b: str, session_1: [{speaker, dia_id, text, img_url?, blip_caption?}], session_1_date_time: str, ... }`
  - Pre-extracted auxiliary fields: `observation`, `session_summary`, `event_summary`
  - `qa`: `[{ question: str, answer?: str, adversarial_answer?: str, category: int, evidence: [str] }]`
- **`GoldBook` separation:**
  - `qa[].answer`, `qa[].adversarial_answer`, `qa[].category`, `qa[].evidence`, and **all** pre-computed `observation` / `session_summary` / `event_summary` fields MUST be withheld in `GoldBook` / discarded prior to ingestion so the soul builds its own memory solely from raw dialogue percepts.
- **6-fact `Percept` mapping:**
  - `actor`: `ActorId(turn.speaker)` (`speaker_a` or `speaker_b`). *Owner choice on soul role:* (a) soul observes `speaker_a` and `speaker_b` as a third-party assistant in the place, then answers an `ActorId("evaluator")` probe; or (b) soul assumes `speaker_b` (requires deciding how pre-written `speaker_b` turns are ingested). Option (a) avoids inventing soul internal actions for historical turns.
  - `place`: `PlaceId("locomo_conv_<idx>")`.
  - `audience`: `[ActorId(speaker_a), ActorId(speaker_b), ActorId("soul")]` (under observer mode).
  - `timestamp_ms`: Parse `session_k_date_time` into epoch ms + deterministic synthetic turn offset `turn_idx * 1000` ms. (**Caveat:** LoCoMo has **session-level timestamps only**, not per-turn timestamps; monotonic `1s` offsets preserve order without fabricating clock claims.)
  - `payload`: `turn.text` (plus optional `turn.blip_caption` text if owner enables multimodal captions; `img_url` must NOT be fetched at runtime due to offline constraint).
  - **Question probe:** After all sessions of a conversation are ingested, emit each `qa[j].question` as a `Percept` from `ActorId("evaluator")` at `t_last_session + (j + 1) * 60_000` ms **on an isolated post-ingestion checkpoint** (so Q&A turns do not leak into subsequent questions' context). Soul responds via `Action` (text reply capability + receipt).
- **Missing facts / caveats:** Missing per-turn timestamps (session-level only); strictly dyadic (2 human speakers); Category-5 refusal phrasing sensitivity under `evaluation.py`.

#### R-54 Sampling & Power
- **Item counts (`locomo10.json`):** **10 conversations**, **1,986 QA pairs** total:
  - Cat 1 (Multi-hop): `282`
  - Cat 2 (Temporal): `321`
  - Cat 3 (Open-domain): `96`
  - Cat 4 (Single-hop): `841`
  - Cat 5 (Adversarial): `446` (`22.5%` of total)
- **`S >= 354–471` paired power feasibility & clustering:**
  - At the QA level (`1,986` QAs), `N > 471`. However, QAs are densely clustered within only **10 conversations** (~198.6 QAs/conversation). Under Donner–Birkett–Buck clustering (`DE = 1 + (m - 1) * rho`), even modest intra-conversation correlation (`rho = 0.05`) inflates variance by `DE ≈ 10.9`, leaving effective sample size `N_eff ≈ 182 < 354`.
  - Therefore, **`locomo` alone cannot achieve `S >= 354` independent scenario-level power** for `A-16` NI certification. Paired CIs MUST be computed via **conversation-clustered bootstrap** (or treat QA-level variance with explicit clustering warning).
- **Stratification rule:**
  - Prefer **full 1,986 QAs** (since ingesting the 10 conversations happens only once per arm).
  - If subsampling for a budget-capped pilot (`S_qa = 471` or `500`), use **proportional stratified sampling across `10 conversations × 5 categories` (`50` strata)** with deterministic seeded PRNG, strictly preserving Category 5 at its full `22.5%` weight (`~106` QAs at `S_qa = 471`).
- **Per-item call cost shape:**
  - Ingestion: `~3,000` turns total across 10 conversations (`~9K–26K` tokens/conv). Amortized once per conversation per arm if checkpoint-branched before QA!
  - Evaluation: `1` soul LLM call per QA (`1,986` calls full; `471` subsampled).
  - Judge: **`0` judge LLM calls** under official `evaluation.py` (`+1` call/QA if optional LLM judge `J` is added).

---

### 2.2 `longmemeval`

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **LongMemEval** (*LongMemEval: Benchmarking Chat Assistants on Long-Term Interactive Memory*, Wu et al., ICLR 2025 / `arXiv:2410.10813v2`).
- **Canonical source & revision:**
  - Code & scripts: `https://github.com/xiaowu0162/LongMemEval`
  - Canonical cleaned dataset (replaces deprecated `xiaowu0162/longmemeval`): `https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned`
  - Paper: `https://arxiv.org/abs/2410.10813`
  - Pinned revision & date: `[unverified from search; resolve via repo/HF API before fetch]` (branch `main`; ICLR 2025, v2 paper).
- **Files needed & sizes (Hugging Face `xiaowu0162/longmemeval-cleaned`):**
  - `longmemeval_oracle.json`: **`15,388,478` bytes** (~15.4 MB)
  - `longmemeval_s_cleaned.json`: **`277,383,467` bytes** (~277.38 MB) — standard primary evaluation split (~40–50 sessions, ~115K tokens/item)
  - `longmemeval_m_cleaned.json`: `~2.74 GB` (`[exact byte count unverified from search; resolve via repo/HF API before fetch]`) — 500-session stress split (~1.5M tokens/item)
- **Checksums (observed from Hugging Face `xiaowu0162/longmemeval-cleaned`):**
  - `longmemeval_oracle.json` SHA-256: `821a2034d219ab45846873dd14c14f12cfe7776e73527a483f9dac095d38620c`
  - `longmemeval_s_cleaned.json` SHA-256: `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`
  - `longmemeval_m_cleaned.json` SHA-256: `9d79e5524794a2e6900a3aa9cb7d9152c5a3e8319c9a87c25494ba1eacee495f`
- **License & access gate:**
  - SPDX ID: **`MIT`** (`xiaowu0162/LongMemEval` and `xiaowu0162/longmemeval-cleaned`).
  - Terms: Permits local copies, commercial use, modification, and redistribution with MIT copyright/license notice.
  - Access gate: None.

#### R-52 Official Evaluation Protocol
- **Registry metrics (`benchmark.rs`):** `acc`, `abstention_f1`.
- **Official definitions & alignment:**
  - `src/evaluation/print_qa_metrics.py` reports **overall accuracy (`overall_acc`)**, **task-averaged accuracy (`task_averaged_acc`)**, and per-category accuracy including **`abstention_acc`** on the 30 `_abs` questions.
  - **Metric alignment gap:** **`abstention_f1` is NOT an official LongMemEval metric.** The official script computes **`abstention_acc`** (fraction of `_abs` items where the LLM judge answers `"yes"`). Per `benchmark-evaluation` spec, either:
    - *Option A:* Update `benchmark.rs` metric name from `abstention_f1` to official `abstention_acc` (and specify whether `acc` means `overall_acc` or `task_averaged_acc`).
    - *Option B:* Keep `abstention_f1` as a MonoAgens-defined F1 over predicted-abstention vs. ground-truth `_abs` across all 500 items, and additionally log official `abstention_acc` and `task_averaged_acc`.
- **Reporting split:** Standard primary split is **`longmemeval_s_cleaned.json`** (`LongMemEval_S`); `longmemeval_m_cleaned.json` (`LongMemEval_M`) is the long-context stress split; `longmemeval_oracle.json` is the retrieval-ceiling reference. (*Owner choice:* which split(s) to require in standard runs.)
- **Official scoring script & revision:** `src/evaluation/evaluate_qa.py` and `src/evaluation/print_qa_metrics.py` in `xiaowu0162/LongMemEval` (`[commit SHA unverified from search; resolve via repo/HF API before fetch]`).
- **LLM judge required?** **Yes.** Official `evaluate_qa.py` defaults to **`gpt-4o`** (specifically `gpt-4o-2024-08-06`, `temperature=0`), using **task-specific prompt templates** keyed by `question_type` and `_abs` suffix:
  - Abstention (`_abs`) prompt (verbatim from `evaluate_qa.py`):
    > `I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.`
    > `Question: {question}` / `Explanation: {answer}` / `Model Response: {response}`
    > `Does the model correctly identify the question as unanswerable? Answer yes or no only.`
  - `temporal-reasoning` prompt (allows minor off-by-one day/week/month boundary tolerance), `knowledge-update` prompt (requires current/latest state), `single-session-preference` prompt (rubric-based adherence), and standard QA prompt for `single-session-user` / `single-session-assistant` / `multi-session` (`[exact verbatim text of the 4 non-abstention templates unverified from search; inspect evaluate_qa.py before Rust/Python scorer integration]`).
- **Known contamination:** Published Oct 2024 and heavily used in 2025–2026 memory papers; newer frontier models risk parametric memorization (which motivated follow-up work such as LongMemEval-V2), though long-haystack retrieval in `S` (~115K tok) and `M` (~1.5M tok) still heavily tests memory selection.

#### R-53 Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`
- **Raw JSON schema (`longmemeval_*_cleaned.json`):** Array of 500 objects:
  - `question_id`: string (ends with `_abs` for the 30 abstention items)
  - `question_type`: `"single-session-user" | "single-session-assistant" | "single-session-preference" | "temporal-reasoning" | "knowledge-update" | "multi-session"`
  - `question`: string
  - `question_date`: timestamp string
  - `answer`: string (or explanation text for `_abs`)
  - `haystack_dates`: `[str]` (one timestamp per session)
  - `haystack_sessions`: `[[{ role: "user" | "assistant", content: str, has_answer?: bool }]]`
- **`GoldBook` separation:**
  - `answer`, `question_type`, `has_answer`, `answer_session_ids` (if present), and the original `question_id` (because of the `_abs` suffix!) MUST be kept exclusively in `GoldBook`. Use an opaque numeric index (`lme_000`..`lme_499`) for `PlaceId` / `CorrelationId`.
- **6-fact `Percept` mapping:**
  - `actor`: `"user"` -> `ActorId("user")`; `"assistant"` -> `ActorId("prior_assistant")` (or `ActorId("soul")` — *owner choice*: using `prior_assistant` treats past assistant turns honestly as observed prior conversation turns without fabricating Core action receipts).
  - `place`: `PlaceId("lme_<idx>")`.
  - `audience`: `[ActorId("user"), ActorId("prior_assistant")]` (or `[user, soul]`).
  - `timestamp_ms`: Parse `haystack_dates[s]` + deterministic intra-session offset `turn_idx * 1000` ms (**caveat:** session-level timestamps only, no per-turn ms). Question percept arrives at `parse_time(question_date)` from `ActorId("user")`.
  - `payload`: `turn.content` for history; `item.question` for final probe.
  - Soul action: `Action` replying to `ActorId("user")` in `PlaceId("lme_<idx>")` (or abstaining on unanswerable queries).
- **Missing facts / caveats:** Strictly 1-on-1 dyadic chat; session-level timestamps only; pre-written assistant turns in haystack; `_abs` suffix in `question_id` would leak gold if exposed in `PlaceId` or `Payload`.

#### R-54 Sampling & Power
- **Item counts (`LongMemEval_S` / `M` / `Oracle`):** **500 independent items** total:
  - `multi-session`: `133`
  - `temporal-reasoning`: `133`
  - `knowledge-update`: `78`
  - `single-session-user`: `70`
  - `single-session-assistant`: `56`
  - `single-session-preference`: `30`
  - Cross-cutting `_abs` abstention items: **`30`** distributed across the types above (`470` answerable + `30` `_abs`).
- **`S >= 354–471` paired power feasibility & clustering:**
  - **Fully feasible (`DE = 1.0` at `R=1`).** Each of the 500 items has its own independent haystack, and `500 >= 471` (`R=1`) and `500 >= 354` (`R=2`). No clustering penalty at `R=1`.
- **Stratification rule:**
  - Primary recommendation: **Run all 500 items** (29 above `471`, eliminating subsampling loss on the small `30`-item `_abs` and `single-session-preference` cells).
  - If subsampling (e.g. `S = 354` for `R=2`): **Proportional stratified sampling across `(question_type × is_abstention)`** via seeded PRNG, or keep all `30` `_abs` items fixed (`100%`) and sample `324/470` proportionally across the 6 `question_type` strata.
- **Per-item call cost shape:**
  - **Crucial cost distinction:** Unlike `locomo` and `group_mem_bench`, **haystack ingestion is NOT shared across items**! Every item requires ingesting ~40–50 sessions (~115K tokens in `S`; 500 sessions / ~1.5M tokens in `M`), i.e., `~20,000–25,000` sessions across 500 items in `S`. If Core ingestion uses zero LLM calls (deterministic storage + local embeddings), ingestion LLM cost is `0`; if any gateway/adapter hook calls an LLM per session, ingestion cost explodes (`~22,500` calls).
  - Query phase: `1` soul LLM call per item (`500` total).
  - Judge phase: `1` `gpt-4o` judge call per item (`500` total).

---

### 2.3 `memory_agent_bench`

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **MemoryAgentBench** (*Evaluating Memory in LLM Agents via Incremental Multi-Turn Interactions*, Hu et al., ICLR 2026 / `arXiv:2507.05257`).
- **Canonical source & revision:**
  - Code & evaluation harness: `https://github.com/HUST-AI-HYZ/MemoryAgentBench`
  - Dataset: `https://huggingface.co/datasets/ai-hyz/MemoryAgentBench`
  - Paper: `https://arxiv.org/abs/2507.05257`
  - Pinned revision & date: `[unverified from search; resolve via repo/HF API before fetch]` (branch `main`; ICLR 2026, posted July 2025).
- **Files needed & sizes:**
  - Hugging Face dataset `ai-hyz/MemoryAgentBench`: total size `~76.6 MB` across Parquet shards for 4 splits (`Accurate_Retrieval`, `Long_Range_Understanding`, `Conflict_Resolution`, `Test_Time_Learning`), plus `entity2id.json` (`1.76 MB`, needed only for Movie-Rec in `TTL`) and `README.md` (`9.88 kB`).
  - Exact Parquet shard file paths & byte sizes: `[unverified from search; resolve via repo/HF API before fetch]`.
- **Checksums:** `[unverified from search; resolve via repo/HF API before fetch]` (HF LFS/Xet exposes SHA-256 per Parquet file and `entity2id.json`).
- **License & access gate:**
  - SPDX ID: **`MIT`** (`HUST-AI-HYZ/MemoryAgentBench` GitHub and `ai-hyz/MemoryAgentBench` HF card).
  - Terms: Permits local copies, commercial use, and redistribution with MIT notice. *Caveat for owner:* Underlying reformulated subsets draw from LongMemEval (MIT), RULER, ∞Bench, EventQA, DetectiveQA, and Movie-Rec; verify upstream sub-dataset terms if redistributing derived artifacts.
  - Access gate: None.

#### R-52 Official Evaluation Protocol
- **Registry metrics (`benchmark.rs`):** `ar_acc`, `selective_forgetting_acc`.
- **Official definitions & alignment:**
  - `ar_acc` corresponds to the **`Accurate_Retrieval`** (`AR`) split; `selective_forgetting_acc` corresponds to the **`Conflict_Resolution`** (`CR`) split (called *Selective Forgetting* / `SF` in earlier drafts, implemented via `FactConsolidation-SH` and `FactConsolidation-MH`).
  - **Sub-task metric heterogeneity:** Official scoring in `HUST-AI-HYZ/MemoryAgentBench` is per sub-task (`source`):
    - `Accurate_Retrieval`: `substring_exact_match` (`SubEM`) / `exact_match` (`EM`) on RULER SH-QA, RULER MH-QA, and EventQA; **LLM-as-a-judge (`gpt-4o`)** on `LongMemEval(S*)` (`300` QAs).
    - `Conflict_Resolution` (`FactConsolidation-SH` & `FactConsolidation-MH`): `substring_exact_match` / `exact_match` accuracy (deterministic, no LLM judge).
    - (`Test_Time_Learning` uses MCC accuracy + `Recall@5` on Movie-Rec; `Long_Range_Understanding` uses `gpt-4o` judge F1 on `InfBench-Sum` + accuracy on `DetectiveQA`.)
  - Neither `ar_acc` nor `selective_forgetting_acc` is a single literal key in the repo; they are macro/micro averages over the `Accurate_Retrieval` and `Conflict_Resolution` splits. (*Owner choice:* run all `AR` sub-tasks with `gpt-4o` judge on `LME(S*)`, or restrict `ar_acc` to deterministic `SubEM`/`EM` sub-tasks under a distinct metric name.)
- **Reporting split:** Hugging Face splits `Accurate_Retrieval` and `Conflict_Resolution` (optionally all 4 splits including `Test_Time_Learning` and `Long_Range_Understanding`).
- **Official scoring script & revision:** `HUST-AI-HYZ/MemoryAgentBench` (`run_MemoryAgentBench.py` / `template.py` / eval scripts; `[exact scorer path & commit SHA unverified from search; resolve via repo/HF API before fetch]`).
- **LLM judge required?** **Mixed:** **No** for `Conflict_Resolution` (`FactConsolidation`) and `AR` sub-tasks `{RULER SH-QA, RULER MH-QA, EventQA}` (`SubEM`/`EM`); **Yes (`gpt-4o`)** for `AR`'s `LongMemEval(S*)` subset and `LRU`'s `InfBench-Sum`.
- **Known contamination:** Mixed — pre-existing reformulated subsets (RULER, LongMemEval, ∞Bench, DetectiveQA) carry pretraining exposure risk; newly constructed `EventQA` and `FactConsolidation` (July 2025) have lower pre-2025 contamination.

#### R-53 Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`
- **Raw schema (`ai-hyz/MemoryAgentBench` Parquet splits):** Rows contain `context` (long text/chunks for incremental injection), `questions` (`[str]`), `answers` (`[str]` or lists), and `metadata` (`qa_pair_ids`, `source`, keypoints).
- **`GoldBook` separation:** `answers`, `qa_pair_ids`, and any `metadata` keypoints MUST remain strictly in `GoldBook`.
- **6-fact `Percept` mapping & HONESTY CAVEAT:**
  - **Missing timestamps, missing speakers, and artificial wrapper:** Aside from the `LongMemEval(S*)` rows inside `AR`, `MemoryAgentBench` contexts are narrative documents or synthetic fact blocks (`FactConsolidation` MQUAKE-style statements, `RULER`, `EventQA` passages) that **lack real timestamps and distinct conversational speakers**. The official harness wraps each chunk in `"Please memorize it and I will ask some questions..."`.
  - **Two options for the owner:**
    - *Option A (Mappable as ordered observation stream):* Ingest each sequential chunk $c_i$ of `context` as a `Percept` with `actor = ActorId("stream_source")`, `place = PlaceId("mab_<split>_<row_idx>")`, `audience = [ActorId("stream_source"), ActorId("soul")]`, `timestamp_ms = T_0 + i * 60_000` (synthetic monotonic stream index preserving causal superseding order for `FactConsolidation` and ACT-R decay intervals without claiming wall-clock dates), and `payload = chunk_text` (omitting the artificial `"Please memorize it"` wrapper). Pose each `questions[j]` from `ActorId("evaluator")` on a cloned post-ingestion checkpoint.
    - *Option B (`not_mappable` for non-conversational subsets):* Under a strict interpretation of R-53 (*"cannot be represented without... inventing facts (for example, missing timestamps or speakers)"*), mark the non-conversational document-chunk rows `not_mappable` (or retain only `Conflict_Resolution` where sequence order is intrinsic to superseding + `LongMemEval-S*`).
- **Missing facts / caveats:** Missing real timestamps and speakers in most sub-tasks; artificial memorization prompt wrapper in official harness; multi-question reuse per context row requires post-ingestion checkpoint isolation.

#### R-54 Sampling & Power
- **Item counts (`ai-hyz/MemoryAgentBench`):** **146 HF rows** total across 4 splits, expanding via "inject once, query multiple times" to **2,071+ QAs** (`R-39` breakdown: `AR` = 22 rows / 1,000 QAs [SH-QA 100, MH-QA 100, LME(S*) 300, EventQA 500]; `TTL` = 6 rows / 300 QAs [MCC 100, Movie-Rec 200]; `LRU` = 110 rows / 171 QAs [∞Bench-Sum 100, DetectiveQA 71]; `CR` = 8 rows covering `FactConsolidation-SH` & `MH` — note `R-39` cites 100+100=200 at 262K while HF/paper length configurations span up to 800 QAs across 8 rows; `[exact per-row QA count in Conflict_Resolution unverified from search; inspect HF Parquet rows on fetch]`).
- **`S >= 354–471` paired power feasibility & clustering:**
  - **Severely clustered (`AR` = 22 context rows, `CR` = 8 context rows!).** Although QA count (`1,000` in `AR`, `200–800` in `CR`) is large, `8` context rows in `Conflict_Resolution` and `22` context rows in `Accurate_Retrieval` **cannot** provide `354` independent scenarios (`DE` over ~45–100 QAs/row collapses effective `N_eff`). Thus `memory_agent_bench` MUST report **row-clustered bootstrap CIs** and serve as a targeted mechanism test (`SF` / `valid_to` superseding & ACT-R decay) rather than standalone `A-16` NI certification.
- **Stratification rule:**
  - Run **all 8 rows of `Conflict_Resolution`** (`SH` + `MH`) and **all 22 rows of `Accurate_Retrieval`** (or sample `S_qa = 471` QAs proportionally across `(split × source × row_id)` with seeded PRNG).
- **Per-item call cost shape:**
  - Ingestion: `30` context streams for `AR+CR` (197K–534K tokens/row; chunked incrementally). Zero LLM calls if Core ingestion is algorithmic/embedder-only; expensive if LLM extraction runs per chunk.
  - Query phase: `1` LLM call per QA (`~1,200–1,800` calls full `AR+CR`, or `471` subsampled).
  - Judge phase: `0` LLM judge calls for `FactConsolidation`, `RULER`, `EventQA`; `1` `gpt-4o` judge call per QA for `LME(S*)` (`300` QAs) and `InfBench-Sum` (`100` QAs).

---

### 2.4 `group_mem_bench`

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **GroupMemBench** (*GroupMemBench: Benchmarking LLM Agent Memory in Multi-Party Conversations*, Yang et al., NeurIPS 2026 D&B / `arXiv:2605.14498v2`).
- **Canonical source & revision (split across two primary repositories):**
  - Conversation data (HF): `https://huggingface.co/datasets/kimperyang/GroupMemBench`
  - Questions & evaluation code (GitHub): `https://github.com/UCSB-NLP-Chang/GroupMemBench`
  - Paper: `https://arxiv.org/abs/2605.14498`
  - Pinned revision & date: `[unverified from search; resolve via repo/HF API before fetch]` (branch `main`; `arXiv:2605.14498v2`, NeurIPS 2026 D&B).
- **Files needed & sizes:**
  - HF `kimperyang/GroupMemBench` (`~159 MB` total):
    - `data/final/Finance/synthetic_domain_channels_rolevariants_Finance.json`
    - `data/final/Technology/synthetic_domain_channels_rolevariants_Technology.json`
    - `data/final/Healthcare/synthetic_domain_channels_rolevariants_Healthcare.json`
    - `data/final/Manufacturing/synthetic_domain_channels_rolevariants_Manufacturing.json`
    *(Exact per-file byte sizes `[unverified from search; resolve via repo/HF API before fetch]`.)*
  - GitHub `UCSB-NLP-Chang/GroupMemBench`:
    - `questions/<Domain>/<category>.jsonl` for `Domain in {Finance, Technology, Healthcare, Manufacturing}` and `category in {multi_hop, knowledge_update, temporal, term_ambiguity, user_implicit, abstention}` (24 `.jsonl` files; exact byte sizes `[unverified from search; resolve via repo/HF API before fetch]`).
    - Evaluation scripts in `UCSB-NLP-Chang/GroupMemBench` (`[exact script filename unverified from search; inspect repo before fetch]`).
- **Checksums:** `[unverified from search; resolve via repo/HF API before fetch]` (HF LFS/Xet provides SHA-256 for the 4 JSON files; GitHub `.jsonl` files have no published checksums, requiring `--record-checksums` on fetch).
- **License & access gate:**
  - SPDX IDs: **`CC-BY-4.0`** on HF `kimperyang/GroupMemBench` conversation files; **`MIT`** on GitHub `UCSB-NLP-Chang/GroupMemBench` code repo (owner should verify whether `questions/*.jsonl` in the GitHub repo fall under `MIT` or `CC-BY-4.0`).
  - Terms: Both `CC-BY-4.0` and `MIT` permit local copies, commercial use, and redistribution with attribution/notice.
  - Access gate: None.

#### R-52 Official Evaluation Protocol
- **Registry metrics (`benchmark.rs`):** `acc`, `knowledge_update`, `term_ambiguity`.
- **Official definitions & alignment:**
  - **Exact alignment:** Official GroupMemBench evaluates QA accuracy over all **745** questions (`acc`) and per-category accuracy across the 6 diagnostic categories (`multi_hop`, `knowledge_update`, `temporal`, `user_implicit`, `term_ambiguity`, `abstention`), where `knowledge_update` and `term_ambiguity` are official category accuracies.
  - *Note on `benchmark.rs` caveat:* GroupMemBench models role-conditioned term ambiguity and multi-party threads, **not** formal confidentiality/non-interference, so `benchmark.rs` requires supplementing it with Lab `VisibleScope` $\Delta_{\text{NI}}$ scenarios.
- **Reporting split:** All 4 domains (`Finance`, `Technology`, `Healthcare`, `Manufacturing`), all 6 query categories (`745` questions total).
- **Official scoring script & revision:** Evaluation script in `UCSB-NLP-Chang/GroupMemBench` (`[exact script path & commit SHA unverified from search; resolve via repo/HF API before fetch]`).
- **LLM judge required?** **Yes.** Official protocol uses **GPT-5** as automated LLM-as-a-judge grading candidate responses against ground-truth answers (`[exact GPT-5 judge prompt text unverified from search; inspect UCSB-NLP-Chang/GroupMemBench before scorer implementation]`). (*Owner choice:* pin official `gpt-5` judge or record an alternate judge under a renamed metric.)
- **Known contamination:** Low for pre-2026 models (`arXiv:2605.14498` posted May 2026, synthetic enterprise chats); increasing risk for models trained after mid-2026.

#### R-53 Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`
- **Raw JSON schema (verified from HF card):**
  - Each domain JSON is a dict keyed by channel name (`channel -> [message]`). Each message has:
    - `msg_node` (`"Msg_<n>"`)
    - `content` (`str`)
    - `author` (`"User_<n>"`)
    - `role` (`str`, e.g. `"Compliance Officer"`)
    - `timestamp` (ISO 8601 `str`)
    - `reply_to` (`"Msg_<m>"` or `null`)
    - `phase_name`, `topic`, `is_noise` (`bool`), `is_decision_point` (`bool`), `tone`, `style`, `expertise`.
- **`GoldBook` separation:**
  - `is_noise`, `is_decision_point`, `phase_name` (evaluation phase marker), and all `questions/<Domain>/<category>.jsonl` ground-truth answers/supporting evidence MUST remain strictly in `GoldBook` and never enter `Percept`.
- **6-fact `Percept` mapping (cleanest native fit in the Memory slice):**
  - `actor`: `ActorId(msg.author)` (`User_<n>`). Gateway can populate `ActorFacts` / initial public role descriptors from `role` (or omit `tone`/`style` tags if treated as post-hoc synthetic metadata; *owner choice* whether `role` is exposed in Gateway actor profile).
  - `place`: `PlaceId("gmb_<domain>_<channel>")`.
  - `audience`: All distinct `author` IDs active in that channel + `ActorId("soul")`.
  - `timestamp_ms`: Direct epoch ms parsed from ISO 8601 `msg.timestamp` (**no missing timestamps**!).
  - `reply_to`: `Option<EventId>` mapped deterministically from `msg.reply_to` (`Msg_<m>`)!
  - `payload`: `msg.content`.
  - **Question probe:** Each query in `questions/<Domain>/<category>.jsonl` is **bound to a specific asker** (`author`). Emit the question as a `Percept` from that asker `ActorId` in the target `PlaceId` on an isolated post-ingestion checkpoint for that domain; soul responds via `Action` (chat reply capability + receipt).
- **Missing facts / caveats:** None on timestamps, speakers, or `reply_to`! Note two manifest requirements: (1) fetching requires combining **two** sources (HF JSON conversations + GitHub JSONL questions); (2) lacks secrecy labels, so it does not replace Lab `VisibleScope` $\Delta_{\text{NI}}$ scenarios.

#### R-54 Sampling & Power
- **Item counts:**
  - Conversations: **120,000 messages** (`30,000`/domain across **33 channels**: Finance `6`, Technology `7`, Healthcare `10`, Manufacturing `10`; `6–18` speakers/domain, `~2.7M` tokens total).
  - Queries: **745 questions** across 6 categories (`multi_hop`, `knowledge_update`, `temporal`, `user_implicit`, `term_ambiguity`, `abstention`; exact per-category counts `[unverified from search; inspect the 24 .jsonl files upon fetch]`).
- **`S >= 354–471` paired power feasibility & clustering:**
  - At the question level (`745 >= 471`), item count exceeds `471`. However, all 745 questions query **only 4 domain workspaces (33 channels)** (`~186` Qs/domain, `~22.6` Qs/channel). Questions sharing a channel/domain are correlated (`DE > 1`), so **channel-clustered bootstrap CIs** MUST be reported alongside item-level CIs; treating 745 Qs as `745` independent scenarios (`DE = 1`) would overstate NI certainty.
- **Stratification rule:**
  - Prefer **all 745 questions** (ingestion is amortized across just 4 domain streams).
  - If subsampling to `S_q = 471` (`R=1`) or `S_q = 354` (`R=2`), use **proportional stratified sampling across `4 domains × 6 categories` (`24` strata)** via deterministic seeded PRNG, keeping minimum cell coverage in `knowledge_update` and `term_ambiguity`.
- **Per-item call cost shape:**
  - Ingestion: `4` domain streams (`120,000` messages, `~2.7M` tokens total), amortized once per arm. (`0` LLM calls if Core ingestion is algorithmic; very high if LLM-per-message.)
  - Query phase: `1` soul LLM call per question (`745` full; `471` subsampled).
  - Judge phase: `1` `gpt-5` judge call per question (`745` full; `471` subsampled).

---

### 2.5 `stale`

#### R-51 Provenance
- **Public existence & canonical name:** Confirmed public. Canonical name: **STALE** (*STALE: Can LLM Agents Know When Their Memories Are No Longer Valid?*, Chao et al., May 2026 / `arXiv:2605.06527`).
- **Canonical source & revision:**
  - Code & CUPMem & eval harness: `https://github.com/icedreamc/STALE`
  - Dataset (HF): `https://huggingface.co/datasets/STALEproj/STALE`
  - Paper: `https://arxiv.org/abs/2605.06527`
  - Pinned revision & date: `[unverified from search; resolve via repo/HF API before fetch]` (branch `main`; paper posted May 2026).
- **Files needed & sizes:**
  - `T1_T2_400_FULL.json` on `STALEproj/STALE` (400 conflict scenarios, 1,200 queries; exact byte size `[unverified from search; resolve via repo/HF API before fetch]`).
- **Checksums:** `[unverified from search; resolve via repo/HF API before fetch]` (HF LFS/Xet exposes SHA-256 for `T1_T2_400_FULL.json`).
- **License & access gate:**
  - SPDX ID: **`CC-BY-4.0`** (`STALEproj/STALE` dataset; distractor haystack sessions sampled from `LongMemEval`, which is **`MIT`**).
  - Terms: Permits local copies, commercial use, and redistribution with attribution.
  - Access gate: None.

#### R-52 Official Evaluation Protocol
- **Registry metrics (`benchmark.rs`):** `state_resolution`, `premise_resistance`.
- **Official definitions & alignment:**
  - `state_resolution` matches official **State Resolution (`SR`)** (detecting that an earlier belief/state is invalidated by newer implicit conflict context).
  - `premise_resistance` matches official **Premise Resistance (`PR`)** (pushing back against/rejecting queries that falsely presuppose the obsolete state).
  - **Omitted third official dimension:** STALE evaluates **three** probing dimensions per scenario (`SR`, `PR`, and **Implicit Policy Adaptation [`IPA`]**, 400 queries each = 1,200 queries total). `benchmark.rs` omits `IPA`. (*Owner choice:* add `implicit_policy_adaptation` to `benchmark.rs`, or score it as a supplementary metric.)
- **Reporting split:** Full `T1_T2_400_FULL.json` (400 scenarios covering Type I co-referential invalidation and Type II propagated invalidation, 1,200 total queries).
- **Official scoring script & revision:** Evaluation scripts in `icedreamc/STALE` (`[exact script path & commit SHA unverified from search; resolve via repo/HF API before fetch]`).
- **LLM judge required?** **Yes.** Automated LLM-as-a-judge (paper evaluations use **`Gemini-3.1-flash-lite`** as judge per search results; exact prompt templates from `icedreamc/STALE` / paper Appendix E/F `[unverified from search; inspect repo before scorer implementation]`).
- **Known contamination:** Low for pre-May-2026 model weights (`arXiv:2605.06527` published May 2026), though LongMemEval-derived distractor haystack sessions predate STALE (2024).

#### R-53 Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`
- **Raw JSON structure (`T1_T2_400_FULL.json`):** 400 scenario objects wrapping old ($m_o$) and new ($m_n$) user-state statements inside chronological multi-turn user-assistant sessions padded with LongMemEval-derived distractor sessions (up to 150K tokens), paired with `SR`, `PR`, and `IPA` queries and ground-truth labels (`[exact JSON field key names unverified from search; inspect T1_T2_400_FULL.json schema during converter implementation]`).
- **`GoldBook` separation:** Type I/II classification, explicit $m_o / m_n$ invalidation annotations, and gold `SR`/`PR`/`IPA` answers/rubrics MUST stay strictly in `GoldBook`.
- **6-fact `Percept` mapping:**
  - `actor`: `"user"` -> `ActorId("user")`; `"assistant"` -> `ActorId("prior_assistant")` (same pre-written assistant-turn ruling as `LongMemEval`).
  - `place`: `PlaceId("stale_<scenario_idx>")`.
  - `audience`: `[ActorId("user"), ActorId("prior_assistant")]`.
  - `timestamp_ms`: Chronological session timestamps from haystack + deterministic intra-session turn offset (`turn_idx * 1000` ms if per-turn ms are absent).
  - `payload`: Turn content; then each probe (`SR`, `PR`, `IPA`) MUST be injected as a `Percept` from `ActorId("user")` **from an independent cloned post-ingestion checkpoint** (so `SR` probing does not leak hint context into `PR` or `IPA`). Soul responds via `Action`.
- **Missing facts / caveats:** Dyadic user-assistant only; pre-written assistant turns in haystack; mandatory checkpoint isolation between `SR`, `PR`, and `IPA` probes on the same scenario.

#### R-54 Sampling & Power
- **Item counts (`T1_T2_400_FULL.json`):** **400 independent scenarios** (Type I + Type II implicit conflicts across >100 topics, up to 150K tokens), **1,200 queries** (`400` `SR` + `400` `PR` + `400` `IPA`).
- **`S >= 354–471` paired power feasibility & clustering:**
  - **Feasible at `R=2` (`400 >= 354` independent scenarios, `DE = 1.0` at `R=1`)!** For per-dimension binary endpoints (`SR` and `PR` independently), each has `S = 400` independent scenarios (`400 >= 354`, `~85%` of `471`). Pooling `SR + PR` to `800` queries does **not** give `800` independent trials because `SR` and `PR` share the same 400 haystacks; report `SR` and `PR` separately at `S = 400` (and scenario-clustered CIs if ever pooled).
- **Stratification rule:**
  - **Run all 400 scenarios** (no subsampling needed; `400` sits squarely inside `354–471`).
  - If subsampling for a smaller pilot (e.g. `S = 354` for `R=2`, or `S = 184` pilot), **stratify proportionally across `(conflict_type [Type I / Type II] × topic/context-length bucket)`** via seeded PRNG, keeping all probes (`SR`, `PR`, [`IPA`]) per selected scenario.
- **Per-item call cost shape:**
  - Ingestion: `400` independent haystacks (up to 150K tokens each; shared across the 2–3 probes per scenario via post-ingestion checkpoint!).
  - Query phase: `2` soul LLM calls/scenario for registered `SR + PR` (`800` total), or `3` calls/scenario if `IPA` is included (`1,200` total).
  - Judge phase: `1` judge LLM call per query (`800` for `SR+PR`; `1,200` with `IPA`).

---

## 3. Consolidated Summary Tables (`R-51`–`R-54`)

### Table R-51 — Provenance & Licensing

| `dataset_id` | Public / Canonical Name | Canonical Source(s) | Pinned Revision & Date | Required Files & Sizes | Source Checksums | License SPDX & Terms | Access Gate |
|---|---|---|---|---|---|---|---|
| `locomo` | Yes — **LoCoMo** (`arXiv:2402.17753`, ACL 2024) | GitHub: `https://github.com/snap-research/locomo` *(10 of 50 paper convs public)* | `[unverified from search; resolve via repo/HF API before fetch]` (`main`; Feb 2024) | `data/locomo10.json` (~2.7–2.8 MB; exact bytes `[unverified]`) | None published (use `--record-checksums`) | **`CC-BY-NC-4.0`** — local copies & adaptation allowed; **non-commercial only** | None |
| `longmemeval` | Yes — **LongMemEval** (`arXiv:2410.10813v2`, ICLR 2025) | HF: `https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned`; GitHub: `https://github.com/xiaowu0162/LongMemEval` | `[unverified from search; resolve via repo/HF API before fetch]` (`main`; ICLR 2025) | `longmemeval_s_cleaned.json` (`277,383,467` B); `longmemeval_oracle.json` (`15,388,478` B); `longmemeval_m_cleaned.json` (~2.74 GB, exact B `[unverified]`) | Yes (HF SHA-256): `s_cleaned` `d6f21ea9...`; `oracle` `821a2034...`; `m_cleaned` `9d79e552...` (full hashes in §2.2) | **`MIT`** — local copies, commercial use & redistribution allowed with notice | None |
| `memory_agent_bench` | Yes — **MemoryAgentBench** (`arXiv:2507.05257`, ICLR 2026) | HF: `https://huggingface.co/datasets/ai-hyz/MemoryAgentBench`; GitHub: `https://github.com/HUST-AI-HYZ/MemoryAgentBench` | `[unverified from search; resolve via repo/HF API before fetch]` (`main`; Jul 2025 / ICLR 2026) | HF Parquet splits (~76.6 MB total) + `entity2id.json` (1.76 MB); exact shard names/bytes `[unverified]` | `[unverified from search; resolve via HF API before fetch]` | **`MIT`** (repo/HF; wraps third-party sub-corpora — verify downstream terms if redistributing) | None |
| `group_mem_bench` | Yes — **GroupMemBench** (`arXiv:2605.14498v2`, NeurIPS 2026 D&B) | HF: `https://huggingface.co/datasets/kimperyang/GroupMemBench` (convs) + GitHub: `https://github.com/UCSB-NLP-Chang/GroupMemBench` (QAs & eval) | `[unverified from search; resolve via repo/HF API before fetch]` (`main`; May 2026 / NeurIPS 2026) | 4 domain JSONs under `data/final/<Domain>/` (~159 MB total) + 24 `questions/<Domain>/<cat>.jsonl` files; per-file bytes `[unverified]` | HF SHA-256 `[unverified]`; GitHub QAs have none (use `--record-checksums`) | **`CC-BY-4.0`** (HF data) + **`MIT`** (GitHub repo) — local copies, commercial use & redistribution allowed with attribution | None |
| `stale` | Yes — **STALE** (`arXiv:2605.06527`, May 2026) | HF: `https://huggingface.co/datasets/STALEproj/STALE`; GitHub: `https://github.com/icedreamc/STALE` | `[unverified from search; resolve via repo/HF API before fetch]` (`main`; May 2026) | `T1_T2_400_FULL.json` (exact bytes `[unverified]`) | `[unverified from search; resolve via HF API before fetch]` | **`CC-BY-4.0`** (STALE) + **`MIT`** (LongMemEval distractors) — local copies, commercial use & redistribution allowed | None |

---

### Table R-52 — Official Evaluation Protocol & Registry Alignment

| `dataset_id` | Registry Metrics (`benchmark.rs`) | Official Metric Definition & Registry Alignment | Reporting Split | Official Scoring Script | LLM Judge Required? (Model & Prompt) | Known Contamination Risk |
|---|---|---|---|---|---|---|
| `locomo` | `qa_f1`, `cat5_adversarial_acc` | **Matches `evaluation.py`** (`eval_question_answering`): Porter-stemmed token F1 on Cats 1–4 (`qa_f1`) + binary refusal substring check (`'no information available'` / `'not mentioned'`) on Cat 5 (`cat5_adversarial_acc`). *(Warning: Mem0 `J` drops Cat 5 and uses LLM judge — non-official vs `snap-research/locomo`.)* | All 10 convs in `locomo10.json` (1,986 QAs; **must retain Cat 5**) | `snap-research/locomo/task_eval/evaluation.py` (rev `[unverified]`) | **No** for official `evaluation.py` (optional `gpt-4o-mini` judge if adding separate `J` metric) | **High** (public since Feb 2024; widely memorized/cited) |
| `longmemeval` | `acc`, `abstention_f1` | `acc` -> `overall_acc` / `task_averaged_acc`. **`abstention_f1` is NOT official** -> closest official is **`abstention_acc`** on 30 `_abs` items (rename or keep as MonoAgens-defined F1). | `longmemeval_s_cleaned.json` (primary); `m_cleaned` (stress); `oracle` (ceiling) | `xiaowu0162/LongMemEval/src/evaluation/{evaluate_qa.py,print_qa_metrics.py}` | **Yes** — `gpt-4o` (`gpt-4o-2024-08-06`, `temp=0`), 5 task-specific prompt templates (`_abs` prompt verbatim in §2.2) | **Moderate–High** (Oct 2024; mitigated by 115K–1.5M haystack length) |
| `memory_agent_bench` | `ar_acc`, `selective_forgetting_acc` | `ar_acc` -> mean accuracy on `Accurate_Retrieval` (`SubEM`/`EM` + `LME(S*)` judge); `selective_forgetting_acc` -> mean `SubEM`/`EM` on **`Conflict_Resolution`** (`FactConsolidation` SH & MH). | `Accurate_Retrieval` + `Conflict_Resolution` (optionally also `TTL`, `LRU`) | `HUST-AI-HYZ/MemoryAgentBench` eval harness (`[path/rev unverified]`) | **Mixed** — **No** for `CR` (`FactConsolidation`), `RULER`, `EventQA` (`SubEM`/`EM`); **Yes (`gpt-4o`)** for `LME(S*)` & `InfBench-Sum` | **Mixed** (pre-existing RULER/LME/∞Bench high; new `EventQA`/`FactConsolidation` Jul 2025 lower) |
| `group_mem_bench` | `acc`, `knowledge_update`, `term_ambiguity` | **Exact official match:** `acc` = overall 6-category accuracy (`745` Qs); `knowledge_update` & `term_ambiguity` = official category accuracies. *(Supplement with Lab `VisibleScope` $\Delta_{\text{NI}}$.)* | All 4 domains × 6 categories (745 Qs over 120K msgs) | `UCSB-NLP-Chang/GroupMemBench` eval script (`[path/rev unverified]`) | **Yes** — **`gpt-5`** judge (exact prompt text `[unverified]`) | **Low** (May 2026 synthetic chats; rising for late-2026 models) |
| `stale` | `state_resolution`, `premise_resistance` | **Matches 2 of 3 official dims:** `SR` (`400` Qs) & `PR` (`400` Qs). **Omitted 3rd official dim:** `Implicit Policy Adaptation` (`IPA`, `400` Qs). | `T1_T2_400_FULL.json` (400 scenarios, 1,200 queries, Type I & II) | `icedreamc/STALE` eval script (`[path/rev unverified]`) | **Yes** — paper uses **`Gemini-3.1-flash-lite`** judge (exact prompt `[unverified]`) | **Low** (May 2026 scenarios; LongMemEval distractors from 2024) |

---

### Table R-53 — Mapping to MonoAgens 6-Fact `Percept` & `GoldBook`

| `dataset_id` | Unit -> `Percept` Mapping (`actor`, `place`, `audience`, `timestamp_ms`, `payload`, `reply_to`) | Soul Expected Output (`Action`) | `GoldBook` (Never Visible to Soul) | Missing Facts / Honesty Caveats |
|---|---|---|---|---|
| `locomo` | `actor`: `speaker_a` / `speaker_b` (soul as observer, or soul = `speaker_b`); `place`: `locomo_conv_<idx>`; `audience`: `[speaker_a, speaker_b, soul]`; `timestamp_ms`: `session_k_date_time` + `turn_idx * 1000` ms; `payload`: `text` (+ optional `blip_caption`) | Chat reply `Action` answering `qa[].question` (or exact refusal on Cat 5) from post-ingestion checkpoint | `qa[].answer`, `adversarial_answer`, `category`, `evidence`, `observation`, `session_summary`, `event_summary` | **Session-level timestamps only** (needs deterministic `+1000 ms/turn` offset); dyadic only; `img_url` not fetched offline |
| `longmemeval` | `actor`: `user` / `prior_assistant`; `place`: `lme_<idx>` (opaque, no `_abs`!); `audience`: `[user, prior_assistant]`; `timestamp_ms`: `haystack_dates[s]` + `turn_idx * 1000` ms; `payload`: `turn.content` | Chat reply `Action` answering `question` at `question_date` (or abstaining on `_abs`) | `answer`, `question_type`, `has_answer`, `answer_session_ids`, original `question_id` (`_abs` tag!) | **Session-level timestamps only**; pre-written `assistant` turns in history; strictly 1-on-1 dyadic |
| `memory_agent_bench` | *If Option A:* `actor`: `stream_source` (`LME-S*` uses `user`/`prior_assistant`); `place`: `mab_<split>_<row>`; `audience`: `[stream_source, soul]`; `timestamp_ms`: synthetic `T_0 + i * 60_000` ms; `payload`: raw chunk text (no `"Please memorize it"` wrapper) | Reply `Action` per `questions[j]` from post-ingestion checkpoint | `answers`, `qa_pair_ids`, `metadata` keypoints | **Missing wall-clock timestamps & multi-party speakers** in non-`LME` sub-tasks; artificial memorization prompt wrapper -> **owner choice: Option A vs. `not_mappable`** |
| `group_mem_bench` | `actor`: `User_<n>` (+ optional Gateway `role` profile); `place`: `gmb_<domain>_<channel>`; `audience`: channel participants + `soul`; `timestamp_ms`: **exact ISO 8601 `timestamp`**; `reply_to`: `Msg_<m>`; `payload`: `content` | Reply `Action` to asker's `question` in target channel from post-ingestion domain checkpoint | `is_noise`, `is_decision_point`, `phase_name`, QA gold answers & evidence | **Cleanest 6-fact fit** (no missing timestamps/speakers); lacks DLM clearance/secrecy labels (requires Lab $\Delta_{\text{NI}}$ supplement) |
| `stale` | `actor`: `user` / `prior_assistant`; `place`: `stale_<scenario_idx>`; `audience`: `[user, prior_assistant]`; `timestamp_ms`: haystack session timestamps (+ `+1000 ms/turn` if needed); `payload`: turn content | Reply `Action` per probe (`SR`, `PR`, [`IPA`]) from **isolated cloned post-ingestion checkpoints** | Type I/II tags, $m_o / m_n$ conflict spans, gold `SR`/`PR`/`IPA` answers & rubrics | Pre-written `assistant` turns; exact JSON keys `[unverified]`; MUST clone checkpoint so `SR` probe does not contaminate `PR`/`IPA` |

---

### Table R-54 — Item Counts, `A-16` Power (`S >= 354–471`), Stratification & Cost Shape

| `dataset_id` | Item Counts per Relevant Split | `S >= 354–471` Paired NI Feasibility & Clustering Note | Exact Stratification Rule (when subsampling) | Per-Item Model / Judge Call Cost Shape |
|---|---|---|---|---|
| `locomo` | **10 convs**, **1,986 QAs** (Cat1 `282`, Cat2 `321`, Cat3 `96`, Cat4 `841`, Cat5 `446`) | **Clustered — cannot reach `354` independent convs** (`10` convs, `~199` QAs/conv -> `DE ≈ 10.9` at `rho=0.05`, `N_eff ≈ 182`). Must use **conv-clustered bootstrap**; cannot standalone-certify `A-16` NI | Prefer **all 1,986 QAs**. If subsampling (`S_qa=471`), proportional across **`10 convs × 5 cats` (50 strata)**, retaining Cat 5 at `22.5%` | Ingestion: 10 convs (~3K turns total, amortized once/arm); Eval: **1 soul call/QA**; Judge: **0 calls** (official `evaluation.py`) |
| `longmemeval` | **500 independent items** (`multi-session` `133`, `temporal` `133`, `knowledge-update` `78`, `ss-user` `70`, `ss-asst` `56`, `ss-pref` `30`; `30` `_abs` embedded) | **Feasible (`DE = 1.0` at `R=1`).** `500 >= 471` (`R=1`) & `>= 354` (`R=2`) independent haystacks | Prefer **all 500 items**. If `S=354` (`R=2`), keep all `30` `_abs` + proportional `324/470` across `question_type` | Ingestion: **500 separate haystacks** (~40–50 sessions, ~115K tok/item in `S`; 0 LLM calls if Core ingestion is non-LLM); Eval: **1 soul call/item**; Judge: **1 `gpt-4o` call/item** |
| `memory_agent_bench` | **146 HF rows** (`AR` `22`, `LRU` `110`, `CR` `8`, `TTL` `6`) -> **2,071+ QAs** (`AR` `1,000`, `TTL` `300`, `LRU` `171`, `CR` `200–800`) | **Severely clustered** (`AR` = 22 rows, `CR` = 8 rows). Cannot reach `354` independent contexts; must use **row-clustered bootstrap** | Run **all 8 `CR` rows + all 22 `AR` rows** (or sample `S_qa=471` proportionally across `split × source × row_id`) | Ingestion: 30 `AR+CR` streams (197K–534K tok/row, amortized); Eval: **1 soul call/QA**; Judge: **0 calls** on `CR`/`RULER`/`EventQA`, **1 `gpt-4o` call/QA** on `LME(S*)` (`300`) & `InfBench-Sum` (`100`) |
| `group_mem_bench` | **4 domains / 33 channels**, `120,000` msgs (~2.7M tok), **745 QAs** across 6 categories | **Clustered at domain/channel level** (`745 >= 471` QAs, but shared across only **4 domains / 33 channels**). Must report **channel-clustered bootstrap CIs** | Prefer **all 745 QAs**. If `S_q=471` (`R=1`) or `354` (`R=2`), proportional across **`4 domains × 6 categories` (24 strata)** | Ingestion: **4 domain streams** (`120K` msgs total, amortized once/arm); Eval: **1 soul call/QA**; Judge: **1 `gpt-5` call/QA** |
| `stale` | **400 independent scenarios** (Type I + II, up to 150K tok), **1,200 queries** (`400` `SR`, `400` `PR`, `400` `IPA`) | **Feasible for `R=2` (`400 >= 354` independent scenarios)** per endpoint (`SR` and `PR` separately at `S=400`, `DE=1.0` at `R=1`) | **Run all 400 scenarios** (if pilot `S=354` or `184`, proportional across `Type I/II × topic/length bucket`) | Ingestion: **400 separate haystacks** (amortized across `SR+PR[+IPA]` via checkpoint); Eval: **2–3 soul calls/scenario** (`800` or `1,200`); Judge: **1 `Gemini-3.1-flash-lite` call/query** |

---

## 4. Primary Source Index

- **LoCoMo:** Paper [`arXiv:2402.17753`](https://arxiv.org/abs/2402.17753); GitHub [`snap-research/locomo`](https://github.com/snap-research/locomo) (`data/locomo10.json`, `task_eval/evaluation.py`, `LICENSE`).
- **LongMemEval:** Paper [`arXiv:2410.10813`](https://arxiv.org/abs/2410.10813); GitHub [`xiaowu0162/LongMemEval`](https://github.com/xiaowu0162/LongMemEval) (`src/evaluation/evaluate_qa.py`, `src/evaluation/print_qa_metrics.py`, `LICENSE`); Hugging Face [`xiaowu0162/longmemeval-cleaned`](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned).
- **MemoryAgentBench:** Paper [`arXiv:2507.05257`](https://arxiv.org/abs/2507.05257); GitHub [`HUST-AI-HYZ/MemoryAgentBench`](https://github.com/HUST-AI-HYZ/MemoryAgentBench); Hugging Face [`ai-hyz/MemoryAgentBench`](https://huggingface.co/datasets/ai-hyz/MemoryAgentBench).
- **GroupMemBench:** Paper [`arXiv:2605.14498`](https://arxiv.org/abs/2605.14498); Hugging Face [`kimperyang/GroupMemBench`](https://huggingface.co/datasets/kimperyang/GroupMemBench); GitHub [`UCSB-NLP-Chang/GroupMemBench`](https://github.com/UCSB-NLP-Chang/GroupMemBench).
- **STALE:** Paper [`arXiv:2605.06527`](https://arxiv.org/abs/2605.06527); Hugging Face [`STALEproj/STALE`](https://huggingface.co/datasets/STALEproj/STALE); GitHub [`icedreamc/STALE`](https://github.com/icedreamc/STALE).
