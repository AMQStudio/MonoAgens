use std::sync::Arc;

use monoagens_core::{CoreConfig, EventDetail, MemStore, NoopDoors};
use monoagens_gateway::{
    ApprovalPromptRequest, BodyTransportConfig, CAPABILITY_APPROVAL_PROMPT, Clock, CommandOutcome,
    CommandRegistry, CommandSpec, CueRecorder, DiagnosticSink, DoorsAdapter, GatewayConfig,
    GatewayOptions, GatewayRuntime, GatewaySeams, IdentityResolver, IdentitySeam, LogLevel,
    ModelProviderKind, PluginConfig, PluginEnvContext, PluginKind, SecretsLoadError,
    ValveDoorController, VirtualClock, build_plugin_env, load_plugin_env_secrets,
    load_provider_secrets, parse_config_str, resolve_plugin_boot_order,
};
use monoagens_protocol::{
    ActionExecuteParams, ActionExecuteResult, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult, BodyId,
    CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, OpaquePayload, Percept, PerceptEmitParams, PlaceId, Receipt,
    ReceiptOutcome, RpcId, Taint, TerminationReason, Time, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response,
};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider, ScriptedReply, SecretString};
use monoagens_valve::{
    ApprovalMode, RoleTier, Valve, ValveConfig, ValveSnapshot, derive_approval_mac_key,
    sign_approval_resolution,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::sync::mpsc;

fn test_gateway_config() -> GatewayConfig {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock-gpt".to_owned();
    cfg.runtime.action_timeout_ms = 500;
    cfg.runtime.hook_timeout_ms = 500;
    cfg.runtime.model_timeout_ms = 5_000;
    cfg
}

async fn start_test_gateway() -> (GatewayRuntime, Arc<VirtualClock>, DiagnosticSink) {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let diagnostics = DiagnosticSink::new(&secrets);
    let rt = GatewayRuntime::start(
        test_gateway_config(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        DoorsAdapter(NoopDoors),
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    (rt, clock, diagnostics)
}

#[test]
fn parses_unified_plugins_roles_places_person_roles_and_legacy_bodies() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let sec_path = tmp.path().join("secrets.toml");

    let toml = r#"
version = 1

[plugins.telegram]
enabled = true
kind = "adapter"
command = "monoagens-body-telegram"
args = ["--stdio"]
optional_dependencies = ["rbac"]

[plugins.memory_rag]
kind = "capability"
socket_path = "/tmp/memory-rag.sock"
dependencies = ["telegram"]

[plugins.disabled_mod]
enabled = false
command = "disabled-bin"

[[bodies]]
id = "telegram"
transport = "stdio"
command = "legacy-tg-ignored"

[[bodies]]
id = "legacy-cli"
transport = "stdio"
command = "monoagens-body-cli"

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
allow = ["command.help", "identity.whoami"]
deny = ["wallet.*"]

[roles.trusted]
base = "trusted"
approval = "strict"
label = "Trusted Member"
context = "verified friend"
[roles.trusted.settings]
daily_quota = 50
can_search = true

[places."tg:group:100"]
allow = ["tg.send", "command.help"]
deny = ["wallet.pay"]

[[person]]
actor_id = "owner"
owner = true
roles = ["trusted"]
body_id = "telegram"
account_id = "tg:user:123"

[[person]]
actor_id = "alice"
roles = ["guest"]
accounts = [{ body_id = "telegram", account_id = "tg:user:456" }]
"#;

    let cfg = parse_config_str(toml, &cfg_path, &sec_path).expect("parse unified config");
    assert_eq!(cfg.plugins.len(), 3);
    assert_eq!(cfg.bodies.len(), 2);

    let tg = cfg.plugins.iter().find(|p| p.id == "telegram").expect("tg");
    assert!(tg.enabled);
    assert_eq!(tg.kind, PluginKind::Adapter);
    assert_eq!(
        tg.transport,
        BodyTransportConfig::Stdio {
            command: "monoagens-body-telegram".to_owned(),
            args: vec!["--stdio".to_owned()],
        }
    );
    assert_eq!(tg.optional_dependencies, vec!["rbac"]);

    let rag = cfg.plugins.iter().find(|p| p.id == "memory_rag").expect("rag");
    assert_eq!(rag.kind, PluginKind::Capability);
    assert_eq!(
        rag.transport,
        BodyTransportConfig::UnixSocket {
            path: std::path::PathBuf::from("/tmp/memory-rag.sock"),
        }
    );
    assert_eq!(rag.dependencies, vec!["telegram"]);

    // `effective_plugins()` merges `[plugins.*]` and `[[bodies]]`, deduping `telegram` cleanly
    // in favor of `[plugins.telegram]`.
    let eff = cfg.effective_plugins();
    assert_eq!(eff.len(), 4);
    let eff_tg = eff.iter().find(|p| p.id == "telegram").expect("eff tg");
    assert_eq!(
        eff_tg.transport,
        BodyTransportConfig::Stdio {
            command: "monoagens-body-telegram".to_owned(),
            args: vec!["--stdio".to_owned()],
        }
    );
    let eff_legacy = eff
        .iter()
        .find(|p| p.id == "legacy-cli")
        .expect("eff legacy-cli");
    assert_eq!(eff_legacy.kind, PluginKind::Adapter);
    assert!(eff_legacy.enabled);

    let rbac = cfg.rbac.as_ref().expect("rbac populated");
    assert_eq!(rbac.fallback_role.as_deref(), Some("guest"));
    let trusted_role = rbac.roles.get("trusted").expect("trusted role");
    assert_eq!(trusted_role.base, RoleTier::Trusted);
    assert_eq!(trusted_role.approval, Some(ApprovalMode::Strict));
    assert_eq!(trusted_role.label.as_deref(), Some("Trusted Member"));
    assert_eq!(trusted_role.context.as_deref(), Some("verified friend"));
    assert_eq!(
        trusted_role.settings.get("daily_quota"),
        Some(&json!(50))
    );
    assert_eq!(
        trusted_role.settings.get("can_search"),
        Some(&json!(true))
    );

    let place_rule = rbac
        .places
        .get(&PlaceId::new("tg:group:100"))
        .expect("place rule");
    assert_eq!(place_rule.allow, vec!["tg.send", "command.help"]);
    assert_eq!(place_rule.deny, vec!["wallet.pay"]);

    assert_eq!(
        rbac.actor_roles.get(&ActorId::new("owner")),
        Some(&vec!["trusted".to_owned()])
    );
    assert_eq!(
        rbac.actor_roles.get(&ActorId::new("alice")),
        Some(&vec!["guest".to_owned()])
    );

    // Omitting [roles], [places], and [[person]].roles keeps rbac = None for 100% backward compat.
    let plain_toml = "version = 1\n[[person]]\nactor_id = \"owner\"\nowner = true\n";
    let plain_cfg = parse_config_str(plain_toml, &cfg_path, &sec_path).expect("plain cfg");
    assert!(plain_cfg.rbac.is_none());
}

fn dummy_plugin(
    id: &str,
    enabled: bool,
    deps: &[&str],
    opt_deps: &[&str],
) -> PluginConfig {
    PluginConfig {
        id: id.to_owned(),
        enabled,
        kind: PluginKind::Capability,
        transport: BodyTransportConfig::Stdio {
            command: format!("bin-{id}"),
            args: Vec::new(),
        },
        dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
        optional_dependencies: opt_deps.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn dependency_resolver_orders_topologically_and_isolates_missing_or_cyclic_plugins() {
    let plugins = vec![
        // Depends on `base` (satisfied) and optional `rbac` (built-in)
        dummy_plugin("app", true, &["base"], &["rbac"]),
        // Base plugin with no deps
        dummy_plugin("base", true, &[], &[]),
        // Disabled plugin
        dummy_plugin("off", false, &[], &[]),
        // Missing required dep `off` (disabled) -> skipped!
        dummy_plugin("needs-off", true, &["off"], &[]),
        // Transitive dependent on `needs-off` -> also skipped!
        dummy_plugin("needs-needs-off", true, &["needs-off"], &[]),
        // Missing unknown dep -> skipped!
        dummy_plugin("broken", true, &["nonexistent"], &[]),
        // Cycle: cyc_a <-> cyc_b -> both skipped, sibling `independent` unaffected!
        dummy_plugin("cyc_a", true, &["cyc_b"], &[]),
        dummy_plugin("cyc_b", true, &["cyc_a"], &[]),
        // Independent plugin with optional dependency on cyclic plugin -> STILL boots!
        dummy_plugin("independent", true, &[], &["cyc_a"]),
    ];

    let plan = resolve_plugin_boot_order(&plugins, &["rbac"]);
    let ordered_ids: Vec<&str> = plan.ordered.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ordered_ids, vec!["base", "app", "independent"]);

    let skipped_ids: Vec<&str> = plan.skipped.iter().map(|s| s.id.as_str()).collect();
    assert!(skipped_ids.contains(&"needs-off"));
    assert!(skipped_ids.contains(&"needs-needs-off"));
    assert!(skipped_ids.contains(&"broken"));
    assert!(skipped_ids.contains(&"cyc_a"));
    assert!(skipped_ids.contains(&"cyc_b"));
    assert!(!skipped_ids.contains(&"off"), "disabled plugin filtered cleanly");
    assert!(!skipped_ids.contains(&"independent"));

    // Requiring `"rbac"` fails if `"rbac"` is not in `builtin_features`, succeeds if present.
    let rbac_plugins = vec![dummy_plugin("rbac_consumer", true, &["rbac"], &[])];
    let without_rbac = resolve_plugin_boot_order(&rbac_plugins, &[]);
    assert!(without_rbac.ordered.is_empty());
    assert_eq!(without_rbac.skipped.len(), 1);
    assert_eq!(without_rbac.skipped[0].id, "rbac_consumer");

    let with_rbac = resolve_plugin_boot_order(&rbac_plugins, &["rbac"]);
    assert_eq!(with_rbac.ordered.len(), 1);
    assert!(with_rbac.skipped.is_empty());
}

#[tokio::test]
async fn connect_configured_plugins_injects_plugin_config_state_and_rbac_env_vars() {
    let (rt, _clock, _diag) = start_test_gateway().await;
    let handle = rt.handle();
    let tmp = TempDir::new().expect("tempdir");
    let cfg_dir = tmp.path().join("cfg");
    let data_dir = tmp.path().join("data");

    // POSIX shell micro-plugin that echoes injected env vars in body/discover capability description.
    let script = r#"IFS= read -r _line
printf '{"jsonrpc":"2.0","id":1,"result":{"body_id":"env-echo-body","protocol_version":{"major":1,"minor":0},"capabilities":[{"id":"env.echo","description":"cfg=%s|state=%s|bstate=%s|rbac=%s","places":["env:place"],"flags":{"communicative":false,"identity_changing":false,"irreversible":false},"typical_latency_ms":5,"capability_schema_hash":"h1"}],"places":["env:place"]}}\n' "$MONOAGENS_PLUGIN_CONFIG" "$MONOAGENS_PLUGIN_STATE_DIR" "$MONOAGENS_BODY_STATE_DIR" "$MONOAGENS_RBAC_ENABLED"
sleep 2"#;

    let plugins = vec![
        PluginConfig {
            id: "broken-missing-dep".to_owned(),
            enabled: true,
            kind: PluginKind::Capability,
            transport: BodyTransportConfig::Stdio {
                command: "/nonexistent/bin".to_owned(),
                args: Vec::new(),
            },
            dependencies: vec!["missing-dep".to_owned()],
            optional_dependencies: Vec::new(),
        },
        PluginConfig {
            id: "echo-plugin".to_owned(),
            enabled: true,
            kind: PluginKind::Adapter,
            transport: BodyTransportConfig::Stdio {
                command: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), script.to_owned()],
            },
            dependencies: Vec::new(),
            optional_dependencies: vec!["rbac".to_owned()],
        },
    ];

    let env_ctx = PluginEnvContext {
        config_dir: Some(cfg_dir.clone()),
        data_dir: Some(data_dir.clone()),
        rbac_enabled: true,
        ..Default::default()
    };

    let report = handle.connect_configured_plugins(&plugins, &env_ctx).await;
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].id, "broken-missing-dep");
    assert!(report.failed.is_empty());
    assert_eq!(report.connected.len(), 1);
    assert_eq!(report.connected[0].config_id, "echo-plugin");
    assert_eq!(
        report.connected[0].result,
        Ok(BodyId::new("env-echo-body"))
    );

    let caps = handle.router().all_capabilities();
    let echo_cap = caps
        .iter()
        .find(|c| c.id == CapabilityId::new("env.echo"))
        .expect("env.echo capability registered");

    let expected_cfg = cfg_dir.join("plugins").join("echo-plugin.toml");
    let expected_state = data_dir.join("plugins").join("echo-plugin");
    let expected_desc = format!(
        "cfg={}|state={}|bstate={}|rbac=1",
        expected_cfg.display(),
        expected_state.display(),
        expected_state.display(),
    );
    assert_eq!(echo_cap.description, expected_desc);

    rt.shutdown().await;
}

