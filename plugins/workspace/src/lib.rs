//! `plugin-workspace`: Sandboxed per-actor workspace Waist-2 capability plugin.
//!
//! Declares `workspace.read`, `workspace.write`, `workspace.edit`, `workspace.grep`,
//! `workspace.glob`, and bounded `workspace.run` under `cap:workspace`, enforcing strict
//! per-actor path containment, symlink escape rejection, binary text guards, file-size
//! limits, and optional RBAC role quotas.

#![forbid(unsafe_code)]

pub mod config;
pub mod sandbox;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use monoagens_body_kit::{capability_schema_hash, make_refused_receipt, make_terminal_receipt};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, MetaValue, PerceptEmitParams, PerceptEmitResult, PlaceId, ReceiptOutcome,
    TerminationReason, Time, Waist2Error, sort_capabilities,
};
use regex::Regex;
use serde_json::{Value, json};

pub use config::{
    WorkspaceConfig, is_rbac_enabled_from_env, resolve_state_dir_from_env,
};
use sandbox::{
    SandboxError, check_write_limits, display_relative, effective_quota_bytes, ensure_actor_root,
    recursive_dir_bytes, resolve_actor_key, resolve_in_sandbox, validate_utf8_text, write_atomic,
};

/// Default Waist-2 body identifier for `plugin-workspace`.
pub const DEFAULT_BODY_ID: &str = "plugin-workspace";

/// Default Waist-2 place declared by `plugin-workspace`.
pub const DEFAULT_PLACE_ID: &str = "cap:workspace";

const DEFAULT_READ_OFFSET: usize = 0;
const DEFAULT_READ_LIMIT: usize = 400;
const MAX_READ_LIMIT: usize = 2000;
const DEFAULT_GREP_LIMIT: usize = 100;
const MAX_GREP_LIMIT: usize = 1000;
const MAX_GREP_LINE_CHARS: usize = 400;
const DEFAULT_GLOB_LIMIT: usize = 200;
const MAX_GLOB_LIMIT: usize = 1000;
const SAFE_PATH_ENV: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Waist-2 capability body providing isolated per-actor workspaces.
pub struct WorkspaceBody {
    body_id: BodyId,
    place: PlaceId,
    config: WorkspaceConfig,
    workspace_root: PathBuf,
    rbac_enabled: bool,
    held_actions: HashSet<ActionId>,
    clock_now_ms: u64,
}

impl Default for WorkspaceBody {
    fn default() -> Self {
        Self::with_config(WorkspaceConfig::default())
    }
}

impl WorkspaceBody {
    /// Creates a [`WorkspaceBody`] using `config`, resolving `workspace_root` and `rbac_enabled`
    /// from configuration/environment.
    #[must_use]
    pub fn with_config(config: WorkspaceConfig) -> Self {
        let workspace_root = config.resolve_workspace_root();
        let rbac_enabled = is_rbac_enabled_from_env();
        Self {
            body_id: BodyId::new(DEFAULT_BODY_ID),
            place: PlaceId::new(DEFAULT_PLACE_ID),
            config,
            workspace_root,
            rbac_enabled,
            held_actions: HashSet::new(),
            clock_now_ms: 0,
        }
    }

    /// Overrides the body identifier.
    #[must_use]
    pub fn with_body_id(mut self, body_id: BodyId) -> Self {
        self.body_id = body_id;
        self
    }

    /// Overrides the declared place identifier.
    #[must_use]
    pub fn with_place(mut self, place: PlaceId) -> Self {
        self.place = place;
        self
    }

