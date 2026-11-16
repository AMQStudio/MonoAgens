//! `plugin-a2a-bridge`: A2A `v1.0.1` client/server bridge exposing remote agents as
//! `a2a:<agent>` Waist-2 places (`monoagens.body.v1`).

#![forbid(unsafe_code)]

pub mod a2a;

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use monoagens_body_kit::{capability_schema_hash, make_refused_receipt, make_terminal_receipt};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, BodyStatus, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, PerceptEmitParams, PerceptEmitResult, PlaceId, Receipt,
    ReceiptOutcome, TerminationReason, Time, Waist2Error, sort_capabilities,
};
use serde_json::json;

pub use a2a::{
    A2A_PROTOCOL_VERSION, A2aMessage, A2aPart, A2aSendResult, A2aTask, A2aTaskState, A2aTaskStatus,
    A2aTransport, A2aTransportError, AgentCard, AgentInterface, AgentSkill,
    HttpLoopbackA2aTransport, LocalMockA2aPeer,
};

/// Configuration for [`A2aBridgeBody`].
#[derive(Debug, Clone)]
pub struct A2aBridgeConfig {
    pub body_id: BodyId,
    pub agent_id: String,
    pub max_poll_attempts: u32,
    pub poll_interval_ms: u64,
}

impl A2aBridgeConfig {
    /// Creates a config targeting place `a2a:<agent_id>`.
    #[must_use]
    pub fn new(agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        Self {
            body_id: BodyId::new(format!("plugin-a2a-{agent_id}")),
            agent_id,
            max_poll_attempts: 20,
            poll_interval_ms: 5,
        }
    }

    /// Returns the virtual place `a2a:<agent_id>`.
    #[must_use]
    pub fn place_id(&self) -> PlaceId {
        PlaceId::new(format!("a2a:{}", self.agent_id))
    }
}

#[derive(Debug, Clone)]
struct PendingA2aApproval {
    skill_id: String,
    task_id: String,
}

/// Waist-2 body bridging an A2A `v1.0.1` counterpart into `a2a:<agent>`.
pub struct A2aBridgeBody<T: A2aTransport> {
    config: A2aBridgeConfig,
    place: PlaceId,
    transport: T,
    capabilities: Vec<CapabilityDescriptor>,
    skill_by_cap: HashMap<CapabilityId, String>,
    pending_approvals: HashMap<ActionId, PendingA2aApproval>,
    in_flight_tasks: HashMap<ActionId, String>,
    health_status: BodyStatus,
    last_error: Option<String>,
    clock_now_ms: u64,
}

impl<T: A2aTransport> A2aBridgeBody<T> {
    /// Connects `transport`, fetches the remote `AgentCard`, and populates sorted capabilities.
    #[must_use]
    pub fn connect(config: A2aBridgeConfig, mut transport: T) -> Self {
        let place = config.place_id();
        let mut capabilities = Vec::new();
        let mut skill_by_cap = HashMap::new();
        let mut health_status = BodyStatus::Ready;
        let mut last_error = None;

        match transport.fetch_agent_card() {
            Ok(card) => {
                for skill in card.skills {
                    let cap_id = CapabilityId::new(format!("a2a.{}.{}", config.agent_id, skill.id));
                    let desc = CapabilityDescriptor {
                        id: cap_id.clone(),
                        description: skill.description.clone(),
                        places: vec![place.clone()],
                        flags: CapabilityFlags {
                            communicative: true,
                            identity_changing: false,
                            irreversible: false,
                        },
                        typical_latency_ms: 50,
                        capability_schema_hash: capability_schema_hash(&json!({
                            "a2a_version": A2A_PROTOCOL_VERSION,
                            "skill_id": skill.id,
                            "name": skill.name,
                            "description": skill.description,
                            "tags": skill.tags,
                        })),
                        sensed_effect: None,
                    };
                    skill_by_cap.insert(cap_id, skill.id);
                    capabilities.push(desc);
                }
                sort_capabilities(&mut capabilities);
            }
            Err(err) => {
                health_status = BodyStatus::Degraded;
                last_error = Some(err.to_string());
            }
        }

        Self {
            config,
            place,
            transport,
            capabilities,
            skill_by_cap,
            pending_approvals: HashMap::new(),
            in_flight_tasks: HashMap::new(),
            health_status,
            last_error,
            clock_now_ms: 0,
        }
    }

    /// Returns the owned `a2a:<agent>` place.
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns current operational health.
    #[must_use]
    pub const fn health_status(&self) -> BodyStatus {
        self.health_status
    }

