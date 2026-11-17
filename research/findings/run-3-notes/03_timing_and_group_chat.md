# Run 3 Research Note — Timing & Group Chat Slice (`pitch`, `proact`, `when2speak`, `ubuntu_irc`, `molweni`)

**Scope:** External benchmark datasets in the Timing (`BenchmarkDomain::ProactivityTiming`) and Group Chat (`BenchmarkDomain::GroupChat`) slice of `standard_benchmark_registry()` ([crates/lab/src/benchmark.rs](file:///home/muqsith/Projects/Mono/crates/lab/src/benchmark.rs#L134-L191)).  
**Spec anchors:** [openspec/changes/lab-datasets/design.md](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/design.md), [benchmark-datasets/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-datasets/spec.md), [benchmark-evaluation/spec.md](file:///home/muqsith/Projects/Mono/openspec/changes/lab-datasets/specs/benchmark-evaluation/spec.md), [crates/protocol/src/message.rs](file:///home/muqsith/Projects/Mono/crates/protocol/src/message.rs#L122-L215).

---

## 1. Executive Summary & Owner Choices Needed

### Snapshot of the 5 Datasets

| `dataset_id` | Public Existence | Canonical Source | License Status (Observed) | Official Protocol vs Registry Alignment | `R-53` Percept Mapping | `R-54` Power (`S >= 354–471`) |
|---|---|---|---|---|---|---|
| `pitch` | **Yes** | `https://github.com/Social-Futures-Lab/pitch` (`arXiv:2610.25896`) | **MIT** | Aligned (`live_best_turn_acc`, `in_window_acc`; 0 LLM judge calls) | Mappable via staged pending knowledge item; **verify turn timestamps on fetch** | **384 candidates / 1,091 judgments** (all 384 candidates $\ge 354$; 35 convs require cluster-aware CIs) |
| `proact` | **No public release found** | Paper only: `https://arxiv.org/abs/2607.03730` | Paper on arXiv; **dataset files not publicly released** | Matches `appropriateness`, `non_interruptiveness` (uses **GPT-5.4-Nano** judge in paper), but **unavailable**; **name collision** with `bosonai/proactbench` & others | Blocked (`unavailable`); single-turn snapshot without multi-turn decay | Would be feasible (`N = 3,244`) if released; currently **`unavailable`** |
| `when2speak` | **Yes** | `https://huggingface.co/datasets/duke-trust-lab/When2Speak` (`arXiv:2605.05626`) | **CC BY 4.0** (HF card; arXiv page uses CC BY-SA 4.0) | Aligned (`macro_f1`, `fir`, `mir`; 0 LLM judge calls) | Mappable as 8-turn windows; **no wall-clock timestamps** (synthetic turn clock only) | **21,687 test items** (13.1% `SPEAK`); recommend $S \ge 1,000$ or stratified sampling |
| `ubuntu_irc` | **Yes (2 distinct corpora)** | **Corpus A:** `https://github.com/jkkummerfeld/irc-disentanglement`<br>**Corpus B:** `https://github.com/hiroki13/response-ranking` | **A:** Data `CC BY 4.0`, Code `ISC`<br>**B:** **No `LICENSE` file in repo** | **Mismatch / Dual-corpus collision:** `thread_f1` is Corpus A; `addressee_p1` is Corpus B | **A:** Rich IRC logs w/ `[HH:MM]` timestamps<br>**B:** Preprocessed tuples without wall-clock timestamps | **A:** 5,000 test msgs<br>**B:** 32,668–35,638 test sessions (`Len-5/10/15`) |
| `molweni` | **Yes** | `https://github.com/HIT-SCIR/Molweni` (`2020.coling-main.238`) | **No explicit `LICENSE` file in GitHub repo** (paper is `CC BY 4.0`) | `qa_f1` official (EM/F1); DP official metric is **`Link F1`** (check vs `sdrt_link_acc`) | Mappable as dialogue turns + probe; **no wall-clock timestamps** (sequential `id` only) | **MRC test:** 2,871 QAs (100 dialogues); **DP test:** 500 dialogues |

### Critical Findings & Owner Choices Needed (No Choices Made Here)

1. **`proact` (`availability = "unavailable"` + Multi-Dataset Name Collision):**
   - Primary verification confirms *ProACT: Towards Breakdown-Aware Proactive Agent in Multi-User Collaboration* (`arXiv:2607.03730`, Yang et al., July 2026) matches the registry's `appropriateness` / `non_interruptiveness` metrics and `"Breakdown intervention precision"` caveat, **but no public GitHub repository or Hugging Face dataset release exists**.
   - **Name collisions found:**
     1. `bosonai/proactbench` (*ProactBench*, Hugging Face, 198 dialogues / 624 triggers, `Apache-2.0` + `CC-BY-4.0`, LLM rubric judge) — a *different* proactive multi-turn benchmark.
     2. `biang889/ProAct` (GitHub/HF, MCTS/RL lookahead in 2048/Sokoban) — unrelated domain.
     3. `UsanoCoCr/ProAct-Data` (HF, motion generation) — unrelated domain.
     4. Note that `ProactiveAgent` / `ProactiveBench` (`thunlp/ProactiveAgent`) is already separately registered as `proactive_bench`.
   - **Owner choice:** Under `benchmark-datasets/spec.md` (*"The manifest names datasets that are not yet published"*), `proact` should be marked `availability = "unavailable"` and kept as a visible gap rather than silently substituted. Confirm whether to keep `proact` (`arXiv:2607.03730`) as `unavailable` or, via a separate OpenSpec change, evaluate `bosonai/proactbench` under its own ID.

2. **`ubuntu_irc` (Dual-Corpus Collision Across Registry Metrics):**
   - `crates/lab/src/benchmark.rs` registers a single ID `ubuntu_irc` with `&["thread_f1", "addressee_p1"]`. Primary sources show these two metrics belong to **two separate benchmarks**:
     - `thread_f1` comes from **Ubuntu IRC Disentanglement** (`jkkummerfeld/irc-disentanglement`, ACL 2019, `CC BY 4.0` data / `ISC` tools).
     - `addressee_p1` comes from **Ouchi & Tsuboi (2016) Ubuntu MPC** (`hiroki13/response-ranking`, EMNLP 2016, **no `LICENSE` file in repo**) or **Hu et al. (2019) GSN** Ubuntu MPC.
   - **Owner choices:**
     - **Option A:** Bind `ubuntu_irc` to `jkkummerfeld/irc-disentanglement` only (`CC BY 4.0`), report official `thread_f1` (and `link_f1` / `1-1` / `scaled_vi`), and either remove `addressee_p1` from `ubuntu_irc` or add a non-official derived parent-speaker accuracy renamed under `benchmark-evaluation/spec.md` (e.g., `parent_speaker_p1`).
     - **Option B:** Split into two manifest/registry IDs (`ubuntu_irc` / `ubuntu_irc_disentanglement` for `jkkummerfeld/irc-disentanglement` and `ubuntu_mpc` for `hiroki13/response-ranking`), noting `hiroki13/response-ranking` lacks an explicit `LICENSE` file.
     - **Option C:** Fetch both corpora under one `ubuntu_irc` manifest entry (awkward because licenses, file layouts, and evaluation splits differ).

3. **`molweni` (Unlicensed GitHub Repo vs Run 2 Assumption + `sdrt_link_acc` Naming + High Contamination):**
   - **License correction from Run 2:** Run 2 (`R39_R48_benchmarks_identity.md`) listed `HIT-SCIR/Molweni Apache-2.0`. Direct verification of `https://github.com/HIT-SCIR/Molweni` shows **no explicit `LICENSE` file** in the repository (only the COLING 2020 paper in ACL Anthology is `CC BY 4.0`). Owner must decide license clearance (`cleared` vs `refused`).
   - **Metric naming choice:** Official Molweni Discourse Parsing papers report **Link F1** (`Link` F1 over directed EDU dependency edges, plus `Link&Rel` F1 over 16 SDRT relations), rather than a metric literally named `sdrt_link_acc`. Owner must choose whether to align `sdrt_link_acc` to official **Link F1** (and rename if required by `benchmark-evaluation/spec.md`) or implement strict edge-accuracy if multi-parent EDUs exist.
   - **Timestamp gap:** Molweni JSON contains sequential `id` integers only (no wall-clock timestamps).

4. **`when2speak` (Variant Choice + Sample Size under 13.1% Positive Class + Missing Timestamps):**
   - **File/variant choice:** `duke-trust-lab/When2Speak` ships two parallel test files of 21,687 examples each: `finetune_test_token.jsonl` (single-token `<` / `>` decision) vs `finetune_test_dialogue.jsonl` (`>` silence or full utterance). Owner must choose which variant the Lab scores (or score both).
   - **Sample size choice:** Because `SPEAK` is only 13.1% of items, a uniform sample of $S = 354\text{--}471$ yields only ~46–62 positive examples (`MIR` denominator). Owner should choose between uniform $S \ge 1,000$ (giving ~131+ positives) vs stratified sampling at $S = 471$ with class-prior reweighting for `macro_f1`.
   - **License note:** HF card states `CC BY 4.0` (while arXiv paper page carries `CC BY-SA 4.0`).

5. **`pitch` (Exact SHA/Size Pinning + Timestamp Verification + Candidate-Presentation Choice):**
   - GitHub commit SHA, exact byte sizes, and SHA-256 checksums for `data/pitch-benchmark-data.json` and `data/human-baseline-data.csv` are not published in the README and could not be observed via search; they must be pinned on first fetch with `--record-checksums`.
   - `pitch` has **384 candidate messages** (1,091 total judgments: 384 author + 707 peer) across **35 conversations**. Owner must decide whether primary reporting scores against **384 author judgments** ($N = 384 \ge 354$) or all **1,091 participant judgments**, and verify whether `pitch-benchmark-data.json` includes per-turn timestamps or turn indices only.

---

## 2. Detailed Dataset Reports (`R-51`–`R-54`)

### 2.1 `pitch` — PITCH (*Proactive Intervention Timing in CHat*)

#### `R-51` Provenance
- **Public existence & canonical name:** Yes — **PITCH** (*Proactive Intervention Timing in CHat*), introduced in *"When Should AI Chime In? Benchmarking Timely Contribution of Proactive Agents in Multi-Party Chat"* (Ruotong Wang, Brian S. Yu, Xinyi Zhou, Joseph Chee Chang, Pao Siangliulue, Jonathan Bragg, Liwei Jiang, Amy X. Zhang; Oct 2026, `arXiv:2610.25896`).
- **Canonical source:**
  - Primary benchmark repo: `https://github.com/Social-Futures-Lab/pitch`
  - Companion human-baseline platform repo: `https://github.com/Social-Futures-Lab/pitch-human-baseline`
  - Project website: `https://ruotongw.github.io/PITCH-website/`
- **Pinned revision:** *Unverified via web search* (no release tag or commit SHA was surfaced in repository summaries; owner/fetch must resolve and pin the exact `main` commit SHA and date at manifest entry time).
- **Files needed & sizes:**
  - `data/pitch-benchmark-data.json` (conversation transcripts, candidate knowledge items, multi-annotator timing judgments & consensus)
  - `data/human-baseline-data.csv` (human baseline timing judgments)
  - `data/README.md` (schema reference)
  - *Exact byte sizes unverified via web search* (not listed in observed README summaries).
- **Checksums:** Not published by source; requires `monoagens-lab datasets fetch --record-checksums` on first fetch.
- **License & access:** **MIT License** (`MIT` SPDX) in both `Social-Futures-Lab/pitch` and `Social-Futures-Lab/pitch-human-baseline`. Allows local copies, commercial use, and redistribution of derived files with copyright/license notice. No access gate.

#### `R-52` Official Evaluation Protocol
- **Primary metrics in registry:** `live_best_turn_acc`, `in_window_acc`.
- **Official definitions (`arXiv:2610.25896`):**
  - **`live_best_turn_acc`:** Under **live causal** evaluation (agent observes turns $1 \dots t$ strictly sequentially without future lookahead), the fraction of candidate contributions placed at the exact human-annotated ideal turn $t_i$ (or correctly withheld as `do-not-surface` when labeled `do-not-surface`). Paper reports offline full-transcript best-turn accuracy of 57.8% collapsing to **19.0% live** (with 46.4% premature vs 1.0% late; lay humans 23.7% live, 39.7% premature).
  - **`in_window_acc`:** Fraction of candidate contributions placed within the human-annotated acceptable turn window $[s_i, e_i]$ (inclusive), or correctly withheld for `do-not-surface`.
  - Additional paper diagnostics: Premature-vs-Late placement rate; offline vs live comparison.
- **Reporting split:** Full benchmark dataset (all 35 unscripted conversations, 2,194 turns, 384 candidate contributions, 1,091 timing judgments = 384 author + 707 peer). No train/dev/test split exists; evaluation-only benchmark.
- **Official scoring script & revision:** No standalone official scoring script path was observed in `Social-Futures-Lab/pitch` (`data/` + `annotation-system/` only); scoring is deterministic turn-index / window interval matching per `arXiv:2610.25896`.
- **LLM judge required?** **No** for timing placement (`live_best_turn_acc`, `in_window_acc`). (The paper additionally analyzed reasoning traces qualitatively.)
- **Contamination:** Published Oct 2026 (`arXiv:2610.25896`); low pretraining contamination for older models, though publicly accessible on GitHub from Oct 2026 onward.

#### `R-53` Mapping to Percepts
- **Unit-to-percept mapping:**
  - Conversation -> `PlaceId` (`conv_id`); 2–4 human participants -> `ActorId`s; `audience` -> all room participants + soul.
  - Each chat turn $t$ -> `Percept` with `actor`, `place`, `audience`, `payload` (turn text).
  - **Candidate knowledge item staging:** Because PITCH tests *when* to surface a given candidate insight/contribution (rather than open-ended invention), the body/converter must stage the candidate knowledge item as an available non-gold input (e.g., via an initial background/staging percept or capability cue before/during the conversation) without any timing hint. At each turn $t$, if the soul wakes and emits an `Action` surfacing that candidate, $t$ is recorded as its predicted intervention turn; if it never surfaces by conversation end, it is recorded as `do-not-surface`.
- **Gold isolation:** `ideal_turn` $t_i$, `acceptable_window` $[s_i, e_i]$, and `do-not-surface` labels reside strictly in `GoldBook` and never enter any `Percept` or memory payload.
- **Representability caveats:**
  - `data/README.md` references parsing conversational turns and timestamps, but exact timestamp field presence/granularity in `pitch-benchmark-data.json` is **unverified until fetch**. If timestamps are missing or relative-only, deterministic synthetic turn spacing must be documented.
  - Chat-only social dynamics (matches `benchmark.rs` caveat).

#### `R-54` Sampling and Power (`S >= 354–471`)
- **Dataset counts:** **35 conversations** (14 collective planning, 21 open research; 2,194 turns), **384 candidate messages**, **1,091 participant-grounded judgments** (384 author + 707 peer).
- **Power feasibility:**
  - **Judgment level ($N = 1,091$):** Exceeds $S \ge 354\text{--}471$ (can evaluate all 1,091 or stratify 471).
  - **Candidate level ($N = 384$):** Meets the lower bound ($384 \ge 354$, slightly below 471). **Run all 384 candidates (100%)** rather than subsampling; use conversation-clustered bootstrap / paired intervals because multiple candidates share each of the 35 conversations.
- **Stratification (for judgment subsampling or subgroup reporting):** Stratify by task domain (`collective planning` vs `open research`), annotator role (`author` vs `peer`), and label (`surfaceable` vs `do-not-surface`).
- **Per-item cost shape:** Up to ~63 turns/conv average (2,194 turns total). If candidates within a conversation are evaluated causally one-by-one, ~384 × ~30–63 prefix turn steps; if multiplexed per conversation, ~2,194 turn ticks total (with LLM calls only when MonoAgens' Gate wakes). **0 LLM judge calls.**

---

### 2.2 `proact` — ProACT (*Towards Breakdown-Aware Proactive Agent in Multi-User Collaboration*)

#### `R-51` Provenance
- **Public existence & canonical name:** **Dataset NOT publicly released (`unavailable`).**
  - The paper matching `benchmark.rs` (`appropriateness`, `non_interruptiveness`, `"Breakdown intervention precision"`) is **ProACT** (*"ProACT: Towards Breakdown-Aware Proactive Agent in Multi-User Collaboration"*, Shu Yang, Difei Xu, Jiaxin Pei, Di Wang; July 2026, `arXiv:2607.03730`, `https://arxiv.org/abs/2607.03730`).
  - No public GitHub repository or Hugging Face dataset card containing the 3,244-example benchmark was found across `arxiv.org`, `github.com`, or `huggingface.co`.
- **Name collisions / Disambiguation (do not substitute silently):**
  1. **`ProactBench` (`bosonai/proactbench` on Hugging Face):** 198 curated dialogues, 624 trigger points (Emergent 201, Critical 232, Recovery 191), `Apache-2.0` (corpus/code) + `CC-BY-4.0` (Nemotron-Personas-USA), evaluated by `proactbench/evaluation.py` with LLM rubric judge (`Pass`/`Partial`/`Fail`). *Different benchmark.*
  2. **`ProAct` (`biang889/ProAct` on GitHub/HF):** Dual-system MCTS/RL lookahead reasoning for interactive environments (2048, Sokoban). *Unrelated.*
  3. **`ProAct-Data` (`UsanoCoCr/ProAct-Data` on HF):** Motion generation dataset. *Unrelated.*
  4. **`ProactiveAgent` / `ProactiveBench` (`thunlp/ProactiveAgent`):** Already registered separately as `proactive_bench`.
- **Canonical source / revision / files / checksums:** None publicly available (`availability = "unavailable"` in manifest per `benchmark-datasets/spec.md`).
- **License:** The `arXiv:2607.03730` preprint itself is available on arXiv (`CC BY 4.0` / arXiv license), but no dataset repository license exists because the dataset files are unreleased.

#### `R-52` Official Evaluation Protocol (from `arXiv:2607.03730` paper)
- **Primary metrics in registry:** `appropriateness`, `non_interruptiveness`.
- **Official definitions (paper):**
  - **Collaborative Appropriateness (0/1):** Whether the agent intervenes on an evidence-grounded collaboration breakdown (6 classes: disagreements/conflicts, ambiguous goals, forgotten constraints, underspecified plans, discussion loops, imbalanced participation) vs staying silent during smooth progress.
  - **Non-Interruptiveness (0/1):** Whether the output avoids disrupting productive human discussion, duplicating active peer repairs, taking sides prematurely, or dominating the group flow.
  - Additional paper metrics: **Conciseness** (0/1) and **Intervention Quality** (1–5).
- **Reporting split:** Full test benchmark of **3,244 turn-level examples** (2,444 real from GitHub issues + QMSum [694 human-reviewed] + 800 synthetic from BEAM; balanced 1,622 intervene / 1,622 silence).
- **Official scoring script:** Not publicly released.
- **LLM judge required?** **Yes** — paper uses **GPT-5.4-Nano** as LLM-as-a-judge against structured rubric criteria (exact verbatim judge prompt template is not publicly released in a repo).
- **Contamination:** Underlying real sources (public GitHub issues, QMSum) exist in standard pretraining corpora; the 800 BEAM synthetic examples and breakdown labels are new (July 2026).

#### `R-53` Mapping to Percepts
- **If released in future:** Each example is a speaker-attributed dialogue prefix up to turn $t$: `actor` = speaker, `place` = issue/meeting ID, `audience` = participants + soul, `payload` = utterance. Soul decides at turn $t$ to stay silent or emit an intervention `Action`.
- **Representability caveats (blocked until release):**
  - Currently **`unavailable`** (cannot map without files).
  - Single-turn snapshot design means no downstream human reaction or multi-turn trust decay is tested.
  - Presence of timestamps across GitHub issues vs QMSum vs BEAM cannot be verified without the files.

#### `R-54` Sampling and Power (`S >= 354–471`)
- **Dataset counts:** 3,244 examples total (1,622 intervene / 1,622 silence; 2,444 real / 800 synthetic; 6 breakdown classes).
- **Power feasibility:** Currently **blocked (`unavailable`)**. If released, $N = 3,244$ easily supports $S = 471\text{--}480$.
- **Recommended stratification (if released):** Stratified by source (`GitHub issues`, `QMSum`, `BEAM synthetic`) × gold action (`intervene` vs `silence`) × 6 breakdown classes.
- **Per-item cost shape (if released):** 1 snapshot decision (0–1 agent LLM call depending on Gate) + **1 LLM judge call** (`GPT-5.4-Nano` or configured judge) per evaluated item.

---

### 2.3 `when2speak` — When2Speak (`duke-trust-lab/When2Speak`)

#### `R-51` Provenance
- **Public existence & canonical name:** Yes — **When2Speak** (*"When2Speak: A Dataset for Temporal Participation and Turn-Taking in Multi-Party Conversations for Large Language Models"*, Vihaan Nama, Shreya Mendi, Zian Ye, Brinnae Bent; May 2026, `arXiv:2605.05626`).
- **Canonical source:** Hugging Face dataset `https://huggingface.co/datasets/duke-trust-lab/When2Speak` (no separate official GitHub repository observed).
- **Pinned revision:** Observed `main` commits on Hugging Face: latest observed commit **`092e409`** (dated **2026-05-04**, README update; initial commit `73d2cf1`). Full 40-character commit SHA must be resolved via HF API / `git ls-remote` at manifest entry time.
- **Files needed & sizes:** Total repository size ~530 MB across two parallel task formats (Token vs Dialogue; 216,800 / 216,799 total examples across 16,000 conversations):
  - `finetune_test_token.jsonl` (Test, Token `<`/`>`): **21,687 examples**, **~26.1 MB**
  - `finetune_test_dialogue.jsonl` (Test, Dialogue `>` or response): **21,687 examples**, **~26.9 MB**
  - *(Other splits, not needed for `external` zero-tuning evaluation: `finetune_val_token.jsonl` 21,788 / ~26.3 MB; `finetune_train_token.jsonl` 173,325 / ~209 MB; `finetune_val_dialogue.jsonl` 21,788 / exact MB unverified; `finetune_train_dialogue.jsonl` 173,325 / ~215 MB.)*
- **Checksums:** No standalone `checksums.txt` observed in repo description; Hugging Face Git LFS pointers record per-file SHA-256 hashes (verifiable during fetch or via `--record-checksums`).
- **License & access:** **CC BY 4.0** (`CC-BY-4.0`) on Hugging Face dataset card (note: arXiv paper page carries `CC BY-SA 4.0`). Allows local copies, commercial use, and redistribution of derived files with attribution. No access gate.

#### `R-52` Official Evaluation Protocol
- **Primary metrics in registry:** `macro_f1`, `fir`, `mir`.
- **Official definitions (`arXiv:2605.05626` / HF card):**
  - **`macro_f1`:** $\frac{\mathrm{F1}_{\text{SPEAK}} + \mathrm{F1}_{\text{SILENT}}}{2}$.
  - **`fir` (False Interruption Rate):** $\frac{\mathrm{FP}}{\mathrm{FP} + \mathrm{TN}}$ (fraction of `SILENT` ground-truth turns where model speaks).
  - **`mir` (Missed Intervention Rate):** $\frac{\mathrm{FN}}{\mathrm{FN} + \mathrm{TP}}$ (fraction of `SPEAK` ground-truth turns where model stays silent).
- **Reporting split:** Conversation-level **Test split** (`finetune_test_token.jsonl` or `finetune_test_dialogue.jsonl`; 21,687 examples, zero conversation overlap with train/val). **Owner choice:** Token variant vs Dialogue variant.
- **Official scoring script:** Binary prediction extraction scripts hosted in `duke-trust-lab/When2Speak` (deterministic exact match on `<` vs `>` for Token, or `>` vs utterance for Dialogue; exact script filename unverified without cloning HF repo).
- **LLM judge required?** **No** (`macro_f1`, `fir`, `mir` are deterministic binary classification metrics).
- **Contamination:** Synthetic dataset released May 2026 (`arXiv:2605.05626`); minimal contamination risk for pre-May-2026 models. Caveat in `benchmark.rs`: fully synthetic (8-turn sliding windows from LLM-synthesized 2–6 speaker transcripts), so it is a stress test for FIR/MIR Pareto trade-offs, never sole proof of human timing.

#### `R-53` Mapping to Percepts
- **Unit-to-percept mapping:**
  - Each JSONL row holds `{"messages": [{"role": ..., "content": ...}]}` representing an 8-turn sliding window with anonymized speakers (`Speaker_0`, `Speaker_1`, ..., `[AGENT]`).
  - Each turn in the 8-turn window maps to a `Percept`: `actor` = speaker ID (`Speaker_k`), `place` = window/conversation ID, `audience` = speakers in window + soul, `payload` = utterance text.
  - At the end of the 8-turn window, if MonoAgens' Gate wakes AND reasoning emits an outbound chat `Action` (rather than staying silent), the prediction is `SPEAK` (`<`); otherwise `SILENT` (`>`).
- **Gold isolation:** The final assistant label (`<` / `>` / gold utterance) is stripped into `GoldBook` and never passed to the soul.
- **Representability caveats:**
  - **Missing wall-clock timestamps:** Synthetic 8-turn windows contain turn order only, **no inter-turn wall-clock timestamps**. Converter must assign a deterministic synthetic virtual step (e.g., `+1000 ms` per turn), which neutralizes time-decay / inter-turn gap realism.
  - Because each row is an isolated 8-turn sliding window, long-horizon episode memory across windows is absent unless conversation IDs are present and reconstructable (presence of explicit `conv_id` field inside JSONL rows is unverified until fetch; observed schema shows root key `messages`).

#### `R-54` Sampling and Power (`S >= 354–471`)
- **Dataset counts:** **21,687 test examples** (86.9% `SILENT` ≈ 18,846; 13.1% `SPEAK` ≈ 2,841; drawn from held-out portion of 16,000 conversations, 2–6 speakers).
- **Power feasibility & class-imbalance warning:** $N = 21,687 \gg 471$. However, at 13.1% `SPEAK`, a uniform sample of $S = 354\text{--}471$ contains only ~46–62 positive `SPEAK` examples, yielding wide CIs on `mir` ($\frac{\mathrm{FN}}{\mathrm{TP}+\mathrm{FN}}$).
- **Stratification:** Recommend either:
  1. Larger uniform/stratified sample **$S = 1,000\text{--}2,000$** (yields ~131–262 `SPEAK` examples, keeping `macro_f1` unweighted and directly comparable to full test), **or**
  2. Balanced/oversampled stratified sample ($S \ge 500$ with 250 `SPEAK` + 250 `SILENT` stratified by speaker count 2–6), keeping `fir` and `mir` unbiased while **reweighting precision/F1 back to the 13.1%/86.9% test prior** before computing `macro_f1`.
- **Per-item cost shape:** 8 prefix percepts (0 LLM calls) + 0 or 1 agent LLM call at decision turn (0 calls whenever MonoAgens' zero-LLM Gate predicts `SILENT`). **0 LLM judge calls.**

---

### 2.4 `ubuntu_irc` — Ubuntu IRC Disentanglement vs Ubuntu MPC (Dual-Corpus Collision)

#### `R-51` Provenance
Two distinct public benchmarks match the name/metrics in `benchmark.rs`:

- **Corpus A — Ubuntu IRC Disentanglement (`jkkummerfeld/irc-disentanglement`, Kummerfeld et al., ACL 2019 `P19-1374`)** *(matches `thread_f1`)*:
  - **Canonical source:** GitHub `https://github.com/jkkummerfeld/irc-disentanglement` (also mirrored on Hugging Face `https://huggingface.co/datasets/jkkummerfeld/irc_disentangle`).
  - **Pinned revision:** *Commit SHA unverified via search* (must pin GitHub commit SHA at manifest creation; ACL 2019 initial release + later ALTA additions).
  - **Files & sizes:** Total HF dataset size **~33.9 MB**; GitHub `data/train/` (67,463 msgs), `data/dev/` (2,500 msgs), `data/test/` (5,000 msgs), plus Channel Two / Elsner & Charniak (2,600 msgs) = **77,563** core annotated messages. Per-file types: `.raw.txt`, `.ascii.txt`, `.tok.txt`, `.annotation.txt`.
  - **Checksums:** None published in GitHub repo; use `--record-checksums` on fetch.
  - **License:** **Data: `CC BY 4.0`**; **Code & Tools: `ISC`**. Allows local copies, commercial use, and redistribution with attribution. No access gate.

- **Corpus B — Ouchi & Tsuboi (2016) Ubuntu MPC (`hiroki13/response-ranking`, EMNLP 2016 `D16-1231`)** *(matches `addressee_p1`)*:
  - **Canonical source:** GitHub `https://github.com/hiroki13/response-ranking` (`data/input/`, split gzipped archives e.g. `train-data.cand-10.1.gz` + `.2.gz`; also redistributed/preprocessed in `https://github.com/JasonForJoy/MPC-BERT` and `https://github.com/lxchtan/HeterMPC`). *(Note: Hu et al. 2019 GSN Ubuntu MPC is a further variant: 311,725 train / 5,000 dev / 5,000 test.)*
  - **Pinned revision & sizes:** *Commit SHA and exact archive byte sizes unverified via search.*
  - **License:** **`hiroki13/response-ranking` has NO explicit `LICENSE` file in its root directory** (built over public Ubuntu IRC logs).

#### `R-52` Official Evaluation Protocol
- **Primary metrics in registry:** `thread_f1`, `addressee_p1`.
- **Protocol mismatch / collision:**
  - **Corpus A (`jkkummerfeld/irc-disentanglement`) official protocol:**
    - Official scripts in repo: `tools/evaluation/dstc8-evaluation.py` (DSTC 8 Track 2 official script), `tools/evaluation/conversation-eval.py`, `tools/evaluation/graph-eval.py`.
    - Official metrics: **Matched Clusters Precision / Recall / F1 (`thread_f1`)**, **1-1 Overlap**, **$1 - \text{Scaled VI}$ (Variation of Information)**, **Adjusted Rand Index**, and **Graph Edge/Link F1** (`graph-eval.py` on reply-to parent edges).
    - **Reporting split:** `data/test/` (**5,000 messages**).
    - **Does NOT define `addressee_p1`.** (While `.annotation.txt` contains `child_idx parent_idx` reply-to edges from which a "parent message's speaker" accuracy could be derived, that would be a *non-official* metric and must be renamed under `benchmark-evaluation/spec.md`.)
  - **Corpus B (`hiroki13/response-ranking`, Ouchi & Tsuboi 2016) official protocol:**
    - Official metrics: **Addressee Recognition `P@1` (`addressee_p1`)**, Session Accuracy, and Response Selection $R_2@1, R_{10}@1$.
    - **Reporting splits:** Test sets for `Len-5` (32,668), `Len-10` (35,638), `Len-15` (35,385).
    - **Does NOT define `thread_f1`.**
- **LLM judge required?** **No** for either corpus (deterministic clustering/graph matching or discrete speaker P@1).
- **Contamination:** **High contamination risk** for both (published 2016 / 2019 from public Ubuntu `#ubuntu` IRC logs). Also note Run 2 caveat: explicit `@nick` mentions let supervised models reach 88–98% P@1 on Ubuntu MPC.

#### `R-53` Mapping to Percepts
- **Corpus A (`jkkummerfeld/irc-disentanglement`):**
  - `.raw.txt` provides explicit IRC `[HH:MM]` timestamps, speaker nicknames (`actor`), channel `#ubuntu` (`place`), channel participants (`audience`), and message text (`payload`).
  - **Evaluation output constraint:** MonoAgens' Core must NOT receive gold thread IDs or reply-to edges (`annotation.txt` -> `GoldBook`). To score `thread_f1`, the soul/evaluation probe must output either (a) predicted reply-to parent message ID per turn (from which connected components form threads), or (b) thread cluster assignments over the log window.
- **Corpus B (`hiroki13/response-ranking`, Ouchi & Tsuboi 2016):**
  - Preprocessed `Len-5`/`Len-10`/`Len-15` sessions provide `(speaker, utterance, addressee)` tuples **without wall-clock timestamps** (sequential turn index only). The final turn's gold addressee is masked into `GoldBook` and the soul predicts the addressed speaker `ActorId` from the candidate participants.

#### `R-54` Sampling and Power (`S >= 354–471`)
- **Corpus A (`jkkummerfeld/irc-disentanglement`):** `data/test/` contains **5,000 messages**.
  - **Stratification warning:** Disentanglement **cannot** be sampled as isolated random messages (doing so destroys thread continuity). Subsampling must select **contiguous log files / time windows** (e.g., full test files totaling $\ge 1,000$ messages, or all 5,000 test messages). Note: `thread_f1` treats threads/clusters as units — the exact number of gold threads across `data/test/`'s 5,000 messages is unverified until fetch; running **100% of `data/test/` (5,000 messages)** maximizes thread count and avoids arbitrary window slicing.
- **Corpus B (`hiroki13/response-ranking`):** `Len-5` = 32,668, `Len-10` = 35,638, `Len-15` = 35,385 test sessions. Easily supports $S = 471\text{--}500$ stratified equally across `Len-5`, `Len-10`, and `Len-15` (~160 per length bucket).
- **Per-item cost shape:**
  - Corpus A: Up to 5,000 turn-level percepts if scored turn-by-turn (or batched per log window). **0 LLM judge calls.**
  - Corpus B: 5–15 prefix percepts + 1 addressee prediction wake per session. **0 LLM judge calls.**

---

### 2.5 `molweni` — Molweni (`HIT-SCIR/Molweni`)

#### `R-51` Provenance
- **Public existence & canonical name:** Yes — **Molweni** (*"Molweni: A Challenge Multiparty Dialogues-based Machine Reading Comprehension Dataset with Discourse Structure"*, Jiaqi Li, Ming Liu, Min-Yen Kan, Zihao Zheng, Zekun Wang, Wenqiang Lei, Ting Liu, Bing Qin; COLING 2020, `2020.coling-main.238`).
- **Canonical source:** GitHub `https://github.com/HIT-SCIR/Molweni`.
- **Pinned revision:** *Commit SHA unverified via search* (must pin `HIT-SCIR/Molweni` commit SHA on fetch).
- **Files needed & sizes:**
  - **MRC task (`MRC/`):** `train.json` (8,771 dialogues / 24,682 QAs), `dev.json` (883 dialogues / 2,513 QAs), `test.json` (100 dialogues / 2,871 QAs) — total 30,066 QAs across 9,754 MRC dialogues (10,000 corpus dialogues overall).
  - **Discourse Parsing task (`DP/`):** `train.json` (9,000 dialogues), `dev.json` (500 dialogues), `test.json` (500 dialogues) — total 10,000 dialogues (88,303 utterances, 78,245 SDRT relations across 16 relation types).
  - *Exact file byte sizes and exact relative subdirectory names (`MRC/data/` vs `MRC/`) unverified via search.*
- **Checksums:** None published; requires `--record-checksums` on first fetch.
- **License & access (CORRECTION FROM RUN 2):** **`HIT-SCIR/Molweni` has NO explicit `LICENSE` file in its GitHub repository** (Run 2 mistakenly listed `Apache-2.0`). The COLING 2020 paper in ACL Anthology is `CC BY 4.0`, and the underlying dialogues come from the public Ubuntu Chat Corpus, but the dataset repository itself declares no formal SPDX license. No access gate. **Requires explicit owner license review.**

#### `R-52` Official Evaluation Protocol
- **Primary metrics in registry:** `qa_f1`, `sdrt_link_acc`.
- **Official definitions (`2020.coling-main.238`):**
  - **MRC (`qa_f1`):** SQuAD 2.0-style **token-level F1** (and **EM**, Exact Match) over answerable and unanswerable questions (unanswerable questions score 1.0 iff the system abstains / predicts no-answer).
  - **Discourse Parsing (`sdrt_link_acc` vs official `Link F1`):** Official Molweni DP literature reports **Link F1** (micro F1 over directed attachment edges `(head, tail)` between EDUs) and **Link & Relation F1** (`Link&Rel` F1 requiring both edge and one of 16 SDRT relation labels). Note: because SDRT dialogue graphs can contain **multi-parent** EDUs (DAGs rather than strict trees), **Link F1** is the canonical metric rather than single-parent attachment accuracy. Under `benchmark-evaluation/spec.md`, either implement official `Link F1` and rename `sdrt_link_acc -> sdrt_link_f1`, or define exact edge accuracy if single-parent decoding is enforced.
- **Reporting splits:** MRC `test.json` (2,871 questions across 100 dialogues) and DP `test.json` (500 dialogues; also sometimes reported as 1,107 last-utterance parsing instances depending on pipeline).
- **Official scoring script:** MRC evaluation follows SQuAD 2.0 `evaluate-v2.0.py` / `HIT-SCIR/Molweni` repo scripts; DP uses Shi & Huang (2019) / Molweni SDRT `Link F1` & `Link&Rel F1` graph evaluation. (Exact script filenames in `HIT-SCIR/Molweni` unverified via search.)
- **LLM judge required?** **No** (normalized span/abstention F1 and exact edge matching).
- **Contamination:** **High contamination risk** — published in 2020 from Ubuntu IRC logs and publicly hosted on GitHub since 2020.

#### `R-53` Mapping to Percepts
- **Unit-to-percept mapping:**
  - Each dialogue -> `PlaceId` (`dialogue_id`); each EDU/utterance (`{id, speaker, text}`) -> `Percept` with `actor = speaker`, `audience` = all dialogue speakers + soul, `payload = text`.
  - **MRC probes:** After dialogue utterances are ingested, each question in `qas` is delivered as a probe `Percept` from a dedicated `evaluator` `ActorId` in the same `place`, expecting a concise span answer or explicit abstention (`unanswerable`).
  - **DP probes:** Requires the soul to emit predicted directed dependency edges `(head_id, tail_id)` (and optionally relation type) via a structured evaluation capability after/during dialogue ingestion.
- **Gold isolation:** `answers`, `is_impossible`, and `relations` (`head`, `tail`, `type`) live strictly in `GoldBook`.
- **Representability caveats:**
  - **Missing wall-clock timestamps:** Molweni JSON dialogues store integer `id` (`0, 1, 2...`) only — **no timestamps**. Converter must inject deterministic fixed virtual timestamps (e.g., `+1000 ms` per EDU), so temporal decay is uniform rather than real IRC timing.
  - Utterances in Molweni are segmented into **EDUs** (elementary discourse units), which occasionally split a single speaker turn into sub-clauses.

#### `R-54` Sampling and Power (`S >= 354–471`)
- **Dataset counts:**
  - **MRC `test.json`:** **2,871 questions** across **100 dialogues** (25,779 answerable / 4,287 unanswerable across full 30,066 corpus ≈ 14.3% unanswerable).
  - **DP `test.json`:** **500 dialogues** (or 1,107 last-utterance instances in sentence-level DP pipelines).
- **Power feasibility:**
  - **MRC:** 2,871 questions $\gg 471$, **but** clustered inside only **100 dialogues** (~28.7 QAs/dialogue). Running a seeded stratified sample of $S = 500$ questions (or all 2,871) must compute dialogue-clustered CIs so correlated questions in the same 100 dialogues do not overstate precision.
  - **DP:** **500 dialogues $\ge 471$** — run **all 500 DP test dialogues (100%)** for full power with zero sampling bias!
- **Stratification (if subsampling MRC):** Stratify by answerability (`answerable` ~85.7% vs `unanswerable` ~14.3%, ensuring $\ge 70$ unanswerable probes) and question type (`who`/`what`/`how`/`why`/`when`/`where`), sampling balanced across all 100 test dialogues.
- **Per-item cost shape:**
  - MRC: ~8–15 prefix percepts per dialogue + 1 QA wake per question (if 100 dialogues amortized, ~880 prefix turns + 500–2,871 QA wakes). **0 LLM judge calls.**
  - DP: ~8–15 prefix percepts + 1 structured edge-prediction wake per dialogue = 500 wakes for full DP test. **0 LLM judge calls.**

---

## 3. Consolidated Summary Tables (`R-51`, `R-52`, `R-53`, `R-54`)

### Table 1: `R-51` Provenance

| `dataset_id` | Public? | Canonical Name | Canonical Source URL | Pinned Revision & Date | Needed Files & Observed Sizes | Source Checksums? | License (SPDX / Repo) & Terms | Access Gate |
|---|---|---|---|---|---|---|---|---|
| `pitch` | Yes | PITCH (`arXiv:2610.25896`) | `https://github.com/Social-Futures-Lab/pitch` (+ `pitch-human-baseline`) | *Unverified SHA* (Oct 2026 release) | `data/pitch-benchmark-data.json`, `data/human-baseline-data.csv` (*sizes unverified*) | No (`--record-checksums`) | **MIT**; local copy, commercial, & derived redistribution allowed w/ notice | None |
| `proact` | **No** | ProACT (`arXiv:2607.03730`) *(collide: `bosonai/proactbench`)* | `https://arxiv.org/abs/2607.03730` (no public repo/HF dataset) | N/A (`unavailable`) | None publicly released | N/A | Paper on arXiv only; **dataset unreleased** | **Unavailable** |
| `when2speak` | Yes | When2Speak (`arXiv:2605.05626`) | `https://huggingface.co/datasets/duke-trust-lab/When2Speak` | `092e409` (2026-05-04; full 40-char SHA to resolve on fetch) | `finetune_test_token.jsonl` (~26.1 MB) and/or `finetune_test_dialogue.jsonl` (~26.9 MB) | HF Git LFS SHA-256 pointers | **CC BY 4.0** (HF card; arXiv page: `CC BY-SA 4.0`); local, commercial, derived allowed w/ attribution | None |
| `ubuntu_irc` | Yes (**2 corpora**) | **A:** Ubuntu IRC Disentanglement<br>**B:** Ouchi & Tsuboi Ubuntu MPC | **A:** `https://github.com/jkkummerfeld/irc-disentanglement`<br>**B:** `https://github.com/hiroki13/response-ranking` | *Unverified SHAs* (A: ACL 2019; B: EMNLP 2016) | **A:** `data/test/*` (5k msgs; ~33.9 MB full HF repo)<br>**B:** `data/input/*` (`Len-5/10/15`, *sizes unverified*) | No (`--record-checksums`) | **A:** Data **`CC BY 4.0`**, Code **`ISC`**<br>**B:** **No `LICENSE` file in repo** | None |
| `molweni` | Yes | Molweni (`2020.coling-main.238`) | `https://github.com/HIT-SCIR/Molweni` | *Unverified SHA* (COLING 2020) | MRC `test.json`, DP `test.json` (*exact paths & byte sizes unverified*) | No (`--record-checksums`) | **No `LICENSE` file in GitHub repo** (COLING 2020 paper is `CC BY 4.0`; owner clearance required) | None |

---

### Table 2: `R-52` Official Evaluation Protocol

| `dataset_id` | Registry Primary Metrics | Official Definition | Reporting Split | Official Scoring Script & Revision | LLM Judge Required? | Known Contamination | Non-Official Metric Gaps / Renames |
|---|---|---|---|---|---|---|---|
| `pitch` | `live_best_turn_acc`, `in_window_acc` | Causal live turn-by-turn placement: exact match to ideal turn $t_i$ (`live_best_turn_acc`) or inside $[s_i, e_i]$ (`in_window_acc`), incl. `do-not-surface` | Full 35 convs / 384 candidates / 1,091 judgments (no train/test split) | Deterministic index/window scorer per `arXiv:2610.25896` (no standalone script observed in repo) | **No** | Low historical (Oct 2026 release) | Aligned (owner choice: 384 author vs 1,091 all judgments) |
| `proact` | `appropriateness`, `non_interruptiveness` | Breakdown-intervention binary scores (0/1) + Conciseness (0/1) & Quality (1–5) across 6 breakdown classes | Full 3,244 turn-level examples (`unavailable`) | Not publicly released | **Yes** (`GPT-5.4-Nano` in paper; prompt not publicly released) | GitHub issues & QMSum public; BEAM synthetic is July 2026 | Blocked (`unavailable`) |
| `when2speak` | `macro_f1`, `fir`, `mir` | $\frac{\mathrm{F1}_{\text{SPEAK}}+\mathrm{F1}_{\text{SILENT}}}{2}$; $\mathrm{FIR}=\frac{\mathrm{FP}}{\mathrm{FP}+\mathrm{TN}}$; $\mathrm{MIR}=\frac{\mathrm{FN}}{\mathrm{FN}+\mathrm{TP}}$ | `finetune_test_token.jsonl` or `finetune_test_dialogue.jsonl` (21,687 items) | Eval script in `duke-trust-lab/When2Speak` (deterministic `<` vs `>`) | **No** | Low historical (May 2026 synthetic release) | Aligned (synthetic stress test only) |
| `ubuntu_irc` | `thread_f1`, `addressee_p1` | **A:** Cluster/Matched-Cluster F1 (`thread_f1`), 1-1, $1-\text{Scaled VI}$, Link F1<br>**B:** Addressee `P@1` (`addressee_p1`), Session Acc, $R_{10}@1$ | **A:** `data/test/` (5,000 msgs)<br>**B:** `Len-5/10/15` test | **A:** `tools/evaluation/dstc8-evaluation.py`, `conversation-eval.py`, `graph-eval.py`<br>**B:** `hiroki13/response-ranking` / `MPC-BERT` | **No** | **High** (2016/2019 public Ubuntu IRC logs) | **Registry mixes A & B:** `addressee_p1` is not an official Corpus A metric (rename if derived as `parent_speaker_p1`, or split corpora) |
| `molweni` | `qa_f1`, `sdrt_link_acc` | **MRC:** SQuAD 2.0 token F1 & EM (w/ unanswerable abstention)<br>**DP:** `Link F1` & `Link&Rel F1` over 16 SDRT relations | MRC `test.json` (2,871 QAs / 100 dlgs); DP `test.json` (500 dlgs) | SQuAD 2.0-style MRC eval; SDRT `Link F1` graph eval in `HIT-SCIR/Molweni` | **No** | **High** (2020 public Ubuntu IRC + GitHub) | Official DP metric is **`Link F1`** (DAG multi-parent); rename `sdrt_link_acc -> sdrt_link_f1` or specify exact decoding rule |

---

### Table 3: `R-53` Mapping to Percepts

| `dataset_id` | Raw Units | Mapping to MonoAgens 6 Facts & Percepts | Expected Soul Output (`Action` / Silence) | Gold Held Apart (`GoldBook`) | Missing Facts / Honest-Mapping Caveats |
|---|---|---|---|---|---|
| `pitch` | 35 convs, 2,194 turns, 384 candidate msgs | Turn -> `Percept` (`actor`=speaker, `place`=conv_id, `audience`=group+soul, `payload`=text); candidate knowledge item staged as non-gold pending input | Causal turn-by-turn decision: emit chat `Action` surfacing candidate at turn $t$, or stay silent (`do-not-surface` if never emitted) | `ideal_turn` $t_i$, `acceptable_window` $[s_i, e_i]$, `do-not-surface` | Exact timestamp fields in `pitch-benchmark-data.json` **unverified until fetch**; chat-only |
| `proact` | Multi-user dialogue prefix snapshots (up to turn $t$) | Prefix turns -> `Percept`s (`actor`, `place`, `audience`, `payload`) | At snapshot turn $t$: stay silent or emit concise breakdown-repair `Action` | Intervene/silence label, breakdown class (1 of 6) | **Blocked (`unavailable`)**; single-turn snapshot (no user trust decay); timestamps unverified |
| `when2speak` | 8-turn `messages` JSONL windows (`Speaker_k`, `[AGENT]`) | Each window turn -> `Percept` (`actor`=`Speaker_k`, `place`=window_id, `audience`=window speakers+soul, `payload`=text) | End-of-window decision: speak (`<` / `Action`) vs remain silent (`>` / Gate hold) | Gold `<` (`SPEAK`) / `>` (`SILENT`) / target response | **No wall-clock timestamps** (synthetic turn order only -> fixed virtual $\Delta t$ neutralizes temporal decay testing) |
| `ubuntu_irc` | **A:** IRC `.raw.txt` stream w/ `[HH:MM]`<br>**B:** `Len-5/10/15` `(spk, utt, addr)` | **A:** Line -> `Percept` w/ parsed `[HH:MM]` `timestamp_ms`, `actor`=nick, `place`=`#ubuntu`, `payload`=msg<br>**B:** Turn -> `Percept` (`actor`, `place`=session, `payload`) | **A:** Output thread cluster IDs or reply-to parent ID per message via eval probe<br>**B:** Output predicted addressee `ActorId` | **A:** `.annotation.txt` reply-to edges & thread clusters<br>**B:** Gold addressee ID | **A:** Needs structured clustering/link probe<br>**B:** **No wall-clock timestamps** (turn index only) |
| `molweni` | Dialogues of `edus` (`{id, speaker, text}`), `qas`, `relations` | EDU -> `Percept` (`actor`=`speaker`, `place`=`dialogue_id`, `audience`=speakers+soul, `payload`=text); MRC Q -> probe `Percept` from `evaluator` actor | **MRC:** Answer span or explicit abstention (`unanswerable`)<br>**DP:** Predicted `(head, tail)` edges via eval probe | `answers`, `is_impossible`, SDRT `relations` (`head`, `tail`, `type`) | **No wall-clock timestamps** (integer `id` only -> fixed virtual $\Delta t$); EDUs may split single turns |

---

### Table 4: `R-54` Sampling and Power (`S >= 354–471`)

| `dataset_id` | Available Test Items | `S >= 354–471` Feasible? | Recommended Stratification & Sampling Rule | Rough Per-Item Cost Shape |
|---|---|---|---|---|
| `pitch` | 35 convs / **384 candidates** / **1,091 judgments** | **Borderline at candidate level (`384 >= 354`, `< 471`); Yes at judgment level (`1,091`)** | Run **100% of all 384 candidates** (no subsampling); report conversation-clustered CIs. If subsampling judgments, stratify by task (`planning` 14 vs `research` 21), `author`/`peer`, & `do-not-surface` | ~2,194 turn ticks multiplexed per conv (or ~384 × 30–63 prefix steps); Gate avoids LLM calls on silent turns; **0 judge calls** |
| `proact` | 3,244 total (`unavailable`) | Blocked until public release (would be Yes at $N = 3,244$) | If released: $S = 480$ stratified by source (`GitHub`, `QMSum`, `BEAM`) × action (`intervene`/`silence`) × 6 breakdown classes | 1 agent decision call (when Gate wakes) + **1 LLM judge call** (`GPT-5.4-Nano` / configured judge) |
| `when2speak` | **21,687 test items** (13.1% `SPEAK`, 86.9% `SILENT`) | **Yes** (warn: uniform $S=471$ gives only ~62 `SPEAK` items) | Recommend **$S = 1,000\text{--}2,000$** stratified by `SPEAK`/`SILENT` (preserving 13.1%/86.9%) & speaker count (2–6); or $S=500$ balanced w/ prior-reweighted `macro_f1` | 8 prefix percepts (0 LLM calls) + 0 or 1 agent call per window (Gate=0 on `SILENT`); **0 judge calls** |
| `ubuntu_irc` | **A:** 5,000 test msgs<br>**B:** 32,668–35,638 test sessions | **Yes** (Corpus A must keep contiguous windows) | **A:** Run **all 5,000 test msgs** (or contiguous whole log files $\ge 1,000$ msgs — never random independent msgs)<br>**B:** $S = 480$ stratified equally across `Len-5`, `Len-10`, `Len-15` | **A:** 5,000 msgs (batched/windowed disentanglement probes)<br>**B:** 5–15 prefix percepts + 1 wake/session; **0 judge calls** |
| `molweni` | **MRC:** 2,871 QAs (100 dlgs)<br>**DP:** 500 dlgs | **Yes** (`500 >= 471` for DP; `2,871` QAs across 100 dlgs for MRC) | **DP:** Run **all 500 test dialogues (100%)**<br>**MRC:** $S = 500$ QAs (or all 2,871) stratified by answerable (~85.7%) vs unanswerable (~14.3%) & question word across all 100 dlgs w/ dialogue-clustered CIs | **MRC:** ~8–15 turns/dlg + 1 wake/QA<br>**DP:** ~8–15 turns/dlg + 1 edge-probe wake/dlg; **0 judge calls** |