    /// Explicitly overrides the root directory under which actor sandboxes live.
    #[must_use]
    pub fn with_workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = root.into();
        self
    }

    /// Explicitly sets RBAC enforcement mode (useful for deterministic unit tests).
    #[must_use]
    pub fn with_rbac_enabled(mut self, enabled: bool) -> Self {
        self.rbac_enabled = enabled;
        self
    }

    /// Sets the virtual clock timestamp used for receipts.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    /// Returns a reference to the active [`WorkspaceConfig`].
    #[must_use]
    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    /// Returns the declared [`PlaceId`].
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns the root directory containing per-actor workspaces.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn capability_descriptors(&self) -> Vec<CapabilityDescriptor> {
        let specs = [
            (
                "workspace.read",
                "Read lines of a UTF-8 text file inside the actor's sandbox workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "offset": { "type": "integer", "minimum": 0, "default": 0 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "default": 400 }
                    },
                    "required": ["path"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                5u64,
                false,
            ),
            (
                "workspace.write",
                "Write a UTF-8 text file atomically inside the actor's sandbox workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["path", "text"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                10u64,
                false,
            ),
            (
                "workspace.edit",
                "Replace exact text inside an existing file in the actor's sandbox workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old": { "type": "string" },
                        "new": { "type": "string" },
                        "replace_all": { "type": "boolean", "default": false }
                    },
                    "required": ["path", "old", "new"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                10u64,
                false,
            ),
            (
                "workspace.grep",
                "Search UTF-8 text files inside the actor's sandbox workspace using a regular expression.",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string" },
                        "glob": { "type": "string" },
                        "mode": { "type": "string", "enum": ["matches", "count", "files"], "default": "matches" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 }
                    },
                    "required": ["pattern"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                25u64,
                false,
            ),
            (
                "workspace.glob",
                "Recursively list paths matching a glob pattern inside the actor's sandbox workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 200 }
                    },
                    "required": ["pattern"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                15u64,
                false,
            ),
            (
                "workspace.run",
                "Execute a bounded shell command inside the actor's sandbox workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "timeout": { "type": "integer", "minimum": 1 },
                        "cwd": { "type": "string" }
                    },
                    "required": ["command"]
                }),
                CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: true,
                },
                1_000u64,
                true,
            ),
        ];

        let mut descriptors: Vec<CapabilityDescriptor> = specs
            .into_iter()
            .map(
                |(id, desc, input_schema, flags, latency_ms, long_running)| {
                    let schema = json!({
                        "capability": id,
                        "input": input_schema,
                        "place": self.place.as_str(),
                        "long_running": long_running,
                        "irreversible": flags.irreversible,
                    });
                    CapabilityDescriptor {
                        id: CapabilityId::new(id),
                        description: desc.to_owned(),
                        places: vec![self.place.clone()],
                        flags,
                        typical_latency_ms: latency_ms,
                        capability_schema_hash: capability_schema_hash(&schema),
                        sensed_effect: None,
                    }
                },
            )
            .collect();
        sort_capabilities(&mut descriptors);
        descriptors
    }

    fn execute_capability(
        &self,
        cap: &str,
        payload: &Value,
        audience: &Audience,
    ) -> Result<Value, OpError> {
        let actor_key = resolve_actor_key(payload, audience, self.body_id.as_str());
        let actor_root = ensure_actor_root(&self.workspace_root, &actor_key)?;
        let quota_bytes = effective_quota_bytes(&self.config, payload, self.rbac_enabled);

        match cap {
            "workspace.read" => self.cap_read(&actor_root, payload),
            "workspace.write" => self.cap_write(&actor_root, payload, quota_bytes),
            "workspace.edit" => self.cap_edit(&actor_root, payload, quota_bytes),
            "workspace.grep" => self.cap_grep(&actor_root, payload),
            "workspace.glob" => self.cap_glob(&actor_root, payload),
            "workspace.run" => self.cap_run(&actor_root, payload, quota_bytes),
            _ => Err(OpError::failed("ERR_UNKNOWN_CAPABILITY", format!("unknown capability: {cap}"))),
        }
    }

    fn cap_read(&self, actor_root: &Path, payload: &Value) -> Result<Value, OpError> {
        let raw_path = required_nonempty_str(payload, "path")?;
        let target = resolve_in_sandbox(actor_root, raw_path)?;
        let meta = fs::symlink_metadata(&target).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SandboxError::NotFound(format!("no such file: {raw_path}"))
            } else {
                SandboxError::Io(format!("failed to stat '{raw_path}': {e}"))
            }
        })?;
        if meta.is_dir() {
            return Err(SandboxError::IsDirectory(format!(
                "path '{raw_path}' is a directory"
            ))
            .into());
        }
        if meta.len() > self.config.max_file_bytes {
            return Err(SandboxError::FileTooLarge {
                size: meta.len(),
                max: self.config.max_file_bytes,
            }
            .into());
        }

        let bytes = fs::read(&target)
            .map_err(|e| SandboxError::Io(format!("failed to read '{raw_path}': {e}")))?;
        let text = validate_utf8_text(&bytes)?;

        let offset = payload
            .get("offset")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_READ_OFFSET, |v| v as usize);
        let limit = payload
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_READ_LIMIT, |v| (v as usize).clamp(1, MAX_READ_LIMIT));

        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len();
        let start = offset.min(total_lines);
        let end = start.saturating_add(limit).min(total_lines);
        let window = lines[start..end].join("\n");

        Ok(json!({
            "path": display_relative(actor_root, &target),
            "offset": offset,
            "total_lines": total_lines,
            "text": window,
        }))
    }

    fn cap_write(
        &self,
        actor_root: &Path,
        payload: &Value,
        quota_bytes: u64,
    ) -> Result<Value, OpError> {
        let raw_path = required_nonempty_str(payload, "path")?;
        let text = payload
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| OpError::failed("ERR_INVALID_ARGS", "missing string field 'text'"))?;

        let target = resolve_in_sandbox(actor_root, raw_path)?;
        if target == actor_root {
            return Err(
                SandboxError::IsDirectory("cannot overwrite workspace root directory".to_owned())
                    .into(),
            );
        }
        if let Ok(meta) = fs::symlink_metadata(&target)
            && meta.is_dir()
        {
            return Err(
                SandboxError::IsDirectory(format!("path '{raw_path}' is a directory")).into(),
            );
        }

        let bytes = text.as_bytes();
        check_write_limits(
            actor_root,
            &target,
            bytes.len() as u64,
            self.config.max_file_bytes,
            quota_bytes,
        )?;
        write_atomic(&target, bytes)?;

        Ok(json!({
            "path": display_relative(actor_root, &target),
            "bytes_written": bytes.len(),
        }))
    }

    fn cap_edit(
        &self,
        actor_root: &Path,
        payload: &Value,
        quota_bytes: u64,
    ) -> Result<Value, OpError> {
        let raw_path = required_nonempty_str(payload, "path")?;
        let old = payload
            .get("old")
            .and_then(Value::as_str)
            .ok_or_else(|| OpError::failed("ERR_INVALID_ARGS", "missing string field 'old'"))?;
        let new = payload
            .get("new")
            .and_then(Value::as_str)
            .ok_or_else(|| OpError::failed("ERR_INVALID_ARGS", "missing string field 'new'"))?;
        if old.is_empty() {
            return Err(OpError::failed(
                "ERR_INVALID_ARGS",
                "field 'old' must not be empty",
            ));
        }
        let replace_all = payload
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let target = resolve_in_sandbox(actor_root, raw_path)?;
        let meta = fs::symlink_metadata(&target).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SandboxError::NotFound(format!("no such file: {raw_path}"))
            } else {
                SandboxError::Io(format!("failed to stat '{raw_path}': {e}"))
            }
        })?;
        if meta.is_dir() {
            return Err(SandboxError::IsDirectory(format!(
                "path '{raw_path}' is a directory"
            ))
            .into());
        }
        if meta.len() > self.config.max_file_bytes {
            return Err(SandboxError::FileTooLarge {
                size: meta.len(),
                max: self.config.max_file_bytes,
            }
            .into());
        }

        let bytes = fs::read(&target)
            .map_err(|e| SandboxError::Io(format!("failed to read '{raw_path}': {e}")))?;
        let text = validate_utf8_text(&bytes)?;

        let matches = text.matches(old).count();
        if matches == 0 {
            return Err(OpError::failed(
                "ERR_EDIT_FAILED",
                "old text not found in file",
            ));
        }
        if !replace_all && matches != 1 {
            return Err(OpError::failed(
                "ERR_EDIT_FAILED",
                format!("old text is not unique ({matches} matches)"),
            ));
        }

        let (updated, replacements) = if replace_all {
            (text.replace(old, new), matches)
        } else {
            (text.replacen(old, new, 1), 1)
        };

        let out_bytes = updated.as_bytes();
        check_write_limits(
            actor_root,
            &target,
            out_bytes.len() as u64,
            self.config.max_file_bytes,
            quota_bytes,
        )?;
        write_atomic(&target, out_bytes)?;

        Ok(json!({
            "path": display_relative(actor_root, &target),
            "replacements": replacements,
        }))
    }

    fn cap_grep(&self, actor_root: &Path, payload: &Value) -> Result<Value, OpError> {
        let pattern = required_nonempty_str(payload, "pattern")?;
        let compiled = Regex::new(pattern)
            .map_err(|e| OpError::failed("ERR_INVALID_REGEX", format!("invalid regex: {e}")))?;

        let raw_path = payload
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let base = if raw_path.is_empty() {
            actor_root.to_path_buf()
        } else {
            resolve_in_sandbox(actor_root, raw_path)?
        };
        if !base.exists() {
            return Err(SandboxError::NotFound(format!("no such path: {raw_path}")).into());
        }

        let mode = payload
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("matches");
        if !matches!(mode, "matches" | "count" | "files") {
            return Err(OpError::failed(
                "ERR_INVALID_ARGS",
                format!("invalid grep mode '{mode}', expected 'matches', 'count', or 'files'"),
            ));
        }

        let glob_regex = match payload
            .get("glob")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(g) => {
                validate_glob_pattern(g)?;
                let normalized_glob = g.replace('\\', "/");
                let effective_glob = if normalized_glob.contains('/') {
                    normalized_glob
                } else {
                    format!("**/{normalized_glob}")
                };
                Some(compile_glob(&effective_glob)?)
            }
            None => None,
        };

        let limit = payload
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_GREP_LIMIT, |v| {
                (v as usize).clamp(1, MAX_GREP_LIMIT)
            });

        let files = collect_sandbox_files(actor_root, &base)?;
        let mut match_rows = Vec::new();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();

        for file_path in files {
            let rel = display_relative(actor_root, &file_path);
            if let Some(ref filter) = glob_regex
                && !filter.is_match(&rel)
            {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&file_path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > self.config.max_file_bytes {
                continue;
            }
            let Ok(bytes) = fs::read(&file_path) else {
                continue;
            };
            let Ok(text) = validate_utf8_text(&bytes) else {
                continue;
            };

            for (idx, line) in text.lines().enumerate() {
                if compiled.is_match(line) {
                    *counts.entry(rel.clone()).or_insert(0) += 1;
                    if mode == "matches" && match_rows.len() < limit {
                        let snippet: String = line.chars().take(MAX_GREP_LINE_CHARS).collect();
                        match_rows.push(json!({
                            "path": rel,
                            "line": idx + 1,
                            "text": snippet,
                        }));
                    }
                    if mode == "matches" && match_rows.len() >= limit {
                        break;
                    }
                }
            }
            if mode == "matches" && match_rows.len() >= limit {
                break;
            }
        }

        match mode {
            "files" => {
                let files_list: Vec<String> = counts.keys().take(limit).cloned().collect();
                let count = files_list.len();
                Ok(json!({
                    "files": files_list,
                    "count": count,
                }))
            }
            "count" => {
                let limited: BTreeMap<String, usize> = counts
                    .into_iter()
                    .take(limit)
                    .collect();
                let total: usize = limited.values().sum();
                Ok(json!({
                    "counts": limited,
                    "total": total,
                }))
            }
            _ => {
                let count = match_rows.len();
                Ok(json!({
                    "matches": match_rows,
                    "count": count,
                }))
            }
        }
    }

    fn cap_glob(&self, actor_root: &Path, payload: &Value) -> Result<Value, OpError> {
        let pattern = required_nonempty_str(payload, "pattern")?;
        validate_glob_pattern(pattern)?;
        let matcher = compile_glob(pattern)?;

        let limit = payload
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_GLOB_LIMIT, |v| {
                (v as usize).clamp(1, MAX_GLOB_LIMIT)
            });

        let files = collect_sandbox_files(actor_root, actor_root)?;
        let mut matches = Vec::new();
        for file_path in files {
            let rel = display_relative(actor_root, &file_path);
            if matcher.is_match(&rel) {
                matches.push(rel);
                if matches.len() >= limit {
                    break;
                }
            }
        }

        Ok(json!({ "matches": matches }))
    }

    fn cap_run(
        &self,
        actor_root: &Path,
        payload: &Value,
        quota_bytes: u64,
    ) -> Result<Value, OpError> {
        if !self.config.run_enabled {
            return Err(OpError::refused(
                "ERR_RUN_DISABLED",
                "workspace.run is disabled by configuration",
            ));
        }

        let used = recursive_dir_bytes(actor_root)?;
        if (self.rbac_enabled && quota_bytes == 0) || used > quota_bytes {
            return Err(SandboxError::QuotaExceeded {
                used,
                quota: quota_bytes,
            }
            .into());
        }

        let command = required_nonempty_str(payload, "command")?;
        let max_secs = self.config.timeout_secs.max(1);
        let timeout_secs = payload
            .get("timeout")
            .and_then(Value::as_u64)
            .map_or(max_secs, |v| v.clamp(1, max_secs));

        let run_cwd = match payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw_cwd) => {
                let resolved = resolve_in_sandbox(actor_root, raw_cwd)?;
                let meta = fs::symlink_metadata(&resolved).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        SandboxError::NotFound(format!("no such directory: {raw_cwd}"))
                    } else {
                        SandboxError::Io(format!("failed to stat cwd '{raw_cwd}': {e}"))
                    }
                })?;
                if !meta.is_dir() {
                    return Err(SandboxError::NotDirectory(format!(
                        "cwd '{raw_cwd}' is not a directory"
                    ))
                    .into());
                }
                resolved
            }
            None => actor_root.to_path_buf(),
        };

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&run_cwd)
            .env_clear()
            .env("PATH", SAFE_PATH_ENV)
            .env("HOME", actor_root)
            .env("PWD", &run_cwd)
            .env("LANG", "C.UTF-8")
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| OpError::failed("ERR_RUN_SPAWN", format!("failed to spawn shell: {e}")))?;

        let cap = self.config.max_output_bytes.max(1);
        let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (done_tx, done_rx) = mpsc::channel::<()>();

        if let Some(stdout) = child.stdout.take() {
            spawn_capped_reader(stdout, Arc::clone(&stdout_buf), cap, done_tx.clone());
        } else {
            let _ = done_tx.send(());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_capped_reader(stderr, Arc::clone(&stderr_buf), cap, done_tx);
        } else {
            let _ = done_tx.send(());
        }

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut timed_out = false;
        let mut exit_code: Option<i32> = None;

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_code = status.code();
                    break;
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(OpError::failed(
                        "ERR_RUN_WAIT",
                        format!("failed while waiting on command: {err}"),
                    ));
                }
            }
        }

        // Wait briefly for stream readers to hit EOF; bound wait on timeout in case a
        // grandchild inherited pipe FDs.
        let drain_budget = if timed_out {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(2)
        };
        let drain_deadline = Instant::now() + drain_budget;
        for _ in 0..2 {
            let rem = drain_deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() || done_rx.recv_timeout(rem).is_err() {
                break;
            }
        }

        let stdout_str = stdout_buf
            .lock()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let stderr_str = stderr_buf
            .lock()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();

        Ok(json!({
            "exit_code": exit_code,
            "timed_out": timed_out,
            "stdout": stdout_str,
            "stderr": stderr_str,
        }))
    }
}

