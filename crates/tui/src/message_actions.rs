//! Per-turn "what happened" summaries, message selection and the `/why` overlay.
//!
//! The data comes from the event log of the turn's trace through the admin methods
//! `events.trace` (rows of the trace) and `events.explain` (the valve/decision explanation that
//! `monoagens log --explain` prints). Nothing here reads payload bytes.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde_json::Value;

use crate::catalog_ui::pad;
use crate::state::ChatTimelineItem;
use crate::theme::ThemePalette;

/// Collapsed/expanded view of what one turn did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnSummary {
    pub turn: u64,
    /// The event the turn's trace is rooted at (the percept).
    pub root_event: Option<u64>,
    pub model_calls: usize,
    pub steps: usize,
    pub verdicts: usize,
    pub rows: Vec<String>,
    pub expanded: bool,
    /// The trace was fetched (an empty summary before that).
    pub loaded: bool,
    pub error: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl TurnSummary {
    #[must_use]
    pub fn pending(turn: u64, root_event: Option<u64>) -> Self {
        Self {
            turn,
            root_event,
            ..Self::default()
        }
    }

    /// Builds the summary from `events.trace` rows (`{kind, summary, ...}`, log order).
    #[must_use]
    pub fn from_rows(turn: u64, root_event: Option<u64>, rows: &[Value], expanded: bool) -> Self {
        let kind = |r: &Value| r.get("kind").and_then(Value::as_str).unwrap_or("").to_owned();
        let summary = |r: &Value| r.get("summary").and_then(Value::as_str).unwrap_or("").to_owned();
        // A reasoning call starts a wake (`wake_started`); a recorded `model_call` event, when a
        // build emits one, describes the same call.
        let recorded = rows.iter().filter(|r| kind(r) == "model_call").count();
        let wakes = rows.iter().filter(|r| kind(r) == "wake_started").count();
        let model_calls = recorded.max(wakes);
        let steps = rows.iter().filter(|r| kind(r) == "action").count();
        let verdicts = rows.iter().filter(|r| kind(r) == "door_verdict").count();
        let mut lines = Vec::new();
        for r in rows {
            match kind(r).as_str() {
                "model_call" => lines.push(format!("model call  {}", summary(r))),
                "wake_started" if recorded == 0 => lines.push("model call  (reasoning wake)".to_owned()),
                "action" => {
                    let valve = if verdicts == 0 { "valve: allowed" } else { "valve: see verdicts" };
                    lines.push(format!("{}  ({valve})", summary(r)));
                }
                "door_verdict" => lines.push(format!("valve  {}", summary(r))),
                "withheld" | "failure" => lines.push(summary(r)),
                _ => {}
            }
        }
        Self {
            turn,
            root_event,
            model_calls,
            steps,
            verdicts,
            rows: lines,
            expanded,
            loaded: true,
            error: None,
        }
    }

    /// The single collapsed line.
    #[must_use]
    pub fn collapsed_line(&self) -> String {
        if let Some(e) = &self.error {
            return format!("▸ what happened: unavailable ({e})");
        }
        if !self.loaded {
            return "▸ what happened: loading…".to_owned();
        }
        let valve = if self.verdicts == 0 {
            "valve allowed".to_owned()
        } else {
            plural(self.verdicts, "valve verdict", "valve verdicts")
        };
        format!(
            "▸ {} · {} · {valve}  (Ctrl+O)",
            plural(self.model_calls, "model call", "model calls"),
            plural(self.steps, "action", "actions"),
        )
    }

    /// The lines to show: the collapsed line, or the header plus one row per item.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if !self.expanded || !self.loaded {
            return vec![self.collapsed_line()];
        }
        let mut out = vec!["▾ what happened  (Ctrl+O to collapse)".to_owned()];
        for r in &self.rows {
            out.push(format!("  · {r}"));
        }
        if self.rows.is_empty() {
            out.push("  · nothing recorded for this turn".to_owned());
        }
        out
    }
}

