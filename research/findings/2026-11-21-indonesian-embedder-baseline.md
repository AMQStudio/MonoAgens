# Indonesian Informal Register `EmbeddingGemma 2` Baseline Probe Sweep (`$0` Local Offline)

- **Date:** 2026-11-21
- **Model:** Local `EmbeddingGemma 2` (`lab/datasets/embedder_models/embeddinggemma-2`, `NVIDIA GeForce RTX 4060 Laptop GPU`, `dtype=bfloat16`, `$0` API cost)
- **Diagnostic Datasets (Offline Component `EmbedderProbe` Only):**
  - `IndoCollex` (`MIT`, `data/full.csv`, `R-54` stratified sample `S = 471` with `SplitMix64(seed=1)`): colloquial Indonesian `(transformed, original-for)` pairs.
  - `MultiLexNorm` (`ind`: `861` non-identity pairs + `ind_eng`: `131` non-identity pairs = `992` total `orig -> norm` pairs from `test-00000-of-00001.parquet`).
- **Task-Prefix Regimes Evaluated:**
  - `t_q / t_d` *(canonical asymmetric retrieval)*: query `"task: search result | query: "`, document `"title: none | text: "`.
  - `t_sts` *(symmetric similarity)*: both sides `"task: sentence similarity | query: "`.
  - `unprefixed` *(`raw` baseline)*: empty prefix `""`.