impl BodyHandler for WorkspaceBody {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let capabilities = self.capability_descriptors();
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities,
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }

        let cap_str = params.capability_id.as_str();
        if !matches!(
            cap_str,
            "workspace.read"
                | "workspace.write"
                | "workspace.edit"
                | "workspace.grep"
                | "workspace.glob"
                | "workspace.run"
        ) {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-workspace",
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
                Some("awaiting owner approval before workspace execution".to_owned()),
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

        let is_delivery_probe = params.payload.as_bytes() == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));
        if is_delivery_probe {
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Done,
                None,
                Some(
                    json!({
                        "d_art_ms": 0,
                        "working_cue_ms": 1_000,
                        "progress_note_ms": 10_000,
                    })
                    .to_string(),
                ),
                Some(TerminationReason::Normal),
                Some(1.0),
                Some(1),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let payload_val: Value = match serde_json::from_slice::<Value>(params.payload.as_bytes()) {
            Ok(Value::Object(map)) => Value::Object(map),
            _ => {
                let receipt = make_terminal_receipt(
                    params.action_id,
                    params.capability_id,
                    ReceiptOutcome::Failed,
                    Some("ERR_INVALID_PAYLOAD".to_owned()),
                    Some("payload must be a JSON object".to_owned()),
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
        };

        let receipt = match self.execute_capability(cap_str, &payload_val, &params.audience) {
            Ok(result_json) => make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Done,
                None,
                Some(result_json.to_string()),
                Some(TerminationReason::Normal),
                Some(1.0),
                Some(1),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            ),
            Err(op_err) if op_err.refused => make_refused_receipt(
                params.action_id,
                params.capability_id,
                op_err.reason_code,
                op_err.detail,
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            ),
            Err(op_err) => make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Failed,
                Some(op_err.reason_code.to_owned()),
                Some(op_err.detail),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            ),
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
            CapabilityId::new("workspace.run"),
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
        if self.held_actions.remove(&params.action_id) {
            return Ok(ApprovalResolveResult { applied: true });
        }
        Ok(ApprovalResolveResult { applied: false })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-workspace",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}

struct OpError {
    reason_code: &'static str,
    detail: String,
    refused: bool,
}

impl OpError {
    fn failed(reason_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code,
            detail: detail.into(),
            refused: false,
        }
    }

    fn refused(reason_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code,
            detail: detail.into(),
            refused: true,
        }
    }
}

