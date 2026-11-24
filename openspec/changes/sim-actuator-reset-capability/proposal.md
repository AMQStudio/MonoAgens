# Proposal — `sim-actuator-reset-capability`

## Why

The `body-sim-actuator` plugin exposes two capabilities (`sim.move` and `sim.stop`) but has no way to reset the simulated actuator position back to zero without manually issuing inverse move commands. Operators need a deterministic "reset to origin" action for test and recovery scenarios.

## What Changes

- Add a new capability `sim.reset` to `body-sim-actuator` that sets position to `0.0`, velocity to `0.0`, and stopped to `true`, then emits a `Done` receipt with a sensor percept on channel `sim:encoder:position` with `delta_obs = 0.0` and `delta_pred = 0.0`.
- Add unit test coverage for `sim.reset` in `plugins/sim-actuator/tests/sim_actuator_tests.rs`.
- No changes to core, valve, gateway, protocol, or any other plugin.

## Impact

- `plugins/sim-actuator/src/lib.rs` — new capability descriptor + execute handling.
- `plugins/sim-actuator/tests/sim_actuator_tests.rs` — new test function.
- No core, valve, gateway, protocol, or cross-plugin impact. Safe to implement in isolation.
