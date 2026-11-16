//! Unified `EmbeddingGemma 2` (`256d` MRL, `8K` ctx) and `EmbeddingGemma-300M` ablation embedders.
//!
//! Supported execution paths in this build:
//! - Local HTTP sidecar (`SidecarHttp`, requires configured existing `model_path`)
//! - Local stdio sidecar (`SidecarStdio`, requires configured existing `model_path`)
//! - Remote OpenAI-compatible endpoint (`RemoteOpenAi`)
//!
//! Note: Native `ort` (`2.0.0-rc.15`) is intentionally NOT enabled in the default build
//! because its `download-binaries` feature fetches ONNX Runtime binaries at build time.
//! Model weights are not vendored; local backends fail at startup when `model_path` is absent.

use core::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::embed::openai::{OpenAiEmbedConfig, OpenAiEmbedder};
use crate::embed::{
    EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix, estimate_tokens,
    l2_normalize,
};
use crate::secrets::{SecretString, redact_secrets};

/// Canonical model version tag for `EmbeddingGemma 2` (`256d` MRL).
pub const EMBEDDING_GEMMA_2_VER: &str = "egemma2-256";
/// Default MRL output dimensionality for `EmbeddingGemma 2`.
pub const EMBEDDING_GEMMA_2_DIM: u32 = 256;
/// Maximum context window in tokens for `EmbeddingGemma 2` (`8K`).
pub const EMBEDDING_GEMMA_2_CTX_TOKENS: usize = 8192;

/// Canonical model version tag for `EmbeddingGemma-300M` ablation baseline.
pub const EMBEDDING_GEMMA_300M_VER: &str = "egemma-300m";
/// Default output dimensionality for `EmbeddingGemma-300M`.
pub const EMBEDDING_GEMMA_300M_DEFAULT_DIM: u32 = 768;
/// Maximum context window in tokens for `EmbeddingGemma-300M`.
pub const EMBEDDING_GEMMA_300M_CTX_TOKENS: usize = 2048;

const GEMMA2_MODALITIES: [EmbedModality; 3] = [
    EmbedModality::Text,
    EmbedModality::Image,
    EmbedModality::AudioVoice,
];
const GEMMA300M_MODALITIES: [EmbedModality; 1] = [EmbedModality::Text];

/// Backend transport for `EmbeddingGemma 2` and `EmbeddingGemma-300M`.
#[derive(Clone)]
pub enum EmbeddingGemma2Backend {
    /// Local HTTP sidecar (e.g. local Python/LiteRT/ONNX server); requires local `model_path`.
    SidecarHttp {
        endpoint_url: String,
        model_path: Option<PathBuf>,
        timeout_ms: u64,
    },
    /// Local stdio JSON-lines sidecar executable; requires local `model_path`.
    SidecarStdio {
        program: String,
        args: Vec<String>,
        model_path: Option<PathBuf>,
        timeout_ms: u64,
    },
    /// Remote OpenAI-compatible embeddings endpoint.
    RemoteOpenAi {
        endpoint_url: String,
        model: String,
        api_key: Option<SecretString>,
        timeout_ms: u64,
    },
}

impl fmt::Debug for EmbeddingGemma2Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SidecarHttp {
                endpoint_url,
                model_path,
                timeout_ms,
            } => f
                .debug_struct("SidecarHttp")
                .field("endpoint_url", endpoint_url)
                .field("model_path", model_path)
                .field("timeout_ms", timeout_ms)
                .finish(),
            Self::SidecarStdio {
                program,
                args,
                model_path,
                timeout_ms,
            } => f
                .debug_struct("SidecarStdio")
                .field("program", program)
                .field("args", args)
                .field("model_path", model_path)
                .field("timeout_ms", timeout_ms)
                .finish(),
            Self::RemoteOpenAi {
                endpoint_url,
                model,
                api_key,
                timeout_ms,
            } => f
                .debug_struct("RemoteOpenAi")
                .field("endpoint_url", endpoint_url)
                .field("model", model)
                .field("api_key", &api_key.as_ref().map(|_| "[REDACTED]"))
                .field("timeout_ms", timeout_ms)
                .finish(),
        }
    }
}

