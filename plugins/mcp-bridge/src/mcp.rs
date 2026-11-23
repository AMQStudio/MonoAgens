//! MCP `2026-07-28` client + `2025-11-25` fallback transport and loopback/stdio adapters.

use core::fmt;
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Negotiated MCP protocol revision (`2026-07-28` primary, `2025-11-25` legacy fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpProtocolVersion {
    V2026_07_28,
    V2025_11_25,
}

impl McpProtocolVersion {
    /// Canonical MCP version string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V2026_07_28 => "2026-07-28",
            Self::V2025_11_25 => "2025-11-25",
        }
    }
}

/// Tool metadata returned by MCP `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default = "default_object_schema")]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<Value>,
}

fn default_object_schema() -> Value {
    json!({ "type": "object" })
}

/// Server-to-client MCP notification observed between RPC calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpNotification {
    pub method: String,
    pub params: Value,
}

/// Transport or protocol error from an MCP peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransportError {
    Unavailable(String),
    RpcError { code: i64, message: String },
    Protocol(String),
}

impl fmt::Display for McpTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(msg) => write!(f, "MCP server unavailable: {msg}"),
            Self::RpcError { code, message } => write!(f, "MCP RPC error [{code}]: {message}"),
            Self::Protocol(msg) => write!(f, "MCP protocol error: {msg}"),
        }
    }
}

impl std::error::Error for McpTransportError {}

/// Synchronous JSON-RPC 2.0 transport to one MCP server.
pub trait McpTransport {
    fn request(&mut self, id: i64, method: &str, params: Value)
    -> Result<Value, McpTransportError>;
    fn notify(&mut self, method: &str, params: Value) -> Result<(), McpTransportError>;
    fn drain_notifications(&mut self) -> Vec<McpNotification>;
}

/// Stdio child-process MCP transport speaking NDJSON JSON-RPC 2.0.
pub struct StdioMcpTransport {
    child: Option<Child>,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: Option<BufReader<ChildStdout>>,
    notifications: VecDeque<McpNotification>,
    spawn_error: Option<String>,
}

impl StdioMcpTransport {
    /// Spawns `command` with `args`; spawn failure is retained cleanly so the bridge degrades
    /// rather than crashing.
    #[must_use]
    pub fn spawn(command: &str, args: &[String]) -> Self {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                let stdin = child.stdin.take().map(BufWriter::new);
                let stdout = child.stdout.take().map(BufReader::new);
                Self {
                    child: Some(child),
                    stdin,
                    stdout,
                    notifications: VecDeque::new(),
                    spawn_error: None,
                }
            }
            Err(err) => Self {
                child: None,
                stdin: None,
                stdout: None,
                notifications: VecDeque::new(),
                spawn_error: Some(format!("failed to spawn '{command}': {err}")),
            },
        }
    }

    fn send_json(&mut self, payload: &Value) -> Result<(), McpTransportError> {
        if let Some(err) = &self.spawn_error {
            return Err(McpTransportError::Unavailable(err.clone()));
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(McpTransportError::Unavailable(
                "MCP stdin closed".to_owned(),
            ));
        };
        let line = serde_json::to_string(payload)
            .map_err(|e| McpTransportError::Protocol(e.to_string()))?;
        if let Err(err) = writeln!(stdin, "{line}").and_then(|()| stdin.flush()) {
            self.stdin = None;
            return Err(McpTransportError::Unavailable(format!(
                "write to MCP stdin failed: {err}"
            )));
        }
        Ok(())
    }
}

