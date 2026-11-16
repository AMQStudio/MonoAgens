//! `plugin-ros2-bridge`: ROS 2 `Lyrical Luth` (`rclrs`) bridge mapping topics and services
//! to `ros2:<node>` Waist-2 places (`monoagens.body.v1`).
//!
//! Real `rclrs` binding is gated behind `--features ros2-rclrs` and requires a sourced
//! ROS 2 Lyrical Luth installation (`ROS_DISTRO` / `AMENT_PREFIX_PATH`).

#![forbid(unsafe_code)]

use core::fmt;
use std::collections::{HashMap, HashSet};
use std::env;

use monoagens_body_kit::{capability_schema_hash, make_refused_receipt, make_terminal_receipt};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, OpaquePayload, OriginActor, PerceptEmitParams,
    PerceptEmitResult, PlaceId, ReceiptOutcome, Taint, TerminationReason, Time, Waist2Error,
    Waist2Headers, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// ROS 2 target distribution documented for this bridge.
pub const TARGET_ROS2_DISTRO: &str = "lyrical";

/// Error returned when ROS 2 `Lyrical Luth` runtime or `ros2-rclrs` feature is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ros2UnavailableError {
    pub reason: String,
}

impl fmt::Display for Ros2UnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ROS 2 Lyrical Luth runtime unavailable: {}", self.reason)
    }
}

impl std::error::Error for Ros2UnavailableError {}

/// Checks whether both the `ros2-rclrs` feature and a sourced ROS 2 environment exist.
pub fn check_ros2_runtime_available() -> Result<(), Ros2UnavailableError> {
    if !cfg!(feature = "ros2-rclrs") {
        return Err(Ros2UnavailableError {
            reason:
                "compiled without 'ros2-rclrs' feature (no ROS 2 installation detected on host)"
                    .to_owned(),
        });
    }
    let has_distro = env::var_os("ROS_DISTRO").is_some_and(|v| !v.is_empty());
    let has_ament = env::var_os("AMENT_PREFIX_PATH").is_some_and(|v| !v.is_empty());
    if !has_distro || !has_ament {
        return Err(Ros2UnavailableError {
            reason: "ROS_DISTRO / AMENT_PREFIX_PATH not set (source /opt/ros/lyrical/setup.sh)"
                .to_owned(),
        });
    }
    Ok(())
}

/// ROS 2 topic descriptor exposed as a Waist-2 publish capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ros2TopicSpec {
    pub topic: String,
    pub msg_type: String,
    pub description: String,
}

/// ROS 2 service descriptor exposed as a Waist-2 service capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ros2ServiceSpec {
    pub service: String,
    pub srv_type: String,
    pub description: String,
}

/// Backend port implemented by `rclrs` (under `ros2-rclrs` feature) or unit-test doubles.
pub trait Ros2Backend {
    fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), String>;
    fn call_service(&mut self, service: &str, payload: &[u8]) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone)]
enum Ros2TargetKind {
    Topic(String),
    Service(String),
}

/// Waist-2 body mapping ROS 2 topics and services onto `ros2:<node>`.
pub struct Ros2BridgeBody<B: Ros2Backend> {
    body_id: BodyId,
    place: PlaceId,
    node_actor: ActorId,
    backend: B,
    capabilities: Vec<CapabilityDescriptor>,
    targets: HashMap<CapabilityId, Ros2TargetKind>,
    held_actions: HashSet<ActionId>,
    clock_now_ms: u64,
}

