//! `body-mcp-bridge`: adapts an MCP `2026-07-28` (`2025-11-25` fallback) server into a
//! virtual `mcp:<server_id>` Waist-2 place (`monoagens.body.v1`).

#![forbid(unsafe_code)]

pub mod mcp;

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use monoagens_body_kit::{capability_schema_hash, make_refused_receipt, make_terminal_receipt};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyHeartbeatParams, BodyId, BodyStatus, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, HeartbeatScheduler,
    PerceptEmitParams, PerceptEmitResult, PlaceId, Receipt, ReceiptOutcome, RpcId,
    TerminationReason, Time, Waist2Error, Waist2Request, sort_capabilities,
};
use serde_json::{Value, json};

pub use mcp::{
    LoopbackMcpTransport, McpCallResult, McpClient, McpNotification, McpProtocolVersion,
    McpToolInfo, McpTransport, McpTransportError, MockMcpServer, StdioMcpTransport,
};

/// Configuration for [`McpBridgeBody`].
#[derive(Debug, Clone)]
pub struct McpBridgeConfig {
    pub body_id: BodyId,
    pub server_id: String,
    pub max_poll_attempts: u32,
    pub poll_interval_cap_ms: u64,
}

impl McpBridgeConfig {
    /// Creates a config targeting virtual place `mcp:<server_id>`.
    #[must_use]
    pub fn new(server_id: impl Into<String>) -> Self {
        let server_id = server_id.into();
        Self {
            body_id: BodyId::new(format!("body-mcp-{server_id}")),
            server_id,
            max_poll_attempts: 20,
            poll_interval_cap_ms: 50,
        }
    }

    /// Virtual place identifier `mcp:<server_id>`.
    #[must_use]
    pub fn place_id(&self) -> PlaceId {
        PlaceId::new(format!("mcp:{}", self.server_id))
    }
}

/// Summary of capability schema changes detected across MCP tool refreshes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaDiff {
    pub added: Vec<CapabilityId>,
    pub removed: Vec<CapabilityId>,
    pub changed: Vec<(CapabilityId, CapabilitySchemaHash, CapabilitySchemaHash)>,
}

#[derive(Debug, Clone)]
struct PendingMcpApproval {
    capability_id: CapabilityId,
    tool_name: String,
    arguments: Value,
    request_state: Option<String>,
    task_id: Option<String>,
}

/// Waist-2 body bridging one MCP server (`2026-07-28` or `2025-11-25`) into `mcp:<server_id>`.
pub struct McpBridgeBody<T: McpTransport> {
    config: McpBridgeConfig,
    place: PlaceId,
    client: McpClient<T>,
    capabilities: Vec<CapabilityDescriptor>,
    tool_by_cap: HashMap<CapabilityId, String>,
    pending_approvals: HashMap<ActionId, PendingMcpApproval>,
    health_status: BodyStatus,
    last_error: Option<String>,
    heartbeat: HeartbeatScheduler,
    clock_now_ms: u64,
}

impl<T: McpTransport> McpBridgeBody<T> {
    /// Connects `transport`, performs initial `tools/list` discovery (degrading gracefully if
    /// the MCP server is broken/absent), and returns the ready/degraded bridge.
    #[must_use]
    pub fn connect(config: McpBridgeConfig, transport: T) -> Self {
        let place = config.place_id();
        let mut bridge = Self {
            config,
            place,
            client: McpClient::new(transport),
            capabilities: Vec::new(),
            tool_by_cap: HashMap::new(),
            pending_approvals: HashMap::new(),
            health_status: BodyStatus::Ready,
            last_error: None,
            heartbeat: HeartbeatScheduler::new(5_000),
            clock_now_ms: 0,
        };
        let _ = bridge.refresh_tools();
        bridge
    }

    /// Returns the virtual place `mcp:<server_id>`.
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns the negotiated MCP protocol version (`2026-07-28` or `2025-11-25`).
    #[must_use]
    pub const fn mcp_protocol_version(&self) -> McpProtocolVersion {
        self.client.protocol_version()
    }

