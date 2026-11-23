//! `/status`: gateway owner, data dir, provider and model, embedder, plugins, queue and budget.

use crossterm::event::KeyEvent;
use serde_json::{Value, json};

use super::{GovRequest, GovTag, Nav, Row, Tone, ViewModel};
use crate::state::AppAction;

#[derive(Debug, Clone, Default)]
pub struct StatusView {
    pub raw: Option<Value>,
    /// `hosting` or `attached` (set by the state from the client kind).
    pub link: String,
    pub nav: Nav,
}

#[must_use]
pub fn request() -> GovRequest {
    GovRequest::new(GovTag::Status, "status", &json!({}))
}


fn s(v: &Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => "-".to_owned(),
        Some(o) => o.to_string(),
    }
}

impl StatusView {
    pub fn handle_key(&mut self, key: KeyEvent, visible: usize) -> Vec<AppAction> {
        let total = self.view().rows.len();
        self.nav.handle_key(key, total, visible);
        Vec::new()
    }

    pub fn set(&mut self, v: &Value) {
        self.raw = Some(v.clone());
    }

    #[must_use]
    pub fn view(&self) -> ViewModel {
        let mut rows = Vec::new();
        if let Some(v) = &self.raw {
            let kv = |k: &str, val: String| Row::new(format!("  {k:<12} {val}"), Tone::Normal);
            rows.push(Row::new("Gateway", Tone::Head));
            rows.push(kv("owner pid", s(v, "/owner/pid")));
            rows.push(kv("mode", s(v, "/owner/mode")));
            rows.push(kv("this TUI", if self.link.is_empty() { "-".to_owned() } else { self.link.clone() }));
            rows.push(kv("data dir", s(v, "/data_dir")));
            rows.push(kv("lock", s(v, "/lock_path")));
            rows.push(Row::new("Model", Tone::Head));
            rows.push(kv("provider", s(v, "/provider")));
            rows.push(kv("model", s(v, "/model")));
            rows.push(kv("embedder", s(v, "/embedder")));
            rows.push(Row::new("Load", Tone::Head));
            rows.push(kv("queue depth", s(v, "/queue_depth")));
            let used = v.pointer("/budget/used").and_then(Value::as_u64).unwrap_or(0);
            let limit = v.pointer("/budget/limit").and_then(Value::as_u64).unwrap_or(0);
            rows.push(kv(
                "budget",
                format!("{used} used / {} left (limit {limit})", limit.saturating_sub(used)),
            ));
            rows.push(Row::new("Plugins", Tone::Head));
            let plugins = v.get("plugins").and_then(Value::as_array).cloned().unwrap_or_default();
            if plugins.is_empty() {
                rows.push(Row::new("  none", Tone::Dim));
            }
            for p in &plugins {
                let state = s(p, "/state");
                let tone = match state.as_str() {
                    "running" => Tone::Good,
                    "failed" => Tone::Bad,
                    _ => Tone::Dim,
                };
                let reason = p.get("reason").and_then(Value::as_str).map(|r| format!(" ({r})")).unwrap_or_default();
                rows.push(Row::new(format!("  {:<14} {state}{reason}", s(p, "/id")), tone));
            }
        }
        ViewModel {
            title: "Status".to_owned(),
            header: String::new(),
            rows,
            selected: Some(self.nav.selected),
            offset: self.nav.offset,
            footer: " Up/Down scroll  Esc close".to_owned(),
            empty: "Loading...".to_owned(),
        }
    }
}
