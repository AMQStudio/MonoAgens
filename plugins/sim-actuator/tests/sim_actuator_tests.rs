use body_sim_actuator::SimActuatorBody;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionId, ActorId, Audience, BodyDiscoverParams, BodyHandler,
    CapabilityId, OpaquePayload, PlaceId, ReceiptOutcome, TerminationReason, Waist2Headers,
};
use serde_json::json;

#[test]
fn sim_actuator_declares_sensed_effects_and_answers_prediction_with_sensor_percept() {
    let mut body = SimActuatorBody::default();
    body.set_now_ms(1_000);

    let disc = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover");
    let move_cap = disc
        .capabilities
        .iter()
        .find(|c| c.id.as_str() == "sim.move")
        .expect("sim.move");
    let effect = move_cap.sensed_effect.as_ref().expect("sensed_effect");
    assert_eq!(effect.channel.as_str(), "sim:encoder:position");
    assert_eq!(effect.window_ms, 200);

    let res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-move"),
            action_id: ActionId::new("act-move-1"),
            capability_id: CapabilityId::new("sim.move"),
            places: vec![PlaceId::new("sim:actuator-0")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(
                json!({ "target_delta": 2.5, "steps": 5 })
                    .to_string()
                    .into_bytes(),
            ),
            timeout_ms: Some(2_000),
            h_act: None,
        })
        .expect("execute");
    let rcpt = res.receipt.expect("receipt");
    assert_eq!(rcpt.outcome, ReceiptOutcome::Done);
    assert_eq!(rcpt.delivered_units, Some(5));
    assert!((body.state().position - 2.5).abs() < 1e-9);

    // Sensor percept answers effect prediction with correlation_id and channel_prediction.
    assert_eq!(body.emitted_percepts().len(), 1);
    let p = &body.emitted_percepts()[0];
    assert_eq!(p.delta_obs, Some(2.5));
    let pred = p.channel_prediction.as_ref().expect("channel_prediction");
    assert_eq!(pred.channel.as_str(), "sim:encoder:position");
    assert_eq!(pred.delta_obs, Some(2.5));
    assert_eq!(pred.delta_pred, Some(2.5));
}

#[test]
fn sim_actuator_preemption_emits_option_a_receipt() {
    let mut body = SimActuatorBody::default();
    body.trigger_external_preempt();

    let res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-preempt"),
            action_id: ActionId::new("act-move-preempt"),
            capability_id: CapabilityId::new("sim.move"),
            places: vec![PlaceId::new("sim:actuator-0")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(
                json!({ "target_delta": 4.0, "steps": 4 })
                    .to_string()
                    .into_bytes(),
            ),
            timeout_ms: Some(2_000),
            h_act: None,
        })
        .expect("execute");
    let rcpt = res.receipt.expect("receipt");
    assert_eq!(rcpt.outcome, ReceiptOutcome::Failed);
    assert_eq!(
        rcpt.termination_reason,
        Some(TerminationReason::PreemptedExternal)
    );
    assert_eq!(rcpt.progress, Some(0.0));
    assert_eq!(rcpt.delivered_units, Some(0));
}

#[test]
fn sim_actuator_reset_returns_position_to_zero() {
    let mut body = SimActuatorBody::default();
    body.set_now_ms(2_000);

    // Move the actuator away from zero first.
    let move_res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-move-before-reset"),
            action_id: ActionId::new("act-move-before-reset"),
            capability_id: CapabilityId::new("sim.move"),
            places: vec![PlaceId::new("sim:actuator-0")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(
                json!({ "target_delta": 5.0, "steps": 5 })
                    .to_string()
                    .into_bytes(),
            ),
            timeout_ms: Some(2_000),
            h_act: None,
        })
        .expect("execute move");
    assert_eq!(move_res.receipt.expect("receipt").outcome, ReceiptOutcome::Done);
    assert!((body.state().position - 5.0).abs() < 1e-9);

    // Reset the actuator back to zero.
    let reset_res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-reset"),
            action_id: ActionId::new("act-reset-1"),
            capability_id: CapabilityId::new("sim.reset"),
            places: vec![PlaceId::new("sim:actuator-0")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"{}"),
            timeout_ms: Some(2_000),
            h_act: None,
        })
        .expect("execute reset");
    let rcpt = reset_res.receipt.expect("receipt");
    assert_eq!(rcpt.outcome, ReceiptOutcome::Done);
    assert_eq!(body.state().position, 0.0);
    assert_eq!(body.state().velocity, 0.0);
    assert!(body.state().stopped);

    // Sensor percept emitted for reset: delta_obs is the actual position change (-5.0).
    assert_eq!(body.emitted_percepts().len(), 2);
    let p = &body.emitted_percepts()[1];
    assert_eq!(p.delta_obs, Some(-5.0));
    let pred = p.channel_prediction.as_ref().expect("channel_prediction");
    assert_eq!(pred.delta_obs, Some(-5.0));
    assert_eq!(pred.delta_pred, Some(0.0));
}

#[test]
fn sim_actuator_discover_exposes_sim_reset() {
    let mut body = SimActuatorBody::default();

    let disc = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d-discover-reset"),
            body_id: None,
        })
        .expect("discover");

    // Capabilities are sorted by id: sim.move, sim.reset, sim.stop.
    let ids: Vec<&str> = disc
        .capabilities
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, vec!["sim.move", "sim.reset", "sim.stop"]);

    // sim.reset exposes sensed_effect + capability_schema_hash.
    let reset_cap = disc
        .capabilities
        .iter()
        .find(|c| c.id.as_str() == "sim.reset")
        .expect("sim.reset");
    assert!(!reset_cap.capability_schema_hash.as_str().is_empty());
    let effect = reset_cap.sensed_effect.as_ref().expect("sensed_effect");
    assert_eq!(effect.channel.as_str(), "sim:encoder:position");
    assert_eq!(effect.window_ms, 50);
    assert_eq!(effect.noise, 0.0);
    assert_eq!(effect.delta_pred, 0.0);
}

#[test]
fn sim_actuator_reset_at_zero_position_keeps_zero() {
    let mut body = SimActuatorBody::default();
    body.set_now_ms(3_000);

    // Already at origin: reset should be a no-op position-wise with delta_obs == -0.0 == 0.0.
    assert_eq!(body.state().position, 0.0);

    let reset_res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-reset-at-zero"),
            action_id: ActionId::new("act-reset-at-zero"),
            capability_id: CapabilityId::new("sim.reset"),
            places: vec![PlaceId::new("sim:actuator-0")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"{}"),
            timeout_ms: Some(2_000),
            h_act: None,
        })
        .expect("execute reset at zero");
    let rcpt = reset_res.receipt.expect("receipt");
    assert_eq!(rcpt.outcome, ReceiptOutcome::Done);
    assert_eq!(rcpt.delivered_units, Some(1));
    assert_eq!(body.state().position, 0.0);
    assert_eq!(body.state().velocity, 0.0);
    assert!(body.state().stopped);

    let p = &body.emitted_percepts()[0];
    assert_eq!(p.delta_obs, Some(-0.0));
    let pred = p.channel_prediction.as_ref().expect("channel_prediction");
    assert_eq!(pred.delta_obs, Some(-0.0));
    assert_eq!(pred.delta_pred, Some(0.0));
}
