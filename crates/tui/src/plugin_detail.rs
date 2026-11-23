//! `Plugins > <id>` detail page: state with reason, stderr tail, Start / Stop / Restart,
//! enable-on-startup, required-secret warnings, typed editors for the plugin's own config
//! values, pairing and the guided install / confirmed remove.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use monoagens_gateway::{BodyTransportConfig, PluginKind};

use crate::config::{
    DynamicTomlDoc, DynamicTomlValue, config_dir_from_path, discover_builtin_binary, find_builtin_preset,
};
use crate::list_editor::ItemEditor;
use crate::selector::{SelectOption, Selector};
use crate::settings::{
    ConfirmAction, ConfirmState, InlineEditState, InlineEditTarget, InstallFlow, PluginDetailState, PluginUiState,
    SettingRow, SettingValueKind,
};
use crate::settings_runtime::SettingsAction;
use crate::state::{AppAction, HitRegions, TuiAppState};
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, fill, render_list, trunc};

/// Plugins whose binary has a `pair` subcommand.
#[must_use]
pub fn can_pair(plugin_id: &str) -> bool {
    plugin_id == "whatsapp"
}

/// Environment variables a plugin's arguments name (`--token-env NAME`, `--xxx-env=NAME`).
#[must_use]
pub fn env_names_in_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if let Some((flag, val)) = a.split_once('=')
            && flag.ends_with("-env")
        {
            out.push(val.to_owned());
        } else if a.ends_with("-env")
            && let Some(next) = it.peek()
            && !next.starts_with('-')
        {
            out.push((*next).clone());
            it.next();
        }
    }
    out.retain(|n| !n.is_empty());
    out.sort();
    out.dedup();
    out
}

impl TuiAppState {
    /// Secrets the plugin needs: env names from its configured (or preset) arguments.
    #[must_use]
    pub fn required_secrets(&self, plugin_id: &str) -> Vec<String> {
        let configured = self.gateway_cfg.effective_plugins().into_iter().find(|p| p.id == plugin_id);
        let args: Vec<String> = match configured.map(|p| p.transport) {
            Some(BodyTransportConfig::Stdio { args, .. }) => args,
            _ => find_builtin_preset(plugin_id)
                .map(|p| p.default_args.iter().map(|s| (*s).to_owned()).collect())
                .unwrap_or_default(),
        };
        env_names_in_args(&args)
    }

    /// Whether `name` is set in `secrets.toml [env]` or the process environment.
    #[must_use]
    pub fn secret_is_set(&self, name: &str) -> bool {
        self.secrets_summary.env_keys.iter().any(|k| k == name) || std::env::var_os(name).is_some_and(|v| !v.is_empty())
    }

    /// Last `n` stderr lines of a plugin from the runtime log.
    #[must_use]
    pub fn plugin_stderr_tail(&self, plugin_id: &str, n: usize) -> Vec<String> {
        let source = format!("plugin:{plugin_id}");
        let mut lines: Vec<String> = self
            .log
            .entries
            .snapshot()
            .into_iter()
            .filter(|e| e.source == source)
            .map(|e| e.message)
            .collect();
        let skip = lines.len().saturating_sub(n);
        lines.drain(..skip);
        lines
    }

    /// Opens `Plugins > <id>`.
    pub fn open_plugin_detail(&mut self, plugin_id: &str) {
        let cfg_dir = config_dir_from_path(&self.paths.config_path);
        let mut detail = PluginDetailState {
            plugin_id: plugin_id.to_owned(),
            selected_row: 0,
            scroll_offset: 0,
            toml_path: cfg_dir.join("plugins").join(format!("{plugin_id}.toml")),
            toml_doc: None,
            toml_entries: Vec::new(),
            show_env_manager: false,
        };
        detail.reload_toml();
        self.settings.plugin_detail = Some(detail);
    }

