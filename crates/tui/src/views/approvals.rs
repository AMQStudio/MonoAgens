//! `/approvals`: every pending approval (TUI and adapter-originated holds), resolved through
//! the same `AppAction::ResolveApproval` path as the inline card.

use crossterm::event::{KeyCode, KeyEvent};
use monoagens_protocol::{ActionId, HAct};
use serde_json::{Value, json};

use super::{GovRequest, GovTag, Nav, Row, Tone, ViewModel, age_label, jstr, ju64};
use crate::state::AppAction;

/// One pending approval.
#[derive(Debug, Clone)]
pub struct ApprovalItem {
    pub action_id: String,
    pub capability: String,
    pub places: Vec<String>,
    pub requester: String,
    pub origin: String,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    pub taint: String,
    pub reason: String,
    pub h_act: String,
    pub state_epoch: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ApprovalsView {
    pub items: Vec<ApprovalItem>,
    pub nav: Nav,
    /// Pending confirmation: `(approve, action_id)`.
    pub confirm: Option<(bool, String)>,
    pub expanded: bool,
    pub loaded: bool,
}

#[must_use]
pub fn request() -> GovRequest {
    GovRequest::new(GovTag::Approvals, "approvals.list", &json!({}))
}

impl ApprovalsView {
    pub fn set(&mut self, v: &Value) {
        self.items = v
            .get("approvals")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|e| ApprovalItem {
                        action_id: jstr(e, "action_id"),
                        capability: jstr(e, "capability_id"),
                        places: e
                            .get("places")
                            .and_then(Value::as_array)
                            .map(|p| p.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                            .unwrap_or_default(),
                        requester: jstr(e, "requester"),
                        origin: jstr(e, "origin"),
                        timestamp_ms: ju64(e, "timestamp_ms"),
                        expires_at_ms: ju64(e, "expires_at_ms"),
                        taint: jstr(e, "taint"),
                        reason: e.get("reason_code").and_then(Value::as_str).unwrap_or("PendingApproval").to_owned(),
                        h_act: jstr(e, "h_act"),
                        state_epoch: ju64(e, "state_epoch"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.loaded = true;
        self.nav.clamp(self.items.len());
        if let Some((_, id)) = &self.confirm
            && !self.items.iter().any(|i| &i.action_id == id)
        {
            self.confirm = None;
        }
    }

    /// Selects rendered row `row_idx` (mapping expanded detail rows back to their item) and
    /// keeps it visible within `visible` rows.
    pub fn select_row(&mut self, row_idx: usize, visible: usize) {
        if self.items.is_empty() {
            self.nav = Nav::default();
            return;
        }
        let mut cursor = 0usize;
        let mut target_item = self.items.len() - 1;
        let mut target_row = 0usize;
        let mut total_rows = 0usize;
        for (i, _) in self.items.iter().enumerate() {
            let item_start = cursor;
            cursor += 1;
            if self.expanded && i == self.nav.selected {
                cursor += 4;
            }
            if row_idx >= item_start && row_idx < cursor {
                target_item = i;
                target_row = item_start;
            }
            total_rows = cursor;
        }
        self.nav.selected = target_item;
        self.nav.offset = crate::core::viewport::scroll_offset_for_row(
            target_row,
            self.nav.offset,
            visible.max(1),
            total_rows,
        );
    }

    pub fn cancel_confirm(&mut self) -> bool {
        self.confirm.take().is_some()
    }

    pub fn handle_key(&mut self, key: KeyEvent, visible: usize, _now: u64) -> Vec<AppAction> {
        if let Some((approve, id)) = self.confirm.clone() {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.confirm = None;
                    return self.resolve(&id, approve);
                }
                KeyCode::Char('n' | 'N') => self.confirm = None,
                _ => {}
            }
            return Vec::new();
        }
        if self.nav.handle_key(key, self.items.len(), visible) {
            return Vec::new();
        }
        match key.code {
            KeyCode::Enter => self.expanded = !self.expanded,
            KeyCode::Char('g') => self.ask(true),
            KeyCode::Char('d') => self.ask(false),
            KeyCode::Char('r') => return vec![AppAction::Gov(request())],
            _ => {}
        }
        Vec::new()
    }

    fn ask(&mut self, approve: bool) {
        if let Some(i) = self.items.get(self.nav.selected) {
            self.confirm = Some((approve, i.action_id.clone()));
        }
    }

    /// Resolves through the existing approval action (signed and sent by the runtime) and
    /// refreshes the list afterwards.
    fn resolve(&mut self, id: &str, approved: bool) -> Vec<AppAction> {
        let Some(item) = self.items.iter().find(|i| i.action_id == id).cloned() else {
            return Vec::new();
        };
        let h_act: HAct = serde_json::from_value(Value::String(item.h_act.clone())).unwrap_or_else(|_| HAct::new(item.h_act.clone()));
        self.items.retain(|i| i.action_id != id);
        self.nav.clamp(self.items.len());
        vec![
            AppAction::ResolveApproval {
                action_id: ActionId::new(item.action_id),
                h_act,
                state_epoch: item.state_epoch,
                approved,
            },
            AppAction::Gov(request()),
        ]
    }

    #[must_use]
    pub fn view(&self, now: u64) -> ViewModel {
        let mut rows = Vec::new();
        let mut sel = None;
        for (i, it) in self.items.iter().enumerate() {
            if i == self.nav.selected {
                sel = Some(rows.len());
            }
            rows.push(Row::new(
                format!(
                    "{:<18} {:<14} {:<18} {:<12} {:>4}",
                    it.capability,
                    it.places.first().map_or("-", String::as_str),
                    it.requester,
                    it.origin,
                    age_label(now, it.timestamp_ms)
                ),
                Tone::Warn,
            ));
            if self.expanded && i == self.nav.selected {
                rows.push(Row::new(format!("    action {}", it.action_id), Tone::Dim));
                rows.push(Row::new(
                    format!("    taint {}  reason {}  epoch {}", it.taint, it.reason, it.state_epoch),
                    Tone::Dim,
                ));
                rows.push(Row::new(
                    format!(
                        "    places {}  expires in {}",
                        it.places.join(","),
                        age_label(it.expires_at_ms, now)
                    ),
                    Tone::Dim,
                ));
                rows.push(Row::new(format!("    h_act {}", it.h_act), Tone::Dim));
            }
        }
        let footer = match &self.confirm {
            Some((a, id)) => format!(" {} {id}?  y confirm  n cancel", if *a { "Grant" } else { "Deny" }),
            None => " Enter detail  g grant  d deny  r refresh  Esc close".to_owned(),
        };
        ViewModel {
            title: "Approvals".to_owned(),
            header: format!(
                "{} pending   capability / place / requester / origin / age",
                self.items.len()
            ),
            rows,
            selected: sel,
            offset: self.nav.offset,
            footer,
            empty: if self.loaded { "No pending approvals.".to_owned() } else { "Loading...".to_owned() },
        }
    }
}
