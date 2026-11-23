//! `/settings` behaviour: key handling for the list and its sub-views, typed edits, and every
//! write (through the gateway client, see [`crate::settings_runtime`]).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::Value;

use monoagens_gateway::{ConfigError, load_config_file};

use crate::config::{DynamicTomlValue, parse_dynamic_input};
use crate::list_editor::{ItemOutcome, render_item_editor};
use crate::selector::{SelectorOutcome, render_selector};
use crate::settings::{
    InlineEditState, InlineEditTarget, ProviderSummary, RowsCtx, SettingRow, SettingValueKind, SettingsTab,
    ValveRuleTogglesState, build_settings_rows_with,
};
use crate::settings_fields::{Edit, SelectCtx, apply_pref, config_patch, number_spec, path_rule, select_spec};
use crate::settings_runtime::{SecretText, SettingsAction, SettingsEvent};

/// `(old, new)` entries of a saved map editor.
pub type MapChange = (Vec<(String, String)>, Vec<(String, String)>);
use crate::state::{AppAction, HitRegions, TuiAppState};
use crate::theme::ThemePalette;

/// One-line text of a row value.
#[must_use]
pub fn value_text(v: &SettingValueKind) -> String {
    match v {
        SettingValueKind::Bool(true) => "● ON".to_owned(),
        SettingValueKind::Bool(false) => "○ off".to_owned(),
        SettingValueKind::Select(s) => format!("✦ {s}"),
        SettingValueKind::Action(s) => format!("▸ {s}"),
        SettingValueKind::Text(s)
        | SettingValueKind::Number(s)
        | SettingValueKind::Path(s)
        | SettingValueKind::ReadOnly(s) => s.clone(),
        SettingValueKind::MaskedSecret { is_set: true } => "🔒 ●●●● (set)".to_owned(),
        SettingValueKind::MaskedSecret { is_set: false } => "🔓 (unset)".to_owned(),
        SettingValueKind::List(items) => {
            if items.is_empty() {
                "[ ] (empty)".to_owned()
            } else {
                format!("[{}] {}", items.len(), items.join(", "))
            }
        }
        SettingValueKind::Map(entries) => {
            if entries.is_empty() {
                "{ } (empty)".to_owned()
            } else {
                format!("{{{}}} {}", entries.len(), entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", "))
            }
        }
    }
}

fn selected_text(v: &SettingValueKind) -> String {
    match v {
        SettingValueKind::Text(s) | SettingValueKind::Number(s) | SettingValueKind::Path(s) => {
            if s.starts_with('(') || s == "default" {
                String::new()
            } else {
                s.clone()
            }
        }
        _ => String::new(),
    }
}

impl TuiAppState {
    /// Catalog providers as the Providers tab lists them.
    fn provider_summaries(&self) -> Vec<ProviderSummary> {
        self.catalog
            .providers
            .iter()
            .map(|p| ProviderSummary {
                id: p.id.clone(),
                label: p.label.clone(),
                keyless: p.keyless,
                key_set: p.key_set,
                key_source: p.key_source.clone(),
            })
            .collect()
    }

    /// Rows of the main list (current tab, current search).
    #[must_use]
    pub fn current_settings_rows(&self) -> Vec<SettingRow> {
        let providers = self.provider_summaries();
        let bindings = self.ux.keymap.rebindable_entries();
        build_settings_rows_with(
            self.settings.tab,
            &self.settings.search_query,
            &RowsCtx {
                prefs: &self.prefs,
                cfg: &self.gateway_cfg,
                paths: &self.paths,
                secrets: &self.secrets_summary,
                valve_toggles: &self.settings.valve_toggles,
                plugins: &self.plugins,
                providers: &providers,
                bindings: &bindings,
                model_name: &self.model_name,
            },
        )
    }

    /// Rows currently shown (the RBAC sub-page replaces the list).
    #[must_use]
    pub fn visible_settings_rows(&self) -> Vec<SettingRow> {
        if self.settings.rbac_page.is_some() {
            self.rbac_page_rows()
        } else {
            self.current_settings_rows()
        }
    }