    /// Rows of the open detail page.
    #[must_use]
    pub fn plugin_detail_rows(&self) -> Vec<SettingRow> {
        use SettingValueKind as V;
        let Some(detail) = &self.settings.plugin_detail else {
            return Vec::new();
        };
        let pid = detail.plugin_id.as_str();
        let entry = self.plugins.iter().find(|p| p.id == pid);
        let row = |id: &str, label: &str, desc: &str, value: V| SettingRow {
            id: id.to_owned(),
            section: "Plugin",
            label: label.to_owned(),
            description: desc.to_owned(),
            value,
        };
        let mut rows = Vec::new();
        let (state_text, running) = match entry.map(|p| &p.state) {
            Some(PluginUiState::Failed(why)) => (format!("{} failed: {why}", "●"), false),
            Some(s @ PluginUiState::NeedsSetup(_)) => (
                format!("{} {} - {}", s.dot_glyph(), s.label(), s.fix_hint().unwrap_or("")),
                false,
            ),
            Some(s) => (
                format!("{} {}", s.dot_glyph(), s.label()),
                matches!(s, PluginUiState::Running | PluginUiState::Starting | PluginUiState::RunningElsewhere),
            ),
            None => ("not installed".to_owned(), false),
        };
        rows.push(row("pd:status", "State", "Live state with the failure reason", V::ReadOnly(state_text)));
        rows.push(row("pd:start", "[ Start ]", "Start the plugin process", V::Action(if running { "running".to_owned() } else { "Start".to_owned() })));
        rows.push(row("pd:stop", "[ Stop ]", "Stop the plugin process", V::Action("Stop".to_owned())));
        rows.push(row("pd:restart", "[ Restart ]", "Stop, then start the plugin", V::Action("Restart".to_owned())));
        rows.push(row(
            "pd:enabled",
            "Enable on startup",
            "Start this plugin when the gateway starts",
            V::Bool(entry.is_some_and(|p| p.enabled)),
        ));
        if let Some(cfg) = self.gateway_cfg.plugins.iter().find(|p| p.id == pid) {
            rows.push(row("pd:kind", "Kind", "adapter (a platform) or capability (tools)", V::Select(cfg.kind.as_str().to_owned())));
            match &cfg.transport {
                BodyTransportConfig::Stdio { command, args } => {
                    rows.push(row("pd:command", "Command", "Executable the gateway launches", V::Path(command.clone())));
                    rows.push(row("pd:args", "Arguments", "Command line arguments", V::List(args.clone())));
                }
                BodyTransportConfig::UnixSocket { path } => {
                    rows.push(row("pd:socket", "Socket", "Unix socket of an already running plugin", V::Path(path.display().to_string())));
                }
            }
            rows.push(row("pd:deps", "Dependencies", "Plugins that must start first", V::List(cfg.dependencies.clone())));
            rows.push(row("pd:optdeps", "Optional dependencies", "Plugins started first when present", V::List(cfg.optional_dependencies.clone())));
        }
        for name in self.required_secrets(pid) {
            let set = self.secret_is_set(&name);
            rows.push(row(
                &format!("pd:secret:{name}"),
                &if set { format!("Secret {name}") } else { format!("! Missing secret {name}") },
                "Required by the plugin's arguments; stored in secrets.toml [env] (never shown)",
                V::Action(if set { "set - Enter to replace".to_owned() } else { "NOT SET - Enter to set".to_owned() }),
            ));
        }
        if can_pair(pid) {
            rows.push(row("pd:pair", "[ Pair device ]", "Link a phone: shows the QR / pair code on a page here", V::Action("Pair".to_owned())));
        }
        rows.push(row("pd:log", "[ View log ]", "Open /log filtered to this plugin", V::Action(format!("plugin:{pid}"))));
        for (i, l) in self.plugin_stderr_tail(pid, 5).into_iter().enumerate() {
            rows.push(row(&format!("pd:err:{i}"), "stderr", "Last stderr lines of the plugin", V::ReadOnly(l)));
        }
        rows.push(row("pd:reload", "[ Reload plugin file ]", &format!("Re-read {}", detail.toml_path.display()), V::Action(format!("{} keys", detail.toml_entries.len()))));
        for e in &detail.toml_entries {
            let id = format!("pv:{}", e.key_path);
            let desc = "Value in plugins/<id>.toml (comments are preserved)";
            rows.push(match &e.value {
                DynamicTomlValue::Bool(b) => row(&id, &e.key_path, desc, V::Bool(*b)),
                DynamicTomlValue::Int(_) | DynamicTomlValue::Float(_) => row(&id, &e.key_path, desc, V::Number(e.value.display_compact())),
                DynamicTomlValue::Str(s) => row(&id, &e.key_path, desc, V::Text(s.clone())),
                DynamicTomlValue::StrArray(a) => row(&id, &e.key_path, desc, V::List(a.clone())),
            });
        }
        rows.push(row("pd:addkey", "[ + Add value ]", "Insert a key in plugins/<id>.toml", V::Action("Add".to_owned())));
        rows
    }

