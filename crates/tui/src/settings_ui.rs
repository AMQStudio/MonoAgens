//! `/settings` behaviour: key handling, paste isolation, mouse/scrollbar routing,
//! live gateway config sync, and `HubFrame` overlay rendering.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use monoagens_gateway::{ConfigError, load_config_file, parse_config_str};
use monoagens_valve::rbac::RbacConfig;

use crate::config::{DynamicTomlValue, parse_dynamic_input};
use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::input::{PasteTarget, TextInput, sanitize_paste};
use crate::core::overlay::{OverlayId, WidgetOutcome, is_key_press};
use crate::core::theme::{ChromeGlyphs, ThemePalette};
use crate::core::viewport::{is_horizontal_scroll, offset_from_scrollbar_y, scroll_offset_for_row};
use crate::list_editor::{ItemMode, ItemOutcome, render_item_editor};
use crate::selector::{SelectorOutcome, render_selector, render_selector_with_hitmap};
use crate::settings::{
    InlineEditState, InlineEditTarget, ProviderSummary, RowsCtx, SettingRow, SettingValueKind,
    SettingsTab, ValveRuleTogglesState, build_settings_rows_with,
};
use crate::settings_fields::{
    Edit, SelectCtx, apply_pref, config_patch, number_spec, path_rule, select_spec,
};
use crate::settings_runtime::{SecretText, SettingsAction, SettingsEvent};
use crate::state::{AppAction, HitRegions, TuiAppState};
use crate::ui_util::{fill, trunc};
use crate::views::rbac::RbacAction;
use crate::widgets::hub_frame::{FooterMode, HubFrameConfig, render_hub_frame};
use crate::widgets::scrollbar::render_vertical_scrollbar;
use crate::widgets::tab_bar::{TabBar, TabItem};