impl From<SandboxError> for OpError {
    fn from(err: SandboxError) -> Self {
        Self {
            reason_code: err.reason_code(),
            refused: err.is_refusal(),
            detail: err.to_string(),
        }
    }
}

fn required_nonempty_str<'a>(payload: &'a Value, field: &str) -> Result<&'a str, OpError> {
    let val = payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OpError::failed(
                "ERR_INVALID_ARGS",
                format!("missing or empty string field '{field}'"),
            )
        })?;
    Ok(val)
}

fn validate_glob_pattern(pattern: &str) -> Result<(), OpError> {
    if pattern.contains('\0') {
        return Err(OpError::refused(
            "ERR_PATH_OUTSIDE_SANDBOX",
            "null byte is not allowed in glob pattern",
        ));
    }
    let normalized = pattern.trim().replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(OpError::refused(
            "ERR_PATH_OUTSIDE_SANDBOX",
            format!("{pattern} is outside your workspace"),
        ));
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(OpError::refused(
            "ERR_PATH_OUTSIDE_SANDBOX",
            format!("{pattern} is outside your workspace"),
        ));
    }
    if normalized.split('/').any(|seg| seg == "..") {
        return Err(OpError::refused(
            "ERR_PATH_OUTSIDE_SANDBOX",
            format!("{pattern} is outside your workspace"),
        ));
    }
    Ok(())
}

