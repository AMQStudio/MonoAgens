//! `/usage`: tokens in and out per purpose for today (UTC), the daily budget left and the cost
//! when every involved model has a known price.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use serde_json::Value;

use crate::catalog_ui::{CatalogUi, Screen, footer_bar, put, title_bar};
use crate::config::IconMode;
use crate::state::AppAction;
use crate::theme::ThemePalette;

/// `/usage` state.
#[derive(Debug, Clone, Default)]
pub struct UsageView {
    pub data: Option<Value>,
    pub error: Option<String>,
    pub offset: usize,
}

impl UsageView {
    pub(crate) fn on_loaded(&mut self, res: Result<Value, String>) {
        match res {
            Ok(v) => {
                self.data = Some(v);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The lines of the view.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if let Some(e) = &self.error {
            return vec![format!("Could not read usage: {e}")];
        }
        let Some(v) = &self.data else {
            return vec!["Loading…".to_owned()];
        };
        let used = v.pointer("/budget/used").and_then(Value::as_u64).unwrap_or(0);
        let limit = v.pointer("/budget/limit").and_then(Value::as_u64).unwrap_or(0);
        let left = v.pointer("/budget/left").and_then(Value::as_u64).unwrap_or(0);
        let mut out = vec![
            format!("Daily token budget: {used} used of {limit}, {left} left"),
            String::new(),
            format!(
                "{:<12} {:<28} {:>6} {:>10} {:>10} {:>9}",
                "purpose", "provider / model", "calls", "tokens in", "tokens out", "cost"
            ),
        ];
        let rows = v.get("purposes").and_then(Value::as_array).cloned().unwrap_or_default();
        if rows.is_empty() {
            out.push("(no model calls yet today)".to_owned());
        }
        for r in &rows {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cost = r
                .get("cost_usd")
                .and_then(Value::as_f64)
                .map_or_else(|| "n/a".to_owned(), fmt_cost);
            out.push(format!(
                "{:<12} {:<28} {:>6} {:>10} {:>10} {:>9}",
                s("purpose"),
                format!("{}/{}", s("provider"), s("model")),
                n("calls"),
                n("input_tokens"),
                n("output_tokens"),
                cost
            ));
        }
        out.push(String::new());
        out.push(match v.get("cost_usd").and_then(Value::as_f64) {
            Some(c) => format!("Cost today: {}", fmt_cost(c)),
            None => "Cost today: unknown (no price for a model in use; see [model.prices])".to_owned(),
        });
        out
    }
}

fn fmt_cost(c: f64) -> String {
    if c > 0.0 && c < 0.01 {
        format!("${c:.4}")
    } else {
        format!("${c:.2}")
    }
}

impl CatalogUi {
    pub(crate) fn usage_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let total = self.usage.lines().len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Closed,
            KeyCode::Up => self.usage.offset = self.usage.offset.saturating_sub(1),
            KeyCode::Down => self.usage.offset = (self.usage.offset + 1).min(total.saturating_sub(1)),
            KeyCode::Char('r') => return vec![AppAction::Catalog(crate::catalog_ui::CatalogAction::LoadUsage)],
            _ => {}
        }
        Vec::new()
    }
}

/// Draws the view.
pub fn render(area: Rect, buf: &mut Buffer, ui: &CatalogUi, palette: &ThemePalette, icons: IconMode) {
    title_bar(area, buf, " Usage today (UTC)", palette, icons);
    let w = area.width as usize;
    let visible = (area.height as usize).saturating_sub(2);
    for (i, line) in ui.usage.lines().iter().skip(ui.usage.offset).take(visible).enumerate() {
        put(
            buf,
            area.x,
            area.y + 1 + i as u16,
            w,
            &format!(" {line}"),
            Style::default().fg(palette.fg).bg(palette.bg),
            icons,
        );
    }
    footer_bar(area, buf, " r refresh · ↑/↓ scroll · ⎋ close", palette, icons);
}
