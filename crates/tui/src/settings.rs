//! 8-tab 2-pane `/settings` overlay model, plugin drill-down state, and first-run Setup Wizard.

use std::path::PathBuf;

use monoagens_gateway::{GatewayConfig, PluginKind, ResolvedPaths};

use crate::config::{DynamicTomlDoc, DynamicTomlEntry, TuiPreferences};
use crate::list_editor::ItemEditor;
use crate::pairing::PairingState;
use crate::rbac_editor::RbacPage;
use crate::selector::Selector;

/// All 8 tabs in the `/settings` modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Appearance,
    SoulBrain,
    Model,
    Valve,
    Rbac,
    Providers,
    Plugins,
    Runtime,
}

impl SettingsTab {
    /// Ordered tabs matching the `/settings` tab strip.
    pub const ALL: [Self; 8] = [
        Self::Appearance,
        Self::SoulBrain,
        Self::Model,
        Self::Valve,
        Self::Rbac,
        Self::Providers,
        Self::Plugins,
        Self::Runtime,
    ];

    /// Human-readable tab label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::SoulBrain => "Soul & Brain",
            Self::Model => "Model",
            Self::Valve => "Valve",
            Self::Rbac => "RBAC",
            Self::Providers => "Providers",
            Self::Plugins => "Plugins",
            Self::Runtime => "Runtime",
        }
    }

    /// Expressive tab icon shown in `/settings`.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Appearance => "🎨",
            Self::SoulBrain => "🧠",
            Self::Model => "🤖",
            Self::Valve => "🛡️",
            Self::Rbac => "🔐",
            Self::Providers => "🔑",
            Self::Plugins => "🔌",
            Self::Runtime => "⚡",
        }
    }

    /// Next tab (wraps around).
    #[must_use]
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Previous tab (wraps around).
    #[must_use]
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// Left-pane section anchors for this tab.
    #[must_use]
    pub const fn sections(self) -> &'static [&'static str] {
        match self {
            Self::Appearance => &["Theme & Layout", "Motion & Shimmer", "Chat & Status Line", "Keybindings"],
            Self::SoulBrain => &["Soul Identity", "Neural Core", "Embeddings"],
            Self::Model => &[
                "Provider & Model",
                "Output Limits",
                "Budget & Headers",
                "Usage Paths & Prices",
                "Per-purpose Models",
            ],
            Self::Valve => &["Enforcement", "Rule Toggles"],
            Self::Rbac => &["Fallback & Roles", "Actors", "Places & Bases"],
            Self::Providers => &["Providers", "Legacy Endpoint & Key", "Plugin [env] Secrets"],
            Self::Plugins => &["Configured & Catalog"],
            Self::Runtime => &["Timeouts & Cues", "Identity Linking", "Storage", "Resolved Paths"],
        }
    }
}

/// Live traffic-light plugin lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginUiState {
    Running,
    Starting,
    Stopping,
    Stopped,
    Failed(String),
    /// Another MonoAgens process holds the plugin lock; nothing is spawned here.
    RunningElsewhere,
    /// The adapter cannot start yet; the text is what it needs (`needs pairing`,
    /// `needs TELEGRAM_BOT_TOKEN`).
    NeedsSetup(String),
}

impl PluginUiState {
    /// Human-readable label for status columns.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Starting => "starting",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed(_) => "failed",
            Self::RunningElsewhere => "running in another MonoAgens",
            Self::NeedsSetup(what) => what,
        }
    }

    /// The action that fixes a `NeedsSetup` state.
    #[must_use]
    pub fn fix_hint(&self) -> Option<&'static str> {
        match self {
            Self::NeedsSetup(what) if what.contains("pairing") => Some("use [ Pair device ] below"),
            Self::NeedsSetup(_) => Some("set the missing secret below"),
            _ => None,
        }
    }

    /// Dot glyph (`●` for steady states, `◐` for transitions).
    #[must_use]
    pub const fn dot_glyph(&self) -> &'static str {
        match self {
            Self::Starting | Self::Stopping => "◐",
            Self::Running | Self::Stopped | Self::Failed(_) => "●",
            Self::RunningElsewhere => "◉",
            Self::NeedsSetup(_) => "◌",
        }
    }
}

/// UI metadata for a configured or built-in catalog plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUiEntry {
    pub id: String,
    pub kind: PluginKind,
    pub enabled: bool,
    pub installed: bool,
    pub state: PluginUiState,
    pub command: Option<String>,
    pub socket_path: Option<String>,
    pub config_path: PathBuf,
    pub state_dir: PathBuf,
    pub dependencies: Vec<String>,
    pub optional_dependencies: Vec<String>,
}

/// Valve per-rule toggles displayed in `/settings -> Valve` and persisted under `[valve]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValveRuleTogglesState {
    pub cedar_abac: bool,
    pub line2_scope: bool,
    pub taint_blocking: bool,
    pub pending_approval: bool,
    pub budget_guard: bool,
    pub chain_brake: bool,
    pub quiet_hours: bool,
    pub proactive_cap: bool,
}

