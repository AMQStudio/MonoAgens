# MonoAgens Research Brief 2 — Part A: A-14, A-15, and A-16

**Date:** 2026-11-16  
**Scope:** Part A research answers for **A-14 (`info_gap` / R-06)**, **A-15 (Indonesian, English & ID-EN Code-Mixed Embeddings)**, and **A-16 (Rebuilt Context on Multi-Party Tasks / R-11)**, strictly conforming to [researcher-prompt-2.md](file:///home/muqsith/Projects/Mono/research/researcher-prompt-2.md) and the architectural invariants of [ARCHITECTURE.md](file:///home/muqsith/Projects/Mono/ARCHITECTURE.md).

---

## Executive Summary: Contradictions and Gaps Found in `core-and-lab` and `RESEARCH.md`

Direct literature verification and statistical power analysis identified **five concrete issues** in the current [core-and-lab design](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md), [lab specification](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md), and [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md):

1. **Conceptual conflation in `RESEARCH.md` §2 (`info_gap`) — contradicts psychology vs. pragmatic dialogue-act recognition:** [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L29-L32) defines `info_gap` as detecting an incoming event's similarity to question/request reference sentences and cites Loewenstein (1994). Loewenstein's (1994) information-gap theory (`[PR]`, DOI `10.1037/0033-2909.116.1.75`) models the agent's *own* epistemic discrepancy (already separated in MonoAgens under `curiosity` in §24 / R-31), whereas detecting that *another participant* is asking a question or seeking help is **dialogue-act recognition** (ISO 24617-2, MIDAS, Coarse Discourse). The gate signal should be renamed/scoped explicitly to `dialogue_act_gap` (external request/question detection) and kept strictly distinct from internal `own_info_gap` (computed from numeric `feeling_of_knowing` and expectation `surprise` without an LLM call).
2. **Positive-only `max_r cos(v, r)` scoring & `design.md` Decision 5 contract defect:** [RESEARCH.md](file:///home/muqsith/Projects/Mono/RESEARCH.md#L30) uses positive-only `max_r cos(v, r)` over `references.json`, and [design.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L164) wires `reference vectors for info_gap` from `reason` to `core`. Positive-only cosine max lacks a negative/contrastive boundary (non-request statements sharing topic nouns score artificially high). Replacing positive reference sets with a **logistic linear head** $\sigma(w^T v + b)$ (or normalized contrastive prototype difference $(\mu_+ - \mu_-)^T v$) supplied as calibrated weight vector(s) from `reason` keeps `core` strictly content-blind and word-list-free while improving class separation at $O(d)$ cost.
3. **`core-and-lab` Decision 14 omits EmbeddingGemma-300M task instruction prefixes & 256d Indonesian ablation:** [design.md Decision 14](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L365-L379) adopts **EmbeddingGemma-300M truncated to 256d**, yet omits two verified operational requirements: (a) EmbeddingGemma (`arXiv:2509.20354` & [Google Model Card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card)) is trained with task-specific prefixes (`task: classification | query: {text}` vs. `task: search result | query: {text}` vs. `title: none | text: {text}`), meaning a single shared unprefixed vector cannot be assumed optimal across both retrieval and classification; and (b) neither `arXiv:2509.20354` nor `embeddings-benchmark/results` publishes per-Indonesian-task scores at 256d Matryoshka truncation (globally on `MTEB(Multilingual, v2)`, 256d drops `-1.5` Task pts overall and `-3.7` pts on Retrieval), nor does any public benchmark test *bahasa gaul* or ID-EN code-mixed text.
4. **Cross-platform/model throughput & license asymmetry in Decision 14:** Directly verified throughput numbers (`arXiv:2605.13521`, Table 9/11) show `embeddinggemma-300m` (`1,349 docs/s`) is slower than `multilingual-e5-small` (`2,604 docs/s`) and `granite-embedding-97m-multilingual-r2` (`2,534 docs/s`), and uses **Gemma Terms of Use** rather than Apache-2.0 — whereas `granite-311m-multi-r2` (Apache-2.0) drops `-22.95` pts on `MassiveIntentClassification (id)` (`54.40` vs. `77.35`), proving why Lab cannot swap embedders purely by multilingual retrieval averages.
5. **Underpowered 5-seed rule & missing non-inferiority protocol in `specs/lab/spec.md`:** [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L90-L94) mandates *"at least five seeds"* and [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L223-L226) requires full-transcript vs. rebuilt-context comparison without specifying a non-inferiority margin $\Delta_{\text{NI}}$ or scenario count $S$. Because repeated seeds on the same scenario correlate ($\text{ICC} = \rho_s \in [0.3, 0.7]$, Design Effect $\text{DE} = 1 + (R-1)\rho_s \in [2.2, 3.8]$), running $R=5$ seeds across a small scenario suite (e.g., 15–30 scenarios) cannot establish non-inferiority at $\Delta_{\text{NI}} = 0.05$ (`N_eff = 471` paired scenarios required at $\alpha = 0.025, 1-\beta = 0.80, p_{\text{disc}} = 0.15$). Furthermore, "within seed noise" under low $S$ risks conflating low statistical power with "no measured effect" (Piaggio et al., 2012 `[PR]`).

---

# A-14 info_gap (R-06): Concept Split, Verified Dialogue-Act Corpora, and Non-Word-List Gate Scoring

**Status:** partly answered

## Answer

`info_gap` in [RESEARCH.md §2](file:///home/muqsith/Projects/Mono/RESEARCH.md#L29-L32) MUST be split into two distinct, non-conflated mechanisms across MonoAgens: (1) an external **dialogue-act request detector (`dialogue_act_gap`)** in the wake gate that estimates the probability $p_{\text{req}}(v) \in [0, 1]$ that an incoming event is an information-seeking question, directive, or help request from another participant (dialogue-act recognition; Stolcke et al. 2000; Bunt et al. 2019; Yu & Yu 2021), and (2) an internal **epistemic drive (`own_info_gap`)** belonging to §24 / R-31 (`curiosity`) that models the soul's *own* subjective knowledge discrepancy (Loewenstein 1994; Kang et al. 2009; Gottlieb et al. 2013; Kidd & Hayden 2015) without reading content or calling an LLM, computed in `core` solely from existing scalar appraisals (`feeling_of_knowing`, `relevance`) and expectation `surprise`. In the gate (`dialogue_act_gap`), positive-only `max_r cos(v, r)` fails because topic overlap makes non-request declaratives mimic requests; instead, `reason` SHOULD supply frozen normalized linear-probe weights $(w, b)$ (or normalized contrastive centroids $\mu_+, \mu_-$) learned offline from verified bilingual dialogue-act/intent corpora (**MIDAS** `[PR]` Apache-2.0, **Reddit Coarse Discourse** `[PR]` CC-BY, **MASSIVE `id-ID` + `en-US`** `[PR]` CC BY 4.0, and **xSID `id` + `en`** `[PR]` CC BY-SA 4.0) over `EmbeddingGemma-300M` (`256d`) vectors so `core` computes a single deterministic $O(d)$ sigmoid-dot product with zero word lists and zero LLM inference.

**Formulas (both mechanisms, every symbol defined):**

1. **External Gate Signal (`dialogue_act_gap`, replacing positive-only `info_gap` in the wake gate):**
   Let $v \in \mathbb{R}^d$ ($d = 256$) be the L2-normalized embedding of the incoming event produced by the embedder in `reason` using the classification prefix (or unit fallback vector if embedder is absent). Let $(w, b) \in \mathbb{R}^d \times \mathbb{R}$ be frozen logistic linear-probe parameters supplied at initialization/config by `reason`:
   $$
   \text{dialogue\_act\_gap}(v) = p_{\text{req}}(v) = \sigma(w^T v + b) = \frac{1}{1 + \exp\!\bigl(-(w^T v + b)\bigr)} \in (0, 1)
   $$
   *Special case — Zero-shot / Low-shot Contrastive Prototype Form (when trained only from labeled centroid examples without SGD logistic regression):*
   $$
   \mu_+ = \frac{\sum_{i \in \mathcal{D}_+} v_i}{\left\|\sum_{i \in \mathcal{D}_+} v_i\right\|_2}, \qquad
   \mu_- = \frac{\sum_{j \in \mathcal{D}_-} v_j}{\left\|\sum_{j \in \mathcal{D}_-} v_j\right\|_2}, \qquad
   w_{\text{proto}} = \frac{\mu_+ - \mu_-}{\tau_{\text{proto}}}, \quad b_{\text{proto}} = 0
   $$
   where $\mathcal{D}_+$ contains information-seeking acts (`factual/opinion/yes-no question`, `task command`, `help/clarification request`), $\mathcal{D}_-$ contains non-request acts (`statement`, `elaboration`, `backchannel`, `agreement`, `announcement`), and $\tau_{\text{proto}} > 0$ is a temperature scaling sharpness (algebraically identical to $\sigma(w^T v + b)$ inside `core`, requiring only two opaque float payloads `(w, b)` passed through the existing `reason -> core` boundary in [design.md Decision 5](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L164)).  
   *Body-generality rule:* In chat bodies, $v$ comes from the text embedder. In non-text/embodied bodies, any normalized modality encoder (e.g., gaze-to-object + beckon/alert feature vector) can supply $(v; w, b)$ through the identical interface, or supply a neutral scalar $p_{\text{req}} = 0.0$ if unsupported.

2. **Internal Epistemic Drive (`own_info_gap` in §24 / R-31, computed in `core` with NO LLM call and NO content inspection):**
   Empirically (Loewenstein 1994 `[PR]`; Kang et al. 2009 `[PR]`; Gottlieb et al. 2013 `[PR]`), intrinsic curiosity follows an **inverted-U** function of partial knowledge/confidence (peaking at intermediate `feeling_of_knowing` where an organism knows enough to recognize a missing piece, plus expected prediction error reduction). Using only opaque scalar appraisals already defined in [appraisal-gate/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/appraisal-gate/spec.md) and [expectations/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/expectations/spec.md):
   $$
   \text{own\_info\_gap}(e, g, \text{state}) = \min\Bigl(1,\; \lambda_{\text{fok}} \cdot \text{rel}(e, g) \cdot 4\,\text{FoK}(v)\bigl(1 - \text{FoK}(v)\bigr) \;+\; \lambda_{\text{surp}} \cdot \bar{S}_{\text{exp}}\Bigr)
   $$
   where:
   - $\text{FoK}(v) \in [0, 1]$ is the existing scalar `feeling_of_knowing` (`mean` or `max` similarity to recalled memory vectors), so $4\,\text{FoK}(1-\text{FoK}) \in [0, 1]$ attains its peak $1.0$ at $\text{FoK} = 0.5$ and vanishes at $\text{FoK} = 0$ (completely alien noise with no schema anchor) and $\text{FoK} = 1$ (already known);
   - $\text{rel}(e, g) \in [0, 1]$ is the existing scalar `relevance` between event vector $v$ and active commitments/goal vector (preventing idle curiosity about irrelevant background chatter; Gottlieb et al. 2013 `[PR]` relevance-gated information-seeking);
   - $\bar{S}_{\text{exp}} \in [0, 1]$ is the bounded recent expectation prediction-error signal (`surprise` scalar from R-13, when an active commitment/prediction misses);
   - $\lambda_{\text{fok}} \ge 0$ and $\lambda_{\text{surp}} \ge 0$ are drive weights (neutral values $\lambda_{\text{fok}} = \lambda_{\text{surp}} = 0$).

---

### Survey of Verified Dialogue-Act & Intent Corpora

#### A. English & Multilingual Dialogue-Act / Multi-Party Corpora (Verified Sizes, Label Sets, URLs, Licenses)

| Corpus | Verified Size | Label Set | Verified URL(s) | Verified License & Caveats |
|---|---|---|---|---|
| **SwDA** (Stolcke et al. 2000 `[PR]`) | **1,155 conversations**, **205,000 utterances** in paper (`213,543` rows in `cgpotts/swda` HF export) | **42 DAMSL tags** (`sd`, `b`, `sv`, `qy`, `qw`, `ad`, `qh`, etc.) | [aclanthology.org/J00-3003](https://aclanthology.org/J00-3003/), [github.com/cgpotts/swda](https://github.com/cgpotts/swda), [huggingface.co/datasets/cgpotts/swda](https://huggingface.co/datasets/cgpotts/swda), [LDC97S62](https://catalog.ldc.upenn.edu/LDC97S62) | HF mirror `CC BY-NC-SA 3.0`; wrapper scripts `GPL-2.0`; underlying audio `LDC97S62` requires LDC user agreement. **Non-commercial** (`NC`). Dyadic spoken telephone. |
| **MRDA** (Shriberg et al. 2004 `[PR]`) | **75 meetings** (~72h speech), **>180,000 DA tags** (multi-party, overlapping speech) | **11 general tags + 39 specific tags** (+2 disruption, +2 indecipherable; often collapsed to 5 basic acts) | [aclanthology.org/W04-2319](https://aclanthology.org/W04-2319/), [groups.inf.ed.ac.uk/ami/icsi/download](https://groups.inf.ed.ac.uk/ami/icsi/download/), [github.com/NathanDuran/MRDA-Corpus](https://github.com/NathanDuran/MRDA-Corpus) | Processing utilities `GPL-3.0`; ICSI core NXT annotations downloadable directly from AMI/ICSI mirror; explicit commercial license text for original ICSI audio/transcripts could not be verified (`license.html` returned HTTP 404). |
| **DailyDialog** (Li et al. 2017 `[PR]`) | **13,118 dialogues** (`11,118` train / `1,000` val / `1,000` test verified on HF; ~8 turns/dial) | **4 communicative DAs** (`inform`, `question`, `directive`, `commissive`) + 7 emotions | [aclanthology.org/I17-1099](https://aclanthology.org/I17-1099/), [arXiv:1710.03957](https://arxiv.org/abs/1710.03957), [huggingface.co/datasets/li2017dailydialog/daily_dialog](https://huggingface.co/datasets/li2017dailydialog/daily_dialog) | **`CC BY-NC-SA 4.0`** (verified on HF card). Dyadic English-learner textbook style; **non-commercial (`NC`)**. |
| **ISO 24617-2 / DialogBank** (Bunt et al. 2019 `[PR]`) | Interoperable gold subcorpora across English (`HCRC Map Task`, `Switchboard`, `TRAINS`, `DBOX`), Dutch, Polish | **9 orthogonal dimensions** + **general & dimension-specific communicative functions** (`Info-seeking`, `Directive`, `Auto-Feedback`, etc.) | [dialogbank.lsv.uni-saarland.de](https://dialogbank.lsv.uni-saarland.de/), [DOI:10.1007/s10579-018-9436-9](https://link.springer.com/article/10.1007/s10579-018-9436-9) | Paper open access **`CC BY 4.0`**; DialogBank site did not expose a single unified license header (`license` subpage 404) and subcorpora inherit source corpus terms (useful as taxonomy reference standard, small scale for training). |
| **MIDAS** (Yu & Yu 2021 `[PR]`) | **468 conversations**, **~24,000 sentences** (`12,900` user utterances; $\kappa = 0.94$) | **23 hierarchical tags**: 13 Semantic (`factual/opinion/yes-no question`, `task/invalid command`, `complaint`, `statement`, etc.) + 10 Functional (`abandon`, `hold`, `back-channeling`, etc.) | [aclanthology.org/2021.eacl-main.94](https://aclanthology.org/2021.eacl-main.94/), [arXiv:1908.10023](https://arxiv.org/abs/1908.10023), [github.com/DianDYu/MIDAS_dialog_act](https://github.com/DianDYu/MIDAS_dialog_act) | **`Apache-2.0`** (verified on GitHub). Best permissive English source distinguishing question subtypes and implicit `task command` (`"can i ask you..."`). Human-machine dyadic. |
| **Reddit Coarse Discourse** (Zhang et al. 2017 `[PR]`) | **9,483 threads, 116,357 posts** (verified directly in raw `coarse_discourse_dataset.json`) | **10 verified `main_type` labels**: `question`, `answer`, `announcement`, `elaboration`, `agreement`, `disagreement`, `appreciation`, `humor`, `negativereaction`, `other` | [github.com/google-research-datasets/coarse-discourse](https://github.com/google-research-datasets/coarse-discourse) | **`CC-BY`** (verified in README). Note: JSON distributes post IDs + Reddit join script (subject to Reddit data retrieval). **Best permissive multi-party written asynchronous discussion source** separating `question` vs. `elaboration`/`announcement`. |
| **Ubuntu Dialogue Corpus** (Lowe et al. 2015 `[PR]`) | ~930K multi-turn IRC logs | Unlabeled for DAs (next-utterance response selection) | [aclanthology.org/W15-4640](https://aclanthology.org/W15-4640/), [github.com/rkadlec/ubuntu-ranking-dataset-creator](https://github.com/rkadlec/ubuntu-ranking-dataset-creator) | Generator code `Apache-2.0`. Multi-party chat text, **no DA tags**. |
| **Kummerfeld IRC Disentanglement** (Kummerfeld et al. 2019 `[PR]`) | **77,563 messages** | Manual reply-to-parent disentanglement graphs (**no DA tags**) | [aclanthology.org/P19-1374](https://aclanthology.org/P19-1374/), [github.com/jkkummerfeld/irc-disentanglement](https://github.com/jkkummerfeld/irc-disentanglement), [huggingface.co/datasets/jkkummerfeld/irc_disentangle](https://huggingface.co/datasets/jkkummerfeld/irc_disentangle) | **`CC BY 4.0`** on HF card (GitHub root `LICENSE` 404). Useful for reply-chain topology / Lab multi-party evaluation, **not DA labels**. |
| **Molweni** (Li et al. 2020 `[PR]`) | **10,000 dialogues**, **88,303 utterances** (avg 3.52 speakers/dial), **78,245 discourse relations**, **30,046 QA pairs** (`25,759` ans / `4,287` unans) | **16 SDRT discourse relations** (incl. `QAP` Question-Answer Pair) + extractive QA questions | [aclanthology.org/2020.coling-main.238](https://aclanthology.org/2020.coling-main.238/), [github.com/HIT-SCIR/Molweni](https://github.com/HIT-SCIR/Molweni) | Paper **`CC BY 4.0`**; GitHub API license `NOASSERTION` (no explicit `LICENSE` file in repo root). Derived from Ubuntu IRC; authentic multi-party `question -> answer` pairs. |
| **Discord-Dialogues** `[IND]` | ~7.3M conversation examples | **Unannotated** (raw ChatML multi-turn conversations) | [huggingface.co/datasets/mookiezi/Discord-Dialogues](https://huggingface.co/datasets/mookiezi/Discord-Dialogues) | **`Apache-2.0`** on HF. **Verification gap:** No public Discord corpus with human-annotated dialogue acts or help-request labels could be verified. |

#### B. Indonesian & Code-Mixed ID-EN Intent / Dialogue / Colloquial Corpora (Verified Sizes, Label Sets, URLs, Licenses)

| Corpus | Verified Size | Label Set | Verified URL(s) | Verified License & Scope Notes |
|---|---|---|---|---|
| **MASSIVE (`id-ID` + 50/51 langs)** (FitzGerald et al. 2023 `[PR]`) | **19,521 parallel utterances** in Indonesian (`id-ID`) (total 1M across 51/52 langs) | **60 intents**, **18 scenarios**, **55 slots** (`qa_*`, `calendar_*`, `email_*`, `alarm_*`, etc.) | [aclanthology.org/2023.acl-long.235](https://aclanthology.org/2023.acl-long.235/), [arXiv:2204.08582](https://arxiv.org/abs/2204.08582), [huggingface.co/datasets/AmazonScience/massive](https://huggingface.co/datasets/AmazonScience/massive) | **`CC BY 4.0`** (verified on HF). Covers Indonesian assistant requests/queries in parallel with English; lacks declarative non-request chat negatives and multi-party threads. |
| **xSID (`id` split, v0.4–0.7)** (van der Goot et al. 2021 `[PR]`) | **800 Indonesian utterances** (`300` val + `500` test in `data/xSID-0.4/id.{valid,test}.conll`) | **15 intent labels** (`AddToPlaylist`, `BookRestaurant`, `SearchCreativeWork`, etc.) + slots | [github.com/mainlp/xsid](https://github.com/mainlp/xsid) | **`CC BY-SA 4.0`** (verified in repo `LICENSE`). Share-alike evaluation set for Indonesian assistant intents vs. English source. |
| **IndoToD** (Kautsar et al. 2023 `[PR]`) | `IndoCamRest`: **676 dialogues** (~4.06 turns/dial); `IndoSMD`: **323 dialogues** (~2.63 turns/dial); benchmarks `COD-id` (194 dials) | 4 ToD domains (Restaurant Search, POI Nav, Calendar, Weather); slot/belief state + response | [aclanthology.org/2023.sealp-1.7](https://aclanthology.org/2023.sealp-1.7/), [arXiv:2311.00958](https://arxiv.org/abs/2311.00958), [github.com/dehanalkautsar/IndoToD](https://github.com/dehanalkautsar/IndoToD) | Datasets **`CC BY-SA 4.0`**, code **`Apache-2.0`** (verified in README). Bilingual EN-ID user requests/queries; **no explicit DA taxonomy** covering casual statements vs. questions. |
| **IndoNLU / IndoLEM / NusaX / NusaCrowd / SEACrowd** | **IndoNLU**: 12 NLU tasks (`indonli`, sentiment, NER, etc.); **IndoLEM**: 7 tasks; **NusaX**: 12 parallel langs; **NusaCrowd**: 137 Indonesian datasets | NLI (`entailment/neutral/contradiction`), 3-class sentiment, emotion, POS/NER — **0 dialogue-act / help-request corpora** | [IndoNLU](https://github.com/IndoNLP/indonlu), [NusaX](https://github.com/IndoNLP/nusax), [IndoLEM](https://github.com/indolem/indolem), [NusaCrowd ACL](https://aclanthology.org/2023.findings-acl.868/), [SEACrowd arXiv:2406.10118](https://arxiv.org/abs/2406.10118) | IndoNLU **`Apache-2.0`** (code/repo `MIT` badge vs `Apache-2.0` `LICENSE` file); NusaX **`CC BY-SA 4.0`** data / **`Apache-2.0`** code; IndoLEM GitHub license **`None`** (unspecified). None provides DA tags! |
| **MultiLexNorm (`ind` & `ind_eng`)** + **Barik et al. (2019)** `[PR]` | `ind`: **4,308 tweets** (`3,016`/`431`/`861`); `ind_eng`: **825 ID-EN code-mixed tweets** (`495`/`165`/`165`) | Token-level original $\to$ normalized form + language ID / translation (from Barik et al. W-NUT 2019) | [aclanthology.org/2021.wnut-1.55](https://aclanthology.org/2021.wnut-1.55/), [aclanthology.org/D19-5554](https://aclanthology.org/D19-5554/), [huggingface.co/datasets/robvanderg/MultiLexNorm](https://huggingface.co/datasets/robvanderg/MultiLexNorm), [github.com/ir-nlp-csui/id-en-code-mixed](https://github.com/ir-nlp-csui/id-en-code-mixed) | `ir-nlp-csui/id-en-code-mixed` has **no `LICENSE` file** (`lic=None`); MultiLexNorm cleans Barik's data under shared-task distribution terms. Lexical normalization, **no DA labels**. |
| **IndoCollex** (Wibowo et al. 2021 `[PR]`) | **2,125 word pairs** (`2,126` CSV lines in `data/full.csv`) | Formal $\leftrightarrow$ colloquial morphological word transformations | [aclanthology.org/2021.findings-acl.280](https://aclanthology.org/2021.findings-acl.280/), [github.com/haryoa/indo-collex](https://github.com/haryoa/indo-collex) | **`MIT`** (verified on GitHub). Word-level colloquial pairs, **no sentence DA labels**. |
| **`ID-ATIS` / `Multi3NLU++`** *(verification check)* | *Not verified as Indonesian corpora* | — | — | Prompt example asked to check `ID-ATIS`: `MultiATIS++` (`LDC2021T04`) covers EN/ES/DE/FR/PT/ZH/JA/HI/TR without Indonesian, and an Indonesian `ID-ATIS` corpus could **not be verified** on ACL Anthology, HF, or SEACrowd. **Do not cite `ID-ATIS`**. |

**Verified Corpus Gap:** Neither English/multilingual permissive benchmarks nor Indonesian hubs (`IndoNLU`, `IndoLEM`, `NusaCrowd`, `SEACrowd`) include an authentic **Indonesian (or ID-EN code-mixed) multi-party dialogue-act corpus** distinguishing casual group statements from implicit help requests and information-seeking questions. Therefore, offline training must pair **Coarse Discourse (`CC-BY`) + MIDAS (`Apache-2.0`)** positive/negative dialogue acts with **MASSIVE (`id-ID`, `CC BY 4.0`) + IndoToD (`CC BY-SA 4.0`)** requests and neutral **NusaX/indonli/IndoNLU** declarative statements as negatives, plus Lab-curated colloquial/code-mixed dev/val validation pairs (kept strictly separate from `lab/scenarios/sealed/`).

---

### Scoring Rule Without LLM Calls or Word Lists: Comparison & Literature Accuracy

1. **Positive-only `max_r cos(v, r)` (current `RESEARCH.md` §2 / `design.md` Decision 5) vs. Contrastive Nearest-Centroid vs. Logistic Linear Head on Frozen Embeddings:**
   - **Why positive-only fails:** In embedding geometry, sentence vectors encode topical domain much more strongly than syntactic/pragmatic illocutionary force unless penalized by a negative class. Computing $\max_r \cos(v, r)$ over question/request templates alone allows declarative topic matches (*"The database migration script is failing again"* vs. *"Can someone help with the database migration script?"*) to score nearly as high as actual requests.
   - **Nearest-centroid / Contrastive prototype difference** (Snell et al. 2017 `[PP, arXiv:1703.05175]`): Computing $\cos(v, \mu_+) - \cos(v, \mu_-)$ requires zero iterative optimization, directly removes shared topic bias when $\mathcal{D}_+$ and $\mathcal{D}_-$ span the same domains, and algebraically reduces to $(\mu_+ - \mu_-)^T v$ for L2-normalized $v$.
   - **L2-regularized Logistic Linear Head on Frozen L2-Normalized Embeddings** (exact MMTEB Classification protocol; Enevoldsen et al. 2025 `[PP, arXiv:2502.13595]`): Learns $(w, b)$ by convex logistic regression (`max 100 iters`, verified in MMTEB Section 2.2) over frozen normalized vectors ($d=256$). Unlike fixed equal-weight centroids $\mu_+ - \mu_-$, logistic regression re-weights Matryoshka subspaces specifically aligned with pragmatic request cues (`w`) and calibrates an operating bias (`b`), outputting calibrated probabilities $p_{\text{req}} \in (0, 1)$. Runtime inference inside `core` is **identical in complexity and invariance** to a single prototype: one dot product $w^T v + b$ (~256 FLOPs), **zero LLM calls, zero strings, zero word lists**.
   - **Verified Literature Accuracy:**
     - **Frozen linear probe (MMTEB protocol) on `EmbeddingGemma-300M`:** On **60-class `MassiveIntentClassification`**, a linear logistic head on frozen 768d `embeddinggemma-300m` reaches **85.79% (`en`)** and **77.35% (`id`)** (`embeddings-benchmark/results`, verified directly; vs. `bge-m3` 71.08%/66.90% and `granite-311m-r2` 63.90%/54.40%). On **18-class `MassiveScenarioClassification` (`id`)**, it reaches **86.08%**. On **3-class `NusaX-senti` (`ind`)**, it reaches **88.42%**.
     - *Gap note:* Exact binary request-vs-non-request F1 of a frozen `EmbeddingGemma-300M (256d)` logistic head on multi-party **Coarse Discourse + MIDAS + Indonesian chat** has **not** been published in the literature (**open** — must be measured in Lab; Yu & Yu 2021 `[PR]` report `79.44%` macro-F1 on 23-class MIDAS with fine-tuned BERT).
     - **Fine-tuned encoder comparison (`SetFit`, Tunstall et al. 2022 `[PP, arXiv:2209.11055]`):** SetFit contrastively fine-tunes Sentence Transformers with `N=8` or `N=64` examples/class + logistic head (`62.3%` at `N=8`, `75.3%` at `N=64` across 6 English benchmarks vs. `43.0%` / `69.7%` vanilla fine-tuning, Table 2) and shows **near-zero cross-lingual drop** on few-shot regression (`86.4` MAE zero-shot `en -> {en,de,ja,zh,fr,es}` vs. `86.6` per-language training, Table 4). However, fine-tuning an entire separate SetFit encoder breaks [design.md Decision 14](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L371) ("The reference embedding model is EmbeddingGemma-300M") and degrades `relevance` / `FoK` / retrieval geometry unless isolated as a tiny task adapter or linear head on the shared frozen `EmbeddingGemma-300M` (`256d`) space. Therefore, a **frozen-embedding logistic linear head (`(w, b)` trained offline)** is the cleanest zero-overhead choice first; SetFit/LoRA adapter is a Wave 2 fallback only if binary request-vs-declarative F1 falls short in Lab.

---

### Concrete Anti-Word-List Verification Tests (Proving $(w, b)$ Does Not Collapse to a Keyword List)

To prevent $(w, b)$ from silently learning a disguised regex for `"?"`, `"help"`, `"tolong"`, `"apa"`, or `"bisa"`:

1. **Test 1 — Interrogative/Keyword-Ablation & Trap Adversarial Probe (`Strip-vs-Trap`):**
   - Construct 200+ balanced pairs in both English and Indonesian + ID-EN code-mixed:
     - *(a) Implicit non-keyword requests (Positives with all WH-words, `?`, `"please/tolong/minta/bisa"` removed):* e.g., *"I've been staring at this staging 502 error for an hour."* / *"Deployment-nya mentok terus di step migration nih, udah buntu."*
     - *(b) Keyword-trap non-requests (Negatives containing explicit question/help surface words in declarative/rhetorical/past-tense contexts):* e.g., *"What caused the outage is already fixed in PR #88."* / *"Aku udah jelasin apa masalahnya kemarin, gak usah dibantu lagi."*
   - **Pass criterion:** A baseline keyword/punctuation heuristic collapses to F1 $\le 0.55$ on this slice; the `dialogue_act_gap` linear head MUST achieve **ROC-AUC $\ge 0.85$** and trap false-positive rate **FPR $\le 0.15$**. If `(w, b)` fails this slice, inspect top token attributions (`w^T \Delta v`) and augment training negatives with rhetorical/reported-speech statements.
2. **Test 2 — Zero-Overlap Paraphrase Invariance:**
   - Construct 100 semantic-equivalent pairs $(u, u')$ expressing identical pragmatic request intent with $< 15\%$ token Jaccard overlap (e.g., *"Can anyone double-check the migration diff?"* vs. *"Need a second pair of eyes before we apply those schema changes."*).
   - **Pass criterion:** Mean $|p_{\text{req}}(u) - p_{\text{req}}(u')| < 0.10$ and binary threshold flip rate $< 5\%$ at $\theta_{\text{req}} = 0.50$.
3. **Test 3 — Cross-Lingual EN $\leftrightarrow$ ID Alignment Correlation:**
   - Run parallel EN $\leftrightarrow$ ID utterances (`MASSIVE` `en-US`/`id-ID` + `IndoToD` + Lab ID-EN gaul translations) through the **EN-only trained** `(w, b)` head.
   - **Pass criterion:** Pearson/Spearman correlation $\rho(w^T v_{\text{en}}, w^T v_{\text{id}}) \ge 0.80$ across parallel requests and non-requests, proving the classifier reads shared cross-lingual geometric directions rather than language-specific vocabulary tokens.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `w_da_gap` (`w` in gate `w_info`) | Gate weight for external `dialogue_act_gap` ($p_{\text{req}}(v)$) | `0.30` (`= 0.6 * helpfulness` at `0.50`; currently `0.0` in `core-and-lab` until linear head/reference set lands) | `[0.0, 0.60]` | Formula from [design.md §6](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L175) & [RESEARCH.md §3](file:///home/muqsith/Projects/Mono/RESEARCH.md#L46-L48); tune in Lab |
| `w_da, b_da` | Logistic linear probe weights ($d=256$ floats + 1 bias float) supplied by `reason` | Fit via L2-regularized logistic regression (`C = 1.0`) on frozen `EmbeddingGemma-300M` `256d` vectors | `C in [0.1, 10.0]` | MMTEB classification protocol (`arXiv:2502.13595`); fit & tune in Lab |
| `theta_req` | Diagnostic threshold for Test 1–3 audit only (gate uses continuous $p_{\text{req}}$) | `0.50` | `[0.35, 0.70]` | Tune in Lab for false-wake vs. miss trade-off |
| `lambda_fok` (`own_info_gap`) | Weight of inverted-U $\text{rel} \cdot 4\text{FoK}(1-\text{FoK})$ internal curiosity term (§24 / R-31) | `0.0` (deferred to §24/R-31; `0.50` start once R-31 enables) | `[0.0, 1.0]` | Functional form inferred from Loewenstein (1994) / Kang et al. (2009); tune in Lab |
| `lambda_surp` (`own_info_gap`) | Weight of expectation `surprise` contribution to internal curiosity (§24 / R-31) | `0.0` (deferred to R-31; `0.50` start once R-31 enables) | `[0.0, 1.0]` | Inferred from Gottlieb et al. (2013); tune in Lab |

## Neutral value

- **`dialogue_act_gap` (gate signal):** **Neutral weight $w_{\text{info}} = 0$** (matching [design.md Decision 19](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L471)); missing-vector fallback returns `p_req = 0.0` so non-text bodies and `embedder = none` modes remain deterministic and unaffected.
- **`own_info_gap` (§24 / R-31 internal curiosity):** **Neutral values $\lambda_{\text{fok}} = 0, \lambda_{\text{surp}} = 0 \implies \text{own\_info\_gap} \equiv 0$**.

## Claim

**Ablating `dialogue_act_gap` from `w_info = start_value` to its neutral value `0`:** On multi-party scenarios where unmentioned participants post implicit or explicit help requests/questions (and no explicit `@mention` occurs), removing `dialogue_act_gap` reduces helpful unsolicited response rate and increases missed help requests while cutting false-wake silence errors by a smaller margin; ablating positive-only `max cos(v, r)` to the contrastive logistic linear head $(w, b)$ reduces false unsolicited wakes on topical declarative messages by $\ge 30\%$ while keeping request recall non-inferior on multi-party scenarios (tested across English, Indonesian, and ID-EN code-mixed benchmarks).

## How the Lab would test it

1. **Offline Gate-Classifier Suite (`dev` + `validation` splits only; never touch `sealed/`):** Train `(w, b)` on frozen `EmbeddingGemma-300M (256d)` vectors from `Coarse Discourse` (`question` vs. `elaboration/announcement`), `MIDAS` (`questions + task command` vs. `statement/back-channel/abandon`), `MASSIVE` (`id-ID` + `en-US`), and `NusaX/indonli` declarative negatives. Report 10-seed/fold ROC-AUC, macro-F1, and precision@recall=0.80 separately on English, Indonesian, and ID-EN code-mixed validation slices, plus all three Anti-Word-List Tests above.
2. **Closed-Loop Multi-Party Ablation in Lab:** Compare 3 gate variants at matched total wake rate or fixed wake threshold across scenario sets seeded with explicit mentions, unmentioned questions, unmentioned implicit requests, and pure bystander topic chatter: `(A)` `w_info = 0` (neutral), `(B)` positive-only `max cos(v, r)` (current `RESEARCH.md` baseline), `(C)` logistic linear head `p_req(v) = sigma(w^T v + b)`. Measure helpful unsolicited assistance F1, bystander over-speaking false-positive rate, and total token cost per scenario (with sample size sized per A-16 cluster-power rules).
3. **Separate `own_info_gap` evaluation (Wave 2 / R-31):** Hold gate `dialogue_act_gap` fixed and ablate $\lambda_{\text{fok}}, \lambda_{\text{surp}}$ from neutral `0` on goal-driven discovery/clarification scenarios.

## Established / inferred / open

- **Established:** Intrinsic curiosity / Loewenstein information-gap operates over an organism's own subjective epistemic gap and peaks at intermediate confidence/knowledge (Loewenstein 1994 `[PR]`; Kang et al. 2009 `[PR]`), gated by goal utility/expected learning progress (Gottlieb et al. 2013 `[PR]`; Kidd & Hayden 2015 `[PR]`), which is fundamentally distinct from classifying an external speaker's communicative speech act (Stolcke et al. 2000 `[PR]`; Bunt et al. 2019 `[PR]`; Yu & Yu 2021 `[PR]`). Regularized logistic regression on frozen normalized embeddings (`arXiv:2502.13595` `[PP]`) is fast, parameter-light, and achieves `77.35%` (`id`) / `85.79%` (`en`) across 60 `MASSIVE` intents with `EmbeddingGemma-300M`, while contrastive sentence-transformer tuning (`SetFit`, `arXiv:2209.11055` `[PP]`) transfers cross-lingually with minimal loss. **Verified gap:** No public Indonesian or ID-EN code-mixed multi-party dialogue-act dataset exists (`ID-ATIS` could not be verified).
- **Inferred:** Renaming gate `info_gap` $\to$ `dialogue_act_gap`, replacing positive-only `max_r cos(v, r)` with `(w, b)` linear probing supplied by `reason`, computing `own_info_gap` strictly from $\text{rel} \cdot 4\text{FoK}(1-\text{FoK}) + \text{surprise}$ without an LLM call, and validating against word-list collapse via the three adversarial probes.
- **Open (not settled by the literature):** Exact binary F1 and false-wake rate of frozen `EmbeddingGemma-300M (256d)` `(w, b)` vs. SetFit/LoRA adapter on Indonesian casual/gaul multi-party requests; whether a single shared bilingual `(w, b)` outperforms separate per-language heads `(w_en, b_en), (w_id, b_id)` selected/blended without storing word lists; optimal weighting of `own_info_gap` (`lambda_fok, lambda_surp`) under §24/R-31. **Lab experiments** in Steps 1–3 above will settle each.

## Sources

- `[PR]` Loewenstein, G. (1994). The psychology of curiosity: A review and reinterpretation. *Psychological Bulletin*, 116(1), 75–98. DOI: `10.1037/0033-2909.116.1.75`
- `[PR]` Kang, M. J., Hsu, M., Krajbich, I. M., Loewenstein, G., McClure, S. M., Wang, J. T., & Camerer, C. F. (2009). The Wick in the Candle of Learning: Epistemic Curiosity Activates Reward Circuitry and Enhances Memory. *Psychological Science*, 20(8), 963–973. DOI: `10.1111/j.1467-9280.2009.02402.x`
- `[PR]` Gottlieb, J., Oudeyer, P.-Y., Lopes, M., & Baranes, A. (2013). Information-seeking, curiosity, and attention: computational and neural mechanisms. *Trends in Cognitive Sciences*, 17(11), 585–593. DOI: `10.1016/j.tics.2013.09.001`
- `[PR]` Kidd, C., & Hayden, B. Y. (2015). The Psychology and Neuroscience of Curiosity. *Neuron*, 88(3), 449–460. DOI: `10.1016/j.neuron.2015.09.010`
- `[PR]` Stolcke, A., et al. (2000). Dialogue Act Modeling for Automatic Tagging and Recognition of Conversational Speech. *Computational Linguistics*, 26(3), 339–371. URL: https://aclanthology.org/J00-3003/
- `[PR]` Shriberg, E., Dhillon, R., Bhagat, S., Ang, J., & Carvey, H. (2004). The ICSI Meeting Recorder Dialog Act (MRDA) Corpus. *SIGDIAL 2004*. URL: https://aclanthology.org/W04-2319/ (downloads: https://groups.inf.ed.ac.uk/ami/icsi/download/)
- `[PR]` Li, Y., Su, H., Shen, X., Li, W., Cao, Z., & Niu, S. (2017). DailyDialog: A Manually Labelled Multi-turn Dialogue Dataset. *IJCNLP 2017*. URL: https://aclanthology.org/I17-1099/ (HF: https://huggingface.co/datasets/li2017dailydialog/daily_dialog)
- `[PR]` Bunt, H., Petukhova, V., Malchanau, A., Fang, A., & Wijnhoven, K. (2019). The DialogBank: dialogues with interoperable annotations. *Language Resources and Evaluation*, 53, 213–249. DOI: `10.1007/s10579-018-9436-9` (portal: https://dialogbank.lsv.uni-saarland.de/)
- `[PR]` Yu, D., & Yu, Z. (2021). MIDAS: A Dialog Act Annotation Scheme for Open Domain HumanMachine Spoken Conversations. *EACL 2021*. DOI: `10.18653/v1/2021.eacl-main.94` (arXiv:1908.10023; repo: https://github.com/DianDYu/MIDAS_dialog_act)
- `[PR]` Zhang, A. X., Culbertson, B., & Paritosh, P. (2017). Characterizing Online Discussion Using Coarse Discourse Sequences. *ICWSM 2017*. Repo: https://github.com/google-research-datasets/coarse-discourse
- `[PR]` Lowe, R., Pow, N., Serban, I., & Pineau, J. (2015). The Ubuntu Dialogue Corpus: A Large Dataset for Research in Unstructured Multi-Turn Dialogue Systems. *SIGDIAL 2015*. DOI: `10.18653/v1/W15-4640`
- `[PR]` Kummerfeld, J. K., et al. (2019). A Large-Scale Corpus for Conversation Disentanglement. *ACL 2019*. DOI: `10.18653/v1/P19-1374` (repo: https://github.com/jkkummerfeld/irc-disentanglement)
- `[PR]` Li, J., Liu, M., Zheng, Z., Wu, Y., Hu, J., Qin, B., & Liu, T. (2020). Molweni: A Challenge Multiparty Dialogues-based Machine Reading Comprehension Dataset with Discourse Structure. *COLING 2020*. DOI: `10.18653/v1/2020.coling-main.238` (repo: https://github.com/HIT-SCIR/Molweni)
- `[IND]` Discord-Dialogues (unannotated ChatML Discord turns). URL: https://huggingface.co/datasets/mookiezi/Discord-Dialogues
- `[PR]` FitzGerald, J., et al. (2023). MASSIVE: A 1M-Example Multilingual Natural Language Understanding Dataset with 51 Typologically-Diverse Languages. *ACL 2023*. DOI: `10.18653/v1/2023.acl-long.235` (arXiv:2204.08582; HF: https://huggingface.co/datasets/AmazonScience/massive)
- `[PR]` van der Goot, R., et al. (2021). xSID (Cross-lingual Slot and Intent Detection repository, v0.4–v0.7). URL: https://github.com/mainlp/xsid
- `[PR]` Kautsar, M. D. A., Nurdini, R., Cahyawijaya, S., Winata, G., & Purwarianti, A. (2023). IndoToD: A Multi-Domain Indonesian Benchmark For End-to-End Task-Oriented Dialogue Systems. *SEALP @ AACL 2023*. DOI: `10.18653/v1/2023.sealp-1.7` (arXiv:2311.00958; repo: https://github.com/dehanalkautsar/IndoToD)
- `[PR]` Wilie, B., et al. (2020). IndoNLU: Benchmark and Resources for Evaluating Indonesian Natural Language Understanding. *AACL-IJCNLP 2020*. DOI: `10.18653/v1/2020.aacl-main.85` (repo: https://github.com/IndoNLP/indonlu)
- `[PR]` Koto, F., Rahimi, A., Lau, J. H., & Baldwin, T. (2020). IndoLEM and IndoBERT: A Benchmark Dataset and Pre-trained Language Model for Indonesian NLP. *COLING 2020*. DOI: `10.18653/v1/2020.coling-main.66` (repo: https://github.com/indolem/indolem)
- `[PR]` Winata, G. I., et al. (2023). NusaX: Multilingual Parallel Sentiment Dataset for 10 Indonesian Local Languages. *EACL 2023*. DOI: `10.18653/v1/2023.eacl-main.57` (repo: https://github.com/IndoNLP/nusax)
- `[PR]` Cahyawijaya, S., et al. (2023). NusaCrowd: Open Source Initiative for Indonesian NLP Resources. *Findings of ACL 2023*. DOI: `10.18653/v1/2023.findings-acl.868`
- `[PR]` Wibowo, H. A., Ardhito, H., & Purwarianti, A. (2021). IndoCollex: A Testbed for Morphological Transformation of Indonesian Colloquial Words. *Findings of ACL-IJCNLP 2021*. DOI: `10.18653/v1/2021.findings-acl.280` (repo: https://github.com/haryoa/indo-collex)
- `[PR]` Barik, A. M., Mahendra, R., & Adriani, M. (2019). Normalization of Indonesian-English Code-Mixed Twitter Data. *W-NUT @ EMNLP 2019*. DOI: `10.18653/v1/D19-5554` (repo: https://github.com/ir-nlp-csui/id-en-code-mixed)
- `[PR]` van der Goot, R., et al. (2021). MultiLexNorm: A Shared Task on Multilingual Lexical Normalization. *W-NUT 2021*. DOI: `10.18653/v1/2021.wnut-1.55` (HF: https://huggingface.co/datasets/robvanderg/MultiLexNorm)
- `[PP]` Snell, J., Swersky, K., & Zemel, R. S. (2017). Prototypical Networks for Few-shot Learning. *arXiv:1703.05175*
- `[PP]` Tunstall, L., et al. (2022). Efficient Few-Shot Learning Without Prompts (SetFit). *arXiv:2209.11055*
- `[PP]` Enevoldsen, K., et al. (2025). MMTEB: Massive Multilingual Text Embedding Benchmark. *arXiv:2502.13595*

---

# A-15 Languages (Indonesian & English / Code-Mixed ID-EN): EmbeddingGemma-300M Verification and Colloquial Benchmarks

**Status:** partly answered

## Answer

Direct inspection of `arXiv:2509.20354` (EmbeddingGemma, Schechter Vera et al. 2025 `[PP]`), `arXiv:2502.13595` (MMTEB `[PP]`), `arXiv:2605.13521` (Granite Embedding Multilingual R2, May 2026 `[PP]`), `arXiv:2508.12243` (SEA-BED `[PP]`), `arXiv:2610.37364` (EmbeddingGemma 2, Nov 2026 `[PP]`), and primary task JSONs in `embeddings-benchmark/results` **strongly confirms [core-and-lab Design Decision 14](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L365-L379)** on standard Indonesian and cross-lingual EN–ID tasks: at native 768d, **`EmbeddingGemma-300M` leads all compared models (`bge-m3`, `multilingual-e5-small/base`, `granite-97m/311m-r2`, `granite-107m/278m`, `LaBSE`, `paraphrase-multilingual-MiniLM-L12-v2`) across Indonesian intent classification (`77.35`), scenario classification (`86.08`), sentiment (`88.42`), NLI (`60.95`), regional emotion (`44.16`), and cross-lingual EN$\leftrightarrow$ID QA retrieval (`Belebele 94.04–95.40`), while tying near the top on `MIRACLHardNegatives (id)` (`51.74` vs. `bge-m3 56.02`) and `NusaXBitextMining` (`98.17`)**. Crucially, `granite-embedding-311m-multi-r2`—despite strong multilingual retrieval averages and 32K long-context support—loses `-22.95` points to `EmbeddingGemma-300M` on Indonesian intent classification (`54.40` vs. `77.35`), and `LaBSE` collapses on monolingual Indonesian retrieval (`10.42` vs. `51.74`). However, three gaps remain **not settled by the literature** and require Lab verification: (1) **Per-Indonesian-task Matryoshka `256d` scores are not published** (globally on `MTEB(Multilingual, v2)`, 256d retains Classification `-1.6` and STS `-0.1` cleanly while Retrieval drops `-3.7`); (2) **Zero public embedding benchmark evaluates colloquial Indonesian (*bahasa gaul*) or ID-EN code-mixed text** (`SEA-BED` explicitly filtered *out* code-switching and omits `EmbeddingGemma-300M`); and (3) Decision 14 omits EmbeddingGemma's mandatory **task instruction prefixes** and hardware throughput/licensing trade-offs (`1,349 docs/s` on H100 vs. `2,604 docs/s` for `e5-small`; Gemma Terms of Use vs. Apache-2.0).

**Formulas (Matryoshka truncation, asymmetric instruction formatting, and cosine gate/recall invariance):**
Given unnormalized model representation $h(t; x) \in \mathbb{R}^{768}$ conditioned on Gemma's required prompt prefix $t$ (Table 18 of `arXiv:2610.37364` & [Google Model Card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card)) and Matryoshka cutoff $d \in \{768, 512, 256, 128\}$:
$$
e_d(t; x) = \frac{h_{1:d}(t; x)}{\|h_{1:d}(t; x)\|_2} \in \mathbb{S}^{d-1}, \qquad
\text{cos}_d(q, p) = e_d(t_q; q)^T e_d(t_p; p) \in [-1, 1]
$$
where:
- For **classification / dialogue-act / intent** ($x$ only): `t_clf(x) = "task: classification | query: " + x` (symmetric query/target prompt).
- For **semantic similarity / relevance / FoK event comparisons**: `t_sts(x) = "task: sentence similarity | query: " + x` (or aligned `query`/`document` prompts if retrieval index and event queue share stored vectors: `t_q(q) = "task: search result | query: " + q`, `t_d(p, \text{title}) = "title: " + \text{title} + " | text: " + p`, with `"none"` when title is absent).
- For any embedder whose output dimension mismatches stored memory vectors (`tag != current_embedder` in [model-providers/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/model-providers/spec.md)), stored embeddings MUST include the `(model_id, dim_d, prompt_regime_id)` tag so `core` never mixes incompatible vector spaces.

---

### Verified Quantitative Comparison Tables

#### Table 1: Global Multilingual & Matryoshka Truncation (`arXiv:2509.20354`, `arXiv:2605.13521`, `arXiv:2610.37364`)

| Model | Params / Mem (bf16) | Dim $d$ | Context / License | `MTEB(Multi, v2)` Task / Type | Bitext | Class. | Clus. | Retrieval | STS | Pair Clf | Source |
|---|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---|
| **`EmbeddingGemma-300M` (bf16)** | 308M / 578 MB | **768** | 2,048 / Gemma Terms | **61.15 / 54.31** | 64.4 | 60.9 | 51.2 | 62.5 | 74.7 | 81.4 | `arXiv:2509.20354` T1,T5,T6 |
| `EmbeddingGemma-300M` (int8 / int4) | 308M (QAT) | 768 | 2,048 / Gemma Terms | 60.93 / 53.95 (int8); 60.62 / 53.61 (int4) | — | — | — | — | — | — | `arXiv:2509.20354` T1 |
| `EmbeddingGemma-300M` (MRL 512d) | 308M / 578 MB | **512** | 2,048 / Gemma Terms | 60.7 / 53.9 | 64.0 | 60.3 | 50.9 | 61.5 | 74.8 | 81.5 | `arXiv:2509.20354` T6 |
| **`EmbeddingGemma-300M` (MRL 256d)** *(Decision 14 target)* | 308M / 578 MB | **256** | 2,048 / Gemma Terms | **59.7 / 53.0** (`-1.5 / -1.3`) | 62.4 (`-2.0`) | 59.3 (`-1.6`) | 50.5 (`-0.7`) | **58.8 (`-3.7`)** | 74.6 (`-0.1`) | 81.5 (`+0.1`) | `arXiv:2509.20354` T6 |
| `EmbeddingGemma-300M` (MRL 128d) | 308M / 578 MB | **128** | 2,048 / Gemma Terms | 58.2 / 51.8 | 59.3 | 57.8 | 50.6 | 55.3 | 73.8 | 81.3 | `arXiv:2509.20354` T6 |
| `EmbeddingGemma 2` (multimodal, Nov 2026) | 271M text+vis (577M +aud, 740M all) | 768 / **256** | Multimodal (8K) / Apache-2.0 | 61.4 / 53.8 (`256d`: **60.4 / 53.0**) | 65.7 (`63.6`) | 63.0 (`61.9`) | 50.4 (`51.1`) | 60.8 (`58.7`) | 74.7 (`74.1`) | 78.8 (`78.7`) | `arXiv:2610.37364` T19 |
| `bge-m3` | 568M / 2,167 MB | 1024 | 8,192 / MIT | 59.56 / 52.18 | — | — | — | — | — | — | `arXiv:2509.20354` T5 |
| `multilingual-e5-base` | 278M / 1,061 MB | 768 | 512 / MIT | 57.0 / 49.8 | 69.4 | 58.2 | 41.8 | 52.7 | 71.4 | — | `arXiv:2509.20354` T6 |
| `multilingual-e5-small` | 118M / 449 MB | 384 | 512 / MIT | 55.8 / 49.0 | 69.5 | 56.2 | 40.8 | 49.3 | 71.7 | — | `arXiv:2509.20354` T6 |
| `granite-embedding-278m-multilingual` (R1) | 278M / 530 MB | 768 | 512 / Apache-2.0 | 53.7 / 47.2 | 58.5 | 54.1 | 41.4 | 52.2 | 67.3 | 75.7 | `arXiv:2509.20354` T6 |
| `granite-embedding-311m-multilingual-r2` (R2) | 311M (110M act.) | 768 / **256** | **32,768** (8K tr.) / Apache-2.0 | *Not reported on full MTEB(Multi, v2)*; Multi-Ret(18): **65.2** (`256d`: **64.7**) | — | — | — | 65.2 (`64.7`) | — | — | `arXiv:2605.13521` T2,T8,T10 |

*(Note: `arXiv:2610.37364` Table 19 also reports newer 2026 sub-300M models `harrier-oss-v1-270m` [66.5/56.9] and `jina-embed-v5-text-nano` [65.5/57.7] on `MTEB(Multi, v2)`; per-Indonesian JSON files for those two models were not extracted in this session and remain unverified for Indonesian.)*

#### Table 2: Verified Indonesian, Cross-Lingual EN–ID, and Indonesian Regional Scores (`embeddings-benchmark/results` JSONs, Native Dimensions)

All values below are exact percentages ($100\times \text{main\_score}$) extracted directly from [github.com/embeddings-benchmark/results](https://github.com/embeddings-benchmark/results) JSON files at each model's indexed commit revision:

| Model (native $d$) | Massive Intent (`id` / `en`) | Massive Scen. (`id` / `en`) | NusaX-senti (`ind`) | IndoNLI (`default`) | Clickbait (`id`) | NusaPara Emot. (10 reg. avg) | Belebele (`id-id` / `id-en` / `en-id`) | MIRACL HardNeg (`id`) | NusaX Bitext (`en-id`) | NusaTrans Bitext (11 reg. avg) | Flores Bitext (`en-id` / `id-jav`) | SIB200 Clus. (`ind`) |
|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|---:|
| **`embeddinggemma-300m` (768d)** | **77.35** / 85.79 | **86.08** / 91.54 | **88.42** | **60.95** | 60.87 | 44.16 | **94.45 / 95.40 / 94.04** | 51.74 | 98.17 | 66.05 | **100.00** / 87.83 | **45.50** |
| `embeddinggemma-2` (768d) | 75.22 / **88.34** | 84.43 / **91.67** | 87.60 | 54.75 | **64.51** | **45.12** | 92.59 / 93.22 / 92.09 | 51.50 | 97.15 | 71.25 | **100.00** / 86.54 | 43.28 |
| `bge-m3` (1024d) | 66.90 / 71.08 | 72.23 / 76.64 | 85.53 | 56.00 | 59.82 | 43.25 | 93.81 / 92.46 / 92.21 | **56.02** | **98.21** | 76.67 | **100.00** / **99.74** | 35.47 |
| `multilingual-e5-base` (768d) | 58.70 / 65.71 | 63.60 / 71.57 | 72.30 | 50.97 | 60.06 | 38.30 | 94.00 / 91.84 / 88.55 | 51.19 | 89.88 | 63.52 | 98.06 / 74.97 | 12.48 |
| `multilingual-e5-small` (384d) | 56.19 / 64.20 | 61.42 / 68.98 | 74.60 | 51.86 | 62.49 | 37.85 | 91.50 / 88.09 / 82.05 | 50.93 | 91.78 | 77.20 | 99.47 / 98.95 | 36.55 |
| `granite-311m-multi-r2` (768d) | 54.40 / 63.90 | *(sub omitted)* | 71.10 | 49.04 | 58.77 | 36.09 | 88.90 / 90.62 / 85.28 | 46.93 | 94.88 | 58.71 | 99.21 / 88.52 | 42.90 |
| `granite-97m-multi-r2` (384d) | 48.16 / — | *(sub omitted)* | 67.83 | 48.65 | 56.65 | 32.44 | 85.81 / 84.26 / 78.17 | 45.98 | 82.71 | 49.24 | 95.36 / 73.55 | 39.93 |
| `granite-278m-multi` (768d) | 51.67 / — | 60.02 / — | 63.58 | 49.54 | 58.08 | — | 84.72 / 83.96 / 81.45 | 47.51 | 84.91 | 59.52 | — | 43.82 |
| `granite-107m-multi` (384d) | 48.40 / — | 57.24 / — | 62.68 | 48.67 | 56.92 | — | 81.87 / 81.47 / 77.13 | 46.04 | 80.25 | 58.58 | — | 40.86 |
| `LaBSE` (768d) | 61.14 / 61.44 | 65.86 / 66.44 | 74.25 | 49.86 | 59.13 | 38.87 | 72.31 / 76.49 / 66.40 | 10.42 | 97.60 | **81.17** | **100.00** / 99.60 | 23.50 |
| `paraphrase-m-MiniLM-L12-v2` (384d) | 59.90 / *(404)* | 66.17 / *(404)* | 74.75 | 55.79 | 54.48 | 28.09 | 80.22 / 81.67 / 79.74 | 28.18 | 89.66 | 37.37 | 99.87 / 49.32 | 44.35 |

*(Also checked across H100 inference throughput in `arXiv:2605.13521` Table 9/12 at 512 tokens, batch 512, `transformers 5.8.0`: `multilingual-e5-small` = 2,604 docs/s; `granite-97m-multi-r2` = 2,534 docs/s; `granite-278m-multi` = 2,164 docs/s; `multilingual-e5-base` = 2,025 docs/s; `granite-311m-multi-r2` = 1,828 docs/s [or 2,960 on 4.57.6]; `embeddinggemma-300m` = 1,349 docs/s. Granite R2 uses the 262,152-token Gemma 3 tokenizer on 311m-r2 and a 180,000-token tokenizer on 97m-r2, and note `2610.37364` Table 26 also reports native audio `MSEB` Indonesian `id-ID` MRR@10 = `75.24` for `EmbeddingGemma 2`.)*

---

### Evidence Audit: Colloquial Indonesian (*Bahasa Gaul*), ID-EN Code-Mixed Text, and Recommended Lab Evaluation Sets

1. **What exists and what is missing (verified):**
   - **SEA-BED (`arXiv:2508.12243v3`, Apr 2026 `[PP]`):** Covers 314 language-dataset pairs across 10 SEA languages (70 Indonesian datasets; best verified Indonesian overall average `79.73` for `Qwen3-Embedding-8B`, `79.50` for `multilingual-e5-large-instruct`, `78.59` for `multilingual-e5-large`, `78.09` for `bge-m3`, `75.19` for `LaBSE`, `70.50` for `multilingual-MiniLM-L12`). **Critically, SEA-BED did NOT evaluate `EmbeddingGemma-300M` (`0` hits), explicitly filtered OUT code-switching in its human quality check (`"(excluding code-switching)"`), and contains zero slang/colloquial *bahasa gaul* benchmarks.**
   - **SEA-HELM (`arXiv:2502.14301v2`, Jun 2025 `[PP]`, `aisingapore/SEA-HELM` `MIT`):** Evaluates LLMs across formal Indonesian NLP, linguistics, culture, and safety; contains **0 colloquial/gaul or code-mixed embedding tasks**.
   - **No named `IndoMTEB` paper/leaderboard** exists independently of MMTEB's Indonesian subset, `SEA-BED`, and community work (`LazarusNLP/indonesian-sentence-embeddings`, `Apache-2.0`, evaluating STS-B/MIRACL/TyDiQA/IndoNLU).
   - **Conclusion:** Performance of `EmbeddingGemma-300M` (and its 256d truncation) on informal chat Indonesian (*bahasa gaul*: abbreviations like *yg, gak, bgt, bkn, dgn*, Jakarta *lu/gue*, reduplication *cepet2*, enclitics *-nya/-kah/-dong/-nih*) and intra-sentential ID-EN code-switching (*"Gue udah push fix-nya ke staging, tinggal tunggu CI green aja"*) is **not settled by the literature**.

2. **Verified Public Evaluation Sets Recommended for the Lab:**
   - **Formal Indonesian & EN–ID Alignment Baseline:** `MassiveIntentClassification (id + en)` (`CC BY 4.0`), `BelebeleRetrieval (ind-ind, ind-eng, eng-ind)`, `MIRACLRetrievalHardNegatives (id)` (`Apache-2.0`), `ACIQuAD` (`CC BY 4.0`), and `NusaX-senti (ind)` (`CC BY-SA 4.0`).
   - **Colloquial & Code-Mixed Robustness / Geometry Retrieval (Directly Verified):**
     1. **`MultiLexNorm` `ind` (4,308 tweets) + `ind_eng` (825 ID-EN code-mixed tweets)** ([robvanderg/MultiLexNorm](https://huggingface.co/datasets/robvanderg/MultiLexNorm); derived from Barik et al. 2019 `[PR, D19-5554]`): Evaluate **paired `orig` (colloquial/code-mixed) $\leftrightarrow$ `norm` (formal normalized)** cosine similarity distribution and **MRR@1 / Recall@1** retrieving a noisy tweet's normalized formal twin against distractors — requires zero new annotations!
     2. **`IndoCollex` (`2,125` colloquial $\leftrightarrow$ formal pairs, `MIT`, [haryoa/indo-collex](https://github.com/haryoa/indo-collex)):** Instantiate paired slot-substituted Lab validation sentences comparing semantic cosine invariance $\cos(e_d(x_{\text{formal}}), e_d(x_{\text{gaul}}))$ and score stability of `dialogue_act_gap` / `relevance`.
     3. **`xSID` (`id` `800` utterances, `CC BY-SA 4.0`, [mainlp/xsid](https://github.com/mainlp/xsid)) & `IndoToD` (`999` native dialogues across `IndoCamRest` + `IndoSMD`, `CC BY-SA 4.0`, [dehanalkautsar/IndoToD](https://github.com/dehanalkautsar/IndoToD)):** Zero-shot and few-shot Indonesian conversational intent/query testbeds; create non-sealed Lab perturbation triples (`EN` $\leftrightarrow$ `Formal ID` $\leftrightarrow$ `Gaul/ID-EN Code-Mixed`).

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `embedder_model` | Primary shared text embedder (`reason`) | `google/embeddinggemma-300m` | `{embeddinggemma-300m, bge-m3, multilingual-e5-base, granite-311m-multi-r2}` | Verified in `arXiv:2509.20354` & `embeddings-benchmark/results`; supports [Decision 14](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L371) |
| `embed_dim` ($d$) | Matryoshka truncation dimensionality passed to `core` | `256` | `{128, 256, 512, 768}` | `arXiv:2509.20354` Table 6 (`256d` MRL retains STS/Clf, drops `-3.7` on Retrieval globally); verify Indonesian `256` vs `512`/`768` in Lab |
| `prompt_regime` | Prefix formatter (`classification` vs. `sentence similarity` vs. asymmetric `search result` query/doc) | Dual or task-matched prefixes (`classification` for `dialogue_act_gap`; `search result` or `sentence similarity` for memory/relevance) | `{unprefixed, symmetric_sts, dual_prefixed}` | [Google Model Card](https://ai.google.dev/gemma/docs/embeddinggemma/model_card) / `arXiv:2610.37364` Table 18; benchmark prefix overhead in Lab |
| `quant_precision` | Weight precision for local CPU/GPU deployment | `bf16` (or `QAT int8`) | `{bf16, int8, int4}` | `arXiv:2509.20354` Table 1 (`-0.22` int8 / `-0.53` int4 on MTEB Multi v2) |

## Neutral value

Neutral mechanism in [model-providers/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/model-providers/spec.md) and [appraisal-gate/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/appraisal-gate/spec.md): **`embedder = none` (vectors absent)** — all vector-dependent gate terms (`dialogue_act_gap`, `feeling_of_knowing`, `relevance`) fall back to neutral `0` and vector memory retrieval falls back to non-vector/recency selection; **or deterministic offline hash/TF-IDF baseline embedder** as the Lab ablation baseline when isolating learned multilingual semantic geometry.

## Claim

1. Truncating `EmbeddingGemma-300M` to `256d` preserves $\ge 97\%$ of full `768d` performance on Indonesian classification/intent (`MassiveIntent id`, `NusaX-senti`), NLI (`indonli`), and EN–ID bitext/QA (`Belebele`), while staying superior to `multilingual-e5-small/base`, `granite-311m-multi-r2`, `LaBSE`, and `paraphrase-multilingual-MiniLM-L12-v2` on Indonesian dialogue/memory tasks (if `256d` Retrieval `MIRACL id` drops $> 2.5$ points, `512d` recovers it).
2. With official EmbeddingGemma task prefixes applied, EN$\leftrightarrow$ID code-mixed and *bahasa gaul* twin retrieval on `MultiLexNorm (ind, ind_eng)` and `IndoCollex` maintains Recall@1 degradation $\le 10$ percentage points relative to formal Indonesian twins.

## How the Lab would test it

1. **Per-Indonesian MRL & Prefix Ablation (`d in {128, 256, 512, 768}`, `{unprefixed, symmetric_sts, official dual prefixes}`):** Run deterministic MTEB-style evaluation locally on `MassiveIntentClassification (id, en)`, `NusaX-senti (ind)`, `indonli`, `BelebeleRetrieval (ind-ind, ind-eng, eng-ind)`, and `MIRACLRetrievalHardNegatives (id)` (plus `bge-m3` and `multilingual-e5-base` controls) to obtain the missing Indonesian `256d` vs `768d` curve and measure latency per event on target hardware.
2. **Colloquial & Code-Mixed Retrieval Stress Test (`MultiLexNorm ind`, `ind_eng`, `IndoCollex`):** Compute paired formal–gaul similarity gap $\cos(e_d(x_{\text{orig}}), e_d(x_{\text{norm}}))$ and cross-form twin retrieval `MRR@1 / Recall@5` across candidate pools of sizes `{165, 861, 4308}`.
3. **Closed-Loop End-to-End Bilingual & Code-Mixed Scenarios:** Run identical multi-party Lab scenarios translated/rewritten into three parallel registers (`English`, `Formal Indonesian`, `Bahasa Gaul / ID-EN Code-Mixed`), comparing `EmbeddingGemma-300M (256d)` vs `512d` vs `bge-m3` vs `deterministic fallback` on wake accuracy, memory recall precision, and task completion.

## Established / inferred / open

- **Established:** At native `768d`, `EmbeddingGemma-300M` achieves top Indonesian classification, NLI, regional sentiment, and cross-lingual EN–ID QA retrieval among $\le 568$M-param public models (`arXiv:2509.20354`, `embeddings-benchmark/results`), substantially beating `granite-embedding-*-r2`, `multilingual-e5-small/base`, `LaBSE`, and `MiniLM`. Globally, MRL `256d` causes only a small drop in classification/STS (`-1.6 / -0.1`) but a sharper retrieval drop (`-3.7` on `MTEB(Multi, v2)`). `SEA-BED` explicitly omitted code-switching and `EmbeddingGemma-300M`, leaving colloquial and code-mixed Indonesian embedding performance unmeasured in public leaderboards. `granite-311m-multi-r2` runs ~35% faster (`1,828` vs `1,349` docs/s) with Apache-2.0 and 32K context, at a `-22.95`-pt cost on `MassiveIntent (id)`.
- **Inferred:** Keeping `EmbeddingGemma-300M` at `256d` (Decision 14) with official Gemma prompt prefixes, verifying whether `512d` is warranted only if `MIRACL id` / memory recall degrades, and reprioritizing `MultiLexNorm (ind, ind_eng)` + `IndoCollex` twin-retrieval as immediate proxy benchmarks for *bahasa gaul* and ID-EN code-mixing.
- **Open (not settled by the literature):** Exact Indonesian `256d` vs `768d` scores on `MassiveIntent`, `Belebele`, and `MIRACL`; quantitative drop of `EmbeddingGemma-300M` under *bahasa gaul* orthography and intra-sentential ID-EN code-mixing; head-to-head Indonesian comparison against late-2026 sub-300M contenders (`harrier-oss-v1-270m`, `jina-embed-v5-text-nano`); and whether asymmetric Gemma prompt prefixes cause double-embedding overhead in `reason`. **Lab experiments 1–3 above will settle these.**

## Sources

- `[PP]` Schechter Vera, H., Dua, S., et al. (2025). EmbeddingGemma: Powerful and Lightweight Text Representations. *arXiv:2509.20354* (v3 Nov 2025)
- `[PP]` Schechter Vera, H., Dua, S., et al. (2026). EmbeddingGemma 2: Unified and Efficient Multimodal Representations. *arXiv:2610.37364* (v2 Nov 2026)
- `[IND]` Google AI. EmbeddingGemma Model Card (inputs, 2K context, 768/512/256/128 MRL, prompt instructions, Gemma Terms of Use). URL: https://ai.google.dev/gemma/docs/embeddinggemma/model_card
- `[PP]` Enevoldsen, K., et al. (2025). MMTEB: Massive Multilingual Text Embedding Benchmark. *arXiv:2502.13595* (v4 Nov 2025)
- `[IND]` MTEB Official Results Repository (`embeddings-benchmark/results`, verified Indonesian/English task JSONs). URL: https://github.com/embeddings-benchmark/results
- `[PP]` IBM Granite Embedding Team (2026). Granite Embedding Multilingual R2: Fast, Compact, and Long-Context Multilingual Embedding Models. *arXiv:2605.13521* (v2 May 2026)
- `[PP]` Ponwitayarat, W., et al. (2025/rev. Apr 2026). SEA-BED: How Do Embedding Models Represent Southeast Asian Languages? *arXiv:2508.12243* (v3)
- `[PP]` Susanto, Y., et al. (2025). SEA-HELM: Southeast Asian Holistic Evaluation of Language Models. *arXiv:2502.14301* (v2; repo: https://github.com/aisingapore/SEA-HELM)
- `[PP]` Lovenia, H., et al. (2024/rev. Mar 2025). SEACrowd: A Multilingual Multimodal Data Hub and Benchmark Suite for Southeast Asian Languages. *arXiv:2406.10118* (v5)
- `[IND]` LazarusNLP. Indonesian Sentence Embeddings benchmark repository. URL: https://github.com/LazarusNLP/indonesian-sentence-embeddings
- `[PR]` FitzGerald, J., et al. (2023). MASSIVE (`id-ID`, `en-US`). DOI: `10.18653/v1/2023.acl-long.235`
- `[PR]` Wilie, B., et al. (2020). IndoNLU (`indonli`, etc.). DOI: `10.18653/v1/2020.aacl-main.85`
- `[PR]` Winata, G. I., et al. (2023). NusaX (`ind` + 10 Indonesian local langs). DOI: `10.18653/v1/2023.eacl-main.57`
- `[PR]` Wibowo, H. A., Ardhito, H., & Purwarianti, A. (2021). IndoCollex. DOI: `10.18653/v1/2021.findings-acl.280`
- `[PR]` Barik, A. M., Mahendra, R., & Adriani, M. (2019). Normalization of Indonesian-English Code-Mixed Twitter Data. DOI: `10.18653/v1/D19-5554`
- `[PR]` van der Goot, R., et al. (2021). MultiLexNorm (`ind` & `ind_eng`). DOI: `10.18653/v1/2021.wnut-1.55` (HF: https://huggingface.co/datasets/robvanderg/MultiLexNorm)
- `[PR]` van der Goot, R., et al. (2021). xSID (`id`). Repo: https://github.com/mainlp/xsid
- `[PR]` Kautsar, M. D. A., et al. (2023). IndoToD. DOI: `10.18653/v1/2023.sealp-1.7`

---

# A-16 Rebuilt Context on Multi-Party Tasks (R-11): Non-Inferiority Protocol, Restart Verification, and Power Analysis

**Status:** not settled by the literature

## Answer

No published experiment through November 2026 tests MonoAgens' rebuilt context structure (stable identity + sorted capabilities + present/self cards + commitments/goal + per-chain working set [`own thoughts/acts in full + M recent observations + older ID placeholders`] + top-$k$ recalled memories + last $N$ place events) against a full-transcript baseline on multi-party tasks including mid-task restarts; closest 2025–2026 evidence splits sharply into three warnings: (1) **Observation masking ($M=10$) halves cost without losing solve rate in single-agent coding (`Qwen3-Coder 480B`: $\Delta = +1.4\text{ pp } [-1.6, +4.4]$), yet causes significant regression in thinking models (`Gemini 2.5 Flash (thinking)`: $-4.0\text{ pp } [-7.8, -0.2]$ masking; $-9.0\text{ pp } [-13.0, -5.2]$ LLM-summary; `arXiv:2508.21433`)**; (2) **Full context still beats verbatim RAG chunks (`59.9%` vs `43.9–47.4%` on LoCoMo) which decisively beat lossy extracted artifacts (`26.5–30.2%`) and summarization (`0.0%` passkey exact recall; `arXiv:2601.00821v4`)**; and (3) **Existing memory systems collapse on multi-party group chats (`GroupMemBench`, `arXiv:2605.14498`: best system `Hindsight 46.01%` overall, dropping to `17.76–27.10%` on Knowledge Updates and `14.15–37.74%` on Term Ambiguity), whereas branching/per-thread discourse rebuilding (`Context-Agent`, `arXiv:2604.05552`) surpasses full flat history while cutting active context tokens `-46.0% to -52.3%` where naive sliding truncation loses `-33.4% to -36.9%` relative TCR.** To test [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L223-L226) rigorously, the Lab MUST run a **3-arm paired trial** (`Arm F`: Full Transcript; `Arm R`: MonoAgens Rebuilt Context; `Arm T`: Matched-Token Naive Sliding Truncation for assay sensitivity) stratified across continuous and **mid-task process-restart** scenarios, requiring **statistical non-inferiority** on task success within a fixed **75% effect-preservation margin $\Delta_{\text{NI}} = 0.05$** (two-sided 95% / one-sided 97.5% CI lower bound $> -0.05$; Blackwelder 1982; FDA 2016; EMA 2005; Piaggio et al. 2012) alongside **cost/token superiority** ($\ge 40\%$ prompt-token reduction on $>15$-turn threads without degrading prompt-cache hit share). Crucially, exact paired power math reveals that [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L90-L94)'s **"at least five seeds"** cannot verify non-inferiority on a small scenario suite due to intra-scenario seed correlation (`DE = 1 + (R-1)\rho_s`); verifying $\Delta_{\text{NI}} = 0.05$ requires $N_{\text{eff}} \approx 471$ paired scenarios ($R=1$–$2$ seeds/scenario across $471$–$500$ scenarios, or $\ge 283$ scenarios at $R=5$ seeds if $\rho_s = 0.5$).

**Non-Inferiority Formulation, Margin Justification, and Sample Size Formulas (every symbol defined):**

1. **Hypotheses (Blackwelder 1982 `[PR]`, FDA 2016 `[IND]`, Piaggio et al. 2012 CONSORT `[PR]`):**
   Let $p_R, p_F, p_T \in [0, 1]$ denote population binary task success rates for **Arm R** (Rebuilt Context), **Arm F** (Full Transcript Control), and **Arm T** (Naive Token-Matched Truncation Assay Control), evaluated overall and within the mid-task **Restart** stratum $R_{\text{str}}$:
   $$
   H_0:\; p_R - p_F \le -\Delta_{\text{NI}} \qquad \text{vs.} \qquad H_1:\; p_R - p_F > -\Delta_{\text{NI}}, \qquad \Delta_{\text{NI}} = 0.05\text{ (5.0 pp)}
   $$
   Non-inferiority is established iff the lower bound of the **two-sided 95% confidence interval** (equivalently one-sided $\alpha = 0.025$) for the paired difference $\hat{\Delta} = \hat{p}_R - \hat{p}_F$ satisfies:
   $$
   \text{CI}_{L,95\%}(\hat{p}_R - \hat{p}_F) > -\Delta_{\text{NI}}
   $$
   both **overall** AND on the **post-restart subset**, AND Arm R simultaneously preserves **prompt-cache hit share** ($C_R \ge C_F - 0.05$) and achieves **$\ge 40\%$ total input-token reduction** ($1 - \bar{T}_R / \bar{T}_F \ge 0.40$) on multi-party tasks exceeding the compression window ($> M + N$ events).

2. **Justification of $\Delta_{\text{NI}} = 0.05$ via FDA (2016) / EMA (2005) Fixed-Margin Method:**
   In FDA Fixed-Margin NI trials, the margin $\Delta_{\text{NI}} = (1 - f)\cdot M_1$ bounds loss to at most $(1 - f)$ of $M_1$, the reliably established superiority of the Full Transcript comparator $F$ over a minimal/truncated baseline $T$ (assay sensitivity):
   - *Empirical $M_1$ from 2025–2026 literature:* On non-linear multi-turn tasks (`Context-Agent`, `arXiv:2604.05552` Table 14), Full-History beats Naive Sliding Truncation by **$30.8\text{ pp}$ (`GPT-4.1`: `86.0% vs 55.2%`), $21.5\text{ pp}$ (`DeepSeek-V3`: `64.3% vs 42.8%`), and $26.4\text{ pp}$ (`GLM-4-Plus`: `71.5% vs 45.1%`)**. On long-context memory benchmarks (`arXiv:2601.00821v4` Table 1 & `arXiv:2410.10813`), truncation falls $> 20\text{ pp}$ below full context. Taking the conservative lower bound $M_1 = 0.20$ (`20 pp`, verified inline on every Lab run via `Arm F` minus `Arm T`):
   - *Requiring effect preservation fraction $f = 0.75$ (preserving $\ge 75\%$ of Full Context's advantage over naive truncation, stricter than FDA's 50% clinical floor):*
     $$
     \Delta_{\text{NI}} = (1 - f) \cdot M_1 = (1 - 0.75) \times 0.20 = 0.05\text{ (5.0 percentage points)}
     $$
   *(For early dev diagnostics only where full 471+ paired scenarios are not yet curated, $\Delta_{\text{NI,pilot}} = 0.08$ corresponds to $f = 0.60$, but cannot be used as final acceptance proof.)*

3. **Paired & Cluster-Adjusted Sample Size Formulas (Tango 1998 `[PR]`; Donner, Birkett & Buck 1981 `[PR]`):**
   Because Arm R and Arm F execute on the **same scenarios and seeds**, outcomes $(Y_{R,s,r}, Y_{F,s,r})$ are positively paired ($d_{s,r} = Y_{R,s,r} - Y_{F,s,r} \in \{-1, 0, +1\}$). Let $p_{\text{disc}} = P(Y_R \neq Y_F) = p_{10} + p_{01}$ be the paired discordant rate (for context, in `arXiv:2508.21433` Table 4 on $n=500$, `Qwen3-Coder 480B` Observation Masking gave paired 95% CI $[-1.6, +4.4]\text{ pp} \implies \text{SE} \approx 1.53\text{ pp} \implies p_{\text{disc}} \approx 0.117$). Under expected true difference $\theta = p_R - p_F = 0$, the required number of **effective independent scenario-pairs** $N_{\text{eff}}$ is:
   $$
   N_{\text{eff}} = \left\lceil \frac{(z_{1-\alpha} + z_{1-\beta})^2 \, p_{\text{disc}}}{(\theta + \Delta_{\text{NI}})^2} \right\rceil
   $$
   When running $R \ge 1$ stochastic seeds per scenario across $S$ scenarios, seed repetitions on the same scenario correlate with intra-scenario correlation $\rho_s = \text{ICC}(d_{s,r}) \in [0.3, 0.7]$ (scenario structure dominates LLM decoding temperature). By Donner, Birkett & Buck (1981), variance inflates by Design Effect $\text{DE} = 1 + (R - 1)\rho_s$, requiring **$S$ distinct scenarios**:
   $$
   S = \left\lceil \frac{N_{\text{eff}} \cdot \bigl(1 + (R - 1)\rho_s\bigr)}{R} \right\rceil
   $$
   and confidence intervals MUST be computed using **cluster-bootstrap (resampling scenarios with replacement) or Tango's paired score CI (`DOI:10.1002/(SICI)1097-0258(19980430)17:8<891::AID-SIM780>3.0.CO;2-B`) when $R=1$**.

---

### Summary of Closely Related Literature (Through November 2026)

| Work | Domain / Task | Context Regime Tested | Verified Key Result | Relevance & Gap for MonoAgens A-16 |
|---|---|---|---|---|
| **Lindenbauer et al. (`arXiv:2508.21433`, Aug 2025 `[PP]`)** — *The Complexity Trap* | SWE-bench Verified (`n=500` paired coding tasks, SWE-agent + OpenHands) | Observation Masking ($M=10$ recent obs full, older omitted) vs. LLM-Summary ($N=21, M=10$ tail) vs. Hybrid vs. Raw | Halves cost (`-50.9% to -56.1%`). On `Qwen3-Coder 480B`, Raw `53.4%`, Masking `54.8%` ($\Delta = +1.4\text{ pp } [-1.6, +4.4]$), Summary `53.8%` ($\Delta = +0.4\text{ pp } [-3.0, +3.8]$). **Failure on `Gemini 2.5 Flash (thinking)`:** Raw `40.4%` drops to Masking `36.4%` ($\Delta = -4.0\text{ pp } [-7.8, -0.2]$, $-9.9\%$ rel., $p=0.0406$) and LLM-Summary `31.4%` ($\Delta = -9.0\text{ pp } [-13.0, -5.2]$, $-22.3\%$ rel.). | Validates MonoAgens' $M=10$ verbatim observation window & warns that **thinking/reasoning models can regress silently** unless tested under NI; does NOT cover multi-party social chat, cache-hit share, or restarts. |
| **An (`arXiv:2601.00821v4`, rev. Jul 2026 `[PP]`)** — *Fidelity Before Structure* | LoCoMo (`arXiv:2402.17753`) & LongMemEval-S (`arXiv:2410.10813`, 500 questions, ~115K tok/prob) | Full Context vs. Verbatim Chunks vs. Graph/Typed Artifacts vs. Summarization | **LoCoMo (Table 1/11):** Full context ceiling **`59.9%`** still beats best Verbatim chunks (`43.9–47.4%`), which beat Typed lossy artifacts (`26.5–30.2%`). On synthetic passkey/keyword recall (Table 5), Summarization gets **`0.0%` pass** vs. RAG `55.5%`. | Proves why MonoAgens MUST preserve **verbatim recent obs & own acts + verbatim recalled memories** rather than lossy summaries — AND proves full context still has a large ceiling advantage (`+12.5 to +16 pp` over plain RAG chunks!), demanding NI testing. |
| **Yang et al. (`arXiv:2605.14498`, May 2026 `[PP]`)** — *GroupMemBench* | 4 domains × 30,000 messages (`120,000` msgs; 6–18 participants, 6–10 projects, `~3,500` cross-project replies + `~1,520` noise) | RAG (`BM25`, `text-embedding-3-large`, `GraphRAG`) vs. `Mem0`, `MemGPT`, `A-Mem`, `HippoRAG`, `Hindsight` | Best average **`Hindsight = 46.01%`** vs. `BM25 = 43.22%`, `HippoRAG = 39.72%`, `MemGPT = 28.15%`, `Mem0 = 25.73%`. **Collapse categories:** `Knowledge Update` (`17.76–27.10%`, `Mem0 4.67%`) and `Term Ambiguity` (`14.15–37.74%`). | Identifies the exact multi-party failure modes MonoAgens' rebuilt context MUST be scored on: **cross-project interference, knowledge updates, speaker-dependent ambiguity, and abstention**. |
| **Hu et al. (`arXiv:2604.05552v2`, Apr 2026 `[PP]`)** — *Context-Agent* | Long non-linear branching multi-turn dialogues (`10–25` turns) + LoCoMo | Dynamic Discourse Tree (branch-aware rebuilt context) vs. Flat Truncation vs. Full-History | Naive truncation crashes TCR relative to Full-History by **`-35.8%` (`GPT-4.1`: `86.0 -> 55.2`), `-33.4%` (`DeepSeek-V3`: `64.3 -> 42.8`), `-36.9%` (`GLM-4-Plus`)**. Tree-structured rebuilt context achieves **`88.9%` (`+3.4%` rel.)** with **`-52.3%` ACT**; removing Tree (`w/o Tree`) crashes to `41.5%`. | Directly validates MonoAgens' **per-chain working set isolation** over flat global truncation. |
| **Tirmazi et al. (`arXiv:2608.01326`, Aug 2026 `[PP]`)** — *Context Compaction Theory* | Formal complexity theory | Context Selection Game (subset retention) vs. Context Generation Game (arbitrary bounded summary) | Theorem 1: Minimum compaction budget for target error $\le \epsilon$ equals the one-way randomized communication complexity of the induced query problem. | Explains why arbitrary lossy summaries cannot beat full context unless exact sparse witnesses (commitments/anchors + episodic recall) are indexed cleanly. |

---

### Exact Sample Size Table (Computed with `statistics.NormalDist`, One-Sided $\alpha = 0.025$ [Two-Sided 95% CI], $\theta = 0$)

| Design / Assumption | Target Margin $\Delta_{\text{NI}}$ | Power $1-\beta$ | Effective Paired Trials $N_{\text{eff}}$ | If $R=5$ seeds/scenario ($\rho_s = 0.5, \text{DE}=3.0$) | Recommended Cost-Optimal Split ($R=1$–$2$ seeds) | Verdict vs. `core-and-lab` |
|---|---:|---:|---:|---:|---:|---|
| Unpaired Blackwelder ($p=0.70$, per arm) | `0.05` | `0.80` | `1,319` per arm (`2,638` total) | — | — | Naive unpaired runs waste $>2.8\times$ trials |
| Paired ($p_{\text{disc}} = 0.10$) | `0.05` | `0.80` | **`314` pairs** | `S = 189` scenarios (`945`/arm) | **`S = 314` ($R=1$)** or `S = 204` ($R=2, \text{DE}=1.5$) | `5 seeds × 20 scenarios` fails completely |
| **Paired (`p_disc = 0.15`, Complexity Trap realistic)** | **`0.05`** | **`0.80`** | **`471` pairs** | **`S = 283` scenarios (`1,415`/arm)** | **`S = 471` ($R=1$)** or **`S = 354` ($R=2, \text{DE}=1.5$)** | **Contradicts naive 5-seed small-suite reading of `lab/spec.md`** |
| Paired ($p_{\text{disc}} = 0.20$) | `0.05` | `0.80` | **`628` pairs** | `S = 377` scenarios (`1,885`/arm) | `S = 628` ($R=1$) or `S = 471` ($R=2$) | Same |
| Paired (`p_disc = 0.15`, tight 90% power) | `0.05` | `0.90` | **`631` pairs** | `S = 379` scenarios (`1,895`/arm) | `S = 631` ($R=1$) | High-precision release gate |
| Pilot Dev Gate (`p_disc = 0.15`, looser margin) | `0.08` | `0.80` | **`184` pairs** | `S = 111` scenarios (`555`/arm) | `S = 184` ($R=1$) | Useful only for dev screening, not NI certification |

*Why the contradiction in [lab/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md#L90-L94) matters:* With $S = 20$ scenarios $\times R = 5$ seeds (`100` paired observations) and $\rho_s = 0.5$ ($\text{DE} = 3.0$), $N_{\text{eff}} = 33.3$, yielding a paired 95% CI half-width of $\pm 1.96 \sqrt{0.15/33.3} \approx \pm 13.2\text{ pp}$. A system losing `-8 pp` (strictly inferior!) passes within seed noise as *"no measured effect"* unless scenarios $S$ are scaled rather than seeds $R$. **Recommendation:** Run paired Arms `(F, R, T)` across $S \ge 350$–$500$ scenarios with $R = 1$–$2$ seeds each for NI certification (allocating $R \ge 5$ repeated seeds only to smaller diagnostic variance/stability checks).

---

### Full Lab Protocol: Rebuilt Context vs. Full Transcript & Mid-Task Restart

1. **Stratified Scenario Matrix (covering `dev`, `validation`, and untouched `sealed/` final check):**
   - **4 Scenario Archetypes (equal strata across $S$ scenarios):**
     1. *Parallel Multi-Chain Cross-Talk (`>15` turns, `3–6` speakers):* Group chat with interleaved threads & cross-project interruptions (`GroupMemBench`-style cross-project replies + noise).
     2. *Sequential Knowledge Updates & Corrections:* User A states Fact X at turn 4; User B revises Fact X $\to$ X' at turn 18; query/task depends on X' at turn 30 (`Update` check).
     3. *Term Ambiguity & Speaker-Grounded Attribution:* Same ambiguous term means different things depending on speaker/thread (`Ambiguity` + attribution check).
     4. *Mid-Task Process Restart (50% of runs paired, or dedicated `S_restart >= 200` scenarios):* Halt the `monoagensd` / agent runtime deterministically at 50%–65% task progress right after persistence of commitments/memories/working-set checkpoint; wipe all ephemeral RAM/KV caches (`reinit`); wake on an asynchronous follow-up message or delayed expectation timer (`restart` resilience check).
   - **Test across BOTH non-thinking AND thinking-mode LLMs** (explicitly guarding against the `arXiv:2508.21433` `Gemini 2.5 Flash (thinking)` regression).
2. **Co-Primary Endpoints (both required to claim superiority of rebuilt context):**
   - **Endpoint A (Non-Inferiority of Task Success):** Paired binary completion/correctness $p_R - p_F$ overall and on the Restart stratum; PASS iff **cluster-bootstrap 95% CI lower bound $> -\Delta_{\text{NI}} = -0.05$** AND assay-sensitivity check confirms $\hat{p}_F - \hat{p}_T \ge 0.15$ (proving the benchmark requires long-range memory!).
   - **Endpoint B (Cost & Cache Efficiency):** Total prompt tokens consumed $\bar{T}_R \le 0.60\,\bar{T}_F$ ($\ge 40\%$ reduction on $>15$-turn threads) AND prompt-cache hit share $C_R \ge C_F - 0.05$ (measured explicitly on `F`, `R` uncached, and `R` prefix-stabilized).
3. **Diagnostic Secondary Metrics:**
   - Stale-Fact Error Rate (citing superseded pre-update facts);
   - Wrong-Speaker Attribution Rate (attributing Speaker A's claim to Speaker B);
   - Post-Restart Context-Recovery Latency (extra wakes/tool calls or `content_miss` lookups after restart before resuming correct chain act);
   - Abstention / Over-Claiming Rate on unanswerable/missing-thread queries.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `M` (`chain_working_set`) | Number of most recent chain observations kept verbatim before ID-placeholder masking ([reasoning/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/reasoning/spec.md)) | `10` | `[4, 20]` | `arXiv:2508.21433` Section 3.1 / D.1 (`M=10` optimal tail); tune in Lab |
| `N` (`place_recent_events`) | Number of recent place events retained verbatim in rebuilt context ([design.md Decision 9](file:///home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md#L276)) | `20` | `[8, 40]` | Tune in Lab across 3–6 speaker interleaved threads |
| `k` (`recalled_memories`) | Top-$k$ verbatim memory records injected per wake | `5` | `[3, 12]` | `arXiv:2601.00821v4` verbatim > lossy artifacts; tune in Lab (`<3` misses multi-hop, `>12` destabilizes cache prefix) |
| `delta_NI` | Fixed non-inferiority margin on binary task success ($\Delta_{\text{NI}}$) | `0.05` (`5.0 pp`) | `[0.03, 0.08]` (never $>0.08$) | Derived via FDA (2016) / EMA (2005) $f=0.75, M_1=0.20$ |
| `S_min` (`R` seeds) | Minimum paired scenarios $S$ (at $R=1$–$2$ seeds/scenario for $\rho_s=0.5$) | `S >= 471` (`R=1`) / `S >= 354` (`R=2`) | `S in [184, 631]` | Exact Tango/cluster NI power ($\alpha=0.025, 1-\beta=0.80, p_{\text{disc}}=0.15$) |

## Neutral value

**`Arm F` (Full Transcript Control):** Uncompacted chronological place + chain transcript sent verbatim up to the model's context limit, with no observation placeholder masking and no episodic memory selection. Ablating the rebuilt context pipeline returns to `Arm F`.

## Claim

**Rebuilt context vs. Neutral (`Arm F` Full Transcript):** On multi-party continuous and mid-task-restart scenarios (`>15` turns), `Arm R` is **statistically non-inferior** to `Arm F` in task success within margin $\Delta_{\text{NI}} = 0.05$ (two-sided 95% cluster-bootstrap CI lower bound $> -0.05$ both overall and on the Restart stratum, while assay control `Arm T` lags `Arm F` by $\ge 15$ pp) AND reduces prompt token consumption by $\ge 40\%$ with preserved/higher prompt-cache hit share.

## How the Lab would test it

Run the **3-arm paired trial (`F`, `R`, `T`)** across `S >= 354` scenarios (`R=2`, $\ge 708$ paired runs/arm, including $\ge 50\%$ mid-task restart runs, on at least one non-thinking and one thinking-mode LLM), compute scenario-clustered 95% bootstrap CIs for $\hat{p}_R - \hat{p}_F$ (overall and per-stratum), verify assay sensitivity $\hat{p}_F - \hat{p}_T$, and log prompt tokens, cached token share (`cached/total`), post-restart `content_miss` recovery calls, stale-fact errors, and speaker-attribution errors.

## Established / inferred / open

- **Established:** Flat chronological truncation drops task completion by `-33% to -37%` relative (`arXiv:2604.05552`); observation masking (`M=10`) halves token cost without hurting non-thinking coding agents (`arXiv:2508.21433`), **but** causes statistically significant `-4.0` to `-9.0` pp drops on thinking models; full context (`59.9%`) still beats verbatim RAG (`43.9–47.4%`) which decisively beats lossy artifact extraction (`26.5–30.2%`) (`arXiv:2601.00821v4`); multi-party agents fail sharply (`17.8–37.7%`) on knowledge updates and speaker/term ambiguity (`arXiv:2605.14498`); one-way communication complexity lower-bounds fixed-budget compaction error (`arXiv:2608.01326`); and FDA/EMA/Piaggio CONSORT require pre-registered $\Delta_{\text{NI}}$, assay sensitivity controls, and properly powered paired/cluster CIs.
- **Inferred:** Fixing $\Delta_{\text{NI}} = 0.05$ (`f=0.75` of $M_1=0.20$), requiring the 3-arm `(F, R, T)` paired trial across continuous and process-restart strata, identifying the cache-prefix risk of dynamic `top-k` recall placement, and proving `core-and-lab`'s 5-seed floor is insufficient unless $S \ge 283$–$471$ scenarios are evaluated.
- **Open (not settled by the literature):** Whether MonoAgens' rebuilt context achieves NI within $\Delta_{\text{NI}} = 0.05$ against full raw transcripts on multi-party tasks (especially with thinking-mode models, knowledge updates, and mid-task restarts); exact empirical $p_{\text{disc}}$ and intra-scenario seed correlation $\rho_s$ in MonoAgens; optimal `M`, `N`, `k` trade-off preserving prefix cache hits across restarts vs. minimizing token cost. **Settled strictly by running the A-16 Lab protocol.**

## Sources

- `[PP]` Lindenbauer, T., et al. (2025). The Complexity Trap: Simple Observation Masking Is as Efficient as LLM Summarization for Agent Context Management. *arXiv:2508.21433*
- `[PP]` An, T. (2026). Fidelity Before Structure: Verbatim Chunks Beat Lossy Artifact Extraction in Long-Conversation LLM Memory. *arXiv:2601.00821* (v4 Jul 2026)
- `[PP]` Maharana, A., Lee, D.-H., Tulyakov, S., Bansal, M., Barbieri, F., & Fang, Y. (2024). Evaluating Very Long-Term Conversational Memory of LLM Agents (LoCoMo). *arXiv:2402.17753*
- `[PP]` Wu, D., Wang, H., Yu, W., Zhang, Y., Chang, K.-W., & Yu, D. (2024/rev. Mar 2025). LongMemEval: Benchmarking Chat Assistants on Long-Term Interactive Memory. *arXiv:2410.10813* (v2)
- `[PP]` Hu, J., Guo, S., Liu, W., Yin, J., & Wei, Y. (2026). Context-Agent: Dynamic Discourse Trees for Non-Linear Dialogue. *arXiv:2604.05552* (v2 Apr 2026)
- `[PP]` Yang, et al. (2026). GroupMemBench: Benchmarking Multi-Party Conversational Memory. *arXiv:2605.14498* (May 2026)
- `[PP]` Tirmazi, H., Markelon, S., Bishop, A., & Mitzenmacher, M. (2026). Context Compaction Theory. *arXiv:2608.01326* (Aug 2026)
- `[PR]` Blackwelder, W. C. (1982). "Proving the null hypothesis" in clinical trials. *Controlled Clinical Trials*, 3(4), 345–353. DOI: `10.1016/0197-2456(82)90024-1`
- `[PR]` Piaggio, G., Elbourne, D. R., Pocock, S. J., Evans, S. J. W., & Altman, D. G. (2012). Reporting of Noninferiority and Equivalence Randomized Trials: Extension of the CONSORT 2010 Statement. *JAMA*, 308(24), 2594–2604. DOI: `10.1001/jama.2012.87802`
- `[PR]` Tango, T. (1998). Equivalence test and confidence interval for the difference in proportions for the paired-sample design. *Statistics in Medicine*, 17(8), 891–908. DOI: `10.1002/(SICI)1097-0258(19980430)17:8<891::AID-SIM780>3.0.CO;2-B`
- `[PR]` Donner, A., Birkett, N., & Buck, C. (1981). Randomization by cluster: sample size requirements and analysis. *American Journal of Epidemiology*, 114(6), 906–914. DOI: `10.1093/oxfordjournals.aje.a113261`
- `[IND]` U.S. Food and Drug Administration (2016). *Non-Inferiority Clinical Trials to Establish Effectiveness: Guidance for Industry*. URL: https://www.fda.gov/media/78504/download
- `[IND]` European Medicines Agency (EMA / CHMP, 2005). *Guideline on the Choice of the Non-Inferiority Margin* (`EMEA/CPMP/EWP/2158/99`; direct PDF check returned HTTP 429 rate-limit, guideline ID and framework verified via indexed guidance documentation).