/// `(old, new)` entries of a saved map editor.
pub type MapChange = (Vec<(String, String)>, Vec<(String, String)>);

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
                format!(
                    "{{{}}} {}",
                    entries.len(),
                    entries
                        .iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
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
                logged_in: p.logged_in,
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

    /// Rows currently shown.
    #[must_use]
    pub fn visible_settings_rows(&self) -> Vec<SettingRow> {
        self.current_settings_rows()
    }

    /// Moves the list selection and keeps it on screen.
    pub(crate) fn move_settings_selection(&mut self, delta: isize) {
        let n = self.visible_settings_rows().len();
        if n == 0 {
            self.settings.selected_index = 0;
            self.settings.scroll_offset = 0;
            return;
        }
        let view = self.settings.list_rows.get().max(1);
        self.settings.selected_index =
            (self.settings.selected_index as isize + delta).clamp(0, n as isize - 1) as usize;
        self.settings.scroll_offset = scroll_offset_for_row(
            self.settings.selected_index,
            self.settings.scroll_offset,
            view,
            n,
        );
    }

    fn reset_list_position(&mut self) {
        self.settings.section_index = 0;
        self.settings.selected_index = 0;
        self.settings.scroll_offset = 0;
    }

    fn current_rbac_config(&self) -> RbacConfig {
        self.gateway_cfg.rbac.clone().unwrap_or_default()
    }

    /// Synchronizes `self.settings.rbac` from `gateway_cfg.rbac`, `capabilities_json`,
    /// `plugins`, and `hints.actors`.
    pub fn sync_rbac_view(&mut self) {
        let rbac_cfg = self.current_rbac_config();
        self.settings.rbac.sync_from_config(
            &rbac_cfg,
            self.settings.capabilities_json.as_ref(),
            &self.plugins,
            &self.settings.hints.actors,
        );
    }

    fn apply_rbac_actions(&mut self, actions: Vec<RbacAction>) -> Vec<AppAction> {
        let mut out = Vec::new();
        for act in actions {
            match act {
                RbacAction::ApplyPatch { patch, toast } => {
                    out.extend(self.submit_patch(patch, toast));
                }
                RbacAction::Toast { message, error } => {
                    self.set_toast(message, error);
                }
                RbacAction::Close => {
                    self.settings.close();
                }
            }
        }
        out
    }

    /// Opens `/settings` on `tab`, syncing the RBAC view and requesting live snapshots when connected.
    pub fn open_settings_at(&mut self, tab: SettingsTab) -> Vec<AppAction> {
        self.settings.open_tab(tab);
        self.sync_rbac_view();
        let mut acts = Vec::new();
        if !self.gateway_link.is_empty() {
            acts.push(AppAction::Settings(SettingsAction::LoadConfigSnapshot));
        }
        if matches!(tab, SettingsTab::Providers | SettingsTab::Model)
            && !self.catalog.providers_loaded
        {
            acts.push(AppAction::Catalog(
                crate::catalog_ui::CatalogAction::LoadProviders,
            ));
        }
        if tab == SettingsTab::Rbac {
            acts.push(AppAction::Settings(SettingsAction::LoadRbacHints));
        }
        acts
    }

    /// Switches to `tab`, asking the gateway for what the tab needs.
    pub(crate) fn switch_settings_tab(&mut self, tab: SettingsTab) -> Vec<AppAction> {
        self.settings.tab = tab;
        self.settings.close_subviews();
        self.reset_list_position();
        self.sync_rbac_view();
        let mut acts = Vec::new();
        if !self.gateway_link.is_empty() {
            acts.push(AppAction::Settings(SettingsAction::LoadConfigSnapshot));
        }
        if matches!(tab, SettingsTab::Providers | SettingsTab::Model)
            && !self.catalog.providers_loaded
        {
            acts.push(AppAction::Catalog(
                crate::catalog_ui::CatalogAction::LoadProviders,
            ));
        }
        if tab == SettingsTab::Rbac {
            acts.push(AppAction::Settings(SettingsAction::LoadRbacHints));
        }
        acts
    }

    /// Opens `/settings` on the row `id` of `tab` (palette jump, config errors).
    pub fn open_settings_row(&mut self, tab: SettingsTab, id: &str) -> Vec<AppAction> {
        let acts = self.open_settings_at(tab);
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

    // ---- paste & key handling ---------------------------------------------------------

    /// Routes bracketed or clipboard paste strictly to the topmost active Settings input.
    ///
    /// Never leaks into `search_input` / `search_query` while a sub-editor, modal, or the RBAC
    /// view is active.
    pub fn handle_settings_paste(&mut self, raw: &str) -> bool {
        if let Some(edit) = &mut self.settings.editing {
            return edit.paste_text(raw);
        }
        if let Some(ed) = &mut self.settings.items {
            if let ItemMode::Edit { buffer, .. } = &mut ed.mode {
                let clean = sanitize_paste(raw, false);
                if clean.is_empty() {
                    return false;
                }
                buffer.push_str(&clean);
                return true;
            }
            return false;
        }
        if let Some(sel) = &mut self.settings.selector {
            return sel.paste_text(raw);
        }
        if self.settings.pairing.is_some()
            || self.settings.confirm.is_some()
            || self.settings.install.is_some()
            || self.settings.plugin_detail.is_some()
        {
            return false;
        }
        if self.settings.tab == SettingsTab::Rbac {
            return self.settings.rbac.handle_paste(raw);
        }
        self.settings.sync_input_from_search();
        if self.settings.search_input.paste_text(raw) {
            self.settings.sync_search_from_input();
            self.reset_list_position();
            true
        } else {
            false
        }
    }

    /// Entry point for every key while `/settings` is open.
    pub(crate) fn settings_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        if !is_key_press(key) {
            return Vec::new();
        }
        if self.settings.pairing.is_some() {
            return self.pairing_key(key);
        }
        if let Some(c) = self.settings.confirm.clone() {
            match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.settings.confirm = None;
                    let crate::settings::ConfirmAction::RemovePlugin(pid) = c.action;
                    self.settings.plugin_detail = None;
                    return self.submit_patch(
                        serde_json::json!({ "plugins": { pid: Value::Null } }),
                        "Removed plugin".to_owned(),
                    );
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
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('v' | 'V'))
            {
                return vec![AppAction::PasteFromClipboard];
            }
            return self.selector_key(key);
        }
        if self.settings.items.is_some() {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('v' | 'V'))
            {
                return vec![AppAction::PasteFromClipboard];
            }
            return self.items_key(key);
        }
        if self.settings.editing.is_some() {
            return self.handle_settings_inline_edit_key(key);
        }
        if self.settings.plugin_detail.is_some() {
            return self.handle_plugin_detail_key(key);
        }
        if self.settings.tab == SettingsTab::Rbac {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('v' | 'V'))
            {
                return vec![AppAction::PasteFromClipboard];
            }
            let rbac_cfg = self.current_rbac_config();
            let outcome = self.settings.rbac.handle_key(key, &rbac_cfg);
            match outcome {
                WidgetOutcome::Consumed => return Vec::new(),
                WidgetOutcome::Action(acts) => return self.apply_rbac_actions(acts),
                WidgetOutcome::Close => {
                    self.settings.close();
                    return Vec::new();
                }
                WidgetOutcome::Ignored => {
                    if !self.settings.rbac.has_active_input() {
                        match key.code {
                            KeyCode::Esc => self.settings.close(),
                            KeyCode::Left => {
                                return self.switch_settings_tab(self.settings.tab.prev());
                            }
                            KeyCode::Right => {
                                return self.switch_settings_tab(self.settings.tab.next());
                            }
                            _ => {}
                        }
                    }
                    // Never leak Backspace or Char keys into search_input / search_query on RBAC.
                    return Vec::new();
                }
            }
        }
        let rows = self.visible_settings_rows();
        match key.code {
            KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return vec![AppAction::PasteFromClipboard];
            }
            KeyCode::Esc => {
                if !self.settings.search_query.is_empty() {
                    self.settings.clear_search();
                    self.reset_list_position();
                } else {
                    self.settings.close();
                }
            }
            KeyCode::Left => {
                return self.switch_settings_tab(self.settings.tab.prev());
            }
            KeyCode::Right => {
                return self.switch_settings_tab(self.settings.tab.next());
            }
            KeyCode::Tab => {
                let sec_len = self.settings.tab.sections().len();
                if sec_len > 0 {
                    self.settings.section_index = (self.settings.section_index + 1) % sec_len;
                    self.jump_to_section(self.settings.section_index);
                }
            }
            KeyCode::Up => self.move_settings_selection(-1),
            KeyCode::Down => self.move_settings_selection(1),
            KeyCode::PageUp => {
                self.move_settings_selection(-(self.settings.list_rows.get().max(2) as isize - 1))
            }
            KeyCode::PageDown => {
                self.move_settings_selection(self.settings.list_rows.get().max(2) as isize - 1)
            }
            KeyCode::Home if self.settings.search_query.is_empty() => {
                self.move_settings_selection(isize::MIN / 2)
            }
            KeyCode::End if self.settings.search_query.is_empty() => {
                self.move_settings_selection(isize::MAX / 2)
            }
            KeyCode::Char(' ') if self.settings.search_query.is_empty() => {
                if self.settings.tab == SettingsTab::Plugins
                    && let Some(row) = rows.get(self.settings.selected_index)
                    && let Some(pid) = row.id.strip_prefix("plugin.")
                {
                    return self.toggle_plugin_running(pid);
                }
                return self.activate_settings_row(self.settings.selected_index);
            }
            KeyCode::Char('i')
                if self.settings.tab == SettingsTab::Plugins
                    && self.settings.search_query.is_empty() =>
            {
                if let Some(row) = rows.get(self.settings.selected_index)
                    && let Some(pid) = row.id.strip_prefix("plugin.")
                {
                    self.begin_install(pid);
                }
            }
            KeyCode::Char('d')
                if matches!(
                    self.settings.tab,
                    SettingsTab::Plugins | SettingsTab::Providers
                ) && self.settings.search_query.is_empty() =>
            {
                if let Some(row) = rows.get(self.settings.selected_index) {
                    if let Some(pid) = row.id.strip_prefix("plugin.") {
                        self.begin_remove(pid);
                    } else if let Some(env_k) = row.id.strip_prefix("secrets.env.") {
                        return self.secret_action(
                            "env",
                            env_k,
                            None,
                            format!("Removed [env] secret '{env_k}'"),
                        );
                    }
                }
            }
            KeyCode::Enter => return self.activate_settings_row(self.settings.selected_index),
            _ => {
                self.settings.sync_input_from_search();
                let before = self.settings.search_input.value.clone();
                if self.settings.search_input.handle_key(key) == WidgetOutcome::Consumed {
                    self.settings.sync_search_from_input();
                    if self.settings.search_query != before {
                        self.reset_list_position();
                    }
                }
            }
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
            ItemOutcome::Pick => Vec::new(),
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

    /// Ctrl+R reveals / hides a secret while typing; Ctrl+V pastes from clipboard.
    pub(crate) fn handle_settings_inline_edit_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        if !is_key_press(key) {
            return Vec::new();
        }
        let Some(edit) = &mut self.settings.editing else {
            return Vec::new();
        };
        let secret_prompt = matches!(&edit.target, InlineEditTarget::NewEnvSecretValue { .. })
            || matches!(&edit.target, InlineEditTarget::SettingField { id } if id.starts_with("secrets."));
        match key.code {
            KeyCode::Esc => self.settings.editing = None,
            KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return vec![AppAction::PasteFromClipboard];
            }
            KeyCode::Char('r' | 'R') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if secret_prompt {
                    edit.toggle_masked();
                }
            }
            KeyCode::Enter => {
                let mut finished = self.settings.editing.take().expect("checked");
                finished.sync_buffer();
                return self.commit_inline_edit(finished);
            }
            _ => {
                let _ = edit.handle_key(key);
            }
        }
        Vec::new()
    }

    /// Typed text of the inline editor was submitted.
    pub(crate) fn commit_inline_edit(&mut self, mut edit: InlineEditState) -> Vec<AppAction> {
        edit.sync_buffer();
        let raw = edit.buffer.trim().to_owned();
        match edit.target {
            InlineEditTarget::NewEnvKeyName => {
                if !raw.is_empty() {
                    self.settings.editing = Some(InlineEditState::new(
                        InlineEditTarget::NewEnvSecretValue {
                            env_key: raw.clone(),
                        },
                        format!("Enter secret value for [env].{raw} (masked, Ctrl+R reveals)"),
                        "",
                        true,
                    ));
                }
                Vec::new()
            }
            InlineEditTarget::NewEnvSecretValue { env_key } => {
                if raw.is_empty() {
                    self.set_toast("secret value must not be empty", true);
                    return Vec::new();
                }
                self.secret_action(
                    "env",
                    &env_key,
                    Some(raw),
                    format!("Saved [env].{env_key} to secrets.toml (0600)"),
                )
            }
            InlineEditTarget::PluginTomlValue {
                plugin_id: _,
                key_path,
            } => {
                let hint = self
                    .settings
                    .plugin_detail
                    .as_ref()
                    .and_then(|d| d.toml_entries.iter().find(|e| e.key_path == key_path))
                    .map(|e| e.value.clone());
                let parsed = parse_dynamic_input(&raw, hint.as_ref());
                if matches!(
                    (&hint, &parsed),
                    (
                        Some(DynamicTomlValue::Int(_) | DynamicTomlValue::Float(_)),
                        DynamicTomlValue::Str(_)
                    )
                ) {
                    self.set_toast(
                        format!("'{raw}' is not a number; {key_path} is unchanged"),
                        true,
                    );
                    return Vec::new();
                }
                self.save_plugin_value(&key_path, parsed);
                Vec::new()
            }
            InlineEditTarget::PluginTomlNewKeyName { plugin_id } => {
                if !raw.is_empty() {
                    self.settings.editing = Some(InlineEditState::new(
                        InlineEditTarget::PluginTomlNewKeyValue {
                            plugin_id,
                            key_path: raw.clone(),
                        },
                        format!("Enter value for {raw} (bool/int/float/string/[a,b])"),
                        "",
                        false,
                    ));
                }
                Vec::new()
            }
            InlineEditTarget::PluginTomlNewKeyValue {
                plugin_id: _,
                key_path,
            } => {
                let parsed = parse_dynamic_input(&raw, None);
                self.save_plugin_value(&key_path, parsed);
                Vec::new()
            }
            InlineEditTarget::SettingField { id } => self.commit_setting_field(&id, &raw),
        }
    }

    fn commit_setting_field(&mut self, id: &str, raw: &str) -> Vec<AppAction> {
        if id.starts_with("pd:") {
            return self.plugin_field_committed(id, raw);
        }
        if let Some(name) = id.strip_prefix("secrets.") {
            if raw.is_empty() {
                self.set_toast("secret value must not be empty", true);
                return Vec::new();
            }
            let value = (!raw.is_empty()).then(|| raw.to_owned());
            return self.secret_action(
                "top",
                name,
                value,
                format!("Saved {name} to secrets.toml (0600)"),
            );
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
                self.settings.editing = Some(InlineEditState::new(
                    InlineEditTarget::NewEnvKeyName,
                    "Enter new [env] variable name (e.g. TELEGRAM_BOT_TOKEN)",
                    "",
                    false,
                ));
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
            self.settings.editing = Some(InlineEditState::new(
                InlineEditTarget::NewEnvSecretValue {
                    env_key: env_k.to_owned(),
                },
                format!("Enter new secret value for [env].{env_k} (masked, Ctrl+R reveals)"),
                "",
                true,
            ));
            return Vec::new();
        }
        match &row.value {
            SettingValueKind::Bool(b) => self.commit_edit(&id, Edit::Toggle(!*b)),
            SettingValueKind::Select(_) => {
                let providers: Vec<(String, String)> = self
                    .catalog
                    .providers
                    .iter()
                    .map(|p| (p.id.clone(), p.label.clone()))
                    .collect();
                let ctx = SelectCtx {
                    prefs: &self.prefs,
                    cfg: &self.gateway_cfg,
                    providers: &providers,
                };
                if let Some((opts, cur)) = select_spec(&id, &ctx) {
                    self.settings.selector = Some(crate::selector::Selector::new(
                        &id,
                        &row.label,
                        &row.description,
                        opts,
                        &cur,
                    ));
                }
                Vec::new()
            }
            SettingValueKind::Text(_) | SettingValueKind::Number(_) | SettingValueKind::Path(_) => {
                let hint = match (&row.value, number_spec(&id), path_rule(&id)) {
                    (SettingValueKind::Number(_), Some(spec), _) => {
                        format!(" (allowed {})", spec.describe())
                    }
                    (SettingValueKind::Path(_), _, Some(true)) => " (must exist)".to_owned(),
                    _ => String::new(),
                };
                self.settings.editing = Some(InlineEditState::new(
                    InlineEditTarget::SettingField { id },
                    format!("Edit {}{hint}", row.label),
                    selected_text(&row.value),
                    false,
                ));
                Vec::new()
            }
            SettingValueKind::MaskedSecret { .. } => {
                self.settings.editing = Some(InlineEditState::new(
                    InlineEditTarget::SettingField { id },
                    format!(
                        "Enter new value for {} (masked, Ctrl+R reveals, saved 0600)",
                        row.label
                    ),
                    "",
                    true,
                ));
                Vec::new()
            }
            SettingValueKind::List(items) => {
                self.settings.items =
                    Some(crate::list_editor::ItemEditor::list(&id, &row.label, items));
                Vec::new()
            }
            SettingValueKind::Map(entries) => {
                self.settings.items_base = entries.clone();
                self.settings.items = Some(crate::list_editor::ItemEditor::map(
                    &id, &row.label, entries,
                ));
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
            match monoagens_gateway::apply_config_patch_to_file(
                &self.paths.config_path,
                &self.paths.secrets_path,
                &patch,
            ) {
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
    pub fn secret_action(
        &mut self,
        section: &str,
        name: &str,
        value: Option<String>,
        ok: String,
    ) -> Vec<AppAction> {
        if self.gateway_link.is_empty() {
            let res = match (section, &value) {
                ("env", Some(v)) => {
                    crate::config::save_secret_env(&self.paths.secrets_path, name, v)
                }
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
        self.settings.valve_toggles =
            ValveRuleTogglesState::load_from_config(&self.paths.config_path);
        self.refresh_secrets_summary();
        self.sync_rbac_view();
        let n = self.visible_settings_rows().len();
        self.settings.selected_index = self.settings.selected_index.min(n.saturating_sub(1));
    }

    /// Applies a live `config.get()` snapshot returned by the connected gateway.
    pub fn apply_live_config_snapshot(&mut self, val: &Value) {
        if let Some(raw) = val.get("toml").and_then(Value::as_str) {
            if let Ok(cfg) =
                parse_config_str(raw, &self.paths.config_path, &self.paths.secrets_path)
            {
                self.model_name = cfg.model.model.clone();
                self.valve_mode = cfg.valve.mode.clone();
                self.daily_token_budget = cfg.model.daily_token_budget.max(1);
                self.gateway_cfg = cfg;
                self.refresh_plugins_from_config();
            }
            self.settings.valve_toggles = ValveRuleTogglesState::parse_toml(raw);
        } else {
            let root = val.get("config").unwrap_or(val);
            if let Some(rbac_val) = root.get("rbac")
                && let Ok(rbac) = serde_json::from_value::<RbacConfig>(rbac_val.clone())
            {
                self.gateway_cfg.rbac = Some(rbac);
            }
            if let Some(valve_obj) = root.get("valve").and_then(Value::as_object) {
                if let Some(mode) = valve_obj.get("mode").and_then(Value::as_str) {
                    self.gateway_cfg.valve.mode = mode.to_owned();
                    self.valve_mode = mode.to_owned();
                }
                if let Some(wakes) = valve_obj.get("max_chain_wakes").and_then(Value::as_u64) {
                    self.gateway_cfg.valve.max_chain_wakes = wakes as u32;
                }
                if let Some(guest) = valve_obj.get("guest_access").and_then(Value::as_bool) {
                    self.gateway_cfg.valve.guest_access = guest;
                }
                self.settings.valve_toggles.apply_json(valve_obj);
            }
        }
        self.sync_rbac_view();
        let n = self.visible_settings_rows().len();
        self.settings.selected_index = self.settings.selected_index.min(n.saturating_sub(1));
    }

    /// Folds an answer of [`crate::settings_runtime`] into the state.
    pub fn on_settings_event(&mut self, ev: SettingsEvent) {
        match ev {
            SettingsEvent::Done { ok, result } => match result {
                Ok(()) => {
                    if self.gateway_link.is_empty() || self.paths.config_path.exists() {
                        self.reload_config_from_disk();
                    } else {
                        self.refresh_secrets_summary();
                        self.sync_rbac_view();
                    }
                    if !ok.is_empty() {
                        self.set_toast(ok, false);
                    }
                }
                Err(e) => {
                    if self.gateway_link.is_empty() || self.paths.config_path.exists() {
                        self.reload_config_from_disk();
                    }
                    self.set_toast(e, true);
                }
            },
            SettingsEvent::ConfigLoaded(val) => self.apply_live_config_snapshot(&val),
            SettingsEvent::Pairing(p) => {
                if let Some(st) = &mut self.settings.pairing {
                    st.apply(p);
                }
            }
            SettingsEvent::WizardTested(t) => self.on_wizard_tested(t),
            SettingsEvent::RbacHints {
                capabilities,
                capabilities_json,
                actors,
            } => {
                self.settings.hints.capabilities = capabilities;
                self.settings.capabilities_json = capabilities_json;
                self.settings.hints.actors = actors;
                self.sync_rbac_view();
            }
        }
    }

    /// Saves key bindings edited in the settings map editor.
    fn save_keybindings(
        &mut self,
        old: &[(String, String)],
        new: &[(String, String)],
    ) -> Vec<AppAction> {
        let mut map = self.ux.keymap.clone();
        for (id, key) in new {
            if let Err(e) = map.set(id, key) {
                self.set_toast(e, true);
                // Put the editor back on the saved bindings.
                if let Some(ed) = &mut self.settings.items {
                    *ed = crate::list_editor::ItemEditor::map(
                        "keys.edit",
                        "Key bindings (action id = key)",
                        &self.ux.keymap.rebindable_entries(),
                    );
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
pub fn render_settings_subviews_with_hitmap(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
    hitmap: &mut HitMap,
) {
    if let Some(flow) = &state.settings.install {
        crate::plugin_detail::render_install_with_hitmap(area, buf, flow, palette, hitmap);
    }
    if let Some(ed) = &state.settings.items {
        render_item_editor(area, buf, ed, palette, hit);
    }
    if let Some(sel) = &state.settings.selector {
        render_selector_with_hitmap(area, buf, sel, palette, hit, hitmap);
    }
    if let Some(p) = &state.settings.pairing {
        crate::pairing::render_pairing_with_hitmap(area, buf, p, palette, hitmap);
    }
    if let Some(c) = &state.settings.confirm {
        crate::plugin_detail::render_confirm_with_hitmap(area, buf, c, palette, hitmap);
    }
}

/// Draws the selector, list editor, pairing page, install flow and confirm box over `/settings`.
pub fn render_settings_subviews(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
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
        if self.settings.has_blocking_modal() {
            return;
        }
        if self.settings.plugin_detail.is_some() {
            let n = self.plugin_detail_rows().len();
            let view = self.settings.list_rows.get().max(1);
            if let Some(d) = &mut self.settings.plugin_detail {
                d.selected_row =
                    (d.selected_row as isize + dir).clamp(0, n.saturating_sub(1) as isize) as usize;
                d.scroll_offset = scroll_offset_for_row(d.selected_row, d.scroll_offset, view, n);
            }
        } else {
            self.move_settings_selection(dir);
        }
    }

    /// Routes mouse clicks, wheel scrolling, and scrollbar dragging within the Settings overlay.
    pub fn handle_settings_mouse(&mut self, mouse: MouseEvent, hitmap: &HitMap) -> Vec<AppAction> {
        if is_horizontal_scroll(mouse.kind) {
            return Vec::new();
        }
        let stored_map = self.settings.hitmap.borrow().clone();
        let effective_map = if hitmap.hit_test(mouse.column, mouse.row).is_some() {
            hitmap
        } else {
            &stored_map
        };

        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                if self.settings.confirm.is_some() || self.settings.install.is_some() {
                    return Vec::new();
                }
                if let Some(sel) = &mut self.settings.selector {
                    sel.move_by(delta);
                    return Vec::new();
                }
                if let Some(ed) = &mut self.settings.items {
                    if matches!(ed.mode, ItemMode::Browse) && !ed.items.is_empty() {
                        ed.sel = (ed.sel as isize + delta).clamp(0, ed.items.len() as isize - 1)
                            as usize;
                        ed.off = scroll_offset_for_row(
                            ed.sel,
                            ed.off,
                            ed.view_rows.max(1),
                            ed.items.len(),
                        );
                    }
                    return Vec::new();
                }
                if let Some(p) = &mut self.settings.pairing {
                    p.scroll_by(delta);
                    return Vec::new();
                }
                if self.settings.plugin_detail.is_some() {
                    self.settings_wheel(delta);
                    return Vec::new();
                }
                if self.settings.tab == SettingsTab::Rbac {
                    let rbac_cfg = self.current_rbac_config();
                    return match self
                        .settings
                        .rbac
                        .handle_mouse(mouse, effective_map, &rbac_cfg)
                    {
                        WidgetOutcome::Action(acts) => self.apply_rbac_actions(acts),
                        WidgetOutcome::Close => {
                            self.settings.close();
                            Vec::new()
                        }
                        WidgetOutcome::Consumed | WidgetOutcome::Ignored => Vec::new(),
                    };
                }
                self.move_settings_selection(delta);
                Vec::new()
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.settings.confirm.is_some() {
                    if matches!(
                        effective_map.hit_test(mouse.column, mouse.row),
                        Some(HitTarget::Backdrop(_))
                    ) {
                        self.settings.confirm = None;
                    }
                    return Vec::new();
                }
                if self.settings.install.is_some() {
                    if matches!(
                        effective_map.hit_test(mouse.column, mouse.row),
                        Some(HitTarget::Backdrop(_))
                    ) {
                        self.settings.install = None;
                    }
                    return Vec::new();
                }
                if let Some(p) = &mut self.settings.pairing {
                    if let Some((_, track, total_rows, viewport_rows)) =
                        effective_map.scrollbar_at(mouse.column, mouse.row)
                    {
                        p.scroll_to(offset_from_scrollbar_y(
                            track.y,
                            track.height,
                            mouse.row,
                            total_rows,
                            viewport_rows,
                        ));
                    } else if matches!(
                        effective_map.hit_test(mouse.column, mouse.row),
                        Some(HitTarget::Backdrop(_))
                    ) {
                        let done = p.status.is_done();
                        self.settings.pairing = None;
                        if !done {
                            return vec![AppAction::Settings(SettingsAction::CancelPairing)];
                        }
                    }
                    return Vec::new();
                }
                if self.settings.selector.is_some() {
                    match effective_map.hit_test(mouse.column, mouse.row).cloned() {
                        Some(HitTarget::ListRow(idx)) => {
                            let picked = if let Some(sel) = &mut self.settings.selector {
                                sel.select_row(idx);
                                sel.highlighted()
                                    .map(|o| (sel.target.clone(), o.value.clone()))
                            } else {
                                None
                            };
                            if let Some((target, value)) = picked {
                                self.settings.selector = None;
                                return self.on_select_picked(&target, &value);
                            }
                        }
                        Some(HitTarget::Scrollbar {
                            track,
                            total_rows,
                            viewport_rows,
                            ..
                        }) => {
                            if let Some(sel) = &mut self.settings.selector {
                                sel.off = offset_from_scrollbar_y(
                                    track.y,
                                    track.height,
                                    mouse.row,
                                    total_rows,
                                    viewport_rows,
                                );
                            }
                        }
                        Some(HitTarget::Backdrop(_)) => {
                            self.settings.selector = None;
                        }
                        _ => {}
                    }
                    return Vec::new();
                }
                if let Some(ed) = &mut self.settings.items {
                    match effective_map.hit_test(mouse.column, mouse.row).cloned() {
                        Some(HitTarget::ListRow(idx)) => {
                            if idx < ed.items.len() {
                                ed.sel = idx;
                            }
                        }
                        Some(HitTarget::Scrollbar {
                            track,
                            total_rows,
                            viewport_rows,
                            ..
                        }) => {
                            ed.off = offset_from_scrollbar_y(
                                track.y,
                                track.height,
                                mouse.row,
                                total_rows,
                                viewport_rows,
                            );
                        }
                        Some(HitTarget::Backdrop(_)) => {
                            self.settings.items = None;
                        }
                        _ => {}
                    }
                    return Vec::new();
                }

                match effective_map.hit_test(mouse.column, mouse.row).cloned() {
                    Some(HitTarget::Backdrop(_)) => {
                        self.settings.close();
                        Vec::new()
                    }
                    Some(HitTarget::Tab(idx)) => {
                        if let Some(tab) = SettingsTab::ALL.get(idx).copied() {
                            self.switch_settings_tab(tab)
                        } else {
                            Vec::new()
                        }
                    }
                    Some(HitTarget::Button(id)) if id == "settings.back" => {
                        self.settings.plugin_detail = None;
                        self.settings.editing = None;
                        Vec::new()
                    }
                    _ if self.settings.plugin_detail.is_some() => {
                        match effective_map.hit_test(mouse.column, mouse.row).cloned() {
                            Some(HitTarget::ListRow(idx)) => {
                                if let Some(d) = &mut self.settings.plugin_detail {
                                    d.selected_row = idx;
                                }
                                self.activate_plugin_detail_row(idx)
                            }
                            Some(HitTarget::Scrollbar {
                                id: ScrollId::SettingsList,
                                track,
                                total_rows,
                                viewport_rows,
                            }) => {
                                let new_off = offset_from_scrollbar_y(
                                    track.y,
                                    track.height,
                                    mouse.row,
                                    total_rows,
                                    viewport_rows,
                                );
                                if let Some(d) = &mut self.settings.plugin_detail {
                                    d.scroll_offset = new_off;
                                }
                                Vec::new()
                            }
                            _ => Vec::new(),
                        }
                    }
                    _ if self.settings.tab == SettingsTab::Rbac => {
                        let rbac_cfg = self.current_rbac_config();
                        match self
                            .settings
                            .rbac
                            .handle_mouse(mouse, effective_map, &rbac_cfg)
                        {
                            WidgetOutcome::Action(acts) => self.apply_rbac_actions(acts),
                            WidgetOutcome::Close => {
                                self.settings.close();
                                Vec::new()
                            }
                            WidgetOutcome::Consumed | WidgetOutcome::Ignored => Vec::new(),
                        }
                    }
                    Some(HitTarget::RailRow(sec_idx)) => {
                        self.settings.section_index = sec_idx;
                        self.jump_to_section(sec_idx);
                        Vec::new()
                    }
                    Some(HitTarget::ListRow(opt_idx) | HitTarget::ToggleRow(opt_idx)) => {
                        self.settings.selected_index = opt_idx;
                        self.activate_settings_row(opt_idx)
                    }
                    Some(HitTarget::Scrollbar {
                        id: ScrollId::SettingsList,
                        track,
                        total_rows,
                        viewport_rows,
                    }) => {
                        let new_off = offset_from_scrollbar_y(
                            track.y,
                            track.height,
                            mouse.row,
                            total_rows,
                            viewport_rows,
                        );
                        self.settings.scroll_offset = new_off;
                        let view = self.settings.list_rows.get().max(1);
                        if self.settings.selected_index < new_off {
                            self.settings.selected_index = new_off;
                        } else if self.settings.selected_index >= new_off + view {
                            self.settings.selected_index = (new_off + view).saturating_sub(1);
                        }
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.settings.tab == SettingsTab::Rbac
                    && !self.settings.has_blocking_modal()
                    && self.settings.plugin_detail.is_none()
                {
                    let rbac_cfg = self.current_rbac_config();
                    return match self
                        .settings
                        .rbac
                        .handle_mouse(mouse, effective_map, &rbac_cfg)
                    {
                        WidgetOutcome::Action(acts) => self.apply_rbac_actions(acts),
                        WidgetOutcome::Close => {
                            self.settings.close();
                            Vec::new()
                        }
                        WidgetOutcome::Consumed | WidgetOutcome::Ignored => Vec::new(),
                    };
                }
                if let Some((scroll_id, track, total_rows, viewport_rows)) =
                    effective_map.scrollbar_at(mouse.column, mouse.row)
                {
                    let new_off = offset_from_scrollbar_y(
                        track.y,
                        track.height,
                        mouse.row,
                        total_rows,
                        viewport_rows,
                    );
                    if let Some(p) = &mut self.settings.pairing {
                        p.scroll_to(new_off);
                    } else if let Some(sel) = &mut self.settings.selector {
                        sel.off = new_off;
                    } else if let Some(ed) = &mut self.settings.items {
                        ed.off = new_off;
                    } else if scroll_id == ScrollId::SettingsList {
                        if let Some(d) = &mut self.settings.plugin_detail {
                            d.scroll_offset = new_off;
                        } else {
                            self.settings.scroll_offset = new_off;
                        }
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Renders the complete Settings overlay using `HubFrame`, `TabBar`, `Friendly Label (tech.id)`
    /// rows, interactive scrollbars, and `RbacViewState` for `SettingsTab::Rbac`.
    pub fn render_settings_overlay(
        &self,
        area: Rect,
        buf: &mut Buffer,
        palette: &ThemePalette,
        hit: &mut HitRegions,
    ) {
        let mut hitmap = std::mem::take(&mut hit.hitmap);
        self.render_settings_overlay_with_hitmap(area, buf, palette, hit, &mut hitmap);
        *self.settings.hitmap.borrow_mut() = hitmap.clone();
        hit.hitmap = hitmap;
    }

    /// Renders the complete Settings overlay into `buf`, recording all interactive zones into
    /// both `hit` and `hitmap`.
    pub fn render_settings_overlay_with_hitmap(
        &self,
        area: Rect,
        buf: &mut Buffer,
        palette: &ThemePalette,
        hit: &mut HitRegions,
        hitmap: &mut HitMap,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let g = ChromeGlyphs::for_mode(self.prefs.icons);
        let tab_items: Vec<TabItem> = SettingsTab::ALL
            .iter()
            .map(|&t| {
                let (glyph, short) = match t {
                    SettingsTab::Appearance => (g.tab_appearance, "App"),
                    SettingsTab::SoulBrain => (g.tab_soul, "Soul"),
                    SettingsTab::Model => (g.tab_model, "Mod"),
                    SettingsTab::Valve => (g.tab_valve, "Vlv"),
                    SettingsTab::Rbac => (g.tab_rbac, "RBAC"),
                    SettingsTab::Providers => (g.tab_providers, "Prv"),
                    SettingsTab::Plugins => (g.tab_plugins, "Plg"),
                    SettingsTab::Runtime => (g.tab_runtime, "Run"),
                };
                TabItem {
                    label: format!("{glyph} {}", t.label()),
                    short_label: short.to_owned(),
                    enabled: true,
                    badge: None,
                }
            })
            .collect();
        let mut tab_bar = TabBar::new(tab_items);
        tab_bar.active = SettingsTab::ALL
            .iter()
            .position(|&t| t == self.settings.tab)
            .unwrap_or(0);

        let is_detail = self.settings.plugin_detail.is_some();
        let is_new_rbac = self.settings.tab == SettingsTab::Rbac && !is_detail;

        let title = if let Some(detail) = &self.settings.plugin_detail {
            format!("Settings › Plugins › {}", detail.plugin_id)
        } else {
            "Settings".to_owned()
        };

        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title,
            max_width: 112,
            max_height: 36,
            width_pct: 90,
            height_pct: 88,
            tab_bar: if is_detail { None } else { Some(&tab_bar) },
            rail_width: if is_detail || is_new_rbac { 0 } else { 24 },
            inspector_height: 0,
        };

        let footer = if let Some(edit) = &self.settings.editing {
            FooterMode::InlineInput {
                prompt: &edit.prompt,
                input: &edit.input,
                hint: "Enter save · Esc cancel",
            }
        } else if is_new_rbac && self.settings.rbac.has_active_input() {
            FooterMode::Hints(&[("Enter", "save"), ("Esc", "cancel"), ("Ctrl+V", "paste")])
        } else if is_detail {
            FooterMode::Hints(&[("↑/↓", "select"), ("Enter", "edit/run"), ("Esc", "back")])
        } else if is_new_rbac {
            FooterMode::Hints(&[
                ("←/→", "tab"),
                ("Space", "toggle"),
                ("Tab", "pane"),
                ("Esc", "close"),
            ])
        } else {
            FooterMode::Hints(&[
                ("←/→", "tab"),
                ("↑/↓", "row"),
                ("Enter", "edit"),
                ("Esc", "close"),
            ])
        };

        let areas = render_hub_frame(area, buf, &cfg, footer, palette, hitmap);
        hit.settings_area = areas.outer;

        if let Some(detail) = &self.settings.plugin_detail {
            crate::plugin_detail::render_plugin_detail_with_hitmap(
                areas.main, buf, self, detail, palette, hit, hitmap,
            );
        } else if is_new_rbac {
            let rbac_cfg = self.current_rbac_config();
            self.settings
                .rbac
                .render(areas.main, buf, &rbac_cfg, palette, hitmap);
        } else {
            if let Some(rail_rect) = areas.rail {
                self.render_settings_rail(rail_rect, buf, palette, hit, hitmap);
            }
            self.render_settings_main(areas.main, buf, g.search, palette, hit, hitmap);
        }

        render_settings_subviews_with_hitmap(area, buf, self, palette, hit, hitmap);
    }

    fn render_settings_rail(
        &self,
        rail: Rect,
        buf: &mut Buffer,
        palette: &ThemePalette,
        hit: &mut HitRegions,
        hitmap: &mut HitMap,
    ) {
        if rail.width == 0 || rail.height == 0 {
            return;
        }
        let sections = self.settings.tab.sections();
        let visible = usize::from(rail.height);
        let has_sb = sections.len() > visible && rail.width > 3;
        let row_w = if has_sb { rail.width - 1 } else { rail.width };

        for (idx, sec) in sections.iter().take(visible).enumerate() {
            let y = rail.y + idx as u16;
            let rect = Rect::new(rail.x, y, row_w, 1);
            hit.settings_sections.push((rect, idx));
            hitmap.record(rect, HitTarget::RailRow(idx));

            let active = idx == self.settings.section_index;
            let bg = if active {
                palette.selection_bg
            } else {
                palette.modal_bg
            };
            fill(buf, rect, bg, palette.fg);
            let prefix = if active { "› " } else { "  " };
            let text_style = if active {
                Style::default()
                    .bg(bg)
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted).bg(bg)
            };
            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent).bg(bg)),
                Span::styled(trunc(sec, usize::from(row_w).saturating_sub(2)), text_style),
            ]);
            buf.set_line(rail.x, y, &line, row_w);
        }

        if has_sb {
            let track = Rect::new(rail.x + rail.width - 1, rail.y, 1, rail.height);
            render_vertical_scrollbar(
                buf,
                track,
                ScrollId::SettingsRail,
                sections.len(),
                visible,
                0,
                hitmap,
                Style::default().fg(palette.dim).bg(palette.modal_bg),
                Style::default().fg(palette.accent).bg(palette.modal_bg),
            );
        }
    }

    fn render_settings_main(
        &self,
        main: Rect,
        buf: &mut Buffer,
        search_glyph: &str,
        palette: &ThemePalette,
        hit: &mut HitRegions,
        hitmap: &mut HitMap,
    ) {
        if main.width == 0 || main.height == 0 {
            return;
        }
        let opt_area = if main.height >= 4 {
            let prefix = format!(" {search_glyph} Search: ");
            let prefix_w = UnicodeWidthStr::width(prefix.as_str());
            let input_w = usize::from(main.width).saturating_sub(prefix_w + 1).max(1);
            let search_line = if self.settings.search_query.is_empty() {
                Line::from(vec![
                    Span::styled(
                        prefix,
                        Style::default()
                            .fg(palette.accent)
                            .bg(palette.modal_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        trunc("type to filter settings…", input_w),
                        Style::default().fg(palette.dim).bg(palette.modal_bg),
                    ),
                ])
            } else {
                let (vis, _) = if self.settings.search_input.as_str() == self.settings.search_query
                {
                    self.settings.search_input.visible_slice(input_w)
                } else {
                    TextInput::new(false)
                        .with_value(self.settings.search_query.clone())
                        .visible_slice(input_w)
                };
                Line::from(vec![
                    Span::styled(
                        prefix,
                        Style::default()
                            .fg(palette.accent)
                            .bg(palette.modal_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{vis}▏"),
                        Style::default().fg(palette.fg).bg(palette.modal_bg),
                    ),
                ])
            };
            buf.set_line(main.x, main.y, &search_line, main.width);
            let sep = "─".repeat(usize::from(main.width));
            buf.set_string(
                main.x,
                main.y + 1,
                sep,
                Style::default().fg(palette.separator).bg(palette.modal_bg),
            );
            Rect::new(main.x, main.y + 2, main.width, main.height - 2)
        } else {
            main
        };

        let rows = self.visible_settings_rows();
        let desc_h = if !rows.is_empty() && opt_area.height >= 6 {
            2u16
        } else {
            0
        };
        let preview_h = if self.settings.tab == SettingsTab::Appearance
            && opt_area.height >= 12
        {
            4u16
        } else {
            0
        };
        let list_h = usize::from(opt_area.height.saturating_sub(desc_h + preview_h));
        self.settings.list_rows.set(list_h);

        let selected = self
            .settings
            .selected_index
            .min(rows.len().saturating_sub(1));
        let scroll = scroll_offset_for_row(
            selected,
            self.settings.scroll_offset,
            list_h.max(1),
            rows.len(),
        );
        let has_sb = rows.len() > list_h && opt_area.width > 4;
        let row_w = if has_sb {
            opt_area.width - 1
        } else {
            opt_area.width
        };

        for (vis_idx, row) in rows.iter().skip(scroll).take(list_h).enumerate() {
            let abs_idx = scroll + vis_idx;
            let y = opt_area.y + vis_idx as u16;
            let rect = Rect::new(opt_area.x, y, row_w, 1);
            hit.settings_options.push((rect, abs_idx));
            hitmap.record(rect, HitTarget::ListRow(abs_idx));

            let is_sel = abs_idx == selected;
            let bg = if is_sel {
                palette.selection_bg
            } else {
                palette.modal_bg
            };
            fill(buf, rect, bg, palette.fg);

            let val_style = match &row.value {
                SettingValueKind::Bool(true) => Style::default()
                    .fg(palette.status_green)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
                SettingValueKind::Bool(false) => Style::default().fg(palette.dim).bg(bg),
                SettingValueKind::Select(_) => Style::default().fg(palette.accent).bg(bg),
                SettingValueKind::Action(_) => Style::default()
                    .fg(palette.accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
                SettingValueKind::MaskedSecret { is_set: true } => {
                    Style::default().fg(palette.status_green).bg(bg)
                }
                SettingValueKind::MaskedSecret { is_set: false } => {
                    Style::default().fg(palette.status_yellow).bg(bg)
                }
                _ => Style::default().fg(palette.muted).bg(bg),
            };
            let val_str = value_text(&row.value);
            let row_cols = usize::from(row_w);
            let val_cols = UnicodeWidthStr::width(val_str.as_str());
            let right_w = val_cols
                .clamp(6, (row_cols / 3).max(6))
                .min(row_cols.saturating_sub(10));
            let left_w = row_cols.saturating_sub(right_w + 1);
            let marker = if is_sel { "› " } else { "  " };
            let label_max = left_w.saturating_sub(2);
            let label_fit = trunc(&row.label, label_max);
            let label_pad = label_max.saturating_sub(UnicodeWidthStr::width(label_fit.as_str()));
            let label_style = Style::default()
                .fg(if is_sel { palette.accent } else { palette.fg })
                .bg(bg)
                .add_modifier(if is_sel {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });

            let mut spans = vec![Span::styled(
                marker,
                Style::default().fg(palette.accent).bg(bg),
            )];
            if let Some((friendly, id_part)) = split_friendly_paren(&label_fit) {
                spans.push(Span::styled(friendly.to_owned(), label_style));
                spans.push(Span::styled(
                    id_part.to_owned(),
                    Style::default().fg(palette.dim).bg(bg),
                ));
            } else {
                spans.push(Span::styled(label_fit, label_style));
            }
            if label_pad > 0 {
                spans.push(Span::styled(" ".repeat(label_pad), Style::default().bg(bg)));
            }
            spans.push(Span::styled(" ", Style::default().bg(bg)));
            spans.push(Span::styled(trunc(&val_str, right_w), val_style));
            buf.set_line(opt_area.x, y, &Line::from(spans), row_w);
        }

        if has_sb && list_h > 0 {
            let track = Rect::new(
                opt_area.x + opt_area.width - 1,
                opt_area.y,
                1,
                list_h as u16,
            );
            render_vertical_scrollbar(
                buf,
                track,
                ScrollId::SettingsList,
                rows.len(),
                list_h,
                scroll,
                hitmap,
                Style::default().fg(palette.dim).bg(palette.modal_bg),
                Style::default().fg(palette.accent).bg(palette.modal_bg),
            );
        }

        if desc_h > 0
            && let Some(sel_row) = rows.get(selected)
        {
            let dy = opt_area.y + list_h as u16;
            let sep = "─".repeat(usize::from(opt_area.width.saturating_sub(1)));
            buf.set_string(
                opt_area.x,
                dy,
                sep,
                Style::default().fg(palette.separator).bg(palette.modal_bg),
            );
            buf.set_string(
                opt_area.x + 1,
                dy + 1,
                trunc(
                    &sel_row.description,
                    usize::from(opt_area.width.saturating_sub(2)),
                ),
                Style::default().fg(palette.muted).bg(palette.modal_bg),
            );
        }

        if preview_h > 0 {
            let py = opt_area.y + list_h as u16 + desc_h;
            let max_w = usize::from(opt_area.width.saturating_sub(2));
            buf.set_string(
                opt_area.x + 1,
                py,
                trunc(&format!("Preview ({})", self.prefs.theme.label()), max_w),
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.modal_bg)
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string(
                opt_area.x + 1,
                py + 1,
                trunc("● User message bubble", max_w),
                Style::default().fg(palette.bubble_fg).bg(palette.modal_bg),
            );
            buf.set_string(
                opt_area.x + 1,
                py + 2,
                trunc("● Assistant response", max_w),
                Style::default()
                    .fg(palette.fg)
                    .bg(palette.modal_bg),
            );
        }
    }
}

fn split_friendly_paren(text: &str) -> Option<(&str, &str)> {
    if !text.ends_with(')') {
        return None;
    }
    let idx = text.rfind(" (")?;
    Some((&text[..idx], &text[idx..]))
}
