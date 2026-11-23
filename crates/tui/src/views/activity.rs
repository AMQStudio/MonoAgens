//! `/activity`: read-only feed of adapter traffic (message in -> turn -> outcome), rebuilt by
//! the gateway from the event log. It has no composer and no way to send input or actions.

use crossterm::event::{KeyCode, KeyEvent};
use serde_json::{Value, json};

use super::{GovRequest, GovTag, Nav, Prompt, PromptFor, Row, Tone, ViewModel, clock_label, jstr, ju64};
use crate::state::AppAction;

#[derive(Debug, Clone)]
pub struct ActivityItem {
    pub trace: u64,
    pub timestamp_ms: u64,
    pub plugin: String,
    pub place: String,
    pub actor: String,
    pub summary: String,
    pub outcome: String,
}

pub const OUTCOMES: [&str; 5] = ["replied", "held", "denied", "silent", "failed"];

#[derive(Debug, Clone, Default)]
pub struct ActivityView {
    pub rows: Vec<ActivityItem>,
    pub nav: Nav,
    pub plugin: Option<String>,
    pub place: Option<String>,
    pub outcome: Option<String>,
    /// Type-to-search text (matches any column).
    pub search: String,
    pub loaded: bool,
}

#[must_use]
pub fn request() -> GovRequest {
    GovRequest::new(GovTag::Activity, "activity.list", &json!({ "limit": 500 }))
}

fn cycle(cur: &Option<String>, all: &[String]) -> Option<String> {
    match cur {
        None => all.first().cloned(),
        Some(c) => all.iter().position(|x| x == c).and_then(|i| all.get(i + 1)).cloned(),
    }
}

impl ActivityView {
    pub fn set(&mut self, v: &Value) {
        self.rows = v
            .get("rows")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|r| ActivityItem {
                        trace: ju64(r, "trace"),
                        timestamp_ms: ju64(r, "timestamp_ms"),
                        plugin: jstr(r, "plugin"),
                        place: jstr(r, "place"),
                        actor: jstr(r, "actor"),
                        summary: jstr(r, "summary"),
                        outcome: jstr(r, "outcome"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.loaded = true;
        let n = self.filtered().len();
        self.nav.clamp(n);
    }

    /// Rows passing the plugin, place, outcome and search filters (newest last).
    #[must_use]
    pub fn filtered(&self) -> Vec<&ActivityItem> {
        let needle = self.search.to_lowercase();
        self.rows
            .iter()
            .filter(|r| self.plugin.as_deref().is_none_or(|p| r.plugin == p))
            .filter(|r| self.place.as_deref().is_none_or(|p| r.place == p))
            .filter(|r| self.outcome.as_deref().is_none_or(|o| r.outcome == o))
            .filter(|r| {
                needle.is_empty()
                    || format!("{} {} {} {} {}", r.plugin, r.place, r.actor, r.summary, r.outcome)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    fn distinct(&self, f: impl Fn(&ActivityItem) -> &String) -> Vec<String> {
        let mut v: Vec<String> = self.rows.iter().map(|r| f(r).clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn handle_key(&mut self, key: KeyEvent, visible: usize, prompt: &mut Option<Prompt>) -> Vec<AppAction> {
        let total = self.filtered().len();
        if self.nav.handle_key(key, total, visible) {
            return Vec::new();
        }
        match key.code {
            KeyCode::Char('p') => {
                self.plugin = cycle(&self.plugin, &self.distinct(|r| &r.plugin));
                self.nav = Nav::default();
            }
            KeyCode::Char('l') => {
                self.place = cycle(&self.place, &self.distinct(|r| &r.place));
                self.nav = Nav::default();
            }
            KeyCode::Char('o') => {
                let all: Vec<String> = OUTCOMES.iter().map(|s| (*s).to_owned()).collect();
                self.outcome = cycle(&self.outcome, &all);
                self.nav = Nav::default();
            }
            KeyCode::Char('/') => {
                *prompt = Some(Prompt {
                    label: "search".to_owned(),
                    text: self.search.clone(),
                    purpose: PromptFor::ActivitySearch,
                });
            }
            KeyCode::Char('x') => {
                self.plugin = None;
                self.place = None;
                self.outcome = None;
                self.search.clear();
                self.nav = Nav::default();
            }
            KeyCode::Char('r') => return vec![AppAction::Gov(request())],
            KeyCode::Enter => {
                if let Some(r) = self.filtered().get(self.nav.selected) {
                    return vec![AppAction::Gov(GovRequest::new(
                        GovTag::Explain,
                        "events.explain",
                        &json!({ "id": r.trace }),
                    ))];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    #[must_use]
    pub fn view(&self, _now: u64) -> ViewModel {
        let rows: Vec<Row> = self
            .filtered()
            .iter()
            .map(|r| {
                let tone = match r.outcome.as_str() {
                    "replied" => Tone::Good,
                    "held" | "denied" => Tone::Warn,
                    "failed" => Tone::Bad,
                    _ => Tone::Dim,
                };
                Row::new(
                    format!(
                        "{} {:<9} {:<18} {:<14} {}  {}",
                        clock_label(r.timestamp_ms),
                        r.plugin,
                        r.place,
                        r.actor,
                        r.summary,
                        r.outcome
                    ),
                    tone,
                )
            })
            .collect();
        let total = rows.len();
        ViewModel {
            title: "Adapter activity (read-only)".to_owned(),
            header: format!(
                "{}/{}  plugin:{}  place:{}  outcome:{}{}",
                total,
                self.rows.len(),
                self.plugin.as_deref().unwrap_or("all"),
                self.place.as_deref().unwrap_or("all"),
                self.outcome.as_deref().unwrap_or("all"),
                if self.search.is_empty() { String::new() } else { format!("  /{}", self.search) }
            ),
            rows,
            selected: Some(self.nav.selected),
            offset: self.nav.offset,
            footer: " p plugin  l place  o outcome  / search  x clear  Enter explain  Esc close".to_owned(),
            empty: if self.loaded { "No adapter activity yet.".to_owned() } else { "Loading...".to_owned() },
        }
    }
}
