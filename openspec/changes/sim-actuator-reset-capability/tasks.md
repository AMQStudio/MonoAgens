# Tasks — `sim-actuator-reset-capability`

## Tasks

- [x] Add `sim.reset` CapabilityDescriptor to `descriptors()` in `plugins/sim-actuator/src/lib.rs`
- [x] Add `sim.reset` handling in `execute()` in `plugins/sim-actuator/src/lib.rs`
- [x] Add `"sim.reset"` to known capabilities check in `execute()`
- [x] Add unit test for `sim.reset` in `plugins/sim-actuator/tests/sim_actuator_tests.rs`
- [x] Run `cargo test -p body-sim-actuator` — 3/3 PASS
- [x] Run `cargo clippy -p body-sim-actuator -- -D warnings` — PASS
- [ ] Run GitNexus impact analysis on `SimActuatorBody` (GitNexus index not available in this environment)
- [ ] Run GitNexus detect-changes before commit (GitNexus CLI not available in this environment)
- [ ] Archive change via `openspec archive sim-actuator-reset-capability` (openspec CLI not available in this environment)
