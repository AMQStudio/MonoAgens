//! `GatewayOwner`: the typed control surface of the lock-holding process. The admin socket
//! (`crate::admin`) maps JSON-RPC methods one-to-one onto these methods, and an in-process
//! TUI can call them directly, so an attached and a local client behave the same.
//!
//! Admin methods never bypass a door: `turn.submit` enters through the same inbound path as
//! any body (`Input::Percept`), and `config.apply` only edits `monoagens.toml` and reloads.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use monoagens_core::Input;
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
        }
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
            "provider": cfg.model.provider.as_str(),
            "model": cfg.model.model,
            "embedder": cfg.embed.backend.as_str(),
            "plugins": self.supervisor.list().into_iter().map(|p| json!({
                "id": p.id,
                "state": p.state,
                "reason": p.reason,
            })).collect::<Vec<_>>(),
            "queue_depth": self.handle.ingress().len(),
            "budget": self.budget_json(),
        })
    }

    fn budget_json(&self) -> Value {
        let (used, limit) = self.handle.budget().snapshot(self.handle.clock().now());
        json!({ "used": used, "limit": limit })
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

