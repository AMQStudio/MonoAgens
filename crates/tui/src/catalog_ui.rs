//! `/model`, `/provider` and `/usage`: shared state, data types and the gateway round trips.
//!
//! The reducers stay synchronous like the rest of the TUI: a key press returns
//! [`AppAction::Catalog`] values, the runtime runs them with [`run_action`] against the
//! [`GatewayClient`] (the same calls for the in-process and the socket client) and the answers come
//! back as [`RuntimeEvent::Catalog`] holding a [`CatalogEvent`], which [`CatalogUi::apply`] folds
//! into the state. The screens themselves live in `model_picker`, `login` and `usage`.
//!
//! Provider labels, key state and model lists all come from the gateway's `providers.list` and
//! `models.discover`; nothing here names a provider.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use unicode_width::UnicodeWidthStr;

use crate::client::{GatewayClient, RuntimeEvent};
use crate::config::IconMode;
use crate::login::LoginView;
use crate::model_picker::ModelPicker;
use crate::state::AppAction;
use crate::theme::ThemePalette;
use crate::usage::UsageView;
use crate::width::{chrome_label, fit_prefix, sanitize};

/// A secret typed by the user. `Debug` never prints it, so actions and events can be logged.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret text (only for the request that stores or tests it).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// A model chosen by the user, applied after an optional key detour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub provider: String,
    pub model: String,
    /// The provider differs from the current one, so the base URL override must be dropped.
    pub provider_changed: bool,
}

/// Requests to the gateway made by the `/model`, `/provider` and `/usage` screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogAction {
    /// `providers.list`, then every configured provider's cached model list, then a background
    /// refresh of the current provider's list.
    LoadProviders,
    /// `models.discover` for one provider.
    Discover { provider: String, refresh: bool },
    /// Connection test; on success the key is saved and `then` (if any) is selected.
    TestKey {
        provider: String,
        key: Secret,
        then: Option<ModelChoice>,
    },
    /// Saves the key without testing it first, then selects `then` (if any).
    SaveKey {
        provider: String,
        key: Secret,
        then: Option<ModelChoice>,
    },
    /// Adds a named custom provider (`[providers.custom.<name>]` plus its optional key).
    AddCustom {
        name: String,
        base_url: String,
        messages_api: bool,
        key: Secret,
    },
    /// Switches the live model through `config.apply`.
    SelectModel(ModelChoice),
    /// `usage.get`.
    LoadUsage,
}

/// Answers of the gateway, folded into the state by [`CatalogUi::apply`].
#[derive(Debug)]
pub enum CatalogEvent {
    Providers(Result<Value, String>),
    Models {
        provider: String,
        refresh: bool,
        result: Result<Value, String>,
    },
    KeyTested {
        provider: String,
        result: Result<Value, String>,
    },
    KeySaved {
        provider: String,
        result: Result<(), String>,
    },
    ModelSelected {
        choice: ModelChoice,
        result: Result<(), String>,
    },
    CustomAdded {
        name: String,
        result: Result<(), String>,
    },
    Usage(Result<Value, String>),
}

/// Which overlay is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Closed,
    Picker,
    LoginList,
    KeyEntry,
    CustomForm,
    Usage,
}

/// One provider from `providers.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    pub id: String,
    pub label: String,
    pub keyless: bool,
    pub key_set: bool,
    pub key_source: Option<String>,
    pub discovery: bool,
    /// Own key set, or a local keyless provider the owner enabled.
    pub logged_in: bool,
    /// A named custom provider (`[providers.custom.<name>]`).
    pub custom: bool,
    pub base_url: Option<String>,
    /// A local keyless provider listed in `[providers] enabled`.
    pub enabled: bool,
}

impl ProviderRow {
    /// Usable without asking for a key.
    #[must_use]
    pub fn configured(&self) -> bool {
        self.logged_in
    }
}

/// One model from `models.discover`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    pub id: String,
    pub display_name: Option<String>,
    pub context_length: Option<u64>,
    pub price_in_per_mtok: Option<f64>,
    pub price_out_per_mtok: Option<f64>,
}