/// `> `-quoted text, one prefix per line.
#[must_use]
pub fn quote_text(text: &str) -> String {
    text.lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text of a user or assistant message (other timeline items are not selectable).
#[must_use]
pub fn message_text(item: &ChatTimelineItem) -> Option<&str> {
    match item {
        ChatTimelineItem::UserMessage { text, .. } | ChatTimelineItem::AssistantMessage { text, .. } => {
            Some(text.as_str())
        }
        _ => None,
    }
}

/// Indices of the selectable messages of a timeline.
#[must_use]
pub fn message_indices(timeline: &[ChatTimelineItem]) -> Vec<usize> {
    timeline
        .iter()
        .enumerate()
        .filter(|(_, it)| message_text(it).is_some())
        .map(|(i, _)| i)
        .collect()
}

/// Moves a selection through the messages; `delta < 0` goes to older ones. No selection starts
/// at the newest message; the ends do not wrap.
#[must_use]
pub fn move_selection(current: Option<usize>, indices: &[usize], delta: i32) -> Option<usize> {
    if indices.is_empty() {
        return None;
    }
    let pos = current.and_then(|c| indices.iter().position(|i| *i == c));
    let next = match pos {
        None => indices.len() - 1,
        Some(p) if delta < 0 => p.saturating_sub(1),
        Some(p) => (p + 1).min(indices.len() - 1),
    };
    Some(indices[next])
}

/// The `/why` overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhyOverlay {
    pub title: String,
    pub lines: Vec<String>,
    pub loading: bool,
    pub scroll: usize,
}

impl WhyOverlay {
    #[must_use]
    pub fn loading(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            lines: vec!["Loading the valve explanation…".to_owned()],
            loading: true,
            scroll: 0,
        }
    }

    /// Fills the overlay from an `events.explain` result (its `lines` are exactly what
    /// `monoagens log --explain` prints).
    pub fn set_explain(&mut self, result: &Value) {
        self.loading = false;
        self.lines = result
            .get("lines")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        if self.lines.is_empty() {
            self.lines = vec!["No explanation recorded.".to_owned()];
        }
    }

    pub fn set_error(&mut self, message: &str) {
        self.loading = false;
        self.lines = vec![format!("No explanation available: {message}")];
    }
}

/// Draws the overlay centred over `area`.
pub fn render_why(area: Rect, buf: &mut Buffer, why: &WhyOverlay, palette: &ThemePalette) {
    if area.width < 30 || area.height < 7 {
        return;
    }
    let w = area.width.saturating_sub(6).min(120);
    let h = (why.lines.len() as u16 + 4).min(area.height.saturating_sub(2)).max(5);
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let body = Style::default().fg(palette.fg).bg(palette.modal_bg);
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(body);
            }
        }
    }
    let inner = rect.width.saturating_sub(4) as usize;
    buf.set_string(
        rect.x + 2,
        rect.y,
        pad(&format!(" Why: {} ", why.title), inner),
        Style::default()
            .fg(palette.accent)
            .bg(palette.modal_bg)
            .add_modifier(Modifier::BOLD),
    );
    let rows = (rect.height as usize).saturating_sub(3);
    let scroll = why.scroll.min(why.lines.len().saturating_sub(rows));
    for (i, line) in why.lines.iter().skip(scroll).take(rows).enumerate() {
        buf.set_string(rect.x + 2, rect.y + 2 + i as u16, pad(line, inner), body);
    }
    buf.set_string(
        rect.x + 2,
        rect.y + rect.height - 1,
        pad("Esc close", inner),
        Style::default().fg(palette.dim).bg(palette.modal_bg),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_counts_and_expands_one_row_per_item() {
        let rows = vec![
            json!({"kind":"percept","summary":"percept"}),
            json!({"kind":"model_call","summary":"model_call"}),
            json!({"kind":"action","summary":"action chat.speak (act_1)"}),
        ];
        let mut s = TurnSummary::from_rows(1, Some(5), &rows, false);
        assert_eq!((s.model_calls, s.steps, s.verdicts), (1, 1, 0));
        assert_eq!(s.lines().len(), 1);
        assert!(s.collapsed_line().contains("1 model call · 1 action · valve allowed"));
        s.expanded = true;
        let l = s.lines();
        assert_eq!(l.len(), 3, "header + model call + action: {l:?}");
        assert!(l[1].contains("model call") && l[2].contains("chat.speak") && l[2].contains("valve: allowed"));
    }

    #[test]
    fn quote_prefixes_every_line() {
        assert_eq!(quote_text("a\nb\n\nc"), "> a\n> b\n> \n> c");
    }

    #[test]
    fn selection_walks_messages_without_wrapping() {
        let idx = [1, 3, 4];
        assert_eq!(move_selection(None, &idx, -1), Some(4));
        assert_eq!(move_selection(Some(4), &idx, -1), Some(3));
        assert_eq!(move_selection(Some(1), &idx, -1), Some(1));
        assert_eq!(move_selection(Some(4), &idx, 1), Some(4));
        assert_eq!(move_selection(None, &[], 1), None);
    }
}