- **Matryoshka Dimensions Evaluated:** `dim_d ∈ {768, 512, 256, 128}` (`[:dim_d]` + L2 normalization).
- **Raw Measurement Artifact:** [`indonesian_embedder_probe.json`](file:///home/muqsith/Projects/Mono/training/reports/indonesian_embedder_probe.json)

---

## 1. Full Sweep Table (`twin_retrieval_r1` Exact Index / Target-String Tie-Aware)

### A. `IndoCollex` (`S = 471`, Colloquial Indonesian Word Transformations)

| Prefix Regime | `dim_d` | `twin_r1` (Exact Idx) | `twin_r1` (Target-Str) | Mean Pos Cosine | Mean Hard-Neg Margin |
|---|---:|---:|---:|---:|---:|
| `t_q / t_d` | 768 | `0.4862` | `0.5265` | `0.8239` | `+0.0028` |
| `t_q / t_d` | 512 | `0.4862` | `0.5265` | `0.8263` | `+0.0021` |
| **`t_q / t_d`** | **256** | **`0.4735`** | **`0.5117`** | **`0.8399`** | **`+0.0001`** |
| `t_q / t_d` | 128 | `0.4544` | `0.4862` | `0.8948` | `-0.0053` |
| `t_sts` | 768 | `0.5329` | `0.5711` | `0.9062` | `+0.0056` |
| `t_sts` | 512 | `0.5244` | `0.5605` | `0.9060` | `+0.0054` |
| **`t_sts`** | **256** | **`0.5138`** | **`0.5520`** | **`0.9117`** | **`+0.0041`** |
| `t_sts` | 128 | `0.4989` | `0.5350` | `0.9457` | `+0.0002` |
| `unprefixed` | 768 | `0.4055` | `0.4395` | `0.8606` | `-0.0092` |
| `unprefixed` | 512 | `0.4055` | `0.4395` | `0.8649` | `-0.0093` |
| `unprefixed` | 256 | `0.4076` | `0.4374` | `0.8779` | `-0.0100` |
| `unprefixed` | 128 | `0.3822` | `0.4098` | `0.9215` | `-0.0094` |

### B. `MultiLexNorm` (`n = 992` Combined: `ind = 861`, `ind_eng = 131`)

| Prefix Regime | `dim_d` | Combined `R@1` (Exact / Str) | `ind` (`n=861`) `R@1` | `ind_eng` (`n=131`) `R@1` | Combined Margin |
|---|---:|---:|---:|---:|---:|
| `t_q / t_d` | 768 | `0.8921` / `0.8972` | `0.8780` | `0.9924` | `+0.0881` |
| `t_q / t_d` | 512 | `0.8911` / `0.8972` | `0.8769` | `0.9924` | `+0.0861` |
| **`t_q / t_d`** | **256** | **`0.8851` / `0.8911`** | **`0.8688`** | **`0.9924`** | **`+0.0774`** |
| `t_q / t_d` | 128 | `0.8629` / `0.8710` | `0.8455` | `0.9924` | `+0.0495` |
| `t_sts` | 768 | `0.9325` / `0.9415` | `0.9233` | `0.9924` | `+0.0706` |
| `t_sts` | 512 | `0.9315` / `0.9405` | `0.9233` | `0.9924` | `+0.0698` |
| **`t_sts`** | **256** | **`0.9335` / `0.9435`** | **`0.9268`** | **`0.9924`** | **`+0.0642`** |
| `t_sts` | 128 | `0.9163` / `0.9274` | `0.9048` | `0.9924` | `+0.0372` |
| `unprefixed` | 768 | `0.8185` / `0.8276` | `0.7933` | `0.9924` | `+0.0596` |
| `unprefixed` | 512 | `0.8216` / `0.8317` | `0.7967` | `0.9924` | `+0.0573` |
| `unprefixed` | 256 | `0.8135` / `0.8226` | `0.7863` | `0.9924` | `+0.0504` |
| `unprefixed` | 128 | `0.7863` / `0.7954` | `0.7526` | `0.9924` | `+0.0316` |

---

## 2. Key Analysis & Prefix-Impact Findings

1. **Task-prefixing produces a massive accuracy jump (`+6.6` to `+12.0` pp at `256d`):**
   - Leaving `EmbeddingGemma 2` unprefixed severely degrades Indonesian colloquial & slang retrieval (`IndoCollex 256d`: `0.4076`, `mean_margin = -0.0100`; `MultiLexNorm 256d`: `0.8135`).
   - Adding canonical retrieval prefixes (`t_q / t_d`) immediately gains **`+6.59 pp`** on `IndoCollex 256d` (`0.4076 → 0.4735`) and **`+7.16 pp`** on `MultiLexNorm 256d` (`0.8135 → 0.8851`, `+8.25 pp` on `ind`).
   - Symmetric `t_sts` prefixes (`"task: sentence similarity | query: "`) perform best overall on twin equivalence matching (`IndoCollex 256d`: **`0.5138`**, `+10.62 pp` over unprefixed; `MultiLexNorm 256d`: **`0.9335`**, `+12.00 pp` over unprefixed, `0.9268` on `ind`), while `t_q / t_d` maintains stronger negative-distractor cosine separation (`mean_margin = +0.0774` vs `+0.0642` on `MultiLexNorm 256d`).
2. **Matryoshka `256d` truncation preserves almost all `768d` capacity:**
   - Moving from `768d → 256d` drops only `-1.27 pp` on `IndoCollex t_q/t_d` (`0.4862 → 0.4735`) and `-0.70 pp` on `MultiLexNorm t_q/t_d` (`0.8921 → 0.8851`), while on `t_sts` `MultiLexNorm 256d` (`0.9335`) matches or edges `768d` (`0.9325`).
   - Dropping further to `128d` incurs a noticeable loss (`-1.91 pp` on `IndoCollex t_q/t_d`, `-2.22 pp` on `MultiLexNorm t_q/t_d`, and negative hard-negative margin `-0.0053` on `IndoCollex`), confirming **`256d` is the right Pareto knee** for MonoAgens memory partitions.
3. **Sentence context (`MultiLexNorm`) vs isolated single words (`IndoCollex`):**
   - With full conversational sentence context (`MultiLexNorm`), `EmbeddingGemma 2 (256d)` already achieves **`88.5%–93.4%` R@1** (`99.24%` exact-index / `100.0%` target-string on `ind_eng`).
   - Isolated single-word slang transforms without sentence context (`IndoCollex`) remain challenging at `47.4%–51.4%` R@1 (`mean_margin ≈ 0`), indicating downstream Core recall benefits from storing conversational context/gists alongside slang rather than bare isolated tokens.