impl Drop for StdioMcpTransport {
    fn drop(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl McpTransport for StdioMcpTransport {
    fn request(
        &mut self,
        id: i64,
        method: &str,
        params: Value,
    ) -> Result<Value, McpTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send_json(&req)?;

        loop {
            let Some(stdout) = self.stdout.as_mut() else {
                return Err(McpTransportError::Unavailable(
                    "MCP stdout closed".to_owned(),
                ));
            };
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) => {
                    self.stdout = None;
                    return Err(McpTransportError::Unavailable(
                        "MCP server closed stdout unexpectedly".to_owned(),
                    ));
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let msg: Value = serde_json::from_str(trimmed).map_err(|e| {
                        McpTransportError::Protocol(format!("invalid JSON from MCP server: {e}"))
                    })?;
                    if let Some(notif_method) = msg.get("method").and_then(Value::as_str)
                        && msg.get("id").is_none()
                    {
                        self.notifications.push_back(McpNotification {
                            method: notif_method.to_owned(),
                            params: msg.get("params").cloned().unwrap_or(Value::Null),
                        });
                        continue;
                    }
                    if msg.get("id").and_then(Value::as_i64) != Some(id) {
                        continue;
                    }
                    if let Some(err) = msg.get("error") {
                        let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603);
                        let message = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown MCP error")
                            .to_owned();
                        return Err(McpTransportError::RpcError { code, message });
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(err) => {
                    self.stdout = None;
                    return Err(McpTransportError::Unavailable(format!(
                        "read from MCP stdout failed: {err}"
                    )));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), McpTransportError> {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_json(&notif)
    }

    fn drain_notifications(&mut self) -> Vec<McpNotification> {
        self.notifications.drain(..).collect()
    }
}

/// Outcome of calling an MCP tool through [`McpClient`].
#[derive(Debug, Clone, PartialEq)]
pub enum McpCallResult {
    Complete {
        content: Value,
        is_error: bool,
    },
    InputRequired {
        request_state: Option<String>,
        input_requests: Value,
        task_id: Option<String>,
    },
    TaskHandle {
        task_id: String,
        status: String,
        poll_interval_ms: u64,
    },
}

/// High-level MCP `2026-07-28` client with automatic `2025-11-25` handshake fallback.
pub struct McpClient<T: McpTransport> {
    transport: T,
    version: McpProtocolVersion,
    initialized_legacy: bool,
    next_id: i64,
}

impl<T: McpTransport> McpClient<T> {
    /// Wraps `transport`, preferring MCP `2026-07-28` stateless-first semantics.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            version: McpProtocolVersion::V2026_07_28,
            initialized_legacy: false,
            next_id: 1,
        }
    }

    /// Returns the active MCP protocol revision.
    #[must_use]
    pub const fn protocol_version(&self) -> McpProtocolVersion {
        self.version
    }

    /// Drains any pending server notifications (e.g. `notifications/tools/list_changed`).
    pub fn drain_notifications(&mut self) -> Vec<McpNotification> {
        self.transport.drain_notifications()
    }

    /// Lists all tools via `tools/list`, falling back to `2025-11-25` `initialize` if required.
    pub fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, McpTransportError> {
        let mut cursor: Option<String> = None;
        let mut tools = Vec::new();

        loop {
            let mut params = serde_json::Map::new();
            if self.version == McpProtocolVersion::V2026_07_28 {
                params.insert("_meta".to_owned(), self.modern_meta());
            }
            if let Some(c) = &cursor {
                params.insert("cursor".to_owned(), Value::String(c.clone()));
            }

            let id = self.alloc_id();
            let res = match self
                .transport
                .request(id, "tools/list", Value::Object(params))
            {
                Ok(v) => v,
                Err(McpTransportError::RpcError { code, message })
                    if !self.initialized_legacy
                        && (code == -32600 || message.to_lowercase().contains("initialize")) =>
                {
                    self.fallback_initialize_2025()?;
                    return self.list_tools();
                }
                Err(err) => return Err(err),
            };

            let batch = res.get("tools").and_then(Value::as_array).ok_or_else(|| {
                McpTransportError::Protocol("tools/list missing tools array".to_owned())
            })?;
            for item in batch {
                let tool: McpToolInfo = serde_json::from_value(item.clone()).map_err(|e| {
                    McpTransportError::Protocol(format!("invalid tool schema: {e}"))
                })?;
                tools.push(tool);
            }

            cursor = res
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if cursor.is_none() {
                break;
            }
        }

        Ok(tools)
    }

    /// Calls `tools/call` on `tool_name`, optionally supplying MRTR `requestState` + `inputResponses`.
    pub fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: Value,
        request_state: Option<&str>,
        input_responses: Option<Value>,
    ) -> Result<McpCallResult, McpTransportError> {
        let mut params = serde_json::Map::new();
        if self.version == McpProtocolVersion::V2026_07_28 {
            params.insert("_meta".to_owned(), self.modern_meta());
        }
        params.insert("name".to_owned(), Value::String(tool_name.to_owned()));
        params.insert("arguments".to_owned(), arguments);
        if let Some(state) = request_state {
            params.insert("requestState".to_owned(), Value::String(state.to_owned()));
        }
        if let Some(responses) = input_responses {
            params.insert("inputResponses".to_owned(), responses);
        }

        let id = self.alloc_id();
        let res = self
            .transport
            .request(id, "tools/call", Value::Object(params))?;
        parse_call_result(&res)
    }

