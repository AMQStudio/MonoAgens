//! 8-tab 2-pane `/settings` overlay model, plugin drill-down state, and first-run Setup Wizard.

use std::path::PathBuf;

use monoagens_gateway::{GatewayConfig, PluginKind, ResolvedPaths};

use crate::config::{DynamicTomlDoc, DynamicTomlEntry, TuiPreferences};

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
            Self::Appearance => &["Theme & Layout", "Motion & Shimmer", "Chat & Status Line"],
            Self::SoulBrain => &["Soul Identity", "Neural Core", "Embeddings"],
            Self::Model => &["Provider & Model", "Output Limits", "Budget & Headers"],
            Self::Valve => &["Enforcement", "Rule Toggles"],
            Self::Rbac => &["Fallback & Roles", "Places & Persons"],
            Self::Providers => &["Endpoint & Key", "Plugin [env] Secrets"],
            Self::Plugins => &["Configured & Catalog"],
            Self::Runtime => &["Timeouts & Cues", "Resolved Paths"],
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
        }
    }

    /// Dot glyph (`●` for steady states, `◐` for transitions).
    #[must_use]
    pub const fn dot_glyph(&self) -> &'static str {
        match self {
            Self::Starting | Self::Stopping => "◐",
            Self::Running | Self::Stopped | Self::Failed(_) => "●",
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
    Cycle(String),
    Text(String),
    Number(String),
    MaskedSecret { is_set: bool },
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
        self.editing = None;
        self.plugin_detail = None;
    }

    /// Closes the modal and clears transient edit buffers.
    pub fn close(&mut self) {
        self.open = false;
        self.search_query.clear();
        self.editing = None;
        self.plugin_detail = None;
    }
}

/// First-run Setup Wizard steps when `model_api_key` is missing for `openai_compatible`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Step1Provider,
    Step2Plugins,
}

/// Centered 2-step first-run Setup Wizard state.
#[derive(Debug, Clone)]
pub struct SetupWizardState {
    pub step: WizardStep,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub selected_field: usize,
    pub editing: bool,
    pub plugin_ids: Vec<String>,
    pub plugin_enabled: Vec<bool>,
    pub plugin_cursor: usize,
    pub error: Option<String>,
}

