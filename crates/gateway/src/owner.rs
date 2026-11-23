//! `GatewayOwner`: the typed control surface of the lock-holding process. The admin socket
//! (`crate::admin`) maps JSON-RPC methods one-to-one onto these methods, and an in-process
//! TUI can call them directly, so an attached and a local client behave the same.
//!
//! Admin methods never bypass a door: `turn.submit` enters through the same inbound path as
//! any body (`Input::Percept`), and `config.apply` only edits `monoagens.toml` and reloads.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use monoagens_core::{Input, ModelPurpose};
use monoagens_reason::model::catalog::{self, CatalogEntry};
use monoagens_reason::model::discovery;
use monoagens_reason::{ProviderSecrets, SecretString};
use monoagens_protocol::{
    ActorId, ApprovalResolveParams, Audience, CapabilityId, ChannelId, OpaquePayload, Percept,
    PlaceId, Taint,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, Item, TableLike, value};

use crate::commands::CommandOutcome;
use crate::config::{ConfigError, GatewayConfig, load_config_file, update_config_with_backup};
use crate::runlog::RunLog;
use crate::runtime::GatewayHandle;
use crate::supervisor::{PluginSupervisor, SupervisorError};

/// JSON-RPC style error code: method not found.
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC style error code: invalid params.
pub const ERR_INVALID_PARAMS: i32 = -32602;
/// Server-side failure while executing a valid request.
pub const ERR_SERVER: i32 = -32000;

/// Error returned by owner / admin methods (maps to a JSON-RPC error object).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminError {
    pub code: i32,
    pub message: String,
}

impl AdminError {
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ERR_INVALID_PARAMS, message)
    }

    #[must_use]
    pub fn server(message: impl Into<String>) -> Self {
        Self::new(ERR_SERVER, message)
    }
}

impl fmt::Display for AdminError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for AdminError {}

impl From<SupervisorError> for AdminError {
    fn from(err: SupervisorError) -> Self {
        Self::invalid_params(err.to_string())
    }
}

/// Which kind of process hosts the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerMode {
    Tui,
    Serve,
}

/// Inputs for [`GatewayOwner::new`].
pub struct OwnerParams {
    pub handle: GatewayHandle,
    pub supervisor: PluginSupervisor,
    pub runlog: RunLog,
    pub mode: OwnerMode,
    pub config: GatewayConfig,
    pub config_path: PathBuf,
    pub secrets_path: PathBuf,
    pub data_dir: PathBuf,
    /// Actor used by `turn.submit` when the request names none.
    pub owner_actor: ActorId,
}

/// What `config.apply` reloaded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReloadReport {
    pub valve: bool,
    pub plugins: Vec<String>,
    pub provider: bool,
}

type ProviderHook = Arc<dyn Fn(&GatewayConfig) + Send + Sync>;

/// Typed control surface of the lock-holding gateway process.
pub struct GatewayOwner {
    handle: GatewayHandle,
    supervisor: PluginSupervisor,
    runlog: RunLog,
    mode: OwnerMode,
    config: Mutex<GatewayConfig>,
    config_path: PathBuf,
    secrets_path: PathBuf,
    data_dir: PathBuf,
    owner_actor: ActorId,
    provider_hook: Mutex<Option<ProviderHook>>,
    jobs: crate::governance::Jobs,
    approvals: tokio::sync::Mutex<ApprovalsCache>,
    approvals_full_replays: std::sync::atomic::AtomicUsize,
}

/// Pending approvals derived from the event log, kept between `approvals.list` polls.
#[derive(Default)]
struct ApprovalsCache {
    /// Id of the last event folded into `entries` (`None` before the first poll).
    last_id: Option<monoagens_protocol::EventId>,
    entries: Vec<crate::governance::PendingApprovalEntry>,
}

