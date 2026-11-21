# neural-training-pipeline Specification

## Purpose
Defines the offline Python/PyTorch training, license-hygiene, probability calibration, and `safetensors` export workflow that produces out-of-the-box weights for the Pure-Rust Neural Cognitive Controller on consumer hardware (RTX 4060 8 GB VRAM).

## Requirements

### Requirement: OSI-permissive training corpus with strict anti-leakage hygiene
The training pipeline SHALL construct distributable default weights exclusively from verified OSI-permissive datasets (`MIT`, `Apache-2.0`, `CC-BY-4.0`, `ISC`) and SHALL enforce strict split isolation so no Lab validation split, held-out test split, or `lab/scenarios/sealed/` content ever enters training or calibration.

#### Scenario: Non-commercial or unlicensed datasets rejected for default weights
- **WHEN** a dataset manifest entry carries a non-commercial (`CC-BY-NC-4.0`) or missing license and the build targets default distributable weights
- **THEN** the data preparation step excludes that dataset and logs the exclusion reason explicitly

#### Scenario: Sealed directory is never accessed
- **WHEN** any data preparation, training, or calibration script executes
- **THEN** `lab/scenarios/sealed/` is never read, listed, or referenced

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

### Requirement: Reproducible `safetensors` export with Rust parity verification
The training pipeline SHALL export trained weights in `safetensors` format accompanied by a `BLAKE3` checksum, architecture metadata (`feature_schema_version` `1` or `2`, layer dimensions, active head flags including `has_v22_heads`, `h_actor_dim`, and embedding contract keys), and golden test fixtures verified against the Rust forward pass within `1e-5` floating-point tolerance for both `v1` (`18d`) and `v2` (`32d` + V2.2 heads + optional `h_actor`) artifacts.

#### Scenario: Python-to-Rust forward-pass parity check passes
- **WHEN** the exported `v1` or `v2` golden fixture (inputs + expected PyTorch outputs) is evaluated by the Rust `neural-controller` test suite
- **THEN** all output logits (`gate_4`, `wait_delta_t`, optional Phase 2/3, `compute_tier`, `p_user_correction`) and updated `h_t` / `h_actor` states match PyTorch within `1e-5` absolute tolerance

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

### Requirement: Indonesian and multi-party pragmatic curricula with OSI-clean SEA dataset ingestion
The training pipeline SHALL extend `augment.py` and `datasets.py` for `schema = 2` (`32d` `x_num`) with *pecah balon* burst patience, spherical code-switch drift, multi-party self-repair, peer-agent, `p_user_correction`, and `compute_tier` curricula, and SHALL ingest only `ALLOWED_OSI_SPDX` SEA/preference corpora while rejecting non-commercial and ShareAlike datasets.

#### Scenario: Schema v2 procedural curricula cover pecah balon, code-switch drift, self-repair, and peer-agent coordination
- **WHEN** `augment.py` generates a `schema = 2` training split
- **THEN** every sequence emits finite `[0, 1]`-bounded `32d` `x_num` vectors plus calibrated `y_gate`, `y_wait_ms`, `y_compute_tier`, and `y_user_correction` targets covering Regimes 0–5, multi-party self-repair, and peer-agent ping-pong/deferral

#### Scenario: Non-OSI SEA and turn-taking corpora are blocked from commercial training builds
- **WHEN** dataset ingestion evaluates manifests containing `CULTURETALK-ID` (`CC-BY-NC-SA-4.0`), `Speak or Stay Silent` (`CC-BY-NC-ND-4.0`), or `PingPong` (`CC-BY-SA-4.0`) for distributable weights
- **THEN** `licenses.py` rejects those datasets explicitly while admitting OSI-permissive `SEADialogues`, `IndoDialect`, `PerMemBench`, `PersonaMem-v2`, and `VARS`
