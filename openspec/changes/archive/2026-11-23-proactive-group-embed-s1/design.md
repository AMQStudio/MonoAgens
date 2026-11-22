# Design

## Architecture & Decisions

1. **Train/Serve Embedding Alignment (`embeddinggemma2_sidecar.py` + `crates/cli`)**:
   - `crates/reason/src/embed/embeddinggemma2.rs` sends `instruction_prefix: "title: none | text: "` for `TaskPrefix::Document` (`t_d`). `embeddinggemma2_sidecar.py` prepends `instruction_prefix` if non-empty and not already present, ensuring exact manifold parity with `training/src/monoagens_train/datasets.py`.
   - When `[embed] backend = "embeddinggemma2_http"` has `model_path` configured and no explicit `endpoint_url` / `MONOAGENS_EMBED_ENDPOINT` is supplied, `cmd_chat` spawns the persistent local `embeddinggemma2_sidecar.py` child on `127.0.0.1:0` (with an explicit `Drop` guard) and populates `cfg.embed.endpoint_url` before booting `GatewayRuntime`.
   - Before every `handle.submit_and_wait(Input::Percept(...))` in `cmd_chat` (both interactive and batch paths), if `handle.pre_queue_service()` is active, `pq.enrich_percept_async(&mut percept).await` populates `percept.embeddings` so `Core`'s `max_visible_mem_cos` and neural input `e_256` receive real semantic vectors.

2. **Asynchronous `OpenWait` Reply Rendering in `crates/cli`**:
   - In interactive mode, terminal `stdin` reads run on a dedicated OS thread forwarding lines through `tokio::sync::mpsc::unbounded_channel`.
   - Between prompts (and while waiting for `WaitDeferral` expiry), `tokio::select!` drains `rendered_bubbles` and `cue_rx` on a `50ms` interval, printing `[otak: Wait (Xms) ...]` and displaying `Mono> ...` automatically whenever `Core`'s `OpenWait` timer triggers a proactive reply.

3. **Proactive Group Helping Curriculum (`training/`)**:
   - Extend `training/src/monoagens_train/augment.py` and `datasets.py` with **Regime 8 (Proactive Group Helping)**:
     - **Positive (`Wait` ~2500ms -> `Go`, and direct `Go` on explicit unanswered info request with high `max_visible_mem_cos >= 0.65`)**: Unaddressed group questions where Mono holds relevant visible memory (`cos >= 0.55`) and no human has answered yet.
     - **Negative / Restraint (`Silent`)**: Casual group chatter, reactions/confirmations even with topical `max_visible_mem_cos`, rapid follow-up after `Soul` already spoke, and third-party answers resolving an `OpenWait`.
     - **Vectorless Invariance (`e_256 = 0`)**: Vectorless unaddressed group turns retain strict `Silent`/`Wait` restraint so offline `development`/`validation` Lab scenarios never suffer spurious false speaks.
   - Fine-tune/retrain V2.2 checkpoints with real `EmbeddingGemma 2` cached shards + `Stage4Preference` + temperature/CIC calibration, export to `training/artifacts/neural_v22_best.safetensors`, verify against `crates/lab` offline suites and `crates/cli` / `crates/core` tests.