    /// Returns current operational health (`Ready` or `Degraded` if MCP server failed).
    #[must_use]
    pub const fn health_status(&self) -> BodyStatus {
        self.health_status
    }

    /// Returns the latest MCP transport error if currently degraded.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Builds an outbound `body/heartbeat` request carrying live MCP health status.
    pub fn build_heartbeat(
        &mut self,
        id: RpcId,
        now_ms: u64,
        idempotency_key: impl Into<String>,
    ) -> Waist2Request<BodyHeartbeatParams> {
        self.clock_now_ms = now_ms;
        self.heartbeat.next_request(
            id,
            self.config.body_id.clone(),
            self.health_status,
            now_ms,
            idempotency_key,
        )
    }

    /// Borrows the inner [`McpClient`] (for tests inspecting or mutating the mock peer).
    pub fn client_mut(&mut self) -> &mut McpClient<T> {
        &mut self.client
    }

    /// Refreshes tool descriptors from MCP `tools/list` and returns any `capability_schema_hash` diffs.
    pub fn refresh_tools(&mut self) -> Result<SchemaDiff, McpTransportError> {
        let old_hashes: HashMap<CapabilityId, CapabilitySchemaHash> = self
            .capabilities
            .iter()
            .map(|c| (c.id.clone(), c.capability_schema_hash.clone()))
            .collect();

        match self.client.list_tools() {
            Ok(tools) => {
                let mut next_caps = Vec::with_capacity(tools.len());
                let mut next_map = HashMap::with_capacity(tools.len());
                for tool in tools {
                    let cap_id =
                        CapabilityId::new(format!("mcp.{}.{}", self.config.server_id, tool.name));
                    let desc = self.tool_to_descriptor(&cap_id, &tool);
                    next_map.insert(cap_id, tool.name);
                    next_caps.push(desc);
                }
                sort_capabilities(&mut next_caps);

                let mut diff = SchemaDiff::default();
                for cap in &next_caps {
                    match old_hashes.get(&cap.id) {
                        None => diff.added.push(cap.id.clone()),
                        Some(prev) if prev != &cap.capability_schema_hash => {
                            diff.changed.push((
                                cap.id.clone(),
                                prev.clone(),
                                cap.capability_schema_hash.clone(),
                            ));
                        }
                        Some(_) => {}
                    }
                }
                for old_id in old_hashes.keys() {
                    if !next_map.contains_key(old_id) {
                        diff.removed.push(old_id.clone());
                    }
                }

                self.capabilities = next_caps;
                self.tool_by_cap = next_map;
                self.health_status = BodyStatus::Ready;
                self.last_error = None;
                Ok(diff)
            }
            Err(err) => {
                self.health_status = BodyStatus::Degraded;
                self.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    fn tool_to_descriptor(
        &self,
        cap_id: &CapabilityId,
        tool: &McpToolInfo,
    ) -> CapabilityDescriptor {
        let irreversible = tool
            .annotations
            .as_ref()
            .and_then(|a| a.get("destructiveHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hash_input = json!({
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "annotations": tool.annotations,
        });
        CapabilityDescriptor {
            id: cap_id.clone(),
            description: tool
                .description
                .clone()
                .or_else(|| tool.title.clone())
                .unwrap_or_else(|| format!("MCP tool {}", tool.name)),
            places: vec![self.place.clone()],
            flags: CapabilityFlags {
                communicative: false,
                identity_changing: false,
                irreversible,
            },
            typical_latency_ms: 20,
            capability_schema_hash: capability_schema_hash(&hash_input),
            sensed_effect: None,
        }
    }

    fn decode_payload_args(payload: &[u8]) -> Value {
        if payload.is_empty() {
            return json!({});
        }
        if let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(payload) {
            return Value::Object(obj);
        }
        match std::str::from_utf8(payload) {
            Ok(text) => json!({ "input": text }),
            Err(_) => json!({ "byte_len": payload.len() }),
        }
    }

    fn settle_mcp_call(
        &mut self,
        params: &ActionExecuteParams,
        tool_name: &str,
        arguments: Value,
        call_result: McpCallResult,
    ) -> Receipt {
        let now = Time::from_millis(self.clock_now_ms);
        match call_result {
            McpCallResult::Complete { content, is_error } => {
                if is_error {
                    make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Failed,
                        Some("ERR_MCP_TOOL_ERROR".to_owned()),
                        Some(content.to_string()),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    )
                } else {
                    make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Done,
                        None,
                        Some(content.to_string()),
                        Some(TerminationReason::Normal),
                        Some(1.0),
                        Some(1),
                        params.h_act.clone(),
                        now,
                    )
                }
            }
            McpCallResult::InputRequired {
                request_state,
                input_requests,
                task_id,
            } => {
                self.pending_approvals.insert(
                    params.action_id.clone(),
                    PendingMcpApproval {
                        capability_id: params.capability_id.clone(),
                        tool_name: tool_name.to_owned(),
                        arguments,
                        request_state,
                        task_id,
                    },
                );
                make_terminal_receipt(
                    params.action_id.clone(),
                    params.capability_id.clone(),
                    ReceiptOutcome::Held,
                    Some("PendingApproval".to_owned()),
                    Some(input_requests.to_string()),
                    Some(TerminationReason::Normal),
                    Some(0.0),
                    Some(0),
                    params.h_act.clone(),
                    now,
                )
            }
            McpCallResult::TaskHandle {
                task_id,
                status,
                poll_interval_ms,
            } => self.poll_task_to_single_receipt(
                params,
                tool_name,
                arguments,
                &task_id,
                &status,
                poll_interval_ms,
            ),
        }
    }

    fn poll_task_to_single_receipt(
        &mut self,
        params: &ActionExecuteParams,
        tool_name: &str,
        arguments: Value,
        task_id: &str,
        initial_status: &str,
        poll_interval_ms: u64,
    ) -> Receipt {
        let now = Time::from_millis(self.clock_now_ms);
        let mut status = initial_status.to_owned();
        let mut interval_ms = poll_interval_ms.min(self.config.poll_interval_cap_ms);

        for attempt in 0..self.config.max_poll_attempts {
            match status.as_str() {
                "completed" => {
                    let result_val =
                        if self.client.protocol_version() == McpProtocolVersion::V2025_11_25 {
                            self.client.get_task_result(task_id)
                        } else {
                            self.client
                                .get_task(task_id)
                                .map(|v| v.get("result").cloned().unwrap_or(v))
                        };
                    return match result_val {
                        Ok(res) => make_terminal_receipt(
                            params.action_id.clone(),
                            params.capability_id.clone(),
                            ReceiptOutcome::Done,
                            None,
                            Some(res.to_string()),
                            Some(TerminationReason::Normal),
                            Some(1.0),
                            Some(1),
                            params.h_act.clone(),
                            now,
                        ),
                        Err(err) => self.degraded_receipt(params, err),
                    };
                }
                "failed" | "cancelled" => {
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Failed,
                        Some(format!("ERR_MCP_TASK_{}", status.to_uppercase())),
                        Some(format!("MCP task '{task_id}' ended with status '{status}'")),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                "input_required" => {
                    self.pending_approvals.insert(
                        params.action_id.clone(),
                        PendingMcpApproval {
                            capability_id: params.capability_id.clone(),
                            tool_name: tool_name.to_owned(),
                            arguments,
                            request_state: None,
                            task_id: Some(task_id.to_owned()),
                        },
                    );
                    return make_terminal_receipt(
                        params.action_id.clone(),
                        params.capability_id.clone(),
                        ReceiptOutcome::Held,
                        Some("PendingApproval".to_owned()),
                        Some(format!("MCP task '{task_id}' requires input")),
                        Some(TerminationReason::Normal),
                        Some(0.0),
                        Some(0),
                        params.h_act.clone(),
                        now,
                    );
                }
                _ => {}
            }

            if attempt > 0 && interval_ms > 0 {
                thread::sleep(Duration::from_millis(interval_ms));
            }
            match self.client.get_task(task_id) {
                Ok(task_val) => {
                    status = task_val
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("working")
                        .to_owned();
                    if let Some(next_interval) =
                        task_val.get("pollInterval").and_then(Value::as_u64)
                    {
                        interval_ms = next_interval.min(self.config.poll_interval_cap_ms);
                    }
                    if status == "completed"
                        && self.client.protocol_version() == McpProtocolVersion::V2026_07_28
                        && let Some(embedded) = task_val.get("result")
                    {
                        return make_terminal_receipt(
                            params.action_id.clone(),
                            params.capability_id.clone(),
                            ReceiptOutcome::Done,
                            None,
                            Some(embedded.to_string()),
                            Some(TerminationReason::Normal),
                            Some(1.0),
                            Some(1),
                            params.h_act.clone(),
                            now,
                        );
                    }
                }
                Err(err) => return self.degraded_receipt(params, err),
            }
        }

        make_terminal_receipt(
            params.action_id.clone(),
            params.capability_id.clone(),
            ReceiptOutcome::Unknown,
            Some("timed_out".to_owned()),
            Some(format!(
                "MCP task '{task_id}' exceeded bounded poll limit ({})",
                self.config.max_poll_attempts
            )),
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
        err: McpTransportError,
    ) -> Receipt {
        self.health_status = BodyStatus::Degraded;
        self.last_error = Some(err.to_string());
        make_terminal_receipt(
            params.action_id.clone(),
            params.capability_id.clone(),
            ReceiptOutcome::Failed,
            Some("ERR_MCP_UNAVAILABLE".to_owned()),
            Some(err.to_string()),
            Some(TerminationReason::Normal),
            Some(0.0),
            Some(0),
            params.h_act.clone(),
            Time::from_millis(self.clock_now_ms),
        )
    }
}

impl<T: McpTransport> BodyHandler for McpBridgeBody<T> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let notifications = self.client.drain_notifications();
        if notifications
            .iter()
            .any(|n| n.method == "notifications/tools/list_changed")
        {
            let _ = self.refresh_tools();
        }
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
        let Some(tool_name) = self.tool_by_cap.get(&params.capability_id).cloned() else {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        };
        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by body-mcp-bridge",
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        let arguments = Self::decode_payload_args(params.payload.as_bytes());
        let receipt = match self
            .client
            .call_tool(&tool_name, arguments.clone(), None, None)
        {
            Ok(call_res) => {
                self.health_status = BodyStatus::Ready;
                self.last_error = None;
                self.settle_mcp_call(&params, &tool_name, arguments, call_res)
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
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new(format!("mcp.{}.cancel", self.config.server_id)),
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
        if !params.approved {
            return Ok(ApprovalResolveResult { applied: true });
        }

        let responses = json!({
            "owner_approval": {
                "action": "accept",
                "content": {
                    "approved": true,
                    "resolver": params.resolver.as_str(),
                    "h_act": params.h_act.as_str()
                }
            }
        });

        let exec_res = if let Some(task_id) = &pending.task_id {
            self.client.update_task(task_id, responses).map(|_| ())
        } else {
            self.client
                .call_tool(
                    &pending.tool_name,
                    pending.arguments,
                    pending.request_state.as_deref(),
                    Some(responses),
                )
                .and_then(|r| match r {
                    McpCallResult::Complete {
                        is_error: false, ..
                    } => Ok(()),
                    McpCallResult::Complete {
                        content,
                        is_error: true,
                    } => Err(McpTransportError::Protocol(content.to_string())),
                    other => Err(McpTransportError::Protocol(format!(
                        "unexpected non-terminal MCP retry outcome: {other:?}"
                    ))),
                })
        };

        if let Err(err) = exec_res {
            self.health_status = BodyStatus::Degraded;
            self.last_error = Some(err.to_string());
            let _ = pending.capability_id;
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
                "percept place '{}' is not owned by body-mcp-bridge",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}
