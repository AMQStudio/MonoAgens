//! OpenAI-compatible remote `/v1/embeddings` HTTP adapter.

use core::fmt;
use std::time::Duration;

use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;

use crate::embed::{
    EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix, estimate_tokens,
    l2_normalize,
};
use crate::secrets::{SecretString, redact_secrets};

const TEXT_ONLY: [EmbedModality; 1] = [EmbedModality::Text];

/// Configuration for [`OpenAiEmbedder`].
#[derive(Clone)]
pub struct OpenAiEmbedConfig {
    pub endpoint_url: String,
    pub model: String,
    pub embed_model_ver: String,
    pub dim_d: u32,
    pub api_key: Option<SecretString>,
    pub timeout_ms: u64,
    pub normalize: bool,
    pub send_dimensions: bool,
    pub apply_task_prefix: bool,
    pub max_context_tokens: Option<usize>,
}

impl fmt::Debug for OpenAiEmbedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiEmbedConfig")
            .field("endpoint_url", &self.endpoint_url)
            .field("model", &self.model)
            .field("embed_model_ver", &self.embed_model_ver)
            .field("dim_d", &self.dim_d)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("timeout_ms", &self.timeout_ms)
            .field("normalize", &self.normalize)
            .field("send_dimensions", &self.send_dimensions)
            .field("apply_task_prefix", &self.apply_task_prefix)
            .field("max_context_tokens", &self.max_context_tokens)
            .finish()
    }
}

impl OpenAiEmbedConfig {
    /// Creates a config with sensible defaults (`15_000 ms` timeout, L2-normalized, task-prefixed).
    #[must_use]
    pub fn new(
        endpoint_url: impl Into<String>,
        model: impl Into<String>,
        dim_d: u32,
        api_key: Option<SecretString>,
    ) -> Self {
        let model = model.into();
        Self {
            endpoint_url: endpoint_url.into(),
            embed_model_ver: model.clone(),
            model,
            dim_d: dim_d.max(1),
            api_key,
            timeout_ms: 15_000,
            normalize: true,
            send_dimensions: true,
            apply_task_prefix: true,
            max_context_tokens: Some(8192),
        }
    }
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

/// Blocking HTTP client for OpenAI-compatible `/v1/embeddings` endpoints.
#[derive(Debug)]
pub struct OpenAiEmbedder {
    config: OpenAiEmbedConfig,
    client: Client,
}

impl OpenAiEmbedder {
    /// Builds a new [`OpenAiEmbedder`] enforcing the configured request timeout.
    pub fn new(config: OpenAiEmbedConfig) -> Result<Self, EmbedError> {
        if config.endpoint_url.trim().is_empty() {
            return Err(EmbedError::Transport {
                detail: "embedding endpoint_url must not be empty".to_owned(),
            });
        }
        if config.timeout_ms == 0 {
            return Err(EmbedError::Timeout {
                timeout_ms: 0,
                detail: "embedding timeout_ms must be > 0".to_owned(),
            });
        }
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|err| EmbedError::Transport {
                detail: format!("failed to build HTTP client: {err}"),
            })?;
        Ok(Self { config, client })
    }

    fn scrub(&self, text: &str) -> String {
        match &self.config.api_key {
            Some(key) => redact_secrets(text, &[key]),
            None => redact_secrets(text, &[]),
        }
    }
}

impl Embedder for OpenAiEmbedder {
    fn backend_name(&self) -> &str {
        "openai_http"
    }

    fn model_ver(&self) -> Option<&str> {
        Some(&self.config.embed_model_ver)
    }

    fn dim_d(&self) -> Option<u32> {
        Some(self.config.dim_d)
    }

    fn max_context_tokens(&self) -> Option<usize> {
        self.config.max_context_tokens
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        &TEXT_ONLY
    }

    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag> {
        Some(EmbeddingTag {
            embed_model_ver: self.config.embed_model_ver.clone(),
            dim_d: self.config.dim_d,
            prompt_regime_id: task.regime_id().to_owned(),
        })
    }

    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        let text = match input.content {
            EmbedContent::Text(t) => t,
            other => {
                return Err(EmbedError::UnsupportedModality {
                    backend: self.backend_name().to_owned(),
                    modality: other.modality(),
                });
            }
        };

        let formatted = if self.config.apply_task_prefix {
            input.task.format_text(text)
        } else {
            text.to_owned()
        };

        if let Some(max_tok) = self.config.max_context_tokens {
            let est = estimate_tokens(&formatted);
            if est > max_tok {
                return Err(EmbedError::ContextLimitExceeded {
                    backend: self.backend_name().to_owned(),
                    estimated_tokens: est,
                    max_tokens: max_tok,
                });
            }
        }

        let mut body = json!({
            "model": self.config.model,
            "input": formatted,
        });
        if self.config.send_dimensions
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("dimensions".to_owned(), json!(self.config.dim_d));
        }

        let mut req = self.client.post(&self.config.endpoint_url).json(&body);
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key.expose());
        }

        let resp = req.send().map_err(|err| {
            let detail = self.scrub(&err.to_string());
            if err.is_timeout() {
                EmbedError::Timeout {
                    timeout_ms: self.config.timeout_ms,
                    detail,
                }
            } else {
                EmbedError::Transport { detail }
            }
        })?;

        let status = resp.status();
        let raw_body = resp.text().map_err(|err| {
            let detail = self.scrub(&err.to_string());
            if err.is_timeout() {
                EmbedError::Timeout {
                    timeout_ms: self.config.timeout_ms,
                    detail,
                }
            } else {
                EmbedError::Transport { detail }
            }
        })?;

        if !status.is_success() {
            let safe_detail = self.scrub(&raw_body);
            return Err(match status.as_u16() {
                401 | 403 => EmbedError::Authentication {
                    detail: safe_detail,
                },
                429 => EmbedError::RateLimit {
                    detail: safe_detail,
                },
                408 | 504 => EmbedError::Timeout {
                    timeout_ms: self.config.timeout_ms,
                    detail: safe_detail,
                },
                code if code >= 500 => EmbedError::Server {
                    status: Some(code),
                    detail: safe_detail,
                },
                code => EmbedError::InvalidResponse {
                    detail: format!("HTTP {code}: {safe_detail}"),
                },
            });
        }

        let parsed: EmbeddingsResponse =
            serde_json::from_str(&raw_body).map_err(|err| EmbedError::InvalidResponse {
                detail: self.scrub(&format!("malformed embeddings JSON: {err}")),
            })?;

        let mut values = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| EmbedError::InvalidResponse {
                detail: "embeddings response missing data[0]".to_owned(),
            })?
            .embedding;

        if values.len() != self.config.dim_d as usize {
            return Err(EmbedError::DimensionMismatch {
                expected: self.config.dim_d,
                actual: values.len(),
            });
        }

        if self.config.normalize {
            l2_normalize(&mut values)?;
        }

        Ok(Some(TaggedEmbedding {
            tag: self.tag_for(input.task).expect("tag present"),
            values,
        }))
    }
}