impl SetupWizardState {
    /// Builds a wizard pre-populated with current config defaults and discovered plugins.
    #[must_use]
    pub fn new(cfg: &GatewayConfig, endpoint: Option<&str>, plugin_ids: Vec<String>) -> Self {
        let len = plugin_ids.len();
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
}

/// Builds the filtered option rows for the active `/settings` tab.
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
    let mut rows = match tab {
        SettingsTab::Appearance => vec![
            SettingRow {
                id: "tui.theme".to_owned(),
                section: "Theme & Layout",
                label: "Theme".to_owned(),
                description: "Color palette (Titanium, Slate, Terracotta, Light)".to_owned(),
                value: SettingValueKind::Cycle(prefs.theme.label().to_owned()),
            },
            SettingRow {
                id: "tui.message_layout".to_owned(),
                section: "Theme & Layout",
                label: "Message Layout".to_owned(),
                description: "Chat bubbles vs full-width stream".to_owned(),
                value: SettingValueKind::Cycle(prefs.message_layout.label().to_owned()),
            },
            SettingRow {
                id: "tui.sidebar".to_owned(),
                section: "Theme & Layout",
                label: "Sidebar".to_owned(),
                description: "Auto (>=120 cols), Open, or Closed".to_owned(),
                value: SettingValueKind::Cycle(prefs.sidebar.label().to_owned()),
            },
            SettingRow {
                id: "tui.motion".to_owned(),
                section: "Motion & Shimmer",
                label: "Motion".to_owned(),
                description: "tachyonfx animations (Full, Reduced, Off)".to_owned(),
                value: SettingValueKind::Cycle(prefs.motion.label().to_owned()),
            },
            SettingRow {
                id: "tui.shimmer".to_owned(),
                section: "Motion & Shimmer",
                label: "Shimmer".to_owned(),
                description: "Thinking status shimmer (Classic, Subtle, Off)".to_owned(),
                value: SettingValueKind::Cycle(prefs.shimmer.label().to_owned()),
            },
            SettingRow {
                id: "tui.mouse_support".to_owned(),
                section: "Motion & Shimmer",
                label: "Mouse Support".to_owned(),
                description: "Enable wheel scroll and click hit-testing".to_owned(),
                value: SettingValueKind::Bool(prefs.mouse_support),
            },
            SettingRow {
                id: "tui.copy_on_select".to_owned(),
                section: "Motion & Shimmer",
                label: "Copy on Select".to_owned(),
                description: "Auto-copy dragged chat selection on mouse release".to_owned(),
                value: SettingValueKind::Bool(prefs.copy_on_select),
            },
            SettingRow {
                id: "tui.auto_chat_title".to_owned(),
                section: "Chat & Status Line",
                label: "Auto Chat Title".to_owned(),
                description: "LLM auto-naming & topic-shift re-titling".to_owned(),
                value: SettingValueKind::Bool(prefs.auto_chat_title),
            },
            SettingRow {
                id: "tui.show_capability_steps".to_owned(),
                section: "Chat & Status Line",
                label: "Show Capability Steps".to_owned(),
                description: "Render ┌─ ... └─ tool execution blocks".to_owned(),
                value: SettingValueKind::Bool(prefs.show_capability_steps),
            },
            SettingRow {
                id: "tui.show_turn_duration".to_owned(),
                section: "Chat & Status Line",
                label: "Show Turn Duration".to_owned(),
                description: "Display elapsed reply time below replies".to_owned(),
                value: SettingValueKind::Bool(prefs.show_turn_duration),
            },
            SettingRow {
                id: "tui.show_brain_trace".to_owned(),
                section: "Chat & Status Line",
                label: "Show Brain Trace (Lab)".to_owned(),
                description: "Show dim brain telemetry in /lab mode".to_owned(),
                value: SettingValueKind::Bool(prefs.show_brain_trace),
            },
            SettingRow {
                id: "tui.show_token_budget".to_owned(),
                section: "Chat & Status Line",
                label: "Status: Token Budget %".to_owned(),
                description: "Show daily budget usage % on status line".to_owned(),
                value: SettingValueKind::Bool(prefs.show_token_budget),
            },
            SettingRow {
                id: "tui.show_cost_estimate".to_owned(),
                section: "Chat & Status Line",
                label: "Status: Cost Estimate".to_owned(),
                description: "Show estimated session cost on status line".to_owned(),
                value: SettingValueKind::Bool(prefs.show_cost_estimate),
            },
            SettingRow {
                id: "tui.show_valve_badge".to_owned(),
                section: "Chat & Status Line",
                label: "Status: Valve Badge".to_owned(),
                description: "Show active Valve mode on status line".to_owned(),
                value: SettingValueKind::Bool(prefs.show_valve_badge),
            },
        ],
        SettingsTab::SoulBrain => vec![
            SettingRow {
                id: "soul.name".to_owned(),
                section: "Soul Identity",
                label: "soul.name".to_owned(),
                description: "Persistent soul display name".to_owned(),
                value: SettingValueKind::Text(cfg.soul.name.clone()),
            },
            SettingRow {
                id: "soul.seed".to_owned(),
                section: "Soul Identity",
                label: "soul.seed".to_owned(),
                description: "Deterministic PRNG seed".to_owned(),
                value: SettingValueKind::Number(cfg.soul.seed.to_string()),
            },
            SettingRow {
                id: "soul.model_path".to_owned(),
                section: "Neural Core",
                label: "soul.model_path".to_owned(),
                description: "Optional .safetensors neural gate weights path".to_owned(),
                value: SettingValueKind::Text(
                    cfg.soul
                        .model_path
                        .as_ref()
                        .map_or_else(|| "(neutral fallback)".to_owned(), |p| p.display().to_string()),
                ),
            },
            SettingRow {
                id: "soul.online_adaptation".to_owned(),
                section: "Neural Core",
                label: "soul.online_adaptation".to_owned(),
                description: "Online 3-factor Hebbian / LinUCB adaptation".to_owned(),
                value: SettingValueKind::Bool(cfg.soul.online_adaptation.unwrap_or(false)),
            },
            SettingRow {
                id: "embed.backend".to_owned(),
                section: "Embeddings",
                label: "embed.backend".to_owned(),
                description: "Pre-queue embedder backend".to_owned(),
                value: SettingValueKind::Cycle(cfg.embed.backend.as_str().to_owned()),
            },
            SettingRow {
                id: "embed.model_path".to_owned(),
                section: "Embeddings",
                label: "embed.model_path".to_owned(),
                description: "Local embedding model directory".to_owned(),
                value: SettingValueKind::Text(
                    cfg.embed
                        .model_path
                        .as_ref()
                        .map_or_else(|| "(auto / unset)".to_owned(), |p| p.display().to_string()),
                ),
            },
            SettingRow {
                id: "embed.timeout_ms".to_owned(),
                section: "Embeddings",
                label: "embed.timeout_ms".to_owned(),
                description: "Pre-queue embedding timeout in ms".to_owned(),
                value: SettingValueKind::Number(cfg.embed.timeout_ms.to_string()),
            },
        ],
        SettingsTab::Model => vec![
            SettingRow {
                id: "model.provider".to_owned(),
                section: "Provider & Model",
                label: "model.provider".to_owned(),
                description: "openai_compatible or scripted".to_owned(),
                value: SettingValueKind::Cycle(cfg.model.provider.as_str().to_owned()),
            },
            SettingRow {
                id: "model.model".to_owned(),
                section: "Provider & Model",
                label: "model.model".to_owned(),
                description: "Model profile / deployment identifier".to_owned(),
                value: SettingValueKind::Text(cfg.model.model.clone()),
            },
            SettingRow {
                id: "model.temperature".to_owned(),
                section: "Provider & Model",
                label: "model.temperature".to_owned(),
                description: "Sampling temperature (0.0..=2.0)".to_owned(),
                value: SettingValueKind::Number(
                    cfg.model
                        .temperature
                        .map_or_else(|| "default".to_owned(), |t| format!("{t:.2}")),
                ),
            },
            SettingRow {
                id: "model.output_limits.classify".to_owned(),
                section: "Output Limits",
                label: "output_limits.classify".to_owned(),
                description: "Max output tokens for Classify calls".to_owned(),
                value: SettingValueKind::Number(cfg.model.output_limits.classify.to_string()),
            },
            SettingRow {
                id: "model.output_limits.reason".to_owned(),
                section: "Output Limits",
                label: "output_limits.reason".to_owned(),
                description: "Max output tokens for Reason calls".to_owned(),
                value: SettingValueKind::Number(cfg.model.output_limits.reason.to_string()),
            },
            SettingRow {
                id: "model.output_limits.consolidate".to_owned(),
                section: "Output Limits",
                label: "output_limits.consolidate".to_owned(),
                description: "Max output tokens for Consolidate calls".to_owned(),
                value: SettingValueKind::Number(cfg.model.output_limits.consolidate.to_string()),
            },
            SettingRow {
                id: "model.daily_token_budget".to_owned(),
                section: "Budget & Headers",
                label: "daily_token_budget".to_owned(),
                description: "UTC-day token budget cap".to_owned(),
                value: SettingValueKind::Number(cfg.model.daily_token_budget.to_string()),
            },
            SettingRow {
                id: "model.output_limit_param".to_owned(),
                section: "Budget & Headers",
                label: "output_limit_param".to_owned(),
                description: "Wire JSON key for max tokens".to_owned(),
                value: SettingValueKind::Text(cfg.model.output_limit_param.clone()),
            },
        ],
        SettingsTab::Valve => vec![
            SettingRow {
                id: "valve.mode".to_owned(),
                section: "Enforcement",
                label: "valve.mode".to_owned(),
                description: "enforcing | partial | off".to_owned(),
                value: SettingValueKind::Cycle(cfg.valve.mode.clone()),
            },
            SettingRow {
                id: "valve.max_chain_wakes".to_owned(),
                section: "Enforcement",
                label: "max_chain_wakes".to_owned(),
                description: "Circuit-breaker limit for chained wakes".to_owned(),
                value: SettingValueKind::Number(cfg.valve.max_chain_wakes.to_string()),
            },
            SettingRow {
                id: "valve.cedar_abac".to_owned(),
                section: "Rule Toggles",
                label: "cedar_abac".to_owned(),
                description: "Cedar policy ABAC evaluation".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.cedar_abac),
            },
            SettingRow {
                id: "valve.line2_scope".to_owned(),
                section: "Rule Toggles",
                label: "line2_scope".to_owned(),
                description: "VisibleScope Line-2 audience boundary".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.line2_scope),
            },
            SettingRow {
                id: "valve.taint_blocking".to_owned(),
                section: "Rule Toggles",
                label: "taint_blocking".to_owned(),
                description: "3-door taint quarantine enforcement".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.taint_blocking),
            },
            SettingRow {
                id: "valve.pending_approval".to_owned(),
                section: "Rule Toggles",
                label: "pending_approval".to_owned(),
                description: "Hold high-risk actions for owner approval".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.pending_approval),
            },
            SettingRow {
                id: "valve.budget_guard".to_owned(),
                section: "Rule Toggles",
                label: "budget_guard".to_owned(),
                description: "Enforce daily_token_budget".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.budget_guard),
            },
            SettingRow {
                id: "valve.chain_brake".to_owned(),
                section: "Rule Toggles",
                label: "chain_brake".to_owned(),
                description: "Enforce max_chain_wakes circuit breaker".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.chain_brake),
            },
            SettingRow {
                id: "valve.quiet_hours".to_owned(),
                section: "Rule Toggles",
                label: "quiet_hours".to_owned(),
                description: "Defer non-urgent outbound notifications in quiet hours".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.quiet_hours),
            },
            SettingRow {
                id: "valve.proactive_cap".to_owned(),
                section: "Rule Toggles",
                label: "proactive_cap".to_owned(),
                description: "Daily cap on unsolicited proactive notifications".to_owned(),
                value: SettingValueKind::Bool(valve_toggles.proactive_cap),
            },
        ],
        SettingsTab::Rbac => {
            let mut out = Vec::new();
            let fallback = cfg
                .rbac
                .as_ref()
                .and_then(|r| r.fallback_role.as_deref())
                .unwrap_or("(none)");
            out.push(SettingRow {
                id: "rbac.fallback_role".to_owned(),
                section: "Fallback & Roles",
                label: "roles.fallback".to_owned(),
                description: "Fallback role for unbound actors".to_owned(),
                value: SettingValueKind::Text(fallback.to_owned()),
            });
            if let Some(rbac) = &cfg.rbac {
                for (name, role) in &rbac.roles {
                    let app = role
                        .approval
                        .map_or_else(|| "default".to_owned(), |a| a.as_str().to_owned());
                    let summary = format!(
                        "base={} approval={} allow={} deny={}",
                        role.base.as_str(),
                        app,
                        role.allow.len(),
                        role.deny.len()
                    );
                    out.push(SettingRow {
                        id: format!("rbac.role.{name}"),
                        section: "Fallback & Roles",
                        label: format!("role.{name}"),
                        description: format!(
                            "allow=[{}] deny=[{}]",
                            role.allow.join(","),
                            role.deny.join(",")
                        ),
                        value: SettingValueKind::ReadOnly(summary),
                    });
                }
                for (place, rule) in &rbac.places {
                    out.push(SettingRow {
                        id: format!("rbac.place.{}", place.as_str()),
                        section: "Places & Persons",
                        label: format!("places.{}", place.as_str()),
                        description: "Per-place narrowing rule".to_owned(),
                        value: SettingValueKind::ReadOnly(format!(
                            "allow=[{}] deny=[{}]",
                            rule.allow.join(","),
                            rule.deny.join(",")
                        )),
                    });
                }
            }
            for person in &cfg.persons {
                out.push(SettingRow {
                    id: format!("rbac.person.{}", person.actor_id.as_str()),
                    section: "Places & Persons",
                    label: format!("person.{}", person.actor_id.as_str()),
                    description: format!("roles=[{}]", person.roles.join(",")),
                    value: SettingValueKind::ReadOnly(if person.is_owner {
                        format!("owner ({} accounts)", person.accounts.len())
                    } else {
                        format!("member ({} accounts)", person.accounts.len())
                    }),
                });
            }
            out
        }
        SettingsTab::Providers => {
            let mut out = vec![
                SettingRow {
                    id: "secrets.model_endpoint".to_owned(),
                    section: "Endpoint & Key",
                    label: "model_endpoint".to_owned(),
                    description: "OpenAI-compatible completions URL (secrets.toml)".to_owned(),
                    value: SettingValueKind::Text(
                        secrets
                            .model_endpoint
                            .clone()
                            .unwrap_or_else(|| monoagens_gateway::DEFAULT_OPENAI_ENDPOINT.to_owned()),
                    ),
                },
                SettingRow {
                    id: "secrets.model_api_key".to_owned(),
                    section: "Endpoint & Key",
                    label: "model_api_key".to_owned(),
                    description: "Stored in secrets.toml (0600, always masked)".to_owned(),
                    value: SettingValueKind::MaskedSecret {
                        is_set: secrets.model_api_key_set,
                    },
                },
                SettingRow {
                    id: "secrets.add_env".to_owned(),
                    section: "Plugin [env] Secrets",
                    label: "[ + Add / Update [env] Secret ]".to_owned(),
                    description: "Write masked secret into secrets.toml [env] (0600)".to_owned(),
                    value: SettingValueKind::Action("Add Key".to_owned()),
                },
            ];
            for env_key in &secrets.env_keys {
                out.push(SettingRow {
                    id: format!("secrets.env.{env_key}"),
                    section: "Plugin [env] Secrets",
                    label: format!("[env] {env_key}"),
                    description: "Press Enter to rotate, d to unset".to_owned(),
                    value: SettingValueKind::MaskedSecret { is_set: true },
                });
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
                    format!(
                        "{} {} (enabled={})",
                        p.state.dot_glyph(),
                        p.state.label(),
                        p.enabled
                    )
                };
                SettingRow {
                    id: format!("plugin.{}", p.id),
                    section: "Configured & Catalog",
                    label: format!("{} {}", p.id, kind_badge),
                    description: "Space: Start/Stop · Enter: Details & TOML · i: Install · d: Remove".to_owned(),
                    value: SettingValueKind::Action(status_label),
                }
            })
            .collect(),
        SettingsTab::Runtime => vec![
            SettingRow {
                id: "runtime.hook_timeout_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "hook_timeout_ms".to_owned(),
                description: "Body handshake / hook timeout (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.hook_timeout_ms.to_string()),
            },
            SettingRow {
                id: "runtime.action_timeout_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "action_timeout_ms".to_owned(),
                description: "Action execution timeout (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.action_timeout_ms.to_string()),
            },
            SettingRow {
                id: "runtime.model_timeout_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "model_timeout_ms".to_owned(),
                description: "Model HTTP request timeout (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.model_timeout_ms.to_string()),
            },
            SettingRow {
                id: "runtime.working_cue_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "working_cue_ms".to_owned(),
                description: "Honest R-47 working cue threshold (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.working_cue_ms.to_string()),
            },
            SettingRow {
                id: "runtime.progress_note_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "progress_note_ms".to_owned(),
                description: "Honest R-47 progress note threshold (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.progress_note_ms.to_string()),
            },
            SettingRow {
                id: "runtime.body_shutdown_grace_ms".to_owned(),
                section: "Timeouts & Cues",
                label: "body_shutdown_grace_ms".to_owned(),
                description: "Grace period when shutting down bodies (ms)".to_owned(),
                value: SettingValueKind::Number(cfg.runtime.body_shutdown_grace_ms.to_string()),
            },
            SettingRow {
                id: "paths.config".to_owned(),
                section: "Resolved Paths",
                label: "monoagens.toml".to_owned(),
                description: "Active config path".to_owned(),
                value: SettingValueKind::ReadOnly(paths.config_path.display().to_string()),
            },
            SettingRow {
                id: "paths.secrets".to_owned(),
                section: "Resolved Paths",
                label: "secrets.toml".to_owned(),
                description: "Active secrets path (0600)".to_owned(),
                value: SettingValueKind::ReadOnly(paths.secrets_path.display().to_string()),
            },
            SettingRow {
                id: "paths.data".to_owned(),
                section: "Resolved Paths",
                label: "data_dir".to_owned(),
                description: "SegmentedStore & tui_chats.json directory".to_owned(),
                value: SettingValueKind::ReadOnly(paths.data_dir.display().to_string()),
            },
        ],
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
