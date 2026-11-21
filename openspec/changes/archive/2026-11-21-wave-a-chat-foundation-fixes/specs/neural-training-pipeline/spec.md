# Spec Delta — neural-training-pipeline

## MODIFIED Requirements

### Requirement: Precomputed frozen embedding training within 8 GB VRAM budget
The training pipeline SHALL precompute and cache frozen `256d` `EmbeddingGemma 2` vectors once per dataset split using the exact canonical runtime `TaskPrefix` instruction formatting (`crates/reason/src/embed/mod.rs`) and recording `expected_embed_model_ver` and `expected_prompt_regime_id` in exported `safetensors` metadata so that training `50k`–`3M` parameter controllers runs comfortably within an 8 GB VRAM / 14 GiB RAM laptop budget without train/serve manifold drift.

#### Scenario: Parameter-scale sweep runs offline over cached embeddings
- **WHEN** the trainer precomputes or loads cached embeddings and runs a parameter sweep over `150k`, `500k`, `1M`, and `3M` configs
- **THEN** every text input is encoded with the matching runtime `TaskPrefix` instruction string (`bf16` on CUDA / `fp32` on CPU, L2-normalized at `256d`), exported artifacts embed `expected_embed_model_ver` and `expected_prompt_regime_id`, and training completes without invoking the embedding model per epoch

### Requirement: Post-hoc probability calibration and finite-sample risk control before export
Before exporting weights for Rust inference, the training pipeline SHALL fit post-hoc temperature scaling (`T > 0`) and a non-vacuous finite-sample Confidence-Interval Calibration (`CIC`) abstention threshold strictly above the 4-class simplex floor (`tau > 0.25`, targeting `target_risk <= 0.05` at `delta = 0.05`) on the held-out calibration split, and report single-turn `ECE`, multi-turn Trajectory-Checkpoint `TC-ECE`, `Brier` score, and selective-prediction `AURC`.

#### Scenario: Temperature scaling and CIC risk thresholds are embedded in exported artifact
- **WHEN** calibration completes on the held-out calibration split
- **THEN** the fitted temperature scalar (`T > 0`) and a non-vacuous `cic_threshold > 0.25` are stored in the exported `safetensors` metadata and validated not to degrade validation NLL

## ADDED Requirements

### Requirement: Stage 4 offline preference and counterfactual post-training
The training pipeline SHALL support an offline Stage 4 post-training pass on the frozen-embedding Phase 3 checkpoint combining closed-form categorical pairwise DPO/margin loss, asymmetric unpaired KTO loss (`lambda_u > lambda_d`), and counterfactual `(e_256, 0_256)` vectorless invariance (`CFPO`) without adding runtime parameters or inference latency, validated via Rust parity (`<= 1e-5`) and `--release` 5-seed Lab offline suites.

#### Scenario: Stage 4 post-training recovers habituation and patience without validation regression
- **WHEN** Stage 4 DPO/KTO + `CFPO` post-training runs on the Phase 3 `1M + Gated DeltaNet-2` checkpoint on RTX 4060
- **THEN** the exported artifact keeps identical parameter count and passes Rust parity within `1e-5`, improves `development` habituation (`dev-07`) and patience (`dev-08`) pass rates, and maintains `10/10` PASS and `when_to_speak_f1 = 1.0` on `validation`

### Requirement: Zero-API Indonesian multilingual embedder probe baseline
The offline evaluation tooling SHALL benchmark the locally pinned `EmbeddingGemma 2` embedder at `$0` API cost on `IndoCollex` (`MIT`) and `MultiLexNorm` (`ind` + `ind_eng`, offline diagnostic only) across Matryoshka dimensions (`768d`, `512d`, `256d`, `128d`) and task-prefix regimes (`t_q/t_d`, `t_sts`, unprefixed), recording measured `twin_retrieval_r1` curves in `research/findings/`.

#### Scenario: Offline Indonesian MRL and prefix sweep produces measured baseline
- **WHEN** the zero-API Indonesian embedder probe benchmark runs against local `EmbeddingGemma 2` weights
- **THEN** it computes and records exact `twin_retrieval_r1` scores across `(768d, 512d, 256d, 128d)` and `(t_q/t_d, t_sts, unprefixed)` for `IndoCollex` and `MultiLexNorm` without making any network or paid LLM API calls