    /// Moves the list selection and keeps it on screen.
    pub(crate) fn move_settings_selection(&mut self, delta: isize) {
        let n = self.visible_settings_rows().len();
        if n == 0 {
            self.settings.selected_index = 0;
            return;
        }
        let view = self.settings.list_rows.get().max(1);
        self.settings.selected_index = (self.settings.selected_index as isize + delta).clamp(0, n as isize - 1) as usize;
        self.settings.scroll_offset =
            crate::render::scroll_window(self.settings.selected_index, self.settings.scroll_offset, view, n);
    }

    fn reset_list_position(&mut self) {
        self.settings.section_index = 0;
        self.settings.selected_index = 0;
        self.settings.scroll_offset = 0;
    }

    /// Switches to `tab`, asking the gateway for what the tab needs.
    pub(crate) fn switch_settings_tab(&mut self, tab: SettingsTab) -> Vec<AppAction> {
        self.settings.tab = tab;
        self.settings.close_subviews();
        self.reset_list_position();
        let mut acts = Vec::new();
        if matches!(tab, SettingsTab::Providers | SettingsTab::Model) && !self.catalog.providers_loaded {
            acts.push(AppAction::Catalog(crate::catalog_ui::CatalogAction::LoadProviders));
        }
        if tab == SettingsTab::Rbac {
            acts.push(AppAction::Settings(SettingsAction::LoadRbacHints));
        }
        acts
    }

    /// Opens `/settings` on the row `id` of `tab` (palette jump, config errors).
    pub fn open_settings_row(&mut self, tab: SettingsTab, id: &str) -> Vec<AppAction> {
        self.settings.open_tab(tab);
        let mut acts = Vec::new();
        if matches!(tab, SettingsTab::Providers | SettingsTab::Model) && !self.catalog.providers_loaded {
            acts.push(AppAction::Catalog(crate::catalog_ui::CatalogAction::LoadProviders));
        }
        if tab == SettingsTab::Rbac {
            acts.push(AppAction::Settings(SettingsAction::LoadRbacHints));
        }
        let rows = self.current_settings_rows();
        if let Some(pos) = rows.iter().position(|r| r.id == id) {
            self.settings.selected_index = pos;
            self.settings.scroll_offset = pos.saturating_sub(2);
            self.settings.section_index = self
                .settings
                .tab
                .sections()
                .iter()
                .position(|s| *s == rows[pos].section)
                .unwrap_or(0);
        }
        acts
    }

    // ---- key handling -----------------------------------------------------------------