/// The model list of one provider as last seen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelsEntry {
    pub models: Vec<ModelRow>,
    pub fetched_at_ms: u64,
    pub stale: bool,
    pub refreshing: bool,
    pub refresh_requested: bool,
    pub error: Option<String>,
}

/// State of the three overlays plus the provider and model data they share.
#[derive(Debug, Clone, Default)]
pub struct CatalogUi {
    pub screen: Screen,
    pub providers: Vec<ProviderRow>,
    pub providers_loaded: bool,
    pub models: BTreeMap<String, ModelsEntry>,
    pub current_provider: Option<String>,
    pub current_model: String,
    pub picker: ModelPicker,
    pub login: LoginView,
    pub custom: crate::login::CustomForm,
    pub usage: UsageView,
    /// Rows available to lists (kept in sync with the terminal size).
    pub view_rows: usize,
    pub(crate) return_to: Screen,
    toast: Option<(String, bool)>,
}

/// Rows of chrome around the lists (title, search or header, detail, footer).
pub const CHROME_ROWS: u16 = 4;

impl CatalogUi {
    /// An overlay is showing and owns the keyboard.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.screen != Screen::Closed
    }

    /// Rows a list can show (never zero).
    #[must_use]
    pub fn rows(&self) -> usize {
        if self.view_rows == 0 { 10 } else { self.view_rows }
    }

    /// Provider row by id.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&ProviderRow> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Display label of provider `id` (the id itself until the list is loaded).
    #[must_use]
    pub fn label_of(&self, id: &str) -> String {
        self.provider(id).map_or_else(|| id.to_owned(), |p| p.label.clone())
    }

    /// Short key state of `current` for the `/provider` menu row.
    #[must_use]
    pub fn login_status(&self, current: Option<&str>) -> Option<String> {
        let row = self.provider(current?)?;
        Some(
            if row.keyless && !row.key_set {
                if row.logged_in { "enabled" } else { "not enabled" }
            } else if row.key_set {
                "key set"
            } else {
                "no key"
            }
            .to_owned(),
        )
    }

    /// Takes the pending toast `(text, is_error)`.
    pub fn take_toast(&mut self) -> Option<(String, bool)> {
        self.toast.take()
    }

    fn set_toast(&mut self, text: impl Into<String>, is_error: bool) {
        self.toast = Some((text.into(), is_error));
    }

    pub(crate) fn set_toast_internal(&mut self, text: &str, is_error: bool) {
        self.set_toast(text, is_error);
    }

    /// `/usage`: opens the view and asks for the numbers.
    pub fn open_usage(&mut self) -> Vec<AppAction> {
        self.screen = Screen::Usage;
        self.usage = UsageView::default();
        vec![AppAction::Catalog(CatalogAction::LoadUsage)]
    }

    /// Routes a key to the open overlay.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        match self.screen {
            Screen::Closed => Vec::new(),
            Screen::Picker => self.picker_key(key),
            Screen::LoginList => self.login_list_key(key),
            Screen::KeyEntry => self.key_entry_key(key),
            Screen::CustomForm => self.custom_form_key(key),
            Screen::Usage => self.usage_key(key),
        }
    }

    /// Pasted text: only the key entry takes it (masked, single line).
    pub fn handle_paste(&mut self, raw: &str) {
        match self.screen {
            Screen::KeyEntry => self.key_entry_paste(raw),
            Screen::CustomForm => self.custom_form_paste(raw),
            Screen::Picker => {
                for c in raw.chars().filter(|c| !c.is_control()) {
                    self.picker.search.push(c);
                }
                self.picker.model_sel = 0;
                self.picker.model_off = 0;
            }
            Screen::LoginList => {
                for c in raw.chars().filter(|c| !c.is_control()) {
                    self.login.search.push(c);
                }
                self.login.sel = 0;
                self.login.off = 0;
            }
            _ => {}
        }
    }

    /// Mouse wheel.
    pub fn scroll_by(&mut self, delta: isize) {
        let key = if delta < 0 { KeyCode::Up } else { KeyCode::Down };
        for _ in 0..delta.unsigned_abs() {
            let _ = self.handle_key(KeyEvent::from(key));
        }
    }

    /// Folds a gateway answer into the state; returns the model switch when one completed.
    pub fn apply(&mut self, ev: CatalogEvent) -> Option<ModelChoice> {
        match ev {
            CatalogEvent::Providers(Ok(v)) => {
                self.providers = parse_providers(&v);
                self.providers_loaded = true;
                if self.current_provider.is_none() {
                    self.current_provider = v
                        .pointer("/current/provider")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                self.on_providers_loaded();
            }
            CatalogEvent::Providers(Err(e)) => self.set_toast(format!("Could not list providers: {e}"), true),
            CatalogEvent::Models {
                provider,
                refresh,
                result,
            } => self.apply_models(&provider, refresh, result),
            CatalogEvent::KeyTested { provider, result } => self.on_key_tested(&provider, result),
            CatalogEvent::KeySaved { provider, result } => self.on_key_saved(&provider, result),
            CatalogEvent::ModelSelected { choice, result } => {
                return match result {
                    Ok(()) => {
                        self.current_provider = Some(choice.provider.clone());
                        self.current_model.clone_from(&choice.model);
                        self.screen = Screen::Closed;
                        let label = self.label_of(&choice.provider);
                        self.set_toast(format!("Model set to {label} / {}", choice.model), false);
                        Some(choice)
                    }
                    Err(e) => {
                        self.set_toast(format!("Could not switch model: {e}"), true);
                        None
                    }
                };
            }
            CatalogEvent::CustomAdded { name, result } => self.on_custom_added(&name, result),
            CatalogEvent::Usage(res) => self.usage.on_loaded(res),
        }
        None
    }

    fn apply_models(&mut self, provider: &str, refresh: bool, result: Result<Value, String>) {
        let entry = self.models.entry(provider.to_owned()).or_default();
        if refresh {
            entry.refreshing = false;
        }
        match result {
            Ok(v) => {
                let fetched = v.get("fetched_at_ms").and_then(Value::as_u64).unwrap_or(0);
                if !refresh && entry.fetched_at_ms > fetched {
                    return; // a fresher list already arrived
                }
                entry.models = parse_models(&v);
                entry.fetched_at_ms = fetched;
                entry.stale = v.get("stale").and_then(Value::as_bool).unwrap_or(false);
                entry.error = v.get("error").and_then(Value::as_str).map(str::to_owned);
            }
            Err(e) => entry.error = Some(e),
        }
        self.clamp_picker();
    }
}

