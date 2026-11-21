# Run 5 Supporting Note 2: Dataset & Multi-Stage Training Matrix (`R-62`)

- **Source Baseline:** [`research/findings/2026-11-17-run-3.md`](file:///home/muqsith/Projects/Mono/research/findings/2026-11-17-run-3.md) (`R-51`–`R-54`) + primary dataset licenses verified in Run 3.
- **Anti-Leakage Rule:** `lab/scenarios/sealed/` is **never** read, indexed, or sampled. Any dataset split used in Lab validation/test reporting MUST be held out strictly from neural controller training.

---

## 1. Public Datasets Mapped to Neural Controller Heads

| Dataset ID | License (Run 3 Verified) | Scale / Splits | Target Head(s) | Training Suitability & Caveat |
|---|---|---|---|---|
| **`when2speak`** | `CC-BY-4.0` (HF) | `21,687` test windows + full `~530 MB` repo (`train`/`val`/`test`) | `gate_4` (`Silent` vs `Go`) | **Primary turn-taking corpus.** Partition strictly by `dialogue_id` so `finetune_test_*` never leaks into train. |
| **`ubuntu_irc` (A)** | `CC-BY-4.0` data / `ISC` code | `~33.9 MB` HF repo (`train`/`dev`/`test`, `5k` test msgs) | `gate_4` + `wait_delta_t` + `p_memory_relevance` | **Cleanest native multi-party timestamps** (`[HH:MM]`). Provides real inter-turn latency for `wait_delta_t`. |
| **`proactive_bench`** | `Apache-2.0` | `train_data.jsonl` + `trainset_reward_llama.json` (`233` test events) | `gate_4` (`Silent`/`Note`/`Go`) | **Permissive OSI.** Good for proactive `propose_assistance` vs `NOOP` supervision. |
| **`pitch`** | `MIT` | `35` convs, `384` candidates, `1,091` judgments | `gate_4` + `wait_delta_t` | Small (`35` convs); best used as **calibration/validation** or few-shot fine-tune, not primary pretraining. |
| **`capa`** | `MIT` code + `CC-BY-4.0` AMI | `137` AMI meetings (`97` train / `20` dev / `20` test; `1,126` episodes) | `gate_4` (`Silent` vs `Go`) | Train strictly on the `97` train meetings; keep `20` test meetings held out for Lab. |
| **`group_mem_bench`** | `CC-BY-4.0` (HF) + `MIT` (GH) | `4` domains, `~159 MB` convs, `745` QAs | `p_memory_relevance` + `p_encode_importance` | Native timestamps + `reply_to` + `VisibleScope` multi-party channels. |
| **`longmemeval`** | `MIT` | `500` scenarios (`s_cleaned` `277 MB`) | `p_memory_relevance` + `p_uncertainty` (abstention `_abs`) | `_abs` questions directly supervise `p_uncertainty` / `p_verify` abstention; hold out eval split. |
| **`memory_agent_bench`** | `MIT` wrapper | `~76.6 MB` Parquet splits | `p_memory_relevance` + `p_encode_importance` | `Conflict_Resolution` supervises memory update/forgetting importance. |
| **`stale`** | `CC-BY-4.0` + `MIT` | `400` scenarios (`1,200` queries) | `p_encode_importance` + `p_verify` | Supervises stale-belief resistance (`SR`/`PR`). Hold out eval subset. |
| **`hi_tom`** | `Apache-2.0` | `1,200` QAs (`~240` stories) | `p_uncertainty` (ToM observability) | Private `Tell` maps to `VisibleScope`; useful for verifying `p_verify` under partial observability. |
| **`dyntom`** | `Apache-2.0` | `78,100` MCQs (`376 MB`) | `p_uncertainty` / `p_encode_importance` | Permissive `Apache-2.0` temporal state-transition dataset. |
| **`MultiWOZ 2.2`** | `MIT` | `~10,438` dialogues | `gate_4` / `p_uncertainty` (clarification) | Open-domain task-oriented dialogue acts; permissive `MIT` license. |
| **`locomo`, `opentom`, `exploretom`, `tomi`** | **`CC-BY-NC-4.0`** *(non-commercial)* | — | **Exclude from default commercial weights** | `opentom` explicitly forbids LLM training/fine-tuning; `CC-BY-NC-4.0` blocks commercial distribution of trained weights. |
| **`pm_bench`, `molweni`, `ubuntu_irc` (B), `multilexnorm`** | **Missing/ambiguous `LICENSE`** | — | **Exclude until owner clearance** | Do not bake into out-of-the-box distributed weights without explicit license clearance. |
| **`pis`, `proact`, `mpbench`** | **`unavailable`** | — | **N/A** | No public dataset exists. |

---

## 2. RTX 4060 Laptop (8 GB VRAM, 14 GiB System RAM) Budget Estimates

Because `EmbeddingGemma 2` (`256d`) embeddings are **precomputed once and cached** as memory-mapped `.npy` / `.parquet` / `safetensors` shards on disk, training the Neural Cognitive Controller itself (`274d -> 50k..3M params`) is lightweight:

| Model Budget | Batch Size (`B`) | FP32 Params + AdamW States + Grads | Peak VRAM (w/ activations, `B=1024`) | Epoch Time (`100k` events, RTX 4060) |
|---|---|---|---|---|
| **50k params** | 1,024 | ~0.8 MB | < 150 MB | ~0.5–1.5 s |
| **200k params** | 1,024 | ~3.2 MB | < 250 MB | ~1–2.5 s |
| **1M params** | 1,024 | ~16 MB | < 450 MB | ~2–5 s |
| **3M params** | 1,024 | ~48 MB | < 800 MB | ~5–12 s |

*(Note: Exact wall-clock seconds depend on DataLoader I/O and sequence length `T` if using `minGRU`/`GRU` over `B=128, T=64` windows; even with `T=64`, peak VRAM for `<= 3M` params stays well under `2 GB` on an 8 GB RTX 4060. Precomputing `EmbeddingGemma 2` embeddings on 100k turns is the dominant one-time cost, fitting comfortably in 8 GB VRAM in `int8`/`bf16`.)*