fn compile_glob(pattern: &str) -> Result<Regex, OpError> {
    let normalized = pattern.trim().replace('\\', "/");
    let chars: Vec<char> = normalized.chars().collect();
    let mut regex_str = String::from("^");
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if chars.get(i + 2) == Some(&'/') {
                regex_str.push_str("(?:.*/)?");
                i += 3;
            } else {
                regex_str.push_str(".*");
                i += 2;
            }
        } else if chars[i] == '*' {
            regex_str.push_str("[^/]*");
            i += 1;
        } else if chars[i] == '?' {
            regex_str.push_str("[^/]");
            i += 1;
        } else {
            let ch = chars[i];
            if matches!(
                ch,
                '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\'
            ) {
                regex_str.push('\\');
            }
            regex_str.push(ch);
            i += 1;
        }
    }
    regex_str.push('$');

    Regex::new(&regex_str)
        .map_err(|e| OpError::failed("ERR_INVALID_ARGS", format!("invalid glob pattern: {e}")))
}

fn collect_sandbox_files(canonical_root: &Path, start: &Path) -> Result<Vec<PathBuf>, OpError> {
    if start.is_file() {
        let canon = fs::canonicalize(start).map_err(|e| {
            SandboxError::Io(format!("failed to canonicalize '{}': {e}", start.display()))
        })?;
        if !canon.starts_with(canonical_root) {
            return Err(SandboxError::PathOutsideSandbox(
                "file escapes workspace sandbox".to_owned(),
            )
            .into());
        }
        return Ok(vec![canon]);
    }

    let mut visited_dirs = BTreeSet::new();
    let mut stack = vec![start.to_path_buf()];
    let mut files = Vec::new();

    while let Some(dir) = stack.pop() {
        let Ok(canon_dir) = fs::canonicalize(&dir) else {
            continue;
        };
        if !canon_dir.starts_with(canonical_root) || !visited_dirs.insert(canon_dir.clone()) {
            continue;
        }

        let Ok(entries) = fs::read_dir(&canon_dir) else {
            continue;
        };
        let mut child_paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        child_paths.sort();

        for path in child_paths {
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn spawn_capped_reader<R: Read + Send + 'static>(
    mut stream: R,
    target: Arc<Mutex<Vec<u8>>>,
    cap: usize,
    done_tx: mpsc::Sender<()>,
) {
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut buf) = target.lock()
                        && buf.len() < cap
                    {
                        let room = cap - buf.len();
                        buf.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
            }
        }
        let _ = done_tx.send(());
    });
}
