//! First-run wizard: provider list from the catalog, key entry with a connection test, model
//! picker over the discovered list, plugin selection. A raw endpoint is only asked for the
//! `custom` provider.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde_json::json;

use monoagens_reason::model::catalog;

use crate::config::{install_builtin_plugin, save_secret_key, write_or_update_gateway_config};
use crate::settings::{SetupWizardState, WizardKeyPhase, WizardStep};
use crate::settings_runtime::{SecretText, SettingsAction};
use crate::state::{AppAction, HitRegions, TuiAppState};
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, render_list, trunc};

/// The test result of the wizard's connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardTest {
    pub result: Result<Vec<String>, String>,
    pub latency_ms: u64,
}

impl TuiAppState {
    /// Keys of the first-run wizard.
    pub(crate) fn handle_wizard_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(w) = &mut self.wizard else {
            return Vec::new();
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match w.step {
            WizardStep::Step1Provider => {
                let n = w.filtered_providers().len();
                match key.code {
                    KeyCode::Esc => {
                        if w.search.is_empty() {
                            self.wizard = None;
                            return vec![AppAction::CompleteSetupWizard];
                        }
                        w.search.clear();
                        w.sel = 0;
                    }
                    KeyCode::Up => w.sel = w.sel.saturating_sub(1),
                    KeyCode::Down => w.sel = (w.sel + 1).min(n.saturating_sub(1)),
                    KeyCode::PageUp => w.sel = w.sel.saturating_sub(8),
                    KeyCode::PageDown => w.sel = (w.sel + 8).min(n.saturating_sub(1)),
                    KeyCode::Backspace => {
                        w.search.pop();
                        w.sel = 0;
                    }
                    KeyCode::Enter => return self.wizard_pick_provider(w_sel_index(self)),
                    KeyCode::Char(c) if !ctrl => {
                        w.search.push(c);
                        w.sel = 0;
                    }
                    _ => {}
                }
                if let Some(w) = &mut self.wizard {
                    let n = w.filtered_providers().len();
                    w.off = crate::render::scroll_window(w.sel, w.off, 10, n);
                }
            }
            WizardStep::StepEndpoint => match key.code {
                KeyCode::Esc => w.step = WizardStep::Step1Provider,
                KeyCode::Enter => {
                    if w.endpoint.trim().is_empty() {
                        w.error = Some("The custom provider needs an endpoint URL".to_owned());
                    } else {
                        w.error = None;
                        w.step = WizardStep::StepKey;
                    }
                }
                KeyCode::Backspace => {
                    w.endpoint.pop();
                }
                KeyCode::Char(c) if !ctrl => w.endpoint.push(c),
                _ => {}
            },
            WizardStep::StepKey => return self.wizard_key_step(key),
            WizardStep::StepModel => {
                let n = w.filtered_models().len();
                match key.code {
                    KeyCode::Esc => w.step = WizardStep::StepKey,
                    KeyCode::Up => w.model_sel = w.model_sel.saturating_sub(1),
                    KeyCode::Down => w.model_sel = (w.model_sel + 1).min(n.saturating_sub(1)),
                    KeyCode::PageUp => w.model_sel = w.model_sel.saturating_sub(8),
                    KeyCode::PageDown => w.model_sel = (w.model_sel + 8).min(n.saturating_sub(1)),
                    KeyCode::Backspace => {
                        w.model_search.pop();
                        w.model_sel = 0;
                    }
                    KeyCode::Enter => {
                        let picked = w
                            .filtered_models()
                            .get(w.model_sel)
                            .map(|m| (*m).clone())
                            .or_else(|| (!w.model_search.trim().is_empty()).then(|| w.model_search.trim().to_owned()));
                        if let Some(m) = picked {
                            w.model = m;
                            w.step = WizardStep::Step2Plugins;
                        }
                    }
                    KeyCode::Char(c) if !ctrl => {
                        w.model_search.push(c);
                        w.model_sel = 0;
                    }
                    _ => {}
                }
                let n = w.filtered_models().len();
                w.model_off = crate::render::scroll_window(w.model_sel, w.model_off, 10, n);
            }
            WizardStep::Step2Plugins => match key.code {
                KeyCode::Up => w.plugin_cursor = w.plugin_cursor.saturating_sub(1),
                KeyCode::Down => {
                    if !w.plugin_ids.is_empty() {
                        w.plugin_cursor = (w.plugin_cursor + 1).min(w.plugin_ids.len() - 1);
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(slot) = w.plugin_enabled.get_mut(w.plugin_cursor) {
                        *slot = !*slot;
                    }
                }
                KeyCode::Left | KeyCode::BackTab => {
                    w.step = if w.provider.is_some() { WizardStep::StepModel } else { WizardStep::Step1Provider };
                }
                KeyCode::Enter => return self.finish_wizard(),
                KeyCode::Esc => {
                    self.wizard = None;
                    return vec![AppAction::CompleteSetupWizard];
                }
                _ => {}
            },
        }
        Vec::new()
    }

    fn wizard_pick_provider(&mut self, index: usize) -> Vec<AppAction> {
        let Some(w) = &mut self.wizard else {
            return Vec::new();
        };
        let Some(p) = w.filtered_providers().get(index).map(|p| (*p).clone()) else {
            return Vec::new();
        };
        w.provider = Some(p.id.clone());
        w.key_phase = WizardKeyPhase::Editing;
        w.api_key.clear();
        w.error = None;
        w.step = if p.id == "custom" { WizardStep::StepEndpoint } else { WizardStep::StepKey };
        if p.id != "custom" {
            w.endpoint.clear();
        }
        Vec::new()
    }

    fn wizard_key_step(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(w) = &mut self.wizard else {
            return Vec::new();
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let provider = w.provider.clone().unwrap_or_default();
        let keyless = catalog::find(&provider).is_some_and(|e| e.keyless);
        match key.code {
            KeyCode::Esc => {
                w.step = if provider == "custom" { WizardStep::StepEndpoint } else { WizardStep::Step1Provider };
                w.api_key.clear();
                w.key_phase = WizardKeyPhase::Editing;
            }
            KeyCode::Enter => {
                if matches!(w.key_phase, WizardKeyPhase::Testing) {
                    return Vec::new();
                }
                if let WizardKeyPhase::Ok { .. } = w.key_phase {
                    w.step = WizardStep::StepModel;
                    return Vec::new();
                }
                if w.api_key.trim().is_empty() && !keyless {
                    w.key_phase = WizardKeyPhase::Failed("Type or paste an API key first.".to_owned());
                    return Vec::new();
                }
                w.key_phase = WizardKeyPhase::Testing;
                let base = if provider == "custom" {
                    (!w.endpoint.trim().is_empty()).then(|| w.endpoint.trim().to_owned())
                } else {
                    w.base_override.clone()
                };
                return vec![AppAction::Settings(SettingsAction::WizardTest {
                    provider,
                    key: SecretText::new(w.api_key.trim()),
                    base_url: base,
                })];
            }
            // Continue without a successful test (offline, or the provider has no listing).
            KeyCode::Char('s') if ctrl => {
                if !matches!(w.key_phase, WizardKeyPhase::Testing) && (keyless || !w.api_key.trim().is_empty()) {
                    w.models = seed_models(&provider);
                    w.step = WizardStep::StepModel;
                }
            }
            KeyCode::Char('u') if ctrl => w.api_key.clear(),
            KeyCode::Backspace => {
                if !matches!(w.key_phase, WizardKeyPhase::Testing) {
                    w.api_key.pop();
                    w.key_phase = WizardKeyPhase::Editing;
                }
            }
            KeyCode::Char(c) if !ctrl && !c.is_whitespace() && !matches!(w.key_phase, WizardKeyPhase::Testing) => {
                w.api_key.push(c);
                w.key_phase = WizardKeyPhase::Editing;
            }
            _ => {}
        }
        Vec::new()
    }

    /// The connection test of the wizard finished.
    pub fn on_wizard_tested(&mut self, test: WizardTest) {
        let Some(w) = &mut self.wizard else {
            return;
        };
        if w.step != WizardStep::StepKey {
            return;
        }
        match test.result {
            Ok(models) => {
                w.key_phase = WizardKeyPhase::Ok { models: models.len(), latency_ms: test.latency_ms };
                w.models = if models.is_empty() { seed_models(w.provider.as_deref().unwrap_or_default()) } else { models };
                w.model_search.clear();
                w.model_sel = 0;
                w.model_off = 0;
                w.step = WizardStep::StepModel;
            }
            Err(e) => {
                w.key_phase = WizardKeyPhase::Failed(format!("Test failed: {e}. Ctrl+S continues anyway."));
            }
        }
    }

    /// Click on a wizard button / row.
    pub(crate) fn activate_wizard_button(&mut self, btn_idx: usize) -> Vec<AppAction> {
        let Some(w) = &mut self.wizard else {
            return Vec::new();
        };
        match w.step {
            WizardStep::Step1Provider => {
                w.sel = btn_idx;
                self.wizard_pick_provider(btn_idx)
            }
            WizardStep::StepModel => {
                if let Some(m) = w.filtered_models().get(btn_idx).map(|m| (*m).clone()) {
                    w.model = m;
                    w.step = WizardStep::Step2Plugins;
                }
                Vec::new()
            }
            WizardStep::Step2Plugins => {
                if btn_idx == 999 {
                    self.finish_wizard()
                } else if btn_idx == 998 {
                    w.step = if w.provider.is_some() { WizardStep::StepModel } else { WizardStep::Step1Provider };
                    Vec::new()
                } else if let Some(slot) = w.plugin_enabled.get_mut(btn_idx) {
                    w.plugin_cursor = btn_idx;
                    *slot = !*slot;
                    Vec::new()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn finish_wizard(&mut self) -> Vec<AppAction> {
        let Some(mut w) = self.wizard.take() else {
            return Vec::new();
        };
        let result = match w.provider.clone() {
            Some(provider) => self.save_wizard_provider(&w, &provider),
            None => self.save_wizard_legacy(&w),
        };
        if let Err(e) = result {
            w.error = Some(e);
            self.wizard = Some(w);
            return Vec::new();
        }
        w.api_key.clear();
        for (idx, enabled) in w.plugin_enabled.iter().enumerate() {
            if *enabled
                && let Some(pid) = w.plugin_ids.get(idx)
                && let Ok(spec) = install_builtin_plugin(&self.paths.config_path, &self.paths.secrets_path, pid)
            {
                self.gateway_cfg.plugins.push(spec);
            }
        }
        self.reload_config_from_disk();
        self.set_toast("Setup saved to secrets.toml (0600) & monoagens.toml", false);
        vec![AppAction::CompleteSetupWizard]
    }

    fn save_wizard_provider(&mut self, w: &SetupWizardState, provider: &str) -> Result<(), String> {
        if !w.api_key.trim().is_empty() {
            monoagens_gateway::write_provider_key(&self.paths.secrets_path, provider, w.api_key.trim())
                .map_err(|e| format!("failed to write secrets: {e}"))?;
        }
        let base = if provider == "custom" && !w.endpoint.trim().is_empty() {
            json!(w.endpoint.trim())
        } else {
            serde_json::Value::Null
        };
        let patch = json!({ "model": {
            "provider": provider,
            "model": w.model.trim(),
            "provider_options": { "base_url": base },
        }});
        monoagens_gateway::apply_config_patch_to_file(&self.paths.config_path, &self.paths.secrets_path, &patch)
            .map_err(|e| e.to_string())
    }

    fn save_wizard_legacy(&mut self, w: &SetupWizardState) -> Result<(), String> {
        if !w.endpoint.trim().is_empty() {
            save_secret_key(&self.paths.secrets_path, "model_endpoint", &w.endpoint)?;
        }
        if !w.api_key.trim().is_empty() {
            save_secret_key(&self.paths.secrets_path, "model_api_key", &w.api_key)?;
            self.secrets_summary.model_api_key_set = true;
        }
        if !w.endpoint.trim().is_empty() {
            self.secrets_summary.model_endpoint = Some(w.endpoint.trim().to_owned());
        }
        if !w.model.trim().is_empty() {
            self.gateway_cfg.model.model = w.model.trim().to_owned();
            self.model_name = self.gateway_cfg.model.model.clone();
        }
        let _ = write_or_update_gateway_config(&self.paths.config_path, &self.paths.secrets_path, &self.gateway_cfg);
        Ok(())
    }
}

fn w_sel_index(state: &TuiAppState) -> usize {
    state.wizard.as_ref().map_or(0, |w| w.sel)
}

/// Default and seed models of a catalog provider (when discovery is not possible).
#[must_use]
pub fn seed_models(provider: &str) -> Vec<String> {
    let Some(e) = catalog::find(provider) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if !e.default_model.is_empty() {
        out.push(e.default_model.clone());
    }
    for m in &e.seed_models {
        if !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// Draws the wizard.
pub fn render_wizard(area: Rect, buf: &mut Buffer, w: &SetupWizardState, palette: &ThemePalette, hit: &mut HitRegions) {
    let title = match w.step {
        WizardStep::Step1Provider => "MonoAgens Setup 1/4: Provider",
        WizardStep::StepEndpoint => "MonoAgens Setup 2/4: Endpoint (custom provider)",
        WizardStep::StepKey => "MonoAgens Setup 2/4: API key",
        WizardStep::StepModel => "MonoAgens Setup 3/4: Model",
        WizardStep::Step2Plugins => "MonoAgens Setup 4/4: Plugins",
    };
    let bx = centered(78.min(area.width), 24.min(area.height), area);
    let inner = boxed(bx, buf, title, palette);
    if inner.width < 20 || inner.height < 8 {
        return;
    }
    let dim = Style::default().fg(palette.dim).bg(palette.modal_bg);
    let normal = Style::default().fg(palette.fg).bg(palette.modal_bg);
    let accent = Style::default().fg(palette.accent).bg(palette.modal_bg).add_modifier(Modifier::BOLD);
    let width = inner.width.saturating_sub(2) as usize;
    let list_area = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(4));
    match w.step {
        WizardStep::Step1Provider => {
            buf.set_string(inner.x + 1, inner.y, format!("Search: {}▏", w.search), accent);
            let rows: Vec<ListRow> = w
                .filtered_providers()
                .iter()
                .map(|p| ListRow {
                    left: p.label.clone(),
                    right: if p.keyless { format!("{} (no key needed)", p.id) } else { p.id.clone() },
                    dim: false,
                })
                .collect();
            let mut hits = Vec::new();
            render_list(list_area, buf, &rows, w.sel, w.off, palette, &mut hits);
            hit.wizard_buttons.extend(hits);
            buf.set_string(inner.x + 1, inner.y + inner.height - 1, "Enter choose · type to search · Esc skip setup", dim);
        }
        WizardStep::StepEndpoint => {
            buf.set_string(inner.x + 1, inner.y + 1, "Endpoint URL of your OpenAI-compatible server:", normal);
            buf.set_string(inner.x + 1, inner.y + 3, format!("{}▏", trunc(&w.endpoint, width)), accent);
            if let Some(e) = &w.error {
                buf.set_string(inner.x + 1, inner.y + 5, trunc(e, width), Style::default().fg(palette.status_red).bg(palette.modal_bg));
            }
            buf.set_string(inner.x + 1, inner.y + inner.height - 1, "Enter next · Esc back", dim);
        }
        WizardStep::StepKey => {
            let name = w.provider.as_deref().unwrap_or("provider");
            buf.set_string(inner.x + 1, inner.y + 1, format!("API key for {name} (stored in secrets.toml, mode 0600):"), normal);
            buf.set_string(inner.x + 1, inner.y + 3, format!("{}▏", w.masked_api_key()), accent);
            let (msg, st) = match &w.key_phase {
                WizardKeyPhase::Editing => (String::new(), dim),
                WizardKeyPhase::Testing => ("Testing the connection...".to_owned(), Style::default().fg(palette.status_yellow).bg(palette.modal_bg)),
                WizardKeyPhase::Failed(m) => (m.clone(), Style::default().fg(palette.status_red).bg(palette.modal_bg)),
                WizardKeyPhase::Ok { models, latency_ms } => (
                    format!("Connected: {models} models, {latency_ms} ms. Enter continues."),
                    Style::default().fg(palette.status_green).bg(palette.modal_bg),
                ),
            };
            buf.set_string(inner.x + 1, inner.y + 5, trunc(&msg, width), st);
            buf.set_string(inner.x + 1, inner.y + inner.height - 1, "Enter test · Ctrl+S continue without test · Esc back", dim);
        }
        WizardStep::StepModel => {
            buf.set_string(inner.x + 1, inner.y, format!("Model (type to search, Enter picks): {}▏", w.model_search), accent);
            let rows: Vec<ListRow> = w
                .filtered_models()
                .iter()
                .map(|m| ListRow { left: (*m).clone(), right: String::new(), dim: false })
                .collect();
            let mut hits = Vec::new();
            render_list(list_area, buf, &rows, w.model_sel, w.model_off, palette, &mut hits);
            hit.wizard_buttons.extend(hits);
            buf.set_string(inner.x + 1, inner.y + inner.height - 1, "Enter pick (typed text is used when nothing matches) · Esc back", dim);
        }
        WizardStep::Step2Plugins => {
            buf.set_string(inner.x + 1, inner.y, "Enable plugins (Space toggles, Enter finishes, Left goes back):", normal);
            let rows: Vec<ListRow> = w
                .plugin_ids
                .iter()
                .zip(&w.plugin_enabled)
                .map(|(id, on)| ListRow { left: id.clone(), right: if *on { "● on".to_owned() } else { "○ off".to_owned() }, dim: false })
                .collect();
            let mut hits = Vec::new();
            render_list(list_area, buf, &rows, w.plugin_cursor, 0, palette, &mut hits);
            hit.wizard_buttons.extend(hits);
            if let Some(e) = &w.error {
                buf.set_string(inner.x + 1, inner.y + inner.height - 2, trunc(e, width), Style::default().fg(palette.status_red).bg(palette.modal_bg));
            }
            buf.set_string(inner.x + 1, inner.y + inner.height - 1, "Enter finish setup · Esc skip", dim);
        }
    }
}