    /// Entry point for every key while `/settings` is open.
    pub(crate) fn settings_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        if self.settings.pairing.is_some() {
            return self.pairing_key(key);
        }
        if let Some(c) = self.settings.confirm.clone() {
            match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.settings.confirm = None;
                    return self.confirmed(c.action);
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.settings.confirm = None,
                _ => {}
            }
            return Vec::new();
        }
        if self.settings.install.is_some() {
            return self.install_key(key);
        }
        if self.settings.selector.is_some() {
            return self.selector_key(key);
        }
        if self.settings.items.is_some() {
            return self.items_key(key);
        }
        if self.settings.editing.is_some() {
            return self.handle_settings_inline_edit_key(key);
        }
        if self.settings.plugin_detail.is_some() {
            return self.handle_plugin_detail_key(key);
        }
        let rows = self.visible_settings_rows();
        match key.code {
            KeyCode::Esc => {
                if self.settings.rbac_page.is_some() {
                    self.rbac_close_page();
                } else if !self.settings.search_query.is_empty() {
                    self.settings.search_query.clear();
                    self.reset_list_position();
                } else {
                    self.settings.close();
                }
            }
            KeyCode::Left if self.settings.rbac_page.is_none() => {
                return self.switch_settings_tab(self.settings.tab.prev());
            }
            KeyCode::Right if self.settings.rbac_page.is_none() => {
                return self.switch_settings_tab(self.settings.tab.next());
            }
            KeyCode::Tab if self.settings.rbac_page.is_none() => {
                let sec_len = self.settings.tab.sections().len();
                if sec_len > 0 {
                    self.settings.section_index = (self.settings.section_index + 1) % sec_len;
                    self.jump_to_section(self.settings.section_index);
                }
            }
            KeyCode::Up => self.move_settings_selection(-1),
            KeyCode::Down => self.move_settings_selection(1),
            KeyCode::PageUp => self.move_settings_selection(-(self.settings.list_rows.get().max(2) as isize - 1)),
            KeyCode::PageDown => self.move_settings_selection(self.settings.list_rows.get().max(2) as isize - 1),
            KeyCode::Home => self.move_settings_selection(isize::MIN / 2),
            KeyCode::End => self.move_settings_selection(isize::MAX / 2),
            KeyCode::Backspace => {
                self.settings.search_query.pop();
                self.reset_list_position();
            }
            KeyCode::Char(' ') if self.settings.search_query.is_empty() || !rows.is_empty() => {
                if self.settings.tab == SettingsTab::Plugins
                    && let Some(row) = rows.get(self.settings.selected_index)
                    && let Some(pid) = row.id.strip_prefix("plugin.")
                {
                    return self.toggle_plugin_running(pid);
                }
                return self.activate_settings_row(self.settings.selected_index);
            }
            KeyCode::Char('i')
                if self.settings.tab == SettingsTab::Plugins && self.settings.search_query.is_empty() =>
            {
                if let Some(row) = rows.get(self.settings.selected_index)
                    && let Some(pid) = row.id.strip_prefix("plugin.")
                {
                    self.begin_install(pid);
                }
            }
            KeyCode::Char('d')
                if matches!(self.settings.tab, SettingsTab::Plugins | SettingsTab::Providers)
                    && self.settings.search_query.is_empty() =>
            {
                if let Some(row) = rows.get(self.settings.selected_index) {
                    if let Some(pid) = row.id.strip_prefix("plugin.") {
                        self.begin_remove(pid);
                    } else if let Some(env_k) = row.id.strip_prefix("secrets.env.") {
                        return self.secret_action("env", env_k, None, format!("Removed [env] secret '{env_k}'"));
                    }
                }
            }
            KeyCode::Enter => return self.activate_settings_row(self.settings.selected_index),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.settings.search_query.push(ch);
                self.reset_list_position();
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn selector_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(sel) = &mut self.settings.selector else {
            return Vec::new();
        };
        match sel.handle_key(key) {
            SelectorOutcome::None => Vec::new(),
            SelectorOutcome::Cancel => {
                self.settings.selector = None;
                Vec::new()
            }
            SelectorOutcome::Picked(v) => {
                let target = sel.target.clone();
                self.settings.selector = None;
                self.on_select_picked(&target, &v)
            }
        }
    }

    /// A selector was answered.
    pub(crate) fn on_select_picked(&mut self, target: &str, value: &str) -> Vec<AppAction> {
        if target.starts_with("rbac.") || target.starts_with("pick.") {
            return self.rbac_select_picked(target, value);
        }
        if target.starts_with("pd:") {
            return self.plugin_select_picked(target, value);
        }
        self.commit_edit(target, Edit::Select(value.to_owned()))
    }

    fn items_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(ed) = &mut self.settings.items else {
            return Vec::new();
        };
        let was_map = ed.is_map;
        match ed.handle_key(key) {
            ItemOutcome::None => Vec::new(),
            ItemOutcome::Close => {
                self.settings.items = None;
                Vec::new()
            }
            ItemOutcome::Pick => {
                let target = ed.target.clone();
                let current = ed.values();
                self.open_capability_picker(&target, &current);
                Vec::new()
            }
            ItemOutcome::Save => {
                let target = ed.target.clone();
                let values = ed.values();
                let entries = ed.entries();
                if was_map {
                    let old = std::mem::replace(&mut self.settings.items_base, entries.clone());
                    self.on_items_saved(&target, None, Some((old, entries)))
                } else {
                    self.on_items_saved(&target, Some(values), None)
                }
            }
        }
    }

    fn on_items_saved(
        &mut self,
        target: &str,
        values: Option<Vec<String>>,
        entries: Option<MapChange>,
    ) -> Vec<AppAction> {
        if target.starts_with("rbac.") {
            return self.rbac_items_saved(target, values, entries);
        }
        if target.starts_with("pd:") || target.starts_with("pv:") {
            return self.plugin_items_saved(target, values.unwrap_or_default());
        }
        if target == "keys.edit" {
            if let Some((old, new)) = entries {
                return self.save_keybindings(&old, &new);
            }
            return Vec::new();
        }
        if let Some(v) = values {
            return self.commit_edit(target, Edit::List(v));
        }
        if let Some((old, new)) = entries {
            return self.commit_edit(target, Edit::Map { old, new });
        }
        Vec::new()
    }

    /// Ctrl+R reveals / hides a secret while typing.
    pub(crate) fn handle_settings_inline_edit_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(edit) = &mut self.settings.editing else {
            return Vec::new();
        };
        let secret_prompt = matches!(&edit.target, InlineEditTarget::NewEnvSecretValue { .. })
            || matches!(&edit.target, InlineEditTarget::SettingField { id } if id.starts_with("secrets."));
        match key.code {
            KeyCode::Esc => self.settings.editing = None,
            KeyCode::Backspace => {
                edit.buffer.pop();
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if secret_prompt {
                    edit.masked = !edit.masked;
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => edit.buffer.push(ch),
            KeyCode::Enter => {
                let finished = self.settings.editing.take().expect("checked");
                return self.commit_inline_edit(finished);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Typed text of the inline editor was submitted.
    pub(crate) fn commit_inline_edit(&mut self, edit: InlineEditState) -> Vec<AppAction> {
        let raw = edit.buffer.trim().to_owned();
        match edit.target {
            InlineEditTarget::NewEnvKeyName => {
                if !raw.is_empty() {
                    self.settings.editing = Some(InlineEditState {
                        target: InlineEditTarget::NewEnvSecretValue { env_key: raw.clone() },
                        prompt: format!("Enter secret value for [env].{raw} (masked, Ctrl+R reveals)"),
                        buffer: String::new(),
                        masked: true,
                    });
                }
                Vec::new()
            }
            InlineEditTarget::NewEnvSecretValue { env_key } => {
                if raw.is_empty() {
                    self.set_toast("secret value must not be empty", true);
                    return Vec::new();
                }
                self.secret_action("env", &env_key, Some(raw), format!("Saved [env].{env_key} to secrets.toml (0600)"))
            }
            InlineEditTarget::PluginTomlValue { plugin_id: _, key_path } => {
                let hint = self
                    .settings
                    .plugin_detail
                    .as_ref()
                    .and_then(|d| d.toml_entries.iter().find(|e| e.key_path == key_path))
                    .map(|e| e.value.clone());
                let parsed = parse_dynamic_input(&raw, hint.as_ref());
                if matches!((&hint, &parsed), (Some(DynamicTomlValue::Int(_) | DynamicTomlValue::Float(_)), DynamicTomlValue::Str(_))) {
                    self.set_toast(format!("'{raw}' is not a number; {key_path} is unchanged"), true);
                    return Vec::new();
                }
                self.save_plugin_value(&key_path, parsed);
                Vec::new()
            }
            InlineEditTarget::PluginTomlNewKeyName { plugin_id } => {
                if !raw.is_empty() {
                    self.settings.editing = Some(InlineEditState {
                        target: InlineEditTarget::PluginTomlNewKeyValue { plugin_id, key_path: raw.clone() },
                        prompt: format!("Enter value for {raw} (bool/int/float/string/[a,b])"),
                        buffer: String::new(),
                        masked: false,
                    });
                }
                Vec::new()
            }
            InlineEditTarget::PluginTomlNewKeyValue { plugin_id: _, key_path } => {
                let parsed = parse_dynamic_input(&raw, None);
                self.save_plugin_value(&key_path, parsed);
                Vec::new()
            }
            InlineEditTarget::SettingField { id } => self.commit_setting_field(&id, &raw),
        }
    }

    fn commit_setting_field(&mut self, id: &str, raw: &str) -> Vec<AppAction> {
        if id.starts_with("rbac.") {
            return self.rbac_text_committed(id, raw);
        }
        if id.starts_with("pd:") {
            return self.plugin_field_committed(id, raw);
        }
        if let Some(name) = id.strip_prefix("secrets.") {
            if raw.is_empty() && name != "model_endpoint" {
                self.set_toast("secret value must not be empty", true);
                return Vec::new();
            }
            let value = (!raw.is_empty()).then(|| raw.to_owned());
            return self.secret_action("top", name, value, format!("Saved {name} to secrets.toml (0600)"));
        }
        let rows = self.visible_settings_rows();
        let kind = rows.iter().find(|r| r.id == id).map(|r| r.value.clone());
        let edit = match kind {
            Some(SettingValueKind::Number(_)) => Edit::Number(raw.to_owned()),
            Some(SettingValueKind::Path(_)) => Edit::Path(raw.to_owned()),
            _ => Edit::Text(raw.to_owned()),
        };
        self.commit_edit(id, edit)
    }

    // ---- activation -------------------------------------------------------------------

    /// Enter / click on row `index` of the list.
    pub(crate) fn activate_settings_row(&mut self, index: usize) -> Vec<AppAction> {
        let rows = self.visible_settings_rows();
        let Some(row) = rows.get(index).cloned() else {
            return Vec::new();
        };
        let id = row.id.clone();
        if self.settings.rbac_page.is_some() {
            return self.rbac_activate_page(&id, &row);
        }
        if id.starts_with("rbac.") {
            return self.rbac_activate_main(&id, &row);
        }
        if let Some(pid) = id.strip_prefix("plugin.") {
            self.open_plugin_detail(pid);
            return Vec::new();
        }
        if let Some(pid) = id.strip_prefix("provider.") {
            let provider = Some(pid.to_owned());
            return self.catalog.open_login(provider, self.model_name.clone());
        }
        match id.as_str() {
            "model.provider" | "model.model" => {
                let provider = self.gateway_cfg.model.catalog_provider.clone();
                return self.catalog.open_picker(provider, self.model_name.clone());
            }
            "secrets.add_env" => {
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::NewEnvKeyName,
                    prompt: "Enter new [env] variable name (e.g. TELEGRAM_BOT_TOKEN)".to_owned(),
                    buffer: String::new(),
                    masked: false,
                });
                return Vec::new();
            }
            "keys.edit" => {
                let entries = self.ux.keymap.rebindable_entries();
                self.settings.items_base = entries.clone();
                self.settings.items = Some(crate::list_editor::ItemEditor::map(
                    "keys.edit",
                    "Key bindings (action id = key)",
                    &entries,
                ));
                return Vec::new();
            }
            _ => {}
        }
        if let Some(env_k) = id.strip_prefix("secrets.env.") {
            self.settings.editing = Some(InlineEditState {
                target: InlineEditTarget::NewEnvSecretValue { env_key: env_k.to_owned() },
                prompt: format!("Enter new secret value for [env].{env_k} (masked, Ctrl+R reveals)"),
                buffer: String::new(),
                masked: true,
            });
            return Vec::new();
        }
        match &row.value {
            SettingValueKind::Bool(b) => self.commit_edit(&id, Edit::Toggle(!*b)),
            SettingValueKind::Select(_) => {
                let providers: Vec<(String, String)> =
                    self.catalog.providers.iter().map(|p| (p.id.clone(), p.label.clone())).collect();
                let ctx = SelectCtx { prefs: &self.prefs, cfg: &self.gateway_cfg, providers: &providers };
                if let Some((opts, cur)) = select_spec(&id, &ctx) {
                    self.settings.selector = Some(crate::selector::Selector::new(&id, &row.label, &row.description, opts, &cur));
                }
                Vec::new()
            }
            SettingValueKind::Text(_) | SettingValueKind::Number(_) | SettingValueKind::Path(_) => {
                let hint = match (&row.value, number_spec(&id), path_rule(&id)) {
                    (SettingValueKind::Number(_), Some(spec), _) => format!(" (allowed {})", spec.describe()),
                    (SettingValueKind::Path(_), _, Some(true)) => " (must exist)".to_owned(),
                    _ => String::new(),
                };
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::SettingField { id },
                    prompt: format!("Edit {}{hint}", row.label),
                    buffer: selected_text(&row.value),
                    masked: false,
                });
                Vec::new()
            }
            SettingValueKind::MaskedSecret { .. } => {
                self.settings.editing = Some(InlineEditState {
                    target: InlineEditTarget::SettingField { id },
                    prompt: format!("Enter new value for {} (masked, Ctrl+R reveals, saved 0600)", row.label),
                    buffer: String::new(),
                    masked: true,
                });
                Vec::new()
            }
            SettingValueKind::List(items) => {
                self.settings.items = Some(crate::list_editor::ItemEditor::list(&id, &row.label, items));
                Vec::new()
            }
            SettingValueKind::Map(entries) => {
                self.settings.items_base = entries.clone();
                self.settings.items = Some(crate::list_editor::ItemEditor::map(&id, &row.label, entries));
                Vec::new()
            }
            SettingValueKind::Action(_) | SettingValueKind::ReadOnly(_) => Vec::new(),
        }
    }

    // ---- writes -------------------------------------------------------------------------

    /// Applies an edit of row `id`.
    pub(crate) fn commit_edit(&mut self, id: &str, edit: Edit) -> Vec<AppAction> {
        if id.starts_with("tui.") {
            return match apply_pref(&self.prefs, id, &edit) {
                Ok(new) => {
                    let mouse_changed = new.mouse_support != self.prefs.mouse_support;
                    self.prefs = new;
                    self.persist_prefs();
                    self.filter_autocomplete();
                    self.set_toast(format!("Saved {id}"), false);
                    if mouse_changed {
                        vec![AppAction::ToggleMouseCapture(self.prefs.mouse_support)]
                    } else {
                        Vec::new()
                    }
                }
                Err(e) => {
                    self.set_toast(e, true);
                    Vec::new()
                }
            };
        }
        match config_patch(id, &edit) {
            Ok(patch) => self.submit_patch(patch, format!("Saved {id}")),
            Err(e) => {
                self.set_toast(e, true);
                Vec::new()
            }
        }
    }

    /// Writes a `config.apply` patch: through the gateway client when there is one (so an
    /// attached daemon changes and reloads), directly into the file when the gateway is not
    /// running (first run, config error).
    pub fn submit_patch(&mut self, patch: Value, ok: String) -> Vec<AppAction> {
        if self.gateway_link.is_empty() {
            match monoagens_gateway::apply_config_patch_to_file(&self.paths.config_path, &self.paths.secrets_path, &patch) {
                Ok(()) => {
                    self.reload_config_from_disk();
                    self.set_toast(ok, false);
                }
                Err(e) => self.set_toast(describe_config_error(&e), true),
            }
            return Vec::new();
        }
        vec![AppAction::Settings(SettingsAction::Apply { patch, ok })]
    }

    /// Writes (or removes, `value == None`) a secret without echoing it.
    pub fn secret_action(&mut self, section: &str, name: &str, value: Option<String>, ok: String) -> Vec<AppAction> {
        if self.gateway_link.is_empty() {
            let res = match (section, &value) {
                ("env", Some(v)) => crate::config::save_secret_env(&self.paths.secrets_path, name, v),
                ("env", None) => crate::config::remove_secret_env(&self.paths.secrets_path, name),
                (_, Some(v)) => crate::config::save_secret_key(&self.paths.secrets_path, name, v),
                (_, None) => Ok(()),
            };
            match res {
                Ok(()) => {
                    self.refresh_secrets_summary();
                    self.set_toast(ok, false);
                }
                Err(e) => self.set_toast(e, true),
            }
            return Vec::new();
        }
        vec![AppAction::Settings(SettingsAction::Secret {
            section: section.to_owned(),
            name: name.to_owned(),
            value: value.map(SecretText::new),
            ok,
        })]
    }

    /// Re-reads `monoagens.toml` / `secrets.toml` into the state after a write.
    pub fn reload_config_from_disk(&mut self) {
        if let Ok(cfg) = load_config_file(&self.paths.config_path, &self.paths.secrets_path) {
            self.model_name = cfg.model.model.clone();
            self.valve_mode = cfg.valve.mode.clone();
            self.daily_token_budget = cfg.model.daily_token_budget.max(1);
            self.gateway_cfg = cfg;
            self.refresh_plugins_from_config();
        }
        self.settings.valve_toggles = ValveRuleTogglesState::load_from_config(&self.paths.config_path);
        self.refresh_secrets_summary();
        // A role page whose role vanished goes back to the list.
        if let Some(page) = &self.settings.rbac_page
            && self.rbac_page_rows().is_empty()
            && !matches!(page, crate::rbac_editor::RbacPage::Actor(_) | crate::rbac_editor::RbacPage::Base(_))
        {
            self.rbac_close_page();
        }
        let n = self.visible_settings_rows().len();
        self.settings.selected_index = self.settings.selected_index.min(n.saturating_sub(1));
    }

    /// Folds an answer of [`crate::settings_runtime`] into the state.
    pub fn on_settings_event(&mut self, ev: SettingsEvent) {
        match ev {
            SettingsEvent::Done { ok, result } => match result {
                Ok(()) => {
                    self.reload_config_from_disk();
                    if !ok.is_empty() {
                        self.set_toast(ok, false);
                    }
                }
                Err(e) => {
                    self.reload_config_from_disk();
                    self.set_toast(e, true);
                }
            },
            SettingsEvent::Pairing(p) => {
                if let Some(st) = &mut self.settings.pairing {
                    st.apply(p);
                }
            }
            SettingsEvent::WizardTested(t) => self.on_wizard_tested(t),
            SettingsEvent::RbacHints { capabilities, actors } => {
                self.settings.hints.capabilities = capabilities;
                self.settings.hints.actors = actors;
            }
        }
    }

    /// Saves key bindings edited in the settings map editor.
    fn save_keybindings(&mut self, old: &[(String, String)], new: &[(String, String)]) -> Vec<AppAction> {
        let mut map = self.ux.keymap.clone();
        for (id, key) in new {
            if let Err(e) = map.set(id, key) {
                self.set_toast(e, true);
                // Put the editor back on the saved bindings.
                if let Some(ed) = &mut self.settings.items {
                    *ed = crate::list_editor::ItemEditor::map("keys.edit", "Key bindings (action id = key)", &self.ux.keymap.rebindable_entries());
                }
                self.settings.items_base = self.ux.keymap.rebindable_entries();
                return Vec::new();
            }
        }
        for (id, _) in old {
            if !new.iter().any(|(n, _)| n == id) {
                map.reset(id);
            }
        }
        let path = crate::keybindings::KeyMap::path_for_config(&self.paths.config_path);
        match map.save(&path) {
            Ok(()) => {
                self.ux.keymap = map;
                self.set_toast("Saved key bindings", false);
            }
            Err(e) => self.set_toast(e, true),
        }
        Vec::new()
    }
}