impl GatewayOwner {
    #[must_use]
    pub fn new(p: OwnerParams) -> Self {
        Self {
            handle: p.handle,
            supervisor: p.supervisor,
            runlog: p.runlog,
            mode: p.mode,
            config: Mutex::new(p.config),
            config_path: p.config_path,
            secrets_path: p.secrets_path,
            data_dir: p.data_dir,
            owner_actor: p.owner_actor,
            provider_hook: Mutex::new(None),
            jobs: crate::governance::Jobs::default(),
            approvals: tokio::sync::Mutex::new(ApprovalsCache::default()),
            approvals_full_replays: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Pending approvals, maintained incrementally: an idle poll only asks the store for events
    /// after the cursor (O(new events)); the log is replayed in full only when a new event
    /// can change the approval projection (hold, verdict, receipt, expectation, forget).
    pub async fn approvals_pending(
        &self,
    ) -> Result<Vec<crate::governance::PendingApprovalEntry>, AdminError> {
        use monoagens_core::EventKind;
        use std::sync::atomic::Ordering;
        let mut cache = self.approvals.lock().await;
        let mut stale = cache.last_id.is_none();
        if let Some(after) = cache.last_id {
            match self.handle.read_events_after(after).await {
                Ok(new) => {
                    if let Some(last) = new.last() {
                        stale = new.iter().any(|e| {
                            matches!(
                                e.kind,
                                EventKind::HardForget
                                    | EventKind::Action
                                    | EventKind::DoorVerdict
                                    | EventKind::Receipt
                                    | EventKind::ExpectationChanged
                            )
                        });
                        cache.last_id = Some(last.id);
                    }
                }
                Err(_) => stale = true,
            }
        }
        if stale {
            self.approvals_full_replays.fetch_add(1, Ordering::Relaxed);
            let events = self
                .handle
                .read_events()
                .await
                .map_err(|e| AdminError::server(e.to_string()))?;
            let cfg = self.config_snapshot();
            let secrets = crate::secrets::load_approval_secrets(Some(&self.secrets_path), |k| {
                std::env::var(k).ok()
            })
            .map_err(|e| AdminError::server(e.to_string()))?;
            let mac_key = crate::secrets::resolve_approval_mac_key(&secrets);
            cache.entries = crate::governance::pending_approvals(
                &events,
                &self.owner_actor,
                &crate::governance::build_valve_config(&cfg),
                mac_key,
            );
            cache.last_id = events.last().map(|e| e.id);
        }
        Ok(cache.entries.clone())
    }

    /// Number of full event-log replays done for `approvals.list` (test hook).
    #[must_use]
    pub fn approvals_full_replays(&self) -> usize {
        self.approvals_full_replays
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Copy of the current configuration (after the latest `config.apply`).
    #[must_use]
    pub fn config_snapshot(&self) -> GatewayConfig {
        self.config.lock().expect("owner config lock").clone()
    }

    #[must_use]
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    #[must_use]
    pub fn secrets_path(&self) -> &std::path::Path {
        &self.secrets_path
    }

    #[must_use]
    pub fn owner_actor(&self) -> &ActorId {
        &self.owner_actor
    }

    pub(crate) fn jobs(&self) -> &crate::governance::Jobs {
        &self.jobs
    }

    #[must_use]
    pub fn handle(&self) -> &GatewayHandle {
        &self.handle
    }

    #[must_use]
    pub fn supervisor(&self) -> &PluginSupervisor {
        &self.supervisor
    }

    #[must_use]
    pub fn runlog(&self) -> &RunLog {
        &self.runlog
    }

    /// Installs the provider-rebuild hook run by `config.apply` when `[model]` / `[embed]`
    /// changed. The rebuild itself belongs to `model-provider-catalog`; until a hook is set
    /// the change is only logged.
    pub fn set_provider_rebuild_hook(&self, hook: ProviderHook) {
        *self.provider_hook.lock().expect("provider hook lock") = Some(hook);
    }

    /// `status`: `{owner:{pid,mode}, data_dir, provider, model, embedder, plugins, queue_depth}`.
    #[must_use]
    pub fn status(&self) -> Value {
        let cfg = self.config.lock().expect("owner config lock").clone();
        json!({
            "owner": { "pid": std::process::id(), "mode": self.mode },
            "data_dir": self.data_dir.display().to_string(),
            "socket": crate::admin::admin_socket_path(&self.data_dir).display().to_string(),
            "lock_path": self.data_dir.join(crate::lock::GATEWAY_LOCK_FILENAME).display().to_string(),
            "provider": cfg.model.provider.as_str(),
            "model": cfg.model.model,
            "embedder": cfg.embed.backend.as_str(),
            "plugins": self.supervisor.list().into_iter().map(|p| json!({
                "id": p.id,
                "state": p.state,
                "reason": p.reason,
            })).collect::<Vec<_>>(),
            "queue_depth": self.handle.ingress().len(),
            "model_calls_in_flight": self.handle.in_flight_model_calls(),
            "budget": self.budget_json(),
        })
    }

    fn budget_json(&self) -> Value {
        let (used, limit) = self.handle.budget().snapshot(self.handle.clock().now());
        json!({ "used": used, "limit": limit, "cost_usd": self.cost_today().0 })
    }

    fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("models")
    }

    /// `(input, output)` USD per million tokens for `provider`/`model`: the user's
    /// `[model.prices]` entry, else the discovery cache.
    fn price_of(&self, cfg: &GatewayConfig, provider: &str, model: &str) -> Option<(f64, f64)> {
        if let Some(p) = cfg.model.price_for(provider, model) {
            return Some((p.input_per_mtok, p.output_per_mtok));
        }
        let list = discovery::read_cache(&self.cache_dir(), provider)?;
        let m = list.models.iter().find(|m| m.id == model)?;
        Some((m.price_in_per_mtok?, m.price_out_per_mtok?))
    }

    /// Today's cost in USD and the per-row costs, or `None` while any price is unknown
    /// (including the active model's): the status bar then hides the cost.
    fn cost_today(&self) -> (Option<f64>, Vec<Option<f64>>) {
        let cfg = self.config.lock().expect("owner config lock").clone();
        let rows = self.handle.budget().usage_snapshot(self.handle.clock().now());
        let label = self.handle.live_model().label_for(ModelPurpose::Reason);
        let (_, model) = self.handle.live_model().route_for(ModelPurpose::Reason);
        let active = self.price_of(&cfg, &label, &model);
        let per_row: Vec<Option<f64>> = rows
            .iter()
            .map(|r| {
                self.price_of(&cfg, &r.provider, &r.model).map(|(pi, po)| {
                    r.counts.input_tokens as f64 / 1e6 * pi
                        + r.counts.output_tokens as f64 / 1e6 * po
                })
            })
            .collect();
        let total = if active.is_some() && per_row.iter().all(Option::is_some) {
            Some(per_row.iter().flatten().sum())
        } else {
            None
        };
        (total, per_row)
    }

    /// `usage.get`: `{day_utc, budget:{used,limit,left}, cost_usd, purposes:[{purpose,
    /// provider, model, calls, input_tokens, output_tokens, cost_usd}]}` for today (UTC).
    #[must_use]
    pub fn usage_get(&self) -> Value {
        let now = self.handle.clock().now();
        let (used, limit) = self.handle.budget().snapshot(now);
        let rows = self.handle.budget().usage_snapshot(now);
        let (total, per_row) = self.cost_today();
        let purposes: Vec<Value> = rows
            .iter()
            .zip(per_row)
            .map(|(r, cost)| {
                json!({
                    "purpose": r.purpose,
                    "provider": r.provider,
                    "model": r.model,
                    "calls": r.counts.calls,
                    "input_tokens": r.counts.input_tokens,
                    "output_tokens": r.counts.output_tokens,
                    "cost_usd": cost,
                })
            })
            .collect();
        json!({
            "day_utc": now.as_millis() / 86_400_000,
            "budget": { "used": used, "limit": limit, "left": limit.saturating_sub(used) },
            "cost_usd": total,
            "purposes": purposes,
        })
    }

    fn load_secrets(&self) -> Result<ProviderSecrets, AdminError> {
        crate::secrets::load_provider_secrets(Some(&self.secrets_path), |n| std::env::var(n).ok())
            .map_err(|e| AdminError::server(format!("failed to load secrets: {e}")))
    }

    fn catalog_entry(provider: &str) -> Result<&'static CatalogEntry, AdminError> {
        catalog::find(provider)
            .ok_or_else(|| AdminError::invalid_params(format!("unknown provider '{provider}'")))
    }

    /// Base URL override from `[model.provider_options]`, only for the configured provider.
    fn base_override(&self, id: &str) -> Option<String> {
        let cfg = self.config.lock().expect("owner config lock");
        (cfg.model.catalog_provider.as_deref() == Some(id))
            .then(|| cfg.model.provider_base_url.clone())
            .flatten()
    }

    /// `providers.list`: `{current:{provider,model}, providers:[{id,label,keyless,key_set,
    /// key_source,discovery,default_model,current}]}` from the catalog plus key state. Never
    /// returns key material.
    pub fn providers_list(&self) -> Result<Value, AdminError> {
        let secrets = self.load_secrets()?;
        let (current, model) = {
            let cfg = self.config.lock().expect("owner config lock");
            (cfg.model.catalog_provider.clone(), cfg.model.model.clone())
        };
        let providers: Vec<Value> = catalog::entries()
            .iter()
            .map(|e| {
                let from_secrets = secrets.provider_keys.get(&e.id).is_some_and(|k| !k.is_empty());
                let from_env = e
                    .api_key_env
                    .iter()
                    .any(|n| std::env::var(n).is_ok_and(|v| !v.trim().is_empty()));
                let source = if from_secrets {
                    Some("secrets")
                } else if from_env {
                    Some("env")
                } else if !e.keyless && secrets.model_api_key.is_some() {
                    Some("legacy")
                } else {
                    None
                };
                json!({
                    "id": e.id,
                    "label": e.label,
                    "keyless": e.keyless,
                    "key_set": source.is_some(),
                    "key_source": source,
                    "discovery": e.discovery != catalog::DiscoveryMode::None,
                    "default_model": e.default_model,
                    "current": current.as_deref() == Some(e.id.as_str()),
                })
            })
            .collect();
        Ok(json!({ "current": { "provider": current, "model": model }, "providers": providers }))
    }

    /// `models.discover` `{provider, refresh}`: `{provider, models:[ModelInfo], fetched_at_ms,
    /// stale, cached, error}`. Without `refresh` only the cache is read (instant, no network);
    /// with `refresh` the provider is asked (10 s timeout) and the cache rewritten, falling
    /// back to the cached list (`stale: true`) plus an `error` text when that fails.
    pub async fn models_discover(&self, provider: &str, refresh: bool) -> Result<Value, AdminError> {
        let entry = Self::catalog_entry(provider)?;
        let cache_dir = self.cache_dir();
        if !refresh {
            let list = discovery::read_cache(&cache_dir, &entry.id);
            return Ok(match list {
                Some(l) => json!({
                    "provider": entry.id, "models": l.models, "fetched_at_ms": l.fetched_at_ms,
                    "stale": false, "cached": true, "error": null,
                }),
                None => json!({
                    "provider": entry.id, "models": [], "fetched_at_ms": 0,
                    "stale": false, "cached": true, "error": null,
                }),
            });
        }
        let key = self.load_secrets()?.api_key_for(entry, |n| std::env::var(n).ok());
        let base = self.base_override(&entry.id);
        let (list, err) = tokio::task::spawn_blocking(move || {
            discovery::list_models_cached(entry, key.as_ref(), base.as_deref(), &cache_dir)
        })
        .await
        .map_err(|e| AdminError::server(format!("discovery task failed: {e}")))?;
        let error = err.map(|e| e.to_string());
        if let Some(e) = &error {
            self.runlog
                .warn("discovery", &format!("model discovery for '{}' failed: {e}", entry.id));
        }
        Ok(json!({
            "provider": entry.id, "models": list.models, "fetched_at_ms": list.fetched_at_ms,
            "stale": list.stale, "cached": false, "error": error,
        }))
    }

    /// `providers.test` `{provider, api_key?}`: connection test by listing models with the
    /// given key (or the stored one): `{ok, model_count, latency_ms, error}`. Saves nothing.
    pub async fn providers_test(
        &self,
        provider: &str,
        api_key: Option<&str>,
    ) -> Result<Value, AdminError> {
        let entry = Self::catalog_entry(provider)?;
        let key = match api_key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => Some(SecretString::new(k)),
            None => self.load_secrets()?.api_key_for(entry, |n| std::env::var(n).ok()),
        };
        let base = self.base_override(&entry.id);
        let started = std::time::Instant::now();
        let res = tokio::task::spawn_blocking(move || {
            discovery::list_models(entry, key.as_ref(), base.as_deref())
        })
        .await
        .map_err(|e| AdminError::server(format!("connection test task failed: {e}")))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(match res {
            Ok(models) => {
                self.runlog.info(
                    "discovery",
                    &format!("connection test for '{}': {} models in {latency_ms} ms", entry.id, models.len()),
                );
                json!({ "ok": true, "model_count": models.len(), "latency_ms": latency_ms, "error": null })
            }
            Err(e) => {
                self.runlog.warn(
                    "discovery",
                    &format!("connection test for '{}' failed: {e}", entry.id),
                );
                json!({ "ok": false, "model_count": 0, "latency_ms": latency_ms, "error": e.to_string() })
            }
        })
    }

