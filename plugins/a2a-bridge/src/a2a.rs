//! A2A `v1.0.1` protocol model, JSON-RPC 2.0 client/server handlers, and local loopback transports.

use core::fmt;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A2A protocol version implemented by this bridge.
pub const A2A_PROTOCOL_VERSION: &str = "1.0.1";

/// Default maximum transitive A2A delegation hop count (`C-14`).
pub const DEFAULT_MAX_A2A_HOPS: u8 = 3;

/// Canonical refusal code emitted when an A2A delegation exceeds `max_a2a_hops` or revisits an agent.
pub const ERR_A2A_CYCLE: &str = "ERR_A2A_CYCLE";

/// A2A `v1.0.1` `AgentInterface` entry inside `AgentCard.supportedInterfaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    pub url: String,
    pub protocol_binding: String,
    pub protocol_version: String,
}

/// Skill declared on an A2A `AgentCard`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A2A `v1.0.1` `AgentCard` fetched from `/.well-known/agent-card.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub supported_interfaces: Vec<AgentInterface>,
    pub skills: Vec<AgentSkill>,
}

/// Canonical A2A `v1.0.1` `TaskState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum A2aTaskState {
    #[serde(rename = "TASK_STATE_UNSPECIFIED")]
    Unspecified,
    #[serde(rename = "TASK_STATE_SUBMITTED")]
    Submitted,
    #[serde(rename = "TASK_STATE_WORKING")]
    Working,
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    #[serde(rename = "TASK_STATE_REJECTED")]
    Rejected,
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED")]
    AuthRequired,
}

/// Text/data part of an A2A `Message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2aPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A2A `Message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub parts: Vec<A2aPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Status block inside an A2A `Task`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2aTaskStatus {
    pub state: A2aTaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<A2aMessage>,
}