    /// Polls `tasks/get` for a long-running MCP task.
    pub fn get_task(&mut self, task_id: &str) -> Result<Value, McpTransportError> {
        let mut params = serde_json::Map::new();
        if self.version == McpProtocolVersion::V2026_07_28 {
            params.insert("_meta".to_owned(), self.modern_meta());
        }
        params.insert("taskId".to_owned(), Value::String(task_id.to_owned()));
        let id = self.alloc_id();
        self.transport
            .request(id, "tasks/get", Value::Object(params))
    }

    /// Submits client-to-server input (`tasks/update` in `2026-07-28`) for a held task.
    pub fn update_task(
        &mut self,
        task_id: &str,
        input_responses: Value,
    ) -> Result<Value, McpTransportError> {
        let mut params = serde_json::Map::new();
        if self.version == McpProtocolVersion::V2026_07_28 {
            params.insert("_meta".to_owned(), self.modern_meta());
        }
        params.insert("taskId".to_owned(), Value::String(task_id.to_owned()));
        params.insert("inputResponses".to_owned(), input_responses);
        let id = self.alloc_id();
        self.transport
            .request(id, "tasks/update", Value::Object(params))
    }

    /// Retrieves completed task output via legacy `tasks/result` (`2025-11-25`).
    pub fn get_task_result(&mut self, task_id: &str) -> Result<Value, McpTransportError> {
        let id = self.alloc_id();
        self.transport
            .request(id, "tasks/result", json!({ "taskId": task_id }))
    }

    /// Borrows the inner transport (for testing/inspection).
    #[must_use]
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn fallback_initialize_2025(&mut self) -> Result<(), McpTransportError> {
        let id = self.alloc_id();
        let init_params = json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {
                "elicitation": { "form": {} },
                "tasks": { "requests": { "tools": { "call": {} } } }
            },
            "clientInfo": {
                "name": "body-mcp-bridge",
                "version": "0.1.0"
            }
        });
        let _init_res = self.transport.request(id, "initialize", init_params)?;
        self.transport
            .notify("notifications/initialized", json!({}))?;
        self.version = McpProtocolVersion::V2025_11_25;
        self.initialized_legacy = true;
        Ok(())
    }

    fn modern_meta(&self) -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {
                "name": "body-mcp-bridge",
                "version": "0.1.0"
            },
            "io.modelcontextprotocol/clientCapabilities": {
                "elicitation": { "form": {} }
            }
        })
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn parse_call_result(res: &Value) -> Result<McpCallResult, McpTransportError> {
    if res.get("resultType").and_then(Value::as_str) == Some("input_required") {
        return Ok(McpCallResult::InputRequired {
            request_state: res
                .get("requestState")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            input_requests: res.get("inputRequests").cloned().unwrap_or(Value::Null),
            task_id: None,
        });
    }

    if let Some(task) = res.get("task") {
        let task_id = task
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or_else(|| McpTransportError::Protocol("task handle missing taskId".to_owned()))?
            .to_owned();
        let status = task
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("working")
            .to_owned();
        let poll_interval_ms = task
            .get("pollInterval")
            .and_then(Value::as_u64)
            .unwrap_or(50);
        if status == "input_required" {
            return Ok(McpCallResult::InputRequired {
                request_state: None,
                input_requests: task.get("inputRequests").cloned().unwrap_or(Value::Null),
                task_id: Some(task_id),
            });
        }
        return Ok(McpCallResult::TaskHandle {
            task_id,
            status,
            poll_interval_ms,
        });
    }

    let is_error = res.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let content = res.get("content").cloned().unwrap_or_else(|| res.clone());
    Ok(McpCallResult::Complete { content, is_error })
}

/// Deterministic in-memory MCP `2026-07-28` / `2025-11-25` server implementing tools,
/// MRTR `input_required` elicitation, async `tasks/get` polling, and schema mutations.
#[derive(Debug, Clone)]
pub struct MockMcpServer {
    pub require_legacy_init: bool,
    pub initialized: bool,
    pub tools: Vec<McpToolInfo>,
    tasks: BTreeMap<String, MockTask>,
    pending_notifications: VecDeque<McpNotification>,
    next_task_seq: u64,
}

#[derive(Debug, Clone)]
struct MockTask {
    polls_remaining: u32,
    final_content: Value,
    status: String,
    fail: bool,
}

impl Default for MockMcpServer {
    fn default() -> Self {
        Self::new(false)
    }
}