    /// `secrets.set` `{provider, api_key}`: writes `[providers.<id>] api_key` into
    /// `secrets.toml` (mode `0600`, other content kept) and rebuilds the live provider when
    /// that provider is in use. The key is never echoed back or logged.
    pub async fn secrets_set(&self, provider: &str, api_key: &str) -> Result<Value, AdminError> {
        let entry = Self::catalog_entry(provider)?;
        let key = api_key.trim();
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(AdminError::invalid_params("api_key must be a non-empty single line"));
        }
        write_provider_key(&self.secrets_path, &entry.id, key)
            .map_err(|e| AdminError::server(format!("failed to write secrets: {e}")))?;
        self.handle
            .diagnostics()
            .register_secret(&SecretString::new(key));
        self.runlog
            .info("settings", &format!("API key saved for provider '{}'", entry.id));
        let cfg = self.config.lock().expect("owner config lock").clone();
        let in_use = cfg.model.catalog_provider.as_deref() == Some(entry.id.as_str())
            || cfg
                .model
                .purposes
                .values()
                .any(|p| p.catalog_provider.as_deref() == Some(entry.id.as_str()));
        let mut rebuilt = false;
        if in_use && cfg.model.provider != crate::config::ModelProviderKind::Scripted {
            let secrets = self.load_secrets()?;
            self.handle
                .rebuild_model(&cfg, &secrets)
                .await
                .map_err(|e| AdminError::server(format!("key saved but provider rebuild failed: {e}")))?;
            rebuilt = true;
        }
        Ok(json!({ "ok": true, "provider_rebuilt": rebuilt }))
    }

    /// `secrets.set` `{env, value}` or `{key, value}`: writes `[env] <NAME>` (plugin environment)
    /// or one of the top-level keys `model_api_key`, `model_endpoint`, `embed_api_key` into
    /// `secrets.toml` (mode `0600`, other content kept). A missing `value` removes the entry.
    /// The value is never echoed or logged.
    pub fn secrets_set_named(&self, section: &str, name: &str, val: Option<&str>) -> Result<Value, AdminError> {
        let name = name.trim();
        let name_ok = !name.is_empty()
            && name.len() <= 128
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !name_ok {
            return Err(AdminError::invalid_params(
                "secret name must be 1..=128 characters of A-Z, a-z, 0-9 or _",
            ));
        }
        match section {
            "env" => {}
            "top" if matches!(name, "model_api_key" | "model_endpoint" | "embed_api_key") => {}
            _ => {
                return Err(AdminError::invalid_params(format!(
                    "unsupported secret '{section}.{name}'"
                )));
            }
        }
        let val = val.map(str::trim);
        if let Some(v) = val
            && (v.is_empty() || v.chars().any(char::is_control))
        {
            return Err(AdminError::invalid_params("value must be a non-empty single line"));
        }
        write_named_secret(&self.secrets_path, section, name, val)
            .map_err(|e| AdminError::server(format!("failed to write secrets: {e}")))?;
        if let Some(v) = val
            && name != "model_endpoint"
        {
            self.handle.diagnostics().register_secret(&SecretString::new(v));
        }
        self.runlog.info(
            "settings",
            &format!("secret {section}.{name} {}", if val.is_some() { "saved" } else { "removed" }),
        );
        Ok(json!({ "ok": true }))
    }

    /// `capabilities.list`: `{capabilities:[{id, body}]}` of every capability registered by a
    /// connected body (what a body reported in `body/discover`), for RBAC allow/deny pickers.
    #[must_use]
    pub fn capabilities_list(&self) -> Value {
        let caps: Vec<Value> = self
            .handle
            .router()
            .all_capabilities()
            .into_iter()
            .map(|c| json!({ "id": c.id.as_str() }))
            .collect();
        json!({ "capabilities": caps })
    }

    /// `events.since`: `{total, events, budget}` with the log entries at index `from..`.
    pub async fn events_since(&self, from: usize) -> Result<Value, AdminError> {
        let events = self
            .handle
            .read_events()
            .await
            .map_err(|e| AdminError::server(e.to_string()))?;
        let total = events.len();
        let tail = events.get(from..).unwrap_or_default();
        let events_json = serde_json::to_value(tail)
            .map_err(|e| AdminError::server(format!("failed to encode events: {e}")))?;
        Ok(json!({ "total": total, "events": events_json, "budget": self.budget_json() }))
    }

    /// `commands.list`: `{commands:[CommandSpec]}` of the gateway slash-command registry.
    #[must_use]
    pub fn commands_list(&self) -> Value {
        let reg = self.handle.command_registry();
        let specs: Vec<_> = reg
            .lock()
            .expect("cmd reg lock")
            .all_commands()
            .into_iter()
            .cloned()
            .collect();
        json!({ "commands": specs })
    }

    /// `commands.run`: runs a slash command from `actor` at `place`. The result is
    /// `{outcome:"handled",reply_text}`, `{outcome:"denied",reason}`,
    /// `{outcome:"forward",body_id,command,args}` or `{outcome:"unknown"}`.
    #[must_use]
    pub fn command_run(&self, raw: &str, actor: Option<&str>, place: Option<&str>) -> Value {
        let actor = actor.map_or_else(|| self.owner_actor.clone(), ActorId::new);
        let place = place.map(PlaceId::new);
        let caps: Vec<CapabilityId> = self
            .handle
            .router()
            .all_capabilities()
            .into_iter()
            .map(|c| c.id)
            .collect();
        let rbac = self.config.lock().expect("owner config lock").rbac.clone();
        let outcome = self
            .handle
            .command_registry()
            .lock()
            .expect("cmd reg lock")
            .try_handle_slash_command(raw, &actor, &self.owner_actor, place.as_ref(), rbac.as_ref(), &caps);
        match outcome {
            Some(CommandOutcome::Handled { reply_text }) => {
                json!({ "outcome": "handled", "reply_text": reply_text })
            }
            Some(CommandOutcome::Denied { reason }) => json!({ "outcome": "denied", "reason": reason }),
            Some(CommandOutcome::ForwardToPlugin { body_id, command, args }) => json!({
                "outcome": "forward", "body_id": body_id.as_str(), "command": command, "args": args,
            }),
            None => json!({ "outcome": "unknown" }),
        }
    }

    /// `approvals.resolve`: submits an attested out-of-band `approval/resolve` into the owner.
    pub async fn approvals_resolve(
        &self,
        params: ApprovalResolveParams,
    ) -> Result<Value, AdminError> {
        let res = self
            .handle
            .resolve_approval(params)
            .await
            .map_err(|e| AdminError::server(e.message))?;
        Ok(json!({ "applied": res.applied }))
    }

    /// `config.get`: `{path, toml}` with the raw file text (comments included).
    pub fn config_get(&self) -> Result<Value, AdminError> {
        let raw = std::fs::read_to_string(&self.config_path)
            .map_err(|e| AdminError::server(format!("failed to read config: {e}")))?;
        Ok(json!({ "path": self.config_path.display().to_string(), "toml": raw }))
    }

    /// `config.apply`: merges the JSON `patch` into `monoagens.toml` with `toml_edit` (a `.bak`
    /// copy is written first, comments are preserved, `null` removes a key) and reloads only
    /// what changed: valve (in the Core owner task), plugins (restart changed ones) and, via
    /// the stub hook, the provider.
    pub async fn config_apply(&self, patch: &Value) -> Result<ReloadReport, AdminError> {
        let Some(obj) = patch.as_object() else {
            return Err(AdminError::invalid_params("config.apply patch must be an object"));
        };
        let obj = obj.clone();
        update_config_with_backup(&self.config_path, &self.secrets_path, |doc| {
            merge_table(doc.as_table_mut(), &obj, "")
        })
        .map_err(|e| AdminError::server(e.to_string()))?;
        let new_cfg = load_config_file(&self.config_path, &self.secrets_path)
            .map_err(|e| AdminError::server(e.to_string()))?;
        let old = std::mem::replace(
            &mut *self.config.lock().expect("owner config lock"),
            new_cfg.clone(),
        );

        // Valve / RBAC settings feed the approval projection: rebuild it on the next poll.
        self.approvals.lock().await.last_id = None;

        let mut report = ReloadReport::default();
        if old.valve != new_cfg.valve
            || old.rbac != new_cfg.rbac
            || old.model.daily_token_budget != new_cfg.model.daily_token_budget
        {
            report.valve = self
                .handle
                .reload_valve(&new_cfg)
                .await
                .map_err(|e| AdminError::server(e.to_string()))?;
        }
        if old.effective_plugins() != new_cfg.effective_plugins() {
            report.plugins = self.supervisor.reload(new_cfg.effective_plugins()).await;
        }
        if old.model != new_cfg.model || old.embed != new_cfg.embed {
            report.provider = true;
            if old.model != new_cfg.model
                && new_cfg.model.provider != crate::config::ModelProviderKind::Scripted
            {
                // Scripted providers are injected by the embedder and cannot be rebuilt
                // from config; every other provider is rebuilt live for the next request.
                let secrets = crate::secrets::load_provider_secrets(
                    Some(&self.secrets_path),
                    |n| std::env::var(n).ok(),
                )
                .map_err(|e| AdminError::server(format!("failed to load secrets: {e}")))?;
                if let Err(e) = self.handle.rebuild_model(&new_cfg, &secrets).await {
                    self.runlog.warn(
                        "provider",
                        &format!("provider rebuild failed, previous provider kept: {e}"),
                    );
                    return Err(AdminError::server(format!(
                        "config saved but the provider could not be rebuilt (previous provider kept): {e}"
                    )));
                }
                self.runlog.info(
                    "provider",
                    &format!(
                        "provider rebuilt: {} / {}",
                        new_cfg
                            .model
                            .catalog_provider
                            .as_deref()
                            .unwrap_or_else(|| new_cfg.model.provider.as_str()),
                        new_cfg.model.model
                    ),
                );
            }
            let hook = self.provider_hook.lock().expect("provider hook lock").clone();
            match hook {
                Some(h) => h(&new_cfg),
                None => self.runlog.info(
                    "settings",
                    "model/embed settings changed; provider rebuild hook not wired yet",
                ),
            }
        }
        self.runlog.info(
            "settings",
            &format!(
                "config applied (valve={}, plugins=[{}], provider={})",
                report.valve,
                report.plugins.join(","),
                report.provider
            ),
        );
        Ok(report)
    }

    /// `plugins.list`.
    #[must_use]
    pub fn plugins_list(&self) -> Value {
        json!({ "plugins": self.supervisor.list() })
    }

    /// `plugins.start`.
    pub async fn plugins_start(&self, id: &str) -> Result<(), AdminError> {
        self.supervisor.start(id).await?;
        Ok(())
    }

    /// `plugins.stop`.
    pub async fn plugins_stop(&self, id: &str) -> Result<(), AdminError> {
        self.supervisor.stop(id).await?;
        Ok(())
    }

    /// `plugins.restart`.
    pub async fn plugins_restart(&self, id: &str) -> Result<(), AdminError> {
        self.supervisor.restart(id).await?;
        Ok(())
    }

    /// `turn.submit`: sends `text` into the inbound door as a trusted percept from the owner
    /// (or `actor`) at `place` (default `cli:terminal`) and waits for the owner step.
    pub async fn turn_submit(
        &self,
        text: &str,
        actor: Option<&str>,
        place: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Value, AdminError> {
        let actor = actor.map_or_else(|| self.owner_actor.clone(), ActorId::new);
        let now = self.handle.clock().now();
        let mut percept = Percept::minimal(
            actor.clone(),
            PlaceId::new(place.unwrap_or("cli:terminal")),
            Audience::new([actor.clone(), ActorId::new("soul")]),
            now,
            OpaquePayload::from_bytes(text.as_bytes()),
        );
        percept.taint = if actor == self.owner_actor {
            Taint::Trusted
        } else {
            Taint::Untrusted
        };
        percept.addressees = vec![ActorId::new("soul")];
        percept.length = Some(text.len() as u64);
        percept.channel = channel.map(ChannelId::new);
        if let Some(pq) = self.handle.pre_queue_service() {
            let _ = pq.enrich_percept_async(&mut percept).await;
        }
        let outcome = self
            .handle
            .submit_and_wait(Input::Percept(Box::new(percept)))
            .await
            .map_err(|e| AdminError::server(e.to_string()))?;
        Ok(json!({ "ok": true, "effects_spawned": outcome.effects_spawned }))
    }

    /// `turn.submit` with a complete client-built percept (the TUI attaches files and lab
    /// actors). It still enters through the inbound door like any body's percept; the admin
    /// socket is owner-only, so the client is trusted to build it.
    pub async fn turn_submit_percept(&self, mut percept: Percept) -> Result<Value, AdminError> {
        if let Some(pq) = self.handle.pre_queue_service() {
            let _ = pq.enrich_percept_async(&mut percept).await;
        }
        let outcome = self
            .handle
            .submit_and_wait(Input::Percept(Box::new(percept)))
            .await
            .map_err(|e| AdminError::server(e.to_string()))?;
        Ok(json!({ "ok": true, "effects_spawned": outcome.effects_spawned }))
    }

    /// `turn.cancel`: `{"cancelled": true|false}`.
    #[must_use]
    pub fn turn_cancel(&self) -> Value {
        let n = self.handle.cancel_turn();
        if n > 0 {
            self.runlog.info("turn", "turn cancelled by owner");
        }
        json!({ "cancelled": n > 0 })
    }
}