impl<B: Ros2Backend> Ros2BridgeBody<B> {
    /// Creates a [`Ros2BridgeBody`] exposing `topics` and `services` at `ros2:<node_name>`.
    #[must_use]
    pub fn new(
        node_name: &str,
        topics: Vec<Ros2TopicSpec>,
        services: Vec<Ros2ServiceSpec>,
        backend: B,
    ) -> Self {
        let place = PlaceId::new(format!("ros2:{node_name}"));
        let mut capabilities = Vec::new();
        let mut targets = HashMap::new();

        for topic in topics {
            let cap_id = CapabilityId::new(format!(
                "ros2.{node_name}.pub.{}",
                topic.topic.trim_start_matches('/').replace('/', ".")
            ));
            let schema = json!({
                "ros2_kind": "topic_publish",
                "topic": topic.topic,
                "msg_type": topic.msg_type,
            });
            capabilities.push(CapabilityDescriptor {
                id: cap_id.clone(),
                description: topic.description,
                places: vec![place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 10,
                capability_schema_hash: capability_schema_hash(&schema),
                sensed_effect: None,
            });
            targets.insert(cap_id, Ros2TargetKind::Topic(topic.topic));
        }

        for srv in services {
            let cap_id = CapabilityId::new(format!(
                "ros2.{node_name}.srv.{}",
                srv.service.trim_start_matches('/').replace('/', ".")
            ));
            let schema = json!({
                "ros2_kind": "service_call",
                "service": srv.service,
                "srv_type": srv.srv_type,
            });
            capabilities.push(CapabilityDescriptor {
                id: cap_id.clone(),
                description: srv.description,
                places: vec![place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: true,
                },
                typical_latency_ms: 50,
                capability_schema_hash: capability_schema_hash(&schema),
                sensed_effect: None,
            });
            targets.insert(cap_id, Ros2TargetKind::Service(srv.service));
        }

        sort_capabilities(&mut capabilities);
        Self {
            body_id: BodyId::new(format!("plugin-ros2-{node_name}")),
            place,
            node_actor: ActorId::new(format!("per_ros2_{node_name}")),
            backend,
            capabilities,
            targets,
            held_actions: HashSet::new(),
            clock_now_ms: 0,
        }
    }

    /// Returns the owned `ros2:<node>` place.
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Translates an observed ROS 2 subscription message into a Waist-2 `percept/emit` envelope.
    #[must_use]
    pub fn translate_topic_message(
        &self,
        topic: &str,
        payload: &[u8],
        audience: Audience,
        timestamp_ms: Time,
        idempotency_key: impl Into<String>,
    ) -> PerceptEmitParams {
        PerceptEmitParams {
            headers: Waist2Headers::new(idempotency_key),
            actor: self.node_actor.clone(),
            origin_actor: Some(OriginActor::new(self.node_actor.clone())),
            place: self.place.clone(),
            audience: Some(audience),
            timestamp_ms,
            payload: OpaquePayload::from_bytes(
                json!({ "topic": topic, "bytes_len": payload.len(), "data": String::from_utf8_lossy(payload) })
                    .to_string()
                    .into_bytes(),
            ),
            taint: Taint::Untrusted,
            addressees: Vec::new(),
            named: Vec::new(),
            orient: Vec::new(),
            reply_to: None,
            correlation_id: None,
            channel: None,
            delta_obs: None,
            channel_prediction: None,
            length: Some(payload.len() as u64),
            negative_flag: None,
            repair: None,
            continued_attention: None,
            f_modal: None,
            embeddings: Vec::new(),
            attachments: Vec::new(),
            dialogue_act_gap: None,
        }
    }

    /// Borrows the backend for assertions in unit tests.
    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: Ros2Backend> BodyHandler for Ros2BridgeBody<B> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: self.capabilities.clone(),
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        let Some(target) = self.targets.get(&params.capability_id).cloned() else {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        };
        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-ros2-bridge",
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting approval before ROS 2 dispatch".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let now = Time::from_millis(self.clock_now_ms);
        let receipt = match target {
            Ros2TargetKind::Topic(topic) => {
                match self.backend.publish(&topic, params.payload.as_bytes()) {
                    Ok(()) => make_terminal_receipt(
                        params.action_id,
                        params.capability_id,
                        ReceiptOutcome::Done,
                        None,
                        Some(format!("published to topic {topic}")),
                        Some(TerminationReason::Normal),
                        Some(1.0),
                        Some(1),
                        params.h_act,
                        now,
                    ),
                    Err(err) => make_terminal_receipt(
                        params.action_id,
                        params.capability_id,
                        ReceiptOutcome::Failed,
                        Some("ERR_ROS2_PUBLISH".to_owned()),
                        Some(err),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act,
                        now,
                    ),
                }
            }
            Ros2TargetKind::Service(service) => {
                match self
                    .backend
                    .call_service(&service, params.payload.as_bytes())
                {
                    Ok(reply) => make_terminal_receipt(
                        params.action_id,
                        params.capability_id,
                        ReceiptOutcome::Done,
                        None,
                        Some(String::from_utf8_lossy(&reply).into_owned()),
                        Some(TerminationReason::Normal),
                        Some(1.0),
                        Some(1),
                        params.h_act,
                        now,
                    ),
                    Err(err) => make_terminal_receipt(
                        params.action_id,
                        params.capability_id,
                        ReceiptOutcome::Failed,
                        Some("ERR_ROS2_SERVICE".to_owned()),
                        Some(err),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act,
                        now,
                    ),
                }
            }
        };

        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt),
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.held_actions.remove(&params.action_id);
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new("ros2.cancel"),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        Ok(ApprovalResolveResult {
            applied: self.held_actions.remove(&params.action_id),
        })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-ros2-bridge",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}

/// Real `rclrs 0.8` ROS 2 Lyrical Luth backend, compiled only when `--features ros2-rclrs`
/// is enabled on a host with ROS 2 Lyrical installed.
#[cfg(feature = "ros2-rclrs")]
pub mod rclrs_backend {
    use rclrs::{Context, CreateBasicExecutor, Node};

    /// Creates an initialized `rclrs::Node` from the sourced ROS 2 Lyrical environment.
    pub fn init_ros2_node(node_name: &str) -> Result<Node, String> {
        let ctx = Context::default_from_env().map_err(|e| format!("rclrs context error: {e}"))?;
        let executor = ctx.create_basic_executor();
        executor
            .create_node(node_name)
            .map_err(|e| format!("rclrs node '{node_name}' error: {e}"))
    }
}
