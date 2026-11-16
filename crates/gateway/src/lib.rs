//! `monoagens-gateway`: single-owner async Tokio runtime bridging Waist-2 bodies,
//! versioned configuration, isolated secrets, stateless model providers, canonical
//! `κ(b, a)` identity linking, scope-keyed `BLAKE3` attachment custody, pre-queue
//! `EmbeddingGemma 2` + `Tool RAG` indexing, and honest `R-47` lifecycle cues into
//! synchronous [`monoagens_core::Core`].

#![forbid(unsafe_code)]

pub mod attachments;
pub mod clock;
pub mod config;
pub mod cues;
pub mod identity;
pub mod logging;
pub mod prequeue;
pub mod provider;
pub mod runtime;
pub mod seams;
pub mod secrets;
pub mod waist2;

pub use attachments::{
    AttachmentCustodyError, AttachmentMasterKey, AttachmentStore, DEFAULT_CUSTODY_TTL_MS,
    GatewayAttachmentBridge, HardForgetAttachmentReport, STAGING_TTL_MS,
    ScopedAttachmentBlobReader, sha256_hex,
};
pub use clock::{Clock, ClockTimeout, SystemClock, VirtualClock, run_with_clock_timeout};
pub use config::{
    BodyPluginConfig, BodyTransportConfig, CONFIG_FILENAME, CURRENT_CONFIG_VERSION, ConfigError,
    DEFAULT_OPENAI_ENDPOINT, EmbedBackendKind, EmbedConfig, GatewayConfig, IdentityConfig,
    LoadedGatewayEnv, ModelConfig, ModelProviderKind, PersonAccountConfig, PersonMappingConfig,
    ResolvedPaths, RuntimeConfig, SECRETS_FILENAME, SoulConfig, ValveSectionConfig,
    default_config_document, extract_cli_config_arg, load_config_file, load_or_init_gateway_env,
    parse_config_str, resolve_paths, resolve_paths_with, update_config_with_backup,
    write_default_config,
};
pub use cues::{
    CueRecorder, DEFAULT_PROGRESS_NOTE_MS, DEFAULT_WORKING_CUE_MS, LifecycleCueEvent,
    LifecycleCueKind, WaitCueGuard, spawn_wait_cue_monitor,
};
pub use identity::{
    ACTOR_ID_DOMAIN_PREFIX, ActiveActorLink, DEFAULT_LINK_CHALLENGE_TTL_S,
    DEFAULT_LINK_MAX_ATTEMPTS, HEURISTIC_MERGE_ENABLED, HEURISTIC_MERGE_THRESHOLD_MU,
    IdentityError, IdentityLinkEvent, IdentityResolver, IssuedLinkChallenge,
    LINK_NONCE_DOMAIN_PREFIX, OWNER_ESCALATION_GUARD, canonical_kappa, derive_per_actor_id,
};
pub use logging::{DiagnosticRecord, DiagnosticSink, LogLevel};
pub use prequeue::{
    EmbedInitError, PreQueueEnrichReport, PreQueueService, ToolRagHit, ToolRagIndex,
    ToolRagQueryResult, build_embedder_from_config, build_embedder_from_config_async,
};
pub use provider::{
    DailyBudgetTracker, GatewayModelExecutor, ProviderInitError, build_provider_from_config,
    build_provider_from_config_async,
};
pub use runtime::{
    GatewayError, GatewayHandle, GatewayOptions, GatewayRuntime, IngressQueue, OwnershipStats,
    TurnOutcome,
};
pub use seams::{
    AsyncGatewayHook, AttachmentCustodySeam, DoorController, DoorsAdapter, GatewaySeams,
    IdentitySeam, PreQueueIndexSeam, SeamError,
};
pub use secrets::{
    AttachmentSecrets, ENV_ATTACHMENT_MASTER_KEY, GatewaySecrets, SecretsLoadError,
    is_forbidden_secret_key, load_attachment_secrets, load_gateway_secrets, load_provider_secrets,
    load_secrets_toml_file, parse_secrets_toml_str, validate_no_inline_secrets_in_toml,
    validate_non_secret_file,
};
pub use waist2::{
    ActionSettlementTracker, BodyConnection, BodyRouter, BodyRpcError, SettlementOutcome,
};