    /// Esc / navigation inside the detail page; Esc leaves the page.
    pub(crate) fn handle_plugin_detail_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let total = self.plugin_detail_rows().len();
        let view = self.settings.list_rows.get().max(1);
        let Some(detail) = &mut self.settings.plugin_detail else {
            return Vec::new();
        };
        let mv = |d: &mut PluginDetailState, delta: isize| {
            if total > 0 {
                d.selected_row = (d.selected_row as isize + delta).clamp(0, total as isize - 1) as usize;
            }
            d.scroll_offset = crate::render::scroll_window(d.selected_row, d.scroll_offset, view, total);
        };
        match key.code {
            KeyCode::Esc => self.settings.plugin_detail = None,
            KeyCode::Up => mv(detail, -1),
            KeyCode::Down => mv(detail, 1),
            KeyCode::PageUp => mv(detail, -(view as isize)),
            KeyCode::PageDown => mv(detail, view as isize),
            KeyCode::Home => mv(detail, isize::MIN / 2),
            KeyCode::End => mv(detail, isize::MAX / 2),
            KeyCode::Char(' ') | KeyCode::Enter => {
                let row = detail.selected_row;
                return self.activate_plugin_detail_row(row);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Activates detail row `index`.
    pub fn activate_plugin_detail_row(&mut self, index: usize) -> Vec<AppAction> {
        let rows = self.plugin_detail_rows();
        let Some(row) = rows.get(index).cloned() else {
            return Vec::new();
        };
        let Some(pid) = self.settings.plugin_detail.as_ref().map(|d| d.plugin_id.clone()) else {
            return Vec::new();
        };
        match row.id.as_str() {
            "pd:start" => {
                self.set_plugin_state(&pid, PluginUiState::Starting);
                vec![AppAction::StartPlugin(pid)]
            }
            "pd:stop" => {
                self.set_plugin_state(&pid, PluginUiState::Stopping);
                vec![AppAction::StopPlugin(pid)]
            }
            "pd:restart" => {
                self.set_plugin_state(&pid, PluginUiState::Starting);
                vec![AppAction::Settings(SettingsAction::RestartPlugin(pid))]
            }
            "pd:enabled" => {
                let next = !self.plugins.iter().find(|p| p.id == pid).is_some_and(|p| p.enabled);
                let patch = json!({ "plugins": { pid.clone(): { "enabled": next } } });
                self.submit_patch(
                    patch,
                    format!("{} '{pid}' on startup", if next { "Enabled" } else { "Disabled" }),
                )
            }
            "pd:kind" => {
                let cur = match &row.value {
                    SettingValueKind::Select(s) => s.clone(),
                    _ => "adapter".to_owned(),
                };
                self.settings.selector = Some(Selector::new(
                    "pd:kind",
                    "Kind",
                    &row.description,
                    vec![
                        SelectOption::plain("adapter", "Connects a chat platform or device"),
                        SelectOption::plain("capability", "Provides tools to the soul"),
                    ],
                    &cur,
                ));
                Vec::new()
            }
            "pd:command" | "pd:socket" => {
                let cur = match &row.value {
                    SettingValueKind::Path(p) => p.clone(),
                    _ => String::new(),
                };
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::SettingField { id: row.id.clone() },
                    prompt: format!("Edit {}", row.label),
                    buffer: cur,
                    masked: false,
                });
                Vec::new()
            }
            "pd:args" | "pd:deps" | "pd:optdeps" => {
                let vals = match &row.value {
                    SettingValueKind::List(v) => v.clone(),
                    _ => Vec::new(),
                };
                self.settings.items = Some(ItemEditor::list(&row.id, &row.label, &vals));
                Vec::new()
            }
            "pd:pair" => self.start_pairing(&pid),
            "pd:log" => {
                self.log.open();
                self.log.source = Some(format!("plugin:{pid}"));
                Vec::new()
            }
            "pd:reload" => {
                if let Some(d) = &mut self.settings.plugin_detail {
                    d.reload_toml();
                }
                self.set_toast("Reloaded plugin config", false);
                Vec::new()
            }
            "pd:addkey" => {
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::PluginTomlNewKeyName { plugin_id: pid },
                    prompt: "Enter new TOML key path (e.g. session.auto_reconnect)".to_owned(),
                    buffer: String::new(),
                    masked: false,
                });
                Vec::new()
            }
            id if id.starts_with("pd:secret:") => {
                let name = id.trim_start_matches("pd:secret:").to_owned();
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::NewEnvSecretValue { env_key: name.clone() },
                    prompt: format!("Value for [env].{name} (masked, Ctrl+R reveals)"),
                    buffer: String::new(),
                    masked: true,
                });
                Vec::new()
            }
            id if id.starts_with("pv:") => {
                let key_path = id.trim_start_matches("pv:").to_owned();
                self.activate_plugin_value(&pid, &key_path, &row)
            }
            _ => Vec::new(),
        }
    }

    fn activate_plugin_value(&mut self, pid: &str, key_path: &str, row: &SettingRow) -> Vec<AppAction> {
        match &row.value {
            SettingValueKind::Bool(b) => {
                self.save_plugin_value(key_path, DynamicTomlValue::Bool(!*b));
            }
            SettingValueKind::List(vals) => {
                self.settings.items = Some(ItemEditor::list(&row.id, key_path, vals));
            }
            SettingValueKind::Number(cur) | SettingValueKind::Text(cur) => {
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::PluginTomlValue {
                        plugin_id: pid.to_owned(),
                        key_path: key_path.to_owned(),
                    },
                    prompt: format!("Edit {key_path}"),
                    buffer: cur.clone(),
                    masked: false,
                });
            }
            _ => {}
        }
        Vec::new()
    }

    /// Writes one value of `plugins/<id>.toml` (format and comments preserved).
    pub(crate) fn save_plugin_value(&mut self, key_path: &str, value: DynamicTomlValue) {
        let Some(detail) = &mut self.settings.plugin_detail else {
            return;
        };
        if detail.toml_doc.is_none() {
            detail.toml_doc = DynamicTomlDoc::load_or_init(&detail.toml_path).ok();
        }
        let Some(doc) = &mut detail.toml_doc else {
            self.set_toast("plugin config file is not valid TOML", true);
            return;
        };
        match doc.set_entry(key_path, value).and_then(|()| doc.save()) {
            Ok(()) => {
                detail.reload_toml();
                self.set_toast(format!("Saved {key_path}"), false);
            }
            Err(e) => self.set_toast(e, true),
        }
    }

    /// A selector over the detail page was answered.
    pub(crate) fn plugin_select_picked(&mut self, target: &str, value: &str) -> Vec<AppAction> {
        let Some(pid) = self.settings.plugin_detail.as_ref().map(|d| d.plugin_id.clone()) else {
            return Vec::new();
        };
        match target {
            "pd:kind" => {
                let patch = json!({ "plugins": { pid.clone(): { "kind": value } } });
                self.submit_patch(patch, format!("Saved kind of '{pid}'"))
            }
            _ => Vec::new(),
        }
    }

    /// A list editor on the detail page saved.
    pub(crate) fn plugin_items_saved(&mut self, target: &str, values: Vec<String>) -> Vec<AppAction> {
        let Some(pid) = self.settings.plugin_detail.as_ref().map(|d| d.plugin_id.clone()) else {
            return Vec::new();
        };
        if let Some(key_path) = target.strip_prefix("pv:") {
            self.save_plugin_value(key_path, DynamicTomlValue::StrArray(values));
            return Vec::new();
        }
        let key = match target {
            "pd:args" => "args",
            "pd:deps" => "dependencies",
            "pd:optdeps" => "optional_dependencies",
            _ => return Vec::new(),
        };
        let patch = json!({ "plugins": { pid.clone(): { key: values } } });
        self.submit_patch(patch, format!("Saved {key} of '{pid}'"))
    }

    /// An inline text edit on the detail page was committed.
    pub(crate) fn plugin_field_committed(&mut self, id: &str, text: &str) -> Vec<AppAction> {
        let Some(pid) = self.settings.plugin_detail.as_ref().map(|d| d.plugin_id.clone()) else {
            return Vec::new();
        };
        let (key, is_path) = match id {
            "pd:command" => ("command", false),
            "pd:socket" => ("socket_path", true),
            _ => return Vec::new(),
        };
        let _ = is_path;
        if text.trim().is_empty() {
            self.set_toast("value must not be empty", true);
            return Vec::new();
        }
        let patch: Value = json!({ "plugins": { pid.clone(): { key: text.trim() } } });
        self.submit_patch(patch, format!("Saved {key} of '{pid}'"))
    }

    /// Starts the in-TUI pairing page.
    pub(crate) fn start_pairing(&mut self, pid: &str) -> Vec<AppAction> {
        match crate::pairing::pair_command(&self.paths, &self.gateway_cfg, pid, None) {
            Ok(cmd) => {
                self.settings.pairing = Some(crate::pairing::PairingState::new(pid));
                vec![AppAction::Settings(SettingsAction::StartPairing(cmd))]
            }
            Err(e) => {
                self.set_toast(e, true);
                Vec::new()
            }
        }
    }

    /// Keys on the pairing page.
    pub(crate) fn pairing_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(st) = &self.settings.pairing else {
            return Vec::new();
        };
        let done = st.status.is_done();
        let pid = st.plugin_id.clone();
        match key.code {
            KeyCode::Esc => {
                self.settings.pairing = None;
                if !done {
                    return vec![AppAction::Settings(SettingsAction::CancelPairing)];
                }
            }
            KeyCode::Char('r') if done => return self.start_pairing(&pid),
            _ => {}
        }
        Vec::new()
    }

    // ---- guided install / remove ----

    /// Starts the guided install of catalog plugin `plugin_id`.
    pub(crate) fn begin_install(&mut self, plugin_id: &str) {
        let Some(preset) = find_builtin_preset(plugin_id) else {
            self.set_toast(format!("Plugin '{plugin_id}' has no built-in preset"), true);
            return;
        };
        let found = discover_builtin_binary(preset).map(|p| p.display().to_string());
        let hint = format!("Build it with: cargo build -p {} (or put '{}' on PATH)", preset.binary, preset.binary);
        self.settings.install = Some(InstallFlow {
            plugin_id: plugin_id.to_owned(),
            kind: preset.kind,
            command: found,
            binary: preset.binary.to_owned(),
            enable: true,
            hint,
        });
    }

    /// Keys of the install flow: `e` toggles enable, `k` toggles kind, Enter installs.
    pub(crate) fn install_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(flow) = &mut self.settings.install else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => self.settings.install = None,
            KeyCode::Char('e') => flow.enable = !flow.enable,
            KeyCode::Char('k') => {
                flow.kind = match flow.kind {
                    PluginKind::Adapter => PluginKind::Capability,
                    PluginKind::Capability => PluginKind::Adapter,
                };
            }
            KeyCode::Enter => {
                let flow = flow.clone();
                let Some(cmd) = flow.command.clone() else {
                    self.set_toast(format!("'{}' was not found; build it first", flow.binary), true);
                    return Vec::new();
                };
                self.settings.install = None;
                let preset = find_builtin_preset(&flow.plugin_id);
                let args: Vec<String> = preset
                    .map(|p| p.default_args.iter().map(|s| (*s).to_owned()).collect())
                    .unwrap_or_default();
                let opt: Vec<String> = preset
                    .map(|p| p.default_optional_deps.iter().map(|s| (*s).to_owned()).collect())
                    .unwrap_or_default();
                let patch = json!({ "plugins": { flow.plugin_id.clone(): {
                    "enabled": flow.enable,
                    "kind": flow.kind.as_str(),
                    "transport": "stdio",
                    "command": cmd,
                    "args": args,
                    "optional_dependencies": opt,
                }}});
                return self.submit_patch(patch, format!("Installed plugin '{}'", flow.plugin_id));
            }
            _ => {}
        }
        Vec::new()
    }

    /// Asks before removing `plugin_id`.
    pub(crate) fn begin_remove(&mut self, plugin_id: &str) {
        self.settings.confirm = Some(ConfirmState {
            prompt: format!("Remove plugin '{plugin_id}' from the configuration? (y/n)"),
            action: ConfirmAction::RemovePlugin(plugin_id.to_owned()),
        });
    }
}

