//! Judge runner used by the memory suite (`Design 3`). Calls go through the metering proxy with
//! the participant attribution `judge`; prompt texts are the published ones, verbatim.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use crate::LabError;
use crate::memory::proxy::CallAttribution;

/// One judge transport used by the scorers.
pub trait JudgeRunner: Send + Sync {
    /// Sends one judge prompt and returns the raw model response text.
    fn judge(&self, prompt: &str, attribution: &CallAttribution) -> Result<String, LabError>;

    /// Number of judge calls attempted so far.
    fn calls(&self) -> u64;
}

/// HTTP judge client posting to the lab metering proxy.
pub struct HttpJudgeClient {
    endpoint: String,
    model: String,
    client: reqwest::blocking::Client,
    calls: Mutex<u64>,
}

impl HttpJudgeClient {
    /// Builds a judge client for the proxy's chat-completions endpoint using `model`.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Result<Self, LabError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| LabError::Protocol(format!("judge HTTP client: {e}")))?;
        Ok(Self {
            endpoint: endpoint.into(),
            model: model.into(),
            client,
            calls: Mutex::new(0),
        })
    }
}

impl JudgeRunner for HttpJudgeClient {
    fn judge(&self, prompt: &str, attribution: &CallAttribution) -> Result<String, LabError> {
        {
            let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
            *calls += 1;
        }
        let body = json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0,
            "max_completion_tokens": crate::memory::suite::MEMORY_MAX_OUTPUT_TOKENS,
        });
        let mut last_error = String::new();
        for attempt in 0..3_u32 {
            let mut req = self
                .client
                .post(&self.endpoint)
                .header("content-type", "application/json")
                .json(&body);
            for (name, value) in attribution.headers() {
                req = req.header(name, value);
            }
            req = req.header("x-lab-purpose", "judge");
            match req.send() {
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok());
                    let text = resp.text().unwrap_or_default();
                    if status.is_success() {
                        let value: serde_json::Value =
                            serde_json::from_str(&text).map_err(|e| {
                                LabError::Protocol(format!("judge response envelope: {e}"))
                            })?;
                        let content = value
                            .get("choices")
                            .and_then(|c| c.as_array())
                            .and_then(|a| a.first())
                            .and_then(|c| c.get("message"))
                            .and_then(|m| m.get("content"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        return Ok(content);
                    }
                    let transient = status.as_u16() == 429 || status.is_server_error();
                    last_error = format!("judge HTTP {}: {text}", status.as_u16());
                    if transient && attempt < 2 {
                        std::thread::sleep(Duration::from_millis(
                            retry_after
                                .map(|s| s.saturating_mul(1_000))
                                .unwrap_or(1_000_u64 << attempt),
                        ));
                        continue;
                    }
                    return Err(LabError::Protocol(last_error));
                }
                Err(err) => {
                    last_error = format!("judge request failed: {err}");
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(1_000_u64 << attempt));
                        continue;
                    }
                }
            }
        }
        Err(LabError::Protocol(last_error))
    }

    fn calls(&self) -> u64 {
        *self.calls.lock().unwrap_or_else(|e| e.into_inner())
    }
}
