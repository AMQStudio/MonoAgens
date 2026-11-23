//! Secret-redacting Gateway diagnostics and model-call audit logs.

use std::sync::{Arc, Mutex};

use monoagens_protocol::Time;
use monoagens_reason::{ModelCallRecord, ProviderSecrets, SecretString, redact_secrets};
use serde::{Deserialize, Serialize};

/// Severity level for Gateway diagnostic entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Structured, secret-redacted Gateway diagnostic record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRecord {
    pub timestamp_ms: Time,
    pub level: LogLevel,
    pub component: String,
    pub message: String,
}

/// Thread-safe Gateway diagnostic and audit sink that scrubs all secrets before recording.
#[derive(Clone, Default)]
pub struct DiagnosticSink {
    secrets: Arc<Mutex<Vec<SecretString>>>,
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    model_calls: Arc<Mutex<Vec<ModelCallRecord>>>,
    emit_stderr: bool,
}

impl std::fmt::Debug for DiagnosticSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiagnosticSink")
            .field(
                "record_count",
                &self.records.lock().map(|r| r.len()).unwrap_or(0),
            )
            .field(
                "model_call_count",
                &self.model_calls.lock().map(|m| m.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl DiagnosticSink {
    /// Creates an in-memory redacting diagnostic sink (suitable for tests and embedded runtimes).
    #[must_use]
    pub fn new(secrets: &ProviderSecrets) -> Self {
        let cloned: Vec<SecretString> = secrets.all_secrets().into_iter().cloned().collect();
        Self {
            secrets: Arc::new(Mutex::new(cloned)),
            records: Arc::new(Mutex::new(Vec::new())),
            model_calls: Arc::new(Mutex::new(Vec::new())),
            emit_stderr: false,
        }
    }

    /// Creates a sink that also mirrors redacted diagnostic lines to stderr.
    #[must_use]
    pub fn with_stderr(secrets: &ProviderSecrets) -> Self {
        let mut sink = Self::new(secrets);
        sink.emit_stderr = true;
        sink
    }

    /// Registers additional secrets to scrub from all future log lines.
    pub fn register_secrets(&self, secrets: &ProviderSecrets) {
        let mut guard = self.secrets.lock().expect("diagnostic secrets lock");
        for sec in secrets.all_secrets() {
            if !sec.is_empty() && !guard.iter().any(|existing| existing == sec) {
                guard.push(sec.clone());
            }
        }
    }

    /// Registers a single secret (such as an attachment master key) for output redaction.
    pub fn register_secret(&self, secret: &SecretString) {
        if secret.is_empty() {
            return;
        }
        let mut guard = self.secrets.lock().expect("diagnostic secrets lock");
        if !guard.iter().any(|existing| existing == secret) {
            guard.push(secret.clone());
        }
    }

    /// Registers multiple raw secret string values (such as `[env]` plugin secrets) for output redaction.
    pub fn register_secret_values<'a>(&self, values: impl IntoIterator<Item = &'a str>) {
        let mut guard = self.secrets.lock().expect("diagnostic secrets lock");
        for raw in values {
            if raw.is_empty() {
                continue;
            }
            let secret = SecretString::new(raw.to_owned());
            if !guard.iter().any(|existing| existing == &secret) {
                guard.push(secret);
            }
        }
    }

    /// Scrubs all registered secrets and common token shapes from `text`.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let guard = self.secrets.lock().expect("diagnostic secrets lock");
        let refs: Vec<&SecretString> = guard.iter().collect();
        redact_secrets(text, &refs)
    }

    /// Records a diagnostic event after redacting any secret material.
    pub fn log(
        &self,
        timestamp_ms: Time,
        level: LogLevel,
        component: impl Into<String>,
        message: &str,
    ) {
        let component = component.into();
        let redacted_msg = self.redact(message);
        if self.emit_stderr {
            let lvl = match level {
                LogLevel::Info => "INFO",
                LogLevel::Warn => "WARN",
                LogLevel::Error => "ERROR",
            };
            eprintln!(
                "[{lvl}][{component}][{}] {redacted_msg}",
                timestamp_ms.as_millis()
            );
        }
        self.records
            .lock()
            .expect("diagnostic records lock")
            .push(DiagnosticRecord {
                timestamp_ms,
                level,
                component,
                message: redacted_msg,
            });
    }

    /// Records a completed or failed [`ModelCallRecord`] (with model name scrubbed defensively).
    pub fn record_model_call(&self, mut record: ModelCallRecord) {
        record.model = self.redact(&record.model);
        self.model_calls
            .lock()
            .expect("diagnostic model_calls lock")
            .push(record);
    }

    /// Returns a snapshot of all recorded diagnostic entries.
    #[must_use]
    pub fn records(&self) -> Vec<DiagnosticRecord> {
        self.records
            .lock()
            .expect("diagnostic records lock")
            .clone()
    }

    /// Returns a snapshot of all recorded model-call audit records.
    #[must_use]
    pub fn model_calls(&self) -> Vec<ModelCallRecord> {
        self.model_calls
            .lock()
            .expect("diagnostic model_calls lock")
            .clone()
    }
}