/// Configuration for [`EmbeddingGemma2Embedder`].
#[derive(Debug, Clone)]
pub struct EmbeddingGemma2Config {
    pub embed_model_ver: String,
    pub dim_d: u32,
    pub max_context_tokens: usize,
    pub backend: EmbeddingGemma2Backend,
}

impl EmbeddingGemma2Config {
    /// Builds a local HTTP sidecar config (`256d`, `8K` ctx).
    #[must_use]
    pub fn local_http(
        endpoint_url: impl Into<String>,
        model_path: Option<PathBuf>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: EMBEDDING_GEMMA_2_DIM,
            max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::SidecarHttp {
                endpoint_url: endpoint_url.into(),
                model_path,
                timeout_ms,
            },
        }
    }

    /// Builds a local stdio sidecar config (`256d`, `8K` ctx).
    #[must_use]
    pub fn local_stdio(
        program: impl Into<String>,
        args: Vec<String>,
        model_path: Option<PathBuf>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: EMBEDDING_GEMMA_2_DIM,
            max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::SidecarStdio {
                program: program.into(),
                args,
                model_path,
                timeout_ms,
            },
        }
    }

    /// Builds a remote OpenAI-compatible config (`256d`, `8K` ctx).
    #[must_use]
    pub fn remote_openai(
        endpoint_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<SecretString>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: EMBEDDING_GEMMA_2_DIM,
            max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::RemoteOpenAi {
                endpoint_url: endpoint_url.into(),
                model: model.into(),
                api_key,
                timeout_ms,
            },
        }
    }
}

/// Configuration for the `EmbeddingGemma-300M` Lab ablation embedder.
#[derive(Debug, Clone)]
pub struct EmbeddingGemma300mConfig {
    pub embed_model_ver: String,
    pub dim_d: u32,
    pub max_context_tokens: usize,
    pub backend: EmbeddingGemma2Backend,
}

impl EmbeddingGemma300mConfig {
    /// Builds a local HTTP sidecar config for `EmbeddingGemma-300M`.
    #[must_use]
    pub fn local_http(
        endpoint_url: impl Into<String>,
        model_path: Option<PathBuf>,
        dim_d: u32,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_300M_VER.to_owned(),
            dim_d: dim_d.max(1),
            max_context_tokens: EMBEDDING_GEMMA_300M_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::SidecarHttp {
                endpoint_url: endpoint_url.into(),
                model_path,
                timeout_ms,
            },
        }
    }

    /// Builds a local stdio sidecar config for `EmbeddingGemma-300M`.
    #[must_use]
    pub fn local_stdio(
        program: impl Into<String>,
        args: Vec<String>,
        model_path: Option<PathBuf>,
        dim_d: u32,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_300M_VER.to_owned(),
            dim_d: dim_d.max(1),
            max_context_tokens: EMBEDDING_GEMMA_300M_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::SidecarStdio {
                program: program.into(),
                args,
                model_path,
                timeout_ms,
            },
        }
    }

    /// Builds a remote OpenAI-compatible config for `EmbeddingGemma-300M`.
    #[must_use]
    pub fn remote_openai(
        endpoint_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<SecretString>,
        dim_d: u32,
        timeout_ms: u64,
    ) -> Self {
        Self {
            embed_model_ver: EMBEDDING_GEMMA_300M_VER.to_owned(),
            dim_d: dim_d.max(1),
            max_context_tokens: EMBEDDING_GEMMA_300M_CTX_TOKENS,
            backend: EmbeddingGemma2Backend::RemoteOpenAi {
                endpoint_url: endpoint_url.into(),
                model: model.into(),
                api_key,
                timeout_ms,
            },
        }
    }
}

