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
The training pipeline SHALL precompute and cache frozen `256d` `EmbeddingGemma 2` vectors once per dataset split so that training `50k`–`3M` parameter controllers runs comfortably within an 8 GB VRAM / 14 GiB RAM laptop budget.

#### Scenario: Parameter-scale sweep runs offline over cached embeddings
- **WHEN** the trainer runs a Phase 1 sweep over `150k`, `500k`, `1M`, and `3M` parameter configs on cached embeddings
- **THEN** each run completes without invoking the embedding model per epoch and logs validation loss, macro-F1, `ECE`, `Brier`, and `AURC`

### Requirement: Post-hoc probability calibration and finite-sample risk control before export
Before exporting weights for Rust inference, the training pipeline SHALL fit post-hoc temperature scaling (`T > 0`) and finite-sample Confidence-Interval Calibration (`CIC`) abstention thresholds on the held-out calibration split, and report single-turn `ECE`, multi-turn Trajectory-Checkpoint `TC-ECE`, `Brier` score, and selective-prediction `AURC`.

#### Scenario: Temperature scaling and CIC risk thresholds are embedded in exported artifact
- **WHEN** calibration completes on the held-out calibration split
- **THEN** the fitted temperature scalar(s) and `CIC` abstention threshold(s) are stored in the exported `safetensors` metadata/tensors and validated not to degrade validation NLL

### Requirement: Reproducible `safetensors` export with Rust parity verification
The training pipeline SHALL export trained weights in `safetensors` format accompanied by a `BLAKE3` checksum, architecture metadata (`feature_schema_version`, layer dimensions, active head flags), and a golden test fixture verified against the Rust forward pass within `1e-5` floating-point tolerance.

#### Scenario: Python-to-Rust forward-pass parity check passes
- **WHEN** the exported golden fixture (inputs + expected PyTorch outputs) is evaluated by the Rust `neural-controller` test suite
- **THEN** all output logits and updated `h_t` states match PyTorch within `1e-5` absolute tolerance
