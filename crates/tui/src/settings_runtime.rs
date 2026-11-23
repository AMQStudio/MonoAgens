//! Async side of `/settings`: every write goes through the gateway client (`config.apply`,
//! `secrets.set`, `plugins.*`) so an attached TUI edits the daemon exactly like a hosting one.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{Notify, mpsc};

use crate::client::{GatewayClient, RuntimeEvent};
use crate::pairing::{PairCommand, PairingEvent, run_pairing};

/// Requests of the settings screens to the gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    /// `config.apply` with a merge patch; `ok` is the toast on success.
    Apply { patch: Value, ok: String },
    /// `secrets.set` (`section` is `env` or `top`); `None` removes the secret. The value is
    /// never echoed back or logged.
    Secret {
        section: String,
        name: String,
        value: Option<SecretText>,
        ok: String,
    },
    /// `plugins.restart`.
    RestartPlugin(String),
    /// Starts the pairing subprocess and streams its output.
    StartPairing(PairCommand),
    /// Cancels a running pairing.
    CancelPairing,
    /// Reads the capability ids of connected bodies and the actors seen in the event log for the
    /// RBAC pickers.
    LoadRbacHints,
    /// Reads the live gateway configuration snapshot (`config.get`) so attached TUIs match the
    /// daemon.
    LoadConfigSnapshot,
    /// First-run wizard connection test: lists the provider's models with `key` (no gateway
    /// is running yet).
    WizardTest {
        provider: String,
        key: SecretText,
        base_url: Option<String>,
    },
}

/// A secret value that never prints.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretText(String);

impl SecretText {
    /// Wraps `value`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plaintext (only for the write to the gateway).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText(..)")
    }
}

/// Answers folded into the state by [`crate::TuiAppState::on_settings_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEvent {
    /// A write finished.
    Done {
        ok: String,
        result: Result<(), String>,
    },
    /// Output of the pairing process.
    Pairing(PairingEvent),
    /// Live configuration snapshot returned by `config.get`.
    ConfigLoaded(Value),
    /// Known capability ids, full `capabilities.list` payload, and seen actors for RBAC pickers.
    RbacHints {
        capabilities: Vec<String>,
        capabilities_json: Option<Value>,
        actors: Vec<String>,
    },
    /// The wizard's connection test finished.
    WizardTested(crate::wizard::WizardTest),
}

static PAIRING_CANCEL: Mutex<Option<Arc<Notify>>> = Mutex::new(None);

/// Runs one action against `client` and reports through `tx`.
pub async fn run_settings_action(
    action: SettingsAction,
    client: Arc<dyn GatewayClient>,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    let send = |ev: SettingsEvent| {
        let _ = tx.send(RuntimeEvent::Settings(ev));
    };
    match action {
        SettingsAction::Apply { patch, ok } => {
            let result = client
                .config_apply(patch)
                .await
                .map(|_| ())
                .map_err(|e| e.message);
            if result.is_ok()
                && let Ok(snapshot) = client.config_get().await
            {
                send(SettingsEvent::ConfigLoaded(snapshot));
            }
            send(SettingsEvent::Done { ok, result });
        }
        SettingsAction::Secret {
            section,
            name,
            value,
            ok,
        } => {
            let result = client
                .secrets_set_named(section, name, value.map(|v| v.expose().to_owned()))
                .await
                .map(|_| ())
                .map_err(|e| e.message);
            send(SettingsEvent::Done { ok, result });
        }
        SettingsAction::RestartPlugin(id) => {
            let result = client
                .plugins_restart(id.clone())
                .await
                .map_err(|e| e.message);
            send(SettingsEvent::Done {
                ok: format!("Restarted plugin '{id}'"),
                result,
            });
        }
        a @ SettingsAction::WizardTest { .. } => {
            run_settings_action_without_client(a, tx.clone()).await
        }
        SettingsAction::LoadConfigSnapshot => match client.config_get().await {
            Ok(val) => send(SettingsEvent::ConfigLoaded(val)),
            Err(e) => send(SettingsEvent::Done {
                ok: String::new(),
                result: Err(e.message),
            }),
        },
        SettingsAction::LoadRbacHints => {
            let capabilities_json = client
                .gov_call("capabilities.list".to_owned(), serde_json::json!({}))
                .await
                .ok();
            let capabilities: Vec<String> = capabilities_json
                .as_ref()
                .and_then(|v| v.get("capabilities").and_then(Value::as_array))
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let events = client
                .gov_call(
                    "events.query".to_owned(),
                    serde_json::json!({ "limit": 1000 }),
                )
                .await;
            let mut actors: Vec<String> = events
                .ok()
                .and_then(|v| v.get("events").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|e| e.get("actor").and_then(Value::as_str).map(str::to_owned))
                .filter(|a| a != "soul")
                .collect();
            actors.sort();
            actors.dedup();
            send(SettingsEvent::RbacHints {
                capabilities,
                capabilities_json,
                actors,
            });
        }
        other @ (SettingsAction::StartPairing(_) | SettingsAction::CancelPairing) => {
            run_settings_action_without_client(other, tx.clone()).await;
        }
    }
}

/// Pairing needs no gateway client: runs it without one (a TUI whose gateway is down can still
/// pair, the session file lives in the data dir).
pub async fn run_settings_action_without_client(
    action: SettingsAction,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    let send = |ev: SettingsEvent| {
        let _ = tx.send(RuntimeEvent::Settings(ev));
    };
    match action {
        SettingsAction::StartPairing(cmd) => {
            let cancel = Arc::new(Notify::new());
            *PAIRING_CANCEL.lock().expect("pairing lock") = Some(Arc::clone(&cancel));
            let (ptx, mut prx) = mpsc::unbounded_channel();
            let runner = tokio::spawn(run_pairing(cmd, ptx, cancel));
            while let Some(ev) = prx.recv().await {
                send(SettingsEvent::Pairing(ev));
            }
            let _ = runner.await;
            *PAIRING_CANCEL.lock().expect("pairing lock") = None;
        }
        SettingsAction::CancelPairing => {
            if let Some(n) = PAIRING_CANCEL.lock().expect("pairing lock").as_ref() {
                n.notify_one();
            }
        }
        SettingsAction::WizardTest {
            provider,
            key,
            base_url,
        } => {
            let started = std::time::Instant::now();
            let res = tokio::task::spawn_blocking(move || {
                let entry = monoagens_reason::model::catalog::find(&provider)
                    .ok_or_else(|| format!("unknown provider '{provider}'"))?;
                let secret = (!key.expose().is_empty())
                    .then(|| monoagens_reason::SecretString::new(key.expose()));
                monoagens_reason::model::discovery::list_models(
                    entry,
                    secret.as_ref(),
                    base_url.as_deref(),
                )
                .map(|models| models.into_iter().map(|m| m.id).collect::<Vec<_>>())
                .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|e| Err(format!("connection test task failed: {e}")));
            let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            send(SettingsEvent::WizardTested(crate::wizard::WizardTest {
                result: res,
                latency_ms,
            }));
        }
        other => send(SettingsEvent::Done {
            ok: String::new(),
            result: Err(format!("the gateway is not running ({other:?})")),
        }),
    }
}