#[derive(Serialize)]
struct SidecarEmbedRequest<'a> {
    model_ver: &'a str,
    model_path: &'a str,
    task_prefix: &'static str,
    instruction_prefix: &'static str,
    modality: &'static str,
    dimensions: u32,
    normalize: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SidecarEmbedResponse {
    Direct { embedding: Vec<f32> },
    OpenAi { data: Vec<OpenAiDatum> },
}

#[derive(Deserialize)]
struct OpenAiDatum {
    embedding: Vec<f32>,
}

enum RuntimeHandle {
    SidecarHttp {
        endpoint_url: String,
        model_path: PathBuf,
        timeout_ms: u64,
        client: Client,
    },
    SidecarStdio {
        program: String,
        args: Vec<String>,
        model_path: PathBuf,
        timeout_ms: u64,
    },
    RemoteOpenAi(OpenAiEmbedder),
}

impl fmt::Debug for RuntimeHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SidecarHttp {
                endpoint_url,
                model_path,
                timeout_ms,
                ..
            } => f
                .debug_struct("SidecarHttp")
                .field("endpoint_url", endpoint_url)
                .field("model_path", model_path)
                .field("timeout_ms", timeout_ms)
                .finish(),
            Self::SidecarStdio {
                program,
                args,
                model_path,
                timeout_ms,
            } => f
                .debug_struct("SidecarStdio")
                .field("program", program)
                .field("args", args)
                .field("model_path", model_path)
                .field("timeout_ms", timeout_ms)
                .finish(),
            Self::RemoteOpenAi(inner) => f.debug_tuple("RemoteOpenAi").field(inner).finish(),
        }
    }
}

/// Unified `EmbeddingGemma 2` (`256d` MRL, `8K` ctx) embedder across text, image, and `audio_voice`.
#[derive(Debug)]
pub struct EmbeddingGemma2Embedder {
    embed_model_ver: String,
    dim_d: u32,
    max_context_tokens: usize,
    runtime: RuntimeHandle,
}

impl EmbeddingGemma2Embedder {
    /// Initializes an `EmbeddingGemma 2` embedder, validating `model_path` at startup for local backends.
    pub fn new(config: EmbeddingGemma2Config) -> Result<Self, EmbedError> {
        let runtime = build_runtime(
            "embeddinggemma2",
            &config.embed_model_ver,
            config.dim_d,
            config.max_context_tokens,
            config.backend,
        )?;
        Ok(Self {
            embed_model_ver: config.embed_model_ver,
            dim_d: config.dim_d,
            max_context_tokens: config.max_context_tokens,
            runtime,
        })
    }
}

impl Embedder for EmbeddingGemma2Embedder {
    fn backend_name(&self) -> &str {
        "embeddinggemma2"
    }

    fn model_ver(&self) -> Option<&str> {
        Some(&self.embed_model_ver)
    }

    fn dim_d(&self) -> Option<u32> {
        Some(self.dim_d)
    }

    fn max_context_tokens(&self) -> Option<usize> {
        Some(self.max_context_tokens)
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        &GEMMA2_MODALITIES
    }

    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag> {
        Some(EmbeddingTag {
            embed_model_ver: self.embed_model_ver.clone(),
            dim_d: self.dim_d,
            prompt_regime_id: task.regime_id().to_owned(),
        })
    }

    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        execute_gemma_embed(
            self.backend_name(),
            &self.embed_model_ver,
            self.dim_d,
            self.max_context_tokens,
            &self.runtime,
            input,
            true,
        )
        .map(Some)
    }
}

/// `EmbeddingGemma-300M` ablation baseline embedder (text-only v1 space).
#[derive(Debug)]
pub struct EmbeddingGemma300mEmbedder {
    embed_model_ver: String,
    dim_d: u32,
    max_context_tokens: usize,
    runtime: RuntimeHandle,
}