/// Draws the plugin detail page into the settings body.
pub fn render_plugin_detail(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    detail: &PluginDetailState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let plugin = state.plugins.iter().find(|p| p.id == detail.plugin_id);
    let (glyph, color, label) = match plugin {
        Some(p) => {
            let color = match &p.state {
                PluginUiState::Running | PluginUiState::RunningElsewhere => palette.status_green,
                PluginUiState::Starting | PluginUiState::Stopping | PluginUiState::NeedsSetup(_) => {
                    palette.status_yellow
                }
                PluginUiState::Stopped | PluginUiState::Failed(_) => palette.status_red,
            };
            (p.state.dot_glyph(), color, p.state.label())
        }
        None => ("●", palette.dim, "unknown"),
    };
    let back = "[ ← Plugins ]";
    let back_w = (unicode_width::UnicodeWidthStr::width(back) as u16).min(area.width.saturating_sub(2));
    hit.settings_back = Some(Rect::new(area.x + 1, area.y, back_w, 1));
    let header = Line::from(vec![
        Span::styled(
            back,
            Style::default().bg(palette.selection_bg).fg(palette.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" > {} (", detail.plugin_id), Style::default().fg(palette.fg).add_modifier(Modifier::BOLD)),
        Span::styled(glyph, Style::default().fg(color)),
        Span::styled(format!(" {label})"), Style::default().fg(palette.muted)),
    ]);
    buf.set_line(area.x + 1, area.y, &header, area.width.saturating_sub(2));

    let rows_data = state.plugin_detail_rows();
    let list_area = Rect::new(area.x + 1, area.y + 2, area.width.saturating_sub(2), area.height.saturating_sub(2));
    state.settings.list_rows.set(list_area.height as usize);
    fill(buf, list_area, palette.modal_bg, palette.fg);
    let rows: Vec<ListRow> = rows_data
        .iter()
        .map(|r| ListRow {
            left: r.label.clone(),
            right: crate::settings_ui::value_text(&r.value),
            dim: matches!(r.value, SettingValueKind::ReadOnly(_)),
        })
        .collect();
    let mut hits = Vec::new();
    render_list(list_area, buf, &rows, detail.selected_row, detail.scroll_offset, palette, &mut hits);
    hit.settings_options.extend(hits);
}

