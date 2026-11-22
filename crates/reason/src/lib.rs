//! Model providers, `ContextPlan` + `[SELF_GUIDE]` prompt rendering, strict output parsing,
//! embedders, secrets isolation, `A-14` dialogue-act gap classification, blind attachment
//! resolution, and tool selection for MonoAgens (`monoagens-reason`).

#![forbid(unsafe_code)]

pub mod attachments;
pub mod embed;
pub mod info_gap_clf;
pub mod model;
pub mod parse;
pub mod render;
pub mod secrets;
pub mod tools;

pub use attachments::{
    AttachmentAccessContext, AttachmentBlobStore, AttachmentError, InMemoryAttachmentStore,
    ResolvedAttachment, default_mime_for_modality, resolve_authorized_attachment,
};
pub use embed::{
    EMBEDDING_GEMMA_2_CTX_TOKENS, EMBEDDING_GEMMA_2_DIM, EMBEDDING_GEMMA_2_VER,
    EMBEDDING_GEMMA_300M_CTX_TOKENS, EMBEDDING_GEMMA_300M_DEFAULT_DIM, EMBEDDING_GEMMA_300M_VER,
    ENV_ADAPTER_PYTHON, EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder,
    EmbeddingGemma2Backend, EmbeddingGemma2Config, EmbeddingGemma2Embedder,
    EmbeddingGemma300mConfig, EmbeddingGemma300mEmbedder, HashEmbedder, LocalEmbedSidecar,
    NoneEmbedder, OpenAiEmbedConfig, OpenAiEmbedder, PartitionedSearchResult, ScriptedEmbedder,
    TaskPrefix, check_partition_match, is_l2_normalized, l2_normalize, partitioned_cosine,
    resolve_adapter_python, same_partition, search_asymmetric_q_to_d, search_partitioned,
};
pub use info_gap_clf::{InfoGapClassifier, InfoGapError, InfoGapEvaluation};
pub use model::{
    CallOutcome, DEFAULT_OUTPUT_LIMIT_PARAM, ExtraHeaderValue, MessagePart, MessageRole,
    ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelMessage, ModelOutput, ModelProvider,
    ModelRequest, OpenAiModelConfig, OpenAiModelProvider, OutputFormatFailureRecord,
    OutputLimitProfile, PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS, ProviderFailureClass, ReasoningEffort,
    ResponseSchemaMode, ScriptedModelProvider, ScriptedReply, TokenUsage, UsageFieldPaths,
    validate_extra_header_name,
};
pub use parse::{
    DecodedRubric, FailedWake, MotivationLevel, ParseOptions, ParsedModelActs, decode_rubric,
    parse_classification_verdict, parse_thought_output, parse_thought_output_with_options,
    strict_classification_verdict_schema, strict_thought_batch_schema,
};
pub use render::{
    RenderExtras, RenderedContext, SELF_GUIDE_MAX_TOKENS, apply_selected_tools,
    build_model_request_from_plan, escape_spotlight_payload, estimate_prompt_tokens,
    format_absolute_utc, render_context_plan, render_context_plan_with, render_segment,
    render_self_guide, self_guide_section_line, spotlight_delimiters,
};
pub use secrets::{
    ConfigSourceKind, ENV_EMBED_API_KEY, ENV_EMBED_ENDPOINT, ENV_MODEL_API_KEY, ENV_MODEL_ENDPOINT,
    ENV_OPENAI_API_KEY, ENV_OPENAI_BASE_URL, ProviderSecrets, SecretError, SecretString,
    redact_secrets, validate_no_secrets_in_json, validate_no_secrets_in_str,
};
pub use tools::{
    CapabilityCatalogItem, DEFAULT_K_CAP, DEFAULT_K_PROC, MAX_STATIC_CORE_CAPS,
    ProcedureCatalogItem, SelectedTools, ToolSelectionBudget, select_tools,
};
