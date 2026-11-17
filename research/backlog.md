# Research & Calibration Backlog

> Synced with [`openspec/changes/core-and-lab/design.md`](../openspec/changes/core-and-lab/design.md) §19 and the current workspace verification state. Every open empirical item below is explicitly unmeasured unless backed by an observed local test or benchmark run.

---

## 1. Open Empirical & Hardware Calibration Items (§19)

| Item | Code Status in Repo | Calibration / Measurement Status |
|---|---|---|
| **`EmbeddingGemma 2` edge latency (`439M`/`577M`/`740M` int4/int8) & Indonesian/Gaul `256d` vs `512d` curve (`A-15`, `R-37`)** | `EmbeddingGemma2Embedder` & HTTP/stdio sidecars implemented in [`crates/reason/`](../crates/reason/) and [`crates/gateway/src/prequeue.rs`](../crates/gateway/src/prequeue.rs); verified offline with `prequeue_tool_rag_tests` (`4/4` passing) and `reason_suite` (`7/7` passing). | **UNMEASURED** on target edge CPUs; `MultiLexNorm`/`IndoCollex` `256d` vs `512d` recall curve awaits local embedding runtime & Lab sweep (`lab/runs/` does not exist). |
| **Indonesian + code-mixed `VAP` (`tau_barge`, `theta_turn_end`) & owner hourly approval cap `B_hr` (`R-37`, `R-42`)** | Voice barge-in defaults (`BARGE_IN_VAD_THRESHOLD = 0.70`, `BARGE_IN_DURATION_MS = 120`, `webrtc-vad` fallback) implemented in [`bodies/body-voice/src/audio.rs`](../bodies/body-voice/src/audio.rs) (`3/3` unit tests on synthetic buffers + `B-MUST-01..07`); `MAX_PROMPTS_PER_HOUR = 6` in [`crates/valve/src/approvals.rs`](../crates/valve/src/approvals.rs) (`valve_v2` `6/6`). | **UNMEASURED** on real human voice audio & warning-fatigue sweeps: Silero ONNX model weights are not present on host, so `body-voice` barge-in latency was verified under `simulated_clock` only. |
| **Comparative `S >= 354–471` non-inferiority & 5-domain Pareto scoreboard (`A-16`, `R-39`)** | Harness, external baseline wrappers, and convex Pareto logic implemented & unit-tested in [`crates/lab/`](../crates/lab/) (`21/21` tests passing across `baseline_and_pareto.rs`, `lab_harness.rs`, `property_suites.rs`, `security_and_ablation.rs`). | **UNMEASURED**: No `lab/runs/` benchmark archive exists on disk; live Lab runs invoke paid models and run only on explicit owner request. |
| **Post-`hard_forget` rollback-replay vs freeze (`R-34`)** | `Two-Epoch Replay` anchor preservation implemented in [`crates/core/src/replay.rs`](../crates/core/src/replay.rs) (`beliefs_sleep_c6::hard_forget_recomputes_credence_and_preserves_two_epoch_replay`) and verified in `C-MUST-02`. | **OPEN EXPERIMENT**: Default freezes pre-forget adaptive weights at the epoch snapshot; multi-month comparative drift against full rollback-replay awaits empirical trace suites. |
| **Mechanism pruning (`R-01`, `A-02`)** | All 22 mechanisms registered with mandatory neutral baseline in [`crates/core/`](../crates/core/) and audited via [`crates/lab/src/main.rs`](../crates/lab/src/main.rs). | **OPEN EXPERIMENT**: Empirical ablation delta per mechanism (`D18` rows 1–22) awaits full Lab run before pruning zero-effect rows. |
| **External benchmark datasets provenance, alignment, percept mapping & power (`R-51`–`R-54`)** | Manifest schema, pin/fetch/verify CLI, `Converter`/`Scorer`/`EmbedderProbe` framework, `--live` preflight guard, and per-dataset modules implemented in [`crates/lab/src/datasets/`](../crates/lab/src/datasets/) & [`lab/datasets.manifest.toml`](../lab/datasets.manifest.toml); see [`2026-11-17-run-3.md`](findings/2026-11-17-run-3.md) and [`run-3-notes/`](findings/run-3-notes/) ([Memory](findings/run-3-notes/01_memory_datasets.md), [Prospective](findings/run-3-notes/02_prospective_and_proactive.md), [Timing & Group Chat](findings/run-3-notes/03_timing_and_group_chat.md), [Theory of Mind](findings/run-3-notes/04_theory_of_mind.md), [Indonesian & Security](findings/run-3-notes/05_indonesian_and_security.md)). | **SYNTHETICALLY VERIFIED OFFLINE** (`R-51`–`R-54` cataloged; `pis`/`proact`/`mpbench` marked `unavailable`); owner `datasets pin`, license clearances, `datasets fetch`, and `--live` runs remain gated on Group 8. |

---

## 2. Runtime & Integration Gaps Observed in Workspace

- **ROS 2 `Lyrical Luth` runtime integration (`bodies/plugin-ros2-bridge`)**: `Ros2BridgeBody` compiles cleanly with default features and exits non-zero when no ROS 2 runtime is present (`2/2` unit tests). Live `rclrs = "0.8"` DDS transport is **UNVERIFIED** (`NO_ROS2_INSTALLED` on host).
- **Silero VAD weights (`bodies/body-voice`)**: Fallback to `webrtc-vad = "0.4.0"` verified; live Silero ONNX runtime latency on microphone hardware is **UNVERIFIED** (`NO_SILERO_FOUND` on host).
- **Gateway inbound `approval/resolve` write seam (`crates/gateway/src/waist2.rs:709-716`)**: Currently returns `-32601`; therefore `monoagens approvals grant|deny` in [`crates/cli/`](../crates/cli/) fails explicitly with a clear blocked message rather than mutating state (`approvals list` reads `PendingApproval` read-only).
- **Gateway HTTP mock rate-limit test**: In `cargo test -p monoagens-gateway`, `gateway_tests::mock_http_provider_sends_store_false_tracks_usage_and_redacts_canary_on_429_and_500` failed once (`left: Failed(RateLimit), right: Success`), while `24/25` remaining Gateway tests passed.

---

## 3. Target Microbenchmarks vs Observed Measurements

- **`Valve::action` (`door_verdict` criterion bench)**: [`crates/valve/benches/door_verdict.rs`](../crates/valve/benches/door_verdict.rs) measured `[12.814 µs, 13.096 µs, 13.454 µs]` (vs `< 50 µs` p99 design budget in `waist-valve-and-bodies`).
- **`Core` & `Store` storage/retrieval latency targets**: Sub-millisecond storage targets (`hot.db` + `usearch`) remain unbenchmarked by an end-to-end load sweep in this environment (`lab/runs/` empty).
