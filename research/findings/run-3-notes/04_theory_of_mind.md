# Run 3 Research Note — Theory of Mind Benchmarks (`R-51`–`R-54`)

- **Date:** `2026-11-17`
- **Scope:** Domain 5 (`BenchmarkDomain::TheoryOfMind`) in [`crates/lab/src/benchmark.rs`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L192-L232): `fantom`, `opentom`, `hi_tom`, `exploretom` (including disambiguation from `DynToM`), and `tomi` (`required_in_standard_suite = false`).
- **Brief & Specs Referenced:** [`researcher-prompt-3.md`](file:///home/muqsith/Projects/Mono/research/researcher-prompt-3.md), [`openspec/changes/lab-datasets/design.md`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/design.md), [`benchmark-datasets/spec.md`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md), [`benchmark-evaluation/spec.md`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-evaluation/spec.md), [`crates/protocol/src/message.rs`](file:///home/muqsith/Projects/Mono/crates/protocol/src/message.rs#L122-L215), [`crates/protocol/src/facts.rs`](file:///home/muqsith/Projects/Mono/crates/protocol/src/facts.rs#L133-L181), [`crates/lab/src/scenario.rs`](file:///home/muqsith/Projects/Mono/crates/lab/src/scenario.rs#L88-L220), and [`A14_A16_info_gap_indonesian_context.md`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A14_A16_info_gap_indonesian_context.md#L313-L427).
- **Verification Discipline:** All dataset facts below were re-verified via web search against primary repositories, Hugging Face dataset cards, and papers. Exact Git commit SHAs, exact repository-internal download URLs, and unprinted byte sizes that were not directly returned by search snippets are explicitly marked **`[unverified — inspect/record on owner fetch]`** and never fabricated.

---

## 1. Executive Summary & Owner Choices Needed

All five registered Theory of Mind (`ToM`) benchmarks exist publicly (`fantom`, `opentom`, `hi_tom`, `exploretom`, `tomi`), plus a sixth distinct benchmark (**`DynToM`**) whose metric name leaked into `exploretom` in Run 2. None of the upstream sources publish standalone `SHA256SUMS` manifests (though Hugging Face LFS/Parquet repos store object hashes internally), so the Lab's `--record-checksums` trust-on-first-use mechanism ([`design.md`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/design.md#L68-L70)) will be needed at fetch time. None require an LLM-as-a-judge for scoring!

Before the architect writes `lab/datasets.manifest.toml` and dataset converters/scorers, **six owner decisions** are required:

### Choice 1 — Resolve the `exploretom` vs. `DynToM` Conflation ([`benchmark.rs:221`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L218-L224))
`crates/lab/src/benchmark.rs:221` registers `exploretom` with `primary_metrics = ["action_acc", "dyntom_acc"]`. This arose because Run 2 ([`R39_R48_benchmarks_identity.md:75`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/R39_R48_benchmarks_identity.md#L75)) placed **ExploreToM** (`arXiv:2412.12175`) and **DynToM** (`arXiv:2505.17663`) in the same table row:
- **`ExploreToM`** (`facebookresearch/ExploreToM`, `facebook/ExploreToM`, `CC-BY-NC-4.0`) does **not** define `action_acc` or `dyntom_acc`; its official metric is **exact-match accuracy** across belief/state-tracking tasks (`1st`/`2nd`-order, accessible vs. inaccessible states, forward/backward action inference). Moreover, its authors warn that the static Hugging Face `train` split (`13,300` rows) is primarily a sample/training resource and advise running their $A^*$ DSL generator dynamically to stress-test new models.
- **`DynToM`** (`GAIR-NLP/DynToM`, `YangXiao-nlp/DynToM`, `Apache-2.0`) is a completely separate ACL 2025 benchmark (`78,100` MCQs across `1,100` social contexts / `5,500` sequential scenarios) scoring **Multiple-Choice Accuracy** across *Understanding (U)* and *Transformation (T)* over 5 mental-state axes (`Belief`, `Emotion`, `Intention`, `Action`, `Subject`).

**Options for the owner:**
1. **Split into two datasets:** Keep `exploretom` (either pinning a deterministic $A^*$-generated eval seed or the static HF slice, with official exact-match belief/action metrics) **and** register `dyntom` as a separate `BenchmarkDatasetSpec` (`Apache-2.0`, official MCQ accuracy over U/T).
2. **Replace `exploretom` with `dyntom`:** If commercial permissiveness (`Apache-2.0`) and a static, zero-generator evaluation corpus (`78,100` MCQs) are preferred over `ExploreToM`'s `CC-BY-NC-4.0` generator.
3. **Keep `exploretom` only:** Fix `primary_metrics` to official ExploreToM exact-match belief/action accuracy and drop `dyntom_acc`.

### Choice 2 — Clear Non-Commercial Licenses & Terms (`R-51`)
Only **`hi_tom`** (`Apache-2.0` repo) and **`dyntom`** (`Apache-2.0`) have unambiguous commercial-friendly OSI licenses. **`fantom`**'s repo (`skywalker023/fantom`) carries an `MIT` `LICENSE`, though no separate data license is explicitly documented in the repo. **`opentom`**, **`exploretom`**, and **`tomi`** are **`CC-BY-NC-4.0`** (non-commercial), and `SeacowX/OpenToM` explicitly forbids using OpenToM data for training/fine-tuning or submitting items into public playgrounds where inputs may be harvested. The owner must set `license.status` (`cleared` vs `refused`) per dataset.

### Choice 3 — Resolve `Percept` Mapping Mode for Narrative/Text ToM Datasets (`R-53`)
MonoAgens `Percept` requires `(actor, place, audience, timestamp_ms, payload)` ([`message.rs:125-138`](file:///home/muqsith/Projects/Mono/crates/protocol/src/message.rs#L125-L138)), where `audience` must be non-empty ([`facts.rs:142`](file:///home/muqsith/Projects/Mono/crates/protocol/src/facts.rs#L142)) and events arrive in `Time` order. **None** of the 5+1 ToM benchmarks include wall-clock timestamps:
- **Option A — Event-by-Event `Percept` Stream with Virtual Clock + Rule-Parsed `Audience`:** Each sentence/turn becomes one `Percept` on a monotonic synthetic virtual clock (e.g., `+1,000 ms` per utterance/event; `+60,000 ms` between `DynToM` stages), matching how `Scenario` uses `at_ms` virtual time in [`scenario.rs:215`](file:///home/muqsith/Projects/Mono/crates/lab/src/scenario.rs#L215). For `fantom` (dialogue turns), `hi_tom` (room entry/exit + private `Tell`), `tomi` (numbered bAbI lines + `.trace` observability tags), and `exploretom` (DSL state/visibility), `audience` can track currently co-present actors (with the soul acting as an **auditor/proctor** included in `audience` so it perceives all scenes and tests `PerceptionLedger` tracking of *other* actors' presence). For `opentom` and `dyntom` (unstructured third-person prose), extracting exact per-sentence room-presence `audience` requires deterministic NLP/rule parsing or falling back to a fixed scene cast.
- **Option B — Single Story/Dialogue Turn `Percept` (Zero Synthetic Fact Parsing):** Feed the entire narrative or conversation transcript as one `OpaquePayload` percept (`actor = "benchmark_narrator"`, `place = "story_0"`, `audience = {soul, narrator}`, `timestamp_ms = 0`), followed by each question percept (`timestamp_ms = 1,000 * q_idx`). Avoids inventing per-sentence timestamps or parsing prose into `audience`, at the cost of testing only the LLM reasoning step rather than MonoAgens's structural `PerceptionLedger`/`VisibleScope` filtering.
- **Option C — Mark Narrative-Only Prose Datasets `not_mappable`:** If the owner forbids virtual-clock timestamps or narrator-framed percepts on third-person stories (`opentom`, `dyntom`, `exploretom`, `hi_tom`, `tomi`), only `fantom` (multi-party dialogue) survives as native conversational interaction.

### Choice 4 — `fantom` Metric & Scorer Dependencies (`R-52`)
`eval_fantom.py` scores **`ALL*`** (6 simultaneous question types) using a `SentenceTransformer` cosine-similarity check + token-F1 on `BeliefQ[Dist.]`, whereas **`ALL`** (5 of 6, omitting `BeliefQ[Dist.]`) uses purely deterministic exact/list/F1 matching.
- **Options:** (a) Invoke `skywalker023/fantom/eval_fantom.py` via `lab/adapters` Python (per [`design.md` Decision 5](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/design.md#L95-L100)) so `BeliefQ[Dist.]` uses the exact upstream `SentenceTransformer` model, OR (b) score `ALL` in pure Rust and mark `ALL*` as requiring the Python `SentenceTransformer` adapter. Also choose whether to evaluate `fantom`'s `short` conversation setting, `full` conversation setting, or both.

### Choice 5 — `opentom` Registry Metric Completeness (`R-52`)
`benchmark.rs:205` lists `&["macro_f1_coarse", "macro_f1_fine", "attitude_f1"]`, covering `Loc_c`, `Loc_f`, and `Att`, while **omitting `MHop`** (12 of 23 questions per narrative = 52.2% of `opentom`) and collapsing 1st-order `(F)` vs. 2nd-order `(S)`.
- **Options:** Keep the 3 listed metrics (reporting `Long` and `ExtraLong` separately or pooled) OR add `macro_f1_mhop` (and optionally `(F)`/`(S)` breakdowns) to match the official OpenToM paper tables.

### Choice 6 — `hi_tom` & `fantom` Story-Level vs. Question-Level Power (`R-54`)
`A-16` targets `S >= 354–471` paired items (`R=2` / `R=1`).
- **`fantom`** has **256 conversations** (`< 354`), **703** information-piece sets (`ALL*` units), and **1,415 `BeliefQ`** items (`> 471`).
- **`hi_tom`** (`Hi-ToM_data.json`) has **1,200 QAs** (`> 471` at the QA level), corresponding to roughly **240 stories** (`< 354` at the story level if 5 orders/story).
- **Options:** Treat the **information-piece set** (`fantom`, `N=703`) or **QA item** (`hi_tom`, `N=1,200`) as the sampled unit with **cluster-bootstrap CIs grouped by conversation/story ID**; OR run all `256` `fantom` conversations / `240` `hi_tom` stories and document the story-level CI width; OR (for `hi_tom` only) use `generate_tomh.sh` with a pinned seed to generate `>= 471` stories (diverging from static `Hi-ToM_data.json`).

---

## 2. Detailed Per-Dataset Analysis (`R-51`–`R-54`)

### 2.1 `fantom` (`FANToM`)

#### R-51 Provenance
- **Public Existence & Canonical Name:** Yes — **FANToM** (*A Benchmark for Stress-testing Machine Theory of Mind in Interactions*; Kim, Sclar, Zhou, Le Bras, Kim, Choi, & Sap, EMNLP 2023).
- **Canonical Sources:**
  - Paper: [arXiv:2310.15421](https://arxiv.org/abs/2310.15421) / Project Page: [hyunw.kim/fantom](https://hyunw.kim/fantom)
  - Official GitHub Repository: [skywalker023/fantom](https://github.com/skywalker023/fantom) (linked from `hyunwkim/FANToM` / project page).
  - Community Hugging Face mirrors exist (e.g., `TomTraining/TomDatasets`, `TomTraining/HardToMDatasets`), but the primary canonical source is `skywalker023/fantom`.
- **Pinned Revision & Date:** No formal GitHub release tag is published on `skywalker023/fantom` (EMNLP 2023 / Oct 2023 release). Exact HEAD commit SHA **`[unverified — pin via git ls-remote at manifest lock]`**.
- **Files Needed & Sizes:**
  - Repo contains `README.md`, `environment.yml`, `eval_fantom.py`, and `LICENSE`.
  - `eval_fantom.py` dynamically downloads the benchmark JSON file(s) (`fantom.json` / `data/`) when executed; the exact remote storage URL inside `eval_fantom.py` and exact byte sizes **`[unverified — inspect eval_fantom.py and record on first fetch]`**.
- **Checksums:** Not published upstream; must be captured via `monoagens-lab datasets fetch --record-checksums`.
- **License & Access Gate:** `LICENSE` in `skywalker023/fantom` is **MIT** (permissive; allows local copies, commercial use, and derived redistribution). **Caveat:** `MIT` explicitly appears on the GitHub code repository; whether the dynamically downloaded `data/` JSON carries a separate data-specific notice inside the archive should be checked on fetch. No sign-up or access gate.

#### R-52 Official Evaluation Protocol
- **Splits & Context Settings:** Single evaluation benchmark of **256 information-asymmetric conversations** (no train split). Evaluated under two input settings: **`short`** (truncated window around the character's departure, unseen exchange, and return; average **13.8 turns**, **21.9 words/turn**) and **`full`** (entire multi-subtopic conversation).
- **Question Counts across 256 Conversations:**
  - `BeliefQ`: **1,415** (`[Dist.]` free-response + `[Choice]` multiple-choice)
  - `FactQ`: **703**
  - `AnswerabilityQ`: **3,392** (`703` `[List]` + `2,689` `[Y/N]`)
  - `InfoAccessQ`: **3,392** (`703` `[List]` + `2,689` `[Y/N]`)
- **Official Metric Definitions:**
  - **`All*` (registry `fantom_all_star`):** Strict consistency score — `1` iff the model answers **all 6 ToM question formulations simultaneously** for the same information piece (`BeliefQ[Choice]`, `BeliefQ[Dist.]`, `AnswerabilityQ[List]`, `AnswerabilityQ[Y/N]`, `InfoAccessQ[List]`, `InfoAccessQ[Y/N]`).
  - **`All` (registry `fantom_all`):** Strict consistency across **5 of the 6** formulations, **excluding `BeliefQ[Dist.]`** (introduced so automated scores can compare against human baselines, which did not take `BeliefQ[Dist.]`).
  - **Component Scorers:** `BeliefQ[Choice]` = rule-based choice accuracy; `BeliefQ[Dist.]` = `SentenceTransformer` cosine similarity (checking proximity to accessible character belief vs. omniscient view) + token-F1 word-overlap filter; `AnswerabilityQ[List]` & `InfoAccessQ[List]` = strict exact-set string match (all and only the correct character names); `AnswerabilityQ[Y/N]` & `InfoAccessQ[Y/N]` = rule-based binary matching / weighted F1; `FactQ` = token-F1.
  - **Official Script:** `eval_fantom.py` in `skywalker023/fantom` (custom agents plug in via `interact()` / `batch_interact()`).
- **LLM Judge Required?** **No.** Uses `SentenceTransformer` embeddings (exact checkpoint name inside `eval_fantom.py` `[unverified]`) + deterministic token-F1/string matching.
- **Registry Metric Alignment:** `fantom_all_star` and `fantom_all` match official **`All*`** and **`All`** (note naming normalization only).
- **Contamination:** Multi-formulation `ALL*` consistency and conversational structure were explicitly designed to defeat static shortcuts; cross-lingual/contamination checks (e.g., XToM analyses) found little evidence of direct verbatim memorization, though the dataset has been public since October 2023.

#### R-53 Mapping to MonoAgens Percepts
- **Native Units:** Multi-party dialogue turns (`speaker: utterance`) with explicit natural-language departure/return utterances (e.g., *"Hey guys, I'll go grab a coffee"*, *"Hey, I'm back..."*) creating private sub-windows where absent characters do not hear intermediate utterances, followed by ToM probes (`BeliefQ`, `AnswerabilityQ`, `InfoAccessQ`, `FactQ`).
- **Percept Mapping:**
  - **Timestamps:** Absent in raw FANToM. Converter assigns deterministic synthetic virtual timestamps (`+1,000 ms` per turn) on the Lab virtual clock.
  - **Actor / Place / Audience:**
    - `actor`: normalized speaker `ActorId` for each dialogue turn.
    - `place`: `PlaceId` per conversation (e.g., `fantom_conv_<id>`).
    - `audience`: **Directly supported via conversational presence tracking!** Maintain the active participant set $P_t$; when a turn contains a departure or return cue (or using metadata if provided in `fantom.json` — check field schema on fetch), update $P_t$ so intermediate turns carry `audience = P_t ∪ {soul_auditor}` (excluding the absent character!). Note: if the soul acts as an **omniscient auditor** (`soul_auditor ∈ audience` on every turn), MonoAgens's `PerceptionLedger` / `VisibleScope` can directly track which *other* actors witnessed each utterance without leaking gold answers.
    - `payload`: raw turn utterance text (gold beliefs/access labels stripped into `GoldBook`).
  - **Probes:** Each question is injected at the end of the conversation (`actor = "proctor"`, `audience = {soul_auditor, proctor}`, `timestamp_ms = T_end + k * 1000`). The soul must emit a `Say` / response act answering the question; `Scorer` evaluates all 6 formulations against `GoldBook` to compute `All*` and `All`.
- **Honesty / Leakage Check:** No gold answer, missing-character flag, or `Answerability` label enters any `Percept` unless the converter injects ground-truth absence metadata not present in text — **presence tracking must derive strictly from the dialogue text turns (or documented speaker-visibility rules)**, never from `GoldBook` question answers.

#### R-54 Sampling & Power (`S >= 354–471`)
- **Dataset Size:** `256` conversations, `703` target information-piece sets (`All*` / `All` units), `1,415` `BeliefQ`s, `703` `FactQ`s, `3,392` `AnswerabilityQ`s, `3,392` `InfoAccessQ`s.
- **Power Feasibility:** At the **conversation level (`N=256`)**, FANToM is **below** `354–471`. At the **information-piece set level (`N=703` `All*` units)**, a seeded subset of **`S = 471` sets** (or **all `703` sets across all `256` conversations**) exceeds the `354–471` paired power target — provided paired CIs use **cluster-bootstrap resampled by `conversation_id`** because sets within the same conversation share context!
- **Stratification:** Stratify by `short` vs `full` setting (or run both paired) and by conversation length / number of participants / number of character exits.
- **Cost Shape:** Per conversation: ~`14` turns (`short`) ingested sequentially (or batched), plus `~2.75` information-piece sets × ~`13–35` question probes per conversation (`~8,902` total Qs across `256` conversations; `~12.7` Qs per `All*` set on average). **0 LLM judge calls/item** (`SentenceTransformer` local scoring only).

---

### 2.2 `opentom` (`OpenToM`)

#### R-51 Provenance
- **Public Existence & Canonical Name:** Yes — **OpenToM** (*OpenToM: A Comprehensive Benchmark for Evaluating Theory-of-Mind Reasoning Capabilities of Large Language Models*; Xu et al., ACL 2024).
- **Canonical Sources:**
  - Paper: [arXiv:2402.06044](https://arxiv.org/abs/2402.06044)
  - Official GitHub Repository: [SeacowX/OpenToM](https://github.com/SeacowX/OpenToM)
  - Official Hugging Face Dataset: [SeacowX/OpenToM](https://huggingface.co/datasets/SeacowX/OpenToM)
- **Pinned Revision & Date:** No formal GitHub release tag (ACL 2024 / Feb 2024 release). Exact commit SHAs on GitHub and HF **`[unverified — pin via git ls-remote at manifest lock]`**.
- **Files Needed & Sizes:**
  - GitHub `data/` and HF `SeacowX/OpenToM` (~**54.9 MB** total HF repo):
    - `opentom.json` (~**34.3 MB**, `596` regular narratives, avg `194.3` words; `13,708` QAs, HF split `Long`)
    - `opentom_long.json` (~**10.0 MB**, `100` extra-long `OpenToM-L` narratives, avg `491.6` words; `2,300` QAs, HF split `ExtraLong`)
    - `opentom_data/` (`metadata.json`, `metadata_long.json`, and genre-split metadata files).
- **Checksums:** Not published by GitHub repo (HF LFS tracks sha256 internally); owner records on first fetch.
- **License & Access Gate:** **`CC-BY-NC-4.0`** (`Creative Commons Attribution-NonCommercial 4.0 International`) applied across the GitHub repo and HF card (no separate MIT/Apache code license). **Additional usage restriction in README:** OpenToM data **shall not be used for training or fine-tuning LLMs**, and users are warned **not** to run items through public web playgrounds that log/harvest inputs. No sign-up gate.

#### R-52 Official Evaluation Protocol
- **Splits:** **Zero train split.** Evaluation splits are `Long` (`opentom.json`: `596` narratives / `13,708` QAs) and `ExtraLong` (`opentom_long.json`: `100` narratives / `2,300` QAs); `696` narratives / `16,008` QAs total (`23` Qs/narrative: `13` 1st-order + `10` 2nd-order).
- **Question Taxonomy (23 Qs per narrative):**
  - `Loc_c` (Coarse Location, `4` Qs): binary/coarse belief about initial vs moved location (`Loc_c(F)` 1st-order, `Loc_c(S)` 2nd-order).
  - `Loc_f` (Fine Location, `6` Qs): exact container/location (`Loc_f(F)`, `Loc_f(S)`).
  - `MHop` (Multi-Hop, `12` Qs): container fullness & social-norm accessibility (`MHop(F)`, `MHop(S)`).
  - `Att` (Psychological Attitude, `1` Q): character emotional/evaluative attitude (1st-order).
  - Perspectives: `mover` vs `observer`.
- **Official Metric Definition:** **Macro-averaged F1** (`sklearn.metrics.f1_score(true, pred, average='macro')`) per task category (`Loc_c`, `Loc_f`, `MHop`, `Att`) and ToM order (`(F)`, `(S)`), chosen because binary/ternary class labels are imbalanced.
- **Official Script & Judge:** `src/evaluate.py` (`--result_path`, `--location_granularity`, `--perspective`). **No LLM judge.** `evaluate.py` extracts answers locally and uses deterministic **argmax lexical overlap** against the item's candidate option set (ties scored incorrect/unscored).
- **Registry Metric Alignment:** `crates/lab/src/benchmark.rs:205` lists `&["macro_f1_coarse", "macro_f1_fine", "attitude_f1"]`. These correspond to `Loc_c` Macro-F1, `Loc_f` Macro-F1, and `Att` Macro-F1, **omitting `MHop` (`12/23` = 52.2% of questions)** and collapsing `(F)` vs `(S)` orders.
- **Contamination:** Designed with zero train split to curb leakage; public on GitHub/HF since Feb 2024 (MuSeToM checks note theoretical exposure risk, though not verbatim memorization).

#### R-53 Mapping to MonoAgens Percepts
- **Native Units:** Unstructured third-person prose paragraphs (`~194` or `~492` words) describing characters with explicit personality traits (*considerate*, *inconsiderate*, *negativistic*), preferences, intents, and object-moving actions, followed by `23` binary/ternary MCQs.
- **Percept Mapping:**
  - **Timestamps:** Absent. Synthetic virtual timestamps (`+1,000 ms` per sentence or single turn).
  - **Sentence-by-Sentence vs. Single-Turn Caveat:** Unlike `FANToM` (dialogue turns) or `ToMi`/`HI-TOM` (structured lines), OpenToM narratives are **natural prose paragraphs**. Splitting into sentence percepts with `actor = "narrator"` is trivial, **but inferring exact per-sentence room-visibility `audience` sets from free-form prose risks parser errors or inventing spatial facts not explicitly demarcated**.
  - **Cleanest Mapping Options:**
    1. *Single-narrative percept* (`actor = "narrator"`, `place = "opentom_<story_id>"`, `audience = {soul, narrator}`) + `23` question percepts (or `1` independent run per story-question if questions share the narrative prefix); OR
    2. *Sentence-stream percepts* with `actor = "narrator"` and `audience = {soul, narrator}` (synthetic `+1,000 ms`/sentence), testing sequential narrative ingestion without fabricating per-sentence character visibility.
- **Honesty / Leakage Check:** Strip `answer`, `ground_truth`, `question_type`, and character-trait metadata tags if not in the narrative text; feed only `narrative` + `question` (+ `options` if presented in prompt format matching `run_baseline.py`).

#### R-54 Sampling & Power (`S >= 354–471`)
- **Dataset Size:** `696` narratives (`596` `Long` + `100` `ExtraLong`), `16,008` QAs total (`696` `Att` Qs, `2,784` `Loc_c` Qs, `4,176` `Loc_f` Qs, `8,352` `MHop` Qs).
- **Power Feasibility:** **Yes.** `696` narratives already exceeds `S >= 354–471` at the **narrative level** (and gives `471` `Att` Qs when sampling `471` narratives, or `696` `Att` Qs when running all narratives!).
- **Stratification:** Sample `S = 471` narratives (or run all `696`) stratified proportionally across `Long` (`596/696 ≈ 85.6%`, `403` items) vs `ExtraLong` (`100/696 ≈ 14.4%`, `68` items — or oversample `ExtraLong` to `100` + `371` `Long` with inverse-probability weighting), personality type (*considerate*, *inconsiderate*, *negativistic*), and `mover`/`observer` perspectives. Cluster-bootstrap CIs by `story_id` when pooling across the 23 Qs per story.
- **Cost Shape:** `1` narrative context (`~194` or `~492` words) + `23` short QA turns (or `4–11` if filtering out `MHop`, though keeping `MHop` is recommended), **0 LLM judge calls/item**.

---

### 2.3 `hi_tom` (`HI-TOM`)

#### R-51 Provenance
- **Public Existence & Canonical Name:** Yes — **HI-TOM** (*HI-TOM: A Benchmark for Evaluating Higher-Order Theory of Mind Reasoning in Large Language Models*; Wu, He, et al., Findings of EMNLP 2023).
- **Canonical Sources:**
  - Paper: [arXiv:2310.16755](https://arxiv.org/abs/2310.16755)
  - Official GitHub Repository: [ying-hui-he/Hi-ToM_dataset](https://github.com/ying-hui-he/Hi-ToM_dataset)
- **Pinned Revision & Date:** No formal GitHub release tag (EMNLP 2023 / Oct 2023). Exact commit SHA **`[unverified — pin via git ls-remote]`**.
- **Files Needed & Sizes:**
  - `Hi-ToM_data/Hi-ToM_data.json` (`1,200` higher-order ToM QA pairs; ~few MB, exact byte size **`[unverified]`**).
  - Generator scripts also included in repo: `generate_tomh.sh`, `generate_tasks.py`, `tasks.py`, `README.md`.
- **Checksums:** Not published upstream; record on first fetch.
- **License & Access Gate:** **Apache-2.0** (`ying-hui-he/Hi-ToM_dataset`; paper is `CC BY-SA 4.0`). Allows local copies, commercial use, and redistribution of derived files. No access gate.

#### R-52 Official Evaluation Protocol
- **Splits:** Single evaluation file `Hi-ToM_data/Hi-ToM_data.json` containing **1,200** multiple-choice QA pairs spanning **0th-order (reality) through 4th-order** recursive beliefs, with and without deceptive `Tell` communications (note: paper also discusses broader 1,800-instance generation / 240-per-order slices; exact per-order counts inside `Hi-ToM_data.json` `[unverified — inspect JSON on fetch]`).
- **Official Metric Definitions:**
  - **Standard Accuracy:** Exact-match MCQ accuracy computed independently at each ToM order (`0th..4th`), stratified by `w/o deception` vs `w/ deception` and story length.
  - **Joint Accuracy:** Cumulative recursive accuracy — an order-$n$ question is marked correct **only if** order $n$ and all lower-order questions for the same story are answered correctly (in deception comparisons, 0th-order reality checks are typically excluded from the recursive joint chain since reality is unaffected by deceptive `Tell` beliefs).
- **Official Script & Judge:** Deterministic MCQ exact/string matching against gold `answer` / `choices`. **No LLM judge.** (Note: `ying-hui-he/Hi-ToM_dataset` ships `generate_tomh.sh` / `generate_tasks.py`; exact filename of any standalone scoring script in the repo was not directly confirmed `[unverified]`, though MCQ exact match is straightforward to reimplement in Rust with a fixture parity check).
- **Registry Metric Alignment:** `crates/lab/src/benchmark.rs:213` lists `joint_acc_0_to_4`. This is **not** an exact upstream identifier — the paper calls the metrics **Standard Accuracy** and **Joint Accuracy** (across orders 0–4). Scorer should report both `joint_acc` (and specify whether order 0 is included or `1..=4` only, matching the paper!) and `standard_acc` per order `0..4`.
- **Contamination:** Public since Oct 2023; synthetic multi-agent room/container templates carry moderate pre-training exposure risk on lower orders (0–2), while 3rd–4th order deceptive `Tell` chains remain challenging.

#### R-53 Mapping to MonoAgens Percepts
- **Native Units:** Synthetic multi-agent object-finding stories (`story` string) with explicit room entry/exit events, container transfers, private `Tell` side-channels, and deceptive claims, paired with recursive `question`, `choices`, `answer`, and `order` metadata.
- **Percept Mapping:**
  - **Timestamps:** Absent. Synthetic monotonic virtual clock (`+1,000 ms` per story sentence).
  - **Actor / Place / Audience:** **Well-suited for Event-by-Event `Percept` streaming (Mode 1)!**
    - Each sentence in `story` cleanly corresponds to a discrete world or communication event:
      - Room movement (`<Agent> entered/exited <Room>`): updates the deterministic room-occupancy map for `PlaceId`.
      - Object action in `<Room>`: `actor = <Agent>`, `place = <Room>`, `audience = occupants(<Room>) ∪ {soul_auditor}`.
      - Private `Tell` / deceptive message (`<A> privately told <B> ...`): `actor = <A>`, `place = <Room>`, **`audience = {<A>, <B>, soul_auditor}`**! This directly exercises MonoAgens's `Audience` / `PerceptionLedger` / `VisibleScope` private side-channel semantics without leaking gold labels!
    - Note: Alternatively, Mode 2 feeds the full `story` as one percept (`actor = "narrator"`) if the Lab forbids parsing `story` lines into `place`/`audience`.
- **Honesty / Leakage Check:** Strip `answer`, `order`, and `deception` tags before constructing percepts; only the story sentences and question+choices reach the soul.

#### R-54 Sampling & Power (`S >= 354–471`)
- **Dataset Size:** `1,200` QA pairs in `Hi-ToM_data.json` (corresponding to ~`240` underlying stories if 5 orders/story).
- **Power Feasibility:**
  - At the **QA level (`N=1,200`)**, `S = 471` (or all `1,200`) exceeds `354–471`.
  - At the **story level (`~240` stories)**, **Joint Accuracy** couples all 5 orders of the same story, so `N_stories ≈ 240 < 354`. Thus running all `1,200` QAs (`~240` stories) with story-clustered CIs is slightly below `354` independent story pairs unless either (a) the Lab uses `generate_tomh.sh` with a pinned seed to generate `>= 471` stories (`>= 2,355` QAs) — note this departs from the static `Hi-ToM_data.json` file — or (b) reports all `1,200` QAs with story-clustered CIs as an exhaustive run of the static release.
- **Stratification:** If subsampling `1,200` QAs, sample **whole stories** (not isolated questions, otherwise Joint Accuracy breaks!) stratified equally by `deception` (`w/` vs `w/o`) and story length.
- **Cost Shape:** `~10–25` short event sentences + `5` recursive MCQ probes per story (`1,200` probes total across all stories), **0 LLM judge calls/item**.

---

### 2.4 `exploretom` (`ExploreToM`) — and `DynToM` Disambiguation

#### R-51 Provenance (`ExploreToM` vs `DynToM`)

| Attribute | `ExploreToM` (matches `dataset_id = "exploretom"`) | `DynToM` (source of `dyntom_acc` in `benchmark.rs:221`) |
|---|---|---|
| **Paper** | Sclar et al. (ICLR 2025), [arXiv:2412.12175](https://arxiv.org/abs/2412.12175) | Xiao et al. (ACL 2025), [arXiv:2505.17663](https://arxiv.org/abs/2505.17663) |
| **GitHub** | [facebookresearch/ExploreToM](https://github.com/facebookresearch/ExploreToM) | [GAIR-NLP/DynToM](https://github.com/GAIR-NLP/DynToM) (release `v1.0.0`, ~May 2024) |
| **Hugging Face** | [facebook/ExploreToM](https://huggingface.co/datasets/facebook/ExploreToM) | [YangXiao-nlp/DynToM](https://huggingface.co/datasets/YangXiao-nlp/DynToM) |
| **Files & Sizes** | HF `train` split: `13,300` rows (CSV / auto-converted Parquet; few MB, exact bytes `[unverified]`). GitHub holds A* DSL generator (`compute_statistics.py`, tests) rather than bulk static test set. | `story.json` + `question_new.json` (on HF `train` split: **~376 MB / `375,836,020` bytes** total). |
| **Pinned Revision** | No formal GitHub release tag; commit SHA `[unverified]`. | GitHub release `v1.0.0` (exact commit SHA `[unverified]`). |
| **Checksums** | Not published (HF LFS internal hashes only). | Not published (HF LFS internal hashes only). |
| **License** | **`CC-BY-NC-4.0`** (both GitHub and HF). | **`Apache-2.0`** (commercial-friendly). |
| **Access Gate** | None. | None. |

#### R-52 Official Evaluation Protocol
- **`ExploreToM`:**
  - **Official Metric:** **Exact-match accuracy** against `expected_answer` across belief/state-tracking tasks (`1st`-order, `2nd`-order, physical/communication knowledge-gain asymmetry `E1`/`E2`, accessible vs. inaccessible states, forward/backward action inference, double transfers).
  - **Splits / Reporting Caveat:** HF `facebook/ExploreToM` provides a single `13,300`-row `train` split (commonly used for SFT!). The `facebookresearch/ExploreToM` README explicitly advises **running their $A^*$ DSL generator** (162 combinatorial settings across $p\in\{2,3,4\}$ people, $a\in\{2,3,4\}$ actions, $r\in\{1,2\}$ rooms, $m\le 15$ total actions) rather than relying solely on static HF samples.
  - **Official Script & Judge:** `facebookresearch/ExploreToM` (`compute_statistics.py` etc.); **no LLM judge** (exact match).
  - **Registry Metric Conflict:** `crates/lab/src/benchmark.rs:221` lists `&["action_acc", "dyntom_acc"]`. **Neither is an official `ExploreToM` metric!** `action_acc` should be renamed to ExploreToM's official exact-match belief/state/action accuracy breakdowns (`exact_match_acc`, optionally stratified by `nth_order` and accessibility), and `dyntom_acc` belongs to `DynToM`.
  - **Contamination:** Explicitly engineered via combinatorial $A^*$ search over a DSL to resist pre-training memorization.
- **`DynToM`:**
  - **Official Metric:** **Multiple-choice Accuracy** (exact ground-truth option match across `78,100` MCQs in `train` split), broken down by **Understanding (U)** vs **Transformation (T)** and 5 dimensions (`Belief`, `Emotion`, `Intention`, `Action`, `Subject`).
  - **Script & Judge:** `GAIR-NLP/DynToM` `eval/` scripts; **no LLM judge**.
  - **Contamination:** Recent (ACL 2025), multi-stage temporal transformations across `1,100` social contexts / `5,500` sequential scenarios; strong resistance to static memorization.

#### R-53 Mapping to MonoAgens Percepts
- **`ExploreToM`:**
  - **Native Units:** Program-generated stories (`story_structure` DSL + natural-language `infilled_story`) with explicit room movements, secret observers/cameras, hidden transfers, paired with `question` and `expected_answer`. **No wall-clock timestamps.**
  - **Percept Mapping:** Like `HI-TOM`, `story_structure` / sentence steps map to event-by-event percepts (`+1,000 ms`/step) with `place` and `audience` reflecting physical room presence (`E1`) or hidden communication channels (`E2`), **OR** single-turn/sentence-stream `narrator` percepts if avoiding prose parsing on `infilled_story`. Strip `Human_State`, `nth_order`, and `expected_answer` columns completely so zero gold leaks.
- **`DynToM`:**
  - **Native Units:** Multi-stage temporal social contexts (`1,100` contexts × `5` interconnected sequential stages = `5,500` scenarios) + `78,100` MCQs (`U` at stage $t$, `T` across stages $t \to t'$). **No wall-clock timestamps**, though stages are explicitly ordered temporally!
  - **Percept Mapping:** Each of the 5 sequential scenarios in a context maps cleanly to a temporal episode/stage (`+60,000 ms` or `+3,600,000 ms` synthetic virtual stage gap between scenarios so MonoAgens episodic boundaries / temporal decay engage naturally!), with questions posed after their corresponding stage. Note: prose scenarios lack deterministic line-by-line room-exit markers, so `actor = "narrator"` and `audience = {soul, narrator}` per stage/sentence avoids inventing visibility facts.

#### R-54 Sampling & Power (`S >= 354–471`)
- **`ExploreToM`:** Static HF slice has `13,300` rows (or generator produces arbitrary $S$). A seeded subset of **`S = 471`** (or `600`) stories/questions easily meets `S >= 354–471`, stratified across number of characters ($p\in\{2,3,4\}$), rooms ($r\in\{1,2\}$), `nth_order` (`1st` vs `2nd`), and accessibility (`with` vs `without` knowledge-gain asymmetry). Cost: `1` short story (`<= 15` actions) + `1` question probe per row (or grouped by `story_structure`), **0 judge calls**.
- **`DynToM`:** `1,100` social contexts (`5,500` scenarios, `78,100` MCQs). Easily reaches `S >= 354–471` at the **context level** (`471` of `1,100` contexts, or the paper's 30% subset = `330` contexts / `24,330` Qs — note `471` contexts yields `~2,355` stages and `~33,441` Qs, or sample `1` U + `1` T question per context/stage for budget control!). Stratified by `Understanding (U)` vs `Transformation (T)` and the 5 mental-state dimensions (`Belief`, `Emotion`, `Intention`, `Action`, `Subject`), cluster-bootstrapped by `context_id`. **0 judge calls**.

---

### 2.5 `tomi` (`ToMi`, Optional in Registry)

#### R-51 Provenance
- **Public Existence & Canonical Name:** Yes — **ToMi** (*Revisiting the Evaluation of Theory of Mind through Question Answering*; Le, Boureau, & Nickel, EMNLP-IJCNLP 2019, `D19-1598`).
- **Canonical Source:** GitHub [facebookresearch/ToMi](https://github.com/facebookresearch/ToMi) | Paper: [ACL Anthology D19-1598](https://aclanthology.org/D19-1598/) / [10.18653/v1/D19-1598](https://doi.org/10.18653/v1/D19-1598).
- **Pinned Revision & Date:** No formal GitHub release tag (EMNLP 2019). Exact commit SHA **`[unverified — pin via git ls-remote]`**.
- **Files Needed & Sizes:** **Critical provenance detail:** `facebookresearch/ToMi` does **not** commit pre-generated `.txt` files in `data/`! Instead, running `python main.py` generates `train.txt`, `train.trace`, `val.txt`, `val.trace`, `test.txt`, `test.trace` (`999` stories × `6` questions = `5,994` Qs per split). Exact generated byte sizes depend on generator flags/seed **`[unverified — verify deterministic seed in main.py and record checksums]`**.
- **Checksums:** Not published (and generated dynamically unless a frozen archive is generated & pinned by owner).
- **License & Access Gate:** **`CC-BY-NC-4.0`** (`LICENSE` in `facebookresearch/ToMi`). Non-commercial only; no access gate.

#### R-52 Official Evaluation Protocol
- **Splits:** `test.txt` (`999` stories, `5,994` questions; `train.txt` and `val.txt` are `5,994` Qs each).
- **6 Questions per Story:** `memory`, `reality`, `first_order` (Agent A), `first_order` (Agent B), `second_order` (Agent A), `second_order` (Agent B); stories classified in `.trace` as `true_belief`, `false_belief`, or `second_order_false_belief`.
- **Official Metrics:** Per-question-type **accuracy** (`memory`, `reality`, `1st-order`, `2nd-order`) and **joint story accuracy** (all 6 questions in a story answered correctly). **No LLM judge** (exact string match on single-word location/object answer).
- **Registry Metric Alignment:** `crates/lab/src/benchmark.rs:229` lists `&["shortcut_perturbation_acc"]`. This is **not** an official `ToMi` metric! It is a Lab-derived perturbation metric motivated by:
  - **Ullman (2023)** ([arXiv:2302.08399](https://arxiv.org/abs/2302.08399)): trivial alterations (e.g., transparent containers, swapped premises/line-of-sight) collapse high LLM accuracy down to near 0% / chance.
  - **Shapira et al. (2024)** ([arXiv:2305.14763](https://arxiv.org/abs/2305.14763), EACL 2024): "Clever Hans" stress-testing perturbations (name swaps, distractors, spatial/syntactic rewrites) drop LLM accuracy by **20 to 50+ percentage points**.
- **Contamination:** **Severe.** Public since 2019; templated bAbI-style stories are heavily present in LLM pre-training data and vulnerable to surface-position shortcuts. Retain **strictly** as a fast scripted/perturbation sanity check (`required_in_standard_suite = false`), never as ToM capability proof.

#### R-53 Mapping to MonoAgens Percepts
- **Native Units:** bAbI-style numbered narrative lines (`1 Mary entered the hall.`, `2 John moved the apple...`) with **no wall-clock timestamps** and **no conversational dialogue**. Crucially, each line in the generator / `.trace` tracks **exact agent observability**!
- **Percept Mapping:**
  - **Timestamps:** Absent; assign synthetic `+1,000 ms` per numbered line.
  - **Actor / Place / Audience:** Every line explicitly states the acting agent and room/exit action, so `actor`, `place`, and `audience` (currently present agents ∪ `{soul_auditor}`) can be mapped deterministically without inventing facts, followed by the 6 question percepts. (Or Mode 2: single story percept if testing raw prompt perturbation).
- **Honesty / Leakage Check:** Do **not** include `.trace` tags (`false_belief`, `first_order_1_tom`, line-level observability IDs) inside `Percept::payload` — keep `.trace` strictly in `GoldBook` for stratification and scoring!

#### R-54 Sampling & Power (`S >= 354–471`)
- **Dataset Size:** `999` stories (`5,994` questions) in `test.txt`.
- **Power Feasibility:** **Yes.** `999` stories easily supports `S = 471` stories (`2,826` questions) or all `999` stories.
- **Stratification:** Stratify `S = 471` stories equally across `.trace` story types (`true_belief`, `false_belief`, `second_order_false_belief`) and pair each base story with its Ullman/Shapira perturbation twin for `shortcut_perturbation_acc` (paired base vs. perturbed delta!).
- **Cost Shape:** `~6–12` short lines + `6` short questions per story (doubled when paired with perturbation variants), **0 LLM judge calls/item**.

---

## 3. Consolidated Summary Tables (`R-51`–`R-54`)

*(Note: `dyntom` is included below as an un-registered candidate entry because `benchmark.rs:221` currently references `dyntom_acc` under `exploretom`.)*

### Table 1 — `R-51` Provenance

| `dataset_id` | Public? / Canonical Name | Canonical Source & Pinned Revision | Needed Files & Sizes | Checksums Published? | License (SPDX) & Terms / Gate |
|---|---|---|---|---|---|
| `fantom` | Yes — **FANToM** (`arXiv:2310.15421`, EMNLP 2023) | [github.com/skywalker023/fantom](https://github.com/skywalker023/fantom) *(no release tag; SHA `[unverified]`)* | `eval_fantom.py` downloads `data/` JSON (`fantom.json` / `data/`) *(exact URL & byte size `[unverified]`)* | No *(owner records on fetch)* | **MIT** (repo `LICENSE`; data-specific notice `[unverified]`); no gate |
| `opentom` | Yes — **OpenToM** (`arXiv:2402.06044`, ACL 2024) | [github.com/SeacowX/OpenToM](https://github.com/SeacowX/OpenToM) & [hf.co/datasets/SeacowX/OpenToM](https://huggingface.co/datasets/SeacowX/OpenToM) *(SHA `[unverified]`)* | `opentom.json` (~34.3 MB), `opentom_long.json` (~10.0 MB), `opentom_data/` (~54.9 MB total HF repo) | No standalone manifest *(HF LFS hashes internal)* | **`CC-BY-NC-4.0`** (non-commercial; **forbids LLM training/fine-tuning & public playground submission**); no gate |
| `hi_tom` | Yes — **HI-TOM** (`arXiv:2310.16755`, EMNLP 2023 Findings) | [github.com/ying-hui-he/Hi-ToM_dataset](https://github.com/ying-hui-he/Hi-ToM_dataset) *(no release tag; SHA `[unverified]`)* | `Hi-ToM_data/Hi-ToM_data.json` (~few MB, exact bytes `[unverified]`); optional `generate_tomh.sh` | No *(owner records on fetch)* | **`Apache-2.0`** (repo; paper `CC BY-SA 4.0`); commercial-friendly; no gate |
| `exploretom` | Yes — **ExploreToM** (`arXiv:2412.12175`, ICLR 2025) | [github.com/facebookresearch/ExploreToM](https://github.com/facebookresearch/ExploreToM) & [hf.co/datasets/facebook/ExploreToM](https://huggingface.co/datasets/facebook/ExploreToM) *(SHA `[unverified]`)* | HF `train` split (CSV/Parquet, 13.3k rows, few MB `[unverified]`) **or** GitHub $A^*$ DSL generator | No standalone manifest *(HF LFS hashes internal)* | **`CC-BY-NC-4.0`** (non-commercial; both GitHub & HF); no gate |
| `dyntom` *(candidate)* | Yes — **DynToM** (`arXiv:2505.17663`, ACL 2025) | [github.com/GAIR-NLP/DynToM](https://github.com/GAIR-NLP/DynToM) (`v1.0.0`) & [hf.co/datasets/YangXiao-nlp/DynToM](https://huggingface.co/datasets/YangXiao-nlp/DynToM) *(SHA `[unverified]`)* | `story.json` + `question_new.json` (HF `train` split: **~376 MB / `375,836,020` B**) | No standalone manifest *(HF LFS hashes internal)* | **`Apache-2.0`** (commercial-friendly); no gate |
| `tomi` *(optional)* | Yes — **ToMi** (`D19-1598`, EMNLP 2019) | [github.com/facebookresearch/ToMi](https://github.com/facebookresearch/ToMi) *(no release tag; SHA `[unverified]`)* | Generated via `main.py`: `train/val/test.{txt,trace}` *(6 files; sizes depend on seed `[unverified]`)* | No *(deterministic generator + owner records)* | **`CC-BY-NC-4.0`** (non-commercial); no gate |

---

### Table 2 — `R-52` Official Evaluation Protocol

| `dataset_id` | Registry Metrics vs. Official Metrics | Reporting Split | Official Script | LLM Judge? | Known Contamination / Shortcut Issues |
|---|---|---|---|---|---|
| `fantom` | `fantom_all_star` → **`All*`** (6/6 simultaneous); `fantom_all` → **`All`** (5/6 excl. `BeliefQ[Dist.]`) | All `256` convs (`short` & `full` settings) | `eval_fantom.py` (`skywalker023/fantom`) | **No** (`SentenceTransformer` cosine + token-F1 + exact/rule match; model name `[unverified]`) | Low direct memorization in XToM checks; `ALL*` resists surface shortcuts, public since Oct 2023 |
| `opentom` | `macro_f1_coarse` → `Loc_c` Macro-F1; `macro_f1_fine` → `Loc_f` Macro-F1; `attitude_f1` → `Att` Macro-F1 *(omits `MHop` & `(F)`/`(S)` split)* | `Long` (`596`) + `ExtraLong` (`100`), zero train split | `src/evaluate.py` (`SeacowX/OpenToM`) | **No** (deterministic argmax lexical overlap + `sklearn` Macro-F1) | Zero train split mitigates memorization; public since Feb 2024 (theoretical leakage risk noted) |
| `hi_tom` | `joint_acc_0_to_4` *(Lab alias)* → **Joint Accuracy** (`0..4`, excl. 0th in dec. chains) + **Standard Accuracy** (`0..4`) | Full `Hi-ToM_data.json` (`1,200` QAs) | Repo has `generate_tomh.sh` / `generate_tasks.py` *(standalone eval script `[unverified]`)* | **No** (exact MCQ option match) | Synthetic templates public since Oct 2023; 0–2 orders exposed, 3–4 orders w/ deception remain hard |
| `exploretom` | **`action_acc` & `dyntom_acc` are NOT official ExploreToM metrics!** Official = **Exact-Match Accuracy** (`1st`/`2nd` belief, accessible/inaccessible, forward/backward action) | Author-recommended: dynamically generated $A^*$ DSL subsets *(HF `13.3k` `train` is mostly SFT)* | `facebookresearch/ExploreToM` (`compute_statistics.py`) | **No** (exact match on `expected_answer`) | Combinatorial $A^*$ DSL space specifically designed to avoid pre-training memorization |
| `dyntom` *(candidate)* | `dyntom_acc` → **Multiple-Choice Accuracy** (`U` Understanding vs `T` Transformation across `Belief`, `Emotion`, `Intention`, `Action`, `Subject`) | `train` (`78,100` MCQs; paper human baseline used 30% = `330` contexts) | `GAIR-NLP/DynToM` `eval/` scripts | **No** (exact MCQ choice match) | ACL 2025; dynamic multi-stage transitions resist static memorization |
| `tomi` *(optional)* | `shortcut_perturbation_acc` *(NOT official; derived from Ullman 2023 / Shapira 2024)*; Official = **per-type & joint story accuracy** | `test.txt` (`999` stories / `5,994` Qs) | Generated/evaluated via `facebookresearch/ToMi` (`main.py`) | **No** (exact string match) | **Severe** — 2019 template corpus leaked in pre-training; Ullman (2023) & Shapira et al. (2024) show 20–50+ pp collapse on trivial perturbations |

---

### Table 3 — `R-53` Mapping to MonoAgens `Percept`s

| `dataset_id` | Raw Unit Structure | Timestamps Present? | Recommended `Percept` Mapping (`actor`, `place`, `audience`, `timestamp_ms`, `payload`) | Gold Isolation & Honest Representation Verdict |
|---|---|---|---|---|
| `fantom` | Multi-party dialogue turns (`speaker: text`) w/ natural-language exit/return cues + 4 Q families | **No** *(use `+1,000 ms`/turn virtual clock)* | `actor` = speaker; `place` = `fantom_conv_<id>`; `audience` = active room occupants `∪ {soul_auditor}` updated on exit/return cues; `payload` = utterance text; then Q probes | **Mappable (Mode 1 native dialogue)** — directly exercises `PerceptionLedger`/`audience`; strip gold beliefs/access sets into `GoldBook` |
| `opentom` | Unstructured prose paragraphs (`194` / `492` words) + `23` binary/ternary MCQs | **No** *(use `+1,000 ms`/sentence or single-turn)* | `actor` = `"narrator"`; `place` = `opentom_<id>`; `audience` = `{soul, narrator}` (sentence-stream or single-narrative percept), followed by `23` MCQ probes | **Mappable via Mode 2 (or narrator sentence-stream)** — free-form prose lacks deterministic line-level room-exit tags for per-sentence `audience`; strip trait/ground-truth tags |
| `hi_tom` | Structured multi-agent room entry/exit, object moves, & private `Tell` / deceptive lines + 5 order MCQs | **No** *(use `+1,000 ms`/line virtual clock)* | **Mode 1:** `actor` = acting agent; `place` = room; `audience` = room occupants `∪ {soul_auditor}` (or `{sender, recipient, soul_auditor}` on private `Tell`!). Then MCQ probes | **Mappable (Mode 1 event-stream)** — cleanly maps room visibility and private deceptive side-channels to `audience`; strip `answer`/`order` |
| `exploretom` | Programmatic DSL `story_structure` / `infilled_story` (room moves, hidden observers) + Q/A | **No** *(use `+1,000 ms`/step virtual clock)* | **Mode 1:** derive per-step `actor`, `place`, `audience` from DSL/event steps (`E1` room occupancy, `E2` secret observer), or **Mode 2** single-story `narrator` percept | **Mappable** — strip `Human_State`, `nth_order`, and `expected_answer` into `GoldBook` |
| `dyntom` *(candidate)* | `5` interconnected temporal prose scenarios per context + MCQs (`U` & `T`) | **No wall-clock** *(ordered stages `1..5`; use `+60,000 ms`/stage)* | Each sequential scenario stage injected at `+60,000 ms` intervals (`actor = "narrator"`, `audience = {soul, narrator}`), with `U`/`T` MCQ probes after their stage | **Mappable (stage-stream)** — preserves temporal evolution across stages without inventing line-level `audience`; strip gold answers |
| `tomi` *(optional)* | Numbered bAbI narrative lines (`1 Mary entered...`) + `.trace` observability tags + `6` Qs | **No** *(use `+1,000 ms`/line virtual clock)* | **Mode 1:** parse line actor/room entry-exit into `actor`, `place`, `audience = room_occupants ∪ {soul_auditor}`, followed by `6` Q probes | **Mappable** — **never** pass `.trace` flags (`false_belief`, observability IDs) into `Percept::payload` |

---

### Table 4 — `R-54` Sampling, Stratification & Cost Shape (`S >= 354–471`)

| `dataset_id` | Total Items per Relevant Split | Reaches `S >= 354–471`? | Recommended Stratification Rule | Per-Item Model-Call Cost Shape |
|---|---|---|---|---|
| `fantom` | `256` convs / `703` info-piece sets (`All*` units) / `1,415` `BeliefQ`s / `8,902` total Qs | **Partial:** `256 < 354` at conv level; **Yes (`N=703`)** at info-piece set level (w/ conv-clustered CIs) | Run all `256` convs (`703` sets) or sample `S = 471` sets stratified by `short`/`full`, participant count, & exits | `~14` turns (`short`) + `~13` Q probes per `All*` set (`~35` Qs/conv); **0 judge calls** |
| `opentom` | `696` narratives (`596` `Long` + `100` `ExtraLong`), `16,008` QAs (`23`/story; `696` `Att`) | **Yes** (`696 > 471` even for `Att` at `1` Q/story) | Seeded `S = 471` narratives (or all `696`) stratified by `Long`/`ExtraLong`, personality trait, & `mover`/`observer`; cluster CIs by `story_id` | `1` narrative (`194`/`492` words) + `23` MCQ turns (or `11` if excluding `MHop`); **0 judge calls** |
| `hi_tom` | `1,200` QAs in `Hi-ToM_data.json` (~`240` stories if 5 orders/story) | **Yes at QA level (`1,200`)**; **No at static story level (`~240 < 354`)** unless generated | Sample **whole stories** stratified by `deception` (`w/` vs `w/o`) & length; run all `~240` stories (`1,200` QAs) or generate `>= 471` stories via `generate_tomh.sh` | `~10–25` lines + `5` recursive MCQ probes/story (`1,200` probes total); **0 judge calls** |
| `exploretom` | `13,300` rows in HF `train` (or arbitrary $A^*$ generator size) | **Yes** (`13,300 >> 471`) | Seeded `S = 471` items stratified by `p` (`2/3/4`), `r` (`1/2`), `nth_order` (`1st`/`2nd`), & knowledge-gain asymmetry | `1` short story (`<= 15` actions) + `1` Q probe per row; **0 judge calls** |
| `dyntom` *(candidate)* | `1,100` contexts / `5,500` scenarios / `78,100` MCQs | **Yes** (`1,100 > 471` at context level) | Seeded `S = 471` contexts (yielding `2,355` stages; subsample `1` U + `1` T Q/stage or all `~33.4k` Qs) stratified across `U`/`T` & 5 mental-state axes; cluster CIs by `context_id` | `5` sequential stage narratives + `~5–71` MCQ probes per context; **0 judge calls** |
| `tomi` *(optional)* | `999` stories / `5,994` Qs in `test.txt` | **Yes** (`999 > 471` stories) | Seeded `S = 471` base stories stratified equally by `true_belief`, `false_belief`, `second_order_false_belief`, each paired with perturbation twin | `~6–12` lines + `6` Q probes per story (×2 with perturbation twin); **0 judge calls** |

---

## 4. Primary Sources Cited

- **FANToM:** Kim et al. (EMNLP 2023), [arXiv:2310.15421](https://arxiv.org/abs/2310.15421); Project page [hyunw.kim/fantom](https://hyunw.kim/fantom); Repo [github.com/skywalker023/fantom](https://github.com/skywalker023/fantom)
- **OpenToM:** Xu et al. (ACL 2024), [arXiv:2402.06044](https://arxiv.org/abs/2402.06044); Repo [github.com/SeacowX/OpenToM](https://github.com/SeacowX/OpenToM); HF [huggingface.co/datasets/SeacowX/OpenToM](https://huggingface.co/datasets/SeacowX/OpenToM)
- **HI-TOM:** Wu, He, et al. (Findings of EMNLP 2023), [arXiv:2310.16755](https://arxiv.org/abs/2310.16755); Repo [github.com/ying-hui-he/Hi-ToM_dataset](https://github.com/ying-hui-he/Hi-ToM_dataset)
- **ExploreToM:** Sclar et al. (ICLR 2025), [arXiv:2412.12175](https://arxiv.org/abs/2412.12175); Repo [github.com/facebookresearch/ExploreToM](https://github.com/facebookresearch/ExploreToM); HF [huggingface.co/datasets/facebook/ExploreToM](https://huggingface.co/datasets/facebook/ExploreToM)
- **DynToM:** Xiao et al. (ACL 2025), [arXiv:2505.17663](https://arxiv.org/abs/2505.17663); Repo [github.com/GAIR-NLP/DynToM](https://github.com/GAIR-NLP/DynToM); HF [huggingface.co/datasets/YangXiao-nlp/DynToM](https://huggingface.co/datasets/YangXiao-nlp/DynToM)
- **ToMi & Shortcut Literature:** Le, Boureau, & Nickel (EMNLP 2019), [D19-1598](https://aclanthology.org/D19-1598/); Repo [github.com/facebookresearch/ToMi](https://github.com/facebookresearch/ToMi); Ullman (2023), [arXiv:2302.08399](https://arxiv.org/abs/2302.08399); Shapira et al. (EACL 2024), [arXiv:2305.14763](https://arxiv.org/abs/2305.14763)
