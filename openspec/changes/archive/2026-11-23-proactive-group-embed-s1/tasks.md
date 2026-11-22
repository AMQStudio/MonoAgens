## 1. Live EmbeddingGemma 2 & Async Chat Loop

- [x] 1.1 Update `lab/adapters/eval_helpers/embeddinggemma2_sidecar.py` to prepend `instruction_prefix` consistently with `TaskPrefix::Document` (`t_d`) and add unit coverage if applicable.
- [x] 1.2 Wire `PreQueueService::enrich_percept_async`, auto-managed `EmbeddingGemma2Http` local sidecar lifecycle (`SidecarProcessGuard`), and async `OpenWait` output polling into `cmd_chat` (`crates/cli/src/lib.rs`).

## 2. Proactive Group Helping System 1 Training & Verification

- [x] 2.1 Add Regime 8 proactive helping (synthetics + real `EmbeddingGemma 2` bilingual Indonesian/English unanswered-question vs statement/chatter trajectories) to `training/src/monoagens_train/` and run `pytest` in `training/`.
- [x] 2.2 Run multi-task V2.2 training + Stage 4 preference calibration into `training/artifacts/neural_v22_best.safetensors`, configure `/home/muqsith/.config/monoagens/monoagens.toml`, and verify `cargo test -p monoagens-cli`, `cargo clippy --workspace -- -D warnings`, `cargo run --release -p monoagens-lab -- compare` on `development` & `validation`, and `openspec validate proactive-group-embed-s1 --strict`.
