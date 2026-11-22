# Proposal

## Why

In interactive multi-party group chats (`monoagens chat` after `/group ...`), MonoAgens currently falls back to neutral (`p_go = 0.00`) on unaddressed questions because:
1. Local config defaults to `[embed] backend = "none"` and points `[soul].model_path` to the minimal test fixture `crates/core/tests/fixtures/neural_v22_golden.safetensors` rather than trained V2.2 weights (`training/artifacts/neural_v22_best.safetensors`).
2. `cmd_chat` calls `GatewayHandle::submit_and_wait(Input::Percept(...))` directly without invoking `PreQueueService::enrich_percept_async`, and does not auto-launch the local `EmbeddingGemma 2` HTTP sidecar or surface asynchronous `OpenWait` wakeup replies while waiting on terminal input.
3. The V2.2 training pipeline (`augment.py` / `datasets.py`) teaches `Silent` on unaddressed chatter and `Wait` only when `is_named_soul = true`, so unaddressed group questions with high visible memory relevance (`max_visible_mem_cos >= 0.55`) predict `Note` instead of proactively helping (`Wait`/`Go`).

## What Changes

- Ensure `lab/adapters/eval_helpers/embeddinggemma2_sidecar.py` honors `instruction_prefix` from `SidecarEmbedRequest` so live `t_d` vectors match training shard encoding (`"title: none | text: ..."`).
- Update `cmd_chat` in `crates/cli/src/lib.rs` to enrich percepts via `handle.pre_queue_service()`, auto-start and cleanly shut down the local `EmbeddingGemma 2` sidecar when `embed.backend = "embeddinggemma2_http"` has `model_path` configured without a pre-bound `endpoint_url`, and asynchronously render delayed replies produced after `OpenWait` timers fire.
- Extend `training/src/monoagens_train/` with a clean OSI-permissive proactive group helping curriculum (`Regime 8` + bilingual Indonesian/English unanswered-question vs statement contrast episodes encoded via real `EmbeddingGemma 2`), retrain/calibrate the V2.2 controller on GPU, and verify zero regression on Lab `development` and `validation` suites plus `cargo test`.

## Capabilities

### New Capabilities
- `proactive-group-embed-s1`: Live `EmbeddingGemma 2` pre-queue enrichment, sidecar lifecycle, async `OpenWait` output in `monoagens chat`, and calibrated System 1 proactive group helping weights.

### Modified Capabilities
- *(none)*

## Impact

- `lab/adapters/eval_helpers/embeddinggemma2_sidecar.py`, `crates/cli/src/lib.rs`, `training/src/monoagens_train/`, and `training/artifacts/neural_v22_best.safetensors`.
- Zero changes to `crates/core`, `crates/valve`, `ARCHITECTURE.md`, `crates/core/tests/fixtures/neural_v22_golden.safetensors`, or `lab/scenarios/sealed/`.
