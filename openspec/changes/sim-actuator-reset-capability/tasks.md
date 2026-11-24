# Tasks — `sim-actuator-reset-capability`

## Tasks

- [x] Add `sim.reset` CapabilityDescriptor to `descriptors()` in `plugins/sim-actuator/src/lib.rs`
- [x] Add `sim.reset` handling in `execute()` in `plugins/sim-actuator/src/lib.rs`
- [x] Fix `sim.reset` semantics: `delta_obs = -prev_pos` and `delivered_units = Some(1)` in `plugins/sim-actuator/src/lib.rs`
- [x] Add `"sim.reset"` to known capabilities check in `execute()`
- [x] Add delta spec at `openspec/changes/sim-actuator-reset-capability/specs/bodies/spec.md` (`## ADDED Requirements` + `#### Scenario:` blocks)
- [x] Add unit tests: `sim.reset` delta_obs/delivered_units, `discover()` ordering + sensed_effect + schema hash, reset-at-zero case in `plugins/sim-actuator/tests/sim_actuator_tests.rs`
- [x] Run `cargo test -p body-sim-actuator` — 5/5 PASS
- [x] Run `cargo clippy -p body-sim-actuator --all-targets -- -D warnings` — PASS
- [ ] Run GitNexus impact analysis on `SimActuatorBody` (GitNexus index not available in this environment)
- [ ] Run GitNexus detect-changes before commit (GitNexus CLI not available in this environment)
- [ ] Archive change via `openspec archive sim-actuator-reset-capability` (openspec CLI not available in this environment)