impl Default for ValveRuleTogglesState {
    fn default() -> Self {
        Self {
            cedar_abac: true,
            line2_scope: true,
            taint_blocking: true,
            pending_approval: true,
            budget_guard: true,
            chain_brake: true,
            quiet_hours: false,
            proactive_cap: true,
        }
    }
}

impl ValveRuleTogglesState {
    /// Reads optional boolean rule toggles under `[valve]` from `monoagens.toml`.
    #[must_use]
    pub fn load_from_config(config_path: &std::path::Path) -> Self {
        let mut out = Self::default();
        let Ok(raw) = std::fs::read_to_string(config_path) else {
            return out;
        };
        let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
            return out;
        };
        let Some(tbl) = doc.get("valve").and_then(toml_edit::Item::as_table_like) else {
            return out;
        };
        let read = |k: &str, default: bool| {
            tbl.get(k)
                .and_then(toml_edit::Item::as_bool)
                .unwrap_or(default)
        };
        out.cedar_abac = read("cedar_abac", out.cedar_abac);
        out.line2_scope = read("line2_scope", out.line2_scope);
        out.taint_blocking = read("taint_blocking", out.taint_blocking);
        out.pending_approval = read("pending_approval", out.pending_approval);
        out.budget_guard = read("budget_guard", out.budget_guard);
        out.chain_brake = read("chain_brake", out.chain_brake);
        out.quiet_hours = read("quiet_hours", out.quiet_hours);
        out.proactive_cap = read("proactive_cap", out.proactive_cap);
        out
    }
}

/// Non-secret summary of `secrets.toml` (never retains plaintext secret values).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretsUiSummary {
    pub model_api_key_set: bool,
    pub model_endpoint: Option<String>,
    pub top_level_keys: Vec<String>,
    pub env_keys: Vec<String>,
}

/// Value presentation kind for a `/settings` option row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValueKind {
    Bool(bool),
    /// Enum: Enter opens the selector; the string is the current label.
    Select(String),
    Text(String),
    Number(String),
    /// Filesystem path (validated on save).
    Path(String),
    MaskedSecret { is_set: bool },
    List(Vec<String>),
    Map(Vec<(String, String)>),
    Action(String),
    ReadOnly(String),
}

/// Single option row inside the `/settings` right-hand pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub id: String,
    pub section: &'static str,
    pub label: String,
    pub description: String,
    pub value: SettingValueKind,
}

/// Target of an active inline text/secret prompt inside `/settings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineEditTarget {
    SettingField { id: String },
    NewEnvKeyName,
    NewEnvSecretValue { env_key: String },
    PluginTomlValue { plugin_id: String, key_path: String },
    PluginTomlNewKeyName { plugin_id: String },
    PluginTomlNewKeyValue { plugin_id: String, key_path: String },
}

/// Active inline editing state inside `/settings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineEditState {
    pub target: InlineEditTarget,
    pub prompt: String,
    pub buffer: String,
    pub masked: bool,
}

/// Level 2 plugin detail drill-down (`Plugins › <id>`).
#[derive(Debug, Clone)]
pub struct PluginDetailState {
    pub plugin_id: String,
    pub selected_row: usize,
    pub scroll_offset: usize,
    pub toml_path: PathBuf,
    pub toml_doc: Option<DynamicTomlDoc>,
    pub toml_entries: Vec<DynamicTomlEntry>,
    pub show_env_manager: bool,
}

impl PluginDetailState {
    /// Fixed action rows at the top of `Plugins › <id>`.
    pub const ACTION_COUNT: usize = 5;

    /// Total selectable rows across action items, dynamic TOML entries, and `[ + Add TOML Key ]`.
    #[must_use]
    pub fn total_rows(&self) -> usize {
        Self::ACTION_COUNT + self.toml_entries.len() + 1
    }

    /// Reloads `<config_dir>/plugins/<id>.toml` from disk.
    pub fn reload_toml(&mut self) {
        if let Ok(doc) = DynamicTomlDoc::load_or_init(&self.toml_path) {
            self.toml_entries = doc.entries();
            self.toml_doc = Some(doc);
        } else {
            self.toml_entries.clear();
            self.toml_doc = None;
        }
    }
}

/// A yes/no question shown over `/settings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmState {
    pub prompt: String,
    pub action: ConfirmAction,
}

/// What a confirmed question does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    RemovePlugin(String),
    DeleteRole(String),
    DeletePlace(String),
    DeleteBase(String),
    RemoveActor(String),
}

/// Guided install of a catalog plugin (command path, kind, enable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallFlow {
    pub plugin_id: String,
    pub kind: PluginKind,
    /// Binary found on disk or on `PATH`.
    pub command: Option<String>,
    /// Name of the binary that was looked for (shown with the build hint).
    pub binary: String,
    pub enable: bool,
    pub hint: String,
}

