//! The TUI side of the live draft preview (owner decision D-3).
//!
//! A draft is shown as a dim "draft" bubble while the model writes, and replaced by the real
//! outcome when the turn resolves. It is display state only: never stored in a timeline or in
//! `tui_chats.json`, never exported, never sent anywhere.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::ThemePalette;

/// The draft of the running turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Model request the draft belongs to.
    pub request_id: u64,
    pub text: String,
}

/// Subscription bookkeeping and the current draft.
#[derive(Debug, Clone, Default)]
pub struct DraftState {
    pub draft: Option<Draft>,
    /// Subscription id at the gateway, when subscribed.
    pub subscription: Option<u64>,
    /// A subscribe/unsubscribe call is in flight.
    pub busy: bool,
    /// Requests whose draft must not be shown again (cancelled, failed or resolved).
    closed: Vec<u64>,
}

impl DraftState {
    /// A draft update for the running turn. Ignored when no turn is running (`busy_turn`), for
    /// an already closed request, or for another request than the one being shown.
    pub fn update(&mut self, request_id: u64, text: String, busy_turn: bool) {
        if !busy_turn || self.closed.contains(&request_id) {
            return;
        }
        match &mut self.draft {
            Some(d) if d.request_id != request_id => {
                // A newer call of the same turn (a retry inside the provider): start over.
                *d = Draft { request_id, text };
            }
            Some(d) => d.text = text,
            None => self.draft = Some(Draft { request_id, text }),
        }
    }

    /// The streamed call ended without a result (`failed`/`cancelled`): the draft disappears.
    /// A completed call keeps it until the real outcome is known.
    pub fn end(&mut self, request_id: u64, completed: bool) {
        if completed {
            return;
        }
        self.close(request_id);
    }

    fn close(&mut self, request_id: u64) {
        if self.draft.as_ref().is_some_and(|d| d.request_id == request_id) {
            self.draft = None;
        }
        if !self.closed.contains(&request_id) {
            if self.closed.len() >= 64 {
                self.closed.remove(0);
            }
            self.closed.push(request_id);
        }
    }

    /// The turn resolved (reply, verdict, silence, cancel, failure): the draft is removed and
    /// its request will not be shown again.
    pub fn resolve(&mut self) {
        if let Some(d) = self.draft.take() {
            self.close(d.request_id);
        }
    }

    /// Lines of the dim draft bubble (empty when there is no draft).
    #[must_use]
    pub fn lines(&self, width: usize, palette: &ThemePalette) -> Vec<Line<'static>> {
        let Some(d) = &self.draft else {
            return Vec::new();
        };
        let dim = Style::default().fg(palette.dim);
        let mut out = vec![Line::from(Span::styled(
            "draft ┄ not final, the valve has not seen it yet",
            dim.add_modifier(Modifier::ITALIC),
        ))];
        for l in wrap(&d.text, width.saturating_sub(2).max(4)) {
            out.push(Line::from(vec![
                Span::styled("┆ ", dim),
                Span::styled(l, dim),
            ]));
        }
        out.push(Line::default());
        out
    }
}

/// Greedy wrap by display width; control characters are replaced.
fn wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut out = Vec::new();
    for raw in crate::width::sanitize(text, true).split('\n') {
        let mut line = String::new();
        let mut used = 0usize;
        for c in raw.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(c);
            used += w;
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_only_for_the_running_turn_and_close_for_good() {
        let mut s = DraftState::default();
        s.update(7, "hel".into(), false);
        assert!(s.draft.is_none(), "no running turn: ignored");
        s.update(7, "hel".into(), true);
        s.update(7, "hello".into(), true);
        assert_eq!(s.draft.as_ref().unwrap().text, "hello");
        s.end(7, true);
        assert!(s.draft.is_some(), "a completed call keeps the draft until the outcome is known");
        s.end(7, false);
        assert!(s.draft.is_none());
        s.update(7, "late".into(), true);
        assert!(s.draft.is_none(), "a closed request never shows again");
        s.update(8, "new".into(), true);
        s.resolve();
        assert!(s.draft.is_none());
        s.update(8, "again".into(), true);
        assert!(s.draft.is_none());
    }
}