    /// Returns latest transport error when degraded.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn settle_task(
        &mut self,
        params: &ActionExecuteParams,
        skill_id: &str,
        mut task: A2aTask,
    ) -> Receipt {
        let now = Time::from_millis(self.clock_now_ms);
        self.in_flight_tasks
            .insert(params.action_id.clone(), task.id.clone());

        for attempt in 0..=self.config.max_poll_attempts {
            match task.status.state {
                A2aTaskState::Completed => {
                    self.in_flight_tasks.remove(&params.action_id);
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Done,
                        None,
                        task_message_text(&task),
                        Some(TerminationReason::Normal),
                        Some(1.0),
                        Some(1),
                        params.h_act.clone(),
                        now,
                    );
                }
                A2aTaskState::InputRequired | A2aTaskState::AuthRequired => {
                    self.pending_approvals.insert(
                        params.action_id.clone(),
                        PendingA2aApproval {
                            skill_id: skill_id.to_owned(),
                            task_id: task.id.clone(),
                        },
                    );
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Held,
                        Some("PendingApproval".to_owned()),
                        task_message_text(&task),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                A2aTaskState::Rejected => {
                    self.in_flight_tasks.remove(&params.action_id);
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Refused,
                        Some("ERR_A2A_REJECTED".to_owned()),
                        task_message_text(&task),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                A2aTaskState::Canceled => {
                    self.in_flight_tasks.remove(&params.action_id);
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Failed,
                        Some("preempted_external".to_owned()),
                        task_message_text(&task),
                        Some(TerminationReason::PreemptedExternal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                A2aTaskState::Failed => {
                    self.in_flight_tasks.remove(&params.action_id);
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Failed,
                        Some("ERR_A2A_TASK_FAILED".to_owned()),
                        task_message_text(&task),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                A2aTaskState::Submitted | A2aTaskState::Working | A2aTaskState::Unspecified => {
                    if attempt == self.config.max_poll_attempts {
                        break;
                    }
                    if attempt > 0 && self.config.poll_interval_ms > 0 {
                        thread::sleep(Duration::from_millis(self.config.poll_interval_ms));
                    }
                    match self.transport.get_task(&task.id) {
                        Ok(next_task) => task = next_task,
                        Err(err) => return self.degraded_receipt(params, err),
                    }
                }
            }
        }

        make_terminal_receipt(
            params.action_id.clone(),
            params.capability_id.clone(),
            ReceiptOutcome::Unknown,
            Some("timed_out".to_owned()),
            Some(format!("A2A task '{}' poll limit exceeded", task.id)),
            Some(TerminationReason::TimedOut),
            None,
            None,
            params.h_act.clone(),
            now,
        )
    }

    fn degraded_receipt(
        &mut self,
        params: &ActionExecuteParams,
        err: A2aTransportError,
    ) -> Receipt {
        self.health_status = BodyStatus::Degraded;
        self.last_error = Some(err.to_string());
        make_terminal_receipt(
            params.action_id.clone(),
            params.capability_id.clone(),
            ReceiptOutcome::Failed,
            Some("ERR_A2A_UNAVAILABLE".to_owned()),
            Some(err.to_string()),
            Some(TerminationReason::Normal),
            Some(0.0),
            Some(0),
            params.h_act.clone(),
            Time::from_millis(self.clock_now_ms),
        )
    }
}

fn task_message_text(task: &A2aTask) -> Option<String> {
    task.status.message.as_ref().map(|m| {
        m.parts
            .iter()
            .filter_map(|p| p.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

impl<T: A2aTransport> BodyHandler for A2aBridgeBody<T> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let mut capabilities = self.capabilities.clone();
        sort_capabilities(&mut capabilities);
        Ok(BodyDiscoverResult {
            body_id: params
                .body_id
                .unwrap_or_else(|| self.config.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities,
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        let Some(skill_id) = self.skill_by_cap.get(&params.capability_id).cloned() else {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        };
        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-a2a-bridge",
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        let text = String::from_utf8_lossy(params.payload.as_bytes()).into_owned();
        let msg = A2aMessage {
            role: "ROLE_USER".to_owned(),
            message_id: Some(format!("msg-{}", params.action_id)),
            task_id: None,
            parts: vec![A2aPart {
                text: Some(text),
                data: None,
            }],
        };

        let receipt = match self.transport.send_message(&skill_id, msg) {
            Ok(A2aSendResult::Message { message }) => {
                self.health_status = BodyStatus::Ready;
                let detail = message
                    .parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n");
                make_terminal_receipt(
                    params.action_id,
                    params.capability_id,
                    ReceiptOutcome::Done,
                    None,
                    Some(detail),
                    Some(TerminationReason::Normal),
                    Some(1.0),
                    Some(1),
                    params.h_act,
                    Time::from_millis(self.clock_now_ms),
                )
            }
            Ok(A2aSendResult::Task { task }) => {
                self.health_status = BodyStatus::Ready;
                self.settle_task(&params, &skill_id, task)
            }
            Err(err) => self.degraded_receipt(&params, err),
        };

        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt),
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.pending_approvals.remove(&params.action_id);
        if let Some(task_id) = self.in_flight_tasks.remove(&params.action_id) {
            let _ = self.transport.cancel_task(&task_id);
        }
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new(format!("a2a.{}.cancel", self.config.agent_id)),
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
        let Some(pending) = self.pending_approvals.remove(&params.action_id) else {
            return Ok(ApprovalResolveResult { applied: false });
        };

        let msg = A2aMessage {
            role: "ROLE_USER".to_owned(),
            message_id: Some(format!("approval-{}", params.action_id)),
            task_id: Some(pending.task_id),
            parts: vec![A2aPart {
                text: Some(format!(
                    "approved={} resolver={} h_act={}",
                    params.approved, params.resolver, params.h_act
                )),
                data: None,
            }],
        };

        if let Err(err) = self.transport.send_message(&pending.skill_id, msg) {
            self.health_status = BodyStatus::Degraded;
            self.last_error = Some(err.to_string());
            return Err(Waist2Error::invalid_receipt_transition(err.to_string()));
        }
        Ok(ApprovalResolveResult { applied: true })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-a2a-bridge",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}