/// Known ids for the RBAC pickers, refreshed from the gateway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RbacHints {
    /// Capability ids of connected bodies (`capabilities.list`).
    pub capabilities: Vec<String>,
    /// Actors seen in the event log (`events.query`).
    pub actors: Vec<String>,
}

/// Complete state of the `/settings` 8-tab 2-pane modal.
#[derive(Debug, Clone, Default)]
pub struct SettingsModalState {
    pub open: bool,
    pub tab: SettingsTab,
    pub section_index: usize,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub search_query: String,
    pub editing: Option<InlineEditState>,
    pub plugin_detail: Option<PluginDetailState>,
    pub valve_toggles: ValveRuleTogglesState,
    /// Enum selector over the list.
    pub selector: Option<Selector>,
    /// List / map editor over the list.
    pub items: Option<ItemEditor>,
    /// Entries of the map being edited as last saved (to null out removed keys).
    pub items_base: Vec<(String, String)>,
    /// RBAC sub-page (role, actor, place, permission base) in place of the list.
    pub rbac_page: Option<RbacPage>,
    /// Main-list selection to restore when the RBAC sub-page closes.
    pub page_return: Option<(usize, usize)>,
    pub pairing: Option<PairingState>,
    pub confirm: Option<ConfirmState>,
    pub install: Option<InstallFlow>,
    pub hints: RbacHints,
    /// Rows the list showed in the last frame (written by the renderer).
    pub list_rows: std::cell::Cell<usize>,
}

impl SettingsModalState {
    /// Opens the modal on `tab`.
    pub fn open_tab(&mut self, tab: SettingsTab) {
        self.open = true;
        self.tab = tab;
        self.section_index = 0;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.search_query.clear();
        self.close_subviews();
    }

    /// Closes every editor, sub-page and dialog opened over the list.
    pub fn close_subviews(&mut self) {
        self.editing = None;
        self.plugin_detail = None;
        self.selector = None;
        self.items = None;
        self.rbac_page = None;
        self.page_return = None;
        self.pairing = None;
        self.confirm = None;
        self.install = None;
    }

    /// Closes the modal and clears transient edit buffers.
    pub fn close(&mut self) {
        self.open = false;
        self.search_query.clear();
        self.close_subviews();
    }

    /// Whether an editor, dialog or sub-page is open over the list.
    #[must_use]
    pub fn has_subview(&self) -> bool {
        self.editing.is_some()
            || self.plugin_detail.is_some()
            || self.selector.is_some()
            || self.items.is_some()
            || self.rbac_page.is_some()
            || self.pairing.is_some()
            || self.confirm.is_some()
            || self.install.is_some()
    }
}

/// First-run Setup Wizard steps: provider, (custom only) endpoint, key + test, model, plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    /// Provider list from the catalog (search, scrollbar).
    Step1Provider,
    /// Raw endpoint: only for the `custom` provider.
    StepEndpoint,
    /// API key (masked) and connection test.
    StepKey,
    /// Model picker over the discovered list.
    StepModel,
    Step2Plugins,
}

/// One catalog provider in the wizard list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardProvider {
    pub id: String,
    pub label: String,
    pub keyless: bool,
}

/// State of the connection test.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WizardKeyPhase {
    #[default]
    Editing,
    Testing,
    Failed(String),
    Ok { models: usize, latency_ms: u64 },
}

/// Centered first-run Setup Wizard state.
#[derive(Debug, Clone)]
pub struct SetupWizardState {
    pub step: WizardStep,
    /// Raw endpoint (`custom` provider) or the legacy endpoint.
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub selected_field: usize,
    pub editing: bool,
    pub plugin_ids: Vec<String>,
    pub plugin_enabled: Vec<bool>,
    pub plugin_cursor: usize,
    pub error: Option<String>,
    pub providers: Vec<WizardProvider>,
    pub provider: Option<String>,
    pub search: String,
    pub sel: usize,
    pub off: usize,
    pub key_phase: WizardKeyPhase,
    /// Models to choose from (discovered, or the catalog seed list when the test failed).
    pub models: Vec<String>,
    pub model_search: String,
    pub model_sel: usize,
    pub model_off: usize,
    /// `[model.provider_options] base_url` already in the config (a proxy, for example); used
    /// by the connection test of catalog providers. Never asked for.
    pub base_override: Option<String>,
}

