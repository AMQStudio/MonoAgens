# MonoAgens Research Run 3 — External Benchmark Datasets: Indonesian Register & Security (`multilexnorm`, `indocollex`, `mpbench`)

- **Date:** `2026-11-17`
- **Slice:** `IndonesianRegister` (`multilexnorm`, `indocollex`) and `Security` (`mpbench`) from [`standard_benchmark_registry()`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L233-L267).
- **In-tree exclusion note:** `tool_rag_scaling` ([`crates/lab/src/benchmark.rs#L251-L257`](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L251-L257)) uses `ds_local` (`requires_external_corpus: false`, `required_in_standard_suite: false`) and runs purely in-tree, so it is excluded from the 22 external datasets.
- **Compliance:** Zero reads of `lab/scenarios/sealed/`, zero dataset downloads into `lab/datasets/`, zero edits to code or OpenSpec files. All provenance, split, schema, and metric facts are verified directly from primary repositories, Hugging Face APIs, ACL Anthology PDFs, and arXiv HTML.

---

## 1. Executive Summary & Owner Choices Needed

### Headline Audit Verdict

| `dataset_id` | Public Existence (`R-51`) | Official Metric vs. Registry Metric (`R-52`) | Honest Percept Mapping (`R-53`) | Power `S >= 354–471` (`R-54`) | Recommended Manifest Status |
|---|---|---|---|---|---|
| **`multilexnorm`** | **Available** (`robvanderg/MultiLexNorm` HF + Bitbucket; two Indonesian configs: `ind` 4,308 & `ind_eng` 825) | Registry lists `retrieval_256d_acc` (**Lab adaptation**, NOT official). Official metric is **`ERR`** (Error Reduction Rate) + Accuracy. | **`not_mappable` for Core `Percept` `run_scenario`** (isolated tweets lack speakers, places, timestamps, and turns); **valid only as a Waist-2 Gateway Embedder probe**. | `ind` test (`861`) passes `S >= 471`; `ind_eng` test (`165`) is **underpowered** unless pooled (`825` total). | `availability = "not_mappable"` for Core scenarios (or scope explicitly as Embedder probe); **owner license + split choice needed**. |
| **`indocollex`** | **Available** (`haryoa/indo-collex`, `MIT`, 2,125 pairs) | Registry lists `retrieval_256d_acc` (**Lab adaptation**, NOT official). Official metrics are **Top-1 / Top-10 Accuracy** & **BLEU** (no official eval script). | **`not_mappable` for Core `Percept` `run_scenario`** (isolated word pairs, no sentences, speakers, places, timestamps, or actions); even `256d` sentence embedding on bare words is OOD. | `I->F` test (`213`) and `F->I` test (`192`) are **underpowered** (`< 354`); only `full.csv` (`2,125`) or `dict/` (`2,623`) reaches `>= 471`. | `availability = "not_mappable"` (recommended), or re-scope to Waist-2 Embedder word-pair probe. |
| **`mpbench`** | **Unavailable** (`arXiv:2606.04329v2` has no dataset link; `pritamdash/mp-bench` on HF is an empty `.gitattributes`-only shell) | Official metrics: **`ASR`**, **`RSR`** (conditional), **`FPR`** across `C1–C4` / 6 classes (LLM judge). Registry lists `compaction_poisoning_asr` (`C3`-only subset of `ASR`) and `delta_ni` (**MonoAgens `R-33` metric**, NOT official). | **Conditionally mappable** from Appendix D.2 schema (`user_query`, `context`, `retrieval_query` -> `Percept`s with synthetic virtual times & untrusted external `origin_actor`), **blocked** by lack of public files. | Paper dataset (`3,240` adversarial + `2,997` benign; `C3 = 600`) exceeds `S >= 471`; `C4` alone (`240`) is `< 354`. | **`availability = "unavailable"`** (per [`benchmark-datasets/spec.md`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md#L77-L83); never silently substitute). |

### Owner Choices Needed

1. **License clearance for `multilexnorm` (`R-51`):**
   - Neither `robvanderg/multilexnorm` (Bitbucket) nor `robvanderg/MultiLexNorm` (Hugging Face) ships a `LICENSE` file.
   - The W-NUT 2021 paper ([`2021.wnut-1.55`](https://aclanthology.org/2021.wnut-1.55.pdf), Appendix A, p. 509) states *"The data is released under a CC-BY-SA license"* (version unspecified), while the upstream `ind_eng` repository (`ir-nlp-csui/id-en-code-mixed`) has **no `LICENSE` file**, `SEACrowd/multilexnorm` conservatively tags `_LICENSE = "CC-BY-NC-SA 4.0"`, and `MultiLexNorm++` (`arXiv:2601.16623`, `CC BY 4.0` paper) added `ind` from the CIL Instagram corpus (Septiandri & Wibisono 2017; Kurnia & Yulianti 2020b) with no verified standalone SPDX tag.
   - **Options:** **(1)** Clear `multilexnorm` for internal/non-commercial Lab use (`CC-BY-SA` / `CC-BY-NC-SA` conservative reading), **(2)** Contact Rob van der Goot / Barik et al. / Kurnia & Yulianti for commercial clearance, or **(3)** Refuse `multilexnorm` if commercial-clean SPDX is strictly required.
2. **Which `multilexnorm` configuration(s) to pin (`ind` vs. `ind_eng` vs. both):**
   - `ind_eng` (825 tweets, W-NUT 2021) covers **Indonesian–English code-mixed Twitter text** (`165` test tweets — underpowered alone for `S >= 354–471`).
   - `ind` (4,308 utterances, MultiLexNorm++ 2026) covers **monolingual informal Indonesian Instagram comments** (`861` test utterances — meets `S >= 471` on `test` alone).
   - **Options:** **(1)** Pin both `ind` and `ind_eng` from `robvanderg/MultiLexNorm` HF (`~630 KiB` total) and stratify across both (`N_test = 1,026`), **(2)** Pin `ind_eng` only (original W-NUT 2021 code-mixed target, requiring pooling `train+val+test = 825` for zero-shot retrieval power), or **(3)** Pin `ind` only.
3. **What to do with non-conversational register/lexicon datasets (`multilexnorm`, `indocollex`) in `standard_benchmark_registry()` (`R-52` / `R-53`):**
   - Neither dataset is a multi-turn agent interaction; neither can be fed to `run_scenario` as `Percept`s without inventing all six domain facts (`actor`, `place`, `audience`, `timestamp_ms`, `capability`, `receipt`), and `indocollex` lacks even sentences (isolated word pairs only).
   - **Options:**
     - **(1) Honest `not_mappable` + Waist-2 Embedder Probe (Recommended):** Mark `indocollex` (and optionally `multilexnorm`) `not_mappable` for Core `Percept` `run_scenario` per [`benchmark-evaluation/spec.md#L38-L40`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-evaluation/spec.md#L38-L40), and evaluate `retrieval_256d_acc` strictly as a **Waist-2 Gateway `Embedder` component probe** (analogous to `tool_rag_scaling` being a component measurement), renaming or explicitly tagging `retrieval_256d_acc` as non-official (`twin_retrieval_r1_256d`) so it is never confused with official `ERR` or `Top-1 Acc`.
     - **(2) Synthetic wrapper harness:** Inject `norm` entries into `MemStore` with synthetic metadata and query with `orig` via Step-4 recall (`multilexnorm` only; `indocollex` would additionally require Lab-written sentence templates, confounding dataset signal with Lab prose).
4. **License for `indocollex` (`R-51`):**
   - Primary GitHub repo `haryoa/indo-collex` is cleanly **`MIT`** (allows local copies, commercial use, derived redistribution). Note that the community mirror `SEACrowd/indocollex` erroneously tags `CC BY-SA 4.0` — fetch from `haryoa/indo-collex` directly and record checksums via `--record-checksums`.
5. **Handling `mpbench` unavailability (`R-51` / `R-52`):**
   - `mpbench` (`arXiv:2606.04329v2`) has **no public dataset release** (`pritamdash/mp-bench` on HF is an empty `.gitattributes`-only repo as of `2026-11-17`).
   - Per [`benchmark-datasets/spec.md#L77-L83`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md#L77-L83), mark `mpbench` `availability = "unavailable"` in `lab/datasets.manifest.toml`. Note also that `delta_ni` in `benchmark.rs` is MonoAgens' own `R-33` non-interference parity metric (not an MPBench metric) and `compaction_poisoning_asr` covers only MPBench's `C3` channel (missing `C1/C2/C4` and `RSR`). If the owner later wants a public external memory-poisoning corpus, that would be a separate OpenSpec change (never silently substituting `mpbench`).

---

## 2. Detailed Per-Dataset Analysis

### 2.1 `multilexnorm` (`indonesian/multilexnorm`)

#### R-51 Provenance
- **Public existence & canonical name:** **Yes.** Canonical name: **MultiLexNorm** (W-NUT 2021 Shared Task on Multilingual Lexical Normalization; van der Goot et al., 2021, [ACL Anthology `2021.wnut-1.55`](https://aclanthology.org/2021.wnut-1.55/)), extended with five Asian languages including monolingual Indonesian (`ind`) in **MultiLexNorm++** (Buaphet et al., 2026, [`arXiv:2601.16623`](https://arxiv.org/abs/2601.16623), *ACM TALLIP* `10.1145/3812651`).
- **Canonical sources & pinned revisions:**
  1. **Hugging Face (Parquet mirror maintained by dataset author `robvanderg`):** [`https://huggingface.co/datasets/robvanderg/MultiLexNorm`](https://huggingface.co/datasets/robvanderg/MultiLexNorm)
     - **Pinned commit SHA:** `3490e1402a6616dd40ca1c01aae26f9067e6e359` (`2026-10-20T11:21:29.000Z`).
     - **Files, exact byte sizes, and source-published LFS SHA-256 checksums** (for the two Indonesian configs):
       - `ind/train-00000-of-00001.parquet`: `299,070` bytes — SHA-256 `7689823da7ab5fa8006d946d114eff0877f86038d03e465514c21c419ce819a9`
       - `ind/validation-00000-of-00001.parquet`: `46,484` bytes — SHA-256 `ddc9c1a703e5f2b95a7acdf7ab3712916349dbab695a3c7ce3ce5b8b03f3aadb`
       - `ind/test-00000-of-00001.parquet`: `89,750` bytes — SHA-256 `df40d6ae5730d64402b72b684ed0de8357cb6df11e570a7eaed5595adf16bc82`
       - `ind_eng/train-00000-of-00001.parquet`: `121,324` bytes — SHA-256 `513d3247f3c57569f9de56a63030d6d697faa99f2c85a87293365db8959dd2fd`
       - `ind_eng/validation-00000-of-00001.parquet`: `45,390` bytes — SHA-256 `c6e94756949c3a41979251426e2061f15f388f2ac5dcd0e799ac6eb05f641e86`
       - `ind_eng/test-00000-of-00001.parquet`: `43,422` bytes — SHA-256 `dd5c99e4959a8e1eabab22eb25605bbb2e1fe5a8b9270c2fcd397713871349e1`
     - *(Note: Hugging Face `cardData` also reports unpacked Arrow `num_bytes`: `ind` `697,988` / `85,344` / `179,636`; `ind_eng` `247,175` / `84,002` / `76,432`.)*
  2. **Bitbucket (Canonical raw `.norm` text + scoring script):** [`https://bitbucket.org/robvanderg/multilexnorm`](https://bitbucket.org/robvanderg/multilexnorm)
     - **Pinned commit SHA:** `e58e946375299a04355fa2d7fa9e1c769234e68a` (`2026-10-20T11:47:56+00:00`; no tags/releases).
     - **Files & byte sizes:** `data/ind/{train,dev,test}.norm` (`463,864` / `56,491` / `120,161` bytes), `data/ind_eng/{train,dev,test}.norm` (`160,016` / `53,993` / `49,081` bytes), `scripts/normEval.py` (`2,960` bytes).
     - **Checksums:** Bitbucket does **not** publish SHA-256 checksums; Hugging Face LFS **does** publish exact SHA-256 OIDs (`sha256` above).
- **Upstream sub-corpus provenance:**
  - `ind_eng` (`ID-EN` in W-NUT 2021): 825 Indonesian–English code-mixed tweets collected by Barik, Mahendra, & Adriani (2019, [`2019.wnut` / `D19-5554`](https://aclanthology.org/D19-5554/)), [`github.com/ir-nlp-csui/id-en-code-mixed`](https://github.com/ir-nlp-csui/id-en-code-mixed) (commit `c0616eb975ffd2de78d59a06dc3e4690d5a204ba`, `2022-10-16T07:32:50Z`).
  - `ind` (added in MultiLexNorm++ 2026): 4,308 Indonesian Instagram comments (`48,716` words in paper Table 1; `47.47%` tokens normalized, capitalization corrected, no 1-n/n-1 splits) adapted from the CIL dataset (Septiandri & Wibisono 2017; Kurnia & Yulianti 2020b).
- **License & access gate:**
  - **Access gate:** None (`gated: false`, public).
  - **License terms:** **Unsettled / conflicting metadata (owner clearance required).**
    - Neither Bitbucket `robvanderg/multilexnorm` nor HF `robvanderg/MultiLexNorm` includes a `LICENSE` file or SPDX metadata.
    - W-NUT 2021 paper (`2021.wnut-1.55`, Appendix A, p. 509) states: *"The data is released under a CC-BY-SA license"* (version unspecified; paper itself is `CC BY 4.0`).
    - Upstream `ir-nlp-csui/id-en-code-mixed` has **no `LICENSE` file** (`license: None`).
    - Community loader [`SEACrowd/multilexnorm`](https://huggingface.co/datasets/SEACrowd/multilexnorm) tags `_LICENSE = "CC-BY-NC-SA 4.0"` (non-commercial) and points to stale commit `e92e5b8f...` `data/iden/` (which no longer exists on `master` after renaming to `data/ind_eng/`).
    - `MultiLexNorm++` (`arXiv:2601.16623`) is `CC BY 4.0` for the paper; no standalone SPDX license for the underlying CIL Instagram subset is stated in Appendix A.
    - **Practical implication:** Local copies for research/benchmarking are standard under `CC-BY-SA`, derived files inherit ShareAlike, and commercial use is **blocked or ambiguous** unless the owner treats `NC` as absent or clears it with the maintainers.

#### R-52 Official Evaluation Protocol
- **Official primary metric:** **Error Reduction Rate (`ERR`)** (van der Goot 2019b; `2021.wnut-1.55` §4.1; `arXiv:2601.16623` §4):
  $$\text{ERR} = \frac{\%\text{accuracy} - \%\text{words\_not\_normed}}{100 - \%\text{words\_not\_normed}} = \frac{\text{Acc} - \text{LAI}}{1 - \text{LAI}}$$
  where $\text{LAI}$ (Leave-As-Is baseline) is the fraction of tokens where `orig == norm`. Reported on `[0, 100]%` (negative if system corrupts more tokens than it fixes). Secondary official metrics: token-level **Accuracy**, **LAI**, and (in MultiLexNorm++) **F1 / Precision / Recall**. Extrinsic `a-LAS` / `a-UAS` / `a-POS` (UD parsing/tagging) in W-NUT 2021 covered only `DE, EN, IT, TR` (not `ind` or `ind_eng`).
- **Official split:** `test` (`ind/test`: 861 sentences; `ind_eng/test`: 165 sentences).
- **Official scoring script:** [`scripts/normEval.py`](https://bitbucket.org/robvanderg/multilexnorm/raw/e58e946375299a04355fa2d7fa9e1c769234e68a/scripts/normEval.py) in `robvanderg/multilexnorm` at commit `e58e946375299a04355fa2d7fa9e1c769234e68a`. Deterministic Python; default **case-sensitive** (`ignCaps=False`), requires exact 1-to-1 sentence/token alignment (`len(gold) == len(pred)` and `len(sentGold) == len(sentPred)`).
- **LLM judge:** **None** (deterministic string comparison).
- **Known contamination:** **High** for `ind_eng` (public since Barik et al. 2019 / W-NUT 2021); **Moderate** for `ind` (CIL Instagram comments public since 2017/2020, merged into `robvanderg/MultiLexNorm` in 2026). Note: contamination mainly biases generative LLM normalization; it is a smaller confounder for frozen embedder geometry unless the embedder was fine-tuned on parallel social-media pairs.
- **Registry metric discrepancy (`retrieval_256d_acc`):**
  - **`retrieval_256d_acc` is NOT an official MultiLexNorm metric.** It was introduced in Run 2 (`A-15`, [`A14_A16_info_gap_indonesian_context.md#L256`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A14_A16_info_gap_indonesian_context.md#L256), [`#L281`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A14_A16_info_gap_indonesian_context.md#L281)) as a Lab probe measuring whether `EmbeddingGemma` truncated to `256d` (`TaggedEmbedding`) retrieves each noisy `orig` utterance's canonical `norm` twin (`Recall@1` / `MRR@1` over candidate pool).
  - Per [`benchmark-evaluation/spec.md#L51-L55`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-evaluation/spec.md#L51-L55), reporting `retrieval_256d_acc` under a distinct non-official name (and NOT calling it `ERR`) is required and already partially reflected in `benchmark.rs`. Official `ERR` can also be computed only if evaluating an actual normalizer (e.g., `reason` LLM normalization baseline), not a vector embedder.

#### R-53 Mapping to Percepts (Honest Evaluation)
- **What the dataset actually contains:** Each item is a pre-tokenized social media sentence pair `(orig: Vec<String>, norm: Vec<String>)` with **no speaker/author ID, no recipient/audience, no place/room ID, no timestamp, no reply-to thread structure, no capability invocation, and no dialogue-act or action label**.
- **Honest verdict:** **`multilexnorm` cannot be mapped to Core `Percept`s for `run_scenario` without inventing all six domain facts (`actor`, `place`, `audience`, `timestamp_ms`) and fabricating a conversational framing.** Moreover, `retrieval_256d_acc` tests **Gateway/Reason's `256d` `Embedder`** (`EmbeddingGemma 2` `256d` / `EmbeddingGemma-300M` `256d`, [`crates/protocol/src/message.rs#L106-L120`](file:///home/muqsith/Projects/Mono/crates/protocol/src/message.rs#L106-L120)), which sits in Waist 2 / `reason` *before* Core's opaque event loop.
- **What the soul would have to do if forced into `run_scenario`:** To evaluate `retrieval_256d_acc` through `Core` (`MemStore`), a converter would have to pre-populate `MemStore` with synthetic `Observed` memories carrying `norm` strings under invented `(actor, place, audience, timestamp_ms)` and then fire `orig` strings as `Percept`s to check whether Step-4 recall retrieves the matching `norm` memory ID in top-1. That tests Step-4 vector recall under **synthetic timestamps/actors** rather than natural agent perception.
- **Recommended framing:** Either mark `multilexnorm` `not_mappable` for Core `run_scenario` (`benchmark-evaluation/spec.md` L38–40) and run `retrieval_256d_acc` as a dedicated **Waist-2 Embedder Calibration Probe** (like `tool_rag_scaling`), OR document the synthetic-memory twin-retrieval harness explicitly as an embedder-recall test rather than a behavioral soul scenario.

#### R-54 Sampling and Power (`S >= 354–471`)
- **Split item counts (verified from HF `cardData`):**
  - `ind`: `train = 3,016`, `validation = 431`, `test = 861` (total `4,308` utterances).
  - `ind_eng`: `train = 495` (`13,949` words), `validation = 165`, `test = 165` (total `825` utterances).
- **Power assessment (`S >= 354–471` paired scenarios):**
  - `ind` `test` (`861` items) **exceeds** `S = 471` on `test` alone (`R = 1` deterministic embedder pass -> zero seed variance `rho_s`!).
  - `ind_eng` `test` (`165` items) is **underpowered** (`165 < 354`); even `validation + test` (`330`) is `< 354`. Only pooling all splits (`825` items — valid for zero-shot embedder retrieval since nothing is trained on `ind_eng`) reaches `S >= 471`.
  - Evaluating the **full `test` union** (`861 + 165 = 1,026` items) costs almost nothing (`~1–2s` local embedder inference) and satisfies `S >= 471` overall, though the `ind_eng` subgroup (`165`) remains underpowered on its own.
- **Stratification (if subsampling to `S = 471`):** Stratify by `(config ∈ {ind, ind_eng}, pct_tokens_changed ∈ {0%, (0,20%], >20%}, token_count_quartile)`; keep all `165` `ind_eng/test` items + `306` stratified `ind/test` items (or run all `1,026`).
- **Cost shape per item:**
  - `retrieval_256d_acc` (Embedder probe): **`0` LLM generation calls, `0` LLM judge calls, `2` local `256d` embedding calls per item** (`orig` query + `norm` candidate; `2,052` total embeddings for all `1,026` test items — `$0` API cost).
  - If ever run with an LLM normalizer under official `ERR`: `1` LLM call per sentence, `0` judge calls (`normEval.py` is deterministic).

---

### 2.2 `indocollex` (`indonesian/indocollex`)

#### R-51 Provenance
- **Public existence & canonical name:** **Yes.** Canonical name: **IndoCollex** (*"IndoCollex: A Testbed for Morphological Transformation of Indonesian Colloquial Words"*, Wibowo, Nityasya, Akyürek, Fitriany, Aji, Prasojo, & Wijaya, *Findings of ACL-IJCNLP 2021*, [`2021.findings-acl.280`](https://aclanthology.org/2021.findings-acl.280/), DOI `10.18653/v1/2021.findings-acl.280`).
- **Canonical source & pinned revision:**
  - **GitHub:** [`https://github.com/haryoa/indo-collex`](https://github.com/haryoa/indo-collex) (default branch `main`, no tags/releases).
  - **Pinned commit SHA:** `df626e812cd5924794fabf40003368a69f4e953c` (`2021-08-10T06:28:06Z`, commit `"Merge pull request #2 from haryoa/refactor/move-data"`).
- **Files needed & exact byte sizes (verified via GitHub git tree at `df626e81`):**
  - `data/full.csv`: `63,999` bytes (`2,126` lines = `2,125` rows; columns `no,transformed,original-for,transformation`)
  - `data/informal_to_formal/train.csv`: `44,799` bytes (`1,483` lines = `1,482` rows)
  - `data/informal_to_formal/dev.csv`: `5,336` bytes (`176` lines = `175` rows)
  - `data/informal_to_formal/test.csv`: `6,116` bytes (`214` lines = `213` rows)
  - `data/formal_to_informal/train.csv`: `49,674` bytes (`1,637` lines = `1,636` rows)
  - `data/formal_to_informal/dev.csv`: `5,499` bytes (`182` lines = `181` rows)
  - `data/formal_to_informal/test.csv`: `5,612` bytes (`193` lines = `192` rows)
  - `dict/inforformal-formal-Indonesian-dictionary.tsv`: `39,167` bytes (`2,624` lines = `2,623` entries; note upstream typo `inforformal`)
  - `LICENSE`: `1,080` bytes
  - Total data payload (`data/` + `dict/`): `~220.2 KiB`.
- **Checksums:** Source publishes **no SHA-256 checksums** (GitHub API returns git blob SHA-1 only). Owner must record SHA-256 on first fetch via `monoagens-lab datasets fetch --record-checksums`.
- **License & access gate:**
  - **License:** **`MIT`** (SPDX: `MIT`, verified directly in `haryoa/indo-collex/LICENSE`, `Copyright (c) 2021 Haryo Akbarianto Wibowo`). Allows local copies, commercial use, and redistribution of derived files provided the MIT copyright and permission notice are preserved.
  - **Access gate:** None (public GitHub repo).
  - *(Metadata warning: community wrapper `SEACrowd/indocollex` erroneously tags `_LICENSE = "CC BY-SA 4.0"`, contradicting the primary repository's `MIT` `LICENSE` file; fetch directly from `haryoa/indo-collex`.)*

#### R-52 Official Evaluation Protocol
- **Official metrics (Wibowo et al. 2021 §7.1 & Table 5):**
  - **Top-1 Accuracy** (exact match on target word), **Top-10 Accuracy** (beam search), and **BLEU** (character/subword level), evaluated separately on **`Informal -> Formal (I->F)`** and **`Formal -> Informal (F->I)`** on the `test` split (with `Loan words` and `Jargon` excluded from character-transduction splits per §4 / `README.md`).
  - Also evaluates word-formation tag classification accuracy (`Transformer(I->F)->T`, `Transformer(F->I)->T`).
- **Official split:** `test` (`data/informal_to_formal/test.csv` = `213` pairs; `data/formal_to_informal/test.csv` = `192` pairs). Note `data/README.md` warns that the GitHub CSVs were slightly updated after publication, so exact split counts differ slightly from paper Table 4 (`F->I`: `1,657 / 186 / 193`). Also note `data/informal_to_formal` contains single-step morphological transformations, so pairs can be `informal -> less_informal` (e.g., `temen2 -> teman2`), whereas `dict/` contains end-to-end phrase pairs.
- **Official scoring script:** **None published** in `haryoa/indo-collex` (repository contains data, dictionary, and guidelines only — no training or evaluation scripts). Must be reimplemented in Rust/Python directly from the paper specification (`exact match` + `BLEU`).
- **LLM judge:** **None** (deterministic string matching / BLEU).
- **Known contamination:** **Moderate-to-high** for generative LLMs (public GitHub repo since Aug 2021, ACL Anthology, SEACrowd/NusaCrowd); low direct effect on frozen embedder geometry unless trained on Indonesian social media slang lexicons.
- **Registry metric discrepancy (`retrieval_256d_acc`):**
  - **`retrieval_256d_acc` is NOT an official IndoCollex metric.** Like `multilexnorm`, it is a MonoAgens Lab adaptation (`A-15`, [`A14_A16_info_gap_indonesian_context.md#L257`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/A14_A16_info_gap_indonesian_context.md#L257)).
  - **Additional methodological mismatch:** IndoCollex contains **isolated words/short phrases** (`kmrn -> kemarin`, `vid -> video`, `bumil -> ibu hamil`), NOT sentences. Testing a sentence-embedding model (`EmbeddingGemma`) via bare word-to-word cosine retrieval probes subword morphology out-of-distribution; Run 2 (`A-15` L257) suggested substituting words into Lab-authored validation sentences, which evaluates Lab-invented templates rather than pure IndoCollex data.

#### R-53 Mapping to Percepts (Honest Evaluation)
- **What the dataset actually contains:** CSV rows `(no, transformed, original-for, transformation)` — isolated word pairs with a morphological category label. **Zero sentences, zero speakers, zero places, zero audiences, zero timestamps, zero actions, zero conversational turns.**
- **Honest verdict:** **`not_mappable` for Core `Percept` `run_scenario`.** Representing a bare word pair as a `Percept` would require fabricating all six required `Percept` facts (`actor`, `place`, `audience`, `timestamp_ms`, etc.) AND either sending ungrammatical single-word `OpaquePayload`s or inventing synthetic carrier sentences (which risks violating the spirit of zero invented facts in [`benchmark-evaluation/spec.md#L38-L40`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-evaluation/spec.md#L38-L40)).
- **If kept solely as a Waist-2 Embedder probe (Owner choice):** Keep `indocollex` marked `not_mappable` for Core `run_scenario`, and optionally score `retrieval_256d_acc` (renamed e.g. `word_twin_retrieval_r1_256d`) directly on the `Embedder` adapter, OR remove `indocollex` from the Core suite in favor of `multilexnorm` (`ind` + `ind_eng`) which has authentic full sentences.

#### R-54 Sampling and Power (`S >= 354–471`)
- **Split item counts (exact row counts at `df626e81`):**
  - `data/informal_to_formal`: `train = 1,482`, `dev = 175`, `test = 213` (total `1,870`).
  - `data/formal_to_informal`: `train = 1,636`, `dev = 181`, `test = 192` (total `2,009`).
  - `data/full.csv`: `2,125` pairs.
  - `dict/inforformal-formal-Indonesian-dictionary.tsv`: `2,623` pairs.
- **Power assessment (`S >= 354–471`):**
  - **Underpowered on official `test` splits:** Neither `informal_to_formal/test.csv` (`213`) nor `formal_to_informal/test.csv` (`192`) reaches `S >= 354`! Even pooling `dev + test` (`388` for `I->F`, `373` for `F->I`) reaches `S >= 354` ($R=2$ assumption) but falls short of `471` ($R=1$).
  - Only zero-shot evaluation on `data/full.csv` (`2,125` rows, stratified sample `S = 471`) or `dict/` (`2,623` rows) reaches `S >= 471`.
- **Stratification:** Stratify by `transformation` (`sound-alter`, `shorten`, `disemvoweling`, `affixation`, `space/dash removal`, `acronym`, `reverse`). Note `reverse` is tiny in `train` and oversampled in `dev/test`.
- **Cost shape per item:** `0` LLM judge calls, `2` local `256d` embedding calls per pair (`< 1s` total CPU/GPU for all `2,125` pairs; `$0` API cost).

---

### 2.3 `mpbench` (`mpbench`)

#### R-51 Provenance
- **Public existence & canonical name:** **NO — Unavailable publicly as of `2026-11-17`.**
  - Canonical paper: *"From Untrusted Input to Trusted Memory: A Systematic Study of Memory Poisoning Attacks in LLM Agents"* (Dash, Ge, Jain, Shah, & Shang, Huawei Canada / Univ. of Waterloo; [`arXiv:2606.04329v2`](https://arxiv.org/abs/2606.04329v2), v1 `2026-06-03`, v2 `2026-06-18`; accepted at ICML 2026 AIWILD Workshop).
- **Verification of non-availability (all primary channels checked):**
  1. **arXiv HTML (`2606.04329v2`):** Contains **zero** dataset or code repository URLs (the only GitHub/OpenReview URLs in the paper point to target agents `openclaw/openclaw`, `NousResearch/hermes-agent`, and prior paper `InjecMEM`).
  2. **Hugging Face (`pritamdash/mp-bench`):** [`https://huggingface.co/datasets/pritamdash/mp-bench`](https://huggingface.co/datasets/pritamdash/mp-bench) exists (commit `b0065dfd4f51b748a63420242f5ebb4f8be51d8f`, `lastModified: 2026-06-19T18:56:03.000Z`, `gated: false`, `private: false`), **but is an empty shell repository containing ONLY `.gitattributes` (`2,504` bytes)** — `README.md` returns HTTP 404 and zero dataset or license files are uploaded.
  3. **GitHub search & third-party replication check:** Zero official Dash et al. repositories on GitHub. Community replication `Zhao-Yan-Chen/context-load-attack` (`src/mpbench_adapter.py`) explicitly notes *"由于 MPBench 官方数据未公开发布"* ("Because the official MPBench dataset has not been publicly released...").
  4. **Name-collision disambiguation:** Three unrelated repositories share the name `MPBench` / `MP-Bench` and **MUST NOT** be confused with Dash et al.'s security benchmark:
     - `xuzhaopan/MPBench` (HF, Mar 2025): multimodal reasoning process-reward benchmark.
     - `Yeonjun/MP-Bench` (HF, May 2026, `arXiv:2603.25001`): multi-perspective failure attribution for multi-agent systems (Adobe Research License).
     - `a84374211/MPBench` (HF, Jun 2026): empty `.gitattributes`-only placeholder.
- **Files, checksums, license, access gate:**
  - **Files / sizes / checksums:** None published (`unavailable`).
  - **License:** Paper (`arXiv:2606.04329v2`) carries arXiv **`CC BY 4.0`**; **no dataset license** exists because no dataset has been released.
  - **Manifest status:** Must be `availability = "unavailable"` per [`benchmark-datasets/spec.md#L77-L83`](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md#L77-L83), remaining visible as a Security scorecard gap rather than being silently replaced.

#### R-52 Official Evaluation Protocol (from `arXiv:2606.04329v2` §4.1 & Appendix D)
- **Official metrics (`§4.1.3`):**
  1. **ASR (Attack Success Rate):** Fraction of `N = 3,240` adversarial test cases where an LLM judge determines at least one entry $e \in S'_i$ in the post-session persistent memory store encodes the same behavioral directive as target instruction $t_i$:
     $$\text{ASR} = \frac{1}{N}\sum_{i=1}^{N} \mathbf{1}\!\left[\exists\, e \in S'_i : \text{Judge}(e, t_i) = 1\right]$$
  2. **RSR (Retrieval Success Rate):** Computed **only over the $N'$ cases where $\text{ASR}=1$**, measuring whether the agent's follow-up response/action $a_i = \mathcal{A}(q_i, R(q_i))$ reflects $t_i$:
     $$\text{RSR} = \frac{1}{N'}\sum_{i=1}^{N'} \mathbf{1}\!\left[\text{Judge}(a_i, t_i) = 1\right]$$
  3. **FPR (False Positive Rate):** Evaluated on `2,997` benign examples for memory/input defense evaluation (`§4.1.2`, `§4.5`).
  4. **Taxonomy breakdown:** Reported across **4 write channels** (`C1` explicit, `C2` system-prompt inferred, `C3` compaction-driven, `C4` experience-to-procedure) and **6 attack classes** (`Explicit Command Insertion`, `Conditional Command Insertion`, `Salience-Driven Compaction Poisoning`, `Policy-Conformant Fact Injection`, `False Precedent Insertion`, `Skill-Procedure Insertion`).
- **Official split:** Single evaluation corpus (`3,240` adversarial + `2,997` benign; no train/test split).
- **Official scoring script & LLM judge:**
  - **No scoring script published.**
  - **Requires a separate LLM judge** for both `ASR` ($\text{Judge}(e, t_i)$) and `RSR` ($\text{Judge}(a_i, t_i)$). The paper validates judge agreement on a random manual sample (`§4.1.3`), **but does not publish the exact judge model identifier or verbatim judge prompt** in `arXiv:2606.04329v2` (tested target agents used `GPT-OSS-120B`).
- **Known contamination:** **Very low** (synthesized via `Meta-Llama-3.1-70B-Instruct` in 2026 and never publicly released).
- **Registry metric discrepancies (`compaction_poisoning_asr`, `delta_ni`):**
  - `compaction_poisoning_asr` is a **channel-scoped subset (`C3` = 600 cases)** of official `ASR`, omitting `C1/C2/C4` and conditional `RSR`.
  - **`delta_ni` is NOT an official MPBench metric.** $\Delta_{\text{NI}}$ is MonoAgens' own `R-33` deterministic non-interference metric ([`R33_R41_R43_privacy_roles_approvals_poisoning.md#L91-L95`](file:///home/muqsith/Projects/Mono/research/findings/run-2-notes/R33_R41_R43_privacy_roles_approvals_poisoning.md#L91-L95)), measuring bit-for-bit trace parity under `VisibleScope` audience projection (confidentiality), whereas MPBench measures persistent memory poisoning (integrity). If evaluated on MPBench-like scenarios, `delta_ni` must be clearly flagged as a MonoAgens companion metric, not an official MPBench score.

#### R-53 Mapping to Percepts (Schema-Level Assessment; Blocked by `unavailable`)
- **Paper schema (`arXiv:2606.04329v2` Appendix D.2):** Each test case is a JSON object with `attack_type` (6 values), `attack_signal` (`strong`/`moderate`/`weak`), `domain` (7 values: `workspace email`, `file and document`, `script execution`, `calendar and scheduling`, `Slack messaging`, `web browsing`, `skill invocation`), `adversarial_goal` (6 values), `user_query`, `context` (external data containing the payload, delivered either statically as labeled external content or dynamically via a tool file read), `expected_memory` ($t_i$), and `retrieval_query` ($q_i$).
- **Mappability if/when released:**
  - **Mappable with deterministic virtual-time/actor assignment (`TestBody`):**
    1. *Session 1 (Ingestion / Attack):* `user_query` -> `Percept` (`actor = Owner`, `place = domain_place`, `audience = {Owner, Agent}`, `taint = Trusted`, `timestamp_ms = t_0`); `context` -> `Percept` or `TestBody` tool observation (`actor = external_untrusted_source`, `taint = Untrusted`, `timestamp_ms = t_1 > t_0`). For `Conditional Command Insertion`, a deterministic follow-up `Owner` affirmative turn (`"yes"/"ok"`) is emitted at `t_1b`.
    2. *Compaction step (`C3`):* Virtual clock advances / triggers `sleep-consolidation` before Session 2.
    3. *Session 2 (Retrieval / Exploitation):* `retrieval_query` -> `Percept` (`actor = Owner`, `taint = Trusted`, `timestamp_ms = t_2 > t_1`).
  - **Gold isolation:** `expected_memory`, `attack_type`, `attack_signal`, and `adversarial_goal` reside strictly in `GoldBook` and never enter any `Percept` or `ContextPlan`.
  - **Gaps/caveats:** `MPBench` lacks real timestamps, real speaker graphs, and executable environments for `dynamic` tool-call cases (which require mock `TestBody` capabilities returning `context` in a `Receipt`). Most importantly, **because the dataset is not publicly released, no converter can be verified or run today**.

#### R-54 Sampling and Power (`S >= 354–471`)
- **Paper dataset size (`Table 7`):** `3,240` adversarial cases (`Explicit Command = 600`, `Conditional Command = 600`, `Salience-Driven Compaction C3 = 600`, `Policy-Conformant Fact = 600`, `False Precedent = 600`, `Skill-Procedure C4 = 240`) + `2,997` benign controls (`6,237` total).
- **Power assessment:**
  - Overall (`3,240` adversarial) and `C3`-only (`600`) **exceed** `S >= 354–471`.
  - `C4` (`240`) alone is `< 354` and underpowered as a standalone paired NI slice.
- **Stratification (if released):** Seeded subset of `S = 471` (or `S = 540` = `90` per attack class × `6` classes, balanced across `7` domains and `2` delivery modes) plus matched `471` benign controls for FPR.
- **Cost shape per item (when/if live):** ~`3–5` agent LLM calls (`Session 1` turn(s) + optional `sleep-consolidation` LLM compaction + `Session 2` `retrieval_query` turn) + **`2` LLM judge calls** (`Judge(e, t_i)` for ASR; `Judge(a_i, t_i)` for RSR when ASR=1) = ~`5–7` LLM calls/item (`~2,355–3,300` calls at `S = 471`).

---

## 3. Consolidated Summary Tables (`R-51`–`R-54`)

### Table 1 — `R-51` Provenance

| `dataset_id` | Publicly Exists? | Canonical Name | Canonical Source & Pinned Revision (Date) | Files & Exact Sizes | Published Checksums? | License (SPDX) & Terms / Access Gate |
|---|---|---|---|---|---|---|
| `multilexnorm` | **Yes** | MultiLexNorm (W-NUT 2021) + MultiLexNorm++ (`ind`) | **HF (recommended):** `robvanderg/MultiLexNorm` `@` `3490e1402a6616dd40ca1c01aae26f9067e6e359` (`2026-10-20`)<br>**Bitbucket:** `robvanderg/multilexnorm` `@` `e58e946375299a04355fa2d7fa9e1c769234e68a` (`2026-10-20`) | **HF Parquet (`645,440` B):** `ind/{train,val,test}` (`299,070` / `46,484` / `89,750` B); `ind_eng/{train,val,test}` (`121,324` / `45,390` / `43,422` B).<br>**BB `.norm`:** `data/ind/{train,dev,test}.norm` (`463,864` / `56,491` / `120,161` B); `data/ind_eng/{train,dev,test}.norm` (`160,016` / `53,993` / `49,081` B); `scripts/normEval.py` (`2,960` B). | **Yes on HF** (LFS SHA-256 OIDs); **No on Bitbucket**. | **Ambiguous (`pending` owner clearance).** No `LICENSE` file in Bitbucket/HF; W-NUT 2021 paper states `CC-BY-SA` (version unspecified); `ir-nlp-csui/id-en-code-mixed` has no `LICENSE`; `SEACrowd` tags `CC-BY-NC-SA 4.0`. No access gate. |
| `indocollex` | **Yes** | IndoCollex (ACL-IJCNLP 2021 Findings) | **GitHub:** `haryoa/indo-collex` `@` `df626e812cd5924794fabf40003368a69f4e953c` (`2021-08-10`) | `data/full.csv` (`63,999` B); `data/informal_to_formal/{train,dev,test}.csv` (`44,799` / `5,336` / `6,116` B); `data/formal_to_informal/{train,dev,test}.csv` (`49,674` / `5,499` / `5,612` B); `dict/inforformal-formal-Indonesian-dictionary.tsv` (`39,167` B). | **No** (Git blob SHA-1 only; use `--record-checksums`). | **`MIT`** (verified in `haryoa/indo-collex/LICENSE`; `SEACrowd` mislabels `CC BY-SA 4.0`). Allows local copies, commercial use, derived redistribution w/ notice. No access gate. |
| `mpbench` | **No (`unavailable`)** | MPBench (`arXiv:2606.04329v2`, Dash et al. 2026) | Paper: `arxiv.org/abs/2606.04329v2` (`2026-06-18`).<br>HF `pritamdash/mp-bench` `@` `b0065dfd4f51b748a63420242f5ebb4f8be51d8f` (`2026-06-19`) is an **empty shell** (`.gitattributes` `2,504` B only). | **None published** (`0` data files). | **N/A** (`unavailable`). | Paper is `CC BY 4.0`; **no dataset license** (unreleased). Unrelated collisions: `xuzhaopan/MPBench`, `Yeonjun/MP-Bench` (Adobe Research License). |

### Table 2 — `R-52` Official Evaluation Protocol

| `dataset_id` | Official Primary Metric(s) | Official Split | Official Scoring Script & Revision | LLM Judge Required? | Known Contamination | Registry Metrics vs. Official Protocol |
|---|---|---|---|---|---|---|
| `multilexnorm` | **`ERR`** (Error Reduction Rate `= (Acc - LAI) / (1 - LAI)`), Token **Acc**, **LAI** (`F1/P/R` in MultiLexNorm++). | `test` | `scripts/normEval.py` in `robvanderg/multilexnorm` `@` `e58e946375299a04355fa2d7fa9e1c769234e68a` (case-sensitive default). | **No** (deterministic Python). | **High** (`ind_eng`, public since 2019/2021); **Moderate** (`ind`, CIL 2017/2020, merged 2026). | `retrieval_256d_acc` is **NOT official** (Lab `256d` `orig->norm` cosine retrieval probe; must be reported under distinct non-official name per spec). |
| `indocollex` | **Top-1 Acc** (Exact Match), **Top-10 Acc**, **BLEU** (`I->F` & `F->I`, excluding loan/jargon). | `test` (`informal_to_formal/test.csv`, `formal_to_informal/test.csv`) | **None published** in `haryoa/indo-collex` (must implement from ACL-IJCNLP 2021 paper). | **No** (deterministic string/BLEU). | **Moderate–High** (public GitHub + ACL Anthology since Aug 2021). | `retrieval_256d_acc` is **NOT official** (Lab `256d` twin-retrieval adaptation; bare words are OOD for sentence embedders). |
| `mpbench` | **`ASR`** (write poisoning rate), **`RSR`** (conditional retrieval action rate), **`FPR`** (`2,997` benign), across `C1–C4` / 6 classes. | Full eval set (`3,240` adv + `2,997` benign) | **None published.** | **Yes** — separate LLM judge for `ASR` and `RSR` (exact judge model & prompt not published in `arXiv:2606.04329v2`). | **Very Low** (synthetic `Llama-3.1-70B` JSONs, unreleased). | `compaction_poisoning_asr` = `C3`-only subset (`600` items) of official `ASR` (omits `RSR`). `delta_ni` is **NOT official** (MonoAgens `R-33` non-interference metric). |

### Table 3 — `R-53` Mapping to Percepts

| `dataset_id` | Raw Dataset Units | Missing Required Percept Facts | Gold Separation Rule | Honest Mapping Verdict (`R-53`) |
|---|---|---|---|---|
| `multilexnorm` | Pre-tokenized tweet/IG pairs `(orig: Vec<String>, norm: Vec<String>)`. | **All structural context:** `actor`, `place`, `audience`, `timestamp_ms`, `reply_to`, `capability`, `receipt`, multi-turn history. | `norm` tokens must stay in `GoldBook` (never in query `Percept`). | **`not_mappable` for Core `run_scenario`** (cannot represent as behavioral agent events without inventing all 6 facts); valid **only as a Waist-2 Gateway `Embedder` probe**. |
| `indocollex` | Isolated word-pair CSV rows `(no, transformed, original-for, transformation)`. | **Sentences AND all 6 facts:** no sentences/utterances, `actor`, `place`, `audience`, `timestamp_ms`, `capability`, or `receipt`. | Target word (`original-for` / `transformed`) stays in `GoldBook`. | **`not_mappable`** (isolated word pairs require inventing both wrapper sentences and all 6 facts; at most a bare Waist-2 `Embedder` lexicon check). |
| `mpbench` | Schema only (`D.2`): `(attack_type, attack_signal, domain, adversarial_goal, user_query, context, expected_memory, retrieval_query)`. | `timestamp_ms` and multi-party `actor`/`audience` IDs (must assign deterministic virtual times `t_0 < t_1 < t_compaction < t_2` and `Owner`/`ExternalSource`); `dynamic` cases need mock tool `Receipt`. | `expected_memory`, `attack_type`, `attack_signal`, `adversarial_goal` strictly in `GoldBook`. | **Conditionally mappable** into 2-session `TestBody` scenarios + sleep compaction, **currently blocked (`unavailable`)** because zero dataset files are publicly released. |

### Table 4 — `R-54` Sampling and Power (`S >= 354–471`)

| `dataset_id` | Item Counts per Relevant Split | Can Reach `S >= 354–471` Without Bias? | Recommended Stratification | Rough Per-Item Call / Cost Shape |
|---|---|---|---|---|
| `multilexnorm` | `ind`: `3,016` train / `431` val / **`861` test**.<br>`ind_eng`: `495` train / `165` val / **`165` test** (`825` total). | **`ind/test` (`861`): Yes.**<br>**`ind_eng/test` (`165`): No** (`< 354` unless pooling all `825` splits for zero-shot embedder eval). | Stratify by `(config ∈ {ind, ind_eng}, %_changed_tokens, length_quartile)`; or run all `1,026` test items (`861 + 165`). | **Embedder probe:** `2` local `256d` embeddings/item, `0` LLM calls, `0` judge calls (`$0` API cost). |
| `indocollex` | `I->F`: `1,482` train / `175` dev / **`213` test**.<br>`F->I`: `1,636` train / `181` dev / **`192` test**.<br>`full.csv`: **`2,125`**; `dict/`: `2,623`. | **No on `test` alone** (`213` and `192` are `< 354`). **Yes** only if evaluating zero-shot on `full.csv` (`2,125`) or `dict/` (`2,623`). | Stratify by `transformation` (7 classes; preserve rare `reverse` stratum). | **Embedder probe:** `2` local `256d` embeddings/pair, `0` LLM calls, `0` judge calls (`$0` API cost). |
| `mpbench` | Paper `Table 7`: **`3,240` adversarial** (`600` each for Explicit, Conditional, `C3` Compaction, Policy-Fact, False Precedent; `240` `C4` Skill-Procedure) + **`2,997` benign**. **Public files: `0`.** | **Yes** on full set (`3,240`) or `C3`-only (`600`); **`C4` alone (`240`) is `< 354`**. Blocked until public release. | Seeded `S = 471` (or `540` = `90 × 6` classes) stratified by `attack_type × domain × delivery_mode`, plus `471` benign controls. | ~`3–5` agent LLM turns (Session 1 + compaction + Session 2) + **`2` LLM judge calls** (`ASR` + `RSR`) = ~`5–7` LLM calls/item. |