/// Sets `[providers.<id>] api_key` in `secrets.toml`, keeping everything else, and leaves the
/// file at mode `0600` (written to a private temporary file, then renamed into place).
/// Public form of the provider key write (used by the first-run wizard, which has no owner yet).
pub fn write_provider_key(path: &std::path::Path, id: &str, key: &str) -> std::io::Result<()> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    if raw.trim_start().starts_with('{') {
        return Err(std::io::Error::other(
            "secrets file is JSON; convert it to TOML to store provider keys",
        ));
    }
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| std::io::Error::other(format!("secrets file is not valid TOML: {e}")))?;
    if !doc.get("providers").is_some_and(Item::is_table_like) {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc.insert("providers", Item::Table(t));
    }
    let providers = doc["providers"].as_table_like_mut().expect("providers table");
    if !providers.get(id).is_some_and(Item::is_table_like) {
        providers.insert(id, Item::Table(toml_edit::Table::new()));
    }
    providers
        .get_mut(id)
        .and_then(Item::as_table_like_mut)
        .expect("provider table")
        .insert("api_key", value(key));
    write_secrets_doc_private(path, &doc)
}

/// Sets or removes (`val == None`) `[env] <name>` or a top-level key of `secrets.toml`
/// (same guarantees as [`write_provider_key`]).
fn write_named_secret(
    path: &std::path::Path,
    section: &str,
    name: &str,
    val: Option<&str>,
) -> std::io::Result<()> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    if raw.trim_start().starts_with('{') {
        return Err(std::io::Error::other(
            "secrets file is JSON; convert it to TOML to store secrets",
        ));
    }
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| std::io::Error::other(format!("secrets file is not valid TOML: {e}")))?;
    if section == "env" {
        if !doc.get("env").is_some_and(Item::is_table_like) {
            if val.is_none() {
                return Ok(());
            }
            doc.insert("env", Item::Table(toml_edit::Table::new()));
        }
        let env = doc["env"].as_table_like_mut().expect("env table");
        match val {
            Some(v) => {
                env.insert(name, value(v));
            }
            None => {
                env.remove(name);
            }
        }
    } else {
        match val {
            Some(v) => {
                doc.insert(name, value(v));
            }
            None => {
                doc.remove(name);
            }
        }
    }
    write_secrets_doc_private(path, &doc)
}