/// `providers.list` -> rows.
#[must_use]
pub fn parse_providers(v: &Value) -> Vec<ProviderRow> {
    v.get("providers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(ProviderRow {
                        id: p.get("id")?.as_str()?.to_owned(),
                        label: p
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        keyless: p.get("keyless").and_then(Value::as_bool).unwrap_or(false),
                        key_set: p.get("key_set").and_then(Value::as_bool).unwrap_or(false),
                        key_source: p.get("key_source").and_then(Value::as_str).map(str::to_owned),
                        discovery: p.get("discovery").and_then(Value::as_bool).unwrap_or(true),
                        logged_in: p.get("logged_in").and_then(Value::as_bool).unwrap_or_else(|| {
                            p.get("key_set").and_then(Value::as_bool).unwrap_or(false)
                        }),
                        custom: p.get("custom").and_then(Value::as_bool).unwrap_or(false),
                        base_url: p.get("base_url").and_then(Value::as_str).map(str::to_owned),
                        enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `models.discover` -> rows.
#[must_use]
pub fn parse_models(v: &Value) -> Vec<ModelRow> {
    v.get("models")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(ModelRow {
                        id: m.get("id")?.as_str()?.to_owned(),
                        display_name: m.get("display_name").and_then(Value::as_str).map(str::to_owned),
                        context_length: m.get("context_length").and_then(Value::as_u64),
                        price_in_per_mtok: m.get("price_in_per_mtok").and_then(Value::as_f64),
                        price_out_per_mtok: m.get("price_out_per_mtok").and_then(Value::as_f64),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `config.apply` patch for switching to `choice`.
#[must_use]
pub fn model_patch(choice: &ModelChoice) -> Value {
    let mut model = json!({ "provider": choice.provider, "model": choice.model });
    if choice.provider_changed {
        // A base URL override belongs to the previous provider.
        model["provider_options"] = json!({ "base_url": null });
    }
    json!({ "model": model })
}

async fn select_model(
    client: &Arc<dyn GatewayClient>,
    tx: &UnboundedSender<RuntimeEvent>,
    choice: ModelChoice,
) {
    let result = client
        .config_apply(model_patch(&choice))
        .await
        .map(|_| ())
        .map_err(|e| e.message);
    let _ = tx.send(RuntimeEvent::Catalog(CatalogEvent::ModelSelected { choice, result }));
}

async fn save_key(
    client: &Arc<dyn GatewayClient>,
    tx: &UnboundedSender<RuntimeEvent>,
    provider: String,
    key: &str,
    then: Option<ModelChoice>,
) {
    let result = if key.is_empty() {
        // A keyless local provider is "logged in" once the owner enables it.
        enable_keyless(client, &provider).await
    } else {
        client
            .secrets_set(provider.clone(), key.to_owned())
            .await
            .map(|_| ())
            .map_err(|e| e.message)
    };
    let saved = result.is_ok();
    let _ = tx.send(RuntimeEvent::Catalog(CatalogEvent::KeySaved {
        provider,
        result,
    }));
    if saved && let Some(choice) = then {
        select_model(client, tx, choice).await;
    }
}

/// Adds `provider` to `[providers] enabled` (keeping the ones already enabled).
async fn enable_keyless(client: &Arc<dyn GatewayClient>, provider: &str) -> Result<(), String> {
    let list = client.providers_list().await.map_err(|e| e.message)?;
    let mut enabled: Vec<String> = parse_providers(&list)
        .into_iter()
        .filter(|r| r.enabled)
        .map(|r| r.id)
        .collect();
    if !enabled.iter().any(|e| e == provider) {
        enabled.push(provider.to_owned());
    }
    client
        .config_apply(json!({ "providers": { "enabled": enabled } }))
        .await
        .map(|_| ())
        .map_err(|e| e.message)
}

/// The `config.apply` patch for a named custom provider.
#[must_use]
pub fn custom_provider_patch(name: &str, base_url: &str, messages_api: bool, keyless: bool) -> Value {
    let mut entry = json!({
        "base_url": base_url,
        "dialect": if messages_api { "messages_api" } else { "openai_chat" },
    });
    if keyless {
        entry["keyless"] = json!(true);
    }
    json!({ "providers": { "custom": { name: entry } } })
}

/// Runs one request against the gateway and reports back through `tx`.
pub async fn run_action(
    action: CatalogAction,
    client: Arc<dyn GatewayClient>,
    tx: UnboundedSender<RuntimeEvent>,
) {
    let send = |ev: CatalogEvent| {
        let _ = tx.send(RuntimeEvent::Catalog(ev));
    };
    match action {
        CatalogAction::LoadProviders => match client.providers_list().await {
            Ok(v) => {
                let rows = parse_providers(&v);
                let current = v
                    .pointer("/current/provider")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                send(CatalogEvent::Providers(Ok(v)));
                // Cache first for every usable provider (instant), then a background refresh
                // of the current one.
                for r in rows.iter().filter(|r| r.configured() && r.discovery) {
                    let result = client
                        .models_discover(r.id.clone(), false)
                        .await
                        .map_err(|e| e.message);
                    send(CatalogEvent::Models {
                        provider: r.id.clone(),
                        refresh: false,
                        result,
                    });
                }
                if let Some(cur) = current
                    && rows.iter().any(|r| r.id == cur && r.configured() && r.discovery)
                {
                    let result = client
                        .models_discover(cur.clone(), true)
                        .await
                        .map_err(|e| e.message);
                    send(CatalogEvent::Models {
                        provider: cur,
                        refresh: true,
                        result,
                    });
                }
            }
            Err(e) => send(CatalogEvent::Providers(Err(e.message))),
        },
        CatalogAction::Discover { provider, refresh } => {
            let result = client
                .models_discover(provider.clone(), refresh)
                .await
                .map_err(|e| e.message);
            send(CatalogEvent::Models {
                provider,
                refresh,
                result,
            });
        }
        CatalogAction::TestKey { provider, key, then } => {
            let k = key.expose().trim().to_owned();
            let arg = (!k.is_empty()).then(|| k.clone());
            let result = client
                .providers_test(provider.clone(), arg)
                .await
                .map_err(|e| e.message);
            let ok = result
                .as_ref()
                .is_ok_and(|v| v.get("ok").and_then(Value::as_bool) == Some(true));
            send(CatalogEvent::KeyTested {
                provider: provider.clone(),
                result,
            });
            if ok {
                save_key(&client, &tx, provider, &k, then).await;
            }
        }
        CatalogAction::SaveKey { provider, key, then } => {
            let k = key.expose().trim().to_owned();
            save_key(&client, &tx, provider, &k, then).await;
        }
        CatalogAction::AddCustom { name, base_url, messages_api, key } => {
            let k = key.expose().trim().to_owned();
            let patch = custom_provider_patch(&name, &base_url, messages_api, k.is_empty());
            let mut result = client.config_apply(patch).await.map(|_| ()).map_err(|e| e.message);
            if result.is_ok() && !k.is_empty() {
                result = client
                    .secrets_set(name.clone(), k)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.message);
            }
            let ok = result.is_ok();
            send(CatalogEvent::CustomAdded { name, result });
            if ok && let Ok(v) = client.providers_list().await {
                send(CatalogEvent::Providers(Ok(v)));
            }
        }
        CatalogAction::SelectModel(choice) => select_model(&client, &tx, choice).await,
        CatalogAction::LoadUsage => {
            let res = client.usage_get().await.map_err(|e| e.message);
            send(CatalogEvent::Usage(res));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Provider failure notices (4.3)
// ---------------------------------------------------------------------------------------------

/// Turns a model-call failure recorded by the gateway into a notice that names the cause, shows
/// the provider's own message and points at `/log`. `None` when `component` / `detail` is not a
/// classified provider failure (the caller keeps its generic text).
#[must_use]
pub fn provider_failure_notice(component: &str, detail: &str) -> Option<String> {
    if component != "model" {
        return None;
    }
    let (class, tail) = if let Some(rest) = detail.strip_prefix("model call failed (") {
        let (head, tail) = rest.split_once("): ")?;
        (head.split(',').next()?.trim().to_owned(), tail.to_owned())
    } else if detail.starts_with("model call timed out") {
        ("timeout".to_owned(), detail.to_owned())
    } else {
        return None;
    };
    let (status, body) = match tail.strip_prefix("HTTP ") {
        Some(rest) => {
            let (code, body) = rest.split_once(": ").unwrap_or((rest, ""));
            (Some(code.trim().to_owned()), body.to_owned())
        }
        None => (None, tail),
    };
    let message = provider_message(&body);
    let code = status.map(|c| format!(" (HTTP {c})")).unwrap_or_default();
    let message = if message.is_empty() {
        String::new()
    } else {
        format!(": {message}")
    };
    Some(match class.as_str() {
        "rate_limit" => format!("Rate limited by the provider{code}{message}. See /log."),
        "authentication" => {
            format!("The provider rejected the API key{code}{message}. See /log, or run /provider to set a key.")
        }
        "timeout" => format!("The provider timed out{code}{message}. See /log."),
        other => format!("Model call failed ({other}){code}{message}. See /log."),
    })
}

/// The provider's own message: `error.message` / `message` / `detail` of a JSON body, else the
/// first 300 characters of the text.
fn provider_message(body: &str) -> String {
    let body = body.trim();
    let from_json = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        [
            v.pointer("/error/message"),
            v.get("message"),
            v.get("detail"),
            v.get("error"),
        ]
        .into_iter()
        .flatten()
        .find_map(|m| m.as_str().map(str::to_owned))
    });
    let text = from_json.unwrap_or_else(|| body.to_owned());
    let text = sanitize(&text, false);
    let cut: String = text.chars().take(300).collect();
    cut.trim().to_owned()
}

// ---------------------------------------------------------------------------------------------
// Rendering shared by the screens
// ---------------------------------------------------------------------------------------------

/// Draws the open overlay over the whole `area`.
pub fn render_overlay(
    area: Rect,
    buf: &mut Buffer,
    ui: &CatalogUi,
    palette: &ThemePalette,
    icons: IconMode,
) {
    if area.width < 20 || area.height < CHROME_ROWS + 2 {
        return;
    }
    let bg = Style::default().fg(palette.fg).bg(palette.bg);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(bg);
            }
        }
    }
    match ui.screen {
        Screen::Closed => {}
        Screen::Picker => crate::model_picker::render(area, buf, ui, palette, icons),
        Screen::LoginList => crate::login::render_list(area, buf, ui, palette, icons),
        Screen::KeyEntry => crate::login::render_key_entry(area, buf, ui, palette, icons),
        Screen::CustomForm => crate::login::render_custom_form(area, buf, ui, palette, icons),
        Screen::Usage => crate::usage::render(area, buf, ui, palette, icons),
    }
}

/// Fits `s` into exactly `width` columns (truncating or padding with spaces).
#[must_use]
pub fn pad(s: &str, width: usize) -> String {
    let fitted = fit_prefix(s, width);
    let used = fitted.width();
    format!("{fitted}{}", " ".repeat(width.saturating_sub(used)))
}

/// Writes `text` (chrome-normalised, fitted) at `(x, y)` over `width` columns.
pub(crate) fn put(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: usize,
    text: &str,
    style: Style,
    icons: IconMode,
) {
    if width == 0 {
        return;
    }
    buf.set_string(x, y, pad(&chrome_label(text, icons), width), style);
}

/// Title bar.
pub(crate) fn title_bar(area: Rect, buf: &mut Buffer, text: &str, palette: &ThemePalette, icons: IconMode) {
    put(
        buf,
        area.x,
        area.y,
        area.width as usize,
        text,
        Style::default()
            .fg(palette.accent)
            .bg(palette.status_bg)
            .add_modifier(Modifier::BOLD),
        icons,
    );
}

/// Footer with the key hints.
pub(crate) fn footer_bar(area: Rect, buf: &mut Buffer, text: &str, palette: &ThemePalette, icons: IconMode) {
    put(
        buf,
        area.x,
        area.y + area.height - 1,
        area.width as usize,
        text,
        Style::default().fg(palette.dim).bg(palette.status_bg),
        icons,
    );
}

/// Compact context size: `8k`, `128k`, `1.0M`.
#[must_use]
pub fn fmt_context(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// USD per million tokens: `$0.5`, `$12`.
#[must_use]
pub fn fmt_price(p: f64) -> String {
    if p >= 10.0 {
        format!("${p:.0}")
    } else {
        format!("${}", format!("{p:.2}").trim_end_matches('0').trim_end_matches('.'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_notice_has_message_and_log_pointer() {
        let detail = "model call failed (rate_limit, model 'm', latency 5ms): HTTP 429: {\"error\":{\"message\":\"slow down please\"}}";
        let n = provider_failure_notice("model", detail).expect("notice");
        assert!(n.contains("Rate limited"), "{n}");
        assert!(n.contains("429"), "{n}");
        assert!(n.contains("slow down please"), "{n}");
        assert!(n.contains("See /log"), "{n}");
    }

    #[test]
    fn auth_and_timeout_notices() {
        let a = provider_failure_notice(
            "model",
            "model call failed (authentication, model 'm', latency 5ms): HTTP 401: bad key",
        )
        .unwrap();
        assert!(a.contains("rejected the API key") && a.contains("/provider") && a.contains("bad key"));
        let t = provider_failure_notice(
            "model",
            "model call timed out after 100ms (model 'm', purpose Reason)",
        )
        .unwrap();
        assert!(t.contains("timed out") && t.contains("See /log"));
        assert_eq!(provider_failure_notice("core", "x"), None);
        assert_eq!(provider_failure_notice("model", "cancelled_by_owner"), None);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let a = CatalogAction::SaveKey {
            provider: "p".into(),
            key: Secret::new("sk-very-secret"),
            then: None,
        };
        assert!(!format!("{a:?}").contains("sk-very-secret"));
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(fmt_context(128_000), "128k");
        assert_eq!(fmt_context(1_000_000), "1.0M");
        assert_eq!(fmt_price(0.5), "$0.5");
        assert_eq!(fmt_price(12.0), "$12");
    }
}

#[cfg(test)]
mod gateway_tests {
    //! Round trips through a hosted gateway (in-process client) against a fake local provider.
    use super::*;
    use crate::client::ClientKind;
    use crate::runtime::{TuiLaunchOptions, boot_gateway};
    use monoagens_gateway::{GatewayConfig, LoadedGatewayEnv, ResolvedPaths, parse_config_str};
    use monoagens_reason::ProviderSecrets;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn serve() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    let mut len = 0usize;
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        let h = h.trim_end().to_ascii_lowercase();
                        if h.is_empty() {
                            break;
                        }
                        if let Some(v) = h.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; len];
                    r.read_exact(&mut body).unwrap();
                    let reply = r#"{"data":[{"id":"fake-a"},{"id":"qwen-fake"}]}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn login_test_saves_the_key_0600_and_models_discover_works_in_process() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-unit-tests");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::TempDir::new_in(base).unwrap();
        let cfg_dir = dir.path().join("config");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let url = serve();
        let text = format!(
            "version = 1\n[model]\nprovider = \"ollama\"\nmodel = \"fake-a\"\n[model.provider_options]\nbase_url = \"{url}/v1\"\n"
        );
        let paths = ResolvedPaths {
            config_path: cfg_dir.join("monoagens.toml"),
            secrets_path: cfg_dir.join("secrets.toml"),
            data_dir: data_dir.clone(),
        };
        std::fs::write(&paths.config_path, &text).unwrap();
        let config: GatewayConfig = parse_config_str(&text, &paths.config_path, &paths.secrets_path).unwrap();
        let loaded = LoadedGatewayEnv {
            config,
            secrets: ProviderSecrets::default(),
            paths: paths.clone(),
            created_default: false,
            migration: None,
        };
        let options = TuiLaunchOptions {
            data_dir: Some(data_dir),
            ..TuiLaunchOptions::default()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = boot_gateway(&loaded, &options, None, tx.clone()).await.expect("host");
        assert_eq!(client.kind(), ClientKind::Hosting);

        // Test + save in one action, as the key page does.
        run_action(
            CatalogAction::TestKey {
                provider: "ollama".into(),
                key: Secret::new("sk-test-key"),
                then: Some(ModelChoice {
                    provider: "ollama".into(),
                    model: "qwen-fake".into(),
                    provider_changed: false,
                }),
            },
            Arc::clone(&client),
            tx.clone(),
        )
        .await;
        let mut tested = None;
        let mut selected = None;
        let mut saved = false;
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            if let RuntimeEvent::Catalog(ev) = ev {
                match ev {
                    CatalogEvent::KeyTested { result, .. } => tested = Some(result.unwrap()),
                    CatalogEvent::KeySaved { result, .. } => saved = result.is_ok(),
                    CatalogEvent::ModelSelected { choice, result } => {
                        result.clone().unwrap();
                        selected = Some(choice);
                    }
                    _ => {}
                }
            }
        }
        let t = tested.expect("test result");
        assert_eq!(t["ok"], json!(true));
        assert_eq!(t["model_count"], json!(2));
        assert!(t["latency_ms"].is_u64());
        assert!(saved);
        assert_eq!(selected.expect("model selected").model, "qwen-fake");
        let mode = std::fs::metadata(&paths.secrets_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let raw = std::fs::read_to_string(&paths.secrets_path).unwrap();
        assert!(raw.contains("[providers.ollama]") && raw.contains("sk-test-key"));
        assert!(std::fs::read_to_string(&paths.config_path).unwrap().contains("qwen-fake"));

        // models.discover through the client trait: cache first (empty), then fresh.
        let cached = client.models_discover("ollama".into(), false).await.unwrap();
        assert_eq!(cached["models"], json!([]));
        let fresh = client.models_discover("ollama".into(), true).await.unwrap();
        assert_eq!(fresh["models"].as_array().unwrap().len(), 2);
        let cached = client.models_discover("ollama".into(), false).await.unwrap();
        assert_eq!(cached["models"].as_array().unwrap().len(), 2);
        let list = client.providers_list().await.unwrap();
        assert!(!list.to_string().contains("sk-test-key"));
        let usage = client.usage_get().await.unwrap();
        assert!(usage["purposes"].as_array().unwrap().is_empty());
        client.shutdown().await;
    }
}