impl SetupWizardState {
    /// Builds a wizard pre-populated with current config defaults and discovered plugins.
    #[must_use]
    pub fn new(cfg: &GatewayConfig, endpoint: Option<&str>, plugin_ids: Vec<String>) -> Self {
        let len = plugin_ids.len();
        let providers: Vec<WizardProvider> = monoagens_reason::model::catalog::entries()
            .iter()
            .map(|e| WizardProvider { id: e.id.clone(), label: e.label.clone(), keyless: e.keyless })
            .collect();
        Self {
            step: WizardStep::Step1Provider,
            endpoint: endpoint
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(monoagens_gateway::DEFAULT_OPENAI_ENDPOINT)
                .to_owned(),
            api_key: String::new(),
            model: cfg.model.model.clone(),
            selected_field: 1,
            editing: false,
            plugin_ids,
            plugin_enabled: vec![false; len],
            plugin_cursor: 0,
            error: None,
            providers,
            provider: None,
            search: String::new(),
            sel: 0,
            off: 0,
            key_phase: WizardKeyPhase::Editing,
            models: Vec::new(),
            model_search: String::new(),
            model_sel: 0,
            model_off: 0,
            base_override: cfg.model.provider_base_url.clone(),
        }
    }

    /// Returns masked API key display (`●●●●...`) without ever revealing the secret.
    #[must_use]
    pub fn masked_api_key(&self) -> String {
        if self.api_key.is_empty() {
            "(not set — press Enter to type)".to_owned()
        } else {
            let dots = "●".repeat(self.api_key.chars().count().min(12));
            format!("{dots} ({} chars)", self.api_key.chars().count())
        }
    }

    /// Providers matching the search text.
    #[must_use]
    pub fn filtered_providers(&self) -> Vec<&WizardProvider> {
        let q = self.search.trim().to_lowercase();
        self.providers
            .iter()
            .filter(|p| q.is_empty() || p.id.to_lowercase().contains(&q) || p.label.to_lowercase().contains(&q))
            .collect()
    }

    /// Models matching the search text.
    #[must_use]
    pub fn filtered_models(&self) -> Vec<&String> {
        let q = self.model_search.trim().to_lowercase();
        self.models.iter().filter(|m| q.is_empty() || m.to_lowercase().contains(&q)).collect()
    }
}

/// Catalog provider as the Providers tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSummary {
    pub id: String,
    pub label: String,
    pub keyless: bool,
    pub key_set: bool,
    pub logged_in: bool,
    pub key_source: Option<String>,
}

/// Inputs of [`build_settings_rows_with`].
pub struct RowsCtx<'a> {
    pub prefs: &'a TuiPreferences,
    pub cfg: &'a GatewayConfig,
    pub paths: &'a ResolvedPaths,
    pub secrets: &'a SecretsUiSummary,
    pub valve_toggles: &'a ValveRuleTogglesState,
    pub plugins: &'a [PluginUiEntry],
    pub providers: &'a [ProviderSummary],
    pub bindings: &'a [(String, String)],
    pub model_name: &'a str,
}

fn row(id: &str, section: &'static str, label: &str, description: &str, value: SettingValueKind) -> SettingRow {
    SettingRow {
        id: id.to_owned(),
        section,
        label: label.to_owned(),
        description: description.to_owned(),
        value,
    }
}

fn opt_path(p: Option<&PathBuf>, none: &str) -> String {
    p.map_or_else(|| none.to_owned(), |p| p.display().to_string())
}

fn key_status(p: &ProviderSummary) -> String {
    if p.keyless && !p.key_set {
        if p.logged_in { "enabled".to_owned() } else { "not enabled".to_owned() }
    } else if p.key_set {
        match &p.key_source {
            Some(src) if src != "secrets" => format!("key set ({src})"),
            _ => "key set".to_owned(),
        }
    } else {
        "key missing".to_owned()
    }
}

/// Builds the filtered option rows for the active `/settings` tab (compatibility wrapper).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn build_settings_rows(
    tab: SettingsTab,
    search_query: &str,
    prefs: &TuiPreferences,
    cfg: &GatewayConfig,
    paths: &ResolvedPaths,
    secrets: &SecretsUiSummary,
    valve_toggles: &ValveRuleTogglesState,
    plugins: &[PluginUiEntry],
) -> Vec<SettingRow> {
    build_settings_rows_with(
        tab,
        search_query,
        &RowsCtx {
            prefs,
            cfg,
            paths,
            secrets,
            valve_toggles,
            plugins,
            providers: &[],
            bindings: &[],
            model_name: &cfg.model.model,
        },
    )
}

/// All rows of every tab, unfiltered (used by the coverage test and the palette).
#[must_use]
pub fn all_settings_rows(ctx: &RowsCtx<'_>) -> Vec<(SettingsTab, SettingRow)> {
    SettingsTab::ALL
        .iter()
        .flat_map(|t| {
            build_settings_rows_with(*t, "", ctx)
                .into_iter()
                .map(move |r| (*t, r))
        })
        .collect()
}