/// A2A `Task`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2aTask {
    pub id: String,
    pub status: A2aTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Union result returned by A2A `SendMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum A2aSendResult {
    Task { task: A2aTask },
    Message { message: A2aMessage },
}

/// Errors surfaced while communicating with an A2A peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2aTransportError {
    Unavailable(String),
    RpcError { code: i64, message: String },
    Protocol(String),
}

impl fmt::Display for A2aTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(msg) => write!(f, "A2A peer unavailable: {msg}"),
            Self::RpcError { code, message } => write!(f, "A2A RPC error [{code}]: {message}"),
            Self::Protocol(msg) => write!(f, "A2A protocol error: {msg}"),
        }
    }
}

impl std::error::Error for A2aTransportError {}

/// Transport abstraction for A2A `v1.0.1` client operations (`AgentCard`, `SendMessage`, `GetTask`, `CancelTask`).
pub trait A2aTransport {
    fn fetch_agent_card(&mut self) -> Result<AgentCard, A2aTransportError>;
    fn send_message(
        &mut self,
        skill_id: &str,
        message: A2aMessage,
    ) -> Result<A2aSendResult, A2aTransportError>;
    fn get_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError>;
    fn cancel_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError>;
}

/// In-process / server-side A2A `v1.0.1` peer handling `/.well-known/agent-card.json` and
/// JSON-RPC `SendMessage`, `GetTask`, `CancelTask`.
#[derive(Debug, Clone)]
pub struct LocalMockA2aPeer {
    card: AgentCard,
    tasks: BTreeMap<String, MockA2aTaskEntry>,
    next_task_seq: u64,
}

#[derive(Debug, Clone)]
struct MockA2aTaskEntry {
    task: A2aTask,
    working_polls_left: u32,
}

impl Default for LocalMockA2aPeer {
    fn default() -> Self {
        Self::new("mock-agent", "http://127.0.0.1:0/rpc")
    }
}

impl LocalMockA2aPeer {
    /// Creates a mock A2A `v1.0.1` peer advertising `delegate` and `async_review` skills.
    #[must_use]
    pub fn new(agent_name: &str, rpc_url: &str) -> Self {
        Self {
            card: AgentCard {
                name: agent_name.to_owned(),
                description: "Mock A2A v1.0.1 counterpart".to_owned(),
                supported_interfaces: vec![AgentInterface {
                    url: rpc_url.to_owned(),
                    protocol_binding: "JSONRPC".to_owned(),
                    protocol_version: A2A_PROTOCOL_VERSION.to_owned(),
                }],
                skills: vec![
                    AgentSkill {
                        id: "delegate".to_owned(),
                        name: "Delegate Task".to_owned(),
                        description: "Delegates work to remote A2A agent".to_owned(),
                        tags: vec!["a2a".to_owned()],
                    },
                    AgentSkill {
                        id: "async_review".to_owned(),
                        name: "Async Review".to_owned(),
                        description: "Runs multi-step A2A task polled via GetTask".to_owned(),
                        tags: vec!["a2a".to_owned(), "async".to_owned()],
                    },
                ],
            },
            tasks: BTreeMap::new(),
            next_task_seq: 0,
        }
    }

    /// Returns the `AgentCard` served at `/.well-known/agent-card.json`.
    #[must_use]
    pub fn agent_card(&self) -> &AgentCard {
        &self.card
    }

    /// Server-side A2A `v1.0.1` JSON-RPC 2.0 handler (`SendMessage`, `GetTask`, `CancelTask`).
    pub fn handle_jsonrpc(&mut self, req: &Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        let result = match method {
            "SendMessage" | "message/send" => self.on_send_message(&params),
            "GetTask" | "tasks/get" => self.on_get_task(&params),
            "CancelTask" | "tasks/cancel" => self.on_cancel_task(&params),
            other => Err((-32601, format!("unsupported A2A method '{other}'"))),
        };

        match result {
            Ok(val) => json!({ "jsonrpc": "2.0", "id": id, "result": val }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        }
    }

    fn on_send_message(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let msg: A2aMessage =
            serde_json::from_value(params.get("message").cloned().unwrap_or(Value::Null))
                .map_err(|e| (-32602, format!("invalid A2A message: {e}")))?;

        let text = msg
            .parts
            .iter()
            .filter_map(|p| p.text.as_deref())
            .collect::<Vec<_>>()
            .join(" ");
        let skill_id = params
            .get("metadata")
            .and_then(|m| m.get("skillId"))
            .and_then(Value::as_str)
            .unwrap_or("delegate");

        if let Some(tid) = msg.task_id {
            let entry = self
                .tasks
                .get_mut(&tid)
                .ok_or((-32602, format!("unknown A2A task '{tid}'")))?;
            let approved = text.contains("approved=true") || !text.contains("approved=false");
            entry.task.status = A2aTaskStatus {
                state: if approved {
                    A2aTaskState::Completed
                } else {
                    A2aTaskState::Rejected
                },
                message: Some(A2aMessage {
                    role: "ROLE_AGENT".to_owned(),
                    message_id: Some(format!("msg-{tid}-done")),
                    task_id: Some(tid.clone()),
                    parts: vec![A2aPart {
                        text: Some(if approved {
                            "A2A task completed after owner approval".to_owned()
                        } else {
                            "A2A task rejected by owner".to_owned()
                        }),
                        data: None,
                    }],
                    metadata: None,
                }),
            };
            return serde_json::to_value(A2aSendResult::Task {
                task: entry.task.clone(),
            })
            .map_err(|e| (-32603, e.to_string()));
        }

        let msg_hop = msg
            .metadata
            .as_ref()
            .and_then(|m| m.get("hopCount").or_else(|| m.get("hop_count")))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let msg_visited: Vec<String> = msg
            .metadata
            .as_ref()
            .and_then(|m| m.get("visitedAgents").or_else(|| m.get("visited_agents")))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let self_visits = msg_visited
            .iter()
            .filter(|a| a.as_str() == self.card.name)
            .count();

        self.next_task_seq += 1;
        let tid = format!("a2a-task-{}", self.next_task_seq);

        let (state, polls_left, reply_text) = if msg_hop > u64::from(DEFAULT_MAX_A2A_HOPS)
            || self_visits > 1
        {
            (
                A2aTaskState::Rejected,
                0,
                format!("{ERR_A2A_CYCLE}: delegation hop/cycle limit exceeded"),
            )
        } else if text.trim() == "hold" {
            (
                A2aTaskState::InputRequired,
                0,
                "A2A counterpart requires owner approval".to_owned(),
            )
        } else if skill_id == "async_review" || text.contains("async") {
            (A2aTaskState::Working, 2, "A2A task working".to_owned())
        } else {
            (
                A2aTaskState::Completed,
                0,
                format!("A2A completed skill={skill_id}: {text}"),
            )
        };

        let task = A2aTask {
            id: tid.clone(),
            status: A2aTaskStatus {
                state,
                message: Some(A2aMessage {
                    role: "ROLE_AGENT".to_owned(),
                    message_id: Some(format!("msg-{tid}")),
                    task_id: Some(tid.clone()),
                    parts: vec![A2aPart {
                        text: Some(reply_text),
                        data: None,
                    }],
                    metadata: msg.metadata.clone(),
                }),
            },
            metadata: msg.metadata,
        };
        self.tasks.insert(
            tid,
            MockA2aTaskEntry {
                task: task.clone(),
                working_polls_left: polls_left,
            },
        );
        serde_json::to_value(A2aSendResult::Task { task }).map_err(|e| (-32603, e.to_string()))
    }

    fn on_get_task(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let tid = params
            .get("id")
            .or_else(|| params.get("taskId"))
            .and_then(Value::as_str)
            .ok_or((-32602, "missing task id".to_owned()))?;
        let entry = self
            .tasks
            .get_mut(tid)
            .ok_or((-32602, format!("unknown task '{tid}'")))?;

        if entry.task.status.state == A2aTaskState::Working {
            if entry.working_polls_left > 0 {
                entry.working_polls_left -= 1;
            }
            if entry.working_polls_left == 0 {
                entry.task.status.state = A2aTaskState::Completed;
                entry.task.status.message = Some(A2aMessage {
                    role: "ROLE_AGENT".to_owned(),
                    message_id: Some(format!("msg-{tid}-complete")),
                    task_id: Some(tid.to_owned()),
                    parts: vec![A2aPart {
                        text: Some(format!("A2A task '{tid}' completed")),
                        data: None,
                    }],
                    metadata: None,
                });
            }
        }
        serde_json::to_value(&entry.task).map_err(|e| (-32603, e.to_string()))
    }

    fn on_cancel_task(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let tid = params
            .get("id")
            .or_else(|| params.get("taskId"))
            .and_then(Value::as_str)
            .ok_or((-32602, "missing task id".to_owned()))?;
        let entry = self
            .tasks
            .get_mut(tid)
            .ok_or((-32602, format!("unknown task '{tid}'")))?;
        entry.task.status.state = A2aTaskState::Canceled;
        serde_json::to_value(&entry.task).map_err(|e| (-32603, e.to_string()))
    }
}

impl A2aTransport for LocalMockA2aPeer {
    fn fetch_agent_card(&mut self) -> Result<AgentCard, A2aTransportError> {
        Ok(self.card.clone())
    }

    fn send_message(
        &mut self,
        skill_id: &str,
        message: A2aMessage,
    ) -> Result<A2aSendResult, A2aTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": message,
                "metadata": { "skillId": skill_id }
            }
        });
        let resp = self.handle_jsonrpc(&req);
        parse_rpc_result(&resp)
    }

    fn get_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "GetTask",
            "params": { "id": task_id }
        });
        let resp = self.handle_jsonrpc(&req);
        parse_rpc_result(&resp)
    }

    fn cancel_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "CancelTask",
            "params": { "id": task_id }
        });
        let resp = self.handle_jsonrpc(&req);
        parse_rpc_result(&resp)
    }
}

/// Minimal HTTP/1.1 loopback A2A `v1.0.1` transport for local mock HTTP server tests.
pub struct HttpLoopbackA2aTransport {
    host_port: String,
    next_id: i64,
}

impl HttpLoopbackA2aTransport {
    /// Connects to a local `host:port` HTTP A2A endpoint (e.g. `127.0.0.1:41234`).
    #[must_use]
    pub fn new(host_port: impl Into<String>) -> Self {
        Self {
            host_port: host_port.into(),
            next_id: 1,
        }
    }

    fn http_exchange(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Value, A2aTransportError> {
        let mut stream = TcpStream::connect(&self.host_port).map_err(|e| {
            A2aTransportError::Unavailable(format!("connect {}: {e}", self.host_port))
        })?;
        let payload = body.unwrap_or(&[]);
        let header = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.host_port,
            payload.len()
        );
        stream
            .write_all(header.as_bytes())
            .and_then(|()| stream.write_all(payload))
            .and_then(|()| stream.flush())
            .map_err(|e| A2aTransportError::Unavailable(e.to_string()))?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut content_length: Option<usize> = None;
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| A2aTransportError::Unavailable(e.to_string()))?;
            if n == 0 || line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_length = v.trim().parse::<usize>().ok();
            }
        }

        let mut body_bytes = Vec::new();
        if let Some(len) = content_length {
            body_bytes.resize(len, 0);
            reader
                .read_exact(&mut body_bytes)
                .map_err(|e| A2aTransportError::Unavailable(e.to_string()))?;
        } else {
            reader
                .read_to_end(&mut body_bytes)
                .map_err(|e| A2aTransportError::Unavailable(e.to_string()))?;
        }

        serde_json::from_slice(&body_bytes)
            .map_err(|e| A2aTransportError::Protocol(format!("invalid A2A JSON body: {e}")))
    }

    fn rpc_call<R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<R, A2aTransportError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let raw =
            serde_json::to_vec(&req).map_err(|e| A2aTransportError::Protocol(e.to_string()))?;
        let resp = self.http_exchange("POST", "/rpc", Some(&raw))?;
        parse_rpc_result(&resp)
    }
}

impl A2aTransport for HttpLoopbackA2aTransport {
    fn fetch_agent_card(&mut self) -> Result<AgentCard, A2aTransportError> {
        let val = self.http_exchange("GET", "/.well-known/agent-card.json", None)?;
        serde_json::from_value(val)
            .map_err(|e| A2aTransportError::Protocol(format!("invalid AgentCard: {e}")))
    }

    fn send_message(
        &mut self,
        skill_id: &str,
        message: A2aMessage,
    ) -> Result<A2aSendResult, A2aTransportError> {
        self.rpc_call(
            "SendMessage",
            json!({
                "message": message,
                "metadata": { "skillId": skill_id }
            }),
        )
    }

    fn get_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError> {
        self.rpc_call("GetTask", json!({ "id": task_id }))
    }

    fn cancel_task(&mut self, task_id: &str) -> Result<A2aTask, A2aTransportError> {
        self.rpc_call("CancelTask", json!({ "id": task_id }))
    }
}

fn parse_rpc_result<R: serde::de::DeserializeOwned>(resp: &Value) -> Result<R, A2aTransportError> {
    if let Some(err) = resp.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown A2A error")
            .to_owned();
        return Err(A2aTransportError::RpcError { code, message });
    }
    let result = resp.get("result").cloned().unwrap_or(Value::Null);
    serde_json::from_value(result)
        .map_err(|e| A2aTransportError::Protocol(format!("invalid A2A result: {e}")))
}