fn describe_config_error(e: &ConfigError) -> String {
    e.to_string()
}

/// Draws the selector, list editor, pairing page, install flow and confirm box over `/settings`.
pub fn render_settings_subviews(area: Rect, buf: &mut Buffer, state: &TuiAppState, palette: &ThemePalette, hit: &mut HitRegions) {
    if let Some(flow) = &state.settings.install {
        crate::plugin_detail::render_install(area, buf, flow, palette);
    }
    if let Some(ed) = &state.settings.items {
        render_item_editor(area, buf, ed, palette, hit);
    }
    if let Some(sel) = &state.settings.selector {
        render_selector(area, buf, sel, palette, hit);
    }
    if let Some(p) = &state.settings.pairing {
        crate::pairing::render_pairing(area, buf, p, palette);
    }
    if let Some(c) = &state.settings.confirm {
        crate::plugin_detail::render_confirm(area, buf, c, palette);
    }
}

impl TuiAppState {
    /// Mouse wheel over `/settings` (rows of the list or the plugin page).
    pub(crate) fn settings_wheel(&mut self, dir: isize) {
        if self.settings.has_subview() && self.settings.plugin_detail.is_none() {
            return;
        }
        if self.settings.plugin_detail.is_some() {
            let n = self.plugin_detail_rows().len();
            let view = self.settings.list_rows.get().max(1);
            if let Some(d) = &mut self.settings.plugin_detail {
                d.selected_row = (d.selected_row as isize + dir).clamp(0, n.saturating_sub(1) as isize) as usize;
                d.scroll_offset = crate::render::scroll_window(d.selected_row, d.scroll_offset, view, n);
            }
        } else {
            self.move_settings_selection(dir);
        }
    }
}
