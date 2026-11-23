use std::process::Command;

use monoagens_body_sdk::{
    ActionExecuteParams, ActionId, ActorId, Audience, BodyDiscoverParams, BodyHandler,
    CapabilityId, OpaquePayload, PlaceId, ReceiptOutcome, Time, Waist2ErrorCode, Waist2Headers,
};
use plugin_ros2_bridge::{
    Ros2Backend, Ros2BridgeBody, Ros2ServiceSpec, Ros2TopicSpec, check_ros2_runtime_available,
};

#[derive(Default)]
struct InMemoryRos2Double {
    published: Vec<(String, Vec<u8>)>,
    service_calls: Vec<(String, Vec<u8>)>,
}

impl Ros2Backend for InMemoryRos2Double {
    fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), String> {
        self.published.push((topic.to_owned(), payload.to_vec()));
        Ok(())
    }

    fn call_service(&mut self, service: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.service_calls
            .push((service.to_owned(), payload.to_vec()));
        Ok(b"srv_ok".to_vec())
    }
}

#[test]
fn ros2_mapping_translates_topics_services_and_enforces_place_and_capability() {
    let mut bridge = Ros2BridgeBody::new(
        "rover",
        vec![Ros2TopicSpec {
            topic: "/cmd_vel".to_owned(),
            msg_type: "geometry_msgs/msg/Twist".to_owned(),
            description: "Publish velocity command".to_owned(),
        }],
        vec![Ros2ServiceSpec {
            service: "/reset_odom".to_owned(),
            srv_type: "std_srvs/srv/Trigger".to_owned(),
            description: "Reset odometry".to_owned(),
        }],
        InMemoryRos2Double::default(),
    );

    let disc = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover");
    assert_eq!(disc.places, vec![PlaceId::new("ros2:rover")]);
    assert_eq!(disc.capabilities.len(), 2);

    let res = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-pub"),
            action_id: ActionId::new("act-ros-pub"),
            capability_id: CapabilityId::new("ros2.rover.pub.cmd_vel"),
            places: vec![PlaceId::new("ros2:rover")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"linear=0.5".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .expect("publish");
    assert_eq!(res.receipt.expect("r").outcome, ReceiptOutcome::Done);
    assert_eq!(
        bridge.backend().published,
        vec![("/cmd_vel".to_owned(), b"linear=0.5".to_vec())]
    );

    // Unknown capability -> -32001.
    let err = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-bad"),
            action_id: ActionId::new("act-bad"),
            capability_id: CapabilityId::new("ros2.rover.pub.unknown"),
            places: vec![PlaceId::new("ros2:rover")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .expect_err("unknown cap");
    assert_eq!(err.error_code(), Some(Waist2ErrorCode::UnknownCapability));

    // Percept translation attaches ros2:<node> place and non-empty audience.
    let p = bridge.translate_topic_message(
        "/odom",
        b"x=1.0",
        Audience::new([ActorId::new("per_owner")]),
        Time::from_millis(500),
        "p-1",
    );
    assert_eq!(p.place, PlaceId::new("ros2:rover"));
    assert!(p.audience.as_ref().is_some_and(|a| !a.is_empty()));
}

#[test]
fn ros2_binary_exits_with_clear_error_when_runtime_is_missing() {
    assert!(check_ros2_runtime_available().is_err());

    let bin = env!("CARGO_BIN_EXE_plugin-ros2-bridge");
    let out = Command::new(bin).output().expect("run plugin-ros2-bridge");
    assert!(
        !out.status.success(),
        "plugin-ros2-bridge must fail loudly without ROS 2 runtime"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ROS 2 Lyrical Luth runtime unavailable"),
        "unexpected stderr: {stderr}"
    );
}
