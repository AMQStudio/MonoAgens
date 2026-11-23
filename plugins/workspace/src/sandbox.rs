//! Strict per-actor sandbox path containment, binary detection, atomic writes, and disk quotas.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use monoagens_body_sdk::Audience;
use serde_json::Value;

use crate::config::WorkspaceConfig;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Typed error produced by sandbox validation and filesystem containment checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    PathOutsideSandbox(String),
    NotFound(String),
    IsDirectory(String),
    NotDirectory(String),
    NotText(String),
    FileTooLarge { size: u64, max: u64 },
    QuotaExceeded { used: u64, quota: u64 },
    Io(String),
}

impl SandboxError {
    /// Stable Waist-2 receipt `reason_code` for this error class.
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::PathOutsideSandbox(_) => "ERR_PATH_OUTSIDE_SANDBOX",
            Self::NotFound(_) => "ERR_NOT_FOUND",
            Self::IsDirectory(_) => "ERR_IS_DIRECTORY",
            Self::NotDirectory(_) => "ERR_NOT_DIRECTORY",
            Self::NotText(_) => "ERR_NOT_TEXT",
            Self::FileTooLarge { .. } => "ERR_FILE_TOO_LARGE",
            Self::QuotaExceeded { .. } => "ERR_QUOTA_EXCEEDED",
            Self::Io(_) => "ERR_WORKSPACE_IO",
        }
    }

    /// Whether this error represents a policy/containment refusal (`Refused`) vs operational failure.
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        matches!(
            self,
            Self::PathOutsideSandbox(_) | Self::FileTooLarge { .. } | Self::QuotaExceeded { .. }
        )
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathOutsideSandbox(msg)
            | Self::NotFound(msg)
            | Self::IsDirectory(msg)
            | Self::NotDirectory(msg)
            | Self::NotText(msg)
            | Self::Io(msg) => f.write_str(msg),
            Self::FileTooLarge { size, max } => {
                write!(f, "file size {size} bytes exceeds max_file_bytes ({max})")
            }
            Self::QuotaExceeded { used, quota } => {
                write!(
                    f,
                    "workspace quota exceeded: {used} bytes required, quota is {quota} bytes"
                )
            }
        }
    }
}

