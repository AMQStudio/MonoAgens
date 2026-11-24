# Specification — sim-actuator-reset-capability

## ADDED Requirements

### Requirement: `sim.reset` capability on `body-sim-actuator`
The `body-sim-actuator` plugin SHALL declare a new `sim.reset` capability that resets the
simulated actuator position to `0.0`, velocity to `0.0`, and stopped to `true`, then emits a
`Done` receipt and a `sim:encoder:position` sensor percept whose `delta_obs` equals the actual
position change (`-prev_pos`) and whose `delta_pred` equals `0.0`.

#### Scenario: Reset returns position to zero and reports the observed position delta
- **WHEN** `sim.move` moves the actuator to `position = 5.0` and then `sim.reset` is executed
- **THEN** `sim.reset` returns `position` to `0.0`, sets `velocity = 0.0` and `stopped = true`, emits a `Done` receipt with `delivered_units = 1`, and emits a `sim:encoder:position` sensor percept with `delta_obs = -5.0` and `delta_pred = 0.0`

#### Scenario: Reset at origin is a no-op position-wise with zero delta
- **WHEN** the actuator is already at `position = 0.0` and `sim.reset` is executed
- **THEN** `sim.reset` keeps `position = 0.0`, `velocity = 0.0`, and `stopped = true`, emits a `Done` receipt with `delivered_units = 1`, and emits a `sim:encoder:position` sensor percept with `delta_obs = -0.0` and `delta_pred = 0.0`
