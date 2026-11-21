## MODIFIED Requirements

### Requirement: Reproducible `safetensors` export with Rust parity verification
The training pipeline SHALL export trained weights in `safetensors` format accompanied by a `BLAKE3` checksum, architecture metadata (`feature_schema_version` `1` or `2`, layer dimensions, active head flags including `has_v22_heads`, `h_actor_dim`, and embedding contract keys), and golden test fixtures verified against the Rust forward pass within `1e-5` floating-point tolerance for both `v1` (`18d`) and `v2` (`32d` + V2.2 heads + optional `h_actor`) artifacts.

#### Scenario: Python-to-Rust forward-pass parity check passes
- **WHEN** the exported `v1` or `v2` golden fixture (inputs + expected PyTorch outputs) is evaluated by the Rust `neural-controller` test suite
- **THEN** all output logits (`gate_4`, `wait_delta_t`, optional Phase 2/3, `compute_tier`, `p_user_correction`) and updated `h_t` / `h_actor` states match PyTorch within `1e-5` absolute tolerance

## ADDED Requirements

### Requirement: Indonesian and multi-party pragmatic curricula with OSI-clean SEA dataset ingestion
The training pipeline SHALL extend `augment.py` and `datasets.py` for `schema = 2` (`32d` `x_num`) with *pecah balon* burst patience, spherical code-switch drift, multi-party self-repair, peer-agent, `p_user_correction`, and `compute_tier` curricula, and SHALL ingest only `ALLOWED_OSI_SPDX` SEA/preference corpora while rejecting non-commercial and ShareAlike datasets.

#### Scenario: Schema v2 procedural curricula cover pecah balon, code-switch drift, self-repair, and peer-agent coordination
- **WHEN** `augment.py` generates a `schema = 2` training split
- **THEN** every sequence emits finite `[0, 1]`-bounded `32d` `x_num` vectors plus calibrated `y_gate`, `y_wait_ms`, `y_compute_tier`, and `y_user_correction` targets covering Regimes 0–5, multi-party self-repair, and peer-agent ping-pong/deferral

#### Scenario: Non-OSI SEA and turn-taking corpora are blocked from commercial training builds
- **WHEN** dataset ingestion evaluates manifests containing `CULTURETALK-ID` (`CC-BY-NC-SA-4.0`), `Speak or Stay Silent` (`CC-BY-NC-ND-4.0`), or `PingPong` (`CC-BY-SA-4.0`) for distributable weights
- **THEN** `licenses.py` rejects those datasets explicitly while admitting OSI-permissive `SEADialogues`, `IndoDialect`, `PerMemBench`, `PersonaMem-v2`, and `VARS`