#[test]
fn command_registry_help_and_execution_filter_by_rbac_without_llm() {
    let mut reg = CommandRegistry::new();
    let builtin_names: Vec<&str> = reg.all_commands().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(builtin_names, vec!["help", "roles", "whoami"]);
    assert!(reg.get("approve").is_none());
    assert!(reg.get("deny").is_none());
    assert!(reg.get("/approve").is_none());
    assert!(reg.get("/deny").is_none());

    let tg_body = BodyId::new("body-telegram");
    reg.register_plugin_commands(
        &tg_body,
        vec![
            CommandSpec {
                name: "status".to_owned(),
                summary: "Check adapter status".to_owned(),
                usage: Some("/status".to_owned()),
                permission: "tg.status".to_owned(),
                base_tier: Some(RoleTier::Guest),
                owner_plugin: None,
            },
            CommandSpec {
                name: "admin_reset".to_owned(),
                summary: "Reset adapter cache".to_owned(),
                usage: Some("/admin_reset".to_owned()),
                permission: "tg.admin_reset".to_owned(),
                base_tier: Some(RoleTier::Admin),
                owner_plugin: None,
            },
        ],
    );

    let tmp = TempDir::new().expect("tempdir");
    let cfg = parse_config_str(
        r#"
version = 1

[roles]
fallback = "guest"

[roles.guest]
base = "guest"

[roles.admin]
base = "admin"

[[person]]
actor_id = "owner"
owner = true

[[person]]
actor_id = "alice"
roles = ["guest"]
"#,
        &tmp.path().join("monoagens.toml"),
        &tmp.path().join("secrets.toml"),
    )
    .expect("parse config");

    let rbac = cfg.rbac.as_ref().expect("rbac");
    let owner = ActorId::new("owner");
    let guest = ActorId::new("alice");
    let caps = vec![
        CapabilityId::new("tg.send"),
        CapabilityId::new("wallet.pay"),
    ];

    let guest_help = match reg.try_handle_slash_command("/help", &guest, &owner, None, Some(rbac), &caps) {
        Some(CommandOutcome::Handled { reply_text }) => reply_text,
        other => panic!("expected Handled for guest /help, got {other:?}"),
    };
    assert!(guest_help.contains("/help"));
    assert!(guest_help.contains("/whoami"));
    assert!(guest_help.contains("/status"));
    assert!(!guest_help.contains("/roles"));
    assert!(!guest_help.contains("/approve"));
    assert!(!guest_help.contains("/deny"));
    assert!(!guest_help.contains("/admin_reset"));

    let owner_help = match reg.try_handle_slash_command("/help", &owner, &owner, None, Some(rbac), &caps) {
        Some(CommandOutcome::Handled { reply_text }) => reply_text,
        other => panic!("expected Handled for owner /help, got {other:?}"),
    };
    assert!(owner_help.contains("/help"));
    assert!(owner_help.contains("/whoami"));
    assert!(owner_help.contains("/roles"));
    assert!(!owner_help.contains("/approve"));
    assert!(!owner_help.contains("/deny"));
    assert!(owner_help.contains("/status"));
    assert!(owner_help.contains("/admin_reset"));

    // Guest and owner can both run /whoami.
    assert!(matches!(
        reg.try_handle_slash_command("/whoami", &guest, &owner, None, Some(rbac), &caps),
        Some(CommandOutcome::Handled { .. })
    ));
    assert!(matches!(
        reg.try_handle_slash_command("/whoami", &owner, &owner, None, Some(rbac), &caps),
        Some(CommandOutcome::Handled { .. })
    ));

    // Unauthorized command for guest is deterministically denied with zero LLM calls, while owner succeeds.
    let denied_roles = reg.try_handle_slash_command("/roles", &guest, &owner, None, Some(rbac), &caps);
    assert!(matches!(denied_roles, Some(CommandOutcome::Denied { .. })));
    let owner_roles = reg.try_handle_slash_command("/roles", &owner, &owner, None, Some(rbac), &caps);
    assert!(matches!(owner_roles, Some(CommandOutcome::Handled { .. })));

    // `/approve` and `/deny` are no longer built-in commands.
    assert_eq!(
        reg.try_handle_slash_command("/approve act-1", &guest, &owner, None, Some(rbac), &caps),
        None
    );
    assert_eq!(
        reg.try_handle_slash_command("/approve act-1", &owner, &owner, None, Some(rbac), &caps),
        None
    );
    assert_eq!(
        reg.try_handle_slash_command("/deny act-1", &owner, &owner, None, Some(rbac), &caps),
        None
    );

    // Permitted plugin command returns ForwardToPlugin; admin-only plugin command is denied for guest.
    let fwd = reg.try_handle_slash_command("/status verbose", &guest, &owner, None, Some(rbac), &caps);
    assert_eq!(
        fwd,
        Some(CommandOutcome::ForwardToPlugin {
            body_id: tg_body.clone(),
            command: "status".to_owned(),
            args: "verbose".to_owned(),
        })
    );
    let denied_admin_reset =
        reg.try_handle_slash_command("/admin_reset", &guest, &owner, None, Some(rbac), &caps);
    assert!(matches!(denied_admin_reset, Some(CommandOutcome::Denied { .. })));
    let fwd_admin_reset =
        reg.try_handle_slash_command("/admin_reset now", &owner, &owner, None, Some(rbac), &caps);
    assert_eq!(
        fwd_admin_reset,
        Some(CommandOutcome::ForwardToPlugin {
            body_id: tg_body.clone(),
            command: "admin_reset".to_owned(),
            args: "now".to_owned(),
        })
    );

    // Disconnecting plugin unregisters only its commands.
    reg.unregister_plugin_commands(&tg_body);
    assert!(reg.get("status").is_none());
    assert!(reg.get("help").is_some());
}