impl MockMcpServer {
    /// Creates a mock MCP server (`require_legacy_init = false` for `2026-07-28`, `true` for `2025-11-25`).
    #[must_use]
    pub fn new(require_legacy_init: bool) -> Self {
        Self {
            require_legacy_init,
            initialized: false,
            tools: vec![
                McpToolInfo {
                    name: "echo".to_owned(),
                    title: Some("Echo Tool".to_owned()),
                    description: Some("Echoes arguments or holds when input='hold'".to_owned()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "input": { "type": "string" },
                            "hold": { "type": "boolean" }
                        }
                    }),
                    annotations: Some(json!({ "readOnlyHint": true })),
                },
                McpToolInfo {
                    name: "long_job".to_owned(),
                    title: Some("Long Job".to_owned()),
                    description: Some("Runs an async MCP task polled via tasks/get".to_owned()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "polls": { "type": "integer" }
                        }
                    }),
                    annotations: Some(json!({ "destructiveHint": false })),
                },
            ],
            tasks: BTreeMap::new(),
            pending_notifications: VecDeque::new(),
            next_task_seq: 0,
        }
    }

    /// Drains queued server notifications.
    pub fn drain_notifications(&mut self) -> Vec<McpNotification> {
        self.pending_notifications.drain(..).collect()
    }

    /// Mutates a tool's `inputSchema` and queues `notifications/tools/list_changed`.
    pub fn bump_tool_schema(&mut self, tool_name: &str, new_property: &str) {
        if let Some(tool) = self.tools.iter_mut().find(|t| t.name == tool_name)
            && let Some(props) = tool
                .input_schema
                .get_mut("properties")
                .and_then(Value::as_object_mut)
        {
            props.insert(new_property.to_owned(), json!({ "type": "string" }));
        }
        self.pending_notifications.push_back(McpNotification {
            method: "notifications/tools/list_changed".to_owned(),
            params: json!({}),
        });
    }

    /// Handles one MCP JSON-RPC 2.0 request and returns the serialized JSON-RPC response.
    pub fn handle_request(&mut self, req: &Value) -> Option<Value> {
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if id.is_none() {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        }

        if method == "initialize" {
            self.initialized = true;
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {
                        "tools": { "listChanged": true },
                        "tasks": { "requests": { "tools": { "call": {} } } }
                    },
                    "serverInfo": { "name": "mock-mcp-server", "version": "0.1.0" }
                }
            }));
        }

        if self.require_legacy_init && !self.initialized {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Server requires initialize handshake (2025-11-25)"
                }
            }));
        }

        let is_2026 = params
            .get("_meta")
            .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
            .and_then(Value::as_str)
            == Some("2026-07-28");

        let result = match method {
            "tools/list" => {
                let mut res = json!({ "tools": self.tools });
                if is_2026 {
                    res["resultType"] = Value::String("complete".to_owned());
                }
                Ok(res)
            }
            "tools/call" => self.handle_tools_call(&params, is_2026),
            "tasks/get" => self.handle_tasks_get(&params, is_2026),
            "tasks/update" => self.handle_tasks_update(&params),
            "tasks/result" => self.handle_tasks_result(&params),
            other => Err((-32601, format!("method '{other}' not found"))),
        };

        Some(match result {
            Ok(val) => json!({ "jsonrpc": "2.0", "id": id, "result": val }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        })
    }

    fn handle_tools_call(&mut self, params: &Value, is_2026: bool) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "missing tool name".to_owned()))?;
        if !self.tools.iter().any(|t| t.name == name) {
            return Err((-32602, format!("unknown tool '{name}'")));
        }

        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let has_responses = params.get("inputResponses").is_some();

        if let Some(prop) = args.get("mutate_schema").and_then(Value::as_str) {
            self.bump_tool_schema(name, prop);
        }

        let wants_hold = args.get("hold").and_then(Value::as_bool) == Some(true)
            || args.get("input").and_then(Value::as_str) == Some("hold");

        if wants_hold && !has_responses {
            if is_2026 {
                return Ok(json!({
                    "resultType": "input_required",
                    "inputRequests": {
                        "owner_approval": {
                            "method": "elicitation/create",
                            "params": {
                                "mode": "form",
                                "message": "Confirm tool execution",
                                "requestedSchema": {
                                    "type": "object",
                                    "properties": { "approved": { "type": "boolean" } },
                                    "required": ["approved"]
                                }
                            }
                        }
                    },
                    "requestState": format!("state:{name}:hold")
                }));
            }
            self.next_task_seq += 1;
            let tid = format!("task-{}", self.next_task_seq);
            self.tasks.insert(
                tid.clone(),
                MockTask {
                    polls_remaining: 0,
                    final_content: json!({
                        "content": [{ "type": "text", "text": "approved-legacy-task" }],
                        "isError": false
                    }),
                    status: "input_required".to_owned(),
                    fail: false,
                },
            );
            return Ok(json!({
                "task": {
                    "taskId": tid,
                    "status": "input_required",
                    "pollInterval": 1
                }
            }));
        }

        if name == "long_job" || args.get("polls").and_then(Value::as_u64).is_some() {
            let polls = args
                .get("polls")
                .and_then(Value::as_u64)
                .unwrap_or(2)
                .min(50) as u32;
            let fail = args.get("fail").and_then(Value::as_bool).unwrap_or(false);
            self.next_task_seq += 1;
            let tid = format!("task-{}", self.next_task_seq);
            self.tasks.insert(
                tid.clone(),
                MockTask {
                    polls_remaining: polls,
                    final_content: json!({
                        "content": [{ "type": "text", "text": format!("job {tid} finished") }],
                        "isError": fail
                    }),
                    status: "working".to_owned(),
                    fail,
                },
            );
            return Ok(json!({
                "task": {
                    "taskId": tid,
                    "status": "working",
                    "pollInterval": 1
                }
            }));
        }

        let mut res = json!({
            "content": [{
                "type": "text",
                "text": format!("tool={name} args={args}")
            }],
            "isError": false
        });
        if is_2026 {
            res["resultType"] = Value::String("complete".to_owned());
        }
        Ok(res)
    }

    fn handle_tasks_get(&mut self, params: &Value, is_2026: bool) -> Result<Value, (i64, String)> {
        let task_id = params
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or((-32602, "missing taskId".to_owned()))?;
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or((-32602, format!("unknown taskId '{task_id}'")))?;

        if task.status == "working" {
            if task.polls_remaining > 0 {
                task.polls_remaining -= 1;
            }
            if task.polls_remaining == 0 {
                task.status = if task.fail {
                    "failed".to_owned()
                } else {
                    "completed".to_owned()
                };
            }
        }

        let mut res = json!({
            "taskId": task_id,
            "status": task.status,
            "pollInterval": 1
        });
        if is_2026 && (task.status == "completed" || task.status == "failed") {
            res["result"] = task.final_content.clone();
        }
        Ok(res)
    }

    fn handle_tasks_update(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let task_id = params
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or((-32602, "missing taskId".to_owned()))?;
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or((-32602, format!("unknown taskId '{task_id}'")))?;
        task.status = "completed".to_owned();
        Ok(json!({
            "taskId": task_id,
            "status": task.status,
            "result": task.final_content
        }))
    }

    fn handle_tasks_result(&self, params: &Value) -> Result<Value, (i64, String)> {
        let task_id = params
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or((-32602, "missing taskId".to_owned()))?;
        let task = self
            .tasks
            .get(task_id)
            .ok_or((-32602, format!("unknown taskId '{task_id}'")))?;
        Ok(task.final_content.clone())
    }
}

/// In-process loopback [`McpTransport`] wrapping [`MockMcpServer`].
#[derive(Debug, Default)]
pub struct LoopbackMcpTransport {
    pub server: MockMcpServer,
}

impl LoopbackMcpTransport {
    /// Creates a loopback transport (`require_legacy_init = false` for MCP `2026-07-28`).
    #[must_use]
    pub fn new(require_legacy_init: bool) -> Self {
        Self {
            server: MockMcpServer::new(require_legacy_init),
        }
    }
}

impl McpTransport for LoopbackMcpTransport {
    fn request(
        &mut self,
        id: i64,
        method: &str,
        params: Value,
    ) -> Result<Value, McpTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let resp = self
            .server
            .handle_request(&req)
            .ok_or_else(|| McpTransportError::Protocol("missing response".to_owned()))?;
        if let Some(err) = resp.get("error") {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603);
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown MCP error")
                .to_owned();
            return Err(McpTransportError::RpcError { code, message });
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), McpTransportError> {
        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let _ = self.server.handle_request(&req);
        Ok(())
    }

    fn drain_notifications(&mut self) -> Vec<McpNotification> {
        self.server.pending_notifications.drain(..).collect()
    }
}