fn write_secrets_doc_private(path: &std::path::Path, doc: &toml_edit::DocumentMut) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(doc.to_string().as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Applies a JSON merge `patch` to `monoagens.toml` on disk exactly like `config.apply` does
/// (`.bak` written first, comments preserved, `null` removes a key, validated before the write)
/// but without reloading a running gateway. Used by a TUI that has no gateway yet.
pub fn apply_config_patch_to_file(
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
    patch: &Value,
) -> Result<(), ConfigError> {
    let Some(obj) = patch.as_object() else {
        return Err(ConfigError::InvalidType {
            key: "patch".to_owned(),
            expected: "JSON object",
        });
    };
    update_config_with_backup(config_path, secrets_path, |doc| {
        merge_table(doc.as_table_mut(), obj, "")
    })
}

fn merge_table(
    table: &mut dyn TableLike,
    patch: &Map<String, Value>,
    prefix: &str,
) -> Result<(), ConfigError> {
    for (key, val) in patch {
        let full = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match val {
            Value::Null => {
                table.remove(key);
            }
            Value::Object(map) => {
                if !table.get(key).is_some_and(Item::is_table_like) {
                    table.insert(key, Item::Table(toml_edit::Table::new()));
                }
                let child = table
                    .get_mut(key)
                    .and_then(Item::as_table_like_mut)
                    .expect("table just ensured");
                merge_table(child, map, &full)?;
            }
            other => {
                let mut new_item = json_to_item(other, &full)?;
                if let (Some(Item::Value(old)), Item::Value(nv)) = (table.get(key), &mut new_item) {
                    *nv.decor_mut() = old.decor().clone();
                }
                table.insert(key, new_item);
            }
        }
    }
    Ok(())
}

fn json_to_item(v: &Value, key: &str) -> Result<Item, ConfigError> {
    if let Value::Array(items) = v
        && !items.is_empty()
        && items.iter().all(Value::is_object)
    {
        let mut aot = ArrayOfTables::new();
        for it in items {
            let Item::Value(toml_edit::Value::InlineTable(inline)) = json_to_item_value(it, key)?
            else {
                continue;
            };
            aot.push(inline.into_table());
        }
        return Ok(Item::ArrayOfTables(aot));
    }
    Ok(Item::Value(json_to_value(v, key)?))
}

fn json_to_item_value(v: &Value, key: &str) -> Result<Item, ConfigError> {
    Ok(Item::Value(json_to_value(v, key)?))
}

fn json_to_value(v: &Value, key: &str) -> Result<toml_edit::Value, ConfigError> {
    let bad = |what: &str| ConfigError::InvalidValue {
        key: key.to_owned(),
        value: what.to_owned(),
        expected: "string, bool, number, array or object",
    };
    Ok(match v {
        Value::Null => return Err(bad("null")),
        Value::Bool(b) => value(*b).into_value().expect("value"),
        Value::String(s) => value(s.as_str()).into_value().expect("value"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                value(i).into_value().expect("value")
            } else if let Some(f) = n.as_f64() {
                value(f).into_value().expect("value")
            } else {
                return Err(bad(&n.to_string()));
            }
        }
        Value::Array(items) => {
            let mut arr = toml_edit::Array::new();
            for it in items {
                arr.push(json_to_value(it, key)?);
            }
            toml_edit::Value::Array(arr)
        }
        Value::Object(map) => {
            let mut tbl = toml_edit::InlineTable::new();
            for (k, val) in map {
                tbl.insert(k, json_to_value(val, key)?);
            }
            toml_edit::Value::InlineTable(tbl)
        }
    })
}