#[tokio::test]
async fn secrets_toml_env_forwards_into_plugins_respects_process_env_and_redacts_logs() {
    let tmp = TempDir::new().expect("tempdir");
    let missing_path = tmp.path().join("missing-secrets.toml");
    assert!(
        load_plugin_env_secrets(&missing_path)
            .expect("missing secrets file returns empty")
            .is_empty()
    );

    let json_path = tmp.path().join("json-secrets.toml");
    std::fs::write(&json_path, r#"{"model_api_key":"sk-test-json-123456"}"#).expect("write json");
    assert!(
        load_plugin_env_secrets(&json_path)
            .expect("json secrets returns empty env")
            .is_empty()
    );

    let bad_env_path = tmp.path().join("bad-env-secrets.toml");
    std::fs::write(&bad_env_path, "[env]\nTELEGRAM_BOT_TOKEN = 123\n").expect("write bad env");
    assert!(matches!(
        load_plugin_env_secrets(&bad_env_path),
        Err(SecretsLoadError::Parse { .. })
    ));

    let secrets_path = tmp.path().join("secrets.toml");
    std::fs::write(
        &secrets_path,
        r#"
model_api_key = "provider-key-abc-999"

[env]
TELEGRAM_BOT_TOKEN = "tg-bot-token-secret-12345"
TAVILY_API_KEY = "tvly-secret-67890"
EMPTY_IGNORED = "   "
"#,
    )
    .expect("write secrets.toml");

    // `[env]` entries do not break ProviderSecrets loading.
    let provider_secrets =
        load_provider_secrets(Some(&secrets_path), |_| None).expect("load provider secrets");
    assert_eq!(
        provider_secrets
            .model_api_key
            .as_ref()
            .map(SecretString::expose),
        Some("provider-key-abc-999")
    );

    let mut plugin_env = load_plugin_env_secrets(&secrets_path).expect("load [env] secrets");
    assert_eq!(plugin_env.len(), 2);
    assert_eq!(
        plugin_env.get("TELEGRAM_BOT_TOKEN").map(SecretString::expose),
        Some("tg-bot-token-secret-12345")
    );
    assert_eq!(
        plugin_env.get("TAVILY_API_KEY").map(SecretString::expose),
        Some("tvly-secret-67890")
    );

    // Explicit process env takes precedence over `[env]` from secrets.toml.
    let existing_env_key = std::env::vars_os()
        .find_map(|(k, _)| k.into_string().ok())
        .expect("at least one process env var");
    plugin_env.insert(
        existing_env_key.clone(),
        SecretString::new("should-not-override-process-env"),
    );

    let env_ctx = PluginEnvContext {
        config_dir: Some(tmp.path().join("cfg")),
        data_dir: Some(tmp.path().join("data")),
        rbac_enabled: false,
        plugin_env,
    };
    let built_envs = build_plugin_env("telegram", &env_ctx);
    assert!(
        built_envs
            .iter()
            .any(|(k, v)| k == "TELEGRAM_BOT_TOKEN" && v == "tg-bot-token-secret-12345")
    );
    assert!(
        !built_envs.iter().any(|(k, _)| k == &existing_env_key),
        "process env key '{existing_env_key}' must take precedence over secrets.toml [env]"
    );

    // Spawn a stdio plugin via `connect_configured_plugins` and verify both env forwarding and redaction.
    let (rt, clock, diagnostics) = start_test_gateway().await;
    let handle = rt.handle();
    let script = r#"IFS= read -r _line
printf '{"jsonrpc":"2.0","id":1,"result":{"body_id":"tg-env-body","protocol_version":{"major":1,"minor":0},"capabilities":[{"id":"tg.env.check","description":"tok=%s","places":["tg:chat:1"],"flags":{"communicative":false,"identity_changing":false,"irreversible":false},"typical_latency_ms":5,"capability_schema_hash":"h1"}],"places":["tg:chat:1"]}}\n' "$TELEGRAM_BOT_TOKEN"
sleep 2"#;
    let plugins = vec![PluginConfig {
        id: "telegram".to_owned(),
        enabled: true,
        kind: PluginKind::Adapter,
        transport: BodyTransportConfig::Stdio {
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
        },
        dependencies: Vec::new(),
        optional_dependencies: Vec::new(),
    }];

    let report = handle.connect_configured_plugins(&plugins, &env_ctx).await;
    assert_eq!(report.connected.len(), 1);
    assert!(report.failed.is_empty());

    let caps = handle.router().all_capabilities();
    let check_cap = caps
        .iter()
        .find(|c| c.id == CapabilityId::new("tg.env.check"))
        .expect("tg.env.check capability");
    assert_eq!(check_cap.description, "tok=tg-bot-token-secret-12345");

    diagnostics.log(
        clock.now(),
        LogLevel::Warn,
        "test.redaction",
        "leaked token tg-bot-token-secret-12345 and tvly-secret-67890",
    );
    let recs = diagnostics.records();
    let last = recs.last().expect("diagnostic record");
    assert!(!last.message.contains("tg-bot-token-secret-12345"));
    assert!(!last.message.contains("tvly-secret-67890"));
    assert!(last.message.contains("[REDACTED]"));

    rt.shutdown().await;
}

struct MockAdapterEndpoint {
    outbound_tx: mpsc::UnboundedSender<String>,
    responses_rx: mpsc::UnboundedReceiver<Waist2Response<Value>>,
    executed_rx: mpsc::UnboundedReceiver<ActionExecuteParams>,
    task: tokio::task::JoinHandle<()>,
}

fn spawn_adapter_with_meta<R, W>(
    body_id: &str,
    places: Vec<PlaceId>,
    caps: Vec<&str>,
    meta: Option<Value>,
    reader: R,
    mut writer: W,
) -> MockAdapterEndpoint
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<String>();
    let (responses_tx, responses_rx) = mpsc::unbounded_channel::<Waist2Response<Value>>();
    let (executed_tx, executed_rx) = mpsc::unbounded_channel::<ActionExecuteParams>();

    let bid = BodyId::new(body_id);
    let cap_descs: Vec<CapabilityDescriptor> = caps
        .into_iter()
        .map(|cid| CapabilityDescriptor {
            id: CapabilityId::new(cid),
            description: format!("Capability {cid}"),
            places: places.clone(),
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 10,
            capability_schema_hash: CapabilitySchemaHash::new("h1"),
            sensed_effect: None,
        })
        .collect();

    let task = tokio::spawn(async move {
        let mut buf_reader = TokioBufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            tokio::select! {
                biased;
                maybe_out = outbound_rx.recv() => {
                    match maybe_out {
                        Some(msg) => {
                            if writer.write_all(msg.as_bytes()).await.is_err()
                                || writer.write_all(b"\n").await.is_err()
                                || writer.flush().await.is_err()
                            {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                read_res = buf_reader.read_line(&mut line) => {
                    let Ok(n) = read_res else { break; };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let raw: Value = serde_json::from_str(trimmed).expect("valid json");
                    if raw.get("method").is_none() {
                        if let Ok(resp) = serde_json::from_value::<Waist2Response<Value>>(raw) {
                            let _ = responses_tx.send(resp);
                        }
                        continue;
                    }
                    let req: Waist2Request<Value> = serde_json::from_value(raw).expect("valid req");
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _p: BodyDiscoverParams = serde_json::from_value(req.params).expect("discover");
                            let res = BodyDiscoverResult {
                                body_id: bid.clone(),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: cap_descs.clone(),
                                places: places.clone(),
                            };
                            let mut val = serde_json::to_value(res).unwrap();
                            if let Some(m) = &meta {
                                val.as_object_mut().unwrap().insert("_meta".to_owned(), m.clone());
                            }
                            let resp = serde_json::to_string(&Waist2Response::ok(req.id, val)).unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                        }
                        Waist2Method::ActionExecute => {
                            let params: ActionExecuteParams = serde_json::from_value(req.params).expect("exec");
                            let _ = executed_tx.send(params.clone());
                            let receipt = Receipt {
                                action_id: params.action_id,
                                capability_id: params.capability_id,
                                outcome: ReceiptOutcome::Done,
                                reason_code: None,
                                detail: Some("ok".to_owned()),
                                termination_reason: Some(TerminationReason::Normal),
                                progress: Some(1.0),
                                delivered_units: Some(1),
                                h_act: params.h_act,
                                timestamp_ms: Time::from_millis(5_000_050),
                            };
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                ActionExecuteResult {
                                    accepted: true,
                                    receipt: Some(receipt),
                                },
                            ))
                            .unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                        }
                        Waist2Method::BodyShutdown => {
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                monoagens_protocol::BodyShutdownResult { accepted: true },
                            ))
                            .unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    MockAdapterEndpoint {
        outbound_tx,
        responses_rx,
        executed_rx,
        task,
    }
}

#[test]
fn generic_approval_prompt_dispatch_and_attested_adapter_resolve_with_identity() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = parse_config_str(
        r#"
version = 1

[model]
provider = "scripted"

[[person]]
actor_id = "owner"
owner = true
body_id = "body-telegram"
account_id = "tg:user:123"
"#,
        &tmp.path().join("monoagens.toml"),
        &tmp.path().join("secrets.toml"),
    )
    .expect("parse cfg");

    let owner = ActorId::new("owner");
    let mac_key = derive_approval_mac_key(b"wave2-approval-prompt-key");
    let clock = Arc::new(VirtualClock::new(Time::from_millis(5_000_000)));

    let mut snap = ValveSnapshot::new(monoagens_core::LogSeq::ZERO, clock.now());
    snap.owner_actor_id = Some(owner.clone());
    snap.known_actors.insert(owner.clone());
    snap.known_actors.insert(ActorId::new("soul"));
    snap.active_context_audience = Some(Audience::new([owner.clone(), ActorId::new("soul")]));
    snap.capability_risk_metadata.insert(
        CapabilityId::new("wallet.pay"),
        monoagens_valve::ActionRiskMetadata {
            is_financial_or_credential: true,
            ..Default::default()
        },
    );
    let mut valve = Valve::with_default_policies(ValveConfig::default(), snap).expect("valve");
    valve.set_approval_attestation_key(mac_key);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let scripted = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(
            json!({
                "acts": [{
                    "act": "act",
                    "capability_id": "wallet.pay",
                    "places": ["tg:chat:1"],
                    "people": ["owner"],
                    "payload": [112, 97, 121],
                    "motivation": 4,
                    "confidence_0_20": 18,
                    "used_ids": [],
                    "expected": null,
                    "domain": null
                }]
            })
            .to_string(),
        )]));

        let id_resolver = Arc::new(IdentityResolver::from_gateway_config(&cfg, None).expect("id"));
        let runtime = GatewayRuntime::start(
            cfg,
            &ProviderSecrets::default(),
            || Ok(MemStore::new()),
            CoreConfig {
                seed: 41,
                ..CoreConfig::default()
            },
            ValveDoorController::new(valve),
            GatewayOptions {
                clock: clock.clone(),
                provider: Some(scripted),
                diagnostics: DiagnosticSink::default(),
                cues: CueRecorder::default(),
                seams: GatewaySeams {
                    identity: Some(id_resolver as Arc<dyn IdentitySeam>),
                    ..Default::default()
                },
            },
        )
        .await
        .expect("start gateway");

        let handle = runtime.handle();

        // Adapter 1: declares `approval.prompt` + `_meta.commands`.
        let (gw1, b1) = tokio::io::duplex(64 * 1024);
        let (gw1_r, gw1_w) = tokio::io::split(gw1);
        let (b1_r, b1_w) = tokio::io::split(b1);
        let mut ep_prompt = spawn_adapter_with_meta(
            "body-telegram",
            vec![PlaceId::new("tg:chat:1")],
            vec!["wallet.pay", CAPABILITY_APPROVAL_PROMPT],
            Some(json!({
                "commands": [{
                    "name": "ping",
                    "summary": "Ping adapter",
                    "permission": "tg.ping",
                    "base_tier": "guest"
                }]
            })),
            b1_r,
            b1_w,
        );
        handle
            .connect_ndjson_body(gw1_r, gw1_w, Some(BodyId::new("body-telegram")))
            .await
            .expect("connect body-telegram");
        handle.register_out_of_band_control_body(BodyId::new("body-telegram"));

        // Adapter 2: does NOT declare `approval.prompt`.
        let (gw2, b2) = tokio::io::duplex(64 * 1024);
        let (gw2_r, gw2_w) = tokio::io::split(gw2);
        let (b2_r, b2_w) = tokio::io::split(b2);
        let ep_plain = spawn_adapter_with_meta(
            "body-plain",
            vec![PlaceId::new("plain:chat:1")],
            vec!["plain.send"],
            None,
            b2_r,
            b2_w,
        );
        handle
            .connect_ndjson_body(gw2_r, gw2_w, Some(BodyId::new("body-plain")))
            .await
            .expect("connect body-plain");

        // Verify _meta.commands from body/discover was automatically registered.
        assert!(
            handle
                .command_registry()
                .lock()
                .expect("lock")
                .get("ping")
                .is_some()
        );

        // Trigger held action via percept from tg:user:123 (resolved to `owner`).
        let mut p = Percept::minimal(
            ActorId::new("tg:user:123"),
            PlaceId::new("tg:chat:1"),
            Audience::new([ActorId::new("tg:user:123"), ActorId::new("soul")]),
            Time::from_millis(5_000_000),
            OpaquePayload::from_bytes(b"pay 10"),
        );
        p.addressees = vec![ActorId::new("soul")];
        p.taint = Taint::Trusted;

        let percept_req = Waist2Request::new(
            Some(RpcId::Int(1)),
            Waist2Method::PerceptEmit,
            PerceptEmitParams::from_percept(Waist2Headers::new("p1"), p),
        );
        ep_prompt
            .outbound_tx
            .send(serde_json::to_string(&percept_req).unwrap())
            .unwrap();
        let _ = ep_prompt.responses_rx.recv().await;

        let mut held_info = None;
        for _ in 0..50 {
            let events = handle.read_events().await.expect("events");
            for ev in &events {
                if let Some(EventDetail::Receipt {
                    action_id,
                    outcome: ReceiptOutcome::Held,
                    h_act: Some(h_act),
                    ..
                }) = &ev.detail
                {
                    held_info = Some((action_id.clone(), h_act.clone()));
                }
            }
            if held_info.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        let (action_id, h_act) = held_info.expect("action held");

        let prompt_req = ApprovalPromptRequest {
            action_id: action_id.clone(),
            h_act: h_act.clone(),
            state_epoch: 0,
            capability_id: CapabilityId::new("wallet.pay"),
            target_places: vec![PlaceId::new("tg:chat:1")],
            summary: "Approve wallet.pay?".to_owned(),
            expires_at_ms: Time::from_millis(5_300_000),
        };

        // Adapter without `approval.prompt` returns Ok(false) cleanly.
        let plain_sent = handle
            .router()
            .dispatch_approval_prompt(
                &PlaceId::new("plain:chat:1"),
                Audience::new([owner.clone()]),
                &prompt_req,
            )
            .await
            .expect("dispatch plain");
        assert!(!plain_sent);

        // Adapter declaring `approval.prompt` receives action/execute and returns Ok(true).
        let tg_sent = handle
            .router()
            .dispatch_approval_prompt(
                &PlaceId::new("tg:chat:1"),
                Audience::new([owner.clone()]),
                &prompt_req,
            )
            .await
            .expect("dispatch tg");
        assert!(tg_sent);

        let prompt_exec = ep_prompt.executed_rx.recv().await.expect("prompt exec");
        assert_eq!(
            prompt_exec.capability_id,
            CapabilityId::new(CAPABILITY_APPROVAL_PROMPT)
        );
        let decoded_prompt: ApprovalPromptRequest =
            serde_json::from_slice(prompt_exec.payload.as_bytes()).expect("prompt payload");
        assert_eq!(decoded_prompt.action_id, action_id);

        // Non-owner raw account `tg:user:999` tapping button fails deterministically in Valve.
        clock.advance_by(1_000);
        let sig = sign_approval_resolution(
            &mac_key,
            &action_id,
            &h_act,
            0,
            true,
            Time::from_millis(5_001_000),
        );
        let non_owner_resolve = Waist2Request::new(
            Some(RpcId::Int(2)),
            Waist2Method::ApprovalResolve,
            ApprovalResolveParams {
                headers: Waist2Headers::new("res-non-owner"),
                action_id: action_id.clone(),
                h_act: h_act.clone(),
                state_epoch: 0,
                approved: true,
                resolver: ActorId::new("tg:user:999"),
                resolved_at_ms: Time::from_millis(5_001_000),
                signature: Some(sig.clone()),
                out_of_band_channel: true,
            },
        );
        ep_prompt
            .outbound_tx
            .send(serde_json::to_string(&non_owner_resolve).unwrap())
            .unwrap();
        let non_owner_resp = ep_prompt.responses_rx.recv().await.expect("non-owner resp");
        assert!(
            non_owner_resp.error.is_some(),
            "non-owner approval/resolve must be refused"
        );

        // Owner raw account `tg:user:123` resolves via `IdentityResolver` -> `"owner"` and succeeds!
        let owner_resolve = Waist2Request::new(
            Some(RpcId::Int(3)),
            Waist2Method::ApprovalResolve,
            ApprovalResolveParams {
                headers: Waist2Headers::new("res-owner"),
                action_id: action_id.clone(),
                h_act: h_act.clone(),
                state_epoch: 0,
                approved: true,
                resolver: ActorId::new("tg:user:123"),
                resolved_at_ms: Time::from_millis(5_001_000),
                signature: Some(sig),
                out_of_band_channel: true,
            },
        );
        ep_prompt
            .outbound_tx
            .send(serde_json::to_string(&owner_resolve).unwrap())
            .unwrap();
        let owner_resp = ep_prompt.responses_rx.recv().await.expect("owner resp");
        assert!(
            owner_resp.error.is_none(),
            "expected owner approval/resolve to succeed, got {:?}",
            owner_resp.error
        );
        let parsed_res: ApprovalResolveResult =
            serde_json::from_value(owner_resp.result.unwrap()).expect("res");
        assert!(parsed_res.applied);

        let wallet_exec = ep_prompt.executed_rx.recv().await.expect("wallet exec");
        assert_eq!(wallet_exec.action_id, action_id);
        assert_eq!(wallet_exec.capability_id, CapabilityId::new("wallet.pay"));

        runtime.shutdown().await;
        let _ = ep_prompt.task.await;
        let _ = ep_plain.task.await;
    });
}
