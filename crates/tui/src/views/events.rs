//! `/events`: the event log with the `monoagens log` filters (place, source, kind, time), paged
//! by cursor, with explain on Enter, a trace view, and (Lab mode only) replay and
//! export-learning jobs that run on a copy of the log.

use crossterm::event::{KeyCode, KeyEvent};
use serde_json::{Value, json};

use super::{Detail, GovRequest, GovTag, Nav, Prompt, PromptFor, Row, Tone, ViewModel, clock_label, jstr, ju64};
use crate::state::AppAction;

#[derive(Debug, Clone)]
pub struct EventItem {
    pub id: u64,
    pub seq: u64,
    pub kind: String,
    pub timestamp_ms: u64,
    pub place: String,
    pub actor: String,
    pub summary: String,
}

/// A running or finished Lab job.
#[derive(Debug, Clone)]
pub struct JobView {
    pub id: u64,
    pub kind: String,
    pub state: String,
    pub done: u64,
    pub total: u64,
}

impl JobView {
    #[must_use]
    pub fn running(&self) -> bool {
        self.state == "running"
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventsView {
    pub items: Vec<EventItem>,
    pub nav: Nav,
    pub place: Option<String>,
    pub source: Option<String>,
    pub kind: Option<String>,
    /// Raw time-range text (`1h`, `1h..10m`, or epoch ms).
    pub range: Option<String>,
    pub next_cursor: Option<u64>,
    pub matching: usize,
    /// Set while the trace view is shown (event id or text).
    pub trace_label: Option<String>,
    pub job: Option<JobView>,
    pub loaded: bool,
}

/// Parses `90s`, `30m`, `1h`, `2d` into milliseconds.
#[must_use]
pub fn parse_relative(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = num.parse().ok()?;
    let mult = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(n * mult)
}

fn parse_point(s: &str, now: u64) -> Option<u64> {
    parse_relative(s).map(|d| now.saturating_sub(d)).or_else(|| s.trim().parse().ok())
}

/// `(since, until)` in epoch ms from the range text.
#[must_use]
pub fn parse_range(text: &str, now: u64) -> Option<(Option<u64>, Option<u64>)> {
    match text.split_once("..") {
        Some((a, b)) => {
            let since = if a.trim().is_empty() { None } else { Some(parse_point(a, now)?) };
            let until = if b.trim().is_empty() { None } else { Some(parse_point(b, now)?) };
            Some((since, until))
        }
        None => Some((Some(parse_point(text, now)?), None)),
    }
}

#[must_use]
pub fn trace_request(trace: &str) -> GovRequest {
    GovRequest::new(GovTag::Trace, "events.trace", &json!({ "trace": trace }))
}

impl EventsView {
    /// `events.query` for the current filters; `more` continues after the last page.
    #[must_use]
    pub fn query_request(&self, now: u64, more: bool) -> GovRequest {
        let mut p = json!({ "limit": 200 });
        if let Some(v) = &self.place {
            p["place"] = json!(v);
        }
        if let Some(v) = &self.source {
            p["source"] = json!(v);
        }
        if let Some(v) = &self.kind {
            p["kind"] = json!(v);
        }
        if let Some((since, until)) = self.range.as_deref().and_then(|r| parse_range(r, now)) {
            if let Some(s) = since {
                p["since"] = json!(s);
            }
            if let Some(u) = until {
                p["until"] = json!(u);
            }
        }
        if more && let Some(c) = self.next_cursor {
            p["cursor"] = json!(c);
        }
        GovRequest::new(if more { GovTag::EventsMore } else { GovTag::Events }, "events.query", &p)
    }

    pub fn set(&mut self, v: &Value, append: bool) {
        let rows = parse_rows(v);
        if append {
            self.items.extend(rows);
        } else {
            self.items = rows;
            self.nav = Nav::default();
        }
        self.next_cursor = v.get("next_cursor").and_then(Value::as_u64);
        self.matching = v.get("matching").and_then(Value::as_u64).map_or(self.items.len(), |n| n as usize);
        self.trace_label = None;
        self.loaded = true;
    }

    /// Selects rendered row `row_idx` and ensures it stays visible.
    pub fn select_row(&mut self, row_idx: usize, visible: usize) {
        let total = self.items.len();
        if total == 0 {
            self.nav = Nav::default();
            return;
        }
        self.nav.selected = row_idx.min(total - 1);
        self.nav.offset = crate::core::viewport::scroll_offset_for_row(
            self.nav.selected,
            self.nav.offset,
            visible.max(1),
            total,
        );
    }

    pub fn set_trace(&mut self, v: &Value) {
        let mut rows = parse_rows(v);
        rows.sort_by_key(|r| r.seq);
        self.items = rows;
        self.nav = Nav::default();
        self.next_cursor = None;
        self.matching = self.items.len();
        self.loaded = true;
    }

    pub fn leave_trace(&mut self) -> bool {
        self.trace_label.take().is_some()
    }

    pub fn set_filter(&mut self, which: &PromptFor, text: &str) -> Result<(), String> {
        let val = if text.is_empty() { None } else { Some(text.to_owned()) };
        match which {
            PromptFor::EventsPlace => self.place = val,
            PromptFor::EventsSource => self.source = val,
            PromptFor::EventsKind => {
                if let Some(k) = &val
                    && !monoagens_core::EventKind::ALL.iter().any(|e| e.as_str() == k)
                {
                    let all: Vec<&str> = monoagens_core::EventKind::ALL.iter().map(|e| e.as_str()).collect();
                    return Err(format!("unknown event kind '{k}'. Kinds: {}", all.join(", ")));
                }
                self.kind = val;
            }
            PromptFor::EventsRange => {
                if let Some(r) = &val
                    && parse_range(r, 0).is_none()
                {
                    return Err("Time range: 30m, 1h, 2d, 1h..10m or epoch milliseconds".to_owned());
                }
                self.range = val;
            }
            _ => {}
        }
        self.trace_label = None;
        Ok(())
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        visible: usize,
        now: u64,
        lab_mode: bool,
        prompt: &mut Option<Prompt>,
    ) -> Vec<AppAction> {
        let total = self.items.len();
        if self.nav.handle_key(key, total, visible) {
            // Scrolling to the end loads the next page.
            if self.trace_label.is_none()
                && self.next_cursor.is_some()
                && total > 0
                && self.nav.selected + 1 >= total
            {
                return vec![AppAction::Gov(self.query_request(now, true))];
            }
            return Vec::new();
        }
        let ask = |label: &str, text: Option<&String>, purpose: PromptFor| Prompt {
            label: label.to_owned(),
            text: text.cloned().unwrap_or_default(),
            purpose,
        };
        match key.code {
            KeyCode::Char('p') => *prompt = Some(ask("place", self.place.as_ref(), PromptFor::EventsPlace)),
            KeyCode::Char('s') => *prompt = Some(ask("source (actor)", self.source.as_ref(), PromptFor::EventsSource)),
            KeyCode::Char('k') => *prompt = Some(ask("kind", self.kind.as_ref(), PromptFor::EventsKind)),
            KeyCode::Char('w') => *prompt = Some(ask("time range (1h, 1h..10m, ms)", self.range.as_ref(), PromptFor::EventsRange)),
            KeyCode::Char('T') => *prompt = Some(ask("trace (event id or text)", None, PromptFor::EventsTrace)),
            KeyCode::Char('x') => {
                self.place = None;
                self.source = None;
                self.kind = None;
                self.range = None;
                return vec![AppAction::Gov(self.query_request(now, false))];
            }
            KeyCode::Char('r') => {
                return vec![AppAction::Gov(match &self.trace_label {
                    Some(t) => trace_request(t),
                    None => self.query_request(now, false),
                })];
            }
            KeyCode::Char('n') if self.trace_label.is_none() && self.next_cursor.is_some() => {
                return vec![AppAction::Gov(self.query_request(now, true))];
            }
            KeyCode::Char('t') => {
                if let Some(e) = self.items.get(self.nav.selected) {
                    self.trace_label = Some(e.id.to_string());
                    return vec![AppAction::Gov(trace_request(&e.id.to_string()))];
                }
            }
            KeyCode::Enter => {
                if let Some(e) = self.items.get(self.nav.selected) {
                    return vec![AppAction::Gov(GovRequest::new(
                        GovTag::Explain,
                        "events.explain",
                        &json!({ "id": e.id }),
                    ))];
                }
            }
            KeyCode::Char('R') if lab_mode => {
                return vec![AppAction::Gov(GovRequest::new(GovTag::LabStart, "lab.replay", &json!({})))];
            }
            KeyCode::Char('E') if lab_mode => {
                return vec![AppAction::Gov(GovRequest::new(
                    GovTag::LabStart,
                    "lab.export_learning",
                    &json!({}),
                ))];
            }
            KeyCode::Char('X') if lab_mode => {
                if let Some(j) = self.job.as_ref().filter(|j| j.running()) {
                    return vec![AppAction::Gov(GovRequest::new(
                        GovTag::LabJob,
                        "lab.cancel",
                        &json!({ "job": j.id }),
                    ))];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    pub fn job_started(&mut self, v: &Value) {
        self.job = Some(JobView { id: ju64(v, "job"), kind: String::new(), state: "running".to_owned(), done: 0, total: 0 });
    }

    #[must_use]
    pub fn job_request(&self) -> GovRequest {
        GovRequest::new(
            GovTag::LabJob,
            "lab.job",
            &json!({ "job": self.job.as_ref().map_or(0, |j| j.id) }),
        )
    }

    pub fn job_failed(&mut self, e: &str) {
        if let Some(j) = &mut self.job {
            j.state = format!("failed: {e}");
        }
    }

    /// Applies a `lab.job` snapshot; returns the result overlay when the job just finished.
    pub fn job_update(&mut self, v: &Value) -> Option<Detail> {
        let was_running = self.job.as_ref().is_some_and(JobView::running);
        let state = jstr(v, "state");
        self.job = Some(JobView {
            id: ju64(v, "job"),
            kind: jstr(v, "kind"),
            state: state.clone(),
            done: ju64(v, "done"),
            total: ju64(v, "total"),
        });
        if !was_running || state == "running" {
            return None;
        }
        let r = v.get("result").cloned().unwrap_or(Value::Null);
        let lines = match (jstr(v, "kind").as_str(), state.as_str()) {
            (_, "cancelled") => vec!["Job cancelled; the live log was not touched.".to_owned()],
            (_, "failed") => vec![format!("Job failed: {}", jstr(v, "error"))],
            ("replay", _) => {
                if jstr(&r, "status") == "ok" {
                    vec![format!(
                        "all decisions reproduced (matched={}, replayed={})",
                        ju64(&r, "matched_decisions"),
                        ju64(&r, "replayed_events")
                    )]
                } else {
                    let d = r.get("first_divergence").cloned().unwrap_or(Value::Null);
                    vec![format!(
                        "replay divergence at seq={} decision_event={} target_event={} field={}: {}",
                        ju64(&d, "seq"),
                        ju64(&d, "decision_event_id"),
                        ju64(&d, "target_event_id"),
                        jstr(&d, "field"),
                        jstr(&d, "detail")
                    )]
                }
            }
            _ => vec![format!(
                "exported learned state to {} (blake3={} seq={} epoch={})",
                jstr(&r, "out"),
                jstr(&r, "export_blake3"),
                ju64(&r, "log_seq"),
                ju64(&r, "state_epoch")
            )],
        };
        let title = if jstr(v, "kind") == "replay" { "Replay (Lab, on a copy)" } else { "Export learning (Lab)" };
        Some(Detail { title: title.to_owned(), lines, offset: 0 })
    }

    #[must_use]
    pub fn view(&self, lab_mode: bool) -> ViewModel {
        let rows: Vec<Row> = self
            .items
            .iter()
            .map(|e| {
                let tone = match e.kind.as_str() {
                    "failure" => Tone::Bad,
                    "door_verdict" => Tone::Warn,
                    _ => Tone::Normal,
                };
                Row::new(
                    format!(
                        "{} #{:<6} {:<12} {:<16} {:<12} {}",
                        clock_label(e.timestamp_ms),
                        e.id,
                        e.kind,
                        e.place,
                        e.actor,
                        e.summary
                    ),
                    tone,
                )
            })
            .collect();
        let mut header = match &self.trace_label {
            Some(t) => format!("trace {t}: {} events", self.items.len()),
            None => format!(
                "{}/{}  place:{} source:{} kind:{} range:{}",
                self.items.len(),
                self.matching,
                self.place.as_deref().unwrap_or("all"),
                self.source.as_deref().unwrap_or("all"),
                self.kind.as_deref().unwrap_or("all"),
                self.range.as_deref().unwrap_or("all"),
            ),
        };
        if let Some(j) = &self.job {
            let pct = (j.done * 100).checked_div(j.total).unwrap_or(0);
            let filled = (pct / 10) as usize;
            header.push_str(&format!(
                "  [{}{}] {} {} {}%",
                "#".repeat(filled),
                ".".repeat(10 - filled),
                if j.kind.is_empty() { "job" } else { &j.kind },
                j.state,
                pct
            ));
        }
        let mut footer = if self.trace_label.is_some() {
            " Enter explain  r refresh  Esc back to the log".to_owned()
        } else {
            " Enter explain  t trace  T trace id  p place  s source  k kind  w time  x clear  n more  Esc close"
                .to_owned()
        };
        if lab_mode {
            footer.push_str("  R replay  E export  X cancel");
        }
        ViewModel {
            title: "Events".to_owned(),
            header,
            rows,
            selected: Some(self.nav.selected),
            offset: self.nav.offset,
            footer,
            empty: if self.loaded { "No events match.".to_owned() } else { "Loading...".to_owned() },
        }
    }
}

fn parse_rows(v: &Value) -> Vec<EventItem> {
    v.get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| EventItem {
                    id: ju64(e, "id"),
                    seq: ju64(e, "seq"),
                    kind: jstr(e, "kind"),
                    timestamp_ms: ju64(e, "timestamp_ms"),
                    place: jstr(e, "place"),
                    actor: jstr(e, "actor"),
                    summary: jstr(e, "summary"),
                })
                .collect()
        })
        .unwrap_or_default()
}