impl EmbeddingGemma300mEmbedder {
    /// Initializes an `EmbeddingGemma-300M` embedder, validating `model_path` at startup for local backends.
    pub fn new(config: EmbeddingGemma300mConfig) -> Result<Self, EmbedError> {
        let runtime = build_runtime(
            "embeddinggemma-300m",
            &config.embed_model_ver,
            config.dim_d,
            config.max_context_tokens,
            config.backend,
        )?;
        Ok(Self {
            embed_model_ver: config.embed_model_ver,
            dim_d: config.dim_d,
            max_context_tokens: config.max_context_tokens,
            runtime,
        })
    }
}

impl Embedder for EmbeddingGemma300mEmbedder {
    fn backend_name(&self) -> &str {
        "embeddinggemma-300m"
    }

    fn model_ver(&self) -> Option<&str> {
        Some(&self.embed_model_ver)
    }

    fn dim_d(&self) -> Option<u32> {
        Some(self.dim_d)
    }

    fn max_context_tokens(&self) -> Option<usize> {
        Some(self.max_context_tokens)
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        &GEMMA300M_MODALITIES
    }

    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag> {
        Some(EmbeddingTag {
            embed_model_ver: self.embed_model_ver.clone(),
            dim_d: self.dim_d,
            prompt_regime_id: task.regime_id().to_owned(),
        })
    }

    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        execute_gemma_embed(
            self.backend_name(),
            &self.embed_model_ver,
            self.dim_d,
            self.max_context_tokens,
            &self.runtime,
            input,
            false,
        )
        .map(Some)
    }
}

fn require_existing_model_path(
    backend_label: &str,
    model_path: Option<PathBuf>,
) -> Result<PathBuf, EmbedError> {
    let path = model_path.ok_or_else(|| EmbedError::MissingModelPath {
        backend: backend_label.to_owned(),
    })?;
    if path.as_os_str().is_empty() {
        return Err(EmbedError::MissingModelPath {
            backend: backend_label.to_owned(),
        });
    }
    if !Path::new(&path).exists() {
        return Err(EmbedError::ModelPathNotFound {
            backend: backend_label.to_owned(),
            path: path.display().to_string(),
        });
    }
    Ok(path)
}

fn build_runtime(
    backend_label: &str,
    embed_model_ver: &str,
    dim_d: u32,
    max_context_tokens: usize,
    backend: EmbeddingGemma2Backend,
) -> Result<RuntimeHandle, EmbedError> {
    match backend {
        EmbeddingGemma2Backend::SidecarHttp {
            endpoint_url,
            model_path,
            timeout_ms,
        } => {
            let verified_path = require_existing_model_path(backend_label, model_path)?;
            if endpoint_url.trim().is_empty() {
                return Err(EmbedError::Transport {
                    detail: "sidecar endpoint_url must not be empty".to_owned(),
                });
            }
            if timeout_ms == 0 {
                return Err(EmbedError::Timeout {
                    timeout_ms: 0,
                    detail: "sidecar timeout_ms must be > 0".to_owned(),
                });
            }
            let client = Client::builder()
                .timeout(Duration::from_millis(timeout_ms))
                .build()
                .map_err(|err| EmbedError::Transport {
                    detail: format!("failed to build sidecar HTTP client: {err}"),
                })?;
            Ok(RuntimeHandle::SidecarHttp {
                endpoint_url,
                model_path: verified_path,
                timeout_ms,
                client,
            })
        }
        EmbeddingGemma2Backend::SidecarStdio {
            program,
            args,
            model_path,
            timeout_ms,
        } => {
            let verified_path = require_existing_model_path(backend_label, model_path)?;
            if program.trim().is_empty() {
                return Err(EmbedError::Sidecar {
                    detail: "stdio sidecar program must not be empty".to_owned(),
                });
            }
            if timeout_ms == 0 {
                return Err(EmbedError::Timeout {
                    timeout_ms: 0,
                    detail: "sidecar timeout_ms must be > 0".to_owned(),
                });
            }
            Ok(RuntimeHandle::SidecarStdio {
                program,
                args,
                model_path: verified_path,
                timeout_ms,
            })
        }
        EmbeddingGemma2Backend::RemoteOpenAi {
            endpoint_url,
            model,
            api_key,
            timeout_ms,
        } => {
            let mut cfg = OpenAiEmbedConfig::new(endpoint_url, model, dim_d, api_key);
            cfg.embed_model_ver = embed_model_ver.to_owned();
            cfg.timeout_ms = timeout_ms;
            cfg.max_context_tokens = Some(max_context_tokens);
            cfg.apply_task_prefix = true;
            cfg.normalize = true;
            Ok(RuntimeHandle::RemoteOpenAi(OpenAiEmbedder::new(cfg)?))
        }
    }
}