/// Determines the actor identifier from payload `_who` / `actor` or the first non-self
/// audience member, sanitized strictly to `[A-Za-z0-9_-]`.
#[must_use]
pub fn resolve_actor_key(payload: &Value, audience: &Audience, body_id: &str) -> String {
    let from_payload = payload
        .get("_who")
        .or_else(|| payload.get("actor"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let raw = from_payload.or_else(|| {
        audience
            .members
            .iter()
            .map(|a| a.as_str().trim())
            .find(|s| !s.is_empty() && *s != "self" && *s != body_id)
            .or_else(|| audience.members.first().map(|a| a.as_str().trim()))
    });

    sanitize_actor_key(raw.unwrap_or("unknown"))
}

/// Sanitizes an actor identifier strictly to `[A-Za-z0-9_-]`, guaranteeing a single safe
/// directory component (never empty, `.`, or `..`).
#[must_use]
pub fn sanitize_actor_key(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

/// Computes the effective disk quota in bytes for an actor/payload.
///
/// When `rbac_enabled` is true and the payload carries `_role` or `role`, uses the
/// matching entry in `config.role_quotas`. `"owner"` / `"admin"` receive a generous quota
/// (falling back to `"admin"` or `10_737_418_240` bytes if not explicitly overridden).
#[must_use]
pub fn effective_quota_bytes(config: &WorkspaceConfig, payload: &Value, rbac_enabled: bool) -> u64 {
    if !rbac_enabled {
        return config.default_quota_bytes;
    }

    let role = payload
        .get("_role")
        .or_else(|| payload.get("role"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(role_name) = role else {
        return config.default_quota_bytes;
    };

    if let Some(&quota) = config.role_quotas.get(role_name) {
        return quota;
    }

    let lower = role_name.to_ascii_lowercase();
    if let Some(&quota) = config.role_quotas.get(&lower) {
        return quota;
    }

    if matches!(lower.as_str(), "owner" | "admin") {
        return config
            .role_quotas
            .get("admin")
            .copied()
            .unwrap_or(10_737_418_240);
    }

    config.default_quota_bytes
}

/// Ensures the actor's sandbox directory exists and returns its canonical path.
pub fn ensure_actor_root(root: &Path, actor_key: &str) -> Result<PathBuf, SandboxError> {
    let safe_key = sanitize_actor_key(actor_key);
    let actor_dir = root.join(safe_key);
    fs::create_dir_all(&actor_dir)
        .map_err(|e| SandboxError::Io(format!("failed to create actor workspace: {e}")))?;
    fs::canonicalize(&actor_dir)
        .map_err(|e| SandboxError::Io(format!("failed to canonicalize actor workspace: {e}")))
}

/// Validates and resolves `raw_path` strictly inside `canonical_root`.
///
/// Rejects null bytes, absolute paths, Windows drive prefixes, `..` components,
/// and any existing or dangling symlink pointing outside `canonical_root`.
pub fn resolve_in_sandbox(canonical_root: &Path, raw_path: &str) -> Result<PathBuf, SandboxError> {
    if raw_path.contains('\0') {
        return Err(SandboxError::PathOutsideSandbox(
            "null byte is not allowed in path".to_owned(),
        ));
    }

    let normalized = raw_path.trim().replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(SandboxError::PathOutsideSandbox(format!(
            "{raw_path} is outside your workspace"
        )));
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(SandboxError::PathOutsideSandbox(format!(
            "{raw_path} is outside your workspace"
        )));
    }

    let mut clean_segments = Vec::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(seg) => clean_segments.push(seg.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(SandboxError::PathOutsideSandbox(format!(
                    "{raw_path} is outside your workspace"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(SandboxError::PathOutsideSandbox(format!(
                    "{raw_path} is outside your workspace"
                )));
            }
        }
    }

    let mut current = canonical_root.to_path_buf();
    for (idx, seg) in clean_segments.iter().enumerate() {
        let candidate = current.join(seg);
        match fs::symlink_metadata(&candidate) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    let resolved = fs::canonicalize(&candidate).map_err(|_| {
                        SandboxError::PathOutsideSandbox(format!(
                            "{raw_path} references an unresolvable symlink"
                        ))
                    })?;
                    if !resolved.starts_with(canonical_root) {
                        return Err(SandboxError::PathOutsideSandbox(format!(
                            "{raw_path} escapes workspace via symlink"
                        )));
                    }
                    current = resolved;
                } else if idx + 1 < clean_segments.len() {
                    if !meta.is_dir() {
                        return Err(SandboxError::NotDirectory(format!(
                            "parent component in '{raw_path}' is not a directory"
                        )));
                    }
                    let resolved = fs::canonicalize(&candidate).map_err(|e| {
                        SandboxError::Io(format!("failed to resolve '{}': {e}", candidate.display()))
                    })?;
                    if !resolved.starts_with(canonical_root) {
                        return Err(SandboxError::PathOutsideSandbox(format!(
                            "{raw_path} is outside your workspace"
                        )));
                    }
                    current = resolved;
                } else {
                    let resolved = fs::canonicalize(&candidate).map_err(|e| {
                        SandboxError::Io(format!("failed to resolve '{}': {e}", candidate.display()))
                    })?;
                    if !resolved.starts_with(canonical_root) {
                        return Err(SandboxError::PathOutsideSandbox(format!(
                            "{raw_path} is outside your workspace"
                        )));
                    }
                    current = resolved;
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                current.push(seg);
                for rem in &clean_segments[idx + 1..] {
                    current.push(rem);
                }
                return Ok(current);
            }
            Err(err) => {
                return Err(SandboxError::Io(format!(
                    "failed to inspect '{}': {err}",
                    candidate.display()
                )));
            }
        }
    }

    Ok(current)
}

/// Formats a sandboxed path as a clean POSIX relative path inside `canonical_root`.
#[must_use]
pub fn display_relative(canonical_root: &Path, target: &Path) -> String {
    target
        .strip_prefix(canonical_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| target.to_string_lossy().replace('\\', "/"))
}

/// Returns `true` if the first 4 KiB contain non-text control bytes (`[\x00-\x08\x0b\x0c\x0e-\x1f]`).
#[must_use]
pub fn has_binary_control_bytes(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4096)];
    sample
        .iter()
        .any(|&b| matches!(b, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f))
}

/// Validates that `bytes` is UTF-8 text without binary control bytes in its first 4 KiB.
pub fn validate_utf8_text(bytes: &[u8]) -> Result<&str, SandboxError> {
    if has_binary_control_bytes(bytes) {
        return Err(SandboxError::NotText("file is not UTF-8 text".to_owned()));
    }
    std::str::from_utf8(bytes)
        .map_err(|_| SandboxError::NotText("file is not UTF-8 text".to_owned()))
}

/// Computes total recursive disk usage in bytes under `dir`, skipping symlinks.
pub fn recursive_dir_bytes(dir: &Path) -> Result<u64, SandboxError> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|e| SandboxError::Io(format!("failed to scan '{}': {e}", current.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                SandboxError::Io(format!("failed to read dir entry in '{}': {e}", current.display()))
            })?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| {
                SandboxError::Io(format!("failed to stat '{}': {e}", path.display()))
            })?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

/// Checks both `max_file_bytes` and `quota_bytes` before writing `new_len` bytes to `target`.
pub fn check_write_limits(
    canonical_root: &Path,
    target: &Path,
    new_len: u64,
    max_file_bytes: u64,
    quota_bytes: u64,
) -> Result<(), SandboxError> {
    if new_len > max_file_bytes {
        return Err(SandboxError::FileTooLarge {
            size: new_len,
            max: max_file_bytes,
        });
    }
    let current_used = recursive_dir_bytes(canonical_root)?;
    let existing_len = match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_file() => meta.len(),
        _ => 0,
    };
    let projected = current_used.saturating_sub(existing_len).saturating_add(new_len);
    if quota_bytes == 0 || projected > quota_bytes {
        return Err(SandboxError::QuotaExceeded {
            used: projected,
            quota: quota_bytes,
        });
    }
    Ok(())
}

/// Atomically writes `bytes` to `target` within its parent directory inside the sandbox.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), SandboxError> {
    let parent = target.parent().ok_or_else(|| {
        SandboxError::PathOutsideSandbox("target path must have a parent directory".to_owned())
    })?;
    fs::create_dir_all(parent)
        .map_err(|e| SandboxError::Io(format!("failed to create parent directory: {e}")))?;

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let filename = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp_path = parent.join(format!(".{filename}.{pid}.{seq}.tmp"));

    let write_res = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(SandboxError::Io(format!("failed to write temporary file: {err}")));
    }

    if let Err(err) = fs::rename(&tmp_path, target) {
        let _ = fs::remove_file(&tmp_path);
        return Err(SandboxError::Io(format!("failed to replace target file: {err}")));
    }
    Ok(())
}
