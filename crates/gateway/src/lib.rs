//! `monoagens-gateway`: single-owner async Tokio runtime bridging Waist-2 bodies,
//! versioned configuration, isolated secrets, stateless model providers, canonical
//! `κ(b, a)` identity linking, scope-keyed `BLAKE3` attachment custody, pre-queue
//! `EmbeddingGemma 2` + `Tool RAG` indexing, and honest `R-47` lifecycle cues into
//! synchronous [`monoagens_core::Core`].

#![forbid(unsafe_code)]

pub mod admin;
pub mod attachments;
pub mod clock;
pub mod commands;
pub mod config;
pub mod config_keys;
pub mod cues;
pub mod governance;
pub mod identity;
pub mod lock;
pub mod logging;
pub mod owner;
pub mod plugins;
pub mod preview;
pub mod prequeue;
pub mod provider;
pub mod runlog;
pub mod runtime;
pub mod seams;
pub mod secrets;
pub mod supervisor;
pub mod waist2;

pub use admin::{ADMIN_SOCKET_FILENAME, AdminClient, AdminServer, admin_socket_path, admin_socket_path_in};
pub use attachments::{
    AttachmentCustodyError, AttachmentMasterKey, AttachmentStore, DEFAULT_CUSTODY_TTL_MS,
    GatewayAttachmentBridge, HardForgetAttachmentReport, STAGING_TTL_MS,
    ScopedAttachmentBlobReader, sha256_hex,
};
pub use clock::{Clock, ClockTimeout, SystemClock, VirtualClock, run_with_clock_timeout};
pub use commands::{CommandOutcome, CommandRegistry, CommandSpec};
pub use config::{
    BodyPluginConfig, BodyTransportConfig, CONFIG_FILENAME, CURRENT_CONFIG_VERSION, ConfigError,
    DEFAULT_OPENAI_ENDPOINT, EmbedBackendKind, EmbedConfig, GatewayConfig, IdentityConfig,
    LoadedGatewayEnv, ModelConfig, ModelPrice, ModelProviderKind, PurposeModelConfig, PersonAccountConfig, PersonMappingConfig,
    PluginConfig, PluginKind, ResolvedPaths, RuntimeConfig, SECRETS_FILENAME, SoulConfig,
    ValveSectionConfig, default_config_document, extract_cli_config_arg, load_config_file,
    load_or_init_gateway_env, parse_config_str, resolve_paths, resolve_paths_with,
    update_config_with_backup, write_default_config,
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
pub use lock::{
    DataDirLock, FileLock, GATEWAY_LOCK_FILENAME, LockError, LockHeld, acquire_data_dir_lock,
    acquire_file_lock, acquire_plugin_lock, read_data_dir_lock_pid,
};
pub use logging::{DiagnosticRecord, DiagnosticSink, LogLevel};
pub use owner::{
    AdminError, GatewayOwner, OwnerMode, OwnerParams, ReloadReport, apply_config_patch_to_file, write_provider_key,
};
pub use plugins::{
    ENV_BODY_STATE_DIR, ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR, ENV_RBAC_ENABLED,
    PluginBootPlan, PluginEnvContext, PluginStartupReport, SkippedPlugin, build_plugin_env,
    resolve_plugin_boot_order,
};
pub use preview::{DraftExtractor, PreviewEnd, PreviewEvent, PreviewHub, PreviewSubscription, draft_text_of};
pub use prequeue::{
    EmbedInitError, PreQueueEnrichReport, PreQueueService, ToolRagHit, ToolRagIndex,
    ToolRagQueryResult, build_embedder_from_config, build_embedder_from_config_async,
};
pub use provider::{
    CANCELLED_BY_OWNER, DailyBudgetTracker, GatewayModelExecutor, LiveModel, ModelRoutes,
    ProviderInitError, TurnRegistry, UsageCounts, UsageRow, build_model_routes, build_model_routes_async,
    build_provider_from_config, build_provider_from_config_async, purpose_from_name, purpose_name,
};
pub use runlog::{
    LogBuffer, LogEntry, LogLevel as RunLogLevel, RunLog, RotatingLogFile,
};
pub use runtime::{
    BodyConnectOutcome, GatewayError, GatewayHandle, GatewayOptions, GatewayRuntime,
    IngressQueue, OwnershipStats, TurnOutcome,
};
pub use seams::{
    AsyncGatewayHook, AttachmentCustodySeam, DoorController, DoorsAdapter, GatewaySeams,
    IdentitySeam, PreQueueIndexSeam, SeamError, ValveDoorController,
};
pub use secrets::{
    ApprovalSecrets, AttachmentSecrets, ENV_APPROVAL_ATTESTATION_KEY, ENV_ATTACHMENT_MASTER_KEY,
    GatewaySecrets, SecretsLoadError, is_forbidden_secret_key, load_approval_secrets,
    load_attachment_secrets, load_gateway_secrets, load_plugin_env_secrets, load_provider_secrets,
    load_secrets_toml_file, parse_plugin_env_secrets_str, parse_secrets_toml_str,
    resolve_approval_mac_key, validate_no_inline_secrets_in_toml, validate_non_secret_file,
};
pub use supervisor::{
    FirstOutcome, PluginInfo, PluginState, PluginStateEvent, PluginSupervisor, RestartPolicy,
    SupervisorError,
};
pub use waist2::{
    ActionSettlementTracker, ApprovalPromptRequest, BodyConnection, BodyRouter, BodyRpcError,
    CAPABILITY_APPROVAL_PROMPT, SettlementOutcome,
};