/// Builds the filtered option rows for the active `/settings` tab.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build_settings_rows_with(tab: SettingsTab, search_query: &str, ctx: &RowsCtx<'_>) -> Vec<SettingRow> {
    use SettingValueKind as V;
    let RowsCtx { prefs, cfg, paths, secrets, valve_toggles, plugins, providers, bindings, model_name } = ctx;
    let sel = |s: &str| V::Select(s.to_owned());
    let mut rows = match tab {
        SettingsTab::Appearance => {
            let mut v = vec![
                row("tui.theme", "Theme & Layout", "Theme", "Color palette (Titanium, Slate, Terracotta, Light)", sel(prefs.theme.label())),
                row("tui.message_layout", "Theme & Layout", "Message Layout", "Chat bubbles vs full-width stream", sel(prefs.message_layout.label())),
                row("tui.sidebar", "Theme & Layout", "Sidebar", "Auto (>=120 cols), Open, or Closed", sel(prefs.sidebar.label())),
                row("tui.motion", "Motion & Shimmer", "Motion", "tachyonfx animations (Full, Reduced, Off)", sel(prefs.motion.label())),
                row("tui.shimmer", "Motion & Shimmer", "Shimmer", "Thinking status shimmer (Classic, Subtle, Off)", sel(prefs.shimmer.label())),
                row("tui.icons", "Theme & Layout", "Icons", "Chrome icons: Symbols (single-width) or Emoji", sel(prefs.icons.label())),
                row("tui.mouse_support", "Motion & Shimmer", "Mouse Support", "Enable wheel scroll and click hit-testing", V::Bool(prefs.mouse_support)),
                row("tui.copy_on_select", "Motion & Shimmer", "Copy on Select", "Auto-copy dragged chat selection on mouse release", V::Bool(prefs.copy_on_select)),
                row("tui.auto_chat_title", "Chat & Status Line", "Auto Chat Title", "LLM auto-naming & topic-shift re-titling", V::Bool(prefs.auto_chat_title)),
                row("tui.show_capability_steps", "Chat & Status Line", "Show Capability Steps", "Render tool execution blocks in the chat", V::Bool(prefs.show_capability_steps)),
                row("tui.show_turn_duration", "Chat & Status Line", "Show Turn Duration", "Display elapsed reply time below replies", V::Bool(prefs.show_turn_duration)),
                row("tui.show_brain_trace", "Chat & Status Line", "Show Brain Trace (Lab)", "Show dim brain telemetry in /lab mode", V::Bool(prefs.show_brain_trace)),
                row("tui.show_token_budget", "Chat & Status Line", "Status: Token Budget %", "Show daily budget usage % on status line", V::Bool(prefs.show_token_budget)),
                row("tui.show_cost_estimate", "Chat & Status Line", "Status: Cost Estimate", "Show estimated session cost on status line", V::Bool(prefs.show_cost_estimate)),
                row("tui.show_valve_badge", "Chat & Status Line", "Status: Valve Badge", "Show active Valve mode on status line", V::Bool(prefs.show_valve_badge)),
                row("tui.conv.notifications", "Chat & Status Line", "Notifications", "Notify when the terminal is unfocused and a reply or approval arrives (off, bell, desktop, both)", sel(prefs.conv.notifications.label())),
                row("tui.conv.live_draft_preview", "Chat & Status Line", "Live draft preview", "Show the reply draft while the model writes", V::Bool(prefs.conv.live_draft_preview)),
                row("tui.conv.image_preview", "Chat & Status Line", "Image preview", "Show inline previews of image attachments where the terminal supports it", V::Bool(prefs.conv.image_preview)),
                row("tui.autocomplete_limit", "Chat & Status Line", "Slash Menu Rows", "Visible rows of the / menu (1..=10); it scrolls past that", V::Number(prefs.autocomplete_limit.to_string())),
            ];
            v.push(row(
                "keys.edit",
                "Keybindings",
                "Key bindings",
                "Rebind actions; stored in keybindings.toml next to tui.toml (? shows the current keys)",
                V::Map(bindings.to_vec()),
            ));
            v
        }
        SettingsTab::SoulBrain => vec![
            row("soul.name", "Soul Identity", "soul.name", "Persistent soul display name", V::Text(cfg.soul.name.clone())),
            row("soul.seed", "Soul Identity", "soul.seed", "Deterministic PRNG seed", V::Number(cfg.soul.seed.to_string())),
            row("soul.core_config", "Neural Core", "soul.core_config", "Optional CoreConfig JSON file", V::Path(opt_path(cfg.soul.core_config.as_ref(), "(default)"))),
            row("soul.model_path", "Neural Core", "soul.model_path", "Optional .safetensors neural gate weights path", V::Path(opt_path(cfg.soul.model_path.as_ref(), "(neutral fallback)"))),
            row("soul.expected_blake3", "Neural Core", "soul.expected_blake3", "Expected BLAKE3 of the weights file (64 hex characters)", V::Text(cfg.soul.expected_blake3.clone().unwrap_or_else(|| "(unset)".to_owned()))),
            row("soul.online_adaptation", "Neural Core", "soul.online_adaptation", "Online 3-factor Hebbian / LinUCB adaptation", V::Bool(cfg.soul.online_adaptation.unwrap_or(false))),
            row("embed.backend", "Embeddings", "embed.backend", "Pre-queue embedder backend", sel(cfg.embed.backend.as_str())),
            row("embed.model_ver", "Embeddings", "embed.model_ver", "Embedding model version tag stored with vectors", V::Text(cfg.embed.model_ver.clone())),
            row("embed.dim_d", "Embeddings", "embed.dim_d", "Embedding dimension", V::Number(cfg.embed.dim_d.to_string())),
            row("embed.timeout_ms", "Embeddings", "embed.timeout_ms", "Pre-queue embedding timeout in ms", V::Number(cfg.embed.timeout_ms.to_string())),
            row("embed.model_path", "Embeddings", "embed.model_path", "Local embedding model directory", V::Path(opt_path(cfg.embed.model_path.as_ref(), "(auto / unset)"))),
            row("embed.endpoint_url", "Embeddings", "embed.endpoint_url", "Sidecar or remote embeddings endpoint", V::Text(cfg.embed.endpoint_url.clone().unwrap_or_else(|| "(unset)".to_owned()))),
            row("embed.program", "Embeddings", "embed.program", "Sidecar program for the stdio backend", V::Text(cfg.embed.program.clone().unwrap_or_else(|| "(unset)".to_owned()))),
            row("embed.args", "Embeddings", "embed.args", "Arguments passed to the sidecar program", V::List(cfg.embed.args.clone())),
            row("embed.remote_model", "Embeddings", "embed.remote_model", "Model name sent to a remote embeddings endpoint", V::Text(cfg.embed.remote_model.clone().unwrap_or_else(|| "(unset)".to_owned()))),
        ],
        SettingsTab::Model => {
            let m = &cfg.model;
            let provider_label = m
                .catalog_provider
                .clone()
                .unwrap_or_else(|| m.provider.as_str().to_owned());
            let mut v = vec![
                row("model.provider", "Provider & Model", "Provider", "Open the provider picker (same screen as /model)", V::Action(provider_label)),
                row("model.model", "Provider & Model", "Model", "Open the model picker with search (same screen as /model)", V::Action((*model_name).to_owned())),
                row("model.provider_options.base_url", "Provider & Model", "Base URL override", "Overrides the provider's base URL (empty removes it)", V::Text(m.provider_base_url.clone().unwrap_or_else(|| "(provider default)".to_owned()))),
                row("model.temperature", "Provider & Model", "model.temperature", "Sampling temperature (0.0..=2.0, empty = provider default)", V::Number(m.temperature.map_or_else(|| "default".to_owned(), |t| format!("{t:.2}")))),
                row("model.max_output_tokens", "Output Limits", "max_output_tokens", "Hard cap on output tokens for every purpose (empty = none)", V::Number(m.max_output_tokens.map_or_else(|| "(none)".to_owned(), |n| n.to_string()))),
                row("model.output_limits.classify", "Output Limits", "output_limits.classify", "Max output tokens for Classify calls", V::Number(m.output_limits.classify.to_string())),
                row("model.output_limits.reason", "Output Limits", "output_limits.reason", "Max output tokens for Reason calls", V::Number(m.output_limits.reason.to_string())),
                row("model.output_limits.consolidate", "Output Limits", "output_limits.consolidate", "Max output tokens for Consolidate calls", V::Number(m.output_limits.consolidate.to_string())),
                row("model.daily_token_budget", "Budget & Headers", "daily_token_budget", "UTC-day token budget cap", V::Number(m.daily_token_budget.to_string())),
                row("model.output_limit_param", "Budget & Headers", "output_limit_param", "Wire JSON key for max tokens (max_tokens or max_completion_tokens)", sel(&m.output_limit_param)),
                row("model.extra_headers", "Budget & Headers", "extra_headers", "Extra HTTP headers sent with every model request", V::Map(m.extra_headers.clone())),
            ];
            let up = &m.usage_paths;
            for (key, val) in [
                ("input_tokens", up.input_tokens.clone()),
                ("output_tokens", up.output_tokens.clone()),
                ("cached_tokens", up.cached_tokens.clone()),
                ("reasoning_tokens", up.reasoning_tokens.clone()),
                ("total_tokens", up.total_tokens.clone()),
                ("cost", up.cost.clone().unwrap_or_default()),
            ] {
                v.push(row(
                    &format!("model.usage_paths.{key}"),
                    "Usage Paths & Prices",
                    &format!("usage_paths.{key}"),
                    "JSON path of this counter in the provider's usage object",
                    V::Text(val),
                ));
            }
            v.push(row(
                "model.prices",
                "Usage Paths & Prices",
                "prices",
                "USD per million tokens as 'input,output' per provider/model (drives cost estimates)",
                V::Map(
                    m.prices
                        .iter()
                        .map(|(k, p)| (k.clone(), format!("{},{}", p.input_per_mtok, p.output_per_mtok)))
                        .collect(),
                ),
            ));
            for purpose in ["reason", "classify", "consolidate"] {
                let pm = m.purposes.get(purpose);
                v.push(row(
                    &format!("model.purposes.{purpose}.model"),
                    "Per-purpose Models",
                    &format!("{purpose}.model"),
                    "Model for this purpose only (empty = use the main model)",
                    V::Text(pm.map_or_else(|| "(main model)".to_owned(), |p| p.model.clone())),
                ));
                v.push(row(
                    &format!("model.purposes.{purpose}.provider"),
                    "Per-purpose Models",
                    &format!("{purpose}.provider"),
                    "Provider for this purpose (default: the main provider)",
                    sel(pm.and_then(|p| p.catalog_provider.as_deref()).unwrap_or("(main provider)")),
                ));
            }
            v
        }
        SettingsTab::Valve => {
            let mut v = vec![
                row("valve.mode", "Enforcement", "valve.mode", "enforcing | partial | off", sel(&cfg.valve.mode)),
                row("valve.max_chain_wakes", "Enforcement", "max_chain_wakes", "Circuit-breaker limit for chained wakes", V::Number(cfg.valve.max_chain_wakes.to_string())),
            ];
            let on = |k: &str| match k {
                "cedar_abac" => valve_toggles.cedar_abac,
                "line2_scope" => valve_toggles.line2_scope,
                "taint_blocking" => valve_toggles.taint_blocking,
                "pending_approval" => valve_toggles.pending_approval,
                "budget_guard" => valve_toggles.budget_guard,
                "chain_brake" => valve_toggles.chain_brake,
                "quiet_hours" => valve_toggles.quiet_hours,
                _ => valve_toggles.proactive_cap,
            };
            for (key, desc) in crate::settings_fields::VALVE_TOGGLES {
                v.push(row(&format!("valve.{key}"), "Rule Toggles", key, desc, V::Bool(on(key))));
            }
            v
        }
        SettingsTab::Rbac => {
            let mut out = Vec::new();
            let rbac = cfg.rbac.as_ref();
            out.push(row(
                "rbac.fallback",
                "Fallback & Roles",
                "Fallback role",
                "Role of actors without an assignment (roles.fallback)",
                sel(rbac.and_then(|r| r.fallback_role.as_deref()).unwrap_or("(none)")),
            ));
            if let Some(r) = rbac {
                for (name, role) in &r.roles {
                    out.push(row(
                        &format!("rbac.role.{name}"),
                        "Fallback & Roles",
                        &format!("role {name}"),
                        &format!("allow=[{}] deny=[{}]", role.allow.join(","), role.deny.join(",")),
                        V::Action(format!(
                            "base={} approval={} allow={} deny={}",
                            role.base.as_str(),
                            role.approval.map_or("inherit", |a| a.as_str()),
                            role.allow.len(),
                            role.deny.len()
                        )),
                    ));
                }
            }
            out.push(row("rbac.new_role", "Fallback & Roles", "[ + New role ]", "Create a role; [roles.<name>] is written on first edit", V::Action("Create".to_owned())));
            for person in &cfg.persons {
                out.push(row(
                    &format!("rbac.actor.{}", person.actor_id.as_str()),
                    "Actors",
                    &format!("actor {}", person.actor_id.as_str()),
                    "Roles assigned to this actor",
                    V::Action(format!(
                        "{}{}",
                        if person.roles.is_empty() { "(no roles)".to_owned() } else { person.roles.join(", ") },
                        if person.is_owner { " (owner)" } else { "" }
                    )),
                ));
            }
            out.push(row("rbac.new_actor", "Actors", "[ + Assign roles to an actor ]", "Pick an actor seen in the event log or type an id", V::Action("Add".to_owned())));
            if let Some(r) = rbac {
                for (place, rule) in &r.places {
                    out.push(row(
                        &format!("rbac.place.{}", place.as_str()),
                        "Places & Bases",
                        &format!("place {}", place.as_str()),
                        "Per-place narrowing: allow / deny capability patterns",
                        V::Action(format!("allow={} deny={}", rule.allow.len(), rule.deny.len())),
                    ));
                }
            }
            out.push(row("rbac.new_place", "Places & Bases", "[ + Add place rule ]", "Narrow what is allowed in one place id", V::Action("Add".to_owned())));
            if let Some(r) = rbac {
                for (cap, tier) in &r.permission_bases {
                    out.push(row(
                        &format!("rbac.base.{cap}"),
                        "Places & Bases",
                        &format!("base {cap}"),
                        "Minimum tier required for this capability",
                        V::Action(tier.map_or_else(|| "owner-only".to_owned(), |t| t.as_str().to_owned())),
                    ));
                }
            }
            out.push(row("rbac.new_base", "Places & Bases", "[ + Set permission base ]", "Require a minimum tier (or owner-only) for a capability id", V::Action("Add".to_owned())));
            out
        }
        SettingsTab::Providers => {
            let mut out = Vec::new();
            let (configured, available): (Vec<_>, Vec<_>) =
                providers.iter().partition(|p| p.logged_in);
            for (group, list) in [("Configured", configured), ("Available", available)] {
                for p in list {
                    out.push(row(
                        &format!("provider.{}", p.id),
                        "Providers",
                        &p.label,
                        &format!("{group}: Enter opens the key / test / use page ({})", p.id),
                        V::Action(key_status(p)),
                    ));
                }
            }
            out.extend([
                row("secrets.embed_api_key", "Embeddings Key", "embed_api_key", "API key of a remote embeddings backend (secrets.toml, masked)", V::MaskedSecret { is_set: secrets.top_level_keys.iter().any(|k| k == "embed_api_key") }),
                row("secrets.add_env", "Plugin [env] Secrets", "[ + Add / Update [env] Secret ]", "Write masked secret into secrets.toml [env] (0600)", V::Action("Add Key".to_owned())),
            ]);
            for env_key in &secrets.env_keys {
                out.push(row(
                    &format!("secrets.env.{env_key}"),
                    "Plugin [env] Secrets",
                    &format!("[env] {env_key}"),
                    "Press Enter to rotate, d to unset",
                    V::MaskedSecret { is_set: true },
                ));
            }
            out
        }
        SettingsTab::Plugins => plugins
            .iter()
            .map(|p| {
                let kind_badge = match p.kind {
                    PluginKind::Adapter => "[Adapter]",
                    PluginKind::Capability => "[Capabilities]",
                };
                let status_label = if !p.installed {
                    "available (press i to install)".to_owned()
                } else {
                    format!("{} {} (enabled={})", p.state.dot_glyph(), p.state.label(), p.enabled)
                };
                row(
                    &format!("plugin.{}", p.id),
                    "Configured & Catalog",
                    &format!("{} {}", p.id, kind_badge),
                    "Space: Start/Stop · Enter: Details & TOML · i: Install · d: Remove",
                    V::Action(status_label),
                )
            })
            .collect(),
        SettingsTab::Runtime => {
            let rt = &cfg.runtime;
            let n = |v: u64| V::Number(v.to_string());
            vec![
                row("runtime.hook_timeout_ms", "Timeouts & Cues", "hook_timeout_ms", "Body handshake / hook timeout (ms)", n(rt.hook_timeout_ms)),
                row("runtime.action_timeout_ms", "Timeouts & Cues", "action_timeout_ms", "Action execution timeout (ms)", n(rt.action_timeout_ms)),
                row("runtime.model_timeout_ms", "Timeouts & Cues", "model_timeout_ms", "Model HTTP request timeout (ms)", n(rt.model_timeout_ms)),
                row("runtime.working_cue_ms", "Timeouts & Cues", "working_cue_ms", "Honest R-47 working cue threshold (ms)", n(rt.working_cue_ms)),
                row("runtime.progress_note_ms", "Timeouts & Cues", "progress_note_ms", "Honest R-47 progress note threshold (ms)", n(rt.progress_note_ms)),
                row("runtime.body_shutdown_grace_ms", "Timeouts & Cues", "body_shutdown_grace_ms", "Grace period when shutting down bodies (ms)", n(rt.body_shutdown_grace_ms)),
                row("identity.link_challenge_ttl_s", "Identity Linking", "link_challenge_ttl_s", "Lifetime of an account-link challenge (seconds)", n(cfg.identity.link_challenge_ttl_s)),
                row("identity.link_max_attempts", "Identity Linking", "link_max_attempts", "Wrong-code attempts before a link challenge is void", n(u64::from(cfg.identity.link_max_attempts))),
                row("identity.retroactive_merge", "Identity Linking", "retroactive_merge", "Merge earlier history of linked accounts into one actor", V::Bool(cfg.identity.retroactive_merge)),
                row("data_dir", "Storage", "data_dir", "Directory of the store, logs and sockets (applies after a restart)", V::Path(cfg.data_dir.as_ref().map_or_else(|| paths.data_dir.display().to_string(), |p| p.display().to_string()))),
                row("paths.config", "Resolved Paths", "monoagens.toml", "Active config path", V::ReadOnly(paths.config_path.display().to_string())),
                row("paths.secrets", "Resolved Paths", "secrets.toml", "Active secrets path (0600)", V::ReadOnly(paths.secrets_path.display().to_string())),
                row("paths.data", "Resolved Paths", "data_dir (resolved)", "SegmentedStore & tui_chats.json directory", V::ReadOnly(paths.data_dir.display().to_string())),
            ]
        }
    };

    let q = search_query.trim().to_ascii_lowercase();
    if !q.is_empty() {
        rows.retain(|r| {
            r.label.to_ascii_lowercase().contains(&q)
                || r.description.to_ascii_lowercase().contains(&q)
                || r.section.to_ascii_lowercase().contains(&q)
                || r.id.to_ascii_lowercase().contains(&q)
        });
    }
    rows
}