fn execute_gemma_embed(
    backend_label: &str,
    embed_model_ver: &str,
    dim_d: u32,
    max_context_tokens: usize,
    runtime: &RuntimeHandle,
    input: &EmbedInput<'_>,
    allow_multimodal: bool,
) -> Result<TaggedEmbedding, EmbedError> {
    if !allow_multimodal && input.content.modality() != EmbedModality::Text {
        return Err(EmbedError::UnsupportedModality {
            backend: backend_label.to_owned(),
            modality: input.content.modality(),
        });
    }

    if let RuntimeHandle::RemoteOpenAi(remote) = runtime {
        return remote
            .embed(input)?
            .ok_or_else(|| EmbedError::InvalidResponse {
                detail: "remote embedder returned empty vector".to_owned(),
            });
    }

    let (modality_str, text_opt, media_b64_opt, mime_opt) = match input.content {
        EmbedContent::Text(text) => {
            let formatted = input.task.format_text(text);
            let est = estimate_tokens(&formatted);
            if est > max_context_tokens {
                return Err(EmbedError::ContextLimitExceeded {
                    backend: backend_label.to_owned(),
                    estimated_tokens: est,
                    max_tokens: max_context_tokens,
                });
            }
            ("text", Some(formatted), None, None)
        }
        EmbedContent::Image {
            bytes,
            mime_type,
            caption,
        } => {
            let cap = caption.map(|c| input.task.format_text(c));
            if let Some(ref c) = cap {
                let est = estimate_tokens(c);
                if est > max_context_tokens {
                    return Err(EmbedError::ContextLimitExceeded {
                        backend: backend_label.to_owned(),
                        estimated_tokens: est,
                        max_tokens: max_context_tokens,
                    });
                }
            }
            ("image", cap, Some(STANDARD.encode(bytes)), Some(mime_type))
        }
        EmbedContent::AudioVoice {
            bytes,
            mime_type,
            transcript,
        } => {
            let tr = transcript.map(|t| input.task.format_text(t));
            if let Some(ref t) = tr {
                let est = estimate_tokens(t);
                if est > max_context_tokens {
                    return Err(EmbedError::ContextLimitExceeded {
                        backend: backend_label.to_owned(),
                        estimated_tokens: est,
                        max_tokens: max_context_tokens,
                    });
                }
            }
            (
                "audio_voice",
                tr,
                Some(STANDARD.encode(bytes)),
                Some(mime_type),
            )
        }
    };

    let mut values = match runtime {
        RuntimeHandle::SidecarHttp {
            endpoint_url,
            model_path,
            timeout_ms,
            client,
        } => {
            let model_path_str = model_path.to_string_lossy();
            let req_body = SidecarEmbedRequest {
                model_ver: embed_model_ver,
                model_path: &model_path_str,
                task_prefix: input.task.regime_id(),
                instruction_prefix: input.task.instruction_prefix(),
                modality: modality_str,
                dimensions: dim_d,
                normalize: true,
                text: text_opt,
                media_base64: media_b64_opt,
                mime_type: mime_opt,
            };
            let resp = client
                .post(endpoint_url)
                .json(&req_body)
                .send()
                .map_err(|err| {
                    let detail = redact_secrets(&err.to_string(), &[]);
                    if err.is_timeout() {
                        EmbedError::Timeout {
                            timeout_ms: *timeout_ms,
                            detail,
                        }
                    } else {
                        EmbedError::Sidecar { detail }
                    }
                })?;
            let status = resp.status();
            let body = resp.text().map_err(|err| {
                let detail = redact_secrets(&err.to_string(), &[]);
                if err.is_timeout() {
                    EmbedError::Timeout {
                        timeout_ms: *timeout_ms,
                        detail,
                    }
                } else {
                    EmbedError::Sidecar { detail }
                }
            })?;
            if !status.is_success() {
                return Err(EmbedError::Server {
                    status: Some(status.as_u16()),
                    detail: redact_secrets(&body, &[]),
                });
            }
            decode_sidecar_response(&body)?
        }
        RuntimeHandle::SidecarStdio {
            program,
            args,
            model_path,
            timeout_ms: _,
        } => {
            let model_path_str = model_path.to_string_lossy();
            let req_body = SidecarEmbedRequest {
                model_ver: embed_model_ver,
                model_path: &model_path_str,
                task_prefix: input.task.regime_id(),
                instruction_prefix: input.task.instruction_prefix(),
                modality: modality_str,
                dimensions: dim_d,
                normalize: true,
                text: text_opt,
                media_base64: media_b64_opt,
                mime_type: mime_opt,
            };
            let mut payload =
                serde_json::to_string(&req_body).map_err(|err| EmbedError::Sidecar {
                    detail: format!("failed to serialize sidecar request: {err}"),
                })?;
            payload.push('\n');

            let mut child = Command::new(program)
                .args(args)
                .arg("--model")
                .arg(model_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|err| EmbedError::Sidecar {
                    detail: format!("failed to spawn stdio sidecar '{program}': {err}"),
                })?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(payload.as_bytes())
                    .map_err(|err| EmbedError::Sidecar {
                        detail: format!("failed to write to stdio sidecar stdin: {err}"),
                    })?;
            }

            let mut stdout_buf = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                stdout
                    .read_to_string(&mut stdout_buf)
                    .map_err(|err| EmbedError::Sidecar {
                        detail: format!("failed to read stdio sidecar stdout: {err}"),
                    })?;
            }
            let status = child.wait().map_err(|err| EmbedError::Sidecar {
                detail: format!("failed waiting on stdio sidecar: {err}"),
            })?;
            if !status.success() {
                let mut err_buf = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut err_buf);
                }
                return Err(EmbedError::Sidecar {
                    detail: format!(
                        "stdio sidecar exited with status {status}: {}",
                        redact_secrets(err_buf.trim(), &[])
                    ),
                });
            }
            decode_sidecar_response(stdout_buf.trim())?
        }
        RuntimeHandle::RemoteOpenAi(_) => unreachable!(),
    };

    if values.len() != dim_d as usize {
        return Err(EmbedError::DimensionMismatch {
            expected: dim_d,
            actual: values.len(),
        });
    }
    l2_normalize(&mut values)?;

    Ok(TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver: embed_model_ver.to_owned(),
            dim_d,
            prompt_regime_id: input.task.regime_id().to_owned(),
        },
        values,
    })
}

fn decode_sidecar_response(raw: &str) -> Result<Vec<f32>, EmbedError> {
    let parsed: SidecarEmbedResponse =
        serde_json::from_str(raw).map_err(|err| EmbedError::InvalidResponse {
            detail: redact_secrets(&format!("invalid sidecar JSON response: {err}"), &[]),
        })?;
    match parsed {
        SidecarEmbedResponse::Direct { embedding } => Ok(embedding),
        SidecarEmbedResponse::OpenAi { data } => {
            data.into_iter().next().map(|d| d.embedding).ok_or_else(|| {
                EmbedError::InvalidResponse {
                    detail: "sidecar OpenAI-shaped response missing data[0]".to_owned(),
                }
            })
        }
    }
}