/// Draws the guided install box.
pub fn render_install(area: Rect, buf: &mut Buffer, flow: &InstallFlow, palette: &ThemePalette) {
    let bx = centered(78.min(area.width), 13.min(area.height), area);
    let inner = boxed(bx, buf, &format!("Install {}", flow.plugin_id), palette);
    let style = Style::default().fg(palette.fg).bg(palette.modal_bg);
    let mut y = inner.y;
    let mut put = |text: String, st: Style| {
        if y < inner.y + inner.height {
            buf.set_string(inner.x + 1, y, trunc(&text, inner.width.saturating_sub(2) as usize), st);
            y += 1;
        }
    };
    match &flow.command {
        Some(c) => put(format!("Command:  {c}"), style),
        None => {
            put(
                format!("Command:  NOT FOUND (looked for '{}' next to this binary, in target/ and on PATH)", flow.binary),
                Style::default().fg(palette.status_red).bg(palette.modal_bg),
            );
            put(format!("Hint:     {}", flow.hint), Style::default().fg(palette.status_yellow).bg(palette.modal_bg));
        }
    }
    put(format!("Kind:     {}   (k toggles)", flow.kind.as_str()), style);
    put(format!("Enable on startup: {}   (e toggles)", if flow.enable { "yes" } else { "no" }), style);
    put(String::new(), style);
    put(
        if flow.command.is_some() {
            "Enter installs · Esc cancels".to_owned()
        } else {
            "Nothing is enabled until the binary exists · Esc closes".to_owned()
        },
        Style::default().fg(palette.dim).bg(palette.modal_bg),
    );
}

/// Draws a yes/no question.
pub fn render_confirm(area: Rect, buf: &mut Buffer, c: &ConfirmState, palette: &ThemePalette) {
    let bx = centered(70.min(area.width), 5.min(area.height), area);
    let inner = boxed(bx, buf, "Confirm", palette);
    buf.set_string(
        inner.x + 1,
        inner.y + 1,
        trunc(&c.prompt, inner.width.saturating_sub(2) as usize),
        Style::default().fg(palette.fg).bg(palette.modal_bg),
    );
}

